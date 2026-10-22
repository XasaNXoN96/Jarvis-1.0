// Remote control from a phone / another device on the same Wi-Fi.
//
// OFF by default. "включи удалённое управление" starts a tiny HTTP server on port 9713;
// the phone opens https://<pc-ip>:9713/?t=<token> and sends text commands (same as typing
// them in the console). Every request must carry the secret token, which is created once
// and stored in the config dir (remote_token). Do not share the link.
//
// The same port speaks https (self-signed certificate made on this PC, the browser warns once) and
// plain http. Browsers give the microphone only to https pages, so the phone page uses https for voice.

use crate::pairing;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;

use once_cell::sync::{Lazy, OnceCell};
use parking_lot::Mutex;
use rand::RngCore;

use crate::origin::{Incoming, Source};
use crate::APP_CONFIG_DIR;

pub const PORT: u16 = 9713;

static TX: OnceCell<Mutex<Sender<Incoming>>> = OnceCell::new();
static RUNNING: AtomicBool = AtomicBool::new(false);
// when a device with the right link last opened the remote page or sent a command (unix seconds, 0 = never)
static LAST_CLIENT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// the secret of the link, only so that the diagnostics report can remove it (never created here)
pub fn token_for_diag() -> String {
    Lazy::get(&TOKEN).map(|t| t.lock().clone()).unwrap_or_default()
}

fn token() -> String {
    TOKEN.lock().clone()
}

// the secret for a link of the tunnel (made at the first use, like the links of the home network)
pub fn link_token() -> String {
    token()
}

#[cfg(test)]
pub(crate) fn test_touch_client() {
    LAST_CLIENT.store(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(1), Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) fn test_untouch_client() {
    LAST_CLIENT.store(0, Ordering::SeqCst);
}

pub fn last_client() -> Option<u64> {
    match LAST_CLIENT.load(Ordering::SeqCst) { 0 => None, t => Some(t) }
}
// the secret of the link; "Новая ссылка" replaces it, so every phone that had the old link loses access
static TOKEN: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(load_or_create_token()));

// ---------- the phone side of the remote ----------
// The remote is a web page in the phone's browser (no Android app). The page polls the PC every two seconds
// (GET /poll) and reports what a web page may know about its phone (POST /state): battery (Chrome's Battery API),
// whether the page is on screen, the state of its voice mode. The PC can send the phone only what a page can do:
// a sound signal to find it, a link to open, a text to copy. Media of other apps, locking the phone and its
// notifications are out of reach of a web page and are not offered.

// %XX and + of a form field
fn url_decode(v: &str) -> String {
    let b = v.as_bytes();
    let hex = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match (hex(b[i + 1]), hex(b[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h * 16 + l);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

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
    // where the phone is (only when its owner switched the sharing on in the page): latitude, longitude, accuracy in metres, when (seconds)
    location: Option<(f64, f64, f64, u64)>,
    // the network the phone reports: "wifi", "4g", "offline" … (what the browser says, nothing more)
    net: String,
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
        "location": p.location.map(|(lat, lon, acc, at)| serde_json::json!({ "lat": lat, "lon": lon, "accuracy": acc, "at": at })),
        "net": p.net,
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

// What Jarvis answers and what happens on the computer reaches the phone too: the user who steers the computer from the
// phone (no microphone on the PC, another room, another network through the tunnel) must not have to look at the PC.
static LAST_PHONE_CMD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
// an answer belongs to the phone when its command came from the phone not long ago
const ANSWER_WINDOW_SECS: u64 = 120;

fn note_phone_command() {
    LAST_PHONE_CMD.store(now_secs().max(1), Ordering::SeqCst);
}

fn enqueue(kind: &'static str, data: String) {
    let now = now_secs();
    let mut ob = OUTBOX.lock();
    ob.0 += 1;
    let id = ob.0;
    ob.1.push_back(Msg { id, kind, data, at: now });
    while ob.1.len() > 20 {
        ob.1.pop_front();
    }
}

// an answer of Jarvis to a command of the phone (the text is private: only its size goes to the log)
pub fn announce_answer(text: &str) {
    let text = text.trim();
    if text.is_empty() || !is_running() {
        return;
    }
    let last = LAST_PHONE_CMD.load(Ordering::SeqCst);
    if last == 0 || now_secs().saturating_sub(last) > ANSWER_WINDOW_SECS {
        return;
    }
    enqueue("say", text.chars().take(1000).collect());
}

// a timer, an alarm, a reminder, a finished task, an error: always (the moment matters, whoever asked)
pub fn announce_note(kind: &str, title: &str, text: &str) {
    if !is_running() {
        return;
    }
    let data = serde_json::json!({ "kind": kind, "title": title.chars().take(120).collect::<String>(), "text": text.chars().take(600).collect::<String>() }).to_string();
    enqueue("note", data);
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
        .filter(|m| if after == 0 { !matches!(m.kind, "ring" | "say" | "note") || now.saturating_sub(m.at) <= 30 } else { m.id > after })
        .map(|m| serde_json::json!({ "id": m.id, "kind": m.kind, "data": m.data }))
        .collect();
    // "now": the clock of the computer; the page keeps the difference to its own clock, so a phone whose clock is wrong still sends valid times (replay.rs)
    serde_json::json!({ "last": ob.0, "msgs": msgs, "now": now })
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
    p.net = short("net").chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').take(16).collect();
    // the place: only real numbers in the real ranges; a page that stopped sharing sends none and the old place is dropped
    p.location = match (v.get("lat").and_then(|x| x.as_f64()), v.get("lon").and_then(|x| x.as_f64())) {
        (Some(lat), Some(lon)) if (-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon) => {
            let acc = v.get("acc").and_then(|x| x.as_f64()).filter(|a| a.is_finite() && *a >= 0.0).unwrap_or(0.0).min(100_000.0);
            Some((lat, lon, acc, now_secs()))
        }
        _ => None,
    };
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


// the app gives us the channel that feeds typed commands into the main loop.
// A phrase for the command handler goes in ONLY through `submit`, with the door it came through: the source is set by
// the code of the entrance, and nothing in the text is read as a source (see origin.rs)
pub fn submit(source: Source, text: &str) -> bool {
    submit_incoming(Incoming::new(source, text))
}

// a script asked ("jarvis.app.run_text"): the phrase keeps the door of the command that was running
pub fn submit_from_script(text: &str) -> bool {
    let mut incoming = Incoming::new(crate::origin::current().unwrap_or(Source::Script), text);
    incoming.via_script = true;
    submit_incoming(incoming)
}

// a phrase that is a follow-up of the previous request, not a new question of the user
pub fn push_internal(text: &str) -> bool {
    submit(Source::Internal, text)
}

fn submit_incoming(incoming: Incoming) -> bool {
    if incoming.text.is_empty() {
        return false;
    }
    #[cfg(test)]
    if capturing(Some(incoming.source)) {
        SUBMITTED.lock().push(incoming);
        return true;
    }
    match TX.get() {
        Some(tx) => tx.lock().send(incoming).is_ok(),
        None => false,
    }
}

// tests: phrases are caught here instead of going to the (not running) main loop. A test catches the phrases of its own
// thread; the Telegram bot works on a thread of its own, so a test of it catches the phrases of that door from any thread
// (what other tests submit at the same time never lands in a list that is being read)
#[cfg(test)]
struct CaptureSpec {
    thread: Option<std::thread::ThreadId>,
    only: Option<Source>,
}
#[cfg(test)]
static CAPTURE: Mutex<Option<CaptureSpec>> = parking_lot::const_mutex(None);
#[cfg(test)]
static SUBMITTED: Lazy<Mutex<Vec<Incoming>>> = Lazy::new(|| Mutex::new(Vec::new()));
#[cfg(test)]
pub(crate) static CAPTURE_LOCK: Mutex<()> = parking_lot::const_mutex(());

#[cfg(test)]
fn capturing(source: Option<Source>) -> bool {
    match CAPTURE.lock().as_ref() {
        Some(spec) => match spec.thread {
            Some(t) => t == std::thread::current().id(),
            None => source.is_none() || spec.only == source,
        },
        None => false,
    }
}

#[cfg(test)]
pub(crate) struct Capture(parking_lot::MutexGuard<'static, ()>);

#[cfg(test)]
impl Capture {
    pub(crate) fn start() -> Capture {
        let g = CAPTURE_LOCK.lock();
        SUBMITTED.lock().clear();
        *CAPTURE.lock() = Some(CaptureSpec { thread: Some(std::thread::current().id()), only: None });
        Capture(g)
    }
    // the phrases of one door from any thread (the Telegram bot)
    pub(crate) fn start_for(source: Source) -> Capture {
        let g = CAPTURE_LOCK.lock();
        SUBMITTED.lock().clear();
        *CAPTURE.lock() = Some(CaptureSpec { thread: None, only: Some(source) });
        Capture(g)
    }
    pub(crate) fn taken(&self) -> Vec<Incoming> {
        std::mem::take(&mut *SUBMITTED.lock())
    }
}

#[cfg(test)]
impl Drop for Capture {
    fn drop(&mut self) {
        *CAPTURE.lock() = None;
    }
}

#[cfg(test)]
pub fn command_queue_connected_for_tests() -> bool {
    // in the tests no command queue is connected: the AI answer has nowhere to go as a command
    TX.get().is_some()
}

// the command queue is being read (the main loop of the app runs): before that a phrase would wait unseen
static QUEUE_READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn mark_queue_ready() {
    QUEUE_READY.store(true, std::sync::atomic::Ordering::SeqCst);
}

pub fn queue_ready() -> bool {
    #[cfg(test)]
    if capturing(None) {
        return true;
    }
    QUEUE_READY.load(std::sync::atomic::Ordering::SeqCst) && TX.get().is_some()
}

pub fn set_sender(tx: Sender<Incoming>) {
    let _ = TX.set(Mutex::new(tx));
}

fn new_secret() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn token_path() -> Option<std::path::PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join("remote_token"))
}

fn load_or_create_token() -> String {
    if let Some(p) = token_path() {
        if let Ok(t) = std::fs::read_to_string(&p) {
            let t = t.trim().to_string();
            if t.len() >= 16 {
                return t;
            }
        }
    }
    let token = new_secret();
    if let Some(p) = token_path() {
        if crate::write_atomic(&p, &token).is_ok() {
            let _ = crate::acl::private_file(&p);
        }
    }
    token
}

// "Новая ссылка": the old link stops working on every phone at once (a lost phone, a link shown to someone)
// The new secret is written first: a link that is revoked only in memory would work again after a restart, so a
// failed write keeps the old link and says so.
pub fn renew_token() -> Result<String, String> {
    let token = new_secret();
    if let Some(p) = token_path() {
        crate::write_atomic(&p, &token).map_err(|e| format!("не удалось сохранить новую ссылку ({}), старая продолжает работать", e))?;
        let _ = crate::acl::private_file(&p);
    }
    *TOKEN.lock() = token.clone();
    PHONE.lock().last_seen = 0;
    info!("Remote control: a new link was made, the old one no longer works");
    Ok(token)
}

// the secret is compared in constant time (the answer time does not tell how many characters were right)
fn token_matches(supplied: Option<&str>, expected: &str) -> bool {
    let Some(s) = supplied else { return false };
    let (a, b) = (s.as_bytes(), expected.as_bytes());
    if a.len() != b.len() || b.is_empty() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// ---------- the addresses of this PC that a phone can reach ----------
// A PC has several: the home Wi-Fi / Ethernet, the Windows mobile hotspot, a VPN (Tailscale, ZeroTier), virtual
// adapters of VirtualBox / Hyper-V / WSL / Docker. The address of the default route (the old way) is the VPN when a
// full VPN is on, the cable when the phone is on the Wi-Fi of another segment, or "localhost" without a network.
// So every usable address is listed: the best one goes into the main QR code, the others are offered next to it.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NetAddr {
    pub ip: String,
    // "lan" (Wi-Fi / Ethernet at home), "hotspot" (this PC shares its Internet), "vpn" (works from other networks)
    pub kind: &'static str,
    pub label: String,
}

// None: an address no phone should be offered (loopback, link-local, virtual machines and containers)
pub fn classify(ip: std::net::Ipv4Addr, adapter: &str) -> Option<(&'static str, u8)> {
    let name = adapter.to_lowercase();
    let o = ip.octets();
    if ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_multicast() {
        return None;
    }
    let virtual_adapter = ["virtualbox", "vmware", "hyper-v", "vethernet", "wsl", "docker", "loopback", "vbox", "pseudo"].iter().any(|v| name.contains(v));
    let vpn_name = ["tailscale", "zerotier", "wireguard", "vpn", "tap-", "tap ", "openvpn", "hamachi", "radmin"].iter().any(|v| name.contains(v));
    // Tailscale (and other overlay networks) give 100.64.0.0/10: reachable from the user's own devices anywhere
    let cgnat = o[0] == 100 && (64..128).contains(&o[1]);
    if vpn_name || cgnat {
        return Some(("vpn", 3));
    }
    if virtual_adapter {
        return None;
    }
    if o == [192, 168, 137, 1] {
        return Some(("hotspot", 2));
    }
    if ip.is_private() {
        return Some(("lan", 1));
    }
    // a public address right on the adapter (rare): not offered, the remote is not meant to face the Internet
    None
}

fn label_of(kind: &str, adapter: &str) -> String {
    match kind {
        "lan" => format!("Домашняя сеть ({})", adapter),
        "hotspot" => "Мобильный хот-спот этого компьютера".to_string(),
        _ => format!("VPN ({}) — для телефона в другой сети или на мобильном интернете", adapter),
    }
}

// the usable addresses, best first; `default_route` (the adapter Windows uses for the Internet) wins among its kind
// `found`: (address, the adapter name shown to the user, all names of the adapter incl. its description)
pub fn rank(found: &[(std::net::Ipv4Addr, String, String)], default_route: Option<std::net::Ipv4Addr>) -> Vec<NetAddr> {
    // the phone is on Wi-Fi: among home addresses the Wi-Fi adapter comes before a cable
    let wifi = |n: &str| { let n = n.to_lowercase(); ["wi-fi", "wifi", "wireless", "wlan", "беспровод"].iter().any(|w| n.contains(w)) };
    let mut list: Vec<(u8, bool, bool, NetAddr)> = Vec::new();
    for (ip, shown, names) in found {
        if let Some((kind, prio)) = classify(*ip, names) {
            if list.iter().any(|(_, _, _, a)| a.ip == ip.to_string()) {
                continue;
            }
            list.push((prio, Some(*ip) != default_route, !wifi(names), NetAddr { ip: ip.to_string(), kind, label: label_of(kind, shown) }));
        }
    }
    list.sort_by_key(|(prio, not_default, not_wifi, _)| (*prio, *not_default, *not_wifi));
    list.into_iter().map(|(_, _, _, a)| a).collect()
}

fn default_route_ip() -> Option<std::net::Ipv4Addr> {
    // connecting a UDP socket sends nothing, it only selects the outgoing interface
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    match s.local_addr().ok()?.ip() {
        std::net::IpAddr::V4(v4) => Some(v4),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn adapters() -> Vec<(std::net::Ipv4Addr, String, String)> {
    use winapi::shared::ws2def::{AF_INET, SOCKADDR_IN};
    use winapi::um::iphlpapi::GetAdaptersAddresses;
    use winapi::um::iptypes::{GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES};
    let mut out = Vec::new();
    let mut size: u32 = 16 * 1024;
    for _ in 0..3 {
        let mut buf: Vec<u8> = vec![0; size as usize];
        let first = buf.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES;
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let rc = unsafe { GetAdaptersAddresses(AF_INET as u32, flags, std::ptr::null_mut(), first, &mut size) };
        if rc == 111 {
            // ERROR_BUFFER_OVERFLOW: `size` now holds what is needed
            continue;
        }
        if rc != 0 {
            break;
        }
        let mut cur = first;
        unsafe {
            while !cur.is_null() {
                let a = &*cur;
                // IfOperStatusUp = 1; 24 = software loopback
                if a.OperStatus == 1 && a.IfType != 24 {
                    let wide = |p: *const u16| -> String {
                        if p.is_null() { return String::new(); }
                        let mut n = 0;
                        while *p.add(n) != 0 { n += 1; }
                        String::from_utf16_lossy(std::slice::from_raw_parts(p, n))
                    };
                    let shown = wide(a.FriendlyName).trim().to_string();
                    let names = format!("{} {}", shown, wide(a.Description));
                    let mut ua = a.FirstUnicastAddress;
                    while !ua.is_null() {
                        let sa = (*ua).Address.lpSockaddr;
                        if !sa.is_null() && (*sa).sa_family as i32 == AF_INET {
                            let sin = &*(sa as *const SOCKADDR_IN);
                            let b = sin.sin_addr.S_un.S_un_b();
                            out.push((std::net::Ipv4Addr::new(b.s_b1, b.s_b2, b.s_b3, b.s_b4), shown.clone(), names.clone()));
                        }
                        ua = (*ua).Next;
                    }
                }
                cur = a.Next;
            }
        }
        break;
    }
    out
}

#[cfg(not(target_os = "windows"))]
fn adapters() -> Vec<(std::net::Ipv4Addr, String, String)> {
    default_route_ip().map(|ip| vec![(ip, "сеть".to_string(), "сеть".to_string())]).unwrap_or_default()
}

pub fn addresses() -> Vec<NetAddr> {
    let default_route = default_route_ip();
    let list = rank(&adapters(), default_route);
    if !list.is_empty() {
        return list;
    }
    // nothing looks like a home network (an unusual network of a company or a provider): the address Windows uses
    // for the Internet is offered as it is, rather than nothing
    default_route
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .map(|ip| vec![NetAddr { ip: ip.to_string(), kind: "lan", label: "Адрес компьютера в сети".to_string() }])
        .unwrap_or_default()
}

// the address of the main link: the best one, or the default route, or "localhost" (then the window says why)
fn local_ip() -> String {
    addresses().first().map(|a| a.ip.clone()).or_else(|| default_route_ip().map(|ip| ip.to_string())).unwrap_or_else(|| "localhost".to_string())
}

pub fn url_for(ip: &str, https: bool) -> String {
    format!("{}://{}:{}/?t={}", if https { "https" } else { "http" }, ip, PORT, token())
}

// main link: plain http opens in every browser without a warning (inside the home network or a VPN)
pub fn url() -> String {
    url_for(&local_ip(), false)
}

// https link (own certificate): browsers warn once, but only here the phone microphone is allowed
pub fn url_https() -> String {
    url_for(&local_ip(), true)
}

pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn start() -> Result<String, String> {
    if is_running() {
        return Ok(url());
    }
    if let Err(e) = tls_current() {
        warn!("Remote control: https is not available ({}), plain http only", e);
    }
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
                    // a limit of connections at once: a flood from the network cannot start endless threads
                    if ACTIVE.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        ACTIVE.fetch_sub(1, Ordering::SeqCst);
                        drop(stream);
                        continue;
                    }
                    std::thread::spawn(move || {
                        // the slot is given back however the connection ends (also on a panic)
                        struct Slot;
                        impl Drop for Slot {
                            fn drop(&mut self) {
                                ACTIVE.fetch_sub(1, Ordering::SeqCst);
                            }
                        }
                        let _slot = Slot;
                        let _ = serve(stream);
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
    // the access from the internet does not outlive the remote
    crate::tunnel::disable();
}

// wrong secrets from one address of the internet (a request through the tunnel names it in Cf-Connecting-Ip): ten in a
// minute close the door to that address for a minute. The secret has 128 bits, this only keeps a guesser away.
const THROTTLE_TRIES: u32 = 10;
const THROTTLE_WINDOW: Duration = Duration::from_secs(60);
static FAILS: Lazy<Mutex<std::collections::HashMap<String, (u32, std::time::Instant)>>> = Lazy::new(|| Mutex::new(std::collections::HashMap::new()));

// The sender behind an address: an IPv6 address is a whole /64 for its holder (rotating inside it costs nothing), an
// IPv4-mapped one is that IPv4 address; "" (a request on the local network, no Cf-Connecting-Ip) stays "".
pub fn throttle_key(ip: &str) -> String {
    let ip = ip.trim();
    if ip.is_empty() {
        return String::new();
    }
    match ip.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.to_string(),
        Ok(std::net::IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let o = v6.octets();
                format!("v6:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}", o[0], o[1], o[2], o[3], o[4], o[5], o[6], o[7])
            }
        },
        // not an address at all (a forged header): one bucket for all such senders
        Err(_) => "invalid".to_string(),
    }
}

// wrong secrets from ALL senders together (many addresses, one guesser): past this the wrong ones are answered 429 whoever
// sends them; a request with the RIGHT secret is never counted here, so the owner is not locked out by someone else
const SHARED_TRIES: u32 = 60;
pub const GLOBAL_KEY: &str = "*tunnel";
pub const LAN_KEY: &str = "*lan";

pub fn throttled(ip: &str, now: std::time::Instant) -> bool {
    throttled_at(ip, now, THROTTLE_TRIES)
}

pub fn throttled_at(ip: &str, now: std::time::Instant, limit: u32) -> bool {
    let mut m = FAILS.lock();
    match m.get(ip) {
        Some((n, since)) if now.duration_since(*since) < THROTTLE_WINDOW => *n >= limit,
        Some(_) => {
            m.remove(ip);
            false
        }
        None => false,
    }
}

pub fn note_failure(ip: &str, now: std::time::Instant) {
    let mut m = FAILS.lock();
    if m.len() > 2000 {
        m.retain(|_, (_, since)| now.duration_since(*since) < THROTTLE_WINDOW);
        if m.len() > 2000 {
            m.clear();
        }
    }
    let e = m.entry(ip.to_string()).or_insert((0, now));
    if now.duration_since(e.1) >= THROTTLE_WINDOW {
        *e = (0, now);
    }
    e.0 += 1;
}

pub fn note_success(ip: &str) {
    FAILS.lock().remove(ip);
}

static ACTIVE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
const MAX_CONNECTIONS: usize = 24;

// the https setup for the current addresses of the PC: made again when they change (another Wi-Fi, a new address
// from the router, a VPN switched on), so the certificate always names the address the phone opens
static TLS: Lazy<Mutex<Option<(String, std::sync::Arc<rustls::ServerConfig>)>>> = Lazy::new(|| Mutex::new(None));

fn tls_current() -> Result<std::sync::Arc<rustls::ServerConfig>, String> {
    let names = cert_names();
    let key = names.join(",");
    if let Some((k, c)) = TLS.lock().as_ref() {
        if *k == key {
            return Ok(c.clone());
        }
    }
    let c = tls_config(&names)?;
    *TLS.lock() = Some((key, c.clone()));
    Ok(c)
}

fn cert_names() -> Vec<String> {
    let mut names: Vec<String> = addresses().into_iter().map(|a| a.ip).collect();
    if names.is_empty() {
        names.push(local_ip());
    }
    names.push("localhost".to_string());
    names.push("127.0.0.1".to_string());
    names.dedup();
    names
}

struct Request {
    method: String,
    path: String,
    token_header: Option<String>,
    // the unique number of the request (nonce) and the time of the phone: see replay.rs
    rid: String,
    ts: String,
    // the address of the phone from Cloudflare (empty for a connection from the home network)
    client_ip: String,
    body: String,
}

// the largest phrase of speech from the page: ten seconds of 16-bit 16 kHz audio in base64
pub const MAX_AUDIO_BODY: usize = 460_000;

fn read_request<S: Read>(stream: &mut S) -> std::io::Result<Request> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];
    // the whole request must arrive within a few seconds (each read has its own timeout, but a client sending one
    // byte now and then could otherwise hold a connection — and with it a slot — for hours)
    let mut deadline = std::time::Instant::now() + Duration::from_secs(10);
    let header_end = loop {
        if std::time::Instant::now() > deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
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
    let mut ts = String::new();
    let mut client_ip = String::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_lowercase();
            let v = v.trim();
            if k == "content-length" {
                content_length = v.parse().unwrap_or(0);
            } else if k == "x-token" {
                token_header = Some(v.to_string());
            } else if k == "x-rid" {
                rid = v.chars().take(80).collect();
            } else if k == "x-ts" {
                ts = v.chars().take(20).collect();
            } else if k == "cf-connecting-ip" {
                // the address of the phone as Cloudflare saw it (only a request through the tunnel has it)
                client_ip = v.chars().take(64).collect();
            }
        }
    }
    // a phrase of speech from the microphone of the page (base64 of 16 kHz audio) is bigger than a command and takes longer to arrive;
    // nothing else may be
    let audio = method == "POST" && path.split('?').next() == Some("/voice-audio");
    if audio {
        deadline = std::time::Instant::now() + Duration::from_secs(25);
    }
    if content_length > if audio { MAX_AUDIO_BODY } else { 2000 } {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    while buf.len() < header_end + content_length {
        if std::time::Instant::now() > deadline {
            return Err(std::io::ErrorKind::TimedOut.into());
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            // the connection ended in the middle of the body: a part of a phrase is never run as a command
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let body_bytes = &buf[header_end..buf.len().min(header_end + content_length)];
    Ok(Request {
        method,
        path,
        token_header,
        rid,
        ts,
        client_ip,
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
const CONNECTION_LIMIT: Duration = Duration::from_secs(15);

// closes the socket when the connection lasts longer than the limit; ends with the connection
struct Watchdog {
    done: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Watchdog {
    fn new(stream: &TcpStream, limit: Duration) -> Watchdog {
        let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Ok(s) = stream.try_clone() {
            let d = done.clone();
            std::thread::spawn(move || {
                let start = std::time::Instant::now();
                while !d.load(Ordering::SeqCst) {
                    if start.elapsed() >= limit {
                        let _ = s.shutdown(std::net::Shutdown::Both);
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(200));
                }
            });
        }
        Watchdog { done }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        self.done.store(true, Ordering::SeqCst);
    }
}

fn serve(stream: TcpStream) -> std::io::Result<()> {
    if !is_running() {
        return Ok(()); // switched off: the connection is dropped without an answer
    }
    // on Windows an accepted connection inherits "non-blocking" from the listener: the first read would fail at once
    // ("would block") before the phone has sent its request, and the page would stay empty
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    // a hard limit for the whole connection: a client sending one byte of the TLS handshake now and then (the
    // handshake happens inside the first read, where the 10 s request deadline is not looked at) is cut off: after
    // the limit the socket is shut down, so the next read fails at once (on Windows a read already waiting ends by
    // its 5 s timeout first) — at most CONNECTION_LIMIT + 5 s per connection
    let _watchdog = Watchdog::new(&stream, CONNECTION_LIMIT);
    let mut first = [0u8; 1];
    let n = stream.peek(&mut first)?;
    if n == 1 && first[0] == 0x16 {
        // (made only for an https connection: the plain polls of the phone do not look at the adapters)
        let config = tls_current().map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
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
fn tls_config(names: &[String]) -> Result<std::sync::Arc<rustls::ServerConfig>, String> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

    let key = names.join(",");
    let dir = APP_CONFIG_DIR.get().cloned();
    let (cert_path, key_path, ip_path) = match &dir {
        Some(d) => (Some(d.join("remote_cert.der")), Some(d.join("remote_key.der")), Some(d.join("remote_cert.ip"))),
        None => (None, None, None),
    };

    let mut cert_der: Option<Vec<u8>> = None;
    let mut key_der: Option<Vec<u8>> = None;
    if let (Some(c), Some(k), Some(i)) = (&cert_path, &key_path, &ip_path) {
        if std::fs::read_to_string(i).map(|v| v.trim() == key).unwrap_or(false) {
            cert_der = std::fs::read(c).ok();
            key_der = std::fs::read(k).ok();
        }
    }
    if cert_der.is_none() || key_der.is_none() {
        let generated = rcgen::generate_simple_self_signed(names.to_vec()).map_err(|e| e.to_string())?;
        let c = generated.cert.der().to_vec();
        let k = generated.key_pair.serialize_der();
        if let (Some(cp), Some(kp), Some(ip_p)) = (&cert_path, &key_path, &ip_path) {
            let _ = std::fs::write(cp, &c);
            let _ = std::fs::write(kp, &k);
            let _ = std::fs::write(ip_p, &key);
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

// the phone can send a phrase when the queue of the program exists (it is created at the start of the program)
fn queue_ready_for_phone() -> bool {
    #[cfg(test)]
    if capturing(None) {
        return true;
    }
    TX.get().is_some()
}

fn token_in_path(path: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix("t=").map(|v| v.to_string()))
}

// the buttons of «Мой голос» on the page: start, cancel, test, delete, enable:1 / enable:0, strict:0..2
fn voiceprint_do(op: &str) -> Result<(), String> {
    match op {
        "start" => crate::voiceprint::start_enrollment(),
        "cancel" => crate::voiceprint::cancel_enrollment(),
        "test" => crate::voiceprint::start_test(),
        "delete" => crate::voiceprint::delete(),
        "enable:1" => return crate::voiceprint::set_enabled(true),
        "enable:0" => return crate::voiceprint::set_enabled(false),
        "strict:0" | "strict:1" | "strict:2" => crate::voiceprint::set_strictness(op.as_bytes()[7] - b'0'),
        _ => return Err("неизвестное действие".into()),
    }
    Ok(())
}

// A phrase of speech recorded by the microphone of the page (16 kHz mono). While the voice is being recorded or tested the phrase is
// only measured. Otherwise: the voice is compared with the print of the owner (when that lock is on), the phrase is recognized here
// (offline, Vosk) and runs as a command of the phone. What came back: for the page to show.
pub fn voice_audio(samples: &[i16]) -> serde_json::Value {
    use serde_json::json;
    if crate::voiceprint::capturing() {
        let message = crate::voiceprint::take_capture(samples, 16_000);
        return json!({ "ok": true, "kind": "capture", "message": message, "status": crate::voiceprint::status_json() });
    }
    if crate::voiceprint::is_active() {
        if let crate::voiceprint::Verdict::Refused { distance, threshold } = crate::voiceprint::check(samples, 16_000) {
            info!("Remote control voice: not the owner's voice (distance {:.2}, threshold {:.2})", distance, threshold);
            crate::uistate::log_event("info", &format!("Голос с пульта не распознан как ваш (расстояние {:.2}, порог {:.2}): фраза проигнорирована", distance, threshold));
            return json!({ "ok": false, "kind": "refused", "message": format!("Голос не распознан как ваш (расстояние {:.2}, порог {:.2}). Если это были вы: «Мягче» или «Записать заново».", distance, threshold) });
        }
    }
    let text = match crate::tgvoice::recognize(samples) {
        Ok(t) => t.trim().to_string(),
        Err(e) => return json!({ "ok": false, "kind": "error", "message": e }),
    };
    if text.is_empty() {
        return json!({ "ok": false, "kind": "empty", "message": "Не расслышал." });
    }
    // the words of Jarvis himself that the microphone of the page caught (the answer is played by the computer or by the page)
    if crate::tts::is_own_echo(&text) {
        info!("Remote control voice: the own voice of Jarvis was ignored");
        return json!({ "ok": false, "kind": "echo", "message": "Это был голос самого Джарвиса." });
    }
    if text.chars().count() > 300 {
        return json!({ "ok": false, "kind": "error", "message": "Фраза слишком длинная." });
    }
    if !queue_ready_for_phone() {
        return json!({ "ok": false, "kind": "error", "message": "Компьютер пока не принимает команды (Jarvis запускается или пульт выключен)." });
    }
    // what was said is private: only its size goes to the log
    info!("Remote control spoken command: {} chars", text.chars().count());
    note_phone_command();
    submit(Source::Phone, &text);
    json!({ "ok": true, "kind": "command", "text": text })
}

fn handle<S: Read + Write>(stream: &mut S) -> std::io::Result<()> {
    let req = read_request(stream)?;

    if !is_running() {
        respond(stream, "503 Service Unavailable", "text/plain", "off");
        return Ok(());
    }

    // the page itself holds no secret and no data (the phone keeps the secret after the first opening of the link):
    // it opens without it, so a reload or a home-screen shortcut works; everything it asks for needs the secret
    let route0 = req.path.split('?').next().unwrap_or("/");
    if req.method == "GET" && route0 == "/" && token_in_path(&req.path).is_none() {
        respond(stream, "200 OK", "text/html", PAGE);
        return Ok(());
    }
    // «Add to the home screen» on Android: the manifest and the icon hold no secret and no data, like the page itself
    if req.method == "GET" && route0 == "/manifest.webmanifest" {
        respond(stream, "200 OK", "application/manifest+json", MANIFEST);
        return Ok(());
    }
    if req.method == "GET" && route0 == "/icon.png" {
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nCache-Control: public, max-age=86400\r\nConnection: close\r\n\r\n", APP_ICON.len());
        let _ = stream.write_all(APP_ICON);
        return Ok(());
    }
    let supplied = req.token_header.clone().or_else(|| token_in_path(&req.path));
    let key = throttle_key(&req.client_ip);
    let now = std::time::Instant::now();
    // pairing with the one-time code (pairing.rs): the only request that needs no secret, and it is throttled like a wrong secret
    if req.method == "POST" && route0 == "/pair" {
        if !key.is_empty() && throttled(&key, now) {
            respond(stream, "429 Too Many Requests", "text/plain", "too many wrong tries, wait a minute");
            return Ok(());
        }
        let field = |name: &str| -> String {
            req.body.split('&').find_map(|kv| kv.strip_prefix(&format!("{}=", name))).map(|v| url_decode(v)).unwrap_or_default()
        };
        match pairing::redeem(&field("code"), &field("name"), now_secs()) {
            Ok(secret) => {
                if !key.is_empty() {
                    note_success(&key);
                }
                info!("Remote control: a device was paired with a code");
                respond(stream, "200 OK", "application/json", &serde_json::json!({ "ok": true, "token": secret }).to_string());
            }
            Err(e) => {
                if !key.is_empty() {
                    note_failure(&key, now);
                }
                note_failure(if key.is_empty() { LAN_KEY } else { GLOBAL_KEY }, now);
                respond(stream, "403 Forbidden", "application/json", &serde_json::json!({ "ok": false, "message": e }).to_string());
            }
        }
        return Ok(());
    }
    if !key.is_empty() && throttled(&key, now) {
        respond(stream, "429 Too Many Requests", "text/plain", "too many wrong tries, wait a minute");
        return Ok(());
    }
    // the shared link, or the own secret of a paired device
    let paired = supplied.as_deref().and_then(|t| pairing::check(t, now_secs()));
    if paired.is_none() && !token_matches(supplied.as_deref(), &token()) {
        let shared = if key.is_empty() { LAN_KEY } else { GLOBAL_KEY };
        if throttled_at(shared, now, SHARED_TRIES) {
            respond(stream, "429 Too Many Requests", "text/plain", "too many wrong tries, wait a minute");
            return Ok(());
        }
        if !key.is_empty() {
            note_failure(&key, now);
        }
        note_failure(shared, now);
        // a flood of wrong requests does not flood the log: one line a minute
        if crate::guard::first("remote:wrong-token", Duration::from_secs(60)) {
            warn!("Remote control: request with a wrong token rejected");
        }
        respond(stream, "403 Forbidden", "text/plain", "forbidden");
        return Ok(());
    }

    if !key.is_empty() {
        note_success(&key);
    }
    // a request that CHANGES something carries the time and a unique number: a recorded request played again is not run again (replay.rs)
    if req.method == "POST" && REPLAY_PROTECTED.contains(&req.path.split('?').next().unwrap_or("")) {
        let who = paired.clone().unwrap_or_else(|| "shared".to_string());
        let route_name = req.path.split('?').next().unwrap_or("").trim_start_matches('/').to_string();
        match crate::replay::accept(&who, &req.ts, &req.rid, now_secs()) {
            crate::replay::Verdict::Accept => {}
            crate::replay::Verdict::Duplicate => {
                crate::audit::record_door("phone", "remote", &route_name, "", "refused", false, "повтор запроса: второй раз не выполняется");
                respond(stream, "200 OK", "application/json", &serde_json::json!({ "ok": true, "duplicate": true, "message": "Этот запрос уже выполнен." }).to_string());
                return Ok(());
            }
            crate::replay::Verdict::Stale => {
                crate::audit::record_rate_limited("replay-stale", 60, "remote", &route_name, "refused", "запрос устарел или время телефона не совпадает");
                respond(stream, "409 Conflict", "application/json", &serde_json::json!({ "ok": false, "stale": true, "now": now_secs(), "message": "Время телефона не совпадает со временем компьютера." }).to_string());
                return Ok(());
            }
            crate::replay::Verdict::Malformed => {
                crate::audit::record_rate_limited("replay-malformed", 60, "remote", &route_name, "refused", "в запросе нет времени или номера");
                respond(stream, "400 Bad Request", "application/json", &serde_json::json!({ "ok": false, "message": "Нужны заголовки X-Ts и X-Rid. Обновите страницу пульта." }).to_string());
                return Ok(());
            }
            crate::replay::Verdict::TooMany => {
                crate::audit::record_rate_limited("remote-rate", 60, "remote", &route_name, "refused", "слишком много запросов с устройства");
                respond(stream, "429 Too Many Requests", "text/plain", "too many requests");
                return Ok(());
            }
        }
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
            } else if queue_ready_for_phone() {
                // what the user typed on the phone is private: only its size goes to the log
                info!("Remote control command: {} chars", text.chars().count());
                note_phone_command();
                submit(Source::Phone, &text);
                respond(stream, "200 OK", "text/plain", "ok");
            } else {
                respond(stream, "503 Service Unavailable", "text/plain", "not ready");
            }
        }
        // «Мой голос» (voiceprint.rs) from the page: the state, the buttons, and the phrases of speech that the page records
        ("GET", "/voiceprint") => respond(stream, "200 OK", "application/json", &crate::voiceprint::status_json().to_string()),
        ("POST", "/voiceprint-do") => match voiceprint_do(req.body.trim()) {
            Ok(()) => respond(stream, "200 OK", "application/json", &crate::voiceprint::status_json().to_string()),
            Err(e) => respond(stream, "400 Bad Request", "application/json", &serde_json::json!({ "ok": false, "message": e }).to_string()),
        },
        ("POST", "/voice-audio") => {
            let bytes = crate::docai::b64_decode(req.body.trim()).unwrap_or_default();
            // from 0.4 s to 10 s of 16 kHz 16-bit audio
            if bytes.len() % 2 != 0 || !(12_800..=320_000).contains(&bytes.len()) {
                respond(stream, "400 Bad Request", "application/json", &serde_json::json!({ "ok": false, "kind": "error", "message": "Звук не подходит: нужна фраза от 0,4 до 10 секунд." }).to_string());
            } else {
                let samples: Vec<i16> = bytes.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]])).collect();
                info!("Remote control voice: {} samples", samples.len());
                respond(stream, "200 OK", "application/json", &voice_audio(&samples).to_string());
            }
        }
        // what plays on the computer now (the window titles of the players and of YouTube in a browser)
        // a picture of the screen of this computer for the phone: only with the permission «Показывать экран…» (off by default) and only for
        // an authenticated device; one picture per request, nothing is recorded or kept
        ("GET", "/screen") => {
            if let Err(e) = crate::perms::check(crate::perms::Cap::RemoteScreen) {
                respond(stream, "403 Forbidden", "text/plain", &e);
            } else if !crate::guard::first("remote:screen", Duration::from_millis(700)) {
                respond(stream, "429 Too Many Requests", "text/plain", "не чаще раза в секунду");
            } else {
                match crate::screen::capture() {
                    Ok(jpeg) => {
                        info!("Remote control: the screen was shown to a device");
                        crate::uistate::log_event("info", "Экран показан на привязанном устройстве");
                        respond(stream, "200 OK", "application/json", &serde_json::json!({ "ok": true, "jpeg": crate::docai::b64_encode(&jpeg) }).to_string());
                    }
                    Err(e) => respond(stream, "503 Service Unavailable", "text/plain", &e),
                }
            }
        }
        // the two sections «ПК» / «Телефон» (devhist.rs): the same history the window shows
        ("GET", "/history") => {
            let door = match query_param(&req.path, "door") { Some("pc") => "pc", Some("phone") => "phone", _ => "all" };
            respond(stream, "200 OK", "application/json", &crate::devhist::json(door, 40).to_string());
        }
        // the sections of the phone: reading (a short list of names) and acting (the actions the phone may ask for, see ui::phone_act)
        ("GET", p) if p.starts_with("/api/") => {
            let query = req.path.split_once('?').map(|x| x.1).unwrap_or("");
            match crate::ui::phone_get(&p["/api/".len()..], query) {
                Some(v) => respond(stream, "200 OK", "application/json", &v.to_string()),
                None => respond(stream, "404 Not Found", "text/plain", "not found"),
            }
        }
        ("POST", "/act") => {
            let body: serde_json::Value = serde_json::from_str(&req.body).unwrap_or(serde_json::json!({}));
            respond(stream, "200 OK", "application/json", &crate::ui::phone_act(&body).to_string());
        }
        // the look of the window (theme, sphere, speed, glow): the phone starts with the same one
        ("GET", "/look") => {
            let ui = crate::uistate::settings().ui;
            let g = |k: &str| ui.get(k).cloned().unwrap_or_default();
            respond(stream, "200 OK", "application/json", &serde_json::json!({ "theme": g("jarvis-theme"), "sphere": g("jarvis-sphere"), "speed": g("jarvis-speed"), "glow": g("jarvis-glow") }).to_string());
        }
        ("GET", "/media") => {
            let items: Vec<serde_json::Value> = crate::media::now().into_iter().map(|n| serde_json::json!({ "app": n.app, "title": n.title })).collect();
            respond(stream, "200 OK", "application/json", &serde_json::json!({ "items": items }).to_string());
        }
        // the voice of the answers (the same choice as in the settings of the computer): the list and the change
        ("GET", "/voices") => {
            let list: Vec<serde_json::Value> = crate::tts::list_voices().into_iter().map(|(id, label)| serde_json::json!({ "id": id, "label": label })).collect();
            respond(stream, "200 OK", "application/json", &serde_json::json!({ "current": crate::uistate::voice_name(), "voices": list }).to_string());
        }
        ("POST", "/voice") => {
            // only a voice of the list (never a free text); the answer is the same for a wrong name and a name of another PC
            let id = req.body.trim().to_string();
            if crate::tts::list_voices().iter().any(|(v, _)| *v == id) {
                crate::uistate::update_settings(&serde_json::json!({ "voice_name": id }));
                respond(stream, "200 OK", "text/plain", "ok");
            } else {
                respond(stream, "400 Bad Request", "text/plain", "unknown voice");
            }
        }
        _ => respond(stream, "404 Not Found", "text/plain", "not found"),
    }
    Ok(())
}

const APP_ICON: &[u8] = include_bytes!("../../../resources/icons/icon.png");
const MANIFEST: &str = r##"{"name":"Jarvis","short_name":"Jarvis","description":"Пульт Jarvis для компьютера","start_url":"/","scope":"/","display":"standalone","orientation":"portrait","background_color":"#08090b","theme_color":"#08090b","icons":[{"src":"/icon.png","sizes":"512x512","type":"image/png","purpose":"any"}]}"##;

// the requests of the phone that change something; each carries X-Ts and X-Rid (replay.rs)
const REPLAY_PROTECTED: [&str; 5] = ["/cmd", "/act", "/voice", "/voiceprint-do", "/voice-audio"];

// the page of the phone: src/remote_page.html (the build puts the shared sphere code of src/orbkit.js into it)
const PAGE: &str = include_str!(concat!(env!("OUT_DIR"), "/remote_page.html"));

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> std::net::Ipv4Addr {
        s.parse().unwrap()
    }

    #[test]
    fn the_place_and_the_network_of_the_phone_are_taken_only_when_they_are_real() {
        let _g = PHONE_TEST_LOCK.lock();
        assert!(phone_state(r#"{"net":"wifi","lat":41.3111,"lon":69.2797,"acc":25.5}"#));
        let j = phone_json();
        assert_eq!(j["net"], "wifi");
        assert_eq!((j["location"]["lat"].as_f64(), j["location"]["accuracy"].as_f64()), (Some(41.3111), Some(25.5)));
        // out of range numbers or a missing half: no place
        assert!(phone_state(r#"{"lat":91.0,"lon":10.0}"#));
        assert!(phone_json()["location"].is_null());
        assert!(phone_state(r#"{"lat":10.0}"#));
        assert!(phone_json()["location"].is_null());
        // a page that stopped sharing sends none: the old place is gone
        assert!(phone_state(r#"{"net":"<script>"}"#));
        assert!(phone_json()["location"].is_null());
        assert_eq!(phone_json()["net"], "script", "only letters, digits and a dash stay");
    }

    #[test]
    fn the_addresses_a_phone_can_reach_are_listed_best_first() {
        let found = vec![
            (ip("10.0.0.5"), "Ethernet 2".to_string(), "Ethernet 2 Realtek".to_string()),
            (ip("172.24.16.1"), "vEthernet (WSL)".to_string(), "vEthernet (WSL) Hyper-V Virtual Ethernet Adapter".to_string()),
            (ip("192.168.56.1"), "Ethernet 3".to_string(), "Ethernet 3 VirtualBox Host-Only Ethernet Adapter".to_string()),
            (ip("100.101.102.103"), "Tailscale".to_string(), "Tailscale Tailscale Tunnel".to_string()),
            (ip("192.168.1.42"), "Беспроводная сеть".to_string(), "Беспроводная сеть Intel Wi-Fi 6".to_string()),
            (ip("192.168.137.1"), "Подключение по локальной сети* 10".to_string(), "Microsoft Wi-Fi Direct Virtual Adapter".to_string()),
            (ip("169.254.10.10"), "Ethernet".to_string(), "Ethernet".to_string()),
            (ip("127.0.0.1"), "Loopback".to_string(), "Loopback Pseudo-Interface 1".to_string()),
        ];
        let list = rank(&found, Some(ip("192.168.1.42")));
        let ips: Vec<&str> = list.iter().map(|a| a.ip.as_str()).collect();
        // home Wi-Fi of the default route first, then the other LAN, the hotspot, the VPN; no virtual machines,
        // no link-local, no loopback
        assert_eq!(ips, ["192.168.1.42", "10.0.0.5", "192.168.137.1", "100.101.102.103"]);
        assert_eq!(list[0].kind, "lan");
        assert!(list[0].label.contains("Беспроводная сеть"));
        assert_eq!(list[2].kind, "hotspot");
        assert_eq!(list[3].kind, "vpn");
        assert!(list[3].label.contains("другой сети"), "{}", list[3].label);
        // a full VPN holding the default route does not push the home Wi-Fi out of the main QR code
        let list = rank(&found, Some(ip("100.101.102.103")));
        assert_eq!(list[0].ip, "192.168.1.42");
        assert!(rank(&[], None).is_empty());
    }

    #[test]
    fn the_secret_is_compared_exactly_and_a_new_link_revokes_the_old_one() {
        assert!(token_matches(Some("abcdef0123456789"), "abcdef0123456789"));
        assert!(!token_matches(Some("abcdef012345678"), "abcdef0123456789"));
        assert!(!token_matches(Some("abcdef0123456780"), "abcdef0123456789"));
        assert!(!token_matches(None, "abcdef0123456789"));
        assert!(!token_matches(Some(""), ""));
        let _g = PHONE_TEST_LOCK.lock();
        let old = token();
        let new = renew_token().unwrap();
        assert_ne!(old, new);
        assert_eq!(new.len(), 32);
        assert!(!token_matches(Some(&old), &token()), "the old link no longer works");
        assert!(url_for("192.168.1.42", false).ends_with(&format!("?t={}", new)));
    }

    #[test]
    fn ten_wrong_secrets_from_one_internet_address_close_the_door_to_it_for_a_minute() {
        let now = std::time::Instant::now();
        let ip = "203.0.113.77";
        note_success(ip);
        for _ in 0..THROTTLE_TRIES - 1 {
            note_failure(ip, now);
        }
        assert!(!throttled(ip, now), "nine tries are still allowed");
        note_failure(ip, now);
        assert!(throttled(ip, now), "the tenth closes the door");
        assert!(!throttled("203.0.113.78", now), "another address is not affected");
        assert!(throttled(ip, now + Duration::from_secs(30)));
        assert!(!throttled(ip, now + Duration::from_secs(61)), "a minute later it is open again");
        note_failure(ip, now);
        note_success(ip);
        assert!(!throttled(ip, now), "the owner who got the secret right starts from zero");
    }

    // the throttle is by sender, not by spelling: the contract is «ten wrong secrets in a minute close the door (429)»
    #[test]
    fn the_throttle_follows_the_sender_not_the_spelling_of_the_address() {
        // one IPv6 holder owns the whole /64: rotating inside it does not reset the count
        let a = throttle_key("2001:db8:1:2:aaaa:bbbb:cccc:dddd");
        assert_eq!(a, throttle_key("2001:db8:1:2:1111:2222:3333:4444"));
        assert_ne!(a, throttle_key("2001:db8:1:3::1"), "another /64 is another sender");
        assert_eq!(throttle_key("::ffff:203.0.113.5"), throttle_key("203.0.113.5"), "an IPv4-mapped address is the IPv4 one");
        assert_eq!(throttle_key(""), "");
        assert_eq!(throttle_key("not an ip"), throttle_key("<script>"), "forged values share one bucket");
        let now = std::time::Instant::now();
        let k = throttle_key("2001:db8:9:9::7");
        note_success(&k);
        for i in 0..THROTTLE_TRIES {
            assert!(!throttled(&throttle_key(&format!("2001:db8:9:9::{:x}", i + 1)), now), "try {} is still allowed", i);
            note_failure(&throttle_key(&format!("2001:db8:9:9::{:x}", i + 1)), now);
        }
        assert!(throttled(&throttle_key("2001:db8:9:9:ffff::1"), now), "ten tries from ten spellings of one /64 closed the door");
        note_success(&k);
    }

    // the shared buckets count wrong secrets only, never close the door for the right one, and are bounded by the same minute
    #[test]
    fn many_senders_together_cannot_guess_either_and_the_owner_is_not_locked_out() {
        let now = std::time::Instant::now();
        note_success(GLOBAL_KEY);
        for i in 0..SHARED_TRIES {
            assert!(!throttled_at(GLOBAL_KEY, now, SHARED_TRIES), "wrong try {}", i);
            note_failure(&throttle_key(&format!("198.51.100.{}", i % 250)), now);
            note_failure(GLOBAL_KEY, now);
        }
        assert!(throttled_at(GLOBAL_KEY, now, SHARED_TRIES), "the shared bucket closes after {} wrong secrets of many senders", SHARED_TRIES);
        assert!(!throttled(&throttle_key("192.0.2.1"), now), "an address that never failed has its own door");
        assert!(!throttled_at(GLOBAL_KEY, now + Duration::from_secs(61), SHARED_TRIES), "a minute later it is open again");
        note_success(GLOBAL_KEY);
    }

    struct Mock {
        inp: std::io::Cursor<Vec<u8>>,
        out: Vec<u8>,
    }
    impl Read for Mock {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.inp.read(b)
        }
    }
    impl Write for Mock {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.out.extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // the status line the server answers to one request (through the real handler)
    fn status_of(ip: &str, token: &str) -> String {
        let raw = format!("GET /poll?after=0 HTTP/1.1\r\nHost: x.trycloudflare.com\r\nCf-Connecting-Ip: {}\r\nX-Token: {}\r\n\r\n", ip, token);
        let mut m = Mock { inp: std::io::Cursor::new(raw.into_bytes()), out: Vec::new() };
        handle(&mut m).unwrap();
        String::from_utf8_lossy(&m.out).lines().next().unwrap_or("").to_string()
    }

    // the whole sequence through the handler: the boundary (the tenth wrong secret is still answered 403, the eleventh 429),
    // the door of an address that went on guessing, the right secret of another sender, a restart of the count after a minute
    #[test]
    fn the_handler_answers_403_ten_times_then_429_and_never_for_another_sender() {
        let _g = PHONE_TEST_LOCK.lock();
        RUNNING.store(true, Ordering::SeqCst);
        let guesser = "2001:db8:aa:bb:1::1";
        note_success(&throttle_key(guesser));
        for i in 0..THROTTLE_TRIES {
            // (a different spelling inside the same /64 every time)
            let st = status_of(&format!("2001:db8:aa:bb:{:x}::{:x}", i + 2, i + 7), "0000000000000000");
            assert!(st.contains("403"), "wrong secret {} got {}", i + 1, st);
        }
        assert!(status_of("2001:db8:aa:bb:ffff::1", "0000000000000000").contains("429"), "the eleventh wrong secret must be 429");
        assert!(status_of("2001:db8:aa:bb::5", &token()).contains("429"), "the door of a guessing sender stays closed even for a right secret");
        let other = status_of("198.51.100.250", &token());
        assert!(other.contains("200"), "another sender with the right secret is answered: {}", other);
        assert!(status_of("198.51.100.249", "0000000000000000").contains("403"), "another sender's wrong secret is a plain 403");
        note_success(&throttle_key(guesser));
        RUNNING.store(false, Ordering::SeqCst);
    }

    fn post_pair(ip: &str, body: &str) -> (String, String) {
        let raw = format!("POST /pair HTTP/1.1\r\nHost: x.trycloudflare.com\r\nCf-Connecting-Ip: {}\r\nContent-Length: {}\r\n\r\n{}", ip, body.len(), body);
        let mut m = Mock { inp: std::io::Cursor::new(raw.into_bytes()), out: Vec::new() };
        handle(&mut m).unwrap();
        let text = String::from_utf8_lossy(&m.out).to_string();
        (text.lines().next().unwrap_or("").to_string(), text.split("\r\n\r\n").nth(1).unwrap_or("").to_string())
    }

    #[test]
    fn a_phone_pairs_with_the_code_and_then_works_with_its_own_secret_until_revoked() {
        let _g = PHONE_TEST_LOCK.lock();
        crate::store::with_temp(|| {
            RUNNING.store(true, Ordering::SeqCst);
            pairing::reset_for_tests();
            let ip = "203.0.113.77";
            note_success(&throttle_key(ip));
            // no code asked: refused
            assert!(post_pair(ip, "code=123456&name=Pixel").0.contains("403"));
            let code = pairing::start_code(now_secs());
            let (st, body) = post_pair(ip, &format!("code={}&name=Pixel+8%20Pro", code));
            assert!(st.contains("200"), "{} {}", st, body);
            let v: serde_json::Value = serde_json::from_str(&body).unwrap();
            let secret = v["token"].as_str().unwrap().to_string();
            assert_eq!(pairing::list()[0].name, "Pixel 8 Pro");
            // the own secret opens the data, a made-up one does not, the shared link keeps working
            assert!(status_of("203.0.113.78", &secret).contains("200"));
            assert!(status_of("203.0.113.79", &"ab".repeat(32)).contains("403"));
            assert!(status_of("203.0.113.80", &token()).contains("200"));
            // the code worked once
            assert!(post_pair(ip, &format!("code={}&name=x", code)).0.contains("403"));
            // revoked: at once
            assert!(pairing::revoke(&pairing::list()[0].id));
            assert!(status_of("203.0.113.81", &secret).contains("403"));
            note_success(&throttle_key(ip));
            RUNNING.store(false, Ordering::SeqCst);
        });
    }

    #[test]
    fn the_screen_is_shown_only_with_the_permission_and_a_secret() {
        let _g = PHONE_TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            RUNNING.store(true, Ordering::SeqCst);
            let get = |ip: &str, tok: &str| {
                let raw = format!("GET /screen HTTP/1.1\r\nHost: x.trycloudflare.com\r\nCf-Connecting-Ip: {}\r\nX-Token: {}\r\n\r\n", ip, tok);
                let mut m = Mock { inp: std::io::Cursor::new(raw.into_bytes()), out: Vec::new() };
                handle(&mut m).unwrap();
                String::from_utf8_lossy(&m.out).to_string()
            };
            assert!(get("203.0.113.90", &"0".repeat(32)).contains("403"), "no secret");
            let off = get("203.0.113.91", &token());
            assert!(off.contains("403") && off.contains("центре разрешений"), "{}", off);
            crate::perms::set(crate::perms::Cap::RemoteScreen, true).unwrap();
            let on = get("203.0.113.92", &token());
            // (no screen in this environment: an honest error, never a made-up picture)
            assert!(on.contains("503") || on.contains("\"jpeg\""), "{}", on);
            RUNNING.store(false, Ordering::SeqCst);
        });
    }

    #[test]
    fn form_fields_are_decoded() {
        assert_eq!(url_decode("Pixel+8%20Pro%D0%B0"), "Pixel 8 Proа");
        assert_eq!(url_decode("100%"), "100%");
        assert_eq!(url_decode("%zz"), "%zz");
    }

    #[test]
    fn the_address_of_the_phone_is_read_from_the_header_of_cloudflare() {
        let raw = "GET /poll?after=0 HTTP/1.1\r\nHost: x.trycloudflare.com\r\nCf-Connecting-Ip: 198.51.100.9\r\n\r\n";
        assert_eq!(read_request(&mut std::io::Cursor::new(raw.as_bytes().to_vec())).unwrap().client_ip, "198.51.100.9");
        let lan = "GET / HTTP/1.1\r\nHost: 192.168.1.5:9713\r\n\r\n";
        assert_eq!(read_request(&mut std::io::Cursor::new(lan.as_bytes().to_vec())).unwrap().client_ip, "");
    }

    #[test]
    fn a_body_cut_off_in_the_middle_is_not_a_command() {
        let req = "POST /cmd HTTP/1.1\r\nHost: x\r\nContent-Length: 20\r\n\r\nвыкл".as_bytes().to_vec();
        let mut cur = std::io::Cursor::new(req);
        assert!(read_request(&mut cur).is_err(), "the phrase arrived only in part");
        let full = "POST /cmd HTTP/1.1\r\nHost: x\r\nContent-Length: 8\r\n\r\nвыкл".as_bytes().to_vec();
        assert_eq!(read_request(&mut std::io::Cursor::new(full)).unwrap().body, "выкл");
    }

    #[test]
    fn a_slow_client_is_cut_off_by_the_watchdog() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let _client = std::net::TcpStream::connect(addr).unwrap();
        let (server, _) = l.accept().unwrap();
        server.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
        let _w = Watchdog::new(&server, Duration::from_millis(300));
        std::thread::sleep(Duration::from_millis(900));
        // the watchdog has closed the socket: the next read of the handshake or the request ends at once instead of
        // waiting for its timeout (on Windows a read that was ALREADY waiting ends by its own 5 s timeout, then the
        // next one fails at once — the connection is still bounded by the limit plus one read timeout)
        let t0 = std::time::Instant::now();
        let mut b = [0u8; 1];
        let r = (&server).read(&mut b);
        assert!(matches!(r, Ok(0)) || r.is_err(), "{:?}", r);
        assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    }

    // the page opens without the secret (a reload, a home-screen shortcut); news and commands never do
    #[test]
    fn the_page_needs_no_secret_but_every_request_for_data_does() {
        let _g = PHONE_TEST_LOCK.lock();
        start().expect("start");
        std::thread::sleep(Duration::from_millis(500));
        let get = |path: &str, extra: &[&str]| {
            let mut args = vec!["-s", "--max-time", "5"];
            args.extend_from_slice(extra);
            let url = format!("http://127.0.0.1:{}{}", PORT, path);
            args.push(&url);
            let out = std::process::Command::new("curl").args(&args).output().expect("curl");
            String::from_utf8_lossy(&out.stdout).to_string()
        };
        assert!(get("/", &[]).contains("Сказать команду"), "the page itself");
        assert_eq!(get("/poll?after=0", &[]), "forbidden");
        assert_eq!(get("/cmd", &["-X", "POST", "-d", "выключи компьютер"]), "forbidden");
        assert_eq!(get("/?t=0123456789abcdef0123456789abcdef", &[]), "forbidden", "a wrong secret in the link");
        let t = token();
        let header = format!("X-Token: {}", t);
        assert!(get("/poll?after=0", &["-H", &header]).contains("\"msgs\""));
    }

    // the same port answers https (self-signed) and plain http
    #[test]
    fn https_and_http_on_one_port() {
        let _g = PHONE_TEST_LOCK.lock();
        // every failure names the stage, the port and what the program thought it was doing (the secret is never printed)
        let started = start();
        assert!(started.is_ok(), "stage=start port={} error={:?} running={}", PORT, started.as_ref().err(), is_running());
        std::thread::sleep(Duration::from_millis(500));
        for scheme in ["https", "http"] {
            let out = std::process::Command::new("curl")
                .args(["-sk", "--max-time", "5", &format!("{}://127.0.0.1:{}/?t={}", scheme, PORT, token())])
                .output()
                .expect("curl");
            let text = String::from_utf8_lossy(&out.stdout);
            assert!(
                text.contains("Сказать команду"),
                "stage=curl-{} address=127.0.0.1:{} curl_status={:?} running={} stdout_len={} stdout_head={:?} stderr={:?}",
                scheme,
                PORT,
                out.status.code(),
                is_running(),
                text.len(),
                text.chars().take(80).collect::<String>(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
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
    #[test]
    fn answers_reach_the_phone_that_asked_and_events_reach_it_always() {
        let _u = crate::uistate::TEST_LOCK.lock();
        let _g = PHONE_TEST_LOCK.lock();
        phone_for_tests(true, true);
        LAST_PHONE_CMD.store(0, Ordering::SeqCst);
        // an answer to a command typed on the computer is not sent to the phone
        announce_answer("четыре");
        assert!(queued_for_tests().is_empty(), "stage=1 an answer to the computer must not reach the phone: {:?} running={}", queued_for_tests(), is_running());
        // after a command of the phone: the answer goes to it, and the phone page says it
        note_phone_command();
        announce_answer("  2 плюс 2 будет четыре  ");
        assert_eq!(queued_for_tests(), vec![("say".to_string(), "2 плюс 2 будет четыре".to_string())], "stage=2 the answer to the phone command");
        // an answer long after that command is not the phone's
        LAST_PHONE_CMD.store(now_secs() - ANSWER_WINDOW_SECS - 5, Ordering::SeqCst);
        OUTBOX.lock().1.clear();
        announce_answer("поздний ответ");
        assert!(queued_for_tests().is_empty(), "stage=3 a late answer: {:?}", queued_for_tests());
        // through the store of the dialogs: an answer of Jarvis (any source) is announced, a question of the user is not
        note_phone_command();
        let dir = tempfile::tempdir().unwrap();
        let store = crate::ai::Store::new(dir.path().to_path_buf());
        let chat = store.create();
        store.append(&chat.id, "user", "сколько будет два плюс два");
        assert!(queued_for_tests().is_empty(), "stage=4 the question stays on the computer: {:?}", queued_for_tests());
        store.append(&chat.id, "assistant", "Четыре");
        assert_eq!(queued_for_tests(), vec![("say".to_string(), "Четыре".to_string())], "stage=5 the answer through the store");
        // a timer, an alarm and the like: always, whoever asked, with the kind, the title and the text
        OUTBOX.lock().1.clear();
        LAST_PHONE_CMD.store(0, Ordering::SeqCst);
        crate::notify::push("timer", "Таймер", "Время вышло: чай готов", false);
        let q = queued_for_tests();
        assert_eq!(q.len(), 1, "stage=6 a timer note: {:?}", q);
        assert_eq!(q[0].0, "note");
        let v: serde_json::Value = serde_json::from_str(&q[0].1).unwrap();
        assert_eq!((v["kind"].as_str(), v["title"].as_str(), v["text"].as_str()), (Some("timer"), Some("Таймер"), Some("Время вышло: чай готов")));
        // nothing is queued when the remote is off
        phone_for_tests(false, false);
        announce_note("timer", "Таймер", "x");
        announce_answer("y");
        assert!(queued_for_tests().is_empty(), "stage=7 remote off: {:?}", queued_for_tests());
    }

    #[test]
    fn a_page_opened_late_does_not_say_old_answers() {
        let _g = PHONE_TEST_LOCK.lock();
        phone_for_tests(true, true);
        enqueue("say", "старый ответ".into());
        OUTBOX.lock().1.back_mut().unwrap().at = now_secs() - 60;
        enqueue("note", "{\"kind\":\"timer\",\"title\":\"Таймер\",\"text\":\"свежий\"}".into());
        let r = poll(0);
        let kinds: Vec<&str> = r["msgs"].as_array().unwrap().iter().map(|m| m["kind"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["note"], "a fresh page gets only what happened a moment ago");
        phone_for_tests(false, false);
    }


    // a duplex made of a request and a place for the answer: the real handler runs on it, no socket
    struct Pipe {
        input: std::io::Cursor<Vec<u8>>,
        output: Vec<u8>,
    }
    impl Read for Pipe {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }
    impl Write for Pipe {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    // a request of the page as it is: the time of the phone and a number that was never used (replay.rs)
    fn phone_request(path: &str, token: &str, body: &str) -> String {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let rid = format!("test-{}-{}", std::process::id(), N.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
        phone_request_with(path, token, body, &now_secs().to_string(), &rid)
    }

    // the same with the time and the number chosen by the test ("" = the header is missing)
    fn phone_request_with(path: &str, token: &str, body: &str, ts: &str, rid: &str) -> String {
        let mut extra = String::new();
        if !ts.is_empty() {
            extra.push_str(&format!("X-Ts: {}\r\n", ts));
        }
        if !rid.is_empty() {
            extra.push_str(&format!("X-Rid: {}\r\n", rid));
        }
        let mut p = Pipe { input: std::io::Cursor::new(format!("POST {} HTTP/1.1\r\nHost: x\r\nX-Token: {}\r\n{}Content-Length: {}\r\n\r\n{}", path, token, extra, body.len(), body).into_bytes()), output: Vec::new() };
        handle(&mut p).unwrap();
        String::from_utf8_lossy(&p.output).to_string()
    }

    fn phone_get(path: &str, token: &str) -> String {
        let mut p = Pipe { input: std::io::Cursor::new(format!("GET {} HTTP/1.1\r\nHost: x\r\nX-Token: {}\r\n\r\n", path, token).into_bytes()), output: Vec::new() };
        handle(&mut p).unwrap();
        String::from_utf8_lossy(&p.output).to_string()
    }

    fn pcm_base64(samples: &[i16]) -> String {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        crate::docai::b64_encode(&bytes)
    }

    // «Мой голос» from the page: every door needs the secret; the voice is recorded from phrases of audio that the page sends; a voice that
    // is not the owner's is stopped before anything is recognized or run
    #[test]
    fn the_page_records_the_voice_of_the_owner_and_the_lock_stops_another_voice_before_recognition() {
        use crate::voiceprint::synth::voice;
        let _g = PHONE_TEST_LOCK.lock();
        let _v = crate::voiceprint::TEST_LOCK.lock();
        phone_for_tests(true, true);
        crate::voiceprint::delete();
        let t = token();
        // the secret
        for path in ["/voiceprint", "/voiceprint-do", "/voice-audio"] {
            let a = if path == "/voiceprint" { phone_get(path, "wrong-token") } else { phone_request(path, "wrong-token", "start") };
            assert!(a.starts_with("HTTP/1.1 403"), "{}: {}", path, a);
        }
        // the buttons
        assert!(phone_request("/voiceprint-do", &t, "format c:").starts_with("HTTP/1.1 400"));
        assert!(phone_request("/voiceprint-do", &t, "enable:1").starts_with("HTTP/1.1 400"), "no print yet");
        let started = phone_request("/voiceprint-do", &t, "start");
        assert!(started.starts_with("HTTP/1.1 200") && started.contains("\"enrolling\":true"), "{}", started);
        // bad audio: not base64, too short
        assert!(phone_request("/voice-audio", &t, "это не звук!").starts_with("HTTP/1.1 400"));
        assert!(phone_request("/voice-audio", &t, &pcm_base64(&voice(110.0, 1.0, 1, 0.1))).starts_with("HTTP/1.1 400"));
        // five phrases of the owner
        for seed in 1..=5u64 {
            let r = phone_request("/voice-audio", &t, &pcm_base64(&voice(110.0, 1.0, seed, 2.5)));
            assert!(r.starts_with("HTTP/1.1 200") && r.contains("\"kind\":\"capture\""), "{}", r);
        }
        assert!(phone_get("/voiceprint", &t).contains("\"enrolled\":true"));
        // the lock on: another voice is refused before the recognition (the message says so), the owner goes on to the recognition
        assert!(phone_request("/voiceprint-do", &t, "enable:1").contains("\"enabled\":true"));
        let other = phone_request("/voice-audio", &t, &pcm_base64(&voice(210.0, 1.2, 41, 2.5)));
        assert!(other.contains("\"kind\":\"refused\"") && other.contains("не распознан как ваш"), "{}", other);
        let owner = phone_request("/voice-audio", &t, &pcm_base64(&voice(110.0, 1.0, 31, 2.5)));
        assert!(!owner.contains("refused"), "the owner is not refused: {}", owner);
        crate::voiceprint::delete();
    }

    // replay: a request is run once; an old one, a repeated one (also after a restart of the server) and one without time or number are not run
    #[test]
    fn a_repeated_old_or_unsigned_request_of_the_phone_is_not_run() {
        let _g = PHONE_TEST_LOCK.lock();
        let _s = crate::uistate::TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::replay::reset_for_tests();
            phone_for_tests(true, true);
            let t = token();
            let now = now_secs().to_string();
            let cap = Capture::start();
            // the first delivery runs
            let a = phone_request_with("/cmd", &t, "открой блокнот", &now, "nonce-first-delivery-1");
            assert!(a.starts_with("HTTP/1.1 200"), "{}", a);
            assert_eq!(cap.taken().len(), 1);
            // the same request again (a retry or a recorded copy): the answer is «ok, already done», nothing runs
            let b = phone_request_with("/cmd", &t, "открой блокнот", &now, "nonce-first-delivery-1");
            assert!(b.starts_with("HTTP/1.1 200") && b.contains("duplicate"), "{}", b);
            assert!(cap.taken().is_empty(), "a repeated delivery does not run the command twice");
            // the same number with another text and another time is the same request for the computer
            let c = phone_request_with("/cmd", &t, "выключи компьютер", &now, "nonce-first-delivery-1");
            assert!(c.contains("duplicate") && cap.taken().is_empty());
            // after a restart of the server the file remembers the number
            crate::replay::forget_memory_for_tests();
            let d = phone_request_with("/cmd", &t, "открой блокнот", &now, "nonce-first-delivery-1");
            assert!(d.contains("duplicate") && cap.taken().is_empty(), "{}", d);
            // an expired request (recorded long ago) and a request from the future
            let old = (now_secs() - crate::replay::WINDOW - 30).to_string();
            let e = phone_request_with("/cmd", &t, "открой блокнот", &old, "nonce-expired-0001");
            assert!(e.starts_with("HTTP/1.1 409") && e.contains("stale") && e.contains("\"now\""), "{}", e);
            let future = (now_secs() + crate::replay::WINDOW + 30).to_string();
            assert!(phone_request_with("/cmd", &t, "открой блокнот", &future, "nonce-future-0001").starts_with("HTTP/1.1 409"));
            // no time, no number, a broken number, an empty body
            for (ts, rid) in [("", "nonce-no-time-0001"), (now.as_str(), ""), (now.as_str(), "short"), (now.as_str(), "бумага-кириллица-1"), ("abc", "nonce-bad-time-01")] {
                let r = phone_request_with("/cmd", &t, "открой блокнот", ts, rid);
                assert!(r.starts_with("HTTP/1.1 400"), "{:?} {:?}: {}", ts, rid, r);
            }
            assert!(phone_request_with("/cmd", &t, "   ", &now, "nonce-empty-body-01").starts_with("HTTP/1.1 400"));
            assert!(cap.taken().is_empty(), "none of those ran anything");
            // the gateway is protected the same way: the second delivery of an action is not run
            let act = "{\"name\":\"setting\",\"key\":\"ai_key\",\"value\":\"x\"}";
            assert!(phone_request_with("/act", &t, act, &now, "nonce-gateway-0001").contains("недоступно"));
            assert!(phone_request_with("/act", &t, act, &now, "nonce-gateway-0001").contains("duplicate"));
            assert!(phone_request_with("/act", &t, "not json {", &now, "nonce-gateway-0002").contains("недоступно"), "a broken body is an answer");
            // the journal knows about the repeats and holds no text of the commands
            let raw = std::fs::read_to_string(crate::store::file("audit_log.json")).unwrap_or_default();
            assert!(raw.contains("повтор запроса") && !raw.contains("открой блокнот") && !raw.contains("выключи компьютер"), "{}", raw);
        });
    }

    // a paired device works, a revoked one is nobody at once, a wrong one too, and the shared link is not a device
    #[test]
    fn a_revoked_or_unknown_device_secret_is_refused_by_the_server() {
        let _g = PHONE_TEST_LOCK.lock();
        let _s = crate::uistate::TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::replay::reset_for_tests();
            phone_for_tests(true, true);
            let code = crate::pairing::start_code(now_secs());
            let secret = crate::pairing::redeem(&code, "Тестовый телефон", now_secs()).unwrap();
            let id = crate::pairing::list()[0].id.clone();
            let act = "{\"name\":\"task\",\"op\":\"clear\"}";
            assert!(phone_request("/act", &secret, act).starts_with("HTTP/1.1 200"), "a paired device is let in");
            assert!(phone_request("/act", &(secret.clone() + "0"), act).starts_with("HTTP/1.1 403"), "a changed secret is not the device");
            assert!(phone_request("/act", &"0".repeat(64), act).starts_with("HTTP/1.1 403"));
            assert!(phone_request("/act", "", act).starts_with("HTTP/1.1 403"));
            assert!(crate::pairing::revoke(&id));
            assert!(phone_request("/act", &secret, act).starts_with("HTTP/1.1 403"), "a revoked device stops at once");
            assert!(phone_get("/api/apps", &secret).starts_with("HTTP/1.1 403"));
            // a device that was not seen for 90 days is no longer accepted
            let code = crate::pairing::start_code(now_secs());
            let s2 = crate::pairing::redeem(&code, "Старый телефон", now_secs()).unwrap();
            assert!(crate::pairing::check(&s2, now_secs()).is_some());
            assert!(crate::pairing::check(&s2, now_secs() + crate::pairing::IDLE_TTL + 10).is_none());
        });
    }

    // the sections of the phone: reading and acting only what ui::phone_get / phone_act name; nothing about keys, the password, the license or files
    #[test]
    fn the_phone_sections_need_the_secret_and_only_the_named_things_work() {
        let _g = PHONE_TEST_LOCK.lock();
        let _s = crate::uistate::TEST_LOCK.lock();
        phone_for_tests(true, true);
        let t = token();
        assert!(phone_get("/api/apps", "wrong-token").starts_with("HTTP/1.1 403"));
        assert!(phone_request("/act", "wrong-token", "{\"name\":\"ess_launch\",\"id\":\"cmd\"}").starts_with("HTTP/1.1 403"));
        let apps = phone_get("/api/apps", &t);
        assert!(apps.starts_with("HTTP/1.1 200") && apps.contains("\"essentials\"") && apps.contains("Командная строка"), "{}", apps);
        for name in ["tasks", "chats", "commands", "scenarios", "devices", "messages", "games", "monitor", "diag", "license", "settings"] {
            let r = phone_get(&format!("/api/{}", name), &t);
            assert!(r.starts_with("HTTP/1.1 200"), "{}: {}", name, r);
        }
        // what is not on the list does not exist for the phone
        for name in ["files", "remote", "avatar", "../x", "voiceprint", "update"] {
            assert!(phone_get(&format!("/api/{}", name), &t).starts_with("HTTP/1.1 404"), "{}", name);
        }
        // the settings that the phone may read hold no key
        let st = phone_get("/api/settings", &t);
        assert!(!st.contains("ai_key") && !st.contains("cloud_tts_key") && !st.contains("token"), "{}", st);
        // actions: the named ones only
        for body in [
            "{\"name\":\"license_activate\"}", "{\"name\":\"owner_set\"}", "{\"name\":\"file_open\",\"path\":\"C:\\\\x.exe\"}", "{\"name\":\"remote_tunnel\"}",
            "{\"name\":\"setting\",\"key\":\"ai_key\",\"value\":\"sk-x\"}", "{\"name\":\"task\",\"op\":\"clear\"}", "{\"name\":\"scenario\",\"op\":\"delete\",\"id\":\"a\"}", "{}",
        ] {
            let r = phone_request("/act", &t, body);
            assert!(r.starts_with("HTTP/1.1 200") && r.contains("недоступно"), "{} -> {}", body, r);
        }
        assert!(crate::uistate::settings().ai_key.is_empty() || crate::uistate::settings().ai_key != "sk-x", "a key cannot be set from the phone");
    }

    // the history of the two sections, the look of the window and the Android home-screen files: the first two need the secret, the last two do not
    #[test]
    fn the_phone_reads_the_history_and_the_look_with_the_secret_and_the_install_files_without() {
        let _g = PHONE_TEST_LOCK.lock();
        let _s = crate::uistate::TEST_LOCK.lock();
        phone_for_tests(true, true);
        let t = token();
        assert!(phone_get("/history?door=phone", "wrong-token").starts_with("HTTP/1.1 403"));
        let h = phone_get("/history?door=phone", &t);
        assert!(h.starts_with("HTTP/1.1 200") && h.contains("\"items\"") && h.contains("\"phone\""), "{}", h);
        let l = phone_get("/look", &t);
        assert!(l.starts_with("HTTP/1.1 200") && l.contains("\"theme\""), "{}", l);
        assert!(phone_get("/look", "wrong-token").starts_with("HTTP/1.1 403"));
        let m = phone_get("/manifest.webmanifest", "");
        assert!(m.starts_with("HTTP/1.1 200") && m.contains("\"standalone\""), "{}", m);
        assert!(phone_get("/icon.png", "").starts_with("HTTP/1.1 200"));
    }

        // the phone chooses the voice of the answers from the list of the computer; nothing else can be set, and a wrong secret sets nothing
    #[test]
    fn the_phone_chooses_a_voice_from_the_list_only_and_only_with_the_secret() {
        let _g = PHONE_TEST_LOCK.lock();
        let _s = crate::uistate::TEST_LOCK.lock();
        phone_for_tests(true, true);
        let t = token();
        let before = crate::uistate::voice_name();
        assert!(phone_get("/voices", "wrong-token").starts_with("HTTP/1.1 403"));
        let list = phone_get("/voices", &t);
        assert!(list.starts_with("HTTP/1.1 200") && list.contains("\"voices\"") && list.contains("\"current\""), "{}", list);
        // what plays now: the secret is needed as for everything else
        assert!(phone_get("/media", "wrong-token").starts_with("HTTP/1.1 403"));
        let media = phone_get("/media", &t);
        assert!(media.starts_with("HTTP/1.1 200") && media.contains("\"items\""), "{}", media);
        assert!(phone_request("/voice", "wrong-token", "piper").starts_with("HTTP/1.1 403"));
        assert_eq!(crate::uistate::voice_name(), before, "a wrong secret changes nothing");
        assert!(phone_request("/voice", &t, "win:../../evil").starts_with("HTTP/1.1 400"));
        assert!(phone_request("/voice", &t, "silero:xenia; calc").starts_with("HTTP/1.1 400"), "only the names of the list");
        assert_eq!(crate::uistate::voice_name(), before);
        assert!(phone_request("/voice", &t, "piper").starts_with("HTTP/1.1 200"));
        assert_eq!(crate::uistate::voice_name(), "piper");
        crate::uistate::update_settings(&serde_json::json!({ "voice_name": before }));
        phone_for_tests(false, false);
    }

    // the door of a phone phrase is set by the phone server; the text cannot name another one
    #[test]
    fn a_phone_phrase_enters_through_the_phone_door_and_cannot_name_another() {
        let _g = PHONE_TEST_LOCK.lock();
        phone_for_tests(true, true);
        let cap = Capture::start();
        let t = token();
        // the words of the phrase, a control character in front (the old mark of "internal") and a wrong token
        assert!(phone_request("/cmd", &t, "\u{1}включи музыку source=voice door=window").starts_with("HTTP/1.1 200"));
        assert!(phone_request("/cmd", "wrong-token", "выключи компьютер").starts_with("HTTP/1.1 403"));
        let got = cap.taken();
        assert_eq!(got.len(), 1, "{:?}", got);
        assert_eq!(got[0].source, Source::Phone);
        assert!(!got[0].via_script);
        assert_eq!(got[0].text, "включи музыку source=voice door=window");
        phone_for_tests(false, false);
    }

    // a script keeps the door of the command that ran it; with no command running it is a plain script
    #[test]
    fn a_script_phrase_cannot_be_more_trusted_than_the_command_that_asked() {
        let cap = Capture::start();
        crate::origin::run_as(Source::Phone, || assert!(submit_from_script("открой блокнот")));
        crate::origin::run_as(Source::Telegram, || assert!(submit_from_script("\u{1}открой блокнот")));
        assert!(submit_from_script("таймер"));
        // the program's own follow-up is the only internal door, and a user entrance never produces it
        assert!(push_internal("найди в интернете кошек"));
        assert!(submit(Source::Window, "\u{1}найди в интернете кошек"));
        let got: Vec<(Source, bool, String)> = cap.taken().into_iter().map(|i| (i.source, i.via_script, i.text)).collect();
        assert_eq!(
            got,
            vec![
                (Source::Phone, true, "открой блокнот".to_string()),
                (Source::Telegram, true, "открой блокнот".to_string()),
                (Source::Script, true, "таймер".to_string()),
                (Source::Internal, false, "найди в интернете кошек".to_string()),
                (Source::Window, false, "найди в интернете кошек".to_string()),
            ]
        );
    }

    // an empty phrase (or only control characters) is not queued at all
    #[test]
    fn an_empty_phrase_is_not_queued() {
        let cap = Capture::start();
        assert!(!submit(Source::Window, "  \u{1}\n "));
        assert!(cap.taken().is_empty());
    }

    // no entrance can queue a phrase without naming its door: the channel is private to this file and the only
    // functions that fill it take a Source (a source text scan keeps it so when somebody adds an entrance)
    #[test]
    fn the_queue_is_filled_only_through_submit() {
        let files = [
            ("remote.rs", include_str!("remote.rs")),
            ("ui.rs", include_str!("ui.rs")),
            ("telegram.rs", include_str!("telegram.rs")),
            ("lua/api/core.rs", include_str!("lua/api/core.rs")),
            ("ipc/server.rs", include_str!("ipc/server.rs")),
            ("scenario.rs", include_str!("scenario.rs")),
        ];
        for (name, text) in files {
            // (the files of a Windows checkout have CRLF line ends)
            let text = text.replace("\r\n", "\n");
            let code = text.split("#[cfg(test)]\nmod tests").next().unwrap();
            assert!(!code.contains("push_text"), "{}: the old unlabeled entrance came back", name);
            assert!(!code.contains("INTERNAL_MARK"), "{}: an in-band mark came back", name);
            if name != "remote.rs" {
                let own_queue = code.replace("BROADCAST_TX", "");
                assert!(!own_queue.contains("TX.get()") && !own_queue.contains("tx.lock().send"), "{}: the queue is filled outside remote::submit", name);
            }
        }
    }

    // the page of the phone shows no list of what Jarvis said; the controls stay (the queue of the server is untouched)
    #[test]
    fn the_phone_page_keeps_no_history_of_jarvis_messages_and_keeps_its_controls() {
        assert!(!PAGE.contains("card('Jarvis'"), "answers are not stacked as cards");
        assert!(!PAGE.contains("children.length > 5"), "no list of the last five messages");
        assert!(PAGE.contains("children.length > 1"), "at most the one link or text that can be tapped");
        assert!(PAGE.contains("function flash("), "what was said is one line that goes out by itself");
        for id in ["g-conn", "c-state", "c-retry", "p-bat", "snd", "inbox", "f", "t", "quick", "media", "mic", "spk", "ringbox", "ringstop"] {
            assert!(PAGE.contains(&format!("id=\"{}\"", id)), "the control {} is gone", id);
        }
        // the answers and events still reach the phone as before (the queue of the server)
        for kind in ["'say'", "'note'", "'ring'", "'stop'", "'link'", "'text'"] {
            assert!(PAGE.contains(&format!("m.kind === {}", kind)), "{} is not handled", kind);
        }
    }

}
