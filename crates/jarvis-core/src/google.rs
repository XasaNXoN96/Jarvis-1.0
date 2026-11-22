//! Google for the morning briefing: the events of the calendar of the next 24 hours and the unread mail (the count and the last
//! three subjects). READ ONLY (the scopes calendar.readonly and gmail.readonly), OPTIONAL and off until the owner connects it.
//!
//! The owner makes his own OAuth client in the Google Cloud console (type «Desktop app») and gives Jarvis its id and secret in
//! Settings → Connections: nothing of ours stands between his account and his computer. The sign-in is the standard flow of an
//! installed program with PKCE: the browser opens the page of Google, Google sends the code back to a one-shot listener on
//! 127.0.0.1 (a random port, only the first request with the right `state`), the code is exchanged for a refresh token, and the
//! token is kept encrypted like the other keys. Subjects of mails and titles of events are FOREIGN text (anyone can send a mail
//! or an invitation): they are given to the model as data only, and one that holds an order to it is dropped (shield.rs).
//!
//! The pure part (the address of the sign-in, PKCE, the parsing of the answers) is tested; the network part needs the owner's
//! client and account and cannot be tested without them.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

pub const SCOPES: &str = "https://www.googleapis.com/auth/calendar.readonly https://www.googleapis.com/auth/gmail.readonly";
const AUTH: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN: &str = "https://oauth2.googleapis.com/token";
const CALENDAR: &str = "https://www.googleapis.com/calendar/v3/calendars/primary/events";
const GMAIL: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const WAIT_SECS: u64 = 180;

// ---------------------------------------------------------------------------------------------
// the pure part
// ---------------------------------------------------------------------------------------------

// base64 of the URL alphabet, without «=» (the form that PKCE wants)
pub fn b64url(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 2);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        if chunk.len() > 1 {
            out.push(T[(n >> 6) as usize & 63] as char);
        }
        if chunk.len() > 2 {
            out.push(T[n as usize & 63] as char);
        }
    }
    out
}

pub fn pkce_challenge(verifier: &str) -> String {
    b64url(&Sha256::digest(verifier.as_bytes()))
}

// a random string of the characters that a verifier and a state may have
pub fn random_string(n: usize) -> String {
    use rand::Rng;
    const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let mut rng = rand::thread_rng();
    (0..n).map(|_| SET[rng.gen_range(0..SET.len())] as char).collect()
}

pub fn pct(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect()
}

pub fn pct_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => {
                match std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

pub fn auth_url(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    format!(
        "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&code_challenge={}&code_challenge_method=S256&access_type=offline&prompt=consent&state={}",
        AUTH,
        pct(client_id),
        pct(redirect_uri),
        pct(SCOPES),
        pct(challenge),
        pct(state)
    )
}

// the first line of the request that the browser sends back: «GET /?code=...&state=... HTTP/1.1» -> the code
pub fn parse_callback(request: &str, state: &str) -> Result<String, String> {
    let line = request.lines().next().unwrap_or("");
    let mut parts = line.split_whitespace();
    if parts.next() != Some("GET") {
        return Err("не тот запрос".into());
    }
    let target = parts.next().ok_or("пустой запрос")?;
    let query = target.split_once('?').map(|(_, q)| q).ok_or("в запросе нет ответа Google")?;
    let (mut code, mut got_state, mut error) = (None, None, None);
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            match k {
                "code" => code = Some(pct_decode(v)),
                "state" => got_state = Some(pct_decode(v)),
                "error" => error = Some(pct_decode(v)),
                _ => {}
            }
        }
    }
    if got_state.as_deref() != Some(state) {
        return Err("чужой запрос (не совпал state)".into());
    }
    if let Some(e) = error {
        return Err(if e == "access_denied" { "вы отказали в доступе".to_string() } else { format!("Google ответил: {}", e) });
    }
    code.filter(|c| !c.is_empty()).ok_or_else(|| "Google не прислал код".to_string())
}

pub fn form(fields: &[(&str, &str)]) -> String {
    fields.iter().map(|(k, v)| format!("{}={}", pct(k), pct(v))).collect::<Vec<_>>().join("&")
}

#[derive(Debug, PartialEq, Eq)]
pub struct Tokens {
    pub access: String,
    pub expires_in: u64,
    pub refresh: Option<String>,
}

pub fn parse_token_response(body: &str) -> Result<Tokens, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| "ответ Google не разобран".to_string())?;
    if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
        let why = match e {
            "invalid_client" => "Google не узнал клиент: проверьте идентификатор и секрет".to_string(),
            "invalid_grant" => "доступ отозван или истёк: подключите Google заново".to_string(),
            "access_denied" => "вы отказали в доступе".to_string(),
            other => format!("Google ответил: {}", other),
        };
        return Err(why);
    }
    let access = v.get("access_token").and_then(|a| a.as_str()).ok_or("в ответе нет ключа доступа")?.to_string();
    Ok(Tokens { access, expires_in: v.get("expires_in").and_then(|e| e.as_u64()).unwrap_or(3000), refresh: v.get("refresh_token").and_then(|r| r.as_str()).map(|r| r.to_string()) })
}

// «в 10:00 — Встреча» for the events of the calendar answer; all-day events are «весь день — …»; a cancelled one is skipped
pub fn parse_events(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return Vec::new() };
    let mut out = Vec::new();
    for item in v.get("items").and_then(|i| i.as_array()).into_iter().flatten() {
        if item.get("status").and_then(|s| s.as_str()) == Some("cancelled") {
            continue;
        }
        let title: String = item.get("summary").and_then(|s| s.as_str()).unwrap_or("без названия").split_whitespace().collect::<Vec<_>>().join(" ");
        let when = match item.pointer("/start/dateTime").and_then(|d| d.as_str()) {
            // 2026-11-22T10:00:00+05:00 -> 10:00
            Some(dt) if dt.len() >= 16 && dt.is_char_boundary(11) && dt.is_char_boundary(16) => format!("в {}", &dt[11..16]),
            _ => "весь день".to_string(),
        };
        out.push(format!("{} — {}", when, title.chars().take(100).collect::<String>()));
    }
    out
}

pub fn parse_unread(body: &str) -> Option<u64> {
    serde_json::from_str::<serde_json::Value>(body).ok()?.get("messagesUnread")?.as_u64()
}

pub fn parse_message_ids(body: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(body) else { return Vec::new() };
    v.get("messages").and_then(|m| m.as_array()).into_iter().flatten().filter_map(|m| m.get("id").and_then(|i| i.as_str())).filter(|i| i.chars().all(|c| c.is_ascii_alphanumeric())).map(|i| i.to_string()).collect()
}

// «Иван Петров <ivan@x.ru>» -> «Иван Петров»; a bare address stays as it is
pub fn sender_name(from: &str) -> String {
    let name = from.split('<').next().unwrap_or(from).trim().trim_matches('"').trim();
    if name.is_empty() {
        from.trim().trim_matches(|c| c == '<' || c == '>').to_string()
    } else {
        name.to_string()
    }
}

// «от Иван: тема» out of a message in the metadata format
pub fn parse_message_line(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let headers = v.pointer("/payload/headers")?.as_array()?;
    let get = |name: &str| headers.iter().find(|h| h.get("name").and_then(|n| n.as_str()).map(|n| n.eq_ignore_ascii_case(name)).unwrap_or(false)).and_then(|h| h.get("value")?.as_str());
    let subject: String = get("Subject").unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" ");
    let from = sender_name(get("From").unwrap_or(""));
    if subject.is_empty() && from.is_empty() {
        return None;
    }
    Some(format!("от {}: {}", from.chars().take(40).collect::<String>(), if subject.is_empty() { "без темы".to_string() } else { subject.chars().take(100).collect() }))
}

// ---------------------------------------------------------------------------------------------
// the network part
// ---------------------------------------------------------------------------------------------

static STATUS: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static ACCESS: Lazy<Mutex<Option<(String, std::time::Instant)>>> = Lazy::new(|| Mutex::new(None));

pub fn status_text() -> String {
    STATUS.lock().clone()
}

fn set_status(s: &str) {
    *STATUS.lock() = s.to_string();
}

pub fn connected() -> bool {
    !crate::uistate::google_refresh().is_empty()
}

#[cfg(feature = "lua")]
fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(8)).timeout(std::time::Duration::from_secs(20)).build().map_err(|e| e.to_string())
}

#[cfg(feature = "lua")]
fn post_form(url: &str, body: String) -> Result<String, String> {
    let resp = client()?.post(url).header("Content-Type", "application/x-www-form-urlencoded").body(body).send().map_err(|_| "нет связи с сервером Google".to_string())?;
    // the answer of an error also carries a JSON that says what is wrong
    resp.text().map_err(|_| "ответ Google оборвался".to_string())
}

#[cfg(feature = "lua")]
fn get_json(url: &str, token: &str) -> Result<String, String> {
    let resp = client()?.get(url).bearer_auth(token).send().map_err(|_| "нет связи с сервером Google".to_string())?;
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return Err("Google отказал в доступе: подключите Google заново".into());
    }
    if !(200..300).contains(&status) {
        return Err(format!("ответ сервера Google {}", status));
    }
    resp.text().map_err(|_| "ответ Google оборвался".to_string())
}

#[cfg(not(feature = "lua"))]
fn post_form(_u: &str, _b: String) -> Result<String, String> {
    Err("в этой сборке нет сетевого модуля".into())
}

#[cfg(not(feature = "lua"))]
fn get_json(_u: &str, _t: &str) -> Result<String, String> {
    Err("в этой сборке нет сетевого модуля".into())
}

// The sign-in: returns the address that the page opens in the browser. The listener waits for Google in the background.
pub fn connect_start() -> Result<String, String> {
    let (id, secret) = crate::uistate::google_client();
    if id.is_empty() || secret.is_empty() {
        return Err("сначала сохраните идентификатор и секрет клиента Google".into());
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| format!("не удалось открыть порт для ответа Google: {}", e))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let redirect = format!("http://127.0.0.1:{}/", port);
    let (verifier, state) = (random_string(64), random_string(32));
    let url = auth_url(&id, &redirect, &pkce_challenge(&verifier), &state);
    set_status("Жду вашего согласия в браузере…");
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(WAIT_SECS);
        let reply = |mut s: std::net::TcpStream, code: &str, text: &str| {
            let body = format!("<!doctype html><meta charset=utf-8><title>Джарвис</title><body style=\"font-family:sans-serif;padding:40px\"><h2>{}</h2>", text);
            let _ = s.write_all(format!("HTTP/1.1 {}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", code, body.len(), body).as_bytes());
        };
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_nonblocking(false);
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                    let mut buf = [0u8; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    let request = String::from_utf8_lossy(&buf[..n]).to_string();
                    match parse_callback(&request, &state) {
                        Ok(code) => {
                            let body = form(&[("client_id", &id), ("client_secret", &secret), ("code", &code), ("code_verifier", &verifier), ("redirect_uri", &redirect), ("grant_type", "authorization_code")]);
                            match post_form(TOKEN, body).and_then(|b| parse_token_response(&b)) {
                                Ok(t) => match t.refresh {
                                    Some(r) => {
                                        crate::uistate::set_google_refresh(&r);
                                        *ACCESS.lock() = Some((t.access, std::time::Instant::now() + std::time::Duration::from_secs(t.expires_in.saturating_sub(60))));
                                        set_status("Google подключён");
                                        reply(stream, "200 OK", "Готово: Джарвис подключён к Google. Окно можно закрыть.");
                                    }
                                    None => {
                                        set_status("Google не выдал постоянный доступ: подключите заново и дайте все разрешения");
                                        reply(stream, "200 OK", "Google не выдал постоянный доступ. Вернитесь в Джарвис.");
                                    }
                                },
                                Err(why) => {
                                    set_status(&why);
                                    reply(stream, "200 OK", &format!("Не получилось: {}", why));
                                }
                            }
                            return;
                        }
                        // a stray request (favicon, a scanner): refused, the wait goes on
                        Err(why) if why.starts_with("чужой") || why.starts_with("не тот") || why.starts_with("в запросе") || why.starts_with("пустой") => reply(stream, "400 Bad Request", "Не тот запрос"),
                        Err(why) => {
                            set_status(&why);
                            reply(stream, "200 OK", &why);
                            return;
                        }
                    }
                }
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(150)),
            }
        }
        set_status("Время на согласие вышло: нажмите «Подключить Google» ещё раз");
    });
    Ok(url)
}

pub fn disconnect() {
    crate::uistate::set_google_refresh("");
    *ACCESS.lock() = None;
    set_status("Google отключён");
}

fn access_token() -> Result<String, String> {
    if let Some((t, until)) = ACCESS.lock().clone() {
        if std::time::Instant::now() < until {
            return Ok(t);
        }
    }
    let refresh = crate::uistate::google_refresh();
    if refresh.is_empty() {
        return Err("Google не подключён".into());
    }
    let (id, secret) = crate::uistate::google_client();
    let body = form(&[("client_id", &id), ("client_secret", &secret), ("refresh_token", &refresh), ("grant_type", "refresh_token")]);
    let t = parse_token_response(&post_form(TOKEN, body)?)?;
    *ACCESS.lock() = Some((t.access.clone(), std::time::Instant::now() + std::time::Duration::from_secs(t.expires_in.saturating_sub(60))));
    Ok(t.access)
}

// the events of the next 24 hours
pub fn calendar_next_day() -> Result<Vec<String>, String> {
    let token = access_token()?;
    let now = chrono::Local::now();
    let url = format!("{}?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime&maxResults=8", CALENDAR, pct(&now.to_rfc3339()), pct(&(now + chrono::Duration::hours(24)).to_rfc3339()));
    Ok(parse_events(&get_json(&url, &token)?))
}

// (the number of unread mails of the inbox, the last three of them in the form «от Иван: тема»)
pub fn mail_summary() -> Result<(u64, Vec<String>), String> {
    let token = access_token()?;
    let unread = parse_unread(&get_json(&format!("{}/labels/INBOX", GMAIL), &token)?).unwrap_or(0);
    let mut lines = Vec::new();
    if unread > 0 {
        let ids = parse_message_ids(&get_json(&format!("{}/messages?q=is%3Aunread&labelIds=INBOX&maxResults=3", GMAIL), &token)?);
        for id in ids.iter().take(3) {
            let url = format!("{}/messages/{}?format=metadata&metadataHeaders=Subject&metadataHeaders=From", GMAIL, id);
            if let Some(line) = get_json(&url, &token).ok().and_then(|b| parse_message_line(&b)) {
                lines.push(line);
            }
        }
    }
    Ok((unread, lines))
}

#[cfg(test)]
mod tests;
