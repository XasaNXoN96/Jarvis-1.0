// Control of the computer from Telegram: the user's own bot (made with @BotFather) sends the phrases the user
// writes to it into the SAME queue as the text typed in the window and the phone remote (remote::submit with the Telegram door), and
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
    // the bot may send a picture of the screen (off until the user allows it in the window)
    pub allow_screen: bool,
    // voice messages are not recognized when this is set (they are on by default)
    pub block_voice: bool,
    // the answer is also sent as a voice message (off by default; needs the voice of Jarvis, Piper)
    pub voice_reply: bool,
    // a timer, an alarm or a reminder is NOT sent to the bot when this is set (they are sent by default)
    pub no_alerts: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct VoiceMsg {
    pub file_id: String,
    pub duration: u32,
    pub size: u64,
}

// a press of a button under a message
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Callback {
    pub id: String,
    pub data: String,
    pub message_id: i64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Incoming {
    pub update_id: i64,
    pub from_id: i64,
    pub chat_id: i64,
    pub private: bool,
    pub name: String,
    pub text: String,
    // when Telegram got it (unix seconds): messages written while Jarvis was off are not run later
    pub date: i64,
    pub voice: Option<VoiceMsg>,
    pub callback: Option<Callback>,
}

// older than this when Jarvis reads it: skipped (Telegram keeps unread messages for a day)
pub const MAX_AGE_SECS: i64 = 120;

pub fn too_old(date: i64, now: i64) -> bool {
    date > 0 && now - date > MAX_AGE_SECS
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
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
    Menu,
    Status,
    Tasks,
    Screen,
    Run(String),
    TooLong,
    // a press of a button of the owner
    Button(Callback),
    // a voice message of the owner
    Voice(VoiceMsg),
}

static CONFIG: Lazy<Mutex<Config>> = Lazy::new(|| Mutex::new(load()));
static CODE: Lazy<Mutex<Option<(String, Instant)>>> = Lazy::new(|| Mutex::new(None));
// wrong codes per account: five from one account cancel the code (a stranger cannot cancel it by guessing from
// many accounts quickly either: each counts on its own, and six digits are never guessed in five tries)
static WRONG: Lazy<Mutex<std::collections::HashMap<i64, u32>>> = Lazy::new(|| Mutex::new(std::collections::HashMap::new()));
const MAX_WRONG: u32 = 5;
// the last update Telegram was told is read (shared by every loop of this run): a restarted loop never runs it again
static OFFSET: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn reset_code() {
    *CODE.lock() = None;
    WRONG.lock().clear();
}

// counts a wrong code of an account; true when this account has used up its tries
pub fn wrong_code(from: i64) -> bool {
    let mut w = WRONG.lock();
    let n = w.entry(from).or_insert(0);
    *n += 1;
    *n >= MAX_WRONG
}
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

// the messages of a getUpdates answer: text, voice messages and presses of buttons
pub fn parse_updates(v: &Value) -> Vec<Incoming> {
    let mut out = Vec::new();
    for u in v["result"].as_array().into_iter().flatten() {
        let Some(update_id) = u["update_id"].as_i64() else { continue };
        let skipped = Incoming { update_id, ..Default::default() };
        let name_of = |f: &Value| -> String { f["first_name"].as_str().or(f["username"].as_str()).unwrap_or("").chars().take(60).collect() };
        if let Some(cq) = u.get("callback_query") {
            let (Some(from_id), Some(chat_id), Some(id), Some(data)) = (cq["from"]["id"].as_i64(), cq["message"]["chat"]["id"].as_i64(), cq["id"].as_str(), cq["data"].as_str()) else {
                out.push(skipped);
                continue;
            };
            out.push(Incoming {
                update_id,
                from_id,
                chat_id,
                private: cq["message"]["chat"]["type"] == "private",
                name: name_of(&cq["from"]),
                // (no age check for a button: the message under it can be old, the confirmations expire by themselves)
                callback: Some(Callback { id: id.chars().take(64).collect(), data: data.chars().take(64).collect(), message_id: cq["message"]["message_id"].as_i64().unwrap_or(0) }),
                ..Default::default()
            });
            continue;
        }
        let m = &u["message"];
        let (Some(from_id), Some(chat_id)) = (m["from"]["id"].as_i64(), m["chat"]["id"].as_i64()) else {
            // still counted as read: the next request starts after it
            out.push(skipped);
            continue;
        };
        let voice = m.get("voice").and_then(|v| {
            Some(VoiceMsg { file_id: v["file_id"].as_str()?.chars().take(200).collect(), duration: v["duration"].as_u64().unwrap_or(0).min(100_000) as u32, size: v["file_size"].as_u64().unwrap_or(0) })
        });
        let text = m["text"].as_str().unwrap_or("").trim().to_string();
        if voice.is_none() && text.is_empty() {
            out.push(skipped);
            continue;
        }
        out.push(Incoming { update_id, from_id, chat_id, private: m["chat"]["type"] == "private", name: name_of(&m["from"]), text, date: m["date"].as_i64().unwrap_or(0), voice, callback: None });
    }
    out
}

// what to do with a message: only the owner in a private chat is obeyed; before pairing only the right code counts
pub fn decide(owner_id: i64, code: Option<&str>, m: &Incoming) -> Decision {
    if !m.private || m.from_id == 0 {
        return Decision::Ignore;
    }
    // buttons and voice messages: only the paired owner (before pairing only the text with the code counts)
    if let Some(cb) = &m.callback {
        return if owner_id != 0 && m.from_id == owner_id { Decision::Button(cb.clone()) } else { Decision::Ignore };
    }
    if let Some(v) = &m.voice {
        return if owner_id != 0 && m.from_id == owner_id { Decision::Voice(v.clone()) } else { Decision::Ignore };
    }
    if m.text.is_empty() {
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
    if t.starts_with("/start ") {
        return Decision::Help;
    }
    // "/menu", "/menu@my_bot": the commands of the bot itself; any other "/phrase" is a phrase for Jarvis
    if t.starts_with('/') && !t.contains(char::is_whitespace) {
        match t.trim_start_matches('/').split('@').next().unwrap_or("") {
            "start" | "help" => return Decision::Help,
            "menu" => return Decision::Menu,
            "status" => return Decision::Status,
            "tasks" => return Decision::Tasks,
            "screen" => return Decision::Screen,
            _ => {}
        }
    }
    if t.chars().count() > MAX_TEXT {
        return Decision::TooLong;
    }
    let cmd = t.trim_start_matches('/').trim();
    if cmd.is_empty() {
        return Decision::Help;
    }
    Decision::Run(cmd.to_string())
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

pub const HELP: &str = "JARVIS на связи. Нажмите /menu — кнопки для таймера, музыки, звука, окон и питания. Или пишите и говорите команды как в окне Джарвиса:\n• поставь таймер на 1 минуту\n• сверни все окна\n• открой ютуб\n• громкость 30\n• выключи компьютер (спросит подтверждение)\n• добавь задачу купить хлеб\n\n🎤 Можно отправить голосовое сообщение — я распознаю его на компьютере и выполню.\n/menu — меню · /status — состояние · /tasks — задачи · /screen — снимок экрана (если разрешён в Jarvis)";

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
        "allow_screen": c.allow_screen,
        "voice_in": !c.block_voice,
        "voice_in_ready": crate::tgvoice::recognition_available(),
        "voice_reply": c.voice_reply,
        "alerts": !c.no_alerts,
        "voice_reply_ready": crate::tts::can_speak().is_ok() && !crate::uistate::voice_name().starts_with("win:"),
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
                // another bot: the old owner and a waiting code do not carry over
                if c.token != t {
                    reset_code();
                    c.owner_id = 0;
                    c.owner_name.clear();
                    c.bot_name.clear();
                }
                c.token = t;
                save(&c);
                let running = c.enabled;
                drop(c);
                // a running bot switches to the new token at once (the old one is not polled any more)
                if running {
                    start();
                }
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
            WRONG.lock().clear();
            Ok(format!("Отправьте боту код {} в течение 10 минут", code))
        }
        "unpair" => {
            reset_code();
            let mut c = CONFIG.lock();
            c.owner_id = 0;
            c.owner_name.clear();
            save(&c);
            Ok("Телефон отвязан: бот никого не слушается, пока вы не привяжете его снова".into())
        }
        // what the bot may do besides the commands: the screen, voice messages, voice answers
        "options" => {
            let mut c = CONFIG.lock();
            if let Some(v) = body["allow_screen"].as_bool() {
                c.allow_screen = v;
            }
            if let Some(v) = body["voice_in"].as_bool() {
                c.block_voice = !v;
            }
            if let Some(v) = body["voice_reply"].as_bool() {
                c.voice_reply = v;
            }
            if let Some(v) = body["alerts"].as_bool() {
                c.no_alerts = !v;
            }
            save(&c);
            Ok("Сохранено".into())
        }
        "forget" => {
            stop();
            reset_code();
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

// what the conversation looked like just before a phrase was sent: its chat, the request ids of its last messages and
// the time (a count of messages would not do: a long conversation is cut to its last messages when it is saved)
pub struct Snapshot {
    chat: String,
    rids: Vec<String>,
    at: u64,
}

fn chat_snapshot() -> Snapshot {
    let store = crate::ai::store();
    let at = now_secs().max(0) as u64;
    match store.active().and_then(|id| store.load(&id)) {
        Some(c) => Snapshot { rids: c.messages.iter().rev().take(20).map(|m| m.rid.clone()).collect(), chat: c.id, at },
        None => Snapshot { chat: String::new(), rids: Vec::new(), at },
    }
}

// the question of the phrase just sent: the newest user message that was not there before
pub fn new_question<'a>(msgs: &'a [crate::ai::ChatMsg], same_chat: bool, snap_rids: &[String], at: u64) -> Option<usize> {
    msgs.iter().enumerate().rev().find(|(_, m)| m.role == "user" && m.ts + 1 >= at && !(same_chat && !m.rid.is_empty() && snap_rids.contains(&m.rid))).map(|(i, _)| i)
}

// the answer of Jarvis to the phrase that was just sent: the replies stored with its request id
// (the same conversation as in the window), when nothing new has come for a moment
fn collect_answer(snap: Snapshot, max: Duration, gen: u64) -> String {
    let store = crate::ai::store();
    let start = Instant::now();
    let mut last_len = 0usize;
    let mut last_change = Instant::now();
    let mut found: Vec<crate::ai::ChatMsg> = Vec::new();
    while alive(gen) {
        std::thread::sleep(Duration::from_millis(400));
        let Some(id) = store.active() else { continue };
        let Some(chat) = store.load_with_live(&id) else { continue };
        let msgs = &chat.messages;
        found = match new_question(msgs, chat.id == snap.chat, &snap.rids, snap.at) {
            Some(i) => {
                let q = &msgs[i];
                msgs[i + 1..].iter().filter(|m| m.role != "user" && (q.rid.is_empty() || m.rid == q.rid)).cloned().collect()
            }
            None => Vec::new(),
        };
        let len: usize = found.iter().map(|m| m.content.len()).sum::<usize>() + found.len();
        if len != last_len {
            last_len = len;
            last_change = Instant::now();
        }
        let streaming = found.iter().any(|m| m.streaming) || crate::ai::is_busy(&chat.id);
        if !found.is_empty() && !streaming && last_change.elapsed() > Duration::from_millis(1800) {
            break;
        }
        if start.elapsed() > max {
            break;
        }
    }
    if found.is_empty() {
        return "Команда передана Джарвису, ответа пока нет.".into();
    }
    found.iter().map(|m| if m.role == "error" { format!("⚠ {}", m.content) } else { m.content.clone() }).collect::<Vec<_>>().join("\n\n")
}

#[cfg(feature = "lua")]
mod net {
    use super::*;

    pub(super) fn client() -> Option<reqwest::blocking::Client> {
        reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(40)).build().ok()
    }

    pub(super) fn call(c: &reqwest::blocking::Client, token: &str, method: &str, body: &Value) -> Result<Value, String> {
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

    // (used from another thread: a timer rang, the address of the tunnel is ready)
    pub(super) fn send_plain(c: &reqwest::blocking::Client, token: &str, chat: i64, text: &str) {
        send_kb(c, token, chat, text, &crate::tgmenu::menu_button());
    }

    fn send_kb(c: &reqwest::blocking::Client, token: &str, chat: i64, text: &str, kb: &Value) {
        let text: String = text.chars().take(3800).collect();
        let _ = call(c, token, "sendMessage", &json!({ "chat_id": chat, "text": text, "disable_web_page_preview": true, "reply_markup": kb }));
    }

    // the text of a message under a button is replaced (a page of the menu); when Telegram refuses (too old) a new one is sent
    fn show(c: &reqwest::blocking::Client, token: &str, chat: i64, message_id: i64, text: &str, kb: &Value) {
        if message_id != 0 && call(c, token, "editMessageText", &json!({ "chat_id": chat, "message_id": message_id, "text": text, "reply_markup": kb })).is_ok() {
            return;
        }
        send_kb(c, token, chat, text, kb);
    }

    fn answer_cb(c: &reqwest::blocking::Client, token: &str, id: &str, text: &str) {
        let _ = call(c, token, "answerCallbackQuery", &json!({ "callback_query_id": id, "text": text }));
    }

    // sendPhoto / sendVoice: a multipart body built by hand (the file is held in memory, never written to disk)
    pub(super) fn send_file(c: &reqwest::blocking::Client, token: &str, method: &str, chat: i64, field: &str, filename: &str, mime: &str, bytes: &[u8], caption: &str) -> Result<(), String> {
        let boundary = format!("jarvis{:016x}", rand::random::<u64>());
        let mut body: Vec<u8> = Vec::new();
        let mut text_part = |name: &str, value: &str| {
            body.extend_from_slice(format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n", boundary, name, value).as_bytes());
        };
        text_part("chat_id", &chat.to_string());
        if !caption.is_empty() {
            text_part("caption", caption);
        }
        body.extend_from_slice(format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n", boundary, field, filename, mime).as_bytes());
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
        let url = format!("{}/bot{}/{}", BASE.lock(), token, method);
        let resp = c
            .post(url)
            .header("Content-Type", format!("multipart/form-data; boundary={}", boundary))
            .body(body)
            .timeout(Duration::from_secs(60))
            .send()
            .map_err(|e| if e.is_timeout() { "нет ответа от Telegram".to_string() } else { "нет связи с api.telegram.org".to_string() })?;
        if resp.status().is_success() { Ok(()) } else { Err(format!("Telegram не принял файл (ошибка {})", resp.status().as_u16())) }
    }

    // the file of a voice message (kept in memory; the address holds the token, so errors never show it)
    pub(super) fn download(c: &reqwest::blocking::Client, token: &str, file_id: &str) -> Result<Vec<u8>, String> {
        let info = call(c, token, "getFile", &json!({ "file_id": file_id }))?;
        let path = info["result"]["file_path"].as_str().ok_or("Telegram не отдал файл")?;
        if path.is_empty() || path.len() > 200 || path.contains("..") || !path.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '.' | '_' | '-')) {
            return Err("Telegram прислал странный адрес файла".into());
        }
        let url = format!("{}/file/bot{}/{}", BASE.lock(), token, path);
        let resp = c.get(url).send().map_err(|_| "не удалось скачать голосовое".to_string())?;
        if !resp.status().is_success() {
            return Err("Telegram не отдал голосовое".into());
        }
        if resp.content_length().map(|l| l > crate::tgvoice::MAX_VOICE_BYTES).unwrap_or(false) {
            return Err("файл слишком большой".into());
        }
        let bytes = resp.bytes().map_err(|_| "не удалось скачать голосовое".to_string())?;
        if bytes.len() as u64 > crate::tgvoice::MAX_VOICE_BYTES {
            return Err("файл слишком большой".into());
        }
        Ok(bytes.to_vec())
    }

    struct Ctx<'a> {
        c: &'a reqwest::blocking::Client,
        token: &'a str,
        gen: u64,
    }

    // one phrase for Jarvis (typed, spoken in a voice message or pressed as a button): the same queue as the window.
    // Returns the answer, None when the phrase was not run.
    fn run_phrase(cx: &Ctx, chat: i64, phrase: &str, offset: i64, times: &mut Vec<Instant>) -> Option<String> {
        if !rate_ok(times, Instant::now()) {
            send(cx.c, cx.token, chat, "Слишком много команд подряд. Подождите минуту.");
            return None;
        }
        // Telegram is told first that this message is read: a crash or a restart during the command
        // never runs it a second time
        let _ = call(cx.c, cx.token, "getUpdates", &json!({ "offset": offset, "timeout": 0, "limit": 1, "allowed_updates": ["message", "callback_query"] }));
        let before = chat_snapshot();
        if !crate::remote::queue_ready() || !crate::remote::submit(crate::origin::Source::Telegram, phrase) {
            send(cx.c, cx.token, chat, "Джарвис ещё запускается, команды пока не принимаются.");
            return None;
        }
        log::info!("Telegram command: {} chars", phrase.chars().count());
        Some(collect_answer(before, Duration::from_secs(45), cx.gen))
    }

    // the answer as text under a "menu" button, and as a voice message when the user switched that on
    fn reply(cx: &Ctx, chat: i64, answer: &str, kb: &Value) {
        // an answer that waits for «да / нет» carries the two buttons (the phone remote has them too)
        let yes_no = crate::tgmenu::yes_no();
        let kb = if super::confirm_open() && *kb == crate::tgmenu::menu_button() { &yes_no } else { kb };
        send_kb(cx.c, cx.token, chat, answer, kb);
        if CONFIG.lock().voice_reply {
            match speak_reply(cx, chat, answer) {
                Ok(()) => {}
                Err(e) => send(cx.c, cx.token, chat, &format!("🔇 Голосовой ответ не получился: {}", e)),
            }
        }
    }

    fn speak_reply(cx: &Ctx, chat: i64, answer: &str) -> Result<(), String> {
        let text: String = answer.chars().take(500).collect();
        let wav = crate::tts::synth_to_wav(&text)?;
        let ogg = crate::tgvoice::encode_wav_to_ogg_opus(&wav)?;
        send_file(cx.c, cx.token, "sendVoice", chat, "voice", "jarvis.ogg", "audio/ogg", &ogg, "")
    }

    fn handle_run(cx: &Ctx, chat: i64, phrase: &str, offset: i64, times: &mut Vec<Instant>) {
        if let Some(answer) = run_phrase(cx, chat, phrase, offset, times) {
            if alive(cx.gen) {
                reply(cx, chat, &answer, &crate::tgmenu::menu_button());
            }
        }
    }

    fn handle_voice(cx: &Ctx, chat: i64, v: &VoiceMsg, offset: i64, times: &mut Vec<Instant>) {
        if CONFIG.lock().block_voice {
            send(cx.c, cx.token, chat, "Голосовые сообщения выключены в настройках Jarvis. Напишите команду текстом.");
            return;
        }
        if v.duration > crate::tgvoice::MAX_VOICE_SECS || (v.size > crate::tgvoice::MAX_VOICE_BYTES) {
            send(cx.c, cx.token, chat, &format!("Голосовое слишком длинное: не больше {} секунд.", crate::tgvoice::MAX_VOICE_SECS));
            return;
        }
        let _ = call(cx.c, cx.token, "sendChatAction", &json!({ "chat_id": chat, "action": "typing" }));
        let samples = match download(cx.c, cx.token, &v.file_id).and_then(|b| crate::tgvoice::decode_ogg_opus(&b)) {
            Ok(s) => s,
            Err(e) => {
                send(cx.c, cx.token, chat, &format!("Не удалось принять голосовое: {}. Напишите команду текстом.", e));
                return;
            }
        };
        if samples.len() < 3200 {
            send(cx.c, cx.token, chat, "Голосовое слишком короткое: я ничего не расслышал.");
            return;
        }
        let text = match crate::tgvoice::recognize(&samples) {
            Ok(t) => t.split_whitespace().collect::<Vec<_>>().join(" "),
            Err(e) => {
                send(cx.c, cx.token, chat, &format!("Не удалось распознать голос: {}. Напишите команду текстом.", e));
                return;
            }
        };
        if text.is_empty() {
            send(cx.c, cx.token, chat, "Я ничего не расслышал. Скажите ещё раз или напишите команду.");
            return;
        }
        let text: String = text.chars().take(MAX_TEXT).collect();
        // what was understood is shown before it is run (and what Jarvis heard is never kept in the log)
        send(cx.c, cx.token, chat, &format!("🎤 «{}»", text));
        handle_run(cx, chat, &text, offset, times);
    }

    fn handle_button(cx: &Ctx, chat: i64, cb: &Callback, offset: i64, times: &mut Vec<Instant>) {
        use crate::tgmenu::{self, Press};
        let allow_screen = CONFIG.lock().allow_screen;
        match tgmenu::parse(&cb.data) {
            Press::Page(name) => {
                answer_cb(cx.c, cx.token, &cb.id, "");
                if let Some((text, kb)) = tgmenu::page(&name, allow_screen) {
                    show(cx.c, cx.token, chat, cb.message_id, &text, &kb);
                }
            }
            Press::Cancel => {
                *PENDING.lock() = None;
                answer_cb(cx.c, cx.token, &cb.id, "Отменено");
                if let Some((text, kb)) = tgmenu::page("main", allow_screen) {
                    show(cx.c, cx.token, chat, cb.message_id, &text, &kb);
                }
            }
            Press::Run(key) => {
                let Some(it) = tgmenu::find(&key) else { return };
                // «да» / «нет»: only to a question that is open now (an old button never answers a later question)
                if matches!(key.as_str(), "yes" | "no") && !super::confirm_open() {
                    answer_cb(cx.c, cx.token, &cb.id, "Сейчас Jarvis ничего не спрашивает.");
                    return;
                }
                match it.ask {
                    Some(q) => {
                        // the question lives for a minute and can be answered once: an old "Да" never runs anything
                        *PENDING.lock() = Some((key.clone(), Instant::now()));
                        answer_cb(cx.c, cx.token, &cb.id, "");
                        show(cx.c, cx.token, chat, cb.message_id, &format!("⚠ {}", q), &tgmenu::confirm_keyboard(&key));
                    }
                    None => {
                        answer_cb(cx.c, cx.token, &cb.id, it.label);
                        handle_run(cx, chat, it.phrase, offset, times);
                    }
                }
            }
            Press::Yes(key) => {
                let fresh = PENDING.lock().take().map(|(k, at)| k == key && at.elapsed() < Duration::from_secs(60)).unwrap_or(false);
                let Some(it) = tgmenu::find(&key).filter(|_| fresh) else {
                    answer_cb(cx.c, cx.token, &cb.id, "Подтверждение устарело. Откройте меню и нажмите ещё раз.");
                    return;
                };
                answer_cb(cx.c, cx.token, &cb.id, "Выполняю");
                crate::uistate::log_event("info", &format!("Telegram: подтверждено кнопкой «{}»", it.label));
                if it.phrase.is_empty() {
                    local_action(cx, chat, it.key);
                    return;
                }
                let Some(first) = run_phrase(cx, chat, it.phrase, offset, times) else { return };
                let mut answer = first;
                // the command asked its own «да / нет»: the button was the answer
                // (only when Jarvis really asks the question of a power command: «да» is never sent into another session)
                if it.then_yes {
                    if super::power_question_open() {
                        if let Some(second) = run_phrase(cx, chat, "да", offset, times) {
                            answer = second;
                        }
                    } else {
                        answer = format!("{}\n\n⚠ Jarvis не задал вопрос о подтверждении, поэтому команда не выполнена.", answer);
                    }
                }
                if alive(cx.gen) {
                    let kb = if it.then_yes { tgmenu::after_power() } else { tgmenu::menu_button() };
                    reply(cx, chat, &answer, &kb);
                }
            }
            Press::Action(name) => {
                answer_cb(cx.c, cx.token, &cb.id, "");
                match name.as_str() {
                    "status" => send_kb(cx.c, cx.token, chat, &super::status_text(), &tgmenu::menu_button()),
                    "tasks" => send_tasks(cx, chat),
                    "screen" => send_screen(cx, chat),
                    _ => {}
                }
            }
            Press::TaskDone(n, hash) => {
                match crate::tasks::list() {
                    Ok(list) => match list.get(n) {
                        Some(t) if tgmenu::task_hash(t) == hash => match crate::tasks::remove(n, t) {
                            Ok(done) => {
                                answer_cb(cx.c, cx.token, &cb.id, "Готово");
                                send(cx.c, cx.token, chat, &format!("✅ Выполнено: {}", done));
                            }
                            Err(e) => answer_cb(cx.c, cx.token, &cb.id, &e),
                        },
                        _ => answer_cb(cx.c, cx.token, &cb.id, "Список задач изменился. Откройте его заново."),
                    },
                    Err(e) => answer_cb(cx.c, cx.token, &cb.id, &e),
                }
                send_tasks(cx, chat);
            }
            Press::Unknown => answer_cb(cx.c, cx.token, &cb.id, "Эта кнопка устарела."),
        }
    }

    // the remote of the phone, from the bot: switched on, off, its link, a new link (each one was confirmed by the owner)
    fn local_action(cx: &Ctx, chat: i64, key: &str) {
        let kb = crate::tgmenu::menu_button();
        match key {
            "rmon" => {
                if let Err(e) = crate::remote::start() {
                    send_kb(cx.c, cx.token, chat, &format!("Не удалось включить пульт: {}", e), &kb);
                    return;
                }
                if let Err(e) = crate::tunnel::enable() {
                    send_kb(cx.c, cx.token, chat, &format!("Пульт включён в вашей сети, но доступ из любой сети не включился: {}", e), &kb);
                    return;
                }
                crate::uistate::log_event("info", "Telegram: пульт и доступ из любой сети включены владельцем");
                send_kb(cx.c, cx.token, chat, "Пульт включён. Адрес для доступа из любой сети готовится (в первый раз это до пары минут: скачивается cloudflared). Пришлю ссылку, как только она будет готова.", &kb);
                // the address comes later: a thread of its own waits for it and sends it (the bot goes on meanwhile)
                let (token, gen) = (cx.token.to_string(), cx.gen);
                std::thread::spawn(move || {
                    let Some(c) = client() else { return };
                    let t0 = Instant::now();
                    while t0.elapsed() < Duration::from_secs(300) && alive(gen) {
                        std::thread::sleep(Duration::from_secs(3));
                        match crate::tunnel::state() {
                            crate::tunnel::State::On(_) => {
                                let link = crate::tunnel::public_link().unwrap_or_default();
                                send_kb(&c, &token, chat, &format!("🔗 Ссылка пульта из любой сети (в ней секрет, не пересылайте):\n{}", link), &crate::tgmenu::menu_button());
                                return;
                            }
                            crate::tunnel::State::Error(e) => {
                                send_kb(&c, &token, chat, &format!("Доступ из любой сети не включился: {}", e), &crate::tgmenu::menu_button());
                                return;
                            }
                            _ => {}
                        }
                    }
                });
            }
            "rmoff" => {
                crate::tunnel::disable();
                crate::remote::stop();
                crate::uistate::log_event("info", "Telegram: пульт и доступ из любой сети выключены владельцем");
                send_kb(cx.c, cx.token, chat, "Пульт и доступ из любой сети выключены.", &kb);
            }
            "rmlink" => {
                if !crate::remote::is_running() {
                    send_kb(cx.c, cx.token, chat, "Пульт выключен. Сначала включите его кнопкой «Включить пульт».", &kb);
                    return;
                }
                let text = match crate::tunnel::public_link() {
                    Some(link) => format!("🔗 Ссылка пульта из любой сети (в ней секрет, не пересылайте):\n{}", link),
                    None => format!("🔗 Адрес пульта в вашей сети (без секрета, https):\n{}\n\nТелефон привязывается одноразовым кодом из окна Jarvis. Для доступа из другой сети включите «Пульт и доступ из любой сети».", crate::remote::url()),
                };
                send_kb(cx.c, cx.token, chat, &text, &kb);
            }
            "rmnew" => match crate::remote::renew_token() {
                Ok(_) => send_kb(cx.c, cx.token, chat, "Готово: старая ссылка больше не работает. Новую пришлёт кнопка «Прислать ссылку пульта».", &kb),
                Err(e) => send_kb(cx.c, cx.token, chat, &format!("Не удалось сделать новую ссылку: {}", e), &kb),
            },
            _ => {}
        }
    }

    fn send_tasks(cx: &Ctx, chat: i64) {
        match crate::tasks::list() {
            Ok(list) => {
                let (text, kb) = crate::tgmenu::tasks_message(&list);
                send_kb(cx.c, cx.token, chat, &text, &kb);
            }
            Err(e) => send(cx.c, cx.token, chat, &e),
        }
    }

    // a picture of the screen goes through the servers of Telegram: only when the user allowed it in the window
    fn send_screen(cx: &Ctx, chat: i64) {
        if !CONFIG.lock().allow_screen {
            send(cx.c, cx.token, chat, "Снимок экрана выключен. Включите «Разрешить боту присылать снимок экрана» в окне Jarvis → Настройки → Telegram. Снимок проходит через серверы Telegram, поэтому по умолчанию это запрещено.");
            return;
        }
        static LAST: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));
        {
            let mut last = LAST.lock();
            if last.map(|t| t.elapsed() < Duration::from_secs(10)).unwrap_or(false) {
                send(cx.c, cx.token, chat, "Снимок можно делать не чаще раза в 10 секунд.");
                return;
            }
            *last = Some(Instant::now());
        }
        let _ = call(cx.c, cx.token, "sendChatAction", &json!({ "chat_id": chat, "action": "upload_photo" }));
        match crate::screen::capture() {
            Ok(jpeg) => match send_file(cx.c, cx.token, "sendPhoto", chat, "photo", "screen.jpg", "image/jpeg", &jpeg, "Экран компьютера") {
                Ok(()) => crate::uistate::log_event("info", "Telegram: снимок экрана отправлен владельцу"),
                Err(e) => send(cx.c, cx.token, chat, &format!("Не удалось отправить снимок: {}", e)),
            },
            Err(e) => send(cx.c, cx.token, chat, &format!("Не удалось сделать снимок: {}", e)),
        }
    }

    pub fn run(gen: u64) {
        let Some(c) = client() else {
            set_status("не удалось запустить сеть");
            return;
        };
        let token = CONFIG.lock().token.clone();
        let cx = Ctx { c: &c, token: &token, gen };
        let mut offset: i64 = OFFSET.load(std::sync::atomic::Ordering::SeqCst);
        let mut backoff = 5u64;
        let mut times: Vec<Instant> = Vec::new();
        match call(&c, &token, "getMe", &json!({})) {
            Ok(v) => {
                let name = v["result"]["username"].as_str().unwrap_or("").to_string();
                let mut cfg = CONFIG.lock();
                // (a loop that was stopped meanwhile, or a token changed or forgotten, writes nothing)
                if alive(gen) && cfg.token == token && cfg.bot_name != name {
                    cfg.bot_name = name;
                    save(&cfg);
                }
                // the list of commands that Telegram shows when "/" is typed
                let commands: Vec<Value> = [("menu", "Меню с кнопками"), ("status", "Состояние компьютера и Jarvis"), ("tasks", "Мои задачи"), ("screen", "Снимок экрана (если разрешён)"), ("help", "Помощь")]
                    .iter()
                    .map(|(c, d)| json!({ "command": c, "description": d }))
                    .collect();
                drop(cfg);
                let _ = call(&c, &token, "setMyCommands", &json!({ "commands": commands }));
            }
            Err(e) => {
                if !alive(gen) {
                    return;
                }
                set_status(&e);
                if e.contains("токен") {
                    return;
                }
            }
        }
        while alive(gen) {
            let r = call(&c, &token, "getUpdates", &json!({ "offset": offset, "timeout": 25, "allowed_updates": ["message", "callback_query"] }));
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
                // switched off (or another token) in the middle of a batch: the rest is not run
                if !alive(gen) {
                    break;
                }
                offset = offset.max(m.update_id + 1);
                OFFSET.fetch_max(offset, std::sync::atomic::Ordering::SeqCst);
                if too_old(m.date, now_secs()) {
                    log::info!("Telegram: an old message is skipped");
                    continue;
                }
                let owner = CONFIG.lock().owner_id;
                let code = CODE.lock().as_ref().filter(|(_, at)| at.elapsed() < CODE_TTL).map(|(c, _)| c.clone());
                match decide(owner, code.as_deref(), &m) {
                    Decision::Ignore => {}
                    Decision::WrongCode => {
                        if wrong_code(m.from_id) {
                            reset_code();
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
                        reset_code();
                        crate::uistate::log_event("info", "Telegram: телефон привязан");
                        let kb = crate::tgmenu::page("main", false).map(|p| p.1).unwrap_or(Value::Null);
                        send_kb(&c, &token, m.chat_id, &format!("Готово: этот аккаунт привязан к JARVIS.\n\n{}", HELP), &kb);
                    }
                    Decision::Help => {
                        let kb = crate::tgmenu::page("main", CONFIG.lock().allow_screen).map(|p| p.1).unwrap_or(Value::Null);
                        send_kb(&c, &token, m.chat_id, HELP, &kb);
                    }
                    Decision::Menu => {
                        if let Some((text, kb)) = crate::tgmenu::page("main", CONFIG.lock().allow_screen) {
                            send_kb(&c, &token, m.chat_id, &text, &kb);
                        }
                    }
                    Decision::Status => send_kb(&c, &token, m.chat_id, &super::status_text(), &crate::tgmenu::menu_button()),
                    Decision::Tasks => send_tasks(&cx, m.chat_id),
                    Decision::Screen => send_screen(&cx, m.chat_id),
                    Decision::TooLong => send(&c, &token, m.chat_id, "Слишком длинное сообщение: не больше 300 символов."),
                    Decision::Run(text) => handle_run(&cx, m.chat_id, &text, offset, &mut times),
                    Decision::Voice(v) => handle_voice(&cx, m.chat_id, &v, offset, &mut times),
                    Decision::Button(cb) => handle_button(&cx, m.chat_id, &cb, offset, &mut times),
                }
            }
        }
    }
}

// what /status shows: the state of the computer and of Jarvis (numbers and states only, no contents)
pub fn status_text() -> String {
    let h = crate::health::json();
    let v = crate::uistate::voice_state();
    let on = |b: bool| if b { "включена" } else { "выключена" };
    let mut lines = vec!["📊 JARVIS и компьютер".to_string()];
    lines.push(format!("🖥 Нагрузка: {} · процессор {}% · память {}% ({} из {} ГБ)", h["pc_status"].as_str().unwrap_or("?"), h["cpu"], h["ram_pct"], h["ram_used"], h["ram_total"]));
    if let Some(d) = h["disk_pct"].as_f64() {
        lines.push(format!("💾 Диск: занято {}% · свободно {} ГБ", d, h["disk_free"]));
    }
    lines.push(format!("🌐 Сеть: {}", if h["network"] == "ONLINE" { "есть" } else { "нет" }));
    let voice = match v["state"].as_str().unwrap_or("") {
        "READY" => "слушает".to_string(),
        "LISTENING" => "слушает команду".to_string(),
        "SPEAKING" => "говорит".to_string(),
        "DISABLED" => format!("выключен — {}", v["detail"].as_str().unwrap_or("")),
        _ => format!("недоступен — {}", v["detail"].as_str().unwrap_or("")),
    };
    lines.push(format!("🎙 Голосовой ввод: {}", voice));
    lines.push(format!("🔈 Озвучка: {}", on(crate::tts::is_enabled())));
    let tasks = crate::tasks::list().map(|l| l.len().to_string()).unwrap_or_else(|_| "?".into());
    lines.push(format!("📋 Задач: {}", tasks));
    let ph = crate::remote::phone_json();
    let phone = if ph["online"] == true {
        let bat = ph["battery"].as_f64().map(|b| format!(", заряд {}%{}", (b * 100.0).round(), if ph["charging"] == true { " ⚡" } else { "" })).unwrap_or_default();
        format!("на связи{}", bat)
    } else if crate::remote::is_running() {
        "пульт включён, телефон не подключён".to_string()
    } else {
        "пульт выключен".to_string()
    };
    let tunnel = match crate::tunnel::state() {
        crate::tunnel::State::On(_) => ", доступ из любой сети включён",
        crate::tunnel::State::Downloading(_) | crate::tunnel::State::Starting => ", доступ из любой сети запускается",
        _ => "",
    };
    lines.push(format!("📱 Телефон: {}{}", phone, tunnel));
    if let Some(mb) = h["self_mb"].as_f64() {
        lines.push(format!("🧠 Jarvis занимает {} МБ памяти", mb));
    }
    lines.push(format!("🕒 На компьютере сейчас {}", chrono::Local::now().format("%H:%M, %d.%m.%Y")));
    lines.join("\n")
}

// Jarvis waits for «да / нет» to something (a power command, the recycle bin, the tasks)
fn confirm_open() -> bool {
    crate::confirm::pending_for(crate::origin::Source::Telegram) || crate::session::kind().as_deref() == Some("confirm")
}

// A timer, an alarm or a reminder: the paired owner gets it in the bot too (the user who steers the
// computer from the phone is not at the screen). Nothing is sent when the bot is off, not paired, or the user switched
// the alerts off. The text is what the user himself asked Jarvis to say.
pub fn announce(title: &str, text: &str) {
    #[cfg(feature = "lua")]
    {
        let c = CONFIG.lock().clone();
        if !c.enabled || c.owner_id == 0 || c.token.is_empty() || c.no_alerts {
            return;
        }
        let body = if text.trim().is_empty() { title.to_string() } else if title.trim().is_empty() { text.to_string() } else { format!("{}: {}", title, text) };
        let msg = format!("⏰ {}", body);
        std::thread::spawn(move || {
            if let Some(cl) = net::client() {
                net::send_plain(&cl, &c.token, c.owner_id, &msg);
            }
        });
    }
    #[cfg(not(feature = "lua"))]
    {
        let _ = (title, text);
    }
}

// Jarvis waits for «да / нет» to a power command (not to something else)
fn power_question_open() -> bool {
    // the question of Jarvis about a dangerous command (confirm.rs) that came through this bot, or the old one of a session
    crate::confirm::pending_for(crate::origin::Source::Telegram) || is_power_question(crate::session::kind().as_deref(), &crate::session::data())
}

fn is_power_question(kind: Option<&str>, data: &str) -> bool {
    kind == Some("confirm") && data.starts_with("power:")
}

// the question of a dangerous button that waits for «Да»: (entry, when asked)
static PENDING: Lazy<Mutex<Option<(String, Instant)>>> = Lazy::new(|| Mutex::new(None));

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
        Incoming { update_id: 1, from_id: from, chat_id: from, private, name: "Хасан".into(), text: text.into(), ..Default::default() }
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
    fn messages_written_while_jarvis_was_off_are_not_run() {
        assert!(too_old(1_000, 1_000 + MAX_AGE_SECS + 1));
        assert!(!too_old(1_000, 1_030));
        assert!(!too_old(0, 1_000_000), "no date: not judged by age");
    }

    #[test]
    fn the_new_question_is_found_by_its_id_even_in_a_cut_conversation() {
        let m = |role: &str, rid: &str, ts: u64| crate::ai::ChatMsg { role: role.into(), content: "x".into(), ts, rid: rid.into(), src: String::new(), streaming: false };
        // the stored conversation keeps its last messages: the count did not grow, the new question is still found
        let msgs = vec![m("user", "old1", 90), m("assistant", "old1", 90), m("user", "new", 100), m("assistant", "new", 101)];
        let snap = vec!["old1".to_string()];
        assert_eq!(new_question(&msgs, true, &snap, 100), Some(2));
        // an old question of the same chat is not taken for the new one
        assert_eq!(new_question(&msgs[..2], true, &snap, 100), None);
        // another chat became active: its question counts
        assert_eq!(new_question(&msgs, false, &snap, 100), Some(2));
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
        set_for_tests(Config { enabled: false, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, owner_name: "Хасан".into(), bot_name: "my_jarvis_bot".into(), ..Default::default() });
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
                    // (only the messages are counted: setMyCommands and the like are not)
                    if first.contains("/sendMessage") {
                        sent2.lock().push(body.clone());
                    }
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
        assert!(OFFSET.load(std::sync::atomic::Ordering::SeqCst) >= 6, "a restarted loop starts after the updates already read");
        *BASE.lock() = "https://api.telegram.org".into();
        set_for_tests(Config::default());
    }

    #[test]
    fn wrong_codes_count_per_account_and_unpair_or_forget_cancel_a_waiting_code() {
        let _g = TEST_LOCK.lock();
        reset_code();
        for _ in 0..MAX_WRONG - 1 {
            assert!(!wrong_code(99));
        }
        assert!(!wrong_code(77), "another account has its own tries: a stranger cannot use them up for the owner");
        assert!(wrong_code(99), "the fifth wrong code of one account");
        set_for_tests(Config { token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, ..Default::default() });
        action(&json!({ "op": "pair" }));
        assert!(CODE.lock().is_some());
        action(&json!({ "op": "unpair" }));
        assert!(CODE.lock().is_none(), "a code made before unpair does not stay valid for the next account");
        action(&json!({ "op": "pair" }));
        action(&json!({ "op": "forget" }));
        assert!(CODE.lock().is_none() && WRONG.lock().is_empty());
        set_for_tests(Config::default());
    }

    #[test]
    fn a_lone_slash_is_help_not_an_empty_command() {
        assert_eq!(decide(77, None, &msg(77, "/", true)), Decision::Help);
        assert_eq!(decide(77, None, &msg(77, "/ ", true)), Decision::Help);
        assert_eq!(decide(77, None, &msg(77, "/громкость 30", true)), Decision::Run("громкость 30".into()));
    }

    #[test]
    fn five_wrong_codes_are_counted_by_the_decision() {
        // (the loop cancels the code after MAX_WRONG of these; six digits are never guessable in five tries)
        let wrong: Vec<Decision> = (0..MAX_WRONG).map(|i| decide(0, Some("482913"), &msg(99, &format!("00000{}", i), true))).collect();
        assert!(wrong.iter().all(|d| *d == Decision::WrongCode));
    }

    #[test]
    fn buttons_and_voice_messages_obey_only_the_paired_owner() {
        let cb = |from: i64, data: &str, private: bool| Incoming { from_id: from, chat_id: from, private, callback: Some(Callback { id: "q1".into(), data: data.into(), message_id: 5 }), ..Default::default() };
        let vo = |from: i64, private: bool| Incoming { from_id: from, chat_id: from, private, voice: Some(VoiceMsg { file_id: "f".into(), duration: 3, size: 1000 }), ..Default::default() };
        assert!(matches!(decide(77, None, &cb(77, "r:pwoff", true)), Decision::Button(_)));
        assert_eq!(decide(77, None, &cb(99, "r:pwoff", true)), Decision::Ignore, "a stranger's button");
        assert_eq!(decide(77, None, &cb(77, "r:pwoff", false)), Decision::Ignore, "never from a group");
        assert_eq!(decide(0, Some("482913"), &cb(77, "r:pwoff", true)), Decision::Ignore, "before pairing nothing is obeyed");
        assert!(matches!(decide(77, None, &vo(77, true)), Decision::Voice(_)));
        assert_eq!(decide(77, None, &vo(99, true)), Decision::Ignore, "a stranger's voice");
        assert_eq!(decide(77, None, &vo(77, false)), Decision::Ignore);
        assert_eq!(decide(0, Some("482913"), &vo(77, true)), Decision::Ignore);
    }

    #[test]
    fn the_commands_of_the_bot_are_not_phrases_for_jarvis() {
        for (t, want) in [("/menu", Decision::Menu), ("/menu@my_jarvis_bot", Decision::Menu), ("/status", Decision::Status), ("/tasks", Decision::Tasks), ("/screen", Decision::Screen), ("/help", Decision::Help), ("/start", Decision::Help)] {
            assert_eq!(decide(77, None, &msg(77, t, true)), want, "{}", t);
        }
        assert_eq!(decide(77, None, &msg(99, "/screen", true)), Decision::Ignore, "a stranger gets no screenshot");
        assert_eq!(decide(77, None, &msg(77, "/screen now", true)), Decision::Run("screen now".into()));
    }

    #[test]
    fn buttons_and_voice_are_read_from_updates() {
        let v = json!({ "ok": true, "result": [
            { "update_id": 20, "callback_query": { "id": "cb1", "from": { "id": 77, "first_name": "Хасан" }, "data": "m:power", "message": { "message_id": 9, "date": 1, "chat": { "id": 77, "type": "private" } } } },
            { "update_id": 21, "message": { "from": { "id": 77 }, "chat": { "id": 77, "type": "private" }, "date": 5, "voice": { "file_id": "AwAD", "duration": 4, "file_size": 20000 } } },
            { "update_id": 22, "callback_query": { "id": "cb2" } }
        ]});
        let u = parse_updates(&v);
        assert_eq!(u.len(), 3);
        let cb = u[0].callback.as_ref().unwrap();
        assert_eq!((cb.id.as_str(), cb.data.as_str(), cb.message_id), ("cb1", "m:power", 9));
        assert!(u[0].private && u[0].from_id == 77);
        assert_eq!(u[0].date, 0, "a button is not judged by the age of the message under it");
        let vm = u[1].voice.as_ref().unwrap();
        assert_eq!((vm.file_id.as_str(), vm.duration, vm.size), ("AwAD", 4, 20000));
        assert!(u[2].callback.is_none() && u[2].voice.is_none() && u[2].update_id == 22, "an incomplete update is only counted");
    }

    // the whole bot against a local imitation of the Bot API
    #[cfg(feature = "lua")]
    mod api {
        use super::*;
        use std::io::{Read, Write};

        pub struct Mock {
            pub calls: std::sync::Arc<Mutex<Vec<(String, Value, usize)>>>,
        }

        // `rounds`: the answers of getUpdates in turn; then empty ones
        pub fn serve(rounds: Vec<Value>) -> Mock {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            *BASE.lock() = format!("http://127.0.0.1:{}", port);
            let calls: std::sync::Arc<Mutex<Vec<(String, Value, usize)>>> = Default::default();
            let calls2 = calls.clone();
            std::thread::spawn(move || {
                let mut round = 0usize;
                for conn in listener.incoming().flatten() {
                    let mut conn = conn;
                    let mut buf: Vec<u8> = Vec::new();
                    let mut chunk = vec![0u8; 65536];
                    let (mut head_end, mut want) = (0usize, usize::MAX);
                    loop {
                        let n = conn.read(&mut chunk).unwrap_or(0);
                        if n == 0 {
                            break;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if head_end == 0 {
                            if let Some(h) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                head_end = h + 4;
                                let head = String::from_utf8_lossy(&buf[..h]).to_lowercase();
                                let len = head.lines().find_map(|l| l.strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0))).unwrap_or(0);
                                want = head_end + len;
                            }
                        }
                        if buf.len() >= want {
                            break;
                        }
                    }
                    let head = String::from_utf8_lossy(&buf[..head_end.min(buf.len())]).to_string();
                    let first = head.lines().next().unwrap_or("").to_string();
                    let body_bytes = &buf[head_end.min(buf.len())..];
                    let body: Value = serde_json::from_slice(body_bytes).unwrap_or(json!({}));
                    let method = first.split_whitespace().nth(1).unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
                    if first.starts_with("GET /file/") {
                        let data = include_bytes!("testdata/voice.ogg");
                        let _ = conn.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", data.len()).as_bytes());
                        let _ = conn.write_all(data);
                        continue;
                    }
                    let answer = match method.as_str() {
                        "getMe" => json!({ "ok": true, "result": { "username": "test_jarvis_bot" } }),
                        "getUpdates" => {
                            if body["timeout"] == 0 {
                                json!({ "ok": true, "result": [] })
                            } else {
                                round += 1;
                                match rounds.get(round - 1) {
                                    Some(r) => r.clone(),
                                    None => {
                                        std::thread::sleep(Duration::from_millis(300));
                                        json!({ "ok": true, "result": [] })
                                    }
                                }
                            }
                        }
                        "getFile" => json!({ "ok": true, "result": { "file_path": "voice/file_1.oga" } }),
                        _ => {
                            calls2.lock().push((method.clone(), body.clone(), body_bytes.len()));
                            json!({ "ok": true, "result": {} })
                        }
                    };
                    let a = answer.to_string();
                    let _ = conn.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", a.len(), a).as_bytes());
                }
            });
            Mock { calls }
        }

        pub fn cb(update_id: i64, from: i64, data: &str) -> Value {
            json!({ "update_id": update_id, "callback_query": { "id": format!("cb{}", update_id), "from": { "id": from }, "data": data, "message": { "message_id": 50, "chat": { "id": from, "type": "private" } } } })
        }

        pub fn text(update_id: i64, from: i64, t: &str) -> Value {
            json!({ "update_id": update_id, "message": { "from": { "id": from }, "chat": { "id": from, "type": "private" }, "text": t } })
        }

        pub fn voice(update_id: i64, from: i64) -> Value {
            json!({ "update_id": update_id, "message": { "from": { "id": from }, "chat": { "id": from, "type": "private" }, "voice": { "file_id": "AwAD", "duration": 4, "file_size": 9522 } } })
        }

        // waits until `n` calls of this method were made
        pub fn wait_for(m: &Mock, method: &str, n: usize) {
            let t0 = Instant::now();
            while m.calls.lock().iter().filter(|c| c.0 == method).count() < n && t0.elapsed() < Duration::from_secs(15) {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    #[cfg(feature = "lua")]
    #[test]
    fn the_menu_pages_confirmations_and_status_work_for_the_owner_only() {
        use api::*;
        let _g = TEST_LOCK.lock();
        let m = serve(vec![
            json!({ "ok": true, "result": [text(1, 77, "/menu"), cb(2, 99, "m:power"), cb(3, 77, "m:power"), cb(4, 77, "y:pwoff"), cb(5, 77, "r:pwoff"), text(6, 77, "/status"), text(7, 77, "/screen")] }),
            json!({ "ok": true, "result": [cb(8, 77, "y:pwoff"), cb(9, 77, "y:pwoff"), cb(10, 77, "a:tasks"), cb(11, 77, "r:nonsense")] }),
        ]);
        set_for_tests(Config { enabled: true, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, ..Default::default() });
        *PENDING.lock() = None;
        start();
        // (sendMessage: the menu, status, screen refusal, "ещё запускается" and the tasks = 5; then a moment for the rest)
        wait_for(&m, "sendMessage", 5);
        wait_for(&m, "answerCallbackQuery", 5);
        std::thread::sleep(Duration::from_millis(500));
        stop();
        let calls = m.calls.lock().clone();
        let by = |name: &str| calls.iter().filter(|c| c.0 == name).map(|c| c.1.clone()).collect::<Vec<_>>();
        // the stranger's button did nothing at all: not even an answer
        assert!(calls.iter().all(|c| c.1["callback_query_id"] != "cb2" && c.1["chat_id"] != 99), "{:?}", calls);
        // the menu as a message with buttons; the page of the power commands as an edit of the same message
        let sent = by("sendMessage");
        assert!(sent[0]["text"].as_str().unwrap().starts_with("JARVIS — выберите действие"));
        assert!(sent[0]["reply_markup"]["inline_keyboard"].is_array());
        let edits = by("editMessageText");
        assert!(edits[0]["text"].as_str().unwrap().contains("Питание ПК"));
        assert_eq!(edits[0]["message_id"], 50);
        // «Да» with no question asked is stale; the question for the power command is a confirmation with two buttons
        let answers = by("answerCallbackQuery");
        assert!(answers.iter().any(|a| a["callback_query_id"] == "cb4" && a["text"].as_str().unwrap().contains("устарело")), "{:?}", answers);
        assert!(edits.iter().any(|e| e["text"].as_str().unwrap().contains("Выключить компьютер?") && e["reply_markup"].to_string().contains("y:pwoff")), "{:?}", edits);
        // /status is real numbers of this computer and nothing from the window
        assert!(sent.iter().any(|s| s["text"].as_str().unwrap().starts_with("📊 JARVIS и компьютер") && s["text"].as_str().unwrap().contains("Нагрузка")), "{:?}", sent);
        // the screenshot is refused until the user allows it
        assert!(sent.iter().any(|s| s["text"].as_str().unwrap().contains("Снимок экрана выключен")), "{:?}", sent);
        // the confirmed button was accepted once (the command queue is not connected in the tests: an honest answer), the second «Да» is stale
        assert!(sent.iter().any(|s| s["text"].as_str().unwrap().contains("ещё запускается")), "{:?}", sent);
        assert!(answers.iter().any(|a| a["callback_query_id"] == "cb9" && a["text"].as_str().unwrap().contains("устарело")), "{:?}", answers);
        // the task list and a button that does not exist
        assert!(sent.iter().any(|s| s["text"].as_str().unwrap().starts_with("📋")), "{:?}", sent);
        assert!(answers.iter().any(|a| a["callback_query_id"] == "cb11" && a["text"].as_str().unwrap().contains("устарела")));
        *BASE.lock() = "https://api.telegram.org".into();
        set_for_tests(Config::default());
    }

    // a voice message: downloaded, decoded, and — with no recognition model in this build — the owner is told so honestly
    #[cfg(feature = "lua")]
    #[test]
    fn a_voice_message_is_downloaded_and_the_owner_is_told_honestly_when_it_cannot_be_recognized() {
        use api::*;
        let _g = TEST_LOCK.lock();
        let m = serve(vec![json!({ "ok": true, "result": [voice(1, 99), voice(2, 77)] })]);
        set_for_tests(Config { enabled: true, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, ..Default::default() });
        start();
        wait_for(&m, "sendMessage", 1);
        std::thread::sleep(Duration::from_millis(500));
        stop();
        let sent: Vec<Value> = m.calls.lock().iter().filter(|c| c.0 == "sendMessage").map(|c| c.1.clone()).collect();
        assert!(sent.iter().all(|s| s["chat_id"] == 77), "nothing for the stranger: {:?}", sent);
        assert_eq!(sent.len(), 1, "{:?}", sent);
        assert!(sent[0]["text"].as_str().unwrap().contains("Не удалось распознать голос"), "{:?}", sent);
        // the same voice with the voice input switched off in the window
        let m = serve(vec![json!({ "ok": true, "result": [voice(3, 77)] })]);
        action(&json!({ "op": "options", "voice_in": false }));
        start();
        wait_for(&m, "sendMessage", 1);
        stop();
        let sent: Vec<Value> = m.calls.lock().iter().filter(|c| c.0 == "sendMessage").map(|c| c.1.clone()).collect();
        assert!(sent[0]["text"].as_str().unwrap().contains("выключены в настройках"), "{:?}", sent);
        *BASE.lock() = "https://api.telegram.org".into();
        set_for_tests(Config::default());
    }

    // sendPhoto / sendVoice: a real multipart request with the file in it
    #[cfg(feature = "lua")]
    #[test]
    fn a_file_is_sent_as_a_multipart_request() {
        use api::*;
        let _g = TEST_LOCK.lock();
        let m = serve(vec![]);
        let c = net::client().unwrap();
        net::send_file(&c, "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw", "sendVoice", 77, "voice", "jarvis.ogg", "audio/ogg", include_bytes!("testdata/voice.ogg"), "").unwrap();
        wait_for(&m, "sendVoice", 1);
        let calls = m.calls.lock().clone();
        assert_eq!(calls[0].0, "sendVoice");
        assert!(calls[0].2 > 9522, "the body holds the whole file: {}", calls[0].2);
        *BASE.lock() = "https://api.telegram.org".into();
    }

    #[test]
    fn the_options_of_the_bot_are_saved_and_start_off_where_they_must() {
        let _g = TEST_LOCK.lock();
        set_for_tests(Config::default());
        let j = json();
        assert_eq!(j["allow_screen"], false, "no screenshots until allowed");
        assert_eq!(j["voice_reply"], false);
        assert_eq!(j["voice_in"], true);
        action(&json!({ "op": "options", "allow_screen": true, "voice_reply": true, "voice_in": false }));
        let j = json();
        assert_eq!((j["allow_screen"].clone(), j["voice_reply"].clone(), j["voice_in"].clone()), (json!(true), json!(true), json!(false)));
        set_for_tests(Config::default());
    }

    #[test]
    fn the_button_answers_only_the_question_of_a_power_command() {
        assert!(is_power_question(Some("confirm"), "power:/s:60:0"));
        assert!(is_power_question(Some("confirm"), "power:/h:0:0"));
        assert!(!is_power_question(Some("confirm"), "trash"), "never «да» to the question about the recycle bin");
        assert!(!is_power_question(Some("confirm"), "tasks_clear"));
        assert!(!is_power_question(Some("word"), "power:/s:60:0"));
        assert!(!is_power_question(None, ""));
    }
    // a timer that rang goes to the paired owner in the bot, unless the alerts are switched off
    #[cfg(feature = "lua")]
    #[test]
    fn a_timer_that_rang_is_sent_to_the_owner_unless_switched_off() {
        use api::*;
        let _g = TEST_LOCK.lock();
        let m = serve(vec![]);
        set_for_tests(Config { enabled: true, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, ..Default::default() });
        announce("Таймер", "Время вышло: чай готов");
        wait_for(&m, "sendMessage", 1);
        let sent: Vec<Value> = m.calls.lock().iter().filter(|c| c.0 == "sendMessage").map(|c| c.1.clone()).collect();
        assert_eq!(sent.len(), 1, "{:?}", sent);
        assert_eq!(sent[0]["chat_id"], 77);
        assert_eq!(sent[0]["text"], "⏰ Таймер: Время вышло: чай готов");
        // alerts off, not paired, bot off: nothing
        let before = m.calls.lock().len();
        set_for_tests(Config { enabled: true, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, no_alerts: true, ..Default::default() });
        announce("Таймер", "x");
        set_for_tests(Config { enabled: true, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 0, ..Default::default() });
        announce("Таймер", "x");
        set_for_tests(Config { enabled: false, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, ..Default::default() });
        announce("Таймер", "x");
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(m.calls.lock().len(), before, "nothing was sent");
        action(&json!({ "op": "options", "alerts": false }));
        assert_eq!(json()["alerts"], false);
        *BASE.lock() = "https://api.telegram.org".into();
        set_for_tests(Config::default());
    }

    // the remote of the phone from the bot: every action asks first; «да» / «нет» only answer an open question
    #[cfg(feature = "lua")]
    #[test]
    fn the_phone_remote_buttons_ask_first_and_yes_no_need_an_open_question() {
        use api::*;
        let _g = TEST_LOCK.lock();
        crate::remote::phone_for_tests(false, false);
        let m = serve(vec![json!({ "ok": true, "result": [cb(1, 77, "m:phone"), cb(2, 77, "y:rmlink"), cb(3, 77, "r:rmlink"), cb(4, 77, "y:rmlink"), cb(5, 77, "r:yes"), cb(6, 77, "a:status")] })]);
        set_for_tests(Config { enabled: true, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, ..Default::default() });
        *PENDING.lock() = None;
        start();
        wait_for(&m, "sendMessage", 2);
        wait_for(&m, "answerCallbackQuery", 6);
        std::thread::sleep(Duration::from_millis(400));
        stop();
        let calls = m.calls.lock().clone();
        let by = |n: &str| calls.iter().filter(|c| c.0 == n).map(|c| c.1.clone()).collect::<Vec<_>>();
        let edits = by("editMessageText");
        assert!(edits.iter().any(|e| e["text"].as_str().unwrap().contains("Телефон и пульт")), "{:?}", edits);
        let answers = by("answerCallbackQuery");
        // a «Да» with no question asked is stale: the link was not sent
        assert!(answers.iter().any(|a| a["callback_query_id"] == "cb2" && a["text"].as_str().unwrap().contains("устарело")), "{:?}", answers);
        // the question for the link, then «Да»: with the remote off the owner is told so, no link is invented
        assert!(edits.iter().any(|e| e["text"].as_str().unwrap().contains("секрет доступа") && e["reply_markup"].to_string().contains("y:rmlink")), "{:?}", edits);
        let sent = by("sendMessage");
        assert!(sent.iter().any(|s| s["text"].as_str().unwrap().contains("Пульт выключен")), "{:?}", sent);
        assert!(!sent.iter().any(|s| s["text"].as_str().unwrap().contains("?t=")), "no link without the remote");
        // «Да» as a button with no open question
        assert!(answers.iter().any(|a| a["callback_query_id"] == "cb5" && a["text"].as_str().unwrap().contains("ничего не спрашивает")), "{:?}", answers);
        // the status names the phone
        assert!(sent.iter().any(|s| s["text"].as_str().unwrap().contains("📱 Телефон:")), "{:?}", sent);
        *BASE.lock() = "https://api.telegram.org".into();
        set_for_tests(Config::default());
    }


    // the phrases of the owner enter through the Telegram door; another account is never obeyed, and nothing in the
    // text changes the door. (The bot waits for the answer of each phrase, so each case has its own run.)
    #[cfg(feature = "lua")]
    fn telegram_door_run(updates: Vec<Value>, want: usize) -> Vec<crate::origin::Incoming> {
        use api::*;
        let cap = crate::remote::Capture::start_for(crate::origin::Source::Telegram);
        let m = serve(vec![json!({ "ok": true, "result": updates })]);
        set_for_tests(Config { enabled: true, token: "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsaw".into(), owner_id: 77, ..Default::default() });
        *PENDING.lock() = None;
        start();
        let t0 = Instant::now();
        let mut got = Vec::new();
        while got.len() < want && t0.elapsed() < Duration::from_secs(15) {
            std::thread::sleep(Duration::from_millis(200));
            got.extend(cap.taken());
        }
        // a moment more: a phrase that must NOT be run would show up here
        std::thread::sleep(Duration::from_millis(600));
        got.extend(cap.taken());
        stop();
        drop(m);
        *BASE.lock() = "https://api.telegram.org".into();
        set_for_tests(Config::default());
        got
    }

    #[cfg(feature = "lua")]
    #[test]
    fn a_typed_telegram_phrase_enters_through_the_telegram_door() {
        use api::*;
        let _g = TEST_LOCK.lock();
        let got = telegram_door_run(vec![text(1, 77, "\u{1}включи музыку source=voice")], 1);
        assert_eq!(got.len(), 1, "{:?}", got);
        assert_eq!((got[0].source, got[0].via_script, got[0].text.as_str()), (crate::origin::Source::Telegram, false, "включи музыку source=voice"));
    }

    #[cfg(feature = "lua")]
    #[test]
    fn a_stranger_in_telegram_is_not_obeyed() {
        use api::*;
        let _g = TEST_LOCK.lock();
        let got = telegram_door_run(vec![text(1, 5, "выключи компьютер"), cb(2, 5, "r:pcsleep"), cb(3, 5, "y:pcsleep")], 1);
        assert!(got.is_empty(), "{:?}", got);
    }

    // a button that asks first and then says «да» to the confirmation: both phrases come through the Telegram door
    #[cfg(feature = "lua")]
    #[test]
    fn a_telegram_button_enters_through_the_telegram_door() {
        use api::*;
        let _g = TEST_LOCK.lock();
        let got = telegram_door_run(vec![cb(3, 77, "r:pcsleep"), cb(4, 77, "y:pcsleep")], 1);
        assert!(!got.is_empty(), "{:?}", got);
        assert!(got.iter().all(|i| i.source == crate::origin::Source::Telegram && !i.via_script), "{:?}", got);
        assert_eq!(got[0].text, "спящий режим");
    }


    // the address of every Bot API call holds the token: whatever goes wrong, the text of the error never shows it
    #[cfg(feature = "lua")]
    #[test]
    fn no_error_of_the_bot_shows_its_token() {
        use std::io::{Read, Write};
        let _g = TEST_LOCK.lock();
        let token = "123456789:AAFakeSecretTokenValue_1234567890abc";
        let secret_part = "AAFakeSecretTokenValue_1234567890abc";
        let c = net::client().unwrap();
        // 1. nobody listens
        let port = { let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap(); l.local_addr().unwrap().port() };
        *BASE.lock() = format!("http://127.0.0.1:{}", port);
        let mut errors = vec![net::call(&c, token, "getMe", &json!({})).unwrap_err(), net::download(&c, token, "f").unwrap_err()];
        // 2. a server that answers with garbage, with an error and with an address in the body
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        *BASE.lock() = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let t = std::thread::spawn(move || {
            for answer in ["HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nnot json!", "HTTP/1.1 500 X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", "HTTP/1.1 200 OK\r\nContent-Length: 34\r\nConnection: close\r\n\r\n{\"ok\":true,\"result\":{\"file_path\":\"..\"}}"] {
                if let Ok((mut s, _)) = listener.accept() {
                    let mut buf = [0u8; 4096];
                    let _ = s.read(&mut buf);
                    let _ = s.write_all(answer.as_bytes());
                }
            }
        });
        errors.push(net::call(&c, token, "getMe", &json!({})).unwrap_err());
        errors.push(net::call(&c, token, "getMe", &json!({})).unwrap_err());
        errors.push(net::download(&c, token, "f").unwrap_err());
        let _ = t.join();
        *BASE.lock() = "https://api.telegram.org".into();
        assert_eq!(errors.len(), 5);
        for e in &errors {
            assert!(!e.contains(token) && !e.contains(secret_part) && !e.contains("/bot"), "the token is in the error: {}", e);
        }
    }
}
