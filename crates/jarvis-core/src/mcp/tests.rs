use super::*;

struct Fake {
    commands: Vec<String>,
    fail: bool,
}

impl Backend for Fake {
    fn status(&mut self) -> Result<String, String> {
        if self.fail { Err("Jarvis не отвечает".into()) } else { Ok("Jarvis запущен: голос работает.".into()) }
    }
    fn command(&mut self, text: &str) -> Result<String, String> {
        if self.fail {
            return Err("Jarvis не отвечает".into());
        }
        self.commands.push(text.to_string());
        Ok("Команда передана Jarvis.".into())
    }
}

fn ask(line: &str, b: &mut Fake) -> Value {
    serde_json::from_str(&handle_line(line, b).expect("an answer")).unwrap()
}

#[test]
fn the_server_introduces_itself_and_keeps_the_version_it_knows() {
    let mut b = Fake { commands: vec![], fail: false };
    let r = ask(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"x","version":"1"}}}"#, &mut b);
    assert_eq!(r["id"], 1);
    assert_eq!(r["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(r["result"]["serverInfo"]["name"], "jarvis");
    assert!(r["result"]["capabilities"]["tools"].is_object());
    let r2 = ask(r#"{"jsonrpc":"2.0","id":"a","method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#, &mut b);
    assert_eq!((r2["id"].as_str(), r2["result"]["protocolVersion"].as_str()), (Some("a"), Some(PROTOCOL)));
}

#[test]
fn notifications_get_no_answer_and_ping_does() {
    let mut b = Fake { commands: vec![], fail: false };
    assert_eq!(handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &mut b), None);
    assert_eq!(handle_line("", &mut b), None);
    assert_eq!(handle_line("   ", &mut b), None);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#, &mut b)["result"], json!({}));
    // an answer of the other side is ignored
    assert_eq!(handle_line(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#, &mut b), None);
}

#[test]
fn the_two_tools_are_listed_with_their_schemas() {
    let mut b = Fake { commands: vec![], fail: false };
    let r = ask(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#, &mut b);
    let tools = r["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(names, vec!["jarvis_status", "jarvis_command"]);
    assert_eq!(tools[1]["inputSchema"]["required"], json!(["text"]));
    assert_eq!(tools[1]["inputSchema"]["properties"]["text"]["maxLength"], MAX_COMMAND_CHARS);
    assert!(tools.iter().all(|t| t["inputSchema"]["type"] == "object"));
}

#[test]
fn a_command_goes_to_jarvis_and_the_answer_is_text() {
    let mut b = Fake { commands: vec![], fail: false };
    let r = ask(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"jarvis_command","arguments":{"text":"  открой блокнот "}}}"#, &mut b);
    assert_eq!(r["result"]["isError"], false);
    assert_eq!(r["result"]["content"][0]["type"], "text");
    assert_eq!(r["result"]["content"][0]["text"], "Команда передана Jarvis.");
    assert_eq!(b.commands, vec!["открой блокнот"]);
    let s = ask(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"jarvis_status"}}"#, &mut b);
    assert_eq!(s["result"]["content"][0]["text"], "Jarvis запущен: голос работает.");
}

#[test]
fn a_bad_command_never_reaches_jarvis() {
    let mut b = Fake { commands: vec![], fail: false };
    for args in [r#"{}"#, r#"{"text":""}"#, r#"{"text":"   "}"#, r#"{"text":5}"#] {
        let r = ask(&format!(r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"jarvis_command","arguments":{}}}}}"#, args), &mut b);
        assert_eq!(r["result"]["isError"], true, "{}", args);
    }
    let long = "я".repeat(MAX_COMMAND_CHARS + 1);
    let r = ask(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": { "name": "jarvis_command", "arguments": { "text": long } } }).to_string(), &mut b);
    assert_eq!(r["result"]["isError"], true);
    assert!(b.commands.is_empty());
}

#[test]
fn a_refusal_of_jarvis_comes_back_as_a_tool_error_not_a_protocol_error() {
    let mut b = Fake { commands: vec![], fail: true };
    let r = ask(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"jarvis_command","arguments":{"text":"открой блокнот"}}}"#, &mut b);
    assert!(r.get("error").is_none());
    assert_eq!((r["result"]["isError"].clone(), r["result"]["content"][0]["text"].clone()), (json!(true), json!("Jarvis не отвечает")));
}

#[test]
fn errors_of_the_protocol_have_the_standard_codes() {
    let mut b = Fake { commands: vec![], fail: false };
    assert_eq!(ask("не json", &mut b)["error"]["code"], -32700);
    assert_eq!(ask("[]", &mut b)["error"]["code"], -32600);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1}"#, &mut b)["error"]["code"], -32600);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1,"method":"resources/list"}"#, &mut b)["error"]["code"], -32601);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"rm_rf"}}"#, &mut b)["error"]["code"], -32602);
    assert_eq!(ask(r#"{"jsonrpc":"2.0","id":9,"method":"x"}"#, &mut b)["id"], 9, "the id of the request is returned");
}

#[test]
fn the_state_is_told_in_a_few_words_without_private_data() {
    let s = summary_of_state(&json!({ "voice_state": { "state": "LISTENING" }, "ai_busy": true, "activity": "searching", "city": "Ташкент", "ai_key": "sk-secret" }));
    assert_eq!(s, "Jarvis запущен: голос работает, ИИ отвечает, занят: ищет.");
    assert!(!s.contains("Ташкент") && !s.contains("sk-"));
    assert_eq!(summary_of_state(&json!({ "voice_state": { "state": "DISABLED" } })), "Jarvis запущен: микрофон выключен.");
    assert_eq!(summary_of_state(&json!({})), "Jarvis запущен: состояние голоса неизвестно.");
}

#[test]
fn without_a_running_jarvis_the_protocol_still_answers() {
    let mut b = Unavailable("Jarvis не запущен".into());
    let r: Value = serde_json::from_str(&handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"jarvis_status"}}"#, &mut b).unwrap()).unwrap();
    assert_eq!((r["result"]["isError"].clone(), r["result"]["content"][0]["text"].clone()), (json!(true), json!("Jarvis не запущен")));
}
