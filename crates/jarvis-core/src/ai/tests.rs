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
        ChatMsg { role: "user".into(), content: "привет".into(), ts: 0 },
        ChatMsg { role: "assistant".into(), content: "здравствуйте".into(), ts: 0 },
        ChatMsg { role: "user".into(), content: "как дела".into(), ts: 0 },
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
        msgs.push(ChatMsg { role: if i % 2 == 0 { "user" } else { "assistant" }.into(), content: format!("сообщение {}", i), ts: 0 });
    }
    msgs.push(ChatMsg { role: "error".into(), content: "ошибка".into(), ts: 0 });
    let ctx = context_for_model(&msgs);
    assert!(ctx.len() <= MAX_CONTEXT_MESSAGES);
    assert_eq!(ctx.first().unwrap().role, "user");
    assert!(ctx.iter().all(|m| m.role == "user" || m.role == "assistant"));
    // a budget in characters
    let big = vec![
        ChatMsg { role: "user".into(), content: "a".repeat(8000), ts: 0 },
        ChatMsg { role: "assistant".into(), content: "b".repeat(8000), ts: 0 },
        ChatMsg { role: "user".into(), content: "вопрос".into(), ts: 0 },
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
fn spoken_text_has_no_code_or_marks() {
    let s = speakable("Вот **пример**:\n```python\nprint(1)\n```\nГотово.");
    assert!(!s.contains("print") && !s.contains('*') && !s.contains('`'));
    assert!(s.contains("Готово"));
    assert!(speakable(&"Слово. ".repeat(300)).chars().count() <= 700);
}
