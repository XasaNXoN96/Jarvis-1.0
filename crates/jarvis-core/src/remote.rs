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
    let listener = TcpListener::bind(("0.0.0.0", PORT))
        .map_err(|e| format!("cannot listen on port {}: {}", PORT, e))?;
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

    let route = req.path.split('?').next().unwrap_or("/");
    match (req.method.as_str(), route) {
        ("GET", "/") => respond(stream, "200 OK", "text/html", PAGE),
        ("POST", "/cmd") => {
            let text = req.body.trim().to_string();
            if text.is_empty() || text.chars().count() > 300 {
                respond(stream, "400 Bad Request", "text/plain", "bad command");
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
    ? 'Нет доступа к микрофону. Откройте страницу по ссылке https (нажмите «Дополнительно → Продолжить»), разрешите микрофон. Или используйте микрофон на клавиатуре в поле ниже.' : 'Ошибка микрофона: ' + e.error; };
  rec.onend = () => { micBtn.classList.remove('on'); micBtn.textContent = '🎤 Сказать команду'; rec = null; if (final.trim()) send(final.trim()); };
  try { rec.start(); } catch (e) { rec = null; micBtn.classList.remove('on'); }
};
document.getElementById('f').onsubmit = e => { e.preventDefault(); const i = document.getElementById('t'); if (i.value.trim()) send(i.value.trim()); i.value=''; };
for (const q of quick) { const b = document.createElement('button'); b.textContent = q; b.onclick = () => send(q); document.getElementById('quick').appendChild(b); }
</script></body></html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    // the same port answers https (self-signed) and plain http
    #[test]
    fn https_and_http_on_one_port() {
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
}
