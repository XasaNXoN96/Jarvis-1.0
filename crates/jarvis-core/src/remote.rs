// Remote control from a phone / another device on the same Wi-Fi.
//
// OFF by default. "включи удалённое управление" starts a tiny HTTP server on port 9713;
// the phone opens http://<pc-ip>:9713/?t=<token> and sends text commands (same as typing
// them in the console). Every request must carry the secret token, which is created once
// and stored in the config dir (remote_token). Do not share the link.

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
static TOKEN: Lazy<String> = Lazy::new(load_or_create_token);

// the app gives us the channel that feeds typed commands into the main loop
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

pub fn url() -> String {
    format!("http://{}:{}/?t={}", local_ip(), PORT, *TOKEN)
}

pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn start() -> Result<String, String> {
    if is_running() {
        return Ok(url());
    }
    let listener = TcpListener::bind(("0.0.0.0", PORT))
        .map_err(|e| format!("cannot listen on port {}: {}", PORT, e))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    RUNNING.store(true, Ordering::SeqCst);
    info!("Remote control started on port {}", PORT);

    std::thread::spawn(move || {
        while RUNNING.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    std::thread::spawn(move || {
                        let _ = handle(stream);
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
    body: String,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
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
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_lowercase();
            let v = v.trim();
            if k == "content-length" {
                content_length = v.parse().unwrap_or(0);
            } else if k == "x-token" {
                token_header = Some(v.to_string());
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
        body: String::from_utf8_lossy(body_bytes).to_string(),
    })
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

fn token_in_path(path: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("t=").map(|v| v.to_string()))
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    let req = read_request(&mut stream)?;

    if !is_running() {
        respond(&mut stream, "503 Service Unavailable", "text/plain", "off");
        return Ok(());
    }

    let supplied = req.token_header.clone().or_else(|| token_in_path(&req.path));
    if supplied.as_deref() != Some(TOKEN.as_str()) {
        warn!("Remote control: request with a wrong token rejected");
        respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
        return Ok(());
    }

    let route = req.path.split('?').next().unwrap_or("/");
    match (req.method.as_str(), route) {
        ("GET", "/") => respond(&mut stream, "200 OK", "text/html", PAGE),
        ("POST", "/cmd") => {
            let text = req.body.trim().to_string();
            if text.is_empty() || text.chars().count() > 300 {
                respond(&mut stream, "400 Bad Request", "text/plain", "bad command");
            } else if let Some(tx) = TX.get() {
                info!("Remote control command: {}", text);
                let _ = tx.lock().send(text);
                respond(&mut stream, "200 OK", "text/plain", "ok");
            } else {
                respond(&mut stream, "503 Service Unavailable", "text/plain", "not ready");
            }
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", "not found"),
    }
    Ok(())
}

const PAGE: &str = r#"<!doctype html>
<html lang="ru"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Jarvis</title>
<style>
  :root { color-scheme: dark; }
  body { margin: 0; padding: 16px; background: #0f1419; color: #e6edf3; font: 16px system-ui, sans-serif; }
  h1 { font-size: 20px; margin: 0 0 12px; }
  form { display: flex; gap: 8px; margin-bottom: 16px; }
  input { flex: 1; padding: 12px; border-radius: 10px; border: 1px solid #30363d; background: #161b22; color: inherit; font-size: 16px; }
  button { padding: 12px 14px; border-radius: 10px; border: 0; background: #238636; color: #fff; font-size: 16px; }
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
  .grid button { background: #21262d; border: 1px solid #30363d; }
  #mic { width: 100%; padding: 18px; margin-bottom: 14px; font-size: 18px; background: #1f6feb; }
  #mic.on { background: #da3633; }
  #heard { min-height: 22px; color: #58a6ff; margin-bottom: 8px; }
  #status { margin-top: 14px; min-height: 22px; color: #8b949e; }
</style></head><body>
<h1>Jarvis</h1>
<button id="mic" type="button">🎤 Сказать команду</button>
<div id="heard"></div>
<form id="f"><input id="t" placeholder="Команда, например: таймер на 5 минут" autocomplete="off"><button>Отправить</button></form>
<div class="grid" id="quick"></div>
<div id="status"></div>
<script>
const token = new URLSearchParams(location.search).get('t') || '';
const quick = ['поставь таймер на 5 минут','отмени таймер','громче','тише','выключи звук','включи звук',
  'пауза','следующий трек','сколько времени','отмени выключение','выключи компьютер через 10 минут','заблокируй компьютер'];
const status = document.getElementById('status');
async function send(text) {
  status.textContent = '…';
  try {
    const r = await fetch('/cmd', {method:'POST', headers:{'X-Token':token}, body:text});
    status.textContent = r.ok ? '✓ ' + text : 'Ошибка: ' + r.status;
  } catch (e) { status.textContent = 'Нет связи с компьютером'; }
}
// voice: the browser listens through the phone microphone and sends the recognized text as a command.
// Browsers allow the microphone only on secure pages (https); on plain http over Wi-Fi it may be refused.
// Then use the microphone button on the phone keyboard in the text field below - it works everywhere.
const micBtn = document.getElementById('mic'), heard = document.getElementById('heard');
const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
let rec = null;
micBtn.onclick = () => {
  if (!SR) { heard.textContent = 'Браузер не умеет слушать. Нажмите значок микрофона на клавиатуре в поле ниже.'; return; }
  if (rec) { rec.stop(); return; }
  rec = new SR(); rec.lang = 'ru-RU'; rec.interimResults = true; rec.continuous = false;
  micBtn.classList.add('on'); micBtn.textContent = '🔴 Слушаю… (нажмите, чтобы остановить)';
  let final = '';
  rec.onresult = e => { let t = ''; for (const r of e.results) { t += r[0].transcript; if (r.isFinal) final = t; } heard.textContent = '«' + t + '»'; };
  rec.onerror = e => { heard.textContent = (e.error === 'not-allowed' || e.error === 'service-not-allowed')
    ? 'Браузер не дал доступ к микрофону на http. Используйте микрофон на клавиатуре в поле ниже.' : 'Ошибка микрофона: ' + e.error; };
  rec.onend = () => { micBtn.classList.remove('on'); micBtn.textContent = '🎤 Сказать команду'; rec = null; if (final.trim()) send(final.trim()); };
  try { rec.start(); } catch (e) { rec = null; micBtn.classList.remove('on'); }
};
document.getElementById('f').onsubmit = e => { e.preventDefault(); const i = document.getElementById('t'); if (i.value.trim()) send(i.value.trim()); i.value=''; };
for (const q of quick) { const b = document.createElement('button'); b.textContent = q; b.onclick = () => send(q); document.getElementById('quick').appendChild(b); }
</script></body></html>
"#;
