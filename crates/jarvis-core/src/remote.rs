// Remote control from a phone / another device on the same Wi-Fi.
//
// OFF by default. "включи удалённое управление" starts a tiny HTTP server on port 9713;
// the phone opens https://<pc-ip>:9713/?t=<token> and sends text commands (same as typing
// them in the console). Every request must carry the secret token, which is created once
// and stored in the config dir (remote_token). Do not share the link.
//
// The same port speaks https (self-signed certificate made on this PC, the browser warns once) and
// plain http. Browsers give the microphone only to https pages, so the phone page uses https for voice.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use once_cell::sync::{Lazy, OnceCell};
use parking_lot::Mutex;
use rand::RngCore;

use crate::APP_CONFIG_DIR;

pub const PORT: u16 = 9713;

static TX: OnceCell<Mutex<Sender<String>>> = OnceCell::new();
static RUNNING: AtomicBool = AtomicBool::new(false);
// when a device with the right link last opened the remote page or sent a command (unix seconds, 0 = never)
static LAST_CLIENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// the secret of the link, only so that the diagnostics report can remove it (never created here)
pub fn token_for_diag() -> String {
    Lazy::get(&TOKEN).cloned().unwrap_or_default()
}

pub fn last_client() -> Option<u64> {
    match LAST_CLIENT.load(Ordering::SeqCst) { 0 => None, t => Some(t) }
}
static TOKEN: Lazy<String> = Lazy::new(load_or_create_token);

// ---------- the phone side of the remote ----------
// The remote is a web page in the phone's browser (no Android app). The page polls the PC every two seconds
// (GET /poll) and reports what a web page may know about its phone (POST /state): battery (Chrome's Battery API),
// whether the page is on screen, the state of its voice mode. The PC can send the phone only what a page can do:
// a sound signal to find it, a link to open, a text to copy. Media of other apps, locking the phone and its
// notifications are out of reach of a web page and are not offered.

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1)
}

// the phone counts as connected while it polled in the last seconds
pub const PHONE_ONLINE_SECS: u64 = 10;
// a message the phone has not picked up in this time is dropped (a "find my phone" signal must not sound an hour later)
const MSG_TTL_SECS: u64 = 120;

#[derive(Default, Clone)]
struct Phone {
    last_seen: u64,
    battery: Option<u8>,
    charging: Option<bool>,
    visible: bool,
    voice: String,
    voice_error: String,
    ringing: bool,
}

struct Msg {
    id: u64,
    kind: &'static str,
    data: String,
    at: u64,
}

static PHONE: Lazy<Mutex<Phone>> = Lazy::new(|| Mutex::new(Phone::default()));
static OUTBOX: Lazy<Mutex<(u64, std::collections::VecDeque<Msg>)>> = Lazy::new(|| Mutex::new((0, std::collections::VecDeque::new())));

fn phone_online(p: &Phone, now: u64) -> bool {
    p.last_seen > 0 && now.saturating_sub(p.last_seen) <= PHONE_ONLINE_SECS
}

// what the PC window shows about the phone (no secrets, no texts)
pub fn phone_json() -> serde_json::Value {
    let p = PHONE.lock().clone();
    let now = now_secs();
    serde_json::json!({
        "enabled": is_running(),
        "online": is_running() && phone_online(&p, now),
        "last_seen": if p.last_seen > 0 { Some(p.last_seen) } else { None },
        "battery": p.battery,
        "charging": p.charging,
        "visible": p.visible,
        "voice": p.voice,
        "voice_error": p.voice_error,
        "ringing": p.ringing && phone_online(&p, now),
    })
}

// queue a message for the phone page: "ring", "stop", "link" (data = https address) or "text" (data = the text).
// Ok(reply) says honestly whether the phone will get it now; Err when it cannot be sent at all.
pub fn send_to_phone(kind: &str, data: &str) -> Result<String, String> {
    let kind: &'static str = match kind {
        "ring" => "ring",
        "stop" => "stop",
        "link" => "link",
        "text" => "text",
        _ => return Err(format!("неизвестное действие для телефона: {}", kind)),
    };
    let data = data.trim();
    match kind {
        "link" => {
            let l = data.to_lowercase();
            if !(l.starts_with("https://") || l.starts_with("http://")) || data.len() > 2000 || data.contains(char::is_whitespace) {
                return Err("это не ссылка: нужен адрес вида https://…".into());
            }
        }
        "text" => {
            if data.is_empty() {
                return Err("нечего отправлять: текст пустой".into());
            }
            if data.chars().count() > 4000 {
                return Err("текст слишком длинный (больше 4000 символов)".into());
            }
        }
        _ => {}
    }
    if !is_running() {
        return Err("пульт телефона выключен. Включите его на главной странице и откройте ссылку на телефоне".into());
    }
    let now = now_secs();
    {
        let mut ob = OUTBOX.lock();
        ob.0 += 1;
        let id = ob.0;
        if kind == "stop" {
            ob.1.retain(|m| m.kind != "ring"); // a signal not yet picked up is cancelled
        }
        ob.1.push_back(Msg { id, kind, data: data.to_string(), at: now });
        while ob.1.len() > 20 {
            ob.1.pop_front();
        }
    }
    // the content of a text or link is private: only its kind and size go to the log
    info!("Remote control: '{}' queued for the phone ({} chars)", kind, data.chars().count());
    let online = phone_online(&PHONE.lock(), now);
    Ok(if online {
        "отправлено на телефон".to_string()
    } else {
        "телефон сейчас не на связи. Откройте пульт на телефоне: сообщение придёт, если сделать это в течение двух минут".to_string()
    })
}

fn query_param<'a>(path: &'a str, name: &str) -> Option<&'a str> {
    let query = path.split_once('?')?.1;
    query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == name).map(|(_, v)| v))
}

// GET /poll?after=<last id the page has>: the messages after it. A freshly opened page (after=0) gets the
// waiting links and texts, and a ring only when it is fresh.
fn poll(after: u64) -> serde_json::Value {
    let now = now_secs();
    PHONE.lock().last_seen = now;
    let mut ob = OUTBOX.lock();
    ob.1.retain(|m| now.saturating_sub(m.at) <= MSG_TTL_SECS);
    // Jarvis was restarted while the page stayed open: its numbers began again
    let after = if after > ob.0 { 0 } else { after };
    let msgs: Vec<serde_json::Value> = ob
        .1
        .iter()
        // a page opened just now gets what is still waiting: a link or a text (up to two minutes), a ring only
        // when it is fresh (an old "find my phone" must not start sounding)
        .filter(|m| if after == 0 { m.kind != "ring" || now.saturating_sub(m.at) <= 30 } else { m.id > after })
        .map(|m| serde_json::json!({ "id": m.id, "kind": m.kind, "data": m.data }))
        .collect();
    serde_json::json!({ "last": ob.0, "msgs": msgs })
}

// POST /state: what the page knows about its phone
fn phone_state(body: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return false };
    let short = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|s| s.chars().take(120).collect::<String>()).unwrap_or_default();
    let mut p = PHONE.lock();
    p.last_seen = now_secs();
    p.battery = v.get("battery").and_then(|b| b.as_f64()).filter(|b| (0.0..=1.0).contains(b)).map(|b| (b * 100.0).round() as u8);
    p.charging = v.get("charging").and_then(|c| c.as_bool());
    p.visible = v.get("visible").and_then(|c| c.as_bool()).unwrap_or(false);
    p.ringing = v.get("ringing").and_then(|c| c.as_bool()).unwrap_or(false);
    p.voice = short("voice");
    p.voice_error = short("voice_error");
    true
}

#[cfg(test)]
pub(crate) static PHONE_TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

// tests of the commands: the remote "on" without a server, the phone "connected" or not, and what was queued
#[cfg(test)]
pub(crate) fn phone_for_tests(running: bool, phone_online: bool) {
    RUNNING.store(running, Ordering::SeqCst);
    PHONE.lock().last_seen = if phone_online { now_secs() } else { 0 };
    OUTBOX.lock().1.clear();
}

#[cfg(test)]
pub(crate) fn queued_for_tests() -> Vec<(String, String)> {
    OUTBOX.lock().1.iter().map(|m| (m.kind.to_string(), m.data.clone())).collect()
}


// the app gives us the channel that feeds typed commands into the main loop
// puts a phrase into the queue of typed commands (used by scripts: "run this as a normal command")
// a phrase that is a follow-up of the previous request, not a new question of the user
pub const INTERNAL_MARK: &str = "\u{1}";

pub fn push_internal(text: &str) -> bool {
    push_text(&format!("{}{}", INTERNAL_MARK, text))
}

pub fn push_text(text: &str) -> bool {
    match TX.get() {
        Some(tx) => tx.lock().send(text.to_string()).is_ok(),
        None => false,
    }
}

#[cfg(test)]
pub fn push_text_would_work_for_tests() -> bool {
    // in the tests no command queue is connected: the AI answer has nowhere to go as a command
    TX.get().is_some()
}

pub fn set_sender(tx: Sender<String>) {
    let _ = TX.set(Mutex::new(tx));
}

fn load_or_create_token() -> String {
    let path = APP_CONFIG_DIR.get().map(|d| d.join("remote_token"));
    if let Some(p) = &path {
        if let Ok(t) = std::fs::read_to_string(p) {
            let t = t.trim().to_string();
            if t.len() >= 16 {
                return t;
            }
        }
    }
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
    if let Some(p) = path {
        let _ = std::fs::write(p, &token);
    }
    token
}

fn local_ip() -> String {
    // connecting a UDP socket sends nothing, it only selects the outgoing interface
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "localhost".to_string())
}

// main link: plain http opens in every browser without a warning
pub fn url() -> String {
    format!("http://{}:{}/?t={}", local_ip(), PORT, *TOKEN)
}

// https link (own certificate): browsers warn once, but only here the phone microphone is allowed
pub fn url_https() -> String {
    format!("https://{}:{}/?t={}", local_ip(), PORT, *TOKEN)
}

pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn start() -> Result<String, String> {
    if is_running() {
        return Ok(url());
    }
    let tls = match tls_config() {
        Ok(c) => Some(c),
        Err(e) => {
            warn!("Remote control: https is not available ({}), plain http only", e);
            None
        }
    };
    // switched off a moment ago: the old listener closes within a fraction of a second, so try again briefly
    let mut attempt = 0;
    let listener = loop {
        match TcpListener::bind(("0.0.0.0", PORT)) {
            Ok(l) => break l,
            Err(e) if attempt < 10 => {
                attempt += 1;
                let _ = e;
                std::thread::sleep(Duration::from_millis(200));
            }
            Err(e) => return Err(format!("cannot listen on port {}: {}", PORT, e)),
        }
    };
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    RUNNING.store(true, Ordering::SeqCst);
    info!("Remote control started on port {}", PORT);

    std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let tls = tls.clone();
                    std::thread::spawn(move || {
                        let _ = serve(stream, tls);
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(150));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(500)),
            }
        }
        info!("Remote control stopped");
    });

    Ok(url())
}

pub fn stop() {
    RUNNING.store(false, Ordering::SeqCst);
}

struct Request {
    method: String,
    path: String,
    token_header: Option<String>,
    // request id of the phone page: a request that arrives twice is handled once
    rid: String,
    body: String,
}

fn read_request<S: Read>(stream: &mut S) -> std::io::Result<Request> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];
    let header_end = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 16 * 1024 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    };

    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let path = parts.next().unwrap_or("").to_string();

    let mut content_length = 0usize;
    let mut token_header = None;
    let mut rid = String::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_lowercase();
            let v = v.trim();
            if k == "content-length" {
                content_length = v.parse().unwrap_or(0);
            } else if k == "x-token" {
                token_header = Some(v.to_string());
            } else if k == "x-rid" {
                rid = v.chars().take(64).collect();
            }
        }
    }
    if content_length > 2000 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body_bytes = &buf[header_end..buf.len().min(header_end + content_length)];
    Ok(Request {
        method,
        path,
        token_header,
        rid,
        body: String::from_utf8_lossy(body_bytes).to_string(),
    })
}

fn respond<S: Write>(stream: &mut S, status: &str, content_type: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.as_bytes().len(),
        body
    );
}

// one port, two protocols: a TLS ClientHello starts with byte 0x16, plain http starts with a letter
fn serve(stream: TcpStream, tls: Option<std::sync::Arc<rustls::ServerConfig>>) -> std::io::Result<()> {
    if !is_running() {
        return Ok(()); // switched off: the connection is dropped without an answer
    }
    // on Windows an accepted connection inherits "non-blocking" from the listener: the first read would fail at once
    // ("would block") before the phone has sent its request, and the page would stay empty
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    let mut first = [0u8; 1];
    let n = stream.peek(&mut first)?;
    if n == 1 && first[0] == 0x16 {
        let config = tls.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::Other, "no tls"))?;
        let conn = rustls::ServerConnection::new(config).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        let mut tls_stream = rustls::StreamOwned::new(conn, stream);
        handle(&mut tls_stream)?;
        tls_stream.conn.send_close_notify();
        let _ = tls_stream.flush();
        Ok(())
    } else {
        let mut stream = stream;
        handle(&mut stream)
    }
}

// self-signed certificate for this PC's address, kept in the config dir (renewed when the address changes)
fn tls_config() -> Result<std::sync::Arc<rustls::ServerConfig>, String> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let ip = local_ip();
    let dir = APP_CONFIG_DIR.get().cloned();
    let (cert_path, key_path, ip_path) = match &dir {
        Some(d) => (Some(d.join("remote_cert.der")), Some(d.join("remote_key.der")), Some(d.join("remote_cert.ip"))),
        None => (None, None, None),
    };

    let mut cert_der: Option<Vec<u8>> = None;
    let mut key_der: Option<Vec<u8>> = None;
    if let (Some(c), Some(k), Some(i)) = (&cert_path, &key_path, &ip_path) {
        if std::fs::read_to_string(i).map(|v| v.trim() == ip).unwrap_or(false) {
            cert_der = std::fs::read(c).ok();
            key_der = std::fs::read(k).ok();
        }
    }
    if cert_der.is_none() || key_der.is_none() {
        let names = vec![ip.clone(), "localhost".to_string(), "127.0.0.1".to_string()];
        let generated = rcgen::generate_simple_self_signed(names).map_err(|e| e.to_string())?;
        let c = generated.cert.der().to_vec();
        let k = generated.key_pair.serialize_der();
        if let (Some(cp), Some(kp), Some(ip_p)) = (&cert_path, &key_path, &ip_path) {
            let _ = std::fs::write(cp, &c);
            let _ = std::fs::write(kp, &k);
            let _ = std::fs::write(ip_p, &ip);
        }
        cert_der = Some(c);
        key_der = Some(k);
    }

    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert_der.unwrap())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der.unwrap())),
        )
        .map_err(|e| e.to_string())?;
    Ok(std::sync::Arc::new(config))
}

fn token_in_path(path: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("t=").map(|v| v.to_string()))
}

fn handle<S: Read + Write>(stream: &mut S) -> std::io::Result<()> {
    let req = read_request(stream)?;

    if !is_running() {
        respond(stream, "503 Service Unavailable", "text/plain", "off");
        return Ok(());
    }

    let supplied = req.token_header.clone().or_else(|| token_in_path(&req.path));
    if supplied.as_deref() != Some(TOKEN.as_str()) {
        warn!("Remote control: request with a wrong token rejected");
        respond(stream, "403 Forbidden", "text/plain", "forbidden");
        return Ok(());
    }

    LAST_CLIENT.store(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1), Ordering::SeqCst);
    let route = req.path.split('?').next().unwrap_or("/");
    match (req.method.as_str(), route) {
        ("GET", "/") => respond(stream, "200 OK", "text/html", PAGE),
        ("GET", "/poll") => {
            let after = query_param(&req.path, "after").and_then(|a| a.parse().ok()).unwrap_or(0);
            respond(stream, "200 OK", "application/json", &poll(after).to_string());
        }
        ("POST", "/state") => {
            if phone_state(&req.body) {
                respond(stream, "200 OK", "text/plain", "ok");
            } else {
                respond(stream, "400 Bad Request", "text/plain", "bad state");
            }
        }
        ("POST", "/cmd") => {
            let text = req.body.trim().to_string();
            if text.is_empty() || text.chars().count() > 300 {
                respond(stream, "400 Bad Request", "text/plain", "bad command");
            } else if crate::guard::request_seen(&req.rid) {
                respond(stream, "200 OK", "text/plain", "ok");
            } else if let Some(tx) = TX.get() {
                info!("Remote control command: {}", text);
                let _ = tx.lock().send(text);
                respond(stream, "200 OK", "text/plain", "ok");
            } else {
                respond(stream, "503 Service Unavailable", "text/plain", "not ready");
            }
        }
        _ => respond(stream, "404 Not Found", "text/plain", "not found"),
    }
    Ok(())
}

const PAGE: &str = r#"<!doctype html>
<html lang="ru"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Jarvis</title>
<style>
  :root { color-scheme: dark; }
  body { margin: 0; padding: 12px 14px 24px; background: #0f1419; color: #e6edf3; font: 16px system-ui, sans-serif; }
  h1 { font-size: 20px; margin: 0 0 10px; }
  section { background: #161b22; border: 1px solid #30363d; border-radius: 14px; padding: 12px 14px; margin-bottom: 12px; }
  section h2 { font-size: 13px; letter-spacing: .06em; text-transform: uppercase; color: #8b949e; margin: 0 0 8px; }
  .row { display: flex; justify-content: space-between; gap: 10px; font-size: 15px; padding: 3px 0; }
  .row b { font-weight: 600; text-align: right; }
  .ok { color: #3fb950; } .bad { color: #ff7b72; } .dim { color: #8b949e; font-size: 13px; line-height: 1.45; }
  form { display: flex; gap: 8px; margin: 10px 0 4px; }
  input { flex: 1; min-width: 0; padding: 12px; border-radius: 10px; border: 1px solid #30363d; background: #0d1117; color: inherit; font-size: 16px; }
  button { padding: 12px 14px; border-radius: 10px; border: 0; background: #238636; color: #fff; font-size: 16px; }
  button.sec { background: #21262d; border: 1px solid #30363d; }
  button.wide { width: 100%; margin-top: 8px; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .grid button { background: #21262d; border: 1px solid #30363d; font-size: 15px; }
  #mic { width: 100%; padding: 18px; font-size: 18px; background: #1f6feb; }
  #mic.on { background: #da3633; }
  #heard { min-height: 22px; color: #58a6ff; margin: 8px 0 4px; }
  #status { min-height: 22px; color: #8b949e; margin-top: 8px; }
  .msg { border: 1px solid #1f6feb; background: #0d2142; border-radius: 12px; padding: 10px 12px; margin-top: 8px; }
  .msg .body { word-break: break-all; user-select: all; margin: 6px 0 8px; }
  .msg .acts { display: flex; gap: 8px; }
  #ringbox { display: none; position: fixed; inset: 0; background: rgba(218,54,51,.94); z-index: 9; text-align: center; padding-top: 30vh; font-size: 22px; }
  #ringbox button { font-size: 22px; padding: 18px 30px; background: #fff; color: #da3633; margin-top: 20px; }
  label.sw { display: flex; align-items: center; gap: 10px; padding: 6px 0; font-size: 15px; }
  label.sw input { flex: none; width: 22px; height: 22px; }
</style></head><body>
<h1>Jarvis — пульт</h1>
<div id="ringbox">📢 Компьютер ищет этот телефон<br><button id="ringstop" type="button">Я нашёл телефон</button></div>

<section id="g-conn"><h2>Подключение</h2>
  <div class="row"><span>Связь с компьютером</span><b id="c-state">проверяю…</b></div>
  <div class="row"><span>Последнее соединение</span><b id="c-last">—</b></div>
  <div class="dim" id="c-err"></div>
  <button class="sec wide" id="c-retry" type="button">Переподключиться</button>
  <label class="sw" id="wl-row" style="display:none"><input type="checkbox" id="wl"> Не гасить экран, пока открыт пульт</label>
  <div class="dim">Пульт работает, пока эта страница открыта на экране: свёрнутый браузер телефон усыпляет.</div>
</section>

<section id="g-phone"><h2>Состояние телефона</h2>
  <div class="row"><span>Заряд</span><b id="p-bat">—</b></div>
  <div class="dim" id="p-note"></div>
</section>

<section id="g-quick"><h2>Быстрые действия</h2>
  <button class="sec wide" id="snd" type="button" style="margin-top:0">🔔 Разрешить сигнал «Найти телефон»</button>
  <div class="dim" id="snd-note">Браузер играет звук только после одного нажатия на странице. Нажмите один раз — потом компьютер сможет позвать телефон командой «Найди телефон».</div>
  <div id="inbox"></div>
  <form id="f"><input id="t" placeholder="Команда для компьютера: таймер на 5 минут" autocomplete="off"><button>Отправить</button></form>
  <div class="grid" id="quick"></div>
  <div id="status"></div>
</section>

<section id="g-media"><h2>Мультимедиа</h2>
  <div class="dim">Эти кнопки управляют звуком и музыкой <b>компьютера</b>. Музыку и громкость других приложений самого телефона веб-страница переключать не может: Android разрешает это только установленному приложению. Для телефона пользуйтесь кнопками громкости и шторкой уведомлений.</div>
  <div class="grid" id="media" style="margin-top:8px"></div>
</section>

<section id="g-voice"><h2>Голос</h2>
  <button id="mic" type="button">🎤 Сказать команду</button>
  <label class="sw"><input type="checkbox" id="cont"> Непрерывный режим (слушать команду за командой)</label>
  <div class="row"><span>Микрофон</span><b id="v-state">выключен</b></div>
  <div id="heard"></div>
  <div class="dim">Речь распознаёт сам браузер телефона (в Chrome — сервис Google через интернет). Компьютер получает только готовый текст команды.</div>
  <div id="help" style="display:none;border-top:1px solid #30363d;margin-top:10px;padding-top:10px;font-size:14px;line-height:1.5">
    <b>Как включить голос на этом телефоне (один раз)</b><br>
    <span id="yandex" style="display:none;color:#ffb347">Яндекс.Браузер для этого не подходит. Откройте эту ссылку в <b>Chrome</b>.<br></span>
    1. В Chrome откройте в адресной строке: <b>chrome://flags/#unsafely-treat-insecure-origin-as-secure</b><br>
    2. В поле впишите адрес: <b id="origin" style="user-select:all;word-break:break-all"></b><br>
    3. Справа выберите <b>Enabled</b> и нажмите <b>Relaunch</b>.<br>
    4. Откройте ссылку пульта снова, нажмите 🎤 и разрешите микрофон.<br>
    <span class="dim">Не хотите настраивать? Нажмите на значок микрофона на клавиатуре телефона в поле команды: он диктует голосом и работает без настроек.</span>
  </div>
  <button id="howto" class="sec wide" type="button">Не работает голос? Как включить</button>
</section>

<section id="g-limits"><h2>Чего пульт не делает</h2>
  <div class="dim">Пульт — это страница в браузере, а не приложение. Поэтому он не блокирует и не усыпляет телефон, не читает уведомления телефона и не управляет его другими приложениями. Обойти экран блокировки, PIN-код или разрешения Android он не пытается.</div>
</section>
<script>
const token = new URLSearchParams(location.search).get('t') || '';
const $ = id => document.getElementById(id);
const rid = () => 'p' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
const quick = ['поставь таймер на 5 минут','отмени таймер','сколько времени','отмени выключение',
  'выключи компьютер через 10 минут','заблокируй компьютер','да','нет'];
const media = ['пауза','продолжи','следующий трек','предыдущий трек','громче','тише','выключи звук','включи звук'];
const status = $('status');
async function send(text) {
  status.textContent = '…';
  try {
    const r = await fetch('/cmd', {method:'POST', headers:{'X-Token':token, 'X-Rid':rid()}, body:text});
    status.textContent = r.ok ? '✓ ' + text : r.status === 503 ? 'Компьютер пока не принимает команды (Jarvis запускается или пульт выключен)'
      : r.status === 403 ? 'Ссылка не подходит: откройте новую ссылку или QR-код на компьютере' : 'Ошибка: ' + r.status;
    if (r.ok) connected();
  } catch (e) { status.textContent = 'Нет связи с компьютером'; }
}
for (const [list, box] of [[quick, 'quick'], [media, 'media']])
  for (const q of list) { const b = document.createElement('button'); b.type = 'button'; b.textContent = q; b.onclick = () => send(q); $(box).appendChild(b); }
$('f').onsubmit = e => { e.preventDefault(); const i = $('t'); if (i.value.trim()) send(i.value.trim()); i.value=''; };

// ---------- connection: the page asks the computer for news every 2 seconds ----------
let lastId = 0, lastOk = 0, failures = 0, timer = null, polling = false, shownId = 0;
const hhmm = t => new Date(t).toLocaleTimeString('ru-RU', {hour:'2-digit', minute:'2-digit', second:'2-digit'});
function connected() { lastOk = Date.now(); failures = 0; $('c-state').textContent = 'есть'; $('c-state').className = 'ok'; $('c-err').textContent = ''; $('c-last').textContent = hhmm(lastOk); }
function lost(why) { failures++; $('c-state').textContent = 'нет связи'; $('c-state').className = 'bad'; $('c-err').textContent = why; }
async function poll() {
  clearTimeout(timer);
  if (polling) return; // one request at a time: two at once would show the same message twice
  polling = true;
  try {
    const r = await fetch('/poll?after=' + lastId, {headers:{'X-Token':token}});
    if (r.status === 403) { lost('Ссылка не подходит: откройте новую ссылку или QR-код на компьютере.'); }
    else if (r.status === 503) { lost('Пульт выключен на компьютере. Включите его на главной странице Jarvis.'); }
    else if (!r.ok) { lost('Компьютер ответил ошибкой ' + r.status + '.'); }
    else { const j = await r.json(); connected(); for (const m of j.msgs) { if (m.id > shownId || j.last < shownId) { handle(m); shownId = m.id; } } lastId = j.last; if (j.last < shownId) shownId = j.last; }
  } catch (e) { lost('Компьютер не отвечает: Jarvis закрыт, пульт выключен на компьютере или телефон в другой Wi-Fi сети.'); }
  polling = false;
  timer = setTimeout(poll, failures ? Math.min(2000 * (failures + 1), 10000) : 2000);
}
$('c-retry').onclick = () => { failures = 0; $('c-state').textContent = 'проверяю…'; $('c-state').className = ''; poll(); reportSoon(); };

// ---------- what the phone tells the computer: battery, screen, voice ----------
let battery = null, reportTimer = null;
const state = { voice: 'off', voice_error: '' };
async function report() {
  const body = { battery: battery ? battery.level : null, charging: battery ? battery.charging : null,
    visible: document.visibilityState === 'visible', ringing: ringing, voice: state.voice, voice_error: state.voice_error };
  try { await fetch('/state', {method:'POST', headers:{'X-Token':token, 'Content-Type':'application/json'}, body: JSON.stringify(body)}); } catch (e) {}
}
function reportSoon() { clearTimeout(reportTimer); reportTimer = setTimeout(report, 200); }
setInterval(report, 15000);
document.addEventListener('visibilitychange', () => { reportSoon(); if (document.visibilityState === 'visible') { poll(); keepAwake(); } });
function showBattery() {
  if (!battery) return;
  $('p-bat').textContent = Math.round(battery.level * 100) + '%' + (battery.charging ? ' ⚡ заряжается' : '');
  reportSoon();
}
if (navigator.getBattery) navigator.getBattery().then(b => { battery = b; b.onlevelchange = b.onchargingchange = showBattery; showBattery(); }).catch(() => { $('p-note').textContent = 'Браузер не сообщает заряд.'; });
else $('p-note').textContent = 'Этот браузер не сообщает заряд (умеет Chrome на Android).';

// keep the screen on while the remote is open (Screen Wake Lock: Chrome, secure page only)
let wakeLock = null;
async function keepAwake() { if ($('wl').checked && 'wakeLock' in navigator && !wakeLock) { try { wakeLock = await navigator.wakeLock.request('screen'); wakeLock.onrelease = () => { wakeLock = null; }; } catch (e) { $('c-err').textContent = 'Не удалось держать экран включённым: ' + e.message; $('wl').checked = false; } } }
if ('wakeLock' in navigator && window.isSecureContext) $('wl-row').style.display = 'flex';
$('wl').onchange = () => { if ($('wl').checked) keepAwake(); else if (wakeLock) wakeLock.release(); };

// ---------- messages from the computer ----------
let audio = null, ringing = false, ringTimer = null;
function unlockSound() { try { audio = audio || new (window.AudioContext || window.webkitAudioContext)(); audio.resume(); } catch (e) {} }
$('snd').onclick = () => { unlockSound(); beep(0); $('snd').textContent = '🔔 Сигнал разрешён'; $('snd-note').textContent = 'Готово. Команда «Найди телефон» на компьютере включит здесь громкий сигнал.'; };
document.addEventListener('pointerdown', unlockSound, {once: true});
function beep(i) {
  if (!audio) return;
  const o = audio.createOscillator(), g = audio.createGain();
  o.type = 'square'; o.frequency.value = i % 2 ? 1320 : 880; g.gain.value = 0.6;
  o.connect(g); g.connect(audio.destination); o.start(); o.stop(audio.currentTime + 0.35);
}
function ring() {
  if (ringing) return;
  ringing = true; $('ringbox').style.display = 'block'; reportSoon();
  let i = 0; const started = Date.now();
  const tick = () => { if (!ringing) return; beep(i++); if (navigator.vibrate) navigator.vibrate(300);
    if (Date.now() - started > 60000) { stopRing(); return; } ringTimer = setTimeout(tick, 450); };
  tick();
  if (!audio || audio.state !== 'running') $('ringbox').firstChild.textContent = '📢 Компьютер ищет этот телефон (звук не разрешён: нажмите «Разрешить сигнал» заранее) ';
}
function stopRing() { ringing = false; clearTimeout(ringTimer); if (navigator.vibrate) navigator.vibrate(0); $('ringbox').style.display = 'none'; reportSoon(); }
$('ringstop').onclick = () => { stopRing(); send('останови сигнал телефона'); };
function copy(text, btn) {
  const done = () => { btn.textContent = 'Скопировано'; };
  if (navigator.clipboard && window.isSecureContext) navigator.clipboard.writeText(text).then(done, () => fallback());
  else fallback();
  function fallback() { const t = document.createElement('textarea'); t.value = text; document.body.appendChild(t); t.select(); try { document.execCommand('copy'); done(); } catch (e) { btn.textContent = 'Выделите текст и скопируйте'; } t.remove(); }
}
function card(title, body, buttons) {
  const d = document.createElement('div'); d.className = 'msg';
  const h = document.createElement('div'); h.textContent = title + ' · ' + hhmm(Date.now()); h.className = 'dim';
  const b = document.createElement('div'); b.className = 'body'; b.textContent = body;
  const a = document.createElement('div'); a.className = 'acts';
  for (const [label, fn] of buttons) { const x = document.createElement('button'); x.type = 'button'; x.textContent = label; x.onclick = () => fn(x); a.appendChild(x); }
  const close = document.createElement('button'); close.type = 'button'; close.className = 'sec'; close.textContent = '✕'; close.onclick = () => d.remove(); a.appendChild(close);
  d.append(h, b, a); $('inbox').prepend(d);
  while ($('inbox').children.length > 5) $('inbox').lastChild.remove();
  if (navigator.vibrate) navigator.vibrate(120);
}
function handle(m) {
  if (m.kind === 'ring') ring();
  else if (m.kind === 'stop') stopRing();
  else if (m.kind === 'link' && /^https?:\/\//i.test(m.data)) card('Ссылка с компьютера', m.data, [['Открыть', () => window.open(m.data, '_blank', 'noopener')], ['Копировать', x => copy(m.data, x)]]);
  else if (m.kind === 'text') card('Текст с компьютера', m.data, [['Копировать', x => copy(m.data, x)]]);
}

// ---------- voice: the browser listens through the phone microphone and sends the recognized text ----------
// Browsers allow the microphone only on secure pages (https); on plain http over Wi-Fi it may be refused.
const micBtn = $('mic'), heard = $('heard');
const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
let rec = null, wantVoice = false;
$('origin').textContent = location.origin;
if (/YaBrowser/i.test(navigator.userAgent)) $('yandex').style.display = 'inline';
$('howto').onclick = () => { $('help').style.display = $('help').style.display === 'none' ? 'block' : 'none'; };
function voice(st, err) {
  state.voice = st; state.voice_error = err || '';
  const names = { off: 'выключен', listening: 'слушаю', error: 'ошибка', unavailable: 'недоступен' };
  $('v-state').textContent = names[st] || st; $('v-state').className = st === 'listening' ? 'ok' : (st === 'error' || st === 'unavailable') ? 'bad' : '';
  if (err) heard.textContent = err;
  reportSoon();
}
if (!window.isSecureContext) { voice('unavailable', 'Голос: нужна одна настройка браузера (кнопка ниже). Текстом всё работает.'); }
else if (!SR) { voice('unavailable', 'Этот браузер не умеет распознавать речь. Используйте Chrome или микрофон на клавиатуре.'); }
function listen() {
  if (!window.isSecureContext) { $('help').style.display = 'block'; voice('unavailable', 'Браузер не разрешает микрофон на этой странице. Сделайте настройку ниже.'); wantVoice = false; return; }
  if (!SR) { voice('unavailable', 'Браузер не умеет слушать. Нажмите значок микрофона на клавиатуре в поле команды.'); wantVoice = false; return; }
  rec = new SR(); rec.lang = 'ru-RU'; rec.interimResults = true; rec.continuous = false;
  micBtn.classList.add('on'); micBtn.textContent = '🔴 Слушаю… (нажмите, чтобы остановить)'; voice('listening');
  let final = '', fatal = false;
  rec.onresult = e => { let t = ''; for (const r of e.results) { t += r[0].transcript; if (r.isFinal) final = t; } heard.textContent = '«' + t + '»'; };
  rec.onerror = e => {
    if (e.error === 'no-speech' || e.error === 'aborted') return; // silence: in the continuous mode just listen again
    fatal = true; wantVoice = false; $('cont').checked = false;
    voice('error', (e.error === 'not-allowed' || e.error === 'service-not-allowed')
      ? 'Нет доступа к микрофону. Разрешите микрофон для этой страницы в настройках сайта или сделайте настройку из подсказки ниже.'
      : e.error === 'network' ? 'Распознавание речи недоступно: у телефона нет интернета.' : 'Ошибка микрофона: ' + e.error);
  };
  rec.onend = () => {
    rec = null; micBtn.classList.remove('on'); micBtn.textContent = '🎤 Сказать команду';
    if (final.trim()) send(final.trim());
    if (wantVoice && !fatal) setTimeout(() => { if (wantVoice && !rec) listen(); }, 300);
    else if (!fatal) voice('off');
  };
  try { rec.start(); } catch (e) { rec = null; micBtn.classList.remove('on'); voice('error', 'Не удалось включить микрофон: ' + e.message); wantVoice = false; }
}
micBtn.onclick = () => { if (rec) { wantVoice = false; $('cont').checked = false; rec.stop(); return; } wantVoice = $('cont').checked; listen(); };
$('cont').onchange = () => { wantVoice = $('cont').checked; if (wantVoice && !rec) listen(); if (!wantVoice && rec) rec.stop(); };

poll(); report();
</script></body></html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    // the same port answers https (self-signed) and plain http
    #[test]
    fn https_and_http_on_one_port() {
        let _g = PHONE_TEST_LOCK.lock();
        start().expect("start");
        std::thread::sleep(Duration::from_millis(500));
        for scheme in ["https", "http"] {
            let out = std::process::Command::new("curl")
                .args(["-sk", "--max-time", "5", &format!("{}://127.0.0.1:{}/?t={}", scheme, PORT, *TOKEN)])
                .output()
                .expect("curl");
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(text.contains("Сказать команду"), "{} page: {}", scheme, text);
        }
        stop();
    }

    #[test]
    fn the_phone_gets_only_real_messages_and_an_old_signal_never_sounds() {
        let _g = PHONE_TEST_LOCK.lock();
        RUNNING.store(false, Ordering::SeqCst);
        assert!(send_to_phone("ring", "").unwrap_err().contains("выключен"), "with the remote off nothing pretends to be sent");
        RUNNING.store(true, Ordering::SeqCst);
        assert!(send_to_phone("lock", "").is_err(), "no fake actions: locking the phone is not something a page can do");
        assert!(send_to_phone("link", "javascript:alert(1)").is_err());
        assert!(send_to_phone("link", "not a link").is_err());
        assert!(send_to_phone("text", "   ").is_err());
        assert!(send_to_phone("text", &"я".repeat(4001)).is_err());

        *PHONE.lock() = Phone::default();
        let start = poll(u64::MAX)["last"].as_u64().unwrap(); // a page left from before a restart gets the current number
        *PHONE.lock() = Phone::default();
        assert!(send_to_phone("ring", "").unwrap().contains("не на связи"), "honest: nobody has the page open");
        let r = poll(start);
        assert_eq!(r["msgs"].as_array().unwrap().len(), 1);
        assert_eq!(r["msgs"][0]["kind"], "ring");
        let after = r["last"].as_u64().unwrap();
        assert!(poll(after)["msgs"].as_array().unwrap().is_empty(), "a message is delivered once");

        assert_eq!(send_to_phone("link", "https://example.com/a?b=1").unwrap(), "отправлено на телефон", "the phone polled just now");
        send_to_phone("text", "привет").unwrap();
        let r = poll(after);
        let kinds: Vec<_> = r["msgs"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, ["link", "text"]);
        assert_eq!(r["msgs"][1]["data"], "привет");

        // a signal nobody picked up is cancelled by "stop", and an expired one is dropped
        let last = r["last"].as_u64().unwrap();
        send_to_phone("ring", "").unwrap();
        send_to_phone("stop", "").unwrap();
        let kinds: Vec<_> = poll(last)["msgs"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, ["stop"]);
        let last = poll(last)["last"].as_u64().unwrap();
        send_to_phone("ring", "").unwrap();
        OUTBOX.lock().1.back_mut().unwrap().at -= MSG_TTL_SECS + 1;
        assert!(poll(last)["msgs"].as_array().unwrap().is_empty(), "an hour-old signal must not sound");
        OUTBOX.lock().1.clear();

        // a page opened later (after=0) still gets a waiting text, but not an old ring
        send_to_phone("text", "ждёт").unwrap();
        send_to_phone("ring", "").unwrap();
        for m in OUTBOX.lock().1.iter_mut() {
            m.at -= 60;
        }
        let kinds: Vec<_> = poll(0)["msgs"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap().to_string()).collect();
        assert_eq!(kinds, ["text"]);
        OUTBOX.lock().1.clear();
        RUNNING.store(false, Ordering::SeqCst);
    }

    #[test]
    fn the_phone_state_is_shown_without_inventing_anything() {
        let _g = PHONE_TEST_LOCK.lock();
        *PHONE.lock() = Phone::default();
        RUNNING.store(true, Ordering::SeqCst);
        let j = phone_json();
        assert_eq!(j["online"], false);
        assert!(j["battery"].is_null() && j["last_seen"].is_null(), "unknown stays unknown");
        assert!(!phone_state("not json"));
        assert!(phone_state(r#"{"battery":0.834,"charging":true,"visible":true,"voice":"listening","voice_error":"","ringing":false}"#));
        let j = phone_json();
        assert_eq!(j["online"], true);
        assert_eq!(j["battery"], 83);
        assert_eq!(j["charging"], true);
        assert_eq!(j["voice"], "listening");
        assert!(phone_state(r#"{"battery":7,"voice":"error","voice_error":"Нет доступа к микрофону"}"#));
        let j = phone_json();
        assert!(j["battery"].is_null(), "a wrong value is not shown as a battery level");
        assert!(j["charging"].is_null());
        assert_eq!(j["voice_error"], "Нет доступа к микрофону");
        PHONE.lock().last_seen -= PHONE_ONLINE_SECS + 1;
        assert_eq!(phone_json()["online"], false, "silence means the connection is lost");
        RUNNING.store(false, Ordering::SeqCst);
        assert_eq!(phone_json()["online"], false);
        *PHONE.lock() = Phone::default();
    }
}
