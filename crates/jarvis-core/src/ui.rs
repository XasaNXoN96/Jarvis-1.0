// The Jarvis window: a small web page (chat, orb, commands, settings) shown in its own
// app-style browser window (Edge / Chrome without address bar).
//
// This server listens on 127.0.0.1 only (never on the network). The page talks to Jarvis
// over the local WebSocket (ipc, port 9712) and loads the command list from /commands.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::Duration;

use serde_json::json;

use crate::{i18n, COMMANDS_LIST};

pub const PORT: u16 = 9714;
const PAGE: &str = include_str!("ui_page.html");
const ICON: &[u8] = include_bytes!("../../../resources/icons/128x128.png");

pub fn url() -> String {
    format!("http://127.0.0.1:{}/", PORT)
}

pub fn start() {
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(l) => l,
        Err(e) => {
            warn!("UI: cannot listen on port {}: {} (is another Jarvis running?)", PORT, e);
            return;
        }
    };
    info!("UI: window page on {}", url());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = handle(stream);
            });
        }
    });
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.as_bytes().len(),
        body
    );
}

fn commands_json() -> String {
    let lang = i18n::get_language();
    let mut list = Vec::new();
    if let Some(commands) = COMMANDS_LIST.get() {
        for pack in commands {
            for cmd in &pack.commands {
                let phrases = cmd.get_phrases(&lang);
                if phrases.is_empty() {
                    continue;
                }
                list.push(json!({
                    "id": cmd.id,
                    "pack": pack.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                    "phrases": phrases.iter().take(4).collect::<Vec<_>>(),
                }));
            }
        }
    }
    json!({ "language": lang, "commands": list }).to_string()
}

fn pack_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn modules_json() -> serde_json::Value {
    let mut list = Vec::new();
    if let Some(commands) = COMMANDS_LIST.get() {
        for pack in commands {
            let name = pack_name(&pack.path);
            list.push(json!({
                "name": name,
                "commands": pack.commands.len(),
                "disabled": crate::uistate::is_pack_disabled(&name),
            }));
        }
    }
    json!(list)
}

fn settings_page_json() -> serde_json::Value {
    let mut v = crate::uistate::settings_json();
    let devices = crate::recorder::get_audio_devices();
    let selected = crate::DB.get().map(|d| d.read().microphone).unwrap_or(-1);
    v["mic_devices"] = json!(devices);
    v["mic_selected"] = json!(selected);
    v
}

fn apply_settings(patch: &serde_json::Value) -> serde_json::Value {
    crate::uistate::update_settings(patch);
    if let Some(idx) = patch.get("mic_device").and_then(|v| v.as_i64()) {
        if let Some(db) = crate::DB.get() {
            db.write().microphone = idx as i32;
            let snapshot = db.read().clone();
            let _ = crate::db::save_settings(&snapshot);
        }
    }
    json!({ "ok": true })
}

fn run_action(body: &serde_json::Value) -> serde_json::Value {
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
    match name {
        "tts_test" => {
            crate::tts::set_enabled(true);
            let text = if i18n::get_language() == "en" { "Hello. This is my voice. How do I sound?" } else { "Здравствуйте. Это мой голос. Как я звучу?" };
            crate::tts::speak(text);
            json!({ "ok": true })
        }
        "tts_toggle" => {
            let on = body.get("value").and_then(|v| v.as_bool()).unwrap_or(true);
            crate::tts::set_enabled(on);
            json!({ "ok": true })
        }
        "mic_muted" => {
            let muted = body.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
            crate::MIC_MUTED.store(muted, std::sync::atomic::Ordering::SeqCst);
            json!({ "ok": true })
        }
        "cancel_timer" => {
            #[cfg(feature = "lua")]
            {
                let id = body.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                return json!({ "ok": crate::lua::cancel_timer(id) });
            }
            #[cfg(not(feature = "lua"))]
            json!({ "ok": false })
        }
        "module" => {
            let pack = body.get("pack").and_then(|v| v.as_str()).unwrap_or("");
            let disabled = body.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
            if pack.is_empty() || pack == "assistant" {
                return json!({ "ok": false, "message": "Основной модуль отключить нельзя" });
            }
            crate::uistate::set_pack_disabled(pack, disabled);
            json!({ "ok": true })
        }
        "ai_test" => ai_test(),
        _ => json!({ "ok": false, "message": "unknown action" }),
    }
}

// "check the connection": asks the provider for its model list with the saved key
#[cfg(feature = "lua")]
fn ai_test() -> serde_json::Value {
    let s = crate::uistate::settings();
    let (url, headers): (String, Vec<(&str, String)>) = match s.ai_provider.as_str() {
        "anthropic" => ("https://api.anthropic.com/v1/models".to_string(), vec![("x-api-key", s.ai_key.clone()), ("anthropic-version", "2023-06-01".to_string())]),
        "openai" => ("https://api.openai.com/v1/models".to_string(), vec![("Authorization", format!("Bearer {}", s.ai_key))]),
        "ollama" => (format!("{}/api/tags", if s.ai_url.is_empty() { "http://127.0.0.1:11434" } else { s.ai_url.trim_end_matches('/') }), vec![]),
        "custom" => (format!("{}/models", s.ai_url.trim_end_matches('/')), vec![("Authorization", format!("Bearer {}", s.ai_key))]),
        _ => return json!({ "ok": false, "message": "Провайдер не выбран" }),
    };
    if s.ai_provider != "ollama" && s.ai_key.is_empty() {
        return json!({ "ok": false, "message": "Введите ключ API" });
    }
    if s.ai_provider == "custom" && s.ai_url.is_empty() {
        return json!({ "ok": false, "message": "Введите адрес сервера" });
    }
    let client = match reqwest::blocking::Client::builder().timeout(Duration::from_secs(8)).build() {
        Ok(c) => c,
        Err(e) => return json!({ "ok": false, "message": e.to_string() }),
    };
    let mut request = client.get(&url);
    for (k, v) in headers {
        request = request.header(k, v);
    }
    match request.send() {
        Ok(r) if r.status().is_success() => json!({ "ok": true, "message": "Соединение работает" }),
        Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => json!({ "ok": false, "message": "Ключ не принят (ошибка доступа)" }),
        Ok(r) => json!({ "ok": false, "message": format!("Сервер ответил: {}", r.status()) }),
        Err(e) => json!({ "ok": false, "message": format!("Нет соединения: {}", e.without_url()) }),
    }
}

#[cfg(not(feature = "lua"))]
fn ai_test() -> serde_json::Value {
    json!({ "ok": false, "message": "not available" })
}

// QR code (SVG) of the phone-remote link
fn qr_svg(data: &str) -> Option<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    Some(
        code.render::<svg::Color>()
            .min_dimensions(300, 300)
            .quiet_zone(true)
            .dark_color(svg::Color("#04101f"))
            .light_color(svg::Color("#ffffff"))
            .build(),
    )
}

// the avatar photo chosen by the user lives in the config dir (never leaves this computer)
fn avatar_path() -> Option<std::path::PathBuf> {
    crate::APP_CONFIG_DIR.get().map(|d| d.join("avatar.img"))
}

const MAX_AVATAR_BYTES: usize = 6 * 1024 * 1024;

struct Request {
    method: String,
    path: String,
    host_ok: bool,
    custom_header: bool,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 32 * 1024 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").split('?').next().unwrap_or("").to_string();

    let mut host_ok = false;
    let mut custom_header = false;
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_lowercase(), v.trim());
            match k.as_str() {
                "host" => host_ok = v == format!("127.0.0.1:{}", PORT) || v == format!("localhost:{}", PORT),
                "content-length" => content_length = v.parse().unwrap_or(0),
                // a web page on another site cannot add this header without our permission
                "x-jarvis" => custom_header = v == "1",
                _ => {}
            }
        }
    }
    if content_length > MAX_AVATAR_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let end = buf.len().min(header_end + content_length);
    Ok(Request { method, path, host_ok, custom_header, body: buf[header_end..end].to_vec() })
}

fn respond_bytes(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        body.len()
    );
    let _ = stream.write_all(body);
}

fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    let req = read_request(&mut stream)?;

    // only our own pages: protects against DNS-rebinding tricks from web pages
    if !req.host_ok {
        respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
        return Ok(());
    }

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => respond(&mut stream, "200 OK", "text/html", PAGE),
        ("GET", "/icon.png") | ("GET", "/favicon.ico") => respond_bytes(&mut stream, "200 OK", "image/png", ICON),
        ("GET", "/remote") => {
            // phone remote state for the Settings page (the secret link is shown only to this window)
            let body = serde_json::json!({
                "enabled": crate::remote::is_running(),
                "url": if crate::remote::is_running() { crate::remote::url() } else { String::new() },
                "url_http": if crate::remote::is_running() { crate::remote::url_http() } else { String::new() },
            })
            .to_string();
            respond(&mut stream, "200 OK", "application/json", &body);
        }
        ("GET", "/remote-qr") | ("GET", "/remote-qr-http") => {
            if !crate::remote::is_running() {
                respond(&mut stream, "404 Not Found", "text/plain", "remote control is off");
            } else {
                let link = if req.path == "/remote-qr-http" { crate::remote::url_http() } else { crate::remote::url() };
                match qr_svg(&link) {
                    Some(svg) => respond_bytes(&mut stream, "200 OK", "image/svg+xml", svg.as_bytes()),
                    None => respond(&mut stream, "500 Internal Server Error", "text/plain", "qr failed"),
                }
            }
        }
        ("GET", "/state") => respond(&mut stream, "200 OK", "application/json", &crate::uistate::state_json().to_string()),
        ("GET", "/settings") => respond(&mut stream, "200 OK", "application/json", &settings_page_json().to_string()),
        ("GET", "/voices") => {
            let list: Vec<_> = crate::tts::list_voices().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
            respond(&mut stream, "200 OK", "application/json", &json!(list).to_string());
        }
        ("GET", "/modules") => respond(&mut stream, "200 OK", "application/json", &modules_json().to_string()),
        ("POST", "/settings") | ("POST", "/action") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            let result = if req.path == "/settings" { apply_settings(&body) } else { run_action(&body) };
            respond(&mut stream, "200 OK", "application/json", &result.to_string());
        }
        ("GET", "/commands") => respond(&mut stream, "200 OK", "application/json", &commands_json()),
        ("GET", "/avatar") => {
            match avatar_path().and_then(|p| std::fs::read(p).ok()) {
                Some(data) => {
                    let ct = sniff_image(&data).unwrap_or("application/octet-stream");
                    respond_bytes(&mut stream, "200 OK", ct, &data);
                }
                None => respond(&mut stream, "404 Not Found", "text/plain", "no avatar"),
            }
        }
        ("POST", "/avatar") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let path = match avatar_path() {
                Some(p) => p,
                None => {
                    respond(&mut stream, "500 Internal Server Error", "text/plain", "no config dir");
                    return Ok(());
                }
            };
            if req.body.is_empty() {
                let _ = std::fs::remove_file(&path);
                info!("UI: avatar removed");
                respond(&mut stream, "200 OK", "text/plain", "removed");
            } else if sniff_image(&req.body).is_none() {
                respond(&mut stream, "400 Bad Request", "text/plain", "not an image");
            } else {
                match std::fs::write(&path, &req.body) {
                    Ok(()) => {
                        info!("UI: avatar saved ({} bytes)", req.body.len());
                        respond(&mut stream, "200 OK", "text/plain", "saved");
                    }
                    Err(e) => respond(&mut stream, "500 Internal Server Error", "text/plain", &e.to_string()),
                }
            }
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", "not found"),
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn try_start(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("cmd")
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// Opens the Jarvis window. Returns true when a browser window was started.
pub fn open_window() -> bool {
    let link = url();
    #[cfg(target_os = "windows")]
    {
        let app = format!("--app={}", link);
        // Edge ships with Windows; Chrome as a second choice (both give a window without address bar)
        for browser in ["msedge", "chrome"] {
            if try_start(&["/C", "start", "", browser, &app, "--window-size=1180,760"]) {
                info!("UI: window opened in {}", browser);
                return true;
            }
        }
        // last resort: default browser in a normal tab
        let ok = Command::new("rundll32")
            .args(["url.dll,FileProtocolHandler", &link])
            .spawn()
            .is_ok();
        if ok {
            info!("UI: opened in the default browser");
        }
        ok
    }
    #[cfg(not(target_os = "windows"))]
    {
        info!("UI: open {} in a browser", link);
        let _ = Command::new("xdg-open").arg(&link).spawn();
        false
    }
}
