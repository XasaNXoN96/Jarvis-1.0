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
                let pack_name = pack.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                list.push(json!({
                    "id": cmd.id,
                    "pack": pack_name,
                    "group": crate::uistate::group_key(&pack_name, &cmd.id),
                    "phrases": phrases.iter().take(5).collect::<Vec<_>>(),
                }));
            }
        }
    }
    let groups: Vec<_> = crate::uistate::GROUPS
        .iter()
        .map(|g| json!({ "key": g.0, "emoji": g.1, "title": g.2, "about": g.3, "disabled": crate::uistate::is_group_disabled(g.0) }))
        .collect();
    json!({ "language": lang, "commands": list, "groups": groups }).to_string()
}

fn pack_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn modules_json() -> serde_json::Value {
    let mut counts: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();
    if let Some(commands) = COMMANDS_LIST.get() {
        for pack in commands {
            let name = pack_name(&pack.path);
            for cmd in &pack.commands {
                *counts.entry(crate::uistate::group_key(&name, &cmd.id)).or_insert(0) += 1;
            }
        }
    }
    let list: Vec<_> = crate::uistate::GROUPS
        .iter()
        .map(|g| json!({
            "key": g.0, "emoji": g.1, "title": g.2, "about": g.3,
            "commands": counts.get(g.0).copied().unwrap_or(0),
            "disabled": crate::uistate::is_group_disabled(g.0),
            "locked": g.0 == "control",
        }))
        .collect();
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
        "reaction_preview" => {
            let id = crate::uistate::settings_json()["reaction_voice"].as_str().unwrap_or("").to_string();
            crate::voices::play_preview(&id);
            json!({ "ok": true })
        }
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
        "group" => {
            let key = body.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let disabled = body.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
            if key.is_empty() || key == "control" {
                return json!({ "ok": false, "message": "Этот раздел отключить нельзя" });
            }
            crate::uistate::set_group_disabled(key, disabled);
            json!({ "ok": true })
        }
        "ai_test" => ai_test(),
        "voice_install" => match crate::voice_install::start() {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "restart_voice" => {
            // starts a fresh Jarvis without --text (after this one has closed and freed its ports) and quits
            if let Ok(exe) = std::env::current_exe() {
                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::process::CommandExt;
                    let _ = Command::new("cmd")
                        .args(["/C", &format!("ping -n 3 127.0.0.1 >nul & start \"\" \"{}\"", exe.display())])
                        .creation_flags(0x0800_0000)
                        .spawn();
                }
                #[cfg(not(target_os = "windows"))]
                let _ = Command::new(&exe).spawn();
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(400));
                    std::process::exit(0);
                });
                return json!({ "ok": true });
            }
            json!({ "ok": false, "message": "Не удалось перезапустить" })
        }
        _ => json!({ "ok": false, "message": "unknown action" }),
    }
}

// "check the connection": asks the provider for its model list with the saved key
fn ai_test() -> serde_json::Value {
    match crate::ai::conf_from_settings() {
        Err(e) => json!({ "ok": false, "message": e.user_text() }),
        Ok(conf) => match crate::ai::test_connection(&conf) {
            Ok(()) => json!({ "ok": true, "message": format!("Соединение с {} работает", conf.name) }),
            Err(e) => json!({ "ok": false, "message": e.user_text() }),
        },
    }
}

// ---- AI chat (the window talks to it over local HTTP; the key never leaves the backend) ----
fn chat_json(id: &str) -> serde_json::Value {
    match crate::ai::store().load(id) {
        Some(c) => json!({ "ok": true, "id": c.id, "title": c.title, "messages": c.messages, "busy": crate::ai::is_busy(&c.id) }),
        None => json!({ "ok": false, "message": "Диалог не найден" }),
    }
}

fn chats_json() -> serde_json::Value {
    let store = crate::ai::store();
    let active = store.active().unwrap_or_default();
    let conf = crate::ai::conf_from_settings();
    let s = crate::uistate::settings();
    json!({
        "active": active,
        "chats": store.list(),
        "configured": conf.is_ok(),
        "problem": conf.as_ref().err().map(|e| e.user_text()).unwrap_or_default(),
        "provider": conf.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
        "model": s.ai_model,
    })
}

fn chat_action(body: &serde_json::Value) -> serde_json::Value {
    let store = crate::ai::store();
    let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "new" => {
            let c = store.create();
            json!({ "ok": true, "id": c.id })
        }
        "select" => {
            if store.load(id).is_some() {
                store.set_active(id);
                json!({ "ok": true, "id": id })
            } else {
                json!({ "ok": false, "message": "Диалог не найден" })
            }
        }
        "send" => {
            let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let chat = if id.is_empty() { None } else { Some(id) };
            match crate::ai::send(chat, text, true) {
                Ok(chat_id) => json!({ "ok": true, "id": chat_id }),
                Err(e) => json!({ "ok": false, "message": e.user_text() }),
            }
        }
        "stop" => json!({ "ok": crate::ai::stop(id) }),
        "clear" => json!({ "ok": !crate::ai::is_busy(id) && store.clear(id) }),
        "delete" => {
            let ok = !crate::ai::is_busy(id) && store.delete(id);
            json!({ "ok": ok })
        }
        _ => json!({ "ok": false, "message": "unknown operation" }),
    }
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
                "url_https": if crate::remote::is_running() { crate::remote::url_https() } else { String::new() },
            })
            .to_string();
            respond(&mut stream, "200 OK", "application/json", &body);
        }
        ("GET", "/remote-qr") | ("GET", "/remote-qr-https") => {
            if !crate::remote::is_running() {
                respond(&mut stream, "404 Not Found", "text/plain", "remote control is off");
            } else {
                let link = if req.path == "/remote-qr-https" { crate::remote::url_https() } else { crate::remote::url() };
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
        ("GET", "/reaction-voices") => {
            let list: Vec<_> = crate::voices::list_voices().iter().map(|v| json!({ "id": v.voice.id, "name": v.voice.name })).collect();
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
        ("GET", "/voice-install") => respond(&mut stream, "200 OK", "application/json", &crate::voice_install::status_json().to_string()),
        ("GET", "/chats") => respond(&mut stream, "200 OK", "application/json", &chats_json().to_string()),
        ("POST", "/chat") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            respond(&mut stream, "200 OK", "application/json", &chat_action(&body).to_string());
        }
        ("GET", p) if p.starts_with("/chat/") => respond(&mut stream, "200 OK", "application/json", &chat_json(&p["/chat/".len()..]).to_string()),
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
            if try_start(&["/C", "start", "", browser, &app, "--window-size=1040,740"]) {
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
