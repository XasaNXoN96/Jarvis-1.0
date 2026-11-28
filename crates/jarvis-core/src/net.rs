//! Own network layer: Jarvis reaches the internet by itself, without a system VPN.
//!
//! Three modes (setting `net_mode`):
//! * `auto` (default): the system settings; when the key hosts are unreachable directly, a proxy is
//!   tried (the saved one, or one found on this computer) and used until the direct way works again.
//! * `proxy`: always through the proxy of the owner (`net_proxy`).
//! * `direct`: ignore every system proxy.
//!
//! Local addresses (the home network, this computer) never go through a proxy.
//! Honest limit: a proxy only helps when it leads out of the blocked network, so a server
//! outside the block is still needed (the owner's own, or a program on this computer that has one).

use parking_lot::RwLock;
use std::time::Duration;

/// Addresses that must never be proxied.
pub const NO_PROXY: &str = "localhost,127.0.0.1,::1,192.168.0.0/16,10.0.0.0/8,172.16.0.0/12,169.254.0.0/16,.local";

/// Hosts Jarvis depends on, checked by `check()` and by the auto mode.
pub const KEY_HOSTS: &[(&str, &str)] = &[
    ("ИИ OpenAI", "https://api.openai.com/"),
    ("ИИ Anthropic", "https://api.anthropic.com/"),
    ("ИИ Google", "https://generativelanguage.googleapis.com/"),
    ("Telegram", "https://api.telegram.org/"),
    ("Голоса и модели", "https://huggingface.co/"),
    ("Обновления", "https://github.com/"),
    ("Погода", "https://api.open-meteo.com/"),
    ("Яндекс", "https://yandex.ru/"),
];

/// Proxies that popular programs open on this computer: (port, who).
pub const LOCAL_PORTS: &[(u16, &str)] = &[
    (10808, "v2rayN / Xray (SOCKS)"),
    (10809, "v2rayN / Xray (HTTP)"),
    (7890, "Clash (смешанный)"),
    (7891, "Clash (SOCKS)"),
    (7897, "Clash Verge"),
    (12334, "Hiddify (смешанный)"),
    (12335, "Hiddify (HTTP)"),
    (2080, "Nekoray / sing-box"),
    (2081, "Nekoray (HTTP)"),
    (1080, "SOCKS5 (по умолчанию)"),
    (1081, "SOCKS5"),
    (8080, "HTTP-прокси"),
    (8118, "Privoxy"),
    (9150, "Tor Browser"),
    (9050, "Tor"),
];

static AUTO_PROXY: RwLock<Option<String>> = RwLock::new(None);

/// Brings the owner's text to a proxy URL: `host:port`, `http://…`, `https://…`, `socks5://…`,
/// `socks5h://…`, with optional `user:pass@`. Anything else: None.
pub fn normalize_proxy(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.len() > 300 || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return None;
    }
    let (scheme, rest) = match s.split_once("://") {
        Some((sc, r)) => (sc.to_ascii_lowercase(), r),
        None => ("http".to_string(), s),
    };
    if !matches!(scheme.as_str(), "http" | "https" | "socks5" | "socks5h") {
        return None;
    }
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() || rest.contains('/') || rest.contains('?') || rest.contains('#') {
        return None;
    }
    let hostport = rest.rsplit_once('@').map(|(_, h)| h).unwrap_or(rest);
    let (host, port) = hostport.rsplit_once(':')?;
    port.parse::<u16>().ok().filter(|p| *p > 0)?;
    let host = host.trim_matches(|c| c == '[' || c == ']');
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_')) {
        return None;
    }
    Some(format!("{}://{}", scheme, rest))
}

/// The proxy address without the password, for the screen and the log.
pub fn mask(url: &str) -> String {
    match url.split_once("://") {
        Some((sc, rest)) => match rest.rsplit_once('@') {
            Some((_, host)) => format!("{}://***@{}", sc, host),
            None => url.to_string(),
        },
        None => url.to_string(),
    }
}

/// Proxy chosen by the settings right now (None: no proxy).
pub fn active_proxy() -> Option<String> {
    let (mode, saved) = crate::uistate::net_config();
    let saved = normalize_proxy(&saved);
    match mode.as_str() {
        "proxy" => saved,
        "direct" => None,
        _ => AUTO_PROXY.read().clone(),
    }
}

/// Changes when the way out changes: cached clients are rebuilt when it differs.
pub fn route_key() -> String {
    let (mode, _) = crate::uistate::net_config();
    format!("{}|{}", mode, active_proxy().unwrap_or_default())
}

/// The way out changed (settings or the auto mode): the Telegram loop keeps its own connection, so it starts again.
pub fn changed() {
    crate::telegram::start();
}

/// What the screen shows: mode and the way that is in use.
pub fn status() -> (String, String) {
    let (mode, _) = crate::uistate::net_config();
    let way = match (mode.as_str(), active_proxy()) {
        ("direct", _) => "напрямую (системные прокси игнорируются)".to_string(),
        (_, Some(p)) => format!("через прокси {}", mask(&p)),
        _ => "как настроено в системе".to_string(),
    };
    (mode, way)
}

/// SOCKS5 greeting `05 01 00` → `05 00` means a SOCKS5 server; an HTTP reply means an HTTP proxy.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Kind {
    Socks5,
    Http,
}

pub fn sniff(reply: &[u8]) -> Option<Kind> {
    if reply.len() >= 2 && reply[0] == 0x05 && (reply[1] == 0x00 || reply[1] == 0x02) {
        Some(Kind::Socks5)
    } else if reply.starts_with(b"HTTP/") {
        Some(Kind::Http)
    } else {
        None
    }
}

/// Looks for proxies opened on this computer by the programs of the owner (loopback only).
pub fn detect_local() -> Vec<(String, String)> {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    let mut found = Vec::new();
    for (port, who) in LOCAL_PORTS {
        let addr = SocketAddr::from(([127, 0, 0, 1], *port));
        let Ok(mut s) = TcpStream::connect_timeout(&addr, Duration::from_millis(250)) else { continue };
        let _ = s.set_read_timeout(Some(Duration::from_millis(600)));
        let _ = s.set_write_timeout(Some(Duration::from_millis(600)));
        // SOCKS5 first; then a plain HTTP request on a fresh connection
        let mut buf = [0u8; 16];
        let mut kind = None;
        if s.write_all(&[0x05, 0x01, 0x00]).is_ok() {
            if let Ok(n) = s.read(&mut buf) {
                kind = sniff(&buf[..n]).filter(|k| *k == Kind::Socks5);
            }
        }
        if kind.is_none() {
            if let Ok(mut s2) = TcpStream::connect_timeout(&addr, Duration::from_millis(250)) {
                let _ = s2.set_read_timeout(Some(Duration::from_millis(600)));
                let _ = s2.set_write_timeout(Some(Duration::from_millis(600)));
                if s2.write_all(b"GET http://127.0.0.1/ HTTP/1.0\r\n\r\n").is_ok() {
                    if let Ok(n) = s2.read(&mut buf) {
                        kind = sniff(&buf[..n]).filter(|k| *k == Kind::Http);
                    }
                }
            }
        }
        if let Some(k) = kind {
            let url = format!("{}://127.0.0.1:{}", if k == Kind::Socks5 { "socks5h" } else { "http" }, port);
            found.push((url, (*who).to_string()));
        }
    }
    found
}

/// A client builder that follows the settings. Replaces `reqwest::blocking::Client::builder()`
/// everywhere Jarvis goes to the internet (not for the home network).
#[cfg(feature = "lua")]
pub fn builder() -> reqwest::blocking::ClientBuilder {
    let b = reqwest::blocking::Client::builder();
    let (mode, _) = crate::uistate::net_config();
    if mode == "direct" {
        return b.no_proxy();
    }
    match active_proxy().and_then(|p| reqwest::Proxy::all(p).ok()) {
        Some(p) => b.proxy(p.no_proxy(reqwest::NoProxy::from_string(NO_PROXY))),
        None => b,
    }
}

#[cfg(feature = "lua")]
fn probe_with(proxy: Option<&str>, url: &str) -> bool {
    let mut b = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(5)).timeout(Duration::from_secs(8)).no_proxy();
    if let Some(p) = proxy.and_then(|p| reqwest::Proxy::all(p).ok()) {
        b = b.proxy(p);
    }
    // any HTTP answer (even 401/403/404) means the host is reachable; only a failed connection counts as blocked
    b.build().ok().and_then(|c| c.get(url).send().ok()).is_some()
}

/// One row of the network check: (name, reachable directly, reachable through the proxy; None: no proxy).
#[cfg(feature = "lua")]
pub fn check() -> Vec<(String, bool, Option<bool>)> {
    let proxy = candidate();
    let jobs: Vec<_> = KEY_HOSTS
        .iter()
        .map(|(name, url)| {
            let (name, url, proxy) = (name.to_string(), url.to_string(), proxy.clone());
            std::thread::spawn(move || {
                let d = probe_with(None, &url);
                let p = proxy.as_deref().map(|p| probe_with(Some(p), &url));
                (name, d, p)
            })
        })
        .collect();
    jobs.into_iter().filter_map(|j| j.join().ok()).collect()
}

/// The proxy to try: the saved one, else the first found on this computer.
#[cfg(feature = "lua")]
fn candidate() -> Option<String> {
    let (_, saved) = crate::uistate::net_config();
    normalize_proxy(&saved).or_else(|| detect_local().into_iter().next().map(|(u, _)| u))
}

#[cfg(feature = "lua")]
fn auto_round() {
    let (mode, _) = crate::uistate::net_config();
    if mode != "auto" {
        *AUTO_PROXY.write() = None;
        return;
    }
    let rows = check_direct();
    let blocked = rows.iter().filter(|ok| !**ok).count();
    // blocked: at least half of the key hosts do not answer directly
    if blocked * 2 < rows.len() {
        if AUTO_PROXY.write().take().is_some() {
            log::info!("net: the direct way works again, the proxy is off");
            changed();
        }
        return;
    }
    if let Some(p) = candidate() {
        let ok = KEY_HOSTS.iter().take(4).filter(|(_, u)| probe_with(Some(&p), u)).count();
        if ok >= 2 {
            log::info!("net: the direct way is blocked, using the proxy {}", mask(&p));
            *AUTO_PROXY.write() = Some(p);
            changed();
        }
    }
}

#[cfg(feature = "lua")]
fn check_direct() -> Vec<bool> {
    let jobs: Vec<_> = KEY_HOSTS.iter().map(|(_, u)| { let u = u.to_string(); std::thread::spawn(move || probe_with(None, &u)) }).collect();
    jobs.into_iter().map(|j| j.join().unwrap_or(false)).collect()
}

/// Background watcher of the auto mode: first look after a minute, then every 10 minutes.
#[cfg(feature = "lua")]
pub fn start_watcher() {
    let _ = std::thread::Builder::new().name("net-watch".into()).spawn(|| {
        std::thread::sleep(Duration::from_secs(60));
        loop {
            auto_round();
            std::thread::sleep(Duration::from_secs(600));
        }
    });
}

#[cfg(not(feature = "lua"))]
pub fn start_watcher() {}

#[cfg(test)]
mod tests;
