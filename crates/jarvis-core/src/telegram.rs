// Control of the computer from Telegram: the user's own bot (made with @BotFather) sends the phrases the user
// writes to it into the SAME queue as the text typed in the window and the phone remote (remote::push_text), and
// sends back what Jarvis answered. No second command engine, no incoming port: Jarvis only asks api.telegram.org for
// new messages (long polling over HTTPS) and sends the answers.
//
// Safety:
// - off by default; the user pastes the token of the bot, it is stored encrypted (DPAPI) and never shown again;
// - the bot obeys ONE Telegram account: the first one that sends the pairing code shown in the window (6 digits,
//   10 minutes); messages of anybody else, and of groups, are ignored;
// - at most 20 phrases a minute, 300 characters each (as the phone remote);
// - "выключи компьютер" and the other dangerous commands ask "да / нет" there as everywhere.
// What is written to the bot passes through the servers of Telegram: the window says so before it is switched on.

use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const MAX_TEXT: usize = 300;
const PER_MINUTE: usize = 20;
const CODE_TTL: Duration = Duration::from_secs(600);

#[derive(Clone, Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(default)]
pub struct Config {
    pub enabled: bool,
    // sealed on disk (secret::seal), plain in memory
    pub token: String,
    pub owner_id: i64,
    pub owner_name: String,
    pub bot_name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Incoming {
    pub update_id: i64,
    pub from_id: i64,
    pub chat_id: i64,
    pub private: bool,
    pub name: String,
    pub text: String,
}

#[derive(Debug, PartialEq)]
pub enum Decision {
    Ignore,
    // the pairing code was right: this account becomes the owner
    Pair,
    // a wrong code while one is waiting: counted, five of them cancel the code (no guessing of 6 digits)
    WrongCode,
    // the owner asked for help / said hello
    Help,
    Run(String),
    TooLong,
}

static CONFIG: Lazy<Mutex<Config>> = Lazy::new(|| Mutex::new(load()));
static CODE: Lazy<Mutex<Option<(String, Instant)>>> = Lazy::new(|| Mutex::new(None));
static WRONG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
const MAX_WRONG: u32 = 5;
static STATUS: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
// a new generation stops the loop of the old one (switched off, another token)
static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
// the address of the Bot API (a local imitation in the tests)
static BASE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new("https://api.telegram.org".to_string()));

fn file() -> Option<std::path::PathBuf> {
    crate::APP_CONFIG_DIR.get().map(|d| d.join("telegram.json"))
}

fn load() -> Config {
    let Some(p) = file() else { return Config::default() };
    let (mut c, problem): (Config, Option<String>) = crate::uistate::parse_or_keep(&p, "настройки Telegram (telegram.json)");
    if let Some(pr) = problem {
        crate::uistate::add_load_problem(&pr);
    }
    c.token = crate::secret::open(&c.token);
    c
}

fn save(c: &Config) {
    let Some(p) = file() else { return };
    let mut v = serde_json::to_value(c).unwrap_or(Value::Null);
    v["token"] = Value::String(crate::secret::seal(&c.token));
    if let Ok(text) = serde_json::to_string_pretty(&v) {
        let _ = crate::write_atomic(&p, text);
    }
}

// "123456789:AA…" — what @BotFather gives; nothing else is sent to Telegram as a token
pub fn token_looks_valid(t: &str) -> bool {
    let Some((id, secret)) = t.split_once(':') else { return false };
    (5..=12).contains(&id.len()) && id.chars().all(|c| c.is_ascii_digit()) && secret.len() >= 30 && secret.len() <= 64
        && secret.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// the messages of a getUpdates answer (only text messages)
pub fn parse_updates(v: &Value) -> Vec<Incoming> {
    let mut out = Vec::new();
    for u in v["result"].as_array().into_iter().flatten() {
        let Some(update_id) = u["update_id"].as_i64() else { continue };
        let m = &u["message"];
        let (Some(from_id), Some(chat_id), Some(text)) = (m["from"]["id"].as_i64(), m["chat"]["id"].as_i64(), m["text"].as_str()) else {
            // still counted as read: the next request starts after it
            out.push(Incoming { update_id, from_id: 0, chat_id: 0, private: false, name: String::new(), text: String::new() });
            continue;
        };
        let name = m["from"]["first_name"].as_str().or(m["from"]["username"].as_str()).unwrap_or("").chars().take(60).collect();
        out.push(Incoming { update_id, from_id, chat_id, private: m["chat"]["type"] == "private", name, text: text.trim().to_string() });
    }
    out
}

// what to do with a message: only the owner in a private chat is obeyed; before pairing only the right code counts
pub fn decide(owner_id: i64, code: Option<&str>, m: &Incoming) -> Decision {
    if !m.private || m.from_id == 0 || m.text.is_empty() {
        return Decision::Ignore;
    }
    if owner_id == 0 {
        let given = m.text.trim_start_matches("/start").trim();
        return match code {
            Some(c) if !c.is_empty() && given == c => Decision::Pair,
            Some(_) if given.len() == 6 && given.chars().all(|ch| ch.is_ascii_digit()) => Decision::WrongCode,
            _ => Decision::Ignore,
        };
    }
    if m.from_id != owner_id {
        return Decision::Ignore;
    }
    let t = m.text.trim();
    if t == "/start" || t == "/help" || t.starts_with("/start ") {
        return Decision::Help;
    }
    if t.chars().count() > MAX_TEXT {
        return Decision::TooLong;
    }
    Decision::Run(t.trim_start_matches('/').to_string())
}

// at most PER_MINUTE phrases in the last minute
pub fn rate_ok(times: &mut Vec<Instant>, now: Instant) -> bool {
    times.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
    if times.len() >= PER_MINUTE {
        return false;
    }
    times.push(now);
    true
}

pub const HELP: &str = "JARVIS на связи. Пишите команды как в окне Джарвиса, например:\n• поставь таймер на 1 минуту\n• сверни все окна\n• открой ютуб\n• громкость 30\n• выключи компьютер (спросит «да / нет»)\n• отмени выключение\n• добавь задачу купить хлеб";

fn set_status(s: &str) {
    *STATUS.lock() = s.to_string();
}

pub fn json() -> Value {
    let c = CONFIG.lock().clone();
    let code = CODE.lock().as_ref().filter(|(_, at)| at.elapsed() < CODE_TTL).map(|(c, at)| json!({ "code": c, "left": (CODE_TTL.saturating_sub(at.elapsed())).as_secs() }));
    json!({
        "enabled": c.enabled,
        "token_saved": !c.token.is_empty(),
        "bot": c.bot_name,
        "paired": c.owner_id != 0,
        "owner": c.owner_name,
        "pairing": code,
        "status": STATUS.lock().clone(),
        "available": cfg!(feature = "lua"),
    })
}

fn new_code() -> String {
    use rand::Rng;
    format!("{:06}", rand::thread_rng().gen_range(0..1_000_000))
}

// the actions of the window
pub fn action(body: &Value) -> Value {
    let op = body["op"].as_str().unwrap_or("");
    let res: Result<String, String> = match op {
        "token" => {
            let t = body["token"].as_str().unwrap_or("").trim().to_string();
            if !token_looks_valid(&t) {
                Err("Это не похоже на токен бота. Он выглядит так: 123456789:AA… — его выдаёт @BotFather".into())
            } else {
                let mut c = CONFIG.lock();
                // another bot: the old owner does not carry over
                if c.token != t {
                    c.owner_id = 0;
                    c.owner_name.clear();
                    c.bot_name.clear();
                }
                c.token = t;
                save(&c);
                Ok("Токен сохранён (в зашифрованном виде)".into())
            }
        }
        "enable" => {
            let on = body["value"].as_bool().unwrap_or(false);
            let mut c = CONFIG.lock();
            if on && c.token.is_empty() {
                Err("Сначала вставьте токен бота".into())
            } else {
                c.enabled = on;
                save(&c);
                drop(c);
                if on {
                    start();
                    Ok("Telegram-бот включён".into())
                } else {
                    stop();
                    Ok("Telegram-бот выключен".into())
                }
            }
        }
        "pair" => {
            let code = new_code();
            *CODE.lock() = Some((code.clone(), Instant::now()));
            WRONG.store(0, std::sync::atomic::Ordering::SeqCst);
            Ok(format!("Отправьте боту код {} в течение 10 минут", code))
        }
        "unpair" => {
            let mut c = CONFIG.lock();
            c.owner_id = 0;
            c.owner_name.clear();
            save(&c);
            Ok("Телефон отвязан: бот никого не слушается, пока вы не привяжете его снова".into())
        }
        "forget" => {
            stop();
            let mut c = CONFIG.lock();
            *c = Config::default();
            save(&c);
            Ok("Токен и привязка удалены с этого компьютера".into())
        }
        _ => Err("unknown op".into()),
    };
    match res {
        Ok(m) => json!({ "ok": true, "message": m }),
        Err(e) => json!({ "ok": false, "message": e }),
    }
}

pub fn stop() {
    GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    set_status("выключен");
}

// at start (when it was on) and when switched on
pub fn start() {
    let c = CONFIG.lock().clone();
    if !c.enabled || c.token.is_empty() {
        return;
    }
    let gen = GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    set_status("подключаюсь…");
    #[cfg(feature = "lua")]
    std::thread::spawn(move || net::run(gen));
    #[cfg(not(feature = "lua"))]
    {
        let _ = gen;
        set_status("недоступно в этой сборке");
    }
}

fn alive(gen: u64) -> bool {
    GEN.load(std::sync::atomic::Ordering::SeqCst) == gen
}

// the answer of Jarvis to the phrase that was just sent: the replies stored with its request id
// (the same conversation as in the window), when nothing new has come for a moment
fn collect_answer(chat_before: Option<(String, usize)>, max: Duration) -> String {
    let store = crate::ai::store();
    let start = Instant::now();
    let mut last_len = 0usize;
    let mut last_change = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(400));
        let Some(id) = store.active() else { continue };
        let Some(chat) = store.load_with_live(&id) else { continue };
        let base = match &chat_before {
            Some((cid, n)) if *cid == chat.id => *n,
            _ => 0,
        };
        let msgs = &chat.messages;
        let replies: Vec<&crate::ai::ChatMsg> = match msgs.iter().enumerate().skip(base).find(|(_, m)| m.role == "user") {
            Some((i, q)) => msgs[i + 1..].iter().filter(|m| m.role != "user" && (q.rid.is_empty() || m.rid == q.rid)).collect(),
            None => Vec::new(),
        };
        let len: usize = replies.iter().map(|m| m.content.len()).sum::<usize>() + replies.len();
        if len != last_len {
            last_len = len;
            last_change = Instant::now();
        }
        let streaming = replies.iter().any(|m| m.streaming) || crate::ai::is_busy(&chat.id);
        if !replies.is_empty() && !streaming && last_change.elapsed() > Duration::from_millis(1800) {
            let text: Vec<String> = replies.iter().map(|m| if m.role == "error" { format!("⚠ {}", m.content) } else { m.content.clone() }).collect();
            return text.join("\n\n");
        }
        if start.elapsed() > max {
            return if replies.is_empty() { "Команда передана Джарвису, ответа пока нет.".into() } else { replies.iter().map(|m| m.content.clone()).collect::<Vec<_>>().join("\n\n") };
        }
    }
}

fn chat_snapshot() -> Option<(String, usize)> {
    let store = crate::ai::store();
    let id = store.active()?;
    store.load(&id).map(|c| (c.id, c.messages.len()))
}

#[cfg(feature = "lua")]
mod net {
    use super::*;

    fn client() -> Option<reqwest::blocking::Client> {
        reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(40)).build().ok()
    }

    fn call(c: &reqwest::blocking::Client, token: &str, method: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}/bot{}/{}", BASE.lock(), token, method);
        // (the address holds the token: errors are reported without it)
        let resp = c.post(url).json(body).send().map_err(|e| if e.is_timeout() { "нет ответа от Telegram".to_string() } else if e.is_connect() { "нет связи с api.telegram.org".to_string() } else { "ошибка сети".to_string() })?;
        let status = resp.status().as_u16();
        let v: Value = resp.json().map_err(|_| "непонятный ответ Telegram".to_string())?;
        if status == 401 || status == 404 {
            return Err("Telegram не принял токен: проверьте его у @BotFather".into());
        }
        if status == 409 {
            return Err("этого бота уже опрашивает другая программа (webhook или другой компьютер)".into());
        }
        if v["ok"] != true {
            return Err(format!("Telegram ответил ошибкой {}", status));
        }
        Ok(v)
    }

    fn send(c: &reqwest::blocking::Client, token: &str, chat: i64, text: &str) {
        let text: String = text.chars().take(3800).collect();
        let _ = call(c, token, "sendMessage", &json!({ "chat_id": chat, "text": text, "disable_web_page_preview": true }));
    }

    pub fn run(gen: u64) {
        let Some(c) = client() else {
            set_status("не удалось запустить сеть");
            return;
        };
        let token = CONFIG.lock().token.clone();
        let mut offset: i64 = 0;
        let mut backoff = 5u64;
        let mut times: Vec<Instant> = Vec::new();
        match call(&c, &token, "getMe", &json!({})) {
            Ok(v) => {
                let name = v["result"]["username"].as_str().unwrap_or("").to_string();
                let mut cfg = CONFIG.lock();
                if cfg.bot_name != name {
                    cfg.bot_name = name;
                    save(&cfg);
                }
            }
            Err(e) => {
                set_status(&e);
                if e.contains("токен") {
                    return;
                }
            }
        }
        while alive(gen) {
            let r = call(&c, &token, "getUpdates", &json!({ "offset": offset, "timeout": 25, "allowed_updates": ["message"] }));
            if !alive(gen) {
                break;
            }
            let v = match r {
                Ok(v) => {
                    backoff = 5;
                    set_status("работает");
                    v
                }
                Err(e) => {
                    set_status(&format!("{} — повтор через {} с", e, backoff));
                    if e.contains("токен") {
                        break;
                    }
                    std::thread::sleep(Duration::from_secs(backoff));
                    backoff = (backoff * 2).min(60);
                    continue;
                }
            };
            for m in parse_updates(&v) {
                offset = offset.max(m.update_id + 1);
                let owner = CONFIG.lock().owner_id;
                let code = CODE.lock().as_ref().filter(|(_, at)| at.elapsed() < CODE_TTL).map(|(c, _)| c.clone());
                match decide(owner, code.as_deref(), &m) {
                    Decision::Ignore => {}
                    Decision::WrongCode => {
                        if WRONG.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 >= MAX_WRONG {
                            *CODE.lock() = None;
                            crate::uistate::log_event("error", "Telegram: код привязки сброшен после 5 неверных попыток");
                            set_status("код привязки сброшен: 5 неверных попыток. Получите новый код");
                        }
                    }
                    Decision::Pair => {
                        {
                            let mut cfg = CONFIG.lock();
                            cfg.owner_id = m.from_id;
                            cfg.owner_name = m.name.clone();
                            save(&cfg);
                        }
                        *CODE.lock() = None;
                        crate::uistate::log_event("info", "Telegram: телефон привязан");
                        send(&c, &token, m.chat_id, &format!("Готово: этот аккаунт привязан к JARVIS.\n\n{}", HELP));
                    }
                    Decision::Help => send(&c, &token, m.chat_id, HELP),
                    Decision::TooLong => send(&c, &token, m.chat_id, "Слишком длинное сообщение: не больше 300 символов."),
                    Decision::Run(text) => {
                        if !rate_ok(&mut times, Instant::now()) {
                            send(&c, &token, m.chat_id, "Слишком много команд подряд. Подождите минуту.");
                            continue;
                        }
                        let before = chat_snapshot();
                        if !crate::remote::push_text(&text) {
                            send(&c, &token, m.chat_id, "Джарвис ещё запускается, команды пока не принимаются.");
                            continue;
                        }
                        log::info!("Telegram command: {} chars", text.chars().count());
                        let answer = collect_answer(before, Duration::from_secs(45));
                        send(&c, &token, m.chat_id, &answer);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
pub fn set_for_tests(c: Config) {
    *CONFIG.lock() = c;
}

#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(from: i64, text: &str, private: bool) -> Incoming {
        Incoming { update_id: 1, from_id: from, chat_id: from, private, name: "Хасан".into(), text: text.into() }
    }

    #[test]
    fn only_a_bot_father_token_is_accepted() {
        assert!(token_looks_valid("123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw"));
        assert!(!token_looks_valid("123456789"));
        assert!(!token_looks_valid("abc:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw"));
        assert!(!token_looks_valid("123456789:short"));
        assert!(!token_looks_valid("123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw/../x"));
    }

    #[test]
    fn before_pairing_only_the_right_code_in_a_private_chat_counts() {
        assert_eq!(decide(0, Some("482913"), &msg(77, "/start 482913", true)), Decision::Pair);
        assert_eq!(decide(0, Some("482913"), &msg(77, "482913", true)), Decision::Pair);
        assert_eq!(decide(0, Some("482913"), &msg(77, "111111", true)), Decision::WrongCode);
        assert_eq!(decide(0, Some("482913"), &msg(77, "привет", true)), Decision::Ignore);
        assert_eq!(decide(0, Some("482913"), &msg(77, "482913", false)), Decision::Ignore, "never from a group");
        assert_eq!(decide(0, None, &msg(77, "482913", true)), Decision::Ignore, "no code asked: nobody pairs");
        assert_eq!(decide(0, Some("482913"), &msg(77, "выключи компьютер", true)), Decision::Ignore);
    }

    #[test]
    fn after_pairing_only_the_owner_is_obeyed() {
        assert_eq!(decide(77, None, &msg(77, "открой ютуб", true)), Decision::Run("открой ютуб".into()));
        assert_eq!(decide(77, None, &msg(99, "открой ютуб", true)), Decision::Ignore, "a stranger is ignored");
        assert_eq!(decide(77, None, &msg(77, "открой ютуб", false)), Decision::Ignore, "a group is ignored");
        assert_eq!(decide(77, Some("482913"), &msg(99, "482913", true)), Decision::Ignore, "a code does not take over a paired bot");
        assert_eq!(decide(77, None, &msg(77, "/help", true)), Decision::Help);
        assert_eq!(decide(77, None, &msg(77, &"я".repeat(301), true)), Decision::TooLong);
    }

    #[test]
    fn updates_are_read_and_non_text_ones_are_skipped_but_counted() {
        let v = json!({ "ok": true, "result": [
            { "update_id": 10, "message": { "from": { "id": 77, "first_name": "Хасан" }, "chat": { "id": 77, "type": "private" }, "text": " сверни все окна " } },
            { "update_id": 11, "message": { "from": { "id": 77 }, "chat": { "id": 77, "type": "private" }, "sticker": {} } },
            { "update_id": 12, "message": { "from": { "id": 5 }, "chat": { "id": -100, "type": "group" }, "text": "hi" } }
        ]});
        let u = parse_updates(&v);
        assert_eq!(u.len(), 3);
        assert_eq!(u[0].text, "сверни все окна");
        assert_eq!(u[0].name, "Хасан");
        assert!(u[0].private);
        assert_eq!((u[1].update_id, u[1].text.as_str()), (11, ""));
        assert!(!u[2].private);
        assert!(parse_updates(&json!({ "ok": false })).is_empty());
    }

    #[test]
    fn twenty_phrases_a_minute_at_most() {
        let mut t = Vec::new();
        let now = Instant::now();
        for _ in 0..20 {
            assert!(rate_ok(&mut t, now));
        }
        assert!(!rate_ok(&mut t, now));
        assert!(rate_ok(&mut t, now + Duration::from_secs(61)), "a minute later it is allowed again");
    }

    #[test]
    fn the_window_never_gets_the_token_back() {
        let _g = TEST_LOCK.lock();
        set_for_tests(Config { enabled: false, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, owner_name: "Хасан".into(), bot_name: "my_jarvis_bot".into() });
        let j = json().to_string();
        assert!(!j.contains("AAHdq"), "{}", j);
        assert!(j.contains("\"token_saved\":true") && j.contains("\"paired\":true"));
        let r = action(&json!({ "op": "token", "token": "nonsense" }));
        assert_eq!(r["ok"], false);
        set_for_tests(Config::default());
    }

    // the whole loop against a local imitation of the Bot API: a stranger is ignored, the right code pairs the
    // account, then the owner gets help and an honest answer when the command queue is not connected
    #[cfg(feature = "lua")]
    #[test]
    fn the_loop_pairs_with_the_code_and_answers_only_the_owner() {
        use std::io::{Read, Write};
        let _g = TEST_LOCK.lock();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        *BASE.lock() = format!("http://127.0.0.1:{}", port);
        let sent: std::sync::Arc<Mutex<Vec<Value>>> = Default::default();
        let sent2 = sent.clone();
        std::thread::spawn(move || {
            let mut round = 0;
            for conn in listener.incoming().flatten() {
                let mut conn = conn;
                let mut buf = vec![0u8; 65536];
                let mut got = 0;
                // headers + body (small requests)
                loop {
                    let n = conn.read(&mut buf[got..]).unwrap_or(0);
                    if n == 0 { break; }
                    got += n;
                    let text = String::from_utf8_lossy(&buf[..got]).to_string();
                    if let Some(h) = text.find("\r\n\r\n") {
                        let len = text.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                        if got >= h + 4 + len { break; }
                    }
                }
                let text = String::from_utf8_lossy(&buf[..got]).to_string();
                let first = text.lines().next().unwrap_or("").to_string();
                let body: Value = text.split("\r\n\r\n").nth(1).and_then(|b| serde_json::from_str(b).ok()).unwrap_or(json!({}));
                let answer = if first.contains("/getMe") {
                    json!({ "ok": true, "result": { "username": "test_jarvis_bot" } })
                } else if first.contains("/getUpdates") {
                    round += 1;
                    let pm = |id: i64, from: i64, text: &str| json!({ "update_id": id, "message": { "from": { "id": from, "first_name": "Тест" }, "chat": { "id": from, "type": "private" }, "text": text } });
                    match round {
                        1 => json!({ "ok": true, "result": [pm(1, 99, "000000"), pm(2, 77, "/start 123456")] }),
                        2 => json!({ "ok": true, "result": [pm(3, 99, "выключи компьютер"), pm(4, 77, "/help"), pm(5, 77, "открой ютуб")] }),
                        _ => { std::thread::sleep(Duration::from_millis(300)); json!({ "ok": true, "result": [] }) }
                    }
                } else {
                    sent2.lock().push(body.clone());
                    json!({ "ok": true, "result": {} })
                };
                let a = answer.to_string();
                let _ = conn.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", a.len(), a).as_bytes());
            }
        });
        set_for_tests(Config { enabled: true, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), ..Default::default() });
        *CODE.lock() = Some(("123456".into(), Instant::now()));
        start();
        let t0 = Instant::now();
        while sent.lock().len() < 3 && t0.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(100));
        }
        stop();
        let msgs = sent.lock().clone();
        assert_eq!(CONFIG.lock().owner_id, 77, "the account with the right code is the owner, not the first writer");
        assert_eq!(CONFIG.lock().bot_name, "test_jarvis_bot");
        assert!(msgs.iter().all(|m| m["chat_id"] == 77), "nothing is sent to the stranger: {:?}", msgs);
        assert!(msgs[0]["text"].as_str().unwrap().starts_with("Готово: этот аккаунт привязан"));
        assert!(msgs[1]["text"].as_str().unwrap().starts_with("JARVIS на связи"));
        // (no command queue in the tests: the owner is told honestly instead of a silence)
        assert!(msgs[2]["text"].as_str().unwrap().contains("ещё запускается"), "{:?}", msgs[2]);
        assert!(CODE.lock().is_none(), "the code works once");
        *BASE.lock() = "https://api.telegram.org".into();
        set_for_tests(Config::default());
    }

    #[test]
    fn five_wrong_codes_are_counted_by_the_decision() {
        // (the loop cancels the code after MAX_WRONG of these; six digits are never guessable in five tries)
        let wrong: Vec<Decision> = (0..MAX_WRONG).map(|i| decide(0, Some("482913"), &msg(99, &format!("00000{}", i), true))).collect();
        assert!(wrong.iter().all(|d| *d == Decision::WrongCode));
    }
}
