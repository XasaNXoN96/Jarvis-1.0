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
            if try_start(&["/C", "start", "", browser, &app, "--window-size=560,820"]) {
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
