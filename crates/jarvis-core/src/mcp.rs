//! Jarvis as an MCP server (Model Context Protocol): another program (Claude Desktop, an editor, an agent) can ask him for his state
//! and give him a command, over standard input and output. The idea is the MCP layer of OpenJarvis
//! (https://github.com/open-jarvis/OpenJarvis, src/openjarvis/mcp/, Apache-2.0); this is our own code and it is much smaller on purpose.
//!
//! How it works. The other program starts `jarvis-app --mcp`: a small bridge that speaks JSON-RPC 2.0 (one message per line) on stdin
//! and stdout and forwards to the Jarvis that is already running, through his local server on 127.0.0.1 with his secret token (the
//! file `ui_token` of the user: a program of another user cannot read it). Two tools only:
//!   - `jarvis_status`: what Jarvis does now (a few words, no private data);
//!   - `jarvis_command`: one phrase for the usual command pipeline, as if said by «another local program» (the door Ipc).
//!
//! The limits (the real ones are in the server, ui.rs `/mcp-command`; the bridge cannot get around them):
//!   - OFF until the owner switches «Разрешить внешним программам управлять Джарвисом (MCP)» on in Settings;
//!   - a phrase of at most 200 characters that passes approvals::is_allowed_phrase (nothing about power, deleting, installing, links,
//!     passwords, remote control), at most 30 a minute;
//!   - the usual checks of rights of the commands apply after that, and everything is in the log of actions.
//! A program that is controlled by a model can be talked into things by a web page or a mail: that is why the list of phrases is the
//! same hard list as for the proposals of the proactive helper, and why the switch is off by default.

use serde_json::{json, Value};

pub const PROTOCOL: &str = "2024-11-05";
pub const MAX_COMMAND_CHARS: usize = 200;

// what the bridge needs from the running Jarvis
pub trait Backend {
    fn status(&mut self) -> Result<String, String>;
    fn command(&mut self, text: &str) -> Result<String, String>;
}

pub fn tools_list() -> Value {
    json!({ "tools": [
        {
            "name": "jarvis_status",
            "description": "What Jarvis does now: listening, speaking, busy, off. No private data.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false }
        },
        {
            "name": "jarvis_command",
            "description": "Give Jarvis one phrase as a command, for example «открой блокнот», «включи музыку», «поставь таймер на 10 минут». Nothing about power, deleting, installing or passwords is accepted. The owner must have switched MCP on in the settings of Jarvis.",
            "inputSchema": { "type": "object", "properties": { "text": { "type": "string", "maxLength": MAX_COMMAND_CHARS, "description": "the phrase, in Russian" } }, "required": ["text"], "additionalProperties": false }
        }
    ] })
}

fn error(id: &Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

fn result(id: &Value, r: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": r }).to_string()
}

fn text_result(text: &str, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

// One line in, the answer line out (None for a notification: it has no answer).
pub fn handle_line(line: &str, backend: &mut dyn Backend) -> Option<String> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let msg: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return Some(error(&Value::Null, -32700, "parse error")),
    };
    if msg.is_array() {
        return Some(error(&Value::Null, -32600, "batches are not supported"));
    }
    let id = msg.get("id").cloned();
    let Some(method) = msg.get("method").and_then(|m| m.as_str()) else {
        // an answer of the other side to something we asked (we ask nothing): ignored
        return if id.is_some() && (msg.get("result").is_some() || msg.get("error").is_some()) { None } else { Some(error(id.as_ref().unwrap_or(&Value::Null), -32600, "no method")) };
    };
    // a notification (no id): nothing is answered
    let Some(id) = id else { return None };
    match method {
        "initialize" => {
            // the version of the client when we know it, else ours
            let asked = msg.pointer("/params/protocolVersion").and_then(|v| v.as_str()).unwrap_or(PROTOCOL);
            let version = if ["2024-11-05", "2025-03-26", "2025-06-18"].contains(&asked) { asked } else { PROTOCOL };
            Some(result(&id, json!({ "protocolVersion": version, "capabilities": { "tools": { "listChanged": false } }, "serverInfo": { "name": "jarvis", "version": env!("CARGO_PKG_VERSION") } })))
        }
        "ping" => Some(result(&id, json!({}))),
        "tools/list" => Some(result(&id, tools_list())),
        "tools/call" => {
            let name = msg.pointer("/params/name").and_then(|n| n.as_str()).unwrap_or("");
            match name {
                "jarvis_status" => Some(result(&id, match backend.status() {
                    Ok(t) => text_result(&t, false),
                    Err(e) => text_result(&e, true),
                })),
                "jarvis_command" => {
                    let text = msg.pointer("/params/arguments/text").and_then(|t| t.as_str()).unwrap_or("").trim().to_string();
                    if text.is_empty() {
                        return Some(result(&id, text_result("нужен текст команды (поле text)", true)));
                    }
                    if text.chars().count() > MAX_COMMAND_CHARS {
                        return Some(result(&id, text_result("команда длиннее 200 знаков", true)));
                    }
                    Some(result(&id, match backend.command(&text) {
                        Ok(t) => text_result(&t, false),
                        Err(e) => text_result(&e, true),
                    }))
                }
                _ => Some(error(&id, -32602, "unknown tool")),
            }
        }
        _ => Some(error(&id, -32601, "method not found")),
    }
}

// ---------------------------------------------------------------------------------------------
// the bridge to the running Jarvis
// ---------------------------------------------------------------------------------------------

#[cfg(feature = "lua")]
struct Local {
    token: String,
    client: reqwest::blocking::Client,
}

#[cfg(feature = "lua")]
impl Local {
    fn new() -> Result<Self, String> {
        let token = crate::ui_auth::read_saved().ok_or("Jarvis не запущен или ещё не создал секрет окна (запустите Jarvis и повторите)")?;
        let client = reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(3)).timeout(std::time::Duration::from_secs(10)).no_proxy().build().map_err(|e| e.to_string())?;
        Ok(Local { token, client })
    }

    fn url(path: &str) -> String {
        format!("http://127.0.0.1:{}{}", crate::ui::PORT, path)
    }
}

#[cfg(feature = "lua")]
impl Backend for Local {
    fn status(&mut self) -> Result<String, String> {
        let resp = self.client.get(Self::url("/state")).header("X-Jarvis-Token", &self.token).send().map_err(|_| "Jarvis не отвечает (он запущен?)".to_string())?;
        if !resp.status().is_success() {
            return Err(format!("Jarvis ответил {}", resp.status()));
        }
        let v: Value = resp.json().map_err(|_| "ответ Jarvis не разобран".to_string())?;
        Ok(summary_of_state(&v))
    }

    fn command(&mut self, text: &str) -> Result<String, String> {
        let resp = self
            .client
            .post(Self::url("/mcp-command"))
            .header("X-Jarvis-Token", &self.token)
            .header("X-Jarvis", "1")
            .header("Content-Type", "application/json")
            .body(json!({ "text": text }).to_string())
            .send()
            .map_err(|_| "Jarvis не отвечает (он запущен?)".to_string())?;
        let status = resp.status().as_u16();
        let v: Value = resp.json().unwrap_or(Value::Null);
        let message = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
        if status == 200 && v.get("ok").and_then(|o| o.as_bool()) == Some(true) {
            Ok("Команда передана Jarvis.".to_string())
        } else if message.is_empty() {
            Err(format!("Jarvis отказал ({})", status))
        } else {
            Err(message.to_string())
        }
    }
}

// a few words about the state: no names, no texts, no settings
pub fn summary_of_state(v: &Value) -> String {
    let mut parts: Vec<String> = Vec::new();
    let voice = v.pointer("/voice_state/state").and_then(|s| s.as_str()).unwrap_or("");
    parts.push(match voice {
        "DISABLED" => "микрофон выключен".to_string(),
        "ERROR" => "голос не работает".to_string(),
        "" => "состояние голоса неизвестно".to_string(),
        _ => "голос работает".to_string(),
    });
    if v.get("ai_busy").and_then(|b| b.as_bool()) == Some(true) {
        parts.push("ИИ отвечает".to_string());
    }
    if let Some(a) = v.get("activity").and_then(|a| a.as_str()).filter(|a| !a.is_empty() && *a != "idle") {
        parts.push(format!("занят: {}", match a { "recognizing" => "распознаёт речь", "searching" => "ищет", "executing" => "выполняет команду", other => other }));
    }
    format!("Jarvis запущен: {}.", parts.join(", "))
}

// `jarvis-app --mcp`: JSON-RPC on stdin and stdout until the other side closes the input. Nothing else is ever written to stdout.
#[cfg(feature = "lua")]
pub fn run_stdio() -> Result<(), String> {
    use std::io::{BufRead, Write};
    let mut backend: Box<dyn Backend> = match Local::new() {
        Ok(l) => Box::new(l),
        Err(e) => Box::new(Unavailable(e)),
    };
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if let Some(answer) = handle_line(&line, backend.as_mut()) {
            if writeln!(out, "{}", answer).is_err() || out.flush().is_err() {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(not(feature = "lua"))]
pub fn run_stdio() -> Result<(), String> {
    Err("в этой сборке нет сетевого модуля".into())
}

// Jarvis is not running: the protocol still answers, the tools say why they cannot work
pub struct Unavailable(pub String);

impl Backend for Unavailable {
    fn status(&mut self) -> Result<String, String> {
        Err(self.0.clone())
    }
    fn command(&mut self, _text: &str) -> Result<String, String> {
        Err(self.0.clone())
    }
}

#[cfg(test)]
mod tests;
