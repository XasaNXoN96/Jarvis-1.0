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
    Ok(ProviderConf { kind: Kind::OpenAiCompatible, name: "Test".into(), base: base.to_string(), model: "test-model".into(), key: KEY.into() })
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
    assert!(ctx.iter().map(|m| m.content.chars().count()).sum::<usize>() <= MAX_CONTEXT_CHARS + 8000);
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
        let m = mock(vec![(status, body.to_string(), 0)]);
        let id = run(store, conf(&m.base), None, "вопрос").unwrap();
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
    assert!(!crate::remote::push_text_would_work_for_tests());
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
    let c = Ok(ProviderConf { kind: Kind::OpenAiCompatible, name: "Google Gemini".into(), base: m.base.clone(), model: "gemini-flash-latest".into(), key: KEY.into() });
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
    let a = ProviderConf { kind: Kind::Anthropic, name: "A".into(), base: "http://x".into(), model: "m".into(), key: "k".into() };
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
    FIRST_FRAGMENT_MS.store(15_000, Ordering::SeqCst);
}

#[test]
fn gemini_is_asked_to_think_little_and_a_provider_that_refuses_the_parameter_is_asked_again_without_it() {
    let store = leaked_store();
    let m = mock(vec![(400, "{\"error\":{\"message\":\"Unknown field reasoning_effort\"}}".to_string(), 0), (200, ok_body("Готово"), 0)]);
    let c = Ok(ProviderConf { kind: Kind::OpenAiCompatible, name: "Google Gemini".into(), base: m.base.clone(), model: "gemini-flash-latest".into(), key: KEY.into() });
    let id = run(store, c, None, "вопрос").unwrap();
    assert_eq!(messages(store, &id).last().unwrap().content, "Готово");
    let reqs = m.requests.lock().clone();
    assert_eq!(reqs.len(), 2);
    assert!(reqs[0].contains("reasoning_effort"), "the first request carries the parameter");
    assert!(!reqs[1].contains("reasoning_effort"), "the second one does not");
    // other providers never get it
    let o = ProviderConf { kind: Kind::OpenAiCompatible, name: "OpenAI".into(), base: "http://x".into(), model: "m".into(), key: "k".into() };
    assert!(build_body_opts(&o, &[], "s", true).get("reasoning_effort").is_none());
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
