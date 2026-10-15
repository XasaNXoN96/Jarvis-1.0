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
    // request id of the phone page: a request that arrives twice is handled once
    rid: String,
    // the address of the phone from Cloudflare (empty for a connection from the home network)
    client_ip: String,
    body: String,
}

fn read_request<S: Read>(stream: &mut S) -> std::io::Result<Request> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 2048];
    // the whole request must arrive within a few seconds (each read has its own timeout, but a client sending one
    // byte now and then could otherwise hold a connection — and with it a slot — for hours)
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let late = || std::time::Instant::now() > deadline;
    let header_end = loop {
        if late() {
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
                rid = v.chars().take(64).collect();
            } else if k == "cf-connecting-ip" {
                // the address of the phone as Cloudflare saw it (only a request through the tunnel has it)
                client_ip = v.chars().take(64).collect();
            }
        }
    }
    if content_length > 2000 {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    while buf.len() < header_end + content_length {
        if late() {
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
    let supplied = req.token_header.clone().or_else(|| token_in_path(&req.path));
    let key = throttle_key(&req.client_ip);
    let now = std::time::Instant::now();
    if !key.is_empty() && throttled(&key, now) {
        respond(stream, "429 Too Many Requests", "text/plain", "too many wrong tries, wait a minute");
        return Ok(());
    }
    if !token_matches(supplied.as_deref(), &token()) {
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
        // what plays on the computer now (the window titles of the players and of YouTube in a browser)
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

const PAGE: &str = r#"<!doctype html>
<html lang="ru"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Jarvis</title>
<style>
  :root { color-scheme: dark; --bg: #0b0c0e; --bg2: #131418; --card: #17181d; --line: #2b2d35; --text: #ececf0; --dim: #8e909a; --a: #6aa8ff; --a2: #8cc0ff; --ok: #5bd98a; --bad: #ff6b5e; --warn: #fbbf24;
    --tile: linear-gradient(160deg, #1d1f26, #17181d); --btn: linear-gradient(135deg, #4c8dff, #2f66d0); --glow: 0 0 20px #6aa8ff33; }
  * { box-sizing: border-box; -webkit-tap-highlight-color: transparent; }
  html, body { height: 100%; }
  body { margin: 0; background: var(--bg); color: var(--text); font: 15px system-ui, sans-serif; display: flex; flex-direction: column; overflow: hidden; }
  button { font: inherit; color: inherit; cursor: pointer; }
  .ok { color: var(--ok); } .bad { color: var(--bad); } .dim { color: var(--dim); font-size: 12.5px; line-height: 1.45; }

  .top { padding: 14px 16px 10px; background: radial-gradient(120% 90% at 50% 0%, #1b2536 0%, var(--bg2) 62%); border-bottom: 1px solid var(--line); flex: none; }
  .brand { display: flex; align-items: center; gap: 10px; }
  .reactor { width: 32px; height: 32px; border-radius: 50%; flex: none; box-shadow: var(--glow); position: relative;
    background: radial-gradient(circle, #fff 0 12%, #cfe3ff 14% 28%, transparent 30%), conic-gradient(from 0deg, #4c8dff, #9fc8ff, #4c8dff, #9fc8ff, #4c8dff); }
  .reactor::after { content: ""; position: absolute; inset: 5px; border-radius: 50%; border: 2px solid #0008; }
  .brand b { font-size: 17px; letter-spacing: .14em; font-weight: 700; background: linear-gradient(90deg, #cfe3ff, #6aa8ff); -webkit-background-clip: text; background-clip: text; color: transparent; }
  .brand small { display: block; color: var(--dim); font-size: 11.5px; letter-spacing: .03em; margin-top: 1px; }
  .chips { display: flex; gap: 7px; margin-top: 10px; flex-wrap: wrap; }
  .chip { display: inline-flex; align-items: center; gap: 6px; padding: 4px 10px; border-radius: 999px; background: #ffffff0d; border: 1px solid var(--line); font-size: 12px; }
  .chip i { width: 8px; height: 8px; border-radius: 50%; background: var(--dim); }
  .chip.ok i { background: var(--ok); box-shadow: 0 0 8px var(--ok); } .chip.bad i { background: var(--bad); box-shadow: 0 0 8px var(--bad); } .chip.warn i { background: var(--warn); box-shadow: 0 0 8px var(--warn); }

  .body { flex: 1; overflow-y: auto; padding: 14px 14px 10px; -webkit-overflow-scrolling: touch; }
  .tab { display: none; } .tab.on { display: block; }
  section { margin-bottom: 16px; }
  section h2 { margin: 0 2px 8px; font-size: 11.5px; letter-spacing: .14em; text-transform: uppercase; color: var(--a); font-weight: 600; }
  .row { display: flex; justify-content: space-between; align-items: center; gap: 10px; padding: 11px 12px; border: 1px solid var(--line); border-radius: 14px; background: var(--tile); margin-bottom: 8px; font-size: 14px; }
  .row small { color: var(--dim); display: block; font-size: 11.5px; margin-top: 2px; line-height: 1.35; }
  .row b { font-weight: 600; text-align: right; }
  label.row input[type=checkbox] { width: 42px; height: 24px; flex: none; appearance: none; -webkit-appearance: none; border-radius: 999px; background: #ffffff22; position: relative; margin: 0; cursor: pointer; transition: .15s; }
  label.row input[type=checkbox]::after { content: ""; position: absolute; top: 3px; left: 3px; width: 18px; height: 18px; border-radius: 50%; background: #fff; transition: .15s; }
  label.row input[type=checkbox]:checked { background: #4c8dff; } label.row input[type=checkbox]:checked::after { left: 21px; }
  .btn { border: 1px solid var(--line); background: #ffffff0a; border-radius: 12px; padding: 11px 14px; }
  .btn.wide { width: 100%; margin-bottom: 8px; } .btn.dash { border-style: dashed; color: var(--dim); background: transparent; }
  select { width: 100%; padding: 11px 12px; border-radius: 12px; border: 1px solid var(--line); background: #0d0e11; color: var(--text); font: inherit; margin-top: 8px; }

  .grid4 { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
  .tile { aspect-ratio: 1 / 1; border-radius: 16px; border: 1px solid var(--line); background: var(--tile); display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 5px; padding: 4px; transition: .15s; }
  .tile span { font-size: 21px; line-height: 1; } .tile em { font-style: normal; font-size: 10px; color: var(--dim); text-align: center; line-height: 1.15; max-width: 100%; overflow-wrap: anywhere; }
  .tile:active { border-color: var(--a); box-shadow: 0 0 14px #6aa8ff33 inset; transform: translateY(1px); }
  .tile.hot span { color: var(--a2); }
  .tile.yes { border-color: #2f7a4a; background: linear-gradient(160deg, #15291d, #101b14); } .tile.yes span { color: var(--ok); } .tile.yes em { color: #9be0b4; }
  .tile.no { border-color: #8a3a34; background: linear-gradient(160deg, #2c1614, #1b0f0e); } .tile.no span { color: var(--bad); } .tile.no em { color: #f0a49c; }

  .player { border: 1px solid var(--line); background: var(--tile); border-radius: 18px; padding: 12px 10px; display: grid; gap: 10px; }
  .ctrl { display: flex; align-items: center; justify-content: center; gap: 10px; }
  .ctrl button { width: 48px; height: 48px; border-radius: 50%; border: 1px solid var(--line); background: #ffffff0a; font-size: 18px; }
  .ctrl button.main { width: 62px; height: 62px; background: var(--btn); border: 0; font-size: 24px; box-shadow: var(--glow); color: #fff; }
  .vol { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 8px; }
  .vol button { height: 40px; border-radius: 12px; border: 1px solid var(--line); background: #ffffff0a; font-size: 16px; }

  #status { min-height: 20px; color: var(--a2); font-size: 13px; margin: 0 2px 8px; word-break: break-word; }
  .msg { border: 1px solid #2f66d0; background: #0d1a33; border-radius: 14px; padding: 10px 12px; margin-bottom: 10px; }
  .msg .body { word-break: break-all; user-select: all; margin: 6px 0 8px; padding: 0; overflow: visible; }
  .msg .acts { display: flex; gap: 8px; } .msg .acts button { padding: 9px 12px; border-radius: 10px; border: 1px solid var(--line); background: #ffffff0f; }
  .msg .acts button:not(.sec) { background: var(--btn); border: 0; color: #fff; }

  .orbwrap { display: grid; place-items: center; padding: 4px 0 6px; gap: 6px; }
  #mic { width: min(58vw, 210px); aspect-ratio: 1; border: 0; padding: 0; border-radius: 50%; background: transparent; position: relative; }
  #mic canvas { width: 100%; height: 100%; display: block; }
  #mic.on::after { content: ""; position: absolute; inset: 4%; border-radius: 50%; border: 2px solid #5bd1c8aa; animation: ring 1.3s infinite; }
  @keyframes ring { from { transform: scale(.92); opacity: .9; } to { transform: scale(1.12); opacity: 0; } }
  #orb-cap { font-size: 12.5px; color: var(--dim); text-align: center; letter-spacing: .02em; }
  #heard { min-height: 18px; color: var(--a2); font-size: 14px; text-align: center; padding: 0 8px; word-break: break-word; }
  #help { display: none; border: 1px solid var(--line); border-radius: 14px; background: var(--tile); padding: 12px; margin-bottom: 8px; font-size: 13.5px; line-height: 1.5; }
  #help .btn { margin-top: 6px; }

  #confirm { display: none; flex: none; padding: 10px 12px; border-top: 1px solid var(--line); background: #101319; align-items: center; gap: 10px; }
  #confirm.on { display: flex; } #confirm .q { flex: 1; min-width: 0; font-size: 13px; line-height: 1.35; }
  #confirm button { flex: none; min-width: 74px; padding: 12px 14px; border-radius: 14px; border: 1px solid; font-weight: 600; font-size: 15px; }
  #confirm .yes { background: linear-gradient(160deg, #1d4a2e, #143321); border-color: #2f7a4a; color: #b6f0c9; }
  #confirm .no { background: linear-gradient(160deg, #4a1f1b, #331411); border-color: #8a3a34; color: #ffc1b9; }

  form#f { display: flex; gap: 8px; padding: 10px 12px; border-top: 1px solid var(--line); background: var(--bg2); flex: none; margin: 0; }
  form#f input { flex: 1; min-width: 0; padding: 12px 14px; border-radius: 14px; border: 1px solid var(--line); background: #00000055; color: var(--text); font: 16px system-ui, sans-serif; }
  form#f button { width: 50px; border: 0; border-radius: 14px; background: var(--btn); color: #fff; font-size: 18px; box-shadow: var(--glow); }
  .tabs { display: grid; grid-template-columns: repeat(4, 1fr); background: var(--bg2); border-top: 1px solid var(--line); padding: 6px 6px calc(10px + env(safe-area-inset-bottom)); flex: none; }
  .tabs button { border: 0; background: transparent; color: var(--dim); display: grid; gap: 2px; justify-items: center; padding: 6px 0; font-size: 11px; border-radius: 12px; }
  .tabs button span { font-size: 19px; } .tabs button.on { color: var(--a2); } .tabs button.on span { filter: drop-shadow(0 0 6px #6aa8ff); }
  /* every screen: a phone, a tablet (upright and sideways) and a computer's browser. The content is a column of at most 760px
     in the middle (the bars at the top and bottom keep the same edges); the sphere, the icons and the texts grow with the width */
  :root { --pad: max(14px, calc((100vw - 760px) / 2)); }
  body { font-size: clamp(15px, 1.9vw, 18px); }
  .top { padding-inline: var(--pad); } .body { padding-inline: var(--pad); } #confirm { padding-inline: var(--pad); } form#f { padding-inline: var(--pad); } .tabs { padding-inline: var(--pad); }
  .brand b { font-size: clamp(17px, 3vw, 24px); } .brand small { font-size: clamp(11.5px, 1.8vw, 14px); } .reactor { width: clamp(32px, 6vw, 46px); height: clamp(32px, 6vw, 46px); }
  .chip { font-size: clamp(12px, 1.9vw, 15px); padding: 4px clamp(10px, 1.8vw, 14px); }
  section h2 { font-size: clamp(11.5px, 1.8vw, 14px); }
  #mic { width: min(66vw, 40vh, 420px); min-width: 170px; }
  #orb-cap { font-size: clamp(12.5px, 2vw, 16px); } #heard { font-size: clamp(14px, 2.2vw, 18px); }
  .tile span { font-size: clamp(22px, 6vw, 42px); } .tile em { font-size: clamp(10px, 2.2vw, 14px); }
  .tile { border-radius: clamp(16px, 2.6vw, 22px); } .grid4 { gap: clamp(8px, 1.6vw, 14px); }
  .player { padding: clamp(12px, 2.4vw, 20px) clamp(10px, 2vw, 18px); gap: clamp(10px, 2vw, 16px); }
  .ctrl { gap: clamp(10px, 2.4vw, 20px); }
  .ctrl button { width: clamp(48px, 12vw, 76px); height: clamp(48px, 12vw, 76px); font-size: clamp(18px, 4.6vw, 30px); }
  .ctrl button.main { width: clamp(62px, 16vw, 100px); height: clamp(62px, 16vw, 100px); font-size: clamp(24px, 6vw, 38px); }
  .vol button { height: clamp(40px, 8vw, 62px); font-size: clamp(16px, 4vw, 28px); }
  .row { font-size: clamp(14px, 2.1vw, 17px); padding: clamp(11px, 1.8vw, 16px) clamp(12px, 2vw, 18px); } .row small { font-size: clamp(11.5px, 1.7vw, 14px); }
  form#f input { font-size: clamp(16px, 2.2vw, 19px); padding-block: clamp(12px, 1.8vw, 16px); } form#f button { width: clamp(50px, 8vw, 66px); }
  .tabs button { font-size: clamp(11px, 1.8vw, 14px); } .tabs button span { font-size: clamp(19px, 3.6vw, 28px); }
  #status { font-size: clamp(13px, 2vw, 16px); }
  /* a low window (a phone sideways): the sphere is smaller so that the buttons stay in sight */
  @media (max-height: 520px) { #mic { width: min(40vh, 220px); min-width: 120px; } }
  .np-item { margin-top: 6px; font-size: clamp(15px, 2.4vw, 19px); line-height: 1.35; word-break: break-word; } .np-item b { color: var(--a2); }
  #ringbox { display: none; position: fixed; inset: 0; background: rgba(218,54,51,.94); z-index: 9; text-align: center; padding-top: 30vh; font-size: 22px; }
  #ringbox button { font-size: 22px; padding: 18px 30px; background: #fff; color: #da3633; margin-top: 20px; border: 0; border-radius: 14px; }
</style></head><body>
<div id="ringbox">📢 Компьютер ищет этот телефон<br><button id="ringstop" type="button">Я нашёл телефон</button></div>

<div class="top">
  <div class="brand"><div class="reactor"></div><div><b>JARVIS</b><small>пульт · ваш компьютер</small></div></div>
  <div class="chips"><span class="chip" id="chip-conn"><i></i><span>Связь…</span></span><span class="chip" id="chip-bat"><i></i><span>Заряд —</span></span><span class="chip" id="chip-voice"><i></i><span>Голос: выкл</span></span></div>
</div>

<div class="body">
  <div class="tab on" id="t-home">
    <div class="orbwrap"><button id="mic" type="button" aria-label="Сказать команду" title="Сказать команду"><canvas id="orb" width="360" height="360"></canvas></button>
      <div id="orb-cap">Нажмите на сферу и скажите команду</div><div id="heard"></div></div>
    <div id="status"></div>
    <div id="inbox"></div>
    <section id="g-quick"><h2>Быстрые действия</h2>
      <div class="grid4" id="quick"></div>
    </section>
  </div>

  <div class="tab" id="t-media">
    <section id="g-media"><h2>Звук и музыка</h2>
      <div class="row" id="nowplaying" style="display:block"><div>Сейчас играет<small id="np-note">смотрю, что запущено на компьютере…</small></div><div id="np-list"></div></div>
      <div class="player" id="media"></div>
      <div class="dim" style="margin-top:6px">Кнопки управляют звуком и музыкой <b>компьютера</b>. Приложения самого телефона веб-страница переключать не может.</div>
    </section>
  </div>

  <div class="tab" id="t-phone">
    <section id="g-voice"><h2>Голос на телефоне</h2>
      <label class="row"><div>Непрерывный режим<small>слушать команду за командой</small></div><input type="checkbox" id="cont"></label>
      <label class="row"><div>Озвучивать ответы<small>на этом телефоне</small></div><input type="checkbox" id="spk" checked></label>
      <div class="row"><div>Микрофон</div><b id="v-state">выключен</b></div>
      <div id="help">
        <b>Как включить голос на этом телефоне (один раз)</b><br>
        <span id="yandex" style="display:none;color:#ffb347">Яндекс.Браузер для этого не подходит. Откройте эту ссылку в <b>Chrome</b>.<br></span>
        Браузер даёт микрофон только защищённой странице (https). У пульта она есть на том же компьютере:<br>
        1. Нажмите <b>«Открыть защищённую страницу»</b> ниже.<br>
        2. Браузер один раз предупредит о сертификате (он сделан самим компьютером, а не центром сертификации): нажмите «Дополнительно» → «Перейти на сайт».<br>
        3. Нажмите на сферу и разрешите микрофон.<br>
        <button id="to-https" class="btn wide" type="button">Открыть защищённую страницу</button>
        <span class="dim">Не хотите настраивать? Нажмите на значок микрофона на клавиатуре телефона в поле команды: он диктует голосом и работает без настроек.</span>
      </div>
      <button id="howto" class="btn dash wide" type="button">Не работает голос? Как включить</button>
      <div class="dim">Речь распознаёт сам браузер телефона (в Chrome — сервис Google через интернет). Компьютер получает только готовый текст команды.</div>
    </section>
    <section id="g-phone"><h2>Телефон</h2>
      <div class="row"><div>Заряд<small id="p-note">обновляется при подключении</small></div><b id="p-bat">—</b></div>
      <section id="g-find" style="margin:0"><div class="row"><div>Сигнал «Найти телефон»<small id="snd-note">Нажмите один раз: потом компьютер сможет позвать телефон командой «Найди телефон».</small></div><button class="btn" id="snd" type="button">🔔 Разрешить</button></div></section>
      <label class="row" id="wl-row" style="display:none"><div>Не гасить экран<small>пока открыт пульт</small></div><input type="checkbox" id="wl"></label>
    </section>
  </div>

  <div class="tab" id="t-more">
    <section id="g-conn"><h2>Подключение</h2>
      <div class="row"><div>Связь с компьютером<small>последнее соединение: <span id="c-last">—</span></small></div><b id="c-state">проверяю…</b></div>
      <div class="dim" id="c-err"></div>
      <button class="btn wide" id="c-retry" type="button">Переподключиться</button>
      <div class="dim">Пульт работает, пока эта страница открыта на экране: свёрнутый браузер телефон усыпляет.</div>
    </section>
    <section id="g-voicepick"><h2>Голос Джарвиса</h2>
      <div class="row" style="display:block"><div>Кто отвечает<small>тот же выбор, что в Настройках → Голос на компьютере</small></div><select id="vsel" aria-label="Голос Джарвиса"><option value="">Загружаю список…</option></select><div class="dim" id="vmsg"></div></div>
    </section>
    <section id="g-limits"><h2>Чего пульт не делает</h2>
      <div class="dim">Пульт — это страница в браузере, а не приложение. Поэтому он не блокирует и не усыпляет телефон, не читает уведомления телефона и не управляет его другими приложениями. Обойти экран блокировки, PIN-код или разрешения Android он не пытается.</div>
    </section>
  </div>
</div>

<div id="confirm"><div class="q" id="confirm-q"></div><button class="yes" id="cy" type="button">Да</button><button class="no" id="cn" type="button">Нет</button></div>
<form id="f"><input id="t" placeholder="Команда: таймер на 5 минут" autocomplete="off"><button aria-label="Отправить">➤</button></form>
<div class="tabs" id="tabs">
  <button class="on" data-tab="home" type="button"><span>⚡</span>Главная</button>
  <button data-tab="media" type="button"><span>🎵</span>Звук</button>
  <button data-tab="phone" type="button"><span>📱</span>Телефон</button>
  <button data-tab="more" type="button"><span>⚙</span>Ещё</button>
</div>
<script>
// the secret of the link is taken from the address once and kept on this phone: it does not stay in the address
// bar, the browser history or a shared screenshot ("Новая ссылка" on the computer revokes it)
const token = (() => {
  // (?t=… from the QR code; #t=… when the page moves to its https address: the part after # never reaches a server)
  const fromUrl = new URLSearchParams(location.search).get('t') || new URLSearchParams(location.hash.slice(1)).get('t') || '';
  let t = fromUrl;
  try { if (fromUrl) localStorage.setItem('jarvis_t', fromUrl); else t = localStorage.getItem('jarvis_t') || ''; } catch (e) {}
  if (fromUrl || location.hash) { try { history.replaceState(null, '', location.pathname); } catch (e) {} }
  return t;
})();
// no request waits forever: the computer asleep, another network or a lost packet ends in "нет связи" within seconds
function fetchT(url, opts) {
  const c = new AbortController(); const t = setTimeout(() => c.abort(), 6000);
  return fetch(url, Object.assign({}, opts || {}, { signal: c.signal })).finally(() => clearTimeout(t));
}
const $ = id => document.getElementById(id);
const rid = () => 'p' + Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
const quick = [
  ['поставь таймер на 5 минут', '⏱', 'Таймер 5 мин', 'hot'], ['отмени таймер', '⏹', 'Отмена таймера', ''], ['сколько времени', '🕐', 'Сколько времени', ''], ['отмени выключение', '⛔', 'Отмена выкл.', ''],
  ['выключи компьютер через 10 минут', '⏻', 'Выкл. через 10 мин', ''], ['заблокируй компьютер', '🔒', 'Блокировка', ''], ['да', '✔', 'Да', 'yes'], ['нет', '✖', 'Нет', 'no']];
const status = $('status');
async function send(text) {
  status.textContent = '…';
  try {
    const r = await fetchT('/cmd', {method:'POST', headers:{'X-Token':token, 'X-Rid':rid()}, body:text});
    status.textContent = r.ok ? '✓ ' + text : r.status === 503 ? 'Компьютер пока не принимает команды (Jarvis запускается или пульт выключен)'
      : r.status === 403 ? 'Ссылка не подходит: откройте новую ссылку или QR-код на компьютере' : 'Ошибка: ' + r.status;
    if (r.ok) connected();
  } catch (e) { status.textContent = 'Нет связи с компьютером'; }
}
for (const [cmd, icon, label, cls] of quick) {
  const b = document.createElement('button'); b.type = 'button'; b.className = 'tile ' + cls; b.title = cmd;
  const i = document.createElement('span'); i.textContent = icon; const e = document.createElement('em'); e.textContent = label;
  b.append(i, e); b.onclick = () => send(cmd); $('quick').appendChild(b);
}
// the music and the sound of the computer: one player, the same eight commands as before
const player = [['ctrl', [['предыдущий трек', '⏮', 'Предыдущий трек', ''], ['пауза', '⏸', 'Пауза', ''], ['продолжи', '⏯', 'Продолжить', 'main'], ['следующий трек', '⏭', 'Следующий трек', '']]],
  ['vol', [['тише', '🔉', 'Тише', ''], ['громче', '🔊', 'Громче', ''], ['выключи звук', '🔇', 'Выключить звук', ''], ['включи звук', '🔈', 'Включить звук', '']]]];
for (const [kind, items] of player) {
  const row = document.createElement('div'); row.className = kind;
  for (const [cmd, icon, label, cls] of items) {
    const b = document.createElement('button'); b.type = 'button'; b.textContent = icon; b.title = label; b.setAttribute('aria-label', label); if (cls) b.className = cls; b.onclick = () => send(cmd); row.appendChild(b);
  }
  $('media').appendChild(row);
}
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
    const r = await fetchT('/poll?after=' + lastId, {headers:{'X-Token':token}});
    if (r.status === 403) { lost('Ссылка больше не действует (на компьютере сделали новую): отсканируйте QR-код в окне Jarvis ещё раз.'); try { localStorage.removeItem('jarvis_t'); } catch (e) {} }
    else if (r.status === 503) { lost('Пульт выключен на компьютере. Включите его на главной странице Jarvis.'); }
    else if (!r.ok) { lost('Компьютер ответил ошибкой ' + r.status + '.'); }
    else { const j = await r.json(); connected(); for (const m of j.msgs) { if (m.id > shownId || j.last < shownId) { handle(m); shownId = m.id; } } lastId = j.last; if (j.last < shownId) shownId = j.last; }
  } catch (e) { lost('Компьютер не отвечает: Jarvis закрыт, компьютер спит, пульт выключен или телефон в другой сети. Из другой сети пульт открывается по адресу «Доступ из любой сети» (включается в окне Jarvis или кнопкой в Telegram-боте). Повторяю автоматически.' + (lastOk ? ' Последняя связь: ' + hhmm(lastOk) + '.' : '')); }
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
  try { await fetchT('/state', {method:'POST', headers:{'X-Token':token, 'Content-Type':'application/json'}, body: JSON.stringify(body)}); } catch (e) {}
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
  // no history of messages on the page: only the one thing that can be tapped (a link, a text) stays, until it is closed or replaced
  while ($('inbox').children.length > 1) $('inbox').lastChild.remove();
  if (navigator.vibrate) navigator.vibrate(120);
}
// the answers and events of the computer are said by the phone itself (its own voice), and always shown as a card
function speakHere(text) {
  if (!$('spk').checked || !('speechSynthesis' in window)) return;
  try { speechSynthesis.cancel(); const u = new SpeechSynthesisUtterance(text); u.lang = /^(en|uk)/i.test(navigator.language || '') ? navigator.language : 'ru-RU'; u.onstart = () => setOrb('speaking'); u.onend = u.onerror = () => setOrb(state.voice === 'listening' ? 'listening' : 'idle'); speechSynthesis.speak(u); } catch (e) {}
}
try { $('spk').checked = localStorage.getItem('jarvis_spk') !== '0'; } catch (e) {}
$('spk').onchange = () => { try { localStorage.setItem('jarvis_spk', $('spk').checked ? '1' : '0'); } catch (e) {} if ($('spk').checked) speakHere('Озвучка на телефоне включена'); else if ('speechSynthesis' in window) speechSynthesis.cancel(); };
// what Jarvis said is not kept on the page (no list of messages): one short line that goes out by itself
let flashTimer = 0;
function flash(text) {
  const st = $('status'); st.textContent = String(text || '').slice(0, 160);
  clearTimeout(flashTimer); flashTimer = setTimeout(() => { st.textContent = ''; }, 8000);
}
function handle(m) {
  if (m.kind === 'say') { flash('Jarvis: ' + m.data); speakHere(m.data); askIfQuestion(m.data); }
  else if (m.kind === 'note') {
    let n = {}; try { n = JSON.parse(m.data); } catch (e) { n = { title: 'Jarvis', text: m.data }; }
    flash((n.title ? n.title + ': ' : '') + (n.text || ''));
    askIfQuestion((n.title ? n.title + '. ' : '') + (n.text || ''));
    if (['timer', 'alarm', 'reminder'].includes(n.kind)) { unlockSound(); beep(0); setTimeout(() => beep(1), 450); if (navigator.vibrate) navigator.vibrate([300, 150, 300]); }
    speakHere((n.title ? n.title + '. ' : '') + (n.text || ''));
  }
  else if (m.kind === 'ring') ring();
  else if (m.kind === 'stop') stopRing();
  else if (m.kind === 'link' && /^https?:\/\//i.test(m.data)) card('Ссылка с компьютера', m.data, [['Открыть', () => window.open(m.data, '_blank', 'noopener')], ['Копировать', x => copy(m.data, x)]]);
  else if (m.kind === 'text') card('Текст с компьютера', m.data, [['Копировать', x => copy(m.data, x)]]);
}

// ---------- the sphere (as on the computer): dots on a sphere; it breathes while listening and ripples while speaking ----------
let oState = 'idle', oLevel = 0, oYaw = 0;
const orbCanvas = $('orb'), octx = orbCanvas.getContext('2d');
const orbPts = [];
for (let i = 0, N = 520; i < N; i++) {
  const y = 1 - (i / (N - 1)) * 2, r = Math.sqrt(1 - y * y), th = Math.PI * (3 - Math.sqrt(5)) * i;
  orbPts.push([Math.cos(th) * r, y, Math.sin(th) * r, Math.random() * Math.PI * 2, Math.atan2(Math.sin(th) * r, Math.cos(th) * r), Math.asin(y)]);
}
function setOrb(st) { oState = st; }
function orbPalette() { return oState === 'error' ? [352, 75] : oState === 'off' ? [220, 6] : oState === 'listening' ? [172, 70] : oState === 'speaking' ? [186, 55] : [176, 45]; }
function drawOrb(t) {
  const W = orbCanvas.width, H = orbCanvas.height, [hue, sat] = orbPalette();
  octx.clearRect(0, 0, W, H);
  const cx = W / 2, cy = H / 2, R = W * 0.33, tilt = 0.38, ct = Math.cos(tilt), st = Math.sin(tilt), cyaw = Math.cos(oYaw), syaw = Math.sin(oYaw);
  const active = oState === 'listening' || oState === 'speaking';
  oLevel += ((active ? 0.55 + 0.45 * Math.sin(t / 190) : 0) - oLevel) * 0.12;
  const glow = octx.createRadialGradient(cx, cy, R * 0.05, cx, cy, R * 1.25);
  glow.addColorStop(0, `hsla(${hue},${sat}%,55%,${0.16 + oLevel * 0.22})`); glow.addColorStop(0.55, `hsla(${hue},${sat}%,40%,${0.05 + oLevel * 0.06})`); glow.addColorStop(1, `hsla(${hue},${sat}%,30%,0)`);
  octx.fillStyle = glow; octx.fillRect(0, 0, W, H);
  const breath = 1 + Math.sin(t / 900) * 0.012 + (oState === 'listening' ? Math.sin(t / 160) * 0.025 * oLevel : 0);
  for (const [x0, y0, z0, ph, lon, lat] of orbPts) {
    let k = breath; if (oState === 'speaking') k += 0.075 * oLevel * Math.sin(3 * lon + t / 170) * Math.sin(2 * lat + t / 230); k += 0.012 * Math.sin(ph + t / 700);
    const x = (x0 * cyaw + z0 * syaw) * k, z1 = (-x0 * syaw + z0 * cyaw) * k, y1 = y0 * k;
    const y = y1 * ct - z1 * st, z = y1 * st + z1 * ct, persp = 3.2 / (3.2 - z), depth = (z + 1) / 2;
    octx.fillStyle = `hsla(${hue},${sat * (1 - depth * 0.8)}%,${62 + depth * 36}%,${(0.10 + depth * 0.9) * (oState === 'off' ? 0.45 : 1)})`;
    const r = 2.6 + depth * 3.2; octx.fillRect(cx + x * R * persp - r / 2, cy + y * R * persp - r / 2, r, r);
  }
}
let orbLast = 0;
function orbLoop(t) { if (document.visibilityState === 'visible' && t - orbLast > 33) { orbLast = t; oYaw += 0.006 * (oState === 'listening' ? 2 : 1); drawOrb(t); } requestAnimationFrame(orbLoop); }
requestAnimationFrame(orbLoop);

// ---------- voice: the browser listens through the phone microphone and sends the recognized text ----------
// Browsers allow the microphone only on secure pages (https); on plain http over Wi-Fi it may be refused.
const micBtn = $('mic'), heard = $('heard');
const SR = window.SpeechRecognition || window.webkitSpeechRecognition;
let rec = null, wantVoice = false;
// the https address of the same computer and port; the secret goes after # (never sent to a server, removed at once)
$('to-https').onclick = () => { location.href = 'https://' + location.host + '/#t=' + encodeURIComponent(token); };
if (location.protocol === 'https:') $('to-https').style.display = 'none';
if (/YaBrowser/i.test(navigator.userAgent)) $('yandex').style.display = 'inline';
$('howto').onclick = () => { $('help').style.display = $('help').style.display === 'none' ? 'block' : 'none'; };
function voice(st, err) {
  state.voice = st; state.voice_error = err || '';
  const names = { off: 'выключен', listening: 'слушаю', error: 'ошибка', unavailable: 'недоступен' };
  setOrb(st === 'listening' ? 'listening' : st === 'error' ? 'error' : st === 'unavailable' ? 'off' : 'idle');
  $('v-state').textContent = names[st] || st; $('v-state').className = st === 'listening' ? 'ok' : (st === 'error' || st === 'unavailable') ? 'bad' : '';
  if (err) heard.textContent = err;
  reportSoon();
}
if (!window.isSecureContext) { voice('unavailable', 'Голос: нужна защищённая страница пульта (кнопка ниже). Текстом всё работает.'); }
else if (!SR) { voice('unavailable', 'Этот браузер не умеет распознавать речь. Используйте Chrome или микрофон на клавиатуре.'); }
function listen() {
  if (!window.isSecureContext) { $('help').style.display = 'block'; voice('unavailable', 'Браузер не разрешает микрофон на этой странице. Откройте защищённую страницу (ниже).'); wantVoice = false; return; }
  if (!SR) { voice('unavailable', 'Браузер не умеет слушать. Нажмите значок микрофона на клавиатуре в поле команды.'); wantVoice = false; return; }
  rec = new SR(); rec.lang = navigator.language && !/^ru/i.test(navigator.language) && /^(en|uk)/i.test(navigator.language) ? navigator.language : 'ru-RU'; rec.interimResults = true; rec.continuous = false;
  micBtn.classList.add('on'); heard.textContent = 'Слушаю… (нажмите на сферу, чтобы остановить)'; voice('listening');
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
    rec = null; micBtn.classList.remove('on');
    if (final.trim()) send(final.trim());
    if (wantVoice && !fatal) setTimeout(() => { if (wantVoice && !rec) listen(); }, 300);
    else if (!fatal) voice('off');
  };
  try { rec.start(); } catch (e) { rec = null; micBtn.classList.remove('on'); voice('error', 'Не удалось включить микрофон: ' + e.message); wantVoice = false; }
}
micBtn.onclick = () => { if (rec) { wantVoice = false; $('cont').checked = false; rec.stop(); return; } wantVoice = $('cont').checked; listen(); };
$('cont').onchange = () => { wantVoice = $('cont').checked; if (wantVoice && !rec) listen(); if (!wantVoice && rec) rec.stop(); };

// ---------- what plays on the computer now (AIMP, YouTube in a browser ...), asked while the sound tab is open ----------
let npTimer = 0;
async function loadNow() {
  clearTimeout(npTimer);
  if (!$('t-media').classList.contains('on')) return;
  try {
    const r = await fetchT('/media', {headers:{'X-Token':token}});
    if (!r.ok) throw new Error(r.status);
    const j = await r.json(); const box = $('np-list'); box.textContent = '';
    for (const it of j.items) {
      const d = document.createElement('div'); d.className = 'np-item';
      const a = document.createElement('b'); a.textContent = it.app; const t = document.createElement('span'); t.textContent = it.title ? ' — ' + it.title : ' запущен';
      d.append(a, t); box.appendChild(d);
    }
    $('np-note').textContent = j.items.length ? 'по заголовкам окон плееров и браузера' : 'AIMP, VLC, Spotify и YouTube в браузере не найдены. Ролик в фоновой вкладке браузера не виден.';
  } catch (e) { $('np-note').textContent = 'Нет связи с компьютером.'; }
  npTimer = setTimeout(loadNow, 4000);
}

// ---------- tabs ----------
document.querySelectorAll('#tabs button').forEach(b => b.onclick = () => {
  document.querySelectorAll('#tabs button').forEach(x => x.classList.toggle('on', x === b));
  document.querySelectorAll('.tab').forEach(t => t.classList.toggle('on', t.id === 't-' + b.dataset.tab));
  if (b.dataset.tab === 'more') loadVoices();
  if (b.dataset.tab === 'media') loadNow();
});

// ---------- the signs at the top mirror the lines below ----------
function chip(id, cls, text) { const c = $(id); c.className = 'chip ' + cls; c.lastChild.textContent = text; }
function syncChips() {
  const st = $('c-state').textContent;
  chip('chip-conn', $('c-state').className === 'ok' ? 'ok' : $('c-state').className === 'bad' ? 'bad' : '', st === 'есть' ? 'Связь есть' : st === 'нет связи' ? 'Нет связи' : 'Связь…');
  const bat = $('p-bat').textContent; chip('chip-bat', bat === '—' ? '' : 'ok', bat === '—' ? 'Заряд —' : '🔋 ' + bat.replace(' ⚡ заряжается', ' ⚡'));
  const v = state.voice; chip('chip-voice', v === 'listening' ? 'ok' : (v === 'error' || v === 'unavailable') ? 'bad' : 'warn', 'Голос: ' + ({ off: 'выкл', listening: 'слушаю', error: 'ошибка', unavailable: 'нет' }[v] || v));
}
setInterval(syncChips, 700); syncChips();

// ---------- the voice of Jarvis (the list and the choice of the computer) ----------
let voicesLoaded = 0;
async function loadVoices() {
  if (Date.now() - voicesLoaded < 30000) return;
  try {
    const r = await fetchT('/voices', {headers:{'X-Token':token}});
    if (!r.ok) throw new Error(r.status);
    const j = await r.json(); const sel = $('vsel'); sel.textContent = '';
    for (const v of j.voices) sel.add(new Option(v.label, v.id));
    sel.value = j.current; if (sel.value !== j.current) sel.value = '';
    voicesLoaded = Date.now(); $('vmsg').textContent = '';
  } catch (e) { $('vmsg').textContent = 'Не удалось получить список голосов с компьютера.'; }
}
$('vsel').onchange = async () => {
  $('vmsg').textContent = 'Сохраняю…';
  try {
    const r = await fetchT('/voice', {method:'POST', headers:{'X-Token':token}, body:$('vsel').value});
    $('vmsg').textContent = r.ok ? 'Готово: компьютер отвечает этим голосом.' : 'Компьютер не принял этот голос.';
  } catch (e) { $('vmsg').textContent = 'Нет связи с компьютером.'; }
};

// ---------- a question "Подтвердить? да / нет": the answer is two big buttons ----------
let confirmTimer = 0;
function askIfQuestion(text) {
  if (!/подтверд|скажите «?да»? или «?нет|да или нет|да\/нет/i.test(String(text || ''))) return;
  $('confirm-q').textContent = String(text).slice(0, 140); $('confirm').classList.add('on');
  clearTimeout(confirmTimer); confirmTimer = setTimeout(() => $('confirm').classList.remove('on'), 30000);
}
for (const [id, word] of [['cy', 'да'], ['cn', 'нет']]) $(id).onclick = () => { $('confirm').classList.remove('on'); clearTimeout(confirmTimer); send(word); };

poll(); report();
</script></body></html>
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> std::net::Ipv4Addr {
        s.parse().unwrap()
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

    fn phone_request(path: &str, token: &str, body: &str) -> String {
        let mut p = Pipe { input: std::io::Cursor::new(format!("POST {} HTTP/1.1\r\nHost: x\r\nX-Token: {}\r\nContent-Length: {}\r\n\r\n{}", path, token, body.len(), body).into_bytes()), output: Vec::new() };
        handle(&mut p).unwrap();
        String::from_utf8_lossy(&p.output).to_string()
    }

    fn phone_get(path: &str, token: &str) -> String {
        let mut p = Pipe { input: std::io::Cursor::new(format!("GET {} HTTP/1.1\r\nHost: x\r\nX-Token: {}\r\n\r\n", path, token).into_bytes()), output: Vec::new() };
        handle(&mut p).unwrap();
        String::from_utf8_lossy(&p.output).to_string()
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
        assert!(phone_request("/voice", &t, "silero:aidar; calc").starts_with("HTTP/1.1 400"), "only the names of the list");
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
