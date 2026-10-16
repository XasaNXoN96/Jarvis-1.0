// Tests of the AI chat with a local mock server (no real provider, no real key).
use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

const KEY: &str = "sk-test-SECRET-1234567890";

struct Mock {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
}

// answers the connections one by one with the given (status, body, delay in ms)
fn mock(responses: Vec<(u16, String, u64)>) -> Mock {
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    std::env::set_var("no_proxy", "127.0.0.1,localhost");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    std::thread::spawn(move || {
        for (status, body, delay) in responses {
            let Ok((mut stream, _)) = listener.accept() else { return };
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let (mut header_end, mut want) = (None, 0usize);
            loop {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if header_end.is_none() {
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        header_end = Some(p + 4);
                        let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                        want = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                    }
                }
                if let Some(h) = header_end {
                    if buf.len() >= h + want {
                        break;
                    }
                }
            }
            seen.lock().push(String::from_utf8_lossy(&buf).to_string());
            std::thread::sleep(Duration::from_millis(delay));
            let _ = write!(stream, "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", status, body.len(), body);
        }
    });
    Mock { base: format!("http://127.0.0.1:{}", port), requests }
}

fn ok_body(text: &str) -> String {
    json!({ "choices": [{ "message": { "role": "assistant", "content": text } }] }).to_string()
}

fn conf(base: &str) -> Result<ProviderConf, AiError> {
    Ok(ProviderConf { kind: Kind::OpenAiCompatible, name: "Test".into(), base: base.to_string(), model: "test-model".into(), key: KEY.into(), route: String::new() })
}

fn leaked_store() -> &'static Store {
    let dir = std::env::temp_dir().join(format!("jarvis-ai-test-{}-{}", std::process::id(), COUNTER.fetch_add(1, Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    Box::leak(Box::new(Store::new(dir)))
}

fn run(store: &'static Store, c: Result<ProviderConf, AiError>, chat: Option<&str>, text: &str) -> Result<String, AiError> {
    let out = send_with(store, c, chat, text, false)?;
    out.handle.join().unwrap();
    Ok(out.chat_id)
}

fn messages(store: &Store, id: &str) -> Vec<ChatMsg> {
    store.load(id).unwrap().messages
}

#[test]
fn settings_to_provider() {
    assert_eq!(conf_from("none", "m", "k", "").unwrap_err(), AiError::NoProvider);
    assert_eq!(conf_from("openai", "gpt", "", "").unwrap_err(), AiError::NoKey("OpenAI".into()));
    assert_eq!(conf_from("ollama", "", "", "").unwrap_err(), AiError::NoModel);
    assert_eq!(conf_from("openai", "", "k", "").unwrap().model, "gpt-4o-mini");
    assert_eq!(conf_from("gemini", "gemini", "k", "").unwrap().model, "gemini-flash-latest");
    assert_eq!(conf_from("gemini", "gemini-2.5-pro", "k", "").unwrap().model, "gemini-2.5-pro");
    assert_eq!(conf_from("custom", "m", "k", "").unwrap_err(), AiError::NoUrl);
    assert!(conf_from("ollama", "llama3", "", "").is_ok()); // no key needed
    assert!(conf_from("custom", "m", "", "http://localhost:8080/v1").is_ok());
    assert_eq!(conf_from("custom", "m", "", "https://api.example.com/v1").unwrap_err(), AiError::NoKey("ИИ-сервер".into()));
    let g = conf_from("gemini", "gemini-2.0-flash", KEY, "").unwrap();
    assert_eq!(g.base, "https://generativelanguage.googleapis.com/v1beta/openai");
    assert_eq!(endpoint(&g), "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions");
    let r = conf_from("openrouter", "openai/gpt-4o-mini", KEY, "").unwrap();
    assert_eq!(endpoint(&r), "https://openrouter.ai/api/v1/chat/completions");
    assert_eq!(endpoint(&conf_from("anthropic", "claude", KEY, "").unwrap()), "https://api.anthropic.com/v1/messages");
    // the key never shows up in Debug output
    assert!(!format!("{:?}", g).contains(KEY));
}

#[test]
fn request_bodies_have_no_key_and_right_shape() {
    let msgs = vec![
        ChatMsg { role: "user".into(), content: "привет".into(), ts: 0, ..Default::default() },
        ChatMsg { role: "assistant".into(), content: "здравствуйте".into(), ts: 0, ..Default::default() },
        ChatMsg { role: "user".into(), content: "как дела".into(), ts: 0, ..Default::default() },
    ];
    let c = conf("http://x").unwrap();
    let body = build_body(&c, &msgs, "SYS");
    assert_eq!(body["model"], "test-model");
    assert_eq!(body["messages"][0], json!({ "role": "system", "content": "SYS" }));
    assert_eq!(body["messages"][3]["content"], "как дела");
    assert!(!body.to_string().contains(KEY));

    let a = ProviderConf { kind: Kind::Anthropic, ..c.clone() };
    let two_users = vec![msgs[0].clone(), msgs[2].clone()];
    let ab = build_body(&a, &two_users, "SYS");
    assert_eq!(ab["system"], "SYS");
    assert_eq!(ab["messages"].as_array().unwrap().len(), 1); // roles must alternate: joined
    assert!(ab["max_tokens"].as_u64().unwrap() > 0);
    assert!(!ab.to_string().contains(KEY));
}

#[test]
fn context_is_limited_and_clean() {
    let mut msgs = Vec::new();
    for i in 0..100 {
        msgs.push(ChatMsg { role: if i % 2 == 0 { "user" } else { "assistant" }.into(), content: format!("сообщение {}", i), ts: 0, ..Default::default() });
    }
    msgs.push(ChatMsg { role: "error".into(), content: "ошибка".into(), ts: 0, ..Default::default() });
    let ctx = context_for_model(&msgs);
    assert!(ctx.len() <= MAX_CONTEXT_MESSAGES);
    assert_eq!(ctx.first().unwrap().role, "user");
    assert!(ctx.iter().all(|m| m.role == "user" || m.role == "assistant"));
    // a budget in characters
    let big = vec![
        ChatMsg { role: "user".into(), content: "a".repeat(8000), ts: 0, ..Default::default() },
        ChatMsg { role: "assistant".into(), content: "b".repeat(8000), ts: 0, ..Default::default() },
        ChatMsg { role: "user".into(), content: "вопрос".into(), ts: 0, ..Default::default() },
    ];
    let ctx = context_for_model(&big);
    assert_eq!(ctx.last().unwrap().content, "вопрос");
    assert!(ctx.iter().map(|m| m.content.chars().count()).sum::<usize>() <= MAX_CONTEXT_CHARS + 2 * MAX_MESSAGE_CHARS, "the question and the exchange before it are always kept");
}

#[test]
fn answers_and_errors_are_parsed() {
    let c = conf("http://x").unwrap();
    assert_eq!(parse_answer(&c, &serde_json::from_str(&ok_body("  Ответ  ")).unwrap()).unwrap(), "Ответ");
    assert_eq!(parse_answer(&c, &json!({ "choices": [] })).unwrap_err(), AiError::BadResponse);
    assert_eq!(parse_answer(&c, &json!({})).unwrap_err(), AiError::BadResponse);
    let a = ProviderConf { kind: Kind::Anthropic, ..c.clone() };
    assert_eq!(parse_answer(&a, &json!({ "content": [{ "type": "text", "text": "Привет" }, { "type": "text", "text": "!" }] })).unwrap(), "Привет!");

    assert_eq!(map_status(&c, 401, ""), AiError::Auth("Test".into()));
    assert_eq!(map_status(&c, 429, ""), AiError::Quota("Test".into()));
    assert_eq!(map_status(&c, 402, ""), AiError::Quota("Test".into()));
    assert_eq!(map_status(&c, 404, "{\"error\":{\"message\":\"The model does not exist\"}}"), AiError::ModelUnknown("test-model".into()));
    assert_eq!(map_status(&c, 503, ""), AiError::Server(503));
    // a provider message that repeats the key is cleaned
    let e = map_status(&c, 400, &json!({ "error": { "message": format!("bad request for key {}", KEY) } }).to_string());
    assert!(!e.user_text().contains(KEY));
    // every error says what to do
    for e in [AiError::NoProvider, AiError::NoKey("X".into()), AiError::Auth("X".into()), AiError::Quota("X".into()), AiError::Timeout, AiError::Server(500), AiError::BadResponse, AiError::Network("x".into())] {
        assert!(e.user_text().len() > 30);
    }
}

#[test]
fn real_answer_through_the_mock_server_and_follow_up() {
    let store = leaked_store();
    let m = mock(vec![(200, ok_body("Москва."), 0), (200, ok_body("Около 13 миллионов."), 0)]);
    let id = run(store, conf(&m.base), None, "Какая столица России?").unwrap();
    let msgs = messages(store, &id);
    assert_eq!(msgs.len(), 2);
    assert_eq!((msgs[0].role.as_str(), msgs[1].role.as_str(), msgs[1].content.as_str()), ("user", "assistant", "Москва."));

    // the request: path, key only in the header, text in the body
    let first = m.requests.lock()[0].clone();
    assert!(first.starts_with("POST /chat/completions HTTP/1.1"));
    assert!(first.to_lowercase().contains(&format!("authorization: bearer {}", KEY.to_lowercase())));
    let first_line = first.lines().next().unwrap();
    assert!(!first_line.contains(KEY));
    let body: Value = serde_json::from_str(first.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert!(!body.to_string().contains(KEY));
    assert_eq!(body["messages"][1]["content"], "Какая столица России?");

    // a follow-up question carries the previous messages
    run(store, conf(&m.base), Some(&id), "А сколько там людей?").unwrap();
    let second = m.requests.lock()[1].clone();
    let body2: Value = serde_json::from_str(second.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    let sent: Vec<String> = body2["messages"].as_array().unwrap().iter().map(|m| m["content"].as_str().unwrap().to_string()).collect();
    assert!(sent.contains(&"Какая столица России?".to_string()) && sent.contains(&"Москва.".to_string()) && sent.contains(&"А сколько там людей?".to_string()));
    assert_eq!(messages(store, &id).len(), 4);
}

#[test]
fn conversations_are_separate_and_persist() {
    let store = leaked_store();
    let m = mock(vec![(200, ok_body("первый ответ"), 0), (200, ok_body("второй ответ"), 0)]);
    let a = run(store, conf(&m.base), None, "тайный вопрос про синий слон").unwrap();
    let b_chat = store.create();
    assert_ne!(a, b_chat.id);
    run(store, conf(&m.base), Some(&b_chat.id), "новая тема").unwrap();
    // the new chat did not get the old history
    let second = m.requests.lock()[1].clone();
    assert!(!second.contains("синий слон"));
    assert_eq!(messages(store, &a).len(), 2);
    assert_eq!(messages(store, &b_chat.id).len(), 2);

    // restored after a restart: a new Store on the same folder
    let reopened = Store::new(store.dir.clone());
    assert_eq!(reopened.load(&a).unwrap().messages[0].content, "тайный вопрос про синий слон");
    assert_eq!(reopened.active().unwrap(), b_chat.id);
    assert_eq!(reopened.list().len(), 2);
    assert_eq!(reopened.load(&a).unwrap().title, "тайный вопрос про синий слон");

    // clear and delete; a path trick does not work
    assert!(reopened.clear(&a));
    assert!(reopened.load(&a).unwrap().messages.is_empty());
    assert!(reopened.delete(&b_chat.id));
    assert!(reopened.load(&b_chat.id).is_none());
    assert!(reopened.load("../../etc/passwd").is_none());
    assert!(!reopened.delete("../x"));
}

#[test]
fn api_errors_are_shown_and_the_key_is_not_stored() {
    for (status, body, expect) in [
        (401, "{}", "не принял ключ"),
        (429, "{}", "лимит"),
        (404, "{\"error\":{\"message\":\"model not found\"}}", "не знает модель"),
        (500, "oops", "ошибку 500"),
        (200, "это не json", "непонятном формате"),
    ] {
        let store = leaked_store();
        // (a 5xx answer is asked for again twice before the error is shown: the same answer three times)
        let tries = if status == 500 { 3 } else { 1 };
        SERVER_RETRY_PAUSE_MS.store(10, Ordering::SeqCst);
        let m = mock((0..tries).map(|_| (status, body.to_string(), 0)).collect());
        let id = run(store, conf(&m.base), None, "вопрос").unwrap();
        SERVER_RETRY_PAUSE_MS.store(600, Ordering::SeqCst);
        let msgs = messages(store, &id);
        assert_eq!(msgs.last().unwrap().role, "error", "{}", status);
        assert!(msgs.last().unwrap().content.contains(expect), "{}: {}", status, msgs.last().unwrap().content);
        // the stored file has no key
        let file = std::fs::read_to_string(store.dir.join(format!("{}.json", id))).unwrap();
        assert!(!file.contains(KEY));
        // an error is not sent to the model with the next question
        assert!(context_for_model(&msgs).iter().all(|m| m.role != "error"));
    }
}

#[test]
fn unreachable_server_is_a_network_error() {
    let store = leaked_store();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener); // nobody listens on that port now
    let id = run(store, conf(&format!("http://127.0.0.1:{}", port)), None, "вопрос").unwrap();
    assert!(messages(store, &id).last().unwrap().content.contains("Нет соединения"));
}

#[test]
fn missing_key_empty_and_long_messages_are_refused_without_a_request() {
    let store = leaked_store();
    let m = mock(vec![]);
    assert_eq!(send_with(store, conf_from("openai", "gpt-4o-mini", "", ""), None, "привет", false).err(), Some(AiError::NoKey("OpenAI".into())));
    assert_eq!(send_with(store, conf(&m.base), None, "   ", false).err(), Some(AiError::Empty));
    assert_eq!(send_with(store, conf(&m.base), None, &"я".repeat(MAX_MESSAGE_CHARS + 1), false).err(), Some(AiError::TooLong));
    assert!(m.requests.lock().is_empty());
    // nothing was stored for the refused messages
    assert!(store.list().iter().all(|c| c["count"] == 0));
}

#[test]
fn a_second_message_waits_and_stop_cancels() {
    let store = leaked_store();
    let m = mock(vec![(200, ok_body("поздний ответ"), 1500)]);
    let first = send_with(store, conf(&m.base), None, "долгий вопрос", false).unwrap();
    assert!(is_busy(&first.chat_id));
    assert_eq!(send_with(store, conf(&m.base), Some(&first.chat_id), "ещё один", false).err(), Some(AiError::Busy));
    assert!(stop(&first.chat_id));
    first.handle.join().unwrap();
    assert!(!is_busy(&first.chat_id));
    let msgs = messages(store, &first.chat_id);
    assert_eq!(msgs.last().unwrap().role, "error");
    assert!(msgs.last().unwrap().content.contains("Остановлено"));
    assert!(msgs.iter().all(|m| m.content != "поздний ответ"));
}

#[test]
fn text_chat_does_not_need_the_microphone_or_the_voice() {
    // voice mode is not running (no microphone) and speech is off: the answer still appears in the chat
    assert!(!crate::VOICE_ACTIVE.load(Ordering::SeqCst));
    let _g = crate::uistate::TEST_LOCK.lock();
    crate::tts::set_enabled(false);
    let store = leaked_store();
    let m = mock(vec![(200, ok_body("ответ без голоса"), 0)]);
    let out = send_with(store, conf(&m.base), None, "привет", true).unwrap();
    out.handle.join().unwrap();
    assert_eq!(messages(store, &out.chat_id).last().unwrap().content, "ответ без голоса");
    // the answer is not a command: nothing was pushed into the queue of typed commands
    assert!(!crate::remote::command_queue_connected_for_tests());
    crate::tts::set_enabled(true);
}

#[test]
fn the_answer_of_the_ai_is_read_aloud_when_speech_is_on() {
    let _g = crate::uistate::TEST_LOCK.lock();
    crate::tts::set_enabled(true);
    let store = leaked_store();
    let m = mock(vec![(200, ok_body("Столица Франции — Париж."), 0)]);
    let before = crate::tts::TEST_SPOKEN.lock().len();
    let out = send_with(store, conf(&m.base), None, "столица франции", true).unwrap();
    out.handle.join().unwrap();
    assert_eq!(messages(store, &out.chat_id).last().unwrap().content, "Столица Франции — Париж.");
    let spoken: Vec<String> = crate::tts::TEST_SPOKEN.lock()[before..].to_vec();
    assert_eq!(spoken.iter().filter(|s| s.contains("Париж")).count(), 1, "said once: {:?}", spoken);
}

#[test]
fn spoken_text_has_no_code_or_marks() {
    let s = speakable("Вот **пример**:\n```python\nprint(1)\n```\nГотово.");
    assert!(!s.contains("print") && !s.contains('*') && !s.contains('`'));
    assert!(s.contains("Готово"));
    assert!(speakable(&"Слово. ".repeat(300)).chars().count() <= 700);
}

// ---------- plain questions go to the selected provider (Gemini) ----------
#[test]
fn gemini_is_configured_with_its_openai_compatible_endpoint_and_a_default_model() {
    let c = conf_from("gemini", "gemini", "k-123", "").unwrap();
    assert_eq!(c.base, "https://generativelanguage.googleapis.com/v1beta/openai");
    assert_eq!(c.model, "gemini-flash-latest");
    assert!(!format!("{:?}", c).contains("k-123"));
}

#[test]
fn a_question_reaches_the_provider_with_the_key_only_in_the_header() {
    let store = leaked_store();
    let m = mock(vec![(200, ok_body("Программа — это набор инструкций."), 0)]);
    let c = Ok(ProviderConf { kind: Kind::OpenAiCompatible, name: "Google Gemini".into(), base: m.base.clone(), model: "gemini-flash-latest".into(), key: KEY.into(), route: String::new() });
    let id = run(store, c, None, "Что такое программа?").unwrap();
    let msgs = messages(store, &id);
    assert_eq!(msgs.last().unwrap().content, "Программа — это набор инструкций.");
    let req = m.requests.lock()[0].clone();
    assert!(req.starts_with("POST /chat/completions"), "{}", req.lines().next().unwrap_or(""));
    assert!(req.contains(&format!("Bearer {}", KEY)));
    assert!(req.contains("Что такое программа?") && req.contains("gemini-flash-latest"));
    // the key is not in the URL and not in the stored dialog
    assert!(!req.lines().next().unwrap().contains(KEY));
    let file = std::fs::read_to_string(store.dir.join(format!("{}.json", id))).unwrap();
    assert!(!file.contains(KEY));
}

#[test]
fn a_failed_answer_calls_the_hook_and_only_outages_allow_a_web_search() {
    for (status, web) in [(429, true), (500, true), (401, false), (404, false)] {
        let store = leaked_store();
        let m = mock(vec![(status, "{\"error\":{\"message\":\"model not found\"}}".to_string(), 0)]);
        let (tx, rx) = std::sync::mpsc::channel();
        let out = send_with_hook(store, conf(&m.base), None, "вопрос", false, Some(Box::new(move |e| { let _ = tx.send(e.clone()); }))).unwrap();
        out.handle.join().unwrap();
        let err = rx.recv_timeout(Duration::from_secs(3)).expect("the hook must be called");
        assert_eq!(err.allows_web_search(), web, "{}: {:?}", status, err);
    }
    // no internet at all
    let store = leaked_store();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    let (tx, rx) = std::sync::mpsc::channel();
    let out = send_with_hook(store, conf(&format!("http://127.0.0.1:{}", port)), None, "вопрос", false, Some(Box::new(move |e| { let _ = tx.send(e.clone()); }))).unwrap();
    out.handle.join().unwrap();
    assert!(rx.recv_timeout(Duration::from_secs(3)).unwrap().allows_web_search());
}

#[test]
fn a_successful_answer_does_not_call_the_error_hook() {
    let store = leaked_store();
    let m = mock(vec![(200, ok_body("ответ"), 0)]);
    let (tx, rx) = std::sync::mpsc::channel::<AiError>();
    let out = send_with_hook(store, conf(&m.base), None, "вопрос", false, Some(Box::new(move |e| { let _ = tx.send(e.clone()); }))).unwrap();
    out.handle.join().unwrap();
    assert!(rx.try_recv().is_err());
}

// =============================================================================================
// streaming, voice by sentences, timing, state of the connection
// =============================================================================================
use std::time::Instant;

fn sse_chunk(text: &str) -> String {
    format!("data: {}\n\n", json!({ "choices": [{ "delta": { "content": text } }] }))
}

// a server that streams: (delay before the part in ms, the part); the connection closes after the last one
fn sse_mock(parts: Vec<(u64, String)>) -> Mock {
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    std::env::set_var("no_proxy", "127.0.0.1,localhost");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let seen = requests.clone();
    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else { return };
        let mut buf = Vec::new();
        let mut chunk = [0u8; 4096];
        let (mut header_end, mut want) = (None, 0usize);
        loop {
            let n = stream.read(&mut chunk).unwrap_or(0);
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if header_end.is_none() {
                if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = Some(p + 4);
                    let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                    want = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                }
            }
            if let Some(h) = header_end {
                if buf.len() >= h + want {
                    break;
                }
            }
        }
        seen.lock().push(String::from_utf8_lossy(&buf).to_string());
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n");
        let _ = stream.flush();
        for (delay, part) in parts {
            std::thread::sleep(Duration::from_millis(delay));
            if stream.write_all(part.as_bytes()).is_err() {
                return;
            }
            let _ = stream.flush();
        }
    });
    Mock { base: format!("http://127.0.0.1:{}", port), requests }
}

#[test]
fn sse_lines_of_both_api_styles_are_parsed() {
    let oa = Kind::OpenAiCompatible;
    assert_eq!(parse_sse_line(&oa, &sse_chunk("Привет").trim_end()), Sse::Delta("Привет".into()));
    assert_eq!(parse_sse_line(&oa, "data: [DONE]"), Sse::Done);
    assert_eq!(parse_sse_line(&oa, ": keep-alive"), Sse::Skip);
    assert_eq!(parse_sse_line(&oa, "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}"), Sse::Skip);
    assert_eq!(parse_sse_line(&oa, "data: {\"error\":{\"message\":\"quota\"}}"), Sse::Error("quota".into()));
    assert_eq!(parse_sse_line(&oa, "data: not json"), Sse::Skip);
    let an = Kind::Anthropic;
    assert_eq!(parse_sse_line(&an, "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}"), Sse::Delta("Hi".into()));
    assert_eq!(parse_sse_line(&an, "data: {\"type\":\"message_stop\"}"), Sse::Done);
    assert_eq!(parse_sse_line(&an, "data: {\"type\":\"ping\"}"), Sse::Skip);
    assert_eq!(parse_sse_line(&an, "event: content_block_delta"), Sse::Skip);
}

#[test]
fn the_stream_body_asks_for_a_stream_and_the_normal_one_does_not() {
    let c = conf("http://x").unwrap();
    let h = vec![ChatMsg { role: "user".into(), content: "вопрос".into(), ..Default::default() }];
    assert_eq!(build_body(&c, &h, "s")["stream"], json!(false));
    assert_eq!(build_body_opts(&c, &h, "s", true)["stream"], json!(true));
    let a = ProviderConf { kind: Kind::Anthropic, name: "A".into(), base: "http://x".into(), model: "m".into(), key: "k".into(), route: String::new() };
    assert!(build_body(&a, &h, "s").get("stream").is_none());
    assert_eq!(build_body_opts(&a, &h, "s", true)["stream"], json!(true));
}

#[test]
fn sentences_are_spoken_whole_without_code_and_marks() {
    let mut s = SentenceSpeaker::new();
    let mut spoken: Vec<String> = Vec::new();
    for d in ["Python — это язык", " программирования, который ", "очень популярен. Он простой! ", "А вот код:\n```py", "thon\nprint('привет')\n```\n", "Запустите **его** так: `python a.py`. Готово"] {
        spoken.extend(s.push(d));
    }
    spoken.extend(s.finish());
    let all = spoken.join(" | ");
    assert!(all.contains("Python — это язык программирования, который очень популярен."), "{}", all);
    assert!(!all.contains("print"), "code is not read: {}", all);
    assert!(all.contains("Код показан в окне"), "{}", all);
    assert!(!all.contains('*') && !all.contains('`'), "{}", all);
    assert!(all.contains("Готово"), "the tail is spoken at the end: {}", all);
    // nothing is said twice
    let joined = spoken.join(" ");
    assert_eq!(joined.matches("Он простой").count(), 1, "{}", joined);
}

#[test]
fn a_very_long_answer_is_not_read_to_the_end() {
    let mut s = SentenceSpeaker::new();
    let mut total = 0usize;
    for _ in 0..80 {
        for p in s.push("Это довольно длинное предложение для проверки предела озвучивания. ") {
            total += p.chars().count();
        }
    }
    for p in s.finish() {
        total += p.chars().count();
    }
    assert!(total > 200 && total < 1500, "{}", total);
}

#[test]
fn status_changes_only_after_a_real_result() {
    let s = Status::unverified();
    assert_eq!(s.conn, Conn::Unverified, "a saved key alone is not a connection");
    let ok = s.after_ok();
    assert_eq!(ok.conn, Conn::Connected);
    let bad = ok.after_error(&AiError::Auth("Gemini".into()));
    assert_eq!(bad.conn, Conn::Error);
    assert!(bad.detail.contains("не принял ключ"), "{}", bad.detail);
    // a stop is not a failure of the connection
    assert_eq!(ok.after_error(&AiError::Cancelled).conn, Conn::Connected);
    assert!(!bad.detail.contains(KEY));
}

fn wait_for<F: Fn() -> bool>(what: &str, secs: u64, f: F) {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(secs) {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    panic!("timeout: {}", what);
}

#[test]
fn the_answer_appears_fragment_by_fragment_and_is_stored_once() {
    let store = leaked_store();
    let parts: Vec<(u64, String)> = vec![(50, sse_chunk("Машинное ")), (250, sse_chunk("обучение — это ")), (250, sse_chunk("раздел ИИ.")), (50, "data: [DONE]\n\n".to_string())];
    let m = sse_mock(parts);
    let out = send_with(store, conf(&m.base), None, "Что такое машинное обучение?", false).unwrap();
    let id = out.chat_id.clone();
    // the first fragment is visible while the answer is still being generated
    wait_for("the first fragment", 5, || store.load_with_live(&id).map(|c| c.messages.last().map(|m| m.streaming && m.content.contains("Машинное")).unwrap_or(false)).unwrap_or(false));
    let mid = store.load_with_live(&id).unwrap();
    assert!(mid.messages.last().unwrap().streaming);
    assert!(!mid.messages.last().unwrap().content.contains("раздел ИИ"), "the end has not arrived yet");
    // the stored history has the question only, the live part is not stored
    assert_eq!(store.load(&id).unwrap().messages.len(), 1);
    let timing = out.handle.join().unwrap();
    let msgs = messages(store, &id);
    assert_eq!(msgs.len(), 2, "one question, one answer");
    assert_eq!(msgs[1].content, "Машинное обучение — это раздел ИИ.");
    assert_eq!((msgs[0].role.as_str(), msgs[1].role.as_str()), ("user", "assistant"));
    assert_eq!(msgs[0].rid, msgs[1].rid, "the question and the answer share the request id");
    assert!(!msgs[1].rid.is_empty());
    assert!(store.load_with_live(&id).unwrap().messages.iter().all(|m| !m.streaming), "nothing is live any more");
    // exactly one request went out and it asked for a stream
    assert_eq!(m.requests.lock().len(), 1);
    assert!(m.requests.lock()[0].contains("\"stream\":true"));
    assert!(timing.streamed && timing.chunks == 3, "{:?}", timing);
}

#[test]
fn stopping_a_stream_keeps_what_arrived_and_ends_the_generation() {
    let store = leaked_store();
    let mut parts: Vec<(u64, String)> = vec![(20, sse_chunk("Начало ответа. "))];
    for _ in 0..60 {
        parts.push((100, sse_chunk("ещё текст ")));
    }
    let m = sse_mock(parts);
    let out = send_with(store, conf(&m.base), None, "длинный вопрос", false).unwrap();
    let id = out.chat_id.clone();
    wait_for("a fragment", 5, || store.load_with_live(&id).map(|c| c.messages.last().map(|m| m.streaming).unwrap_or(false)).unwrap_or(false));
    let t = Instant::now();
    assert!(stop(&id));
    out.handle.join().unwrap();
    assert!(t.elapsed() < Duration::from_secs(2), "the generation must end quickly after Stop: {:?}", t.elapsed());
    let msgs = messages(store, &id);
    assert_eq!(msgs[1].role, "assistant");
    assert!(msgs[1].content.contains("Начало ответа"));
    assert_eq!(msgs.last().unwrap().role, "error");
    assert!(msgs.last().unwrap().content.contains("Остановлено"));
    let count = msgs.len();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(messages(store, &id).len(), count, "nothing is added after Stop");
    assert!(!is_busy(&id));
}

#[test]
fn a_broken_stream_keeps_the_partial_answer_and_shows_the_error() {
    let store = leaked_store();
    let parts = vec![(20, sse_chunk("Часть ответа. ")), (20, "data: {\"error\":{\"message\":\"internal failure\"}}\n\n".to_string())];
    let m = sse_mock(parts);
    let id = run(store, conf(&m.base), None, "вопрос").unwrap();
    let msgs = messages(store, &id);
    assert!(msgs.iter().any(|m| m.role == "assistant" && m.content.contains("Часть ответа")), "{:?}", msgs);
    let last = msgs.last().unwrap();
    assert_eq!(last.role, "error");
    assert!(last.content.contains("internal failure"), "the error of the provider is not hidden: {}", last.content);
}

#[test]
fn streaming_shows_the_first_words_long_before_the_whole_answer() {
    // The same 12-fragment answer (100 ms per fragment), once as a stream and once as one JSON at the end.
    let words: Vec<String> = (1..=12).map(|i| format!("слово{} ", i)).collect();
    let store = leaked_store();
    let mut parts: Vec<(u64, String)> = words.iter().map(|w| (100, sse_chunk(w))).collect();
    parts.push((0, "data: [DONE]\n\n".to_string()));
    let m = sse_mock(parts);
    let out = send_with(store, conf(&m.base), None, "вопрос", false).unwrap();
    let id = out.chat_id.clone();
    let streamed = out.handle.join().unwrap();
    assert_eq!(messages(store, &id).last().unwrap().content, words.concat().trim());

    let store2 = leaked_store();
    let m2 = mock(vec![(200, ok_body(words.concat().trim()), 1200)]);
    let out2 = send_with(store2, conf(&m2.base), None, "вопрос", false).unwrap();
    let id2 = out2.chat_id.clone();
    let plain = out2.handle.join().unwrap();
    assert_eq!(messages(store2, &id2).last().unwrap().content, words.concat().trim());

    eprintln!("TIMING stream : first fragment {:?} ms, total {} ms, fragments {}", streamed.first_chunk_ms, streamed.total_ms, streamed.chunks);
    eprintln!("TIMING plain  : first fragment {:?} ms, total {} ms, fragments {}", plain.first_chunk_ms, plain.total_ms, plain.chunks);
    assert!(streamed.streamed && !plain.streamed);
    assert!(streamed.first_chunk_ms.unwrap() < 400, "{:?}", streamed);
    assert!(plain.first_chunk_ms.unwrap() >= 1100, "{:?}", plain);
    assert!(streamed.first_chunk_ms.unwrap() * 3 < plain.first_chunk_ms.unwrap());
}

#[test]
fn the_connection_is_reused_between_questions() {
    // one HTTP client for the program: two questions to the same server use one connection (no new handshake)
    let store = leaked_store();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    let accepted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let acc = accepted.clone();
    std::thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            acc.fetch_add(1, Ordering::SeqCst);
            std::thread::spawn(move || {
                // keep-alive: answer every request that comes on this connection
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                loop {
                    let n = match stream.read(&mut chunk) { Ok(0) | Err(_) => return, Ok(n) => n };
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                        let want = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if buf.len() >= p + 4 + want {
                            buf.clear();
                            let body = ok_body("ответ");
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}", body.len(), body);
                        }
                    }
                }
            });
        }
    });
    let base = format!("http://127.0.0.1:{}", port);
    let id = run(store, conf(&base), None, "первый").unwrap();
    run(store, conf(&base), Some(&id), "второй").unwrap();
    assert_eq!(messages(store, &id).len(), 4);
    assert_eq!(accepted.load(Ordering::SeqCst), 1, "the second question must reuse the first connection");
}

#[test]
fn commands_and_their_replies_are_not_sent_to_the_model() {
    let msgs = vec![
        ChatMsg { role: "user".into(), content: "открой ворд".into(), src: "command".into(), ..Default::default() },
        ChatMsg { role: "assistant".into(), content: "Открываю Word".into(), src: "command".into(), ..Default::default() },
        ChatMsg { role: "user".into(), content: "привет, как дела?".into(), src: "local".into(), ..Default::default() },
        ChatMsg { role: "assistant".into(), content: "Отлично, сэр".into(), src: "local".into(), ..Default::default() },
        ChatMsg { role: "user".into(), content: "что такое питон".into(), src: "ai".into(), ..Default::default() },
        ChatMsg { role: "assistant".into(), content: "Язык программирования".into(), src: "ai".into(), ..Default::default() },
        ChatMsg { role: "user".into(), content: "а кто его создал".into(), src: "ai".into(), ..Default::default() },
    ];
    let ctx = context_for_model(&msgs);
    let texts: Vec<&str> = ctx.iter().map(|m| m.content.as_str()).collect();
    assert_eq!(texts, vec!["что такое питон", "Язык программирования", "а кто его создал"]);
}

// ---------- a stream that stays silent / a provider that does not know a parameter ----------
#[test]
fn a_silent_stream_is_asked_again_without_a_stream_and_remembered() {
    FIRST_FRAGMENT_MS.store(500, Ordering::SeqCst);
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Vec::<String>::new()));
    let s2 = seen.clone();
    std::thread::spawn(move || {
        let mut held: Vec<std::net::TcpStream> = Vec::new();
        while let Ok((mut stream, _)) = listener.accept() {
            let mut buf = vec![0u8; 16384];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            s2.lock().push(req.clone());
            if req.contains("\"stream\":true") {
                held.push(stream); // the first request: accepted and never answered
            } else {
                let body = ok_body("Ответ без потока");
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
            }
        }
    });
    let base = format!("http://127.0.0.1:{}", port);
    let store = leaked_store();
    let t = Instant::now();
    let out = send_with(store, conf(&base), None, "вопрос", false).unwrap();
    let id = out.chat_id.clone();
    let timing = out.handle.join().unwrap();
    assert!(t.elapsed() < Duration::from_secs(6), "the answer must not wait for the stalled stream: {:?}", t.elapsed());
    assert_eq!(messages(store, &id).last().unwrap().content, "Ответ без потока");
    assert!(timing.fallback && !timing.streamed, "{:?}", timing);
    // the next question does not even try a stream for a while
    let id2 = run(store, conf(&base), Some(&id), "второй").unwrap();
    assert_eq!(messages(store, &id2).last().unwrap().content, "Ответ без потока");
    let reqs = seen.lock().clone();
    assert_eq!(reqs.iter().filter(|r| r.contains("\"stream\":true")).count(), 1, "one failed stream attempt only");
    assert_eq!(reqs.iter().filter(|r| r.contains("\"stream\":false")).count(), 2);
    FIRST_FRAGMENT_MS.store(10_000, Ordering::SeqCst);
}

#[test]
fn gemini_is_asked_to_think_little_and_a_provider_that_refuses_the_parameter_is_asked_again_without_it() {
    let store = leaked_store();
    let m = mock(vec![(400, "{\"error\":{\"message\":\"Unknown field reasoning_effort\"}}".to_string(), 0), (200, ok_body("Готово"), 0)]);
    let c = Ok(ProviderConf { kind: Kind::OpenAiCompatible, name: "Google Gemini".into(), base: m.base.clone(), model: "gemini-flash-latest".into(), key: KEY.into(), route: String::new() });
    let id = run(store, c, None, "вопрос").unwrap();
    assert_eq!(messages(store, &id).last().unwrap().content, "Готово");
    let reqs = m.requests.lock().clone();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[0].contains("reasoning_effort"), "the first request carries the parameter");
    assert!(!reqs[1].contains("reasoning_effort"), "the second one does not");
    // other providers never get it
    let o = ProviderConf { kind: Kind::OpenAiCompatible, name: "OpenAI".into(), base: "http://x".into(), model: "m".into(), key: "k".into(), route: String::new() };
    assert!(build_body_opts(&o, &[], "s", true).get("reasoning_effort").is_none());
}

// an overloaded server (503/504) is asked again before an error is shown; a server that stays down gives the error after two more tries
#[test]
fn a_busy_server_is_asked_again_and_a_dead_one_gives_the_error_after_two_tries() {
    SERVER_RETRY_PAUSE_MS.store(20, Ordering::SeqCst);
    let store = leaked_store();
    let m = mock(vec![(503, "{\"error\":{\"message\":\"overloaded\"}}".to_string(), 0), (504, "".to_string(), 0), (200, ok_body("Ответил со второй попытки"), 0)]);
    let id = run(store, conf(&m.base), None, "вопрос").unwrap();
    assert_eq!(messages(store, &id).last().unwrap().content, "Ответил со второй попытки");
    assert_eq!(m.requests.lock().len(), 3);

    let m2 = mock(vec![(503, "".to_string(), 0), (503, "".to_string(), 0), (503, "".to_string(), 0)]);
    let id2 = run(store, conf(&m2.base), None, "ещё вопрос").unwrap();
    let last = messages(store, &id2).last().unwrap().clone();
    assert!(last.role == "error" && last.content.contains("503"), "after the tries the error is shown, as before: {:?}", last.content);
    assert_eq!(m2.requests.lock().len(), 3, "the first try and two more, not endless");
    SERVER_RETRY_PAUSE_MS.store(600, Ordering::SeqCst);
}

#[test]
fn the_host_order_goes_to_openrouter_only_and_only_as_plain_names() {
    assert_eq!(route_provider(""), None);
    assert_eq!(route_provider("baidu"), Some(json!({ "order": ["baidu"], "allow_fallbacks": true })));
    assert_eq!(route_provider("Baidu, DeepInfra; novita"), Some(json!({ "order": ["baidu", "deepinfra", "novita"], "allow_fallbacks": true })));
    assert_eq!(route_provider("sort:latency"), Some(json!({ "sort": "latency" })));
    assert_eq!(route_provider("sort:throughput"), Some(json!({ "sort": "throughput" })));
    assert_eq!(route_provider("sort:evil"), None, "only the known ways to choose");
    // nothing but plain host names reaches the request
    assert_eq!(route_provider("a\"b, {x}, ok"), Some(json!({ "order": ["ok"], "allow_fallbacks": true })));
    assert_eq!(route_provider("{\"x\":1}"), None);
    let many = (1..=20).map(|i| format!("h{}", i)).collect::<Vec<_>>().join(",");
    assert_eq!(route_provider(&many).unwrap()["order"].as_array().unwrap().len(), 8, "at most eight hosts");
    let or = ProviderConf { kind: Kind::OpenAiCompatible, name: "OpenRouter".into(), base: "https://openrouter.ai/api/v1".into(), model: "m".into(), key: "k".into(), route: "baidu".into() };
    let body = build_body_opts(&or, &[], "s", true);
    assert_eq!(body["provider"]["order"][0], "baidu");
    assert_eq!(body["provider"]["allow_fallbacks"], true);
    assert!(build_body_opts(&ProviderConf { route: String::new(), ..or.clone() }, &[], "s", true).get("provider").is_none(), "no order set: nothing is added");
    // a route that was set for another provider never goes out
    assert!(build_body_opts(&ProviderConf { name: "OpenAI".into(), base: "https://api.openai.com/v1".into(), ..or.clone() }, &[], "s", true).get("provider").is_none());
    assert!(build_body_opts(&ProviderConf { kind: Kind::Anthropic, name: "Anthropic".into(), ..or.clone() }, &[], "s", true).get("provider").is_none());
    // the key is not in the body
    assert!(!body.to_string().contains("\"k\""));
}

#[test]
fn the_route_of_the_settings_reaches_openrouter_only() {
    let _g = crate::uistate::TEST_LOCK.lock();
    crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_provider: "openrouter".into(), ai_key: "sk-or-v1-test".into(), ai_route: "baidu".into(), ..Default::default() });
    assert_eq!(conf_from_settings().unwrap().route, "baidu");
    crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_provider: "openai".into(), ai_key: "sk-test".into(), ai_route: "baidu".into(), ..Default::default() });
    assert_eq!(conf_from_settings().unwrap().route, "", "OpenAI never gets a host order");
    crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
}

#[test]
fn a_provider_that_refuses_the_host_order_is_asked_again_without_it() {
    let store = leaked_store();
    let m = mock(vec![(400, "{\"error\":{\"message\":\"Unknown provider: baiduu\"}}".to_string(), 0), (200, ok_body("Готово"), 0)]);
    let c = Ok(ProviderConf { kind: Kind::OpenAiCompatible, name: "OpenRouter".into(), base: m.base.clone(), model: "m".into(), key: KEY.into(), route: "baiduu".into() });
    let id = run(store, c, None, "вопрос").unwrap();
    assert_eq!(messages(store, &id).last().unwrap().content, "Готово");
    let reqs = m.requests.lock().clone();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[0].contains("\"provider\"") && reqs[0].contains("baiduu"), "the first request carries the order");
    assert!(!reqs[1].contains("\"provider\""), "the second one does not");
}

#[test]
fn with_the_ai_switched_off_nothing_is_sent_or_stored() {
    let _g = crate::uistate::TEST_LOCK.lock();
    crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_enabled: false, ai_provider: "gemini".into(), ai_key: "k".into(), ..Default::default() });
    let dir = tempfile::tempdir().unwrap();
    let store: &'static Store = Box::leak(Box::new(Store::new(dir.path().to_path_buf())));
    assert_eq!(conf_from_settings().unwrap_err(), AiError::Disabled);
    let r = send_with(store, conf_from_settings(), None, "что такое python", true);
    assert_eq!(r.err(), Some(AiError::Disabled));
    assert!(store.list().iter().all(|c| c["count"].as_u64().unwrap_or(0) == 0), "the question is not stored either");
    assert!(AiError::Disabled.user_text().contains("Приватность"));
    assert_eq!(status_json()["state"], "DISCONNECTED");
    crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
}

#[test]
fn deleting_the_whole_history_removes_every_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let a = store.create();
    store.append(&a.id, "user", "раз");
    let b = store.create();
    store.append(&b.id, "user", "два");
    assert_eq!(store.stats()["chats"], 2);
    assert_eq!(store.stats()["messages"], 2);
    assert_eq!(store.delete_all(), (2, 0));
    assert!(store.list().is_empty());
    assert!(store.active().is_none());
}

#[test]
fn with_the_ai_switched_off_no_path_opens_a_connection_or_a_web_search() {
    let _g = crate::uistate::TEST_LOCK.lock();
    // a "provider" on this computer that counts every connection made to it
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://127.0.0.1:{}/v1", listener.local_addr().unwrap().port());
    crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_enabled: false, ai_provider: "custom".into(), ai_model: "m".into(), ai_url: url, ..Default::default() });

    // every entry that leads to the provider: the window chat, a voice/typed question, the connection check
    assert_eq!(send(None, "что такое python", false).unwrap_err(), AiError::Disabled);
    let hook_called = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = hook_called.clone();
    let r = send_question("объясни, как работает интернет", Box::new(move |_| flag.store(true, std::sync::atomic::Ordering::SeqCst)));
    assert_eq!(r.unwrap_err(), AiError::Disabled);
    assert!(conf_from_settings().is_err(), "the connection check gets no provider either");
    assert!(!is_configured());
    // the router never sends a question to the AI, and the refusal never allows the web search of plan B
    assert_ne!(crate::route::decide("что такое python", is_configured()), crate::route::Route::Ai);
    assert!(!AiError::Disabled.allows_web_search());
    std::thread::sleep(std::time::Duration::from_millis(300));
    assert!(!hook_called.load(std::sync::atomic::Ordering::SeqCst), "the error hook (which starts the web search) is not called");
    assert!(listener.accept().is_err(), "no connection was made to the provider");
    crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
}

#[test]
fn switching_the_ai_off_stops_the_answer_that_is_being_generated() {
    let _g = crate::uistate::TEST_LOCK.lock();
    crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
    let chat = store().create();
    store().set_active(&chat.id);
    let cancel = Arc::new(AtomicBool::new(false));
    JOBS.lock().insert(chat.id.clone(), cancel.clone());
    crate::uistate::update_settings(&json!({ "ai_enabled": false }));
    assert!(cancel.load(Ordering::SeqCst), "the running generation is cancelled at once");
    JOBS.lock().remove(&chat.id);
    crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
}

#[test]
fn a_picture_of_the_screen_needs_the_consent_and_the_ai_switched_on() {
    let _g = crate::uistate::TEST_LOCK.lock();
    let base = crate::uistate::UiSettings { ai_provider: "gemini".into(), ai_key: "k".into(), ..Default::default() };
    crate::uistate::set_for_tests(base.clone());
    assert_eq!(screen_conf().unwrap_err(), AiError::ScreenNotAllowed, "off by default: nothing is captured or sent");
    assert!(!crate::uistate::UiSettings::default().ai_screen);
    crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_screen: true, ai_enabled: false, ..base.clone() });
    assert_eq!(screen_conf().unwrap_err(), AiError::Disabled, "the AI switch wins");
    assert_eq!(ask_about_screen("что на экране").unwrap_err(), AiError::Disabled, "refused before any picture is taken");
    crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_screen: true, ..base });
    assert!(screen_conf().is_ok());
    crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
}

#[test]
fn the_picture_is_sent_in_the_format_of_each_provider() {
    assert_eq!(base64(b"Man"), "TWFu");
    assert_eq!(base64(b"Ma"), "TWE=");
    assert_eq!(base64(b"M"), "TQ==");
    assert_eq!(base64(&[0xFF, 0xD8, 0xFF]), "/9j/");
    let oa = conf_from("openrouter", "google/gemini-2.5-flash-lite", "k", "").unwrap();
    let b = vision_body(&oa, "что тут?", "image/jpeg", "AAAA");
    assert_eq!(b["messages"][1]["content"][1]["image_url"]["url"], "data:image/jpeg;base64,AAAA");
    assert_eq!(b["messages"][1]["content"][0]["text"], "что тут?");
    assert!(!b.to_string().contains("\"k\""), "the key is never in the body");
    let an = conf_from("anthropic", "claude-sonnet-5-5", "k", "").unwrap();
    let b = vision_body(&an, "что тут?", "image/jpeg", "AAAA");
    assert_eq!(b["messages"][0]["content"][0]["source"]["data"], "AAAA");
    assert_eq!(b["messages"][0]["content"][0]["source"]["media_type"], "image/jpeg");
}

// a picture attached in the chat: sent once with its question, never stored, only with the AI switched on
#[test]
fn a_picture_question_sends_the_picture_and_keeps_only_the_text() {
    let m = mock(vec![(200, ok_body("На картинке кот."), 0)]);
    let store = leaked_store();
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    let (id, h) = ask_about_image_with(store, conf(&m.base), None, "Кто это?", "кот.png", "image/png", png.to_string()).unwrap();
    h.join().unwrap();
    let msgs = messages(store, &id);
    assert_eq!(msgs.len(), 2);
    assert!(msgs[0].content.contains("кот.png") && msgs[0].content.contains("Кто это?"));
    assert!(!msgs.iter().any(|x| x.content.contains(png)), "the picture is not kept in the history");
    assert_eq!(msgs[1].content, "На картинке кот.");
    let req = m.requests.lock()[0].clone();
    assert!(req.contains("data:image/png;base64,") && req.contains("Кто это?"), "{}", &req[..req.len().min(400)]);

    // checked before anything is sent or stored
    let store2 = leaked_store();
    assert!(matches!(ask_about_image_with(store2, Err(AiError::Disabled), None, "x", "a.png", "image/png", png.into()), Err(AiError::Disabled)));
    assert!(ask_about_image_with(store2, conf(&m.base), None, "x", "a.exe", "application/x-msdownload", png.into()).is_err());
    assert!(ask_about_image_with(store2, conf(&m.base), None, "x", "a.png", "image/png", "not base64 !!".into()).is_err());
    assert!(check_image("image/png", &"A".repeat(MAX_IMAGE_BYTES / 3 * 4 + 8)).is_err(), "too big");
    assert!(store2.list().is_empty(), "nothing stored for a refused picture");
}

#[test]
fn a_picture_question_reports_a_provider_error_in_the_conversation() {
    let m = mock(vec![(400, json!({"error": {"message": "image input not supported"}}).to_string(), 0)]);
    let store = leaked_store();
    let (id, h) = ask_about_image_with(store, conf(&m.base), None, "", "a.jpg", "image/jpeg", "AAAA".into()).unwrap();
    h.join().unwrap();
    let msgs = messages(store, &id);
    assert!(msgs[0].content.contains("Что на этой картинке?"), "the default question");
    assert_eq!(msgs[1].role, "error");
    assert!(msgs[1].content.contains("картинки"), "{}", msgs[1].content);
    assert!(!is_busy(&id));
}


#[test]
fn only_a_provider_on_this_computer_or_the_home_network_counts_as_local() {
    for local in ["http://localhost:11434/v1", "http://127.0.0.1:1234", "http://192.168.1.20:8080/v1", "http://10.0.0.5", "http://172.16.3.4/v1", "http://[::1]:11434"] {
        assert!(is_local(local), "{}", local);
    }
    // a piece of text in the address is not the host: these are on the Internet (no key check skipped)
    for remote in ["http://10.evil.com/v1", "http://localhost.evil.com", "https://192.168.example.org", "https://api.openai.com/v1", "http://user@8.8.8.8/v1"] {
        assert!(!is_local(remote), "{}", remote);
    }
}

#[test]
fn a_deleted_dialog_goes_to_the_archive_with_its_messages_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let a = store.create();
    store.append(&a.id, "user", "первый вопрос про синего слона");
    store.append(&a.id, "assistant", "ответ про слона");
    store.append(&a.id, "user", "второй вопрос");
    store.archive(&a.id).unwrap();
    assert!(store.load(&a.id).is_none(), "gone from the list of dialogs");
    assert!(store.list().is_empty());
    let got = store.archive_get(&a.id).unwrap();
    let msgs = got["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[0]["content"], "первый вопрос про синего слона");
    assert_eq!(msgs[2]["content"], "второй вопрос");
    let text = got["text"].as_str().unwrap();
    assert!(text.find("первый вопрос").unwrap() < text.find("второй вопрос").unwrap());
    assert!(text.contains("Jarvis: ответ про слона"));
    assert!(store.archive_path(&a.id, "txt").unwrap().exists(), "a readable text copy is kept");
    // survives a restart
    let reopened = Store::new(dir.path().to_path_buf());
    assert_eq!(reopened.archive_list("").len(), 1);
    assert_eq!(reopened.stats()["archived"], 1);
}

#[test]
fn the_archive_is_searched_in_titles_and_texts_and_an_empty_dialog_is_not_archived() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let a = store.create();
    store.append(&a.id, "user", "рецепт борща");
    let b = store.create();
    store.append(&b.id, "user", "привет");
    store.append(&b.id, "assistant", "Ответ про Погоду");
    let c = store.create();
    store.archive(&a.id).unwrap();
    store.archive(&b.id).unwrap();
    store.archive(&c.id).unwrap();
    assert_eq!(store.archive_list("").len(), 2, "an empty dialog has nothing to keep");
    assert!(store.load(&c.id).is_none());
    assert_eq!(store.archive_list("БОРЩ").len(), 1);
    assert_eq!(store.archive_list("погоду")[0]["id"], b.id);
    assert!(store.archive_list("нет такого").is_empty());
}

#[test]
fn restore_brings_the_dialog_back_once_and_repeated_actions_are_harmless() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let a = store.create();
    store.append(&a.id, "user", "один");
    store.append(&a.id, "assistant", "два");
    store.archive(&a.id).unwrap();
    assert!(store.archive(&a.id).is_err(), "a second archive of the same dialog finds nothing");
    store.archive_restore(&a.id).unwrap();
    assert_eq!(store.load(&a.id).unwrap().messages.len(), 2);
    assert!(store.archive_list("").is_empty(), "no copy stays in the archive");
    assert_eq!(store.list().len(), 1, "no duplicate");
    assert!(store.archive_restore(&a.id).is_err(), "a second restore finds nothing");
    // the same id in both places (a crash between the steps): the dialog is not doubled or overwritten
    store.archive(&a.id).unwrap();
    let again = Chat { id: a.id.clone(), title: "новее".into(), created: 1, updated: 2, messages: vec![ChatMsg { role: "user".into(), content: "новее".into(), ..Default::default() }] };
    store.save(&again);
    store.archive_restore(&a.id).unwrap();
    assert_eq!(store.load(&a.id).unwrap().messages[0].content, "новее");
    assert!(store.archive_list("").is_empty());
}

#[test]
fn deleting_from_the_archive_and_clearing_it_remove_only_the_archive() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let keep = store.create();
    store.append(&keep.id, "user", "живой диалог");
    let mut ids = Vec::new();
    for i in 0..3 {
        let c = store.create();
        store.append(&c.id, "user", &format!("старый {}", i));
        store.archive(&c.id).unwrap();
        ids.push(c.id);
    }
    assert!(store.archive_delete(&ids[0]));
    assert!(!store.archive_delete(&ids[0]), "already gone");
    assert!(!store.archive_path(&ids[0], "txt").unwrap().exists());
    assert_eq!(store.archive_list("").len(), 2);
    assert_eq!(store.archive_clear(), 2);
    assert!(store.archive_list("").is_empty());
    assert_eq!(store.load(&keep.id).unwrap().messages.len(), 1, "the live dialogs are untouched");
    // path tricks
    assert!(store.archive_get("../x").is_none());
    assert!(store.archive_restore("../x").is_err());
    assert!(!store.archive_delete("../x"));
}

#[test]
fn a_storage_error_never_loses_the_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let a = store.create();
    store.append(&a.id, "user", "важное");
    // a FILE where the archive folder must be: the archive cannot be written
    std::fs::write(dir.path().join("archive"), "x").unwrap();
    assert!(store.archive(&a.id).is_err());
    assert_eq!(store.load(&a.id).unwrap().messages[0].content, "важное", "the dialog stays where it was");
    // a damaged archive file is skipped, the others are listed
    std::fs::remove_file(dir.path().join("archive")).unwrap();
    let b = store.create();
    store.append(&b.id, "user", "второй");
    store.archive(&b.id).unwrap();
    std::fs::write(dir.path().join("archive").join("cbroken.json"), "{not json").unwrap();
    assert_eq!(store.archive_list("").len(), 1);
    assert_eq!(store.archive_clear(), 1);
}

#[test]
fn archive_all_keeps_the_busy_dialog() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    for i in 0..3 {
        let c = store.create();
        store.append(&c.id, "user", &format!("диалог {}", i));
    }
    let (done, kept) = store.archive_all();
    assert_eq!((done, kept), (3, 0));
    assert!(store.list().is_empty());
    assert_eq!(store.archive_list("").len(), 3);
}

// ---------------------------------------------------------------------------------------------
// speed of the answer: a generating mock server (NOT a real provider; it only shows the mechanisms: the size of the
// request, the cap of the answer, the time to the first fragment and to the end)
// ---------------------------------------------------------------------------------------------
mod speed {
    use super::*;

    // the language of the messages depends on the settings of the machine that runs the test
    trait ContainsAny {
        fn contains_any(&self, words: &[&str]) -> bool;
    }
    impl ContainsAny for str {
        fn contains_any(&self, words: &[&str]) -> bool {
            words.iter().any(|w| self.contains(w))
        }
    }

    // One word is one "token". `full` words are generated unless the request asks for fewer (max_tokens or
    // max_completion_tokens); a capped answer ends with finish_reason "length". Sentences have 8 words.
    pub struct GenMock {
        pub base: String,
        pub bodies: Arc<Mutex<Vec<String>>>,
    }

    pub fn word(i: usize, sentence: usize) -> String {
        format!("слово{}{} ", i, if i % sentence == 0 { "." } else { "" })
    }

    pub fn gen_mock(connections: usize, full: usize, first_ms: u64, per_token_ms: u64, sentence: usize) -> GenMock {
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        std::env::set_var("no_proxy", "127.0.0.1,localhost");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let seen = bodies.clone();
        std::thread::spawn(move || {
            for _ in 0..connections {
                let Ok((mut stream, _)) = listener.accept() else { return };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let (mut header_end, mut want) = (None, 0usize);
                loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if header_end.is_none() {
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            header_end = Some(p + 4);
                            let head = String::from_utf8_lossy(&buf[..p]).to_lowercase();
                            want = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        }
                    }
                    if let Some(h) = header_end {
                        if buf.len() >= h + want {
                            break;
                        }
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let body_text = text.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
                seen.lock().push(body_text.clone());
                let v: Value = serde_json::from_str(&body_text).unwrap_or(Value::Null);
                let cap = v.get("max_tokens").or_else(|| v.get("max_completion_tokens")).and_then(|c| c.as_u64()).map(|c| c as usize);
                let n = cap.map(|c| c.min(full)).unwrap_or(full);
                let capped = n < full;
                std::thread::sleep(Duration::from_millis(first_ms));
                if v.get("stream").and_then(|s| s.as_bool()).unwrap_or(false) {
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n");
                    for i in 1..=n {
                        if stream.write_all(sse_chunk(&word(i, sentence)).as_bytes()).is_err() {
                            break;
                        }
                        let _ = stream.flush();
                        std::thread::sleep(Duration::from_millis(per_token_ms));
                    }
                    let fin = json!({ "choices": [{ "delta": {}, "finish_reason": if capped { "length" } else { "stop" } }] });
                    let _ = write!(stream, "data: {}\n\ndata: [DONE]\n\n", fin);
                } else {
                    std::thread::sleep(Duration::from_millis(per_token_ms * n as u64));
                    let answer: String = (1..=n).map(|i| word(i, sentence)).collect::<String>().trim().to_string();
                    let body = json!({ "choices": [{ "message": { "role": "assistant", "content": answer }, "finish_reason": if capped { "length" } else { "stop" } }] }).to_string();
                    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                }
            }
        });
        GenMock { base: format!("http://127.0.0.1:{}", port), bodies }
    }

    // a long earlier dialog: `pairs` questions with answers of about 300 characters each
    pub fn long_dialog(store: &'static Store, pairs: usize) -> String {
        let chat = store.active_or_new();
        for i in 0..pairs {
            store.append_with(&chat.id, "user", &format!("Вопрос номер {} про разные вещи. {}", i, "подробность ".repeat(8)), &new_rid(), "ai");
            store.append_with(&chat.id, "assistant", &format!("Ответ номер {}. {}", i, "объяснение ".repeat(26)), &new_rid(), "ai");
        }
        chat.id
    }

    fn cap_in(body: &str) -> Option<(String, u64)> {
        let v: Value = serde_json::from_str(body).ok()?;
        for k in ["max_tokens", "max_completion_tokens"] {
            if let Some(n) = v.get(k).and_then(|n| n.as_u64()) {
                return Some((k.to_string(), n));
            }
        }
        None
    }

    fn system_of(body: &str) -> String {
        let v: Value = serde_json::from_str(body).unwrap();
        v["messages"][0]["content"].as_str().unwrap_or("").to_string()
    }

    #[test]
    fn a_short_spoken_question_gets_a_brief_answer_and_a_wordy_model_is_stopped_at_a_whole_sentence() {
        let _g = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(true);
        let m = gen_mock(1, 700, 20, 1, 8);
        let store = leaked_store();
        let before = crate::tts::TEST_SPOKEN.lock().len();
        let out = send_with(store, conf(&m.base), None, "что такое питон", true).unwrap();
        let id = out.chat_id.clone();
        let t = out.handle.join().unwrap();
        let body = m.bodies.lock()[0].clone();
        assert_eq!(cap_in(&body), Some(("max_tokens".to_string(), 500)), "{}", body);
        assert!(system_of(&body).contains_any(&["вслух", "aloud"]), "the model is asked to be brief: {}", system_of(&body));
        let answer = messages(store, &id).last().unwrap().content.clone();
        assert!(answer.contains("слово496.") && !answer.contains("слово497"), "cut at the last whole sentence: {}", &answer[answer.len().saturating_sub(160)..]);
        assert!(answer.contains_any(&["сокращён", "Shortened"]) && answer.contains_any(&["подробнее", "more details"]), "and it says so: {}", answer);
        assert!(t.shortened && t.limit_tokens == Some(500) && !t.retried_without_limit, "{:?}", t);
        let spoken = crate::tts::TEST_SPOKEN.lock()[before..].join(" ");
        assert!(!spoken.contains("слово497") && !spoken.contains("слово500"), "the cut-off end is not read aloud");
    }

    #[test]
    fn typed_and_detailed_questions_are_never_capped() {
        let _g = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(true);
        for (label, question, spoken) in [("typed", "что такое питон", false), ("asks for details", "расскажи подробно про питон", true), ("asks for code", "напиши функцию сортировки", true), ("long question", "а вот скажи мне пожалуйста вот такую вещь которая долго объясняется как мне быть если у меня есть много разных дел на этой неделе и я не успеваю ничего сделать совсем", true)] {
            let m = gen_mock(1, 700, 10, 0, 8);
            let store = leaked_store();
            let out = send_with(store, conf(&m.base), None, question, spoken).unwrap();
            let id = out.chat_id.clone();
            let t = out.handle.join().unwrap();
            let body = m.bodies.lock()[0].clone();
            assert_eq!(cap_in(&body), None, "{}: no cap in {}", label, body);
            assert!(!system_of(&body).contains_any(&["вслух", "aloud"]), "{}: no request for brevity", label);
            let answer = messages(store, &id).last().unwrap().content.clone();
            assert!(answer.contains("слово700") && !answer.contains_any(&["сокращён", "Shortened"]), "{}: the whole answer, nothing cut", label);
            assert!(!t.shortened && t.limit_tokens.is_none(), "{}: {:?}", label, t);
        }
    }

    #[test]
    fn with_the_voice_switched_off_the_answer_is_a_text_answer_without_a_cap() {
        let _g = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(false);
        let m = gen_mock(1, 700, 10, 0, 8);
        let store = leaked_store();
        let out = send_with(store, conf(&m.base), None, "что такое питон", true).unwrap();
        let id = out.chat_id.clone();
        let t = out.handle.join().unwrap();
        crate::tts::set_enabled(true);
        let body = m.bodies.lock()[0].clone();
        assert_eq!(cap_in(&body), None, "nobody reads it aloud: no cap in {}", body);
        assert!(!system_of(&body).contains_any(&["вслух", "aloud"]));
        assert!(messages(store, &id).last().unwrap().content.contains("слово700") && !t.shortened);
    }

    #[test]
    fn a_cap_that_is_too_tight_is_dropped_and_the_question_is_asked_again() {
        let _g = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(true);
        // a model whose answer has no whole sentence in the first 500 tokens (it thinks aloud, one long run)
        let m = gen_mock(2, 700, 10, 0, 100_000);
        let store = leaked_store();
        let out = send_with(store, conf(&m.base), None, "что такое питон", true).unwrap();
        let id = out.chat_id.clone();
        let t = out.handle.join().unwrap();
        let bodies = m.bodies.lock().clone();
        assert_eq!(bodies.len(), 2, "asked twice");
        assert!(cap_in(&bodies[0]).is_some() && cap_in(&bodies[1]).is_none(), "the second time without the cap");
        assert!(system_of(&bodies[0]).contains_any(&["вслух", "aloud"]) && !system_of(&bodies[1]).contains_any(&["вслух", "aloud"]));
        let msgs = messages(store, &id);
        let answers: Vec<&ChatMsg> = msgs.iter().filter(|x| x.role == "assistant").collect();
        assert_eq!(answers.len(), 1, "one answer, not two");
        assert!(answers[0].content.contains("слово700") && !answers[0].content.contains_any(&["сокращён", "Shortened"]), "the whole answer");
        assert!(t.retried_without_limit && !t.shortened, "{:?}", t);
    }

    #[test]
    fn a_capped_answer_that_comes_in_one_piece_is_cut_the_same_way() {
        let _g = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(true);
        let m = gen_mock(1, 700, 10, 0, 8);
        STREAM_BROKEN.lock().insert(m.base.clone(), std::time::Instant::now());
        let store = leaked_store();
        let out = send_with(store, conf(&m.base), None, "что такое питон", true).unwrap();
        let id = out.chat_id.clone();
        let t = out.handle.join().unwrap();
        let body = m.bodies.lock()[0].clone();
        assert!(body.contains("\"stream\":false") && cap_in(&body).is_some(), "{}", body);
        let answer = messages(store, &id).last().unwrap().content.clone();
        assert!(answer.contains("слово496.") && !answer.contains("слово497") && answer.contains_any(&["сокращён", "Shortened"]), "{}", &answer[answer.len().saturating_sub(120)..]);
        assert!(t.shortened && !t.streamed, "{:?}", t);
    }

    #[test]
    fn the_cap_uses_the_parameter_of_each_api_and_none_where_it_is_not_known_to_be_safe() {
        let c = |kind: Kind, name: &str, base: &str, model: &str| ProviderConf { kind, name: name.into(), base: base.into(), model: model.into(), key: "k".into(), route: String::new() };
        assert_eq!(voice_cap(&c(Kind::Anthropic, "Anthropic", "https://api.anthropic.com", "claude-sonnet-5-5")), Some(Cap { tokens: 500, param: "max_tokens" }));
        assert_eq!(voice_cap(&c(Kind::OpenAiCompatible, "OpenAI", "https://api.openai.com/v1", "gpt-4o-mini")), Some(Cap { tokens: 500, param: "max_completion_tokens" }));
        assert_eq!(voice_cap(&c(Kind::OpenAiCompatible, "OpenRouter", "https://openrouter.ai/api/v1", "google/gemini-2.5-flash-lite")), Some(Cap { tokens: 500, param: "max_tokens" }));
        assert_eq!(voice_cap(&c(Kind::OpenAiCompatible, "Ollama", "http://127.0.0.1:11434/v1", "llama3.1")), Some(Cap { tokens: 500, param: "max_tokens" }));
        // Gemini counts its thinking in the same limit: room for it
        assert_eq!(voice_cap(&c(Kind::OpenAiCompatible, "Google Gemini", "https://generativelanguage.googleapis.com/v1beta/openai", "gemini-flash-latest")), Some(Cap { tokens: 1524, param: "max_tokens" }));
        // models that think: their thinking is in the limit and its size is not known - no cap
        for model in ["gpt-5", "gpt-6.1-sol", "gpt-6-luna", "gpt-6-astra", "o3-mini", "openai/o4-mini", "deepseek/deepseek-r1", "qwen3:8b", "some-thinking-model"] {
            assert_eq!(voice_cap(&c(Kind::OpenAiCompatible, "OpenRouter", "https://openrouter.ai/api/v1", model)), None, "{}", model);
        }
        // the body
        let o = c(Kind::OpenAiCompatible, "OpenAI", "https://api.openai.com/v1", "gpt-4o-mini");
        let b = build_body_limit(&o, &[], "s", true, voice_cap(&o));
        assert_eq!(b["max_completion_tokens"], 500);
        assert!(b.get("max_tokens").is_none(), "only the parameter this API knows");
        assert!(build_body_opts(&o, &[], "s", true).get("max_completion_tokens").is_none(), "the usual body has no cap");
        let a = c(Kind::Anthropic, "Anthropic", "https://api.anthropic.com", "m");
        assert_eq!(build_body_limit(&a, &[], "s", false, voice_cap(&a))["max_tokens"], 500);
        assert_eq!(build_body_opts(&a, &[], "s", false)["max_tokens"], 2048, "Anthropic needs a number: the old one when nothing is capped");
    }

    #[test]
    fn what_asks_for_a_long_answer_and_where_a_sentence_ends() {
        assert!(short_voice_answer("что такое питон", true));
        assert!(!short_voice_answer("что такое питон", false), "a typed question is not capped");
        for q in ["расскажи подробно про питон", "подробнее", "дай пошаговую инструкцию", "напиши скрипт", "переведи текст", "составь список дел", "explain in detail", "step by step please", "write a script"] {
            assert!(wants_detail(q), "{}", q);
            assert!(!short_voice_answer(q, true), "{}", q);
        }
        assert!(!short_voice_answer(&"слово ".repeat(31), true), "a long question is not capped");
        assert_eq!(cut_to_sentence("Первое предложение достаточно длинное для проверки. Второе предложение тоже длинное и полное. Третье обор"), Some("Первое предложение достаточно длинное для проверки. Второе предложение тоже длинное и полное.".to_string()));
        assert_eq!(cut_to_sentence("Совсем без конца предложения и без точки внутри текста вообще никакой"), None);
        assert_eq!(cut_to_sentence("Коротко. Мало."), None, "less than 60 characters: not an answer");
        assert_eq!(cut_to_sentence(""), None);
        let mut sp = SentenceSpeaker::new();
        let mut said: Vec<String> = sp.push("Это первое законченное предложение для озвучки здесь. Это второе, тоже законченное, предложение. Это начало третьего и оно об");
        said.extend(sp.finish_complete());
        let said = said.join(" ");
        assert!(said.contains("второе") && !said.contains("третьего"), "{}", said);
    }

    #[test]
    fn the_context_is_short_but_keeps_the_question_and_the_exchange_before_it() {
        let mut msgs = Vec::new();
        for i in 0..60 {
            msgs.push(ChatMsg { role: if i % 2 == 0 { "user" } else { "assistant" }.into(), content: format!("сообщение номер {} {}", i, "текст ".repeat(40)), ts: 0, ..Default::default() });
        }
        msgs.push(ChatMsg { role: "user".into(), content: "а что дальше".into(), ts: 0, ..Default::default() });
        let ctx = context_for_model(&msgs);
        assert!(ctx.len() <= MAX_CONTEXT_MESSAGES, "{}", ctx.len());
        assert!(ctx.iter().map(|m| m.content.chars().count()).sum::<usize>() <= MAX_CONTEXT_CHARS + 300);
        assert_eq!(ctx.first().unwrap().role, "user");
        assert_eq!(ctx.last().unwrap().content, "а что дальше");
        // the step before: a pasted text and its answer are still there, whatever their size
        let big = vec![
            ChatMsg { role: "user".into(), content: "x".repeat(7000), ts: 0, ..Default::default() },
            ChatMsg { role: "assistant".into(), content: "y".repeat(7000), ts: 0, ..Default::default() },
            ChatMsg { role: "user".into(), content: "сократи это".into(), ts: 0, ..Default::default() },
        ];
        let ctx = context_for_model(&big);
        assert_eq!(ctx.len(), 3, "the pasted text, its answer and the question: the question about it keeps its meaning");
        // older messages of a long talk are dropped when the budget is over
        let mut older = vec![ChatMsg { role: "user".into(), content: "z".repeat(5000), ts: 0, ..Default::default() }, ChatMsg { role: "assistant".into(), content: "w".repeat(5000), ts: 0, ..Default::default() }];
        older.extend(big.clone());
        let ctx = context_for_model(&older);
        assert_eq!(ctx.len(), 3, "the exchange before the last one is dropped, it does not fit");
        // nothing but the dialog: commands and errors never go to the model
        assert!(context_for_model(&[ChatMsg { role: "user".into(), content: "открой ворд".into(), src: "cmd".into(), ts: 0, ..Default::default() }]).is_empty());
    }

    #[test]
    fn measure_the_speed_on_the_mock() {
        let _g = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(true);
        // (label, question, spoken, words the model would write, words already in the dialog as pairs)
        let cases: [(&str, &str, bool, usize, usize); 5] = [
            ("short voice question, short answer", "сколько будет два плюс два", true, 40, 0),
            ("short voice question, wordy model", "что такое питон", true, 700, 0),
            ("voice, long dialog before", "а что дальше", true, 40, 30),
            ("asks for details by voice", "расскажи подробно про питон", true, 700, 0),
            ("typed question, wordy model", "что такое питон", false, 700, 0),
        ];
        for (label, question, spoken, words, pairs) in cases {
            let m = gen_mock(1, words, 150, 6, 8);
            let store = leaked_store();
            let chat = if pairs > 0 { Some(long_dialog(store, pairs)) } else { None };
            let out = send_with(store, conf(&m.base), chat.as_deref(), question, spoken).unwrap();
            let id = out.chat_id.clone();
            let t = out.handle.join().unwrap();
            let body = m.bodies.lock()[0].clone();
            let answer = messages(store, &id).iter().filter(|x| x.role == "assistant").last().map(|x| x.content.clone()).unwrap_or_default();
            let sent: usize = serde_json::from_str::<Value>(&body).map(|v| v["messages"].as_array().map(|a| a.len()).unwrap_or(0)).unwrap_or(0);
            eprintln!(
                "MEASURE[mock] {:<38} first fragment {:>4?} ms | total {:>5} ms | answer {:>5} chars | request {:>6} bytes, {:>2} messages | cap in request: {}",
                label, t.first_chunk_ms, t.total_ms, answer.chars().count(), body.len(), sent,
                if body.contains("max_tokens") { "max_tokens" } else if body.contains("max_completion_tokens") { "max_completion_tokens" } else { "none" }
            );
        }
    }
}

// ---- the manner of the answers ----
#[test]
fn mode_phrases_switch_the_manner_and_other_phrases_do_not() {
    assert_eq!(super::mode_phrase("режим психолога").map(|m| m.0), Some("psychologist"));
    assert_eq!(super::mode_phrase("включи режим психолога").map(|m| m.0), Some("psychologist"));
    assert_eq!(super::mode_phrase("включи режим учителя").map(|m| m.0), Some("teacher"));
    assert_eq!(super::mode_phrase("режим программиста").map(|m| m.0), Some("coder"));
    assert_eq!(super::mode_phrase("обычный режим").map(|m| m.0), Some(""));
    assert_eq!(super::mode_phrase("выключи режим психолога").map(|m| m.0), Some(""));
    // not a mode phrase: a command about the game mode, a long sentence, a question
    assert_eq!(super::mode_phrase("включи игровой режим"), None);
    assert_eq!(super::mode_phrase("расскажи мне про психолога и про режим дня на работе"), None);
    assert_eq!(super::mode_phrase("что такое режим сна"), None);
}

#[test]
fn a_mode_changes_only_the_prompt_and_an_unknown_mode_is_the_usual_manner() {
    assert_eq!(super::mode_id("psychologist"), "psychologist");
    assert_eq!(super::mode_id("hacker"), "");
    assert_eq!(super::mode_id(""), "");
    let usual = super::system_prompt();
    crate::uistate::update_settings(&serde_json::json!({ "ai_mode": "psychologist" }));
    let psych = super::system_prompt();
    crate::uistate::update_settings(&serde_json::json!({ "ai_mode": "" }));
    assert!(psych.starts_with(&usual) && psych.len() > usual.len() && psych.contains("психолога"));
    assert!(psych.contains("не ставь диагнозов") || psych.contains("Не ставь диагнозов"), "the mode keeps the safe limits");
    assert_eq!(super::system_prompt(), usual);
}

// ---- Perplexity Sonar: fresh answers from the internet ----
#[test]
fn perplexity_is_a_provider_with_a_default_model_and_needs_a_key() {
    assert_eq!(conf_from("perplexity", "", "", "").unwrap_err(), AiError::NoKey("Perplexity".into()));
    let c = conf_from("perplexity", "", "pplx-test", "").unwrap();
    assert_eq!(c.base, "https://api.perplexity.ai");
    assert_eq!(c.model, "sonar");
    assert_eq!(conf_from("perplexity", "sonar-pro", "pplx-test", "").unwrap().model, "sonar-pro");
    // the key is sent only to its own host (a custom address in the settings does not redirect it)
    assert_eq!(conf_from("perplexity", "", "pplx-test", "https://evil.example.com").unwrap().base, "https://api.perplexity.ai");
}

#[test]
fn the_numbers_of_sources_are_removed_from_the_answer_only_for_perplexity() {
    assert_eq!(strip_citations("Северный Арал частично восстановили в 2005 году[1][3]. Это помогло [2], но не всем."), "Северный Арал частично восстановили в 2005 году. Это помогло, но не всем.");
    assert_eq!(strip_citations("список [x] и `a[0]` остаются, [12345] тоже, а [7] нет"), "список [x] и `a[0]` остаются, [12345] тоже, а нет");
    let body = serde_json::json!({ "choices": [{ "message": { "content": "Да[1]. Нет[2]." } }] });
    let p = conf_from("perplexity", "", "k", "").unwrap();
    assert_eq!(parse_answer(&p, &body).unwrap(), "Да. Нет.");
    let o = conf_from("openai", "gpt", "k", "").unwrap();
    assert_eq!(parse_answer(&o, &body).unwrap(), "Да[1]. Нет[2].");
}

// ---- the automatic choice: fresh questions to Perplexity, the rest to the usual provider ----
#[test]
fn which_questions_need_fresh_facts() {
    for q in ["какие сегодня новости", "сколько сейчас стоит биткоин", "какой курс доллара", "кто победил вчера в матче", "что произошло в 2026 году", "найди в интернете расписание поездов", "когда выйдет новая версия"] {
        assert!(needs_fresh(q), "{}", q);
    }
    for q in ["объясни, что такое Python", "как зовут столицу Франции", "переведи привет на английский", "перемести курсор влево", "расскажи сказку", "реши уравнение x плюс 2 равно 5"] {
        assert!(!needs_fresh(q), "{}", q);
    }
}

#[test]
fn a_fresh_question_goes_to_perplexity_and_the_others_to_the_usual_provider() {
    crate::uistate::update_settings(&serde_json::json!({ "ai_enabled": true, "ai_provider": "openai", "ai_model": "gpt-4o-mini", "ai_key": "sk-main", "ai_search_key": "pplx-test", "ai_search_auto": true }));
    assert_eq!(conf_for("какие сегодня новости").unwrap().name, "Perplexity");
    assert_eq!(conf_for("объясни, что такое Python").unwrap().name, "OpenAI");
    // the automatic choice is switched off: the search key stays, everything goes to the usual provider
    crate::uistate::update_settings(&serde_json::json!({ "ai_search_auto": false }));
    assert_eq!(conf_for("какие сегодня новости").unwrap().name, "OpenAI");
    // no usual provider: Perplexity answers everything
    crate::uistate::update_settings(&serde_json::json!({ "ai_search_auto": true, "ai_provider": "none" }));
    assert_eq!(conf_for("объясни, что такое Python").unwrap().name, "Perplexity");
    assert!(is_configured());
    // no search key: the usual provider (or the old error) as before
    crate::uistate::update_settings(&serde_json::json!({ "ai_search_key": "", "ai_provider": "openai" }));
    assert_eq!(conf_for("какие сегодня новости").unwrap().name, "OpenAI");
    crate::uistate::update_settings(&serde_json::json!({ "ai_provider": "none", "ai_key": "", "ai_model": "" }));
    assert!(!is_configured());
}

#[test]
fn a_model_of_another_provider_is_not_sent_to_perplexity() {
    for left_over in ["gemini-flash-latest", "Ai", "gpt-4o-mini", ""] {
        assert_eq!(conf_from("perplexity", left_over, "pplx-x", "").unwrap().model, "sonar", "{}", left_over);
    }
    assert_eq!(conf_from("perplexity", "sonar-pro", "pplx-x", "").unwrap().model, "sonar-pro");
    assert_eq!(conf_from("perplexity", "Sonar-Reasoning", "pplx-x", "").unwrap().model, "Sonar-Reasoning");
    // the others keep what the user typed
    assert_eq!(conf_from("openai", "gpt-4o", "k", "").unwrap().model, "gpt-4o");
}

#[test]
fn perplexity_is_checked_by_a_tiny_question_not_by_a_list_of_models() {
    // the server of the test has no «/models» (like Perplexity): only a question gets 200
    let m = mock(vec![(200, ok_body("pong"), 0)]);
    let mut c = conf(&m.base).unwrap();
    c.name = "Perplexity".into();
    c.model = "sonar".into();
    assert!(test_connection(&c).is_ok());
    let seen = m.requests.lock().clone();
    assert!(seen[0].starts_with("POST /chat/completions"), "{}", seen[0]);
    assert!(seen[0].contains("\"max_tokens\":1") && seen[0].contains("\"model\":\"sonar\""));
    // a wrong key is still reported as a wrong key (401), not as a wrong address
    let m = mock(vec![(401, "{\"error\":\"bad key\"}".into(), 0)]);
    let mut c = conf(&m.base).unwrap();
    c.name = "Perplexity".into();
    assert!(matches!(test_connection(&c), Err(AiError::Auth(_))));
    // the others are still checked by the list of models
    let m = mock(vec![(200, "{}".into(), 0)]);
    assert!(test_connection(&conf(&m.base).unwrap()).is_ok());
    assert!(m.requests.lock()[0].starts_with("GET /models"));
}


#[test]
fn openai_reasoning_models_think_briefly_and_the_others_get_no_such_parameter() {
    let mk = |name: &str, model: &str| ProviderConf { kind: Kind::OpenAiCompatible, name: name.into(), base: format!("https://{}.example/v1", name), model: model.into(), key: KEY.into(), route: String::new() };
    for model in ["gpt-6.1-sol", "gpt-6-luna", "gpt-5"] {
        let b = build_body_limit(&mk("OpenAI", model), &[], "s", true, None);
        assert_eq!(b["reasoning_effort"], "low", "{}", model);
    }
    // a model that does not think, another provider, a self-made server: nothing is added
    assert!(build_body_limit(&mk("OpenAI", "gpt-4o-mini"), &[], "s", true, None).get("reasoning_effort").is_none());
    assert!(build_body_limit(&mk("OpenRouter", "gpt-6.1-sol"), &[], "s", true, None).get("reasoning_effort").is_none());
    assert!(build_body_limit(&mk("ИИ-сервер", "gpt-5"), &[], "s", true, None).get("reasoning_effort").is_none());
}
