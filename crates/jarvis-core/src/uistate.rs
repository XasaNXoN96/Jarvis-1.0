// State behind the Jarvis window: action log, computer state, settings of the settings center,
// switched-off command packs. Everything is stored in the config dir.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::APP_CONFIG_DIR;

// ---------------------------------------------------------------------------------------------
// action log
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub time: u64,
    pub text: String,
    pub command: String,
    pub ok: bool,
}

static LOG: Lazy<Mutex<VecDeque<LogEntry>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn log_action(text: &str, command: &str, ok: bool) {
    let mut log = LOG.lock();
    log.push_front(LogEntry { time: now(), text: text.chars().take(120).collect(), command: command.to_string(), ok });
    log.truncate(60);
}

// ---------------------------------------------------------------------------------------------
// events of the program (the "Command Output" card): what really happened, newest first. No secrets are ever put here.
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Serialize)]
pub struct Event {
    pub id: u64,
    pub time: u64,
    pub kind: String, // start | command | ai | voice | error | info
    pub text: String,
}

static EVENTS: Lazy<Mutex<VecDeque<Event>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static EVENT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn log_event(kind: &str, text: &str) {
    let mut ev = EVENTS.lock();
    ev.push_front(Event { id: EVENT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst), time: now(), kind: kind.to_string(), text: text.chars().take(200).collect() });
    ev.truncate(80);
}

pub fn recent_events(n: usize) -> Vec<Event> {
    EVENTS.lock().iter().take(n).cloned().collect()
}

pub fn recent_actions() -> Vec<LogEntry> {
    LOG.lock().iter().cloned().collect()
}

// ---------------------------------------------------------------------------------------------
// settings of the settings center
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub voice_rate: f32,     // 0.6 .. 1.6, 1.0 = normal
    pub reaction_voice: String, // recorded reaction pack (jarvis-og / jarvis-remaster / jarvis-howdy), "" = app default
    pub city: String,        // the city of the weather block ("" = not chosen: nothing is guessed)
    pub voice_name: String,  // "" = automatic (Piper, else a female Windows voice)
    pub ai_provider: String, // none | openai | openrouter | gemini | anthropic | ollama | custom
    pub ai_model: String,
    pub ai_key: String,
    pub ai_url: String,
    pub ai_route: String,    // OpenRouter: the order of the hosts of the model ("" = OpenRouter chooses), see ai::route_provider
    pub ai_fallback: bool, // phrases that are not commands go to the AI (off by default: nothing leaves the PC unasked)
    // privacy: the AI may be switched off as a whole (no request leaves the computer, local commands keep working)
    pub ai_enabled: bool,
    // privacy: the microphone switched off by the user stays off after a restart
    pub mic_off: bool,
    // notifications of Jarvis (timers, reminders, finished tasks, errors)
    pub notify_show: bool,
    pub notify_speak: bool,
    pub notify_sound: bool,
    // "do not disturb": no popups and no sound; critical errors are still kept in the window, silently
    pub dnd: bool,
    // notifications of other Windows programs: off by default, only with the permission of Windows
    pub ext_enabled: bool,
    pub ext_show: bool,
    pub ext_speak: bool,
    pub ext_strict: bool,       // only the programs of the allowed list
    pub ext_allowed: Vec<String>,
    pub ext_blocked: Vec<String>,
    pub ext_seen: Vec<String>,  // names of programs that sent a notification (never their text): to pick from
    // social networks (social.rs) whose notifications are let through, also from a browser, and those read aloud
    pub ext_social: Vec<String>,
    pub ext_social_speak: Vec<String>,
    // interface choices of the window (sphere mode, photo settings): the page keeps a copy in its browser storage
    pub ui: std::collections::BTreeMap<String, String>,
    // loudness of Jarvis' voice, 0..100 (the Windows volume is not touched)
    pub tts_volume: u8,
    // the voice of Jarvis (Piper) stays loaded in memory: the speech starts sooner, about 100 MB of memory
    pub tts_warm: bool,
    // "Анализ экрана": a picture of the screen may be sent to the AI provider, only by the command «что на экране»
    pub ai_screen: bool,
    // the browser for sites opened by voice: "" = the Windows default | chrome | edge | firefox | opera | brave | yandex
    pub browser: String,
    // how easily the microphone takes a sound for speech, 0..100 (50 = as before)
    pub mic_sensitivity: u8,
}

// the speech threshold of the microphone for a sensitivity: 50 % = 100 (the old fixed value), 100 % = 30 (hears a
// quiet voice), 0 % = 300 (only loud speech). Pure, tested.
pub fn vad_threshold(sensitivity: u8) -> f32 {
    let s = sensitivity.min(100) as f32;
    if s >= 50.0 { 100.0 - (s - 50.0) / 50.0 * 70.0 } else { 300.0 - s / 50.0 * 200.0 }
}

static MIC_THRESHOLD: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x42C8_0000); // 100.0

// read for every microphone frame: no lock
pub fn mic_threshold() -> f32 {
    f32::from_bits(MIC_THRESHOLD.load(std::sync::atomic::Ordering::Relaxed))
}

fn apply_mic_threshold(s: &UiSettings) {
    MIC_THRESHOLD.store(vad_threshold(s.mic_sensitivity).to_bits(), std::sync::atomic::Ordering::Relaxed);
}

// the program of a browser choice (an App Paths name Windows knows)
pub fn browser_exe(key: &str) -> Option<&'static str> {
    match key {
        "chrome" => Some("chrome.exe"),
        "edge" => Some("msedge.exe"),
        "firefox" => Some("firefox.exe"),
        "opera" => Some("opera.exe"),
        "brave" => Some("brave.exe"),
        "yandex" => Some("browser.exe"),
        _ => None,
    }
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            voice_rate: 1.0, voice_name: String::new(), city: String::new(), reaction_voice: String::new(), ai_provider: "none".into(), ai_model: String::new(), ai_key: String::new(), ai_url: String::new(), ai_route: String::new(), ai_fallback: false,
            ai_enabled: true, mic_off: false,
            notify_show: true, notify_speak: true, notify_sound: true, dnd: false,
            ext_enabled: false, ext_show: true, ext_speak: false, ext_strict: true, ext_allowed: Vec::new(), ext_blocked: Vec::new(), ext_seen: Vec::new(),
            ext_social: Vec::new(), ext_social_speak: Vec::new(),
            ui: Default::default(),
            tts_volume: 100, tts_warm: true, browser: String::new(), mic_sensitivity: 50, ai_screen: false,
        }
    }
}

static SETTINGS: Lazy<Mutex<UiSettings>> = Lazy::new(|| Mutex::new(load_settings()));
// a settings file that could not be read: what happened (shown once as an important notification)
static LOAD_PROBLEM: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

fn file(name: &str) -> Option<PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join(name))
}

// An unreadable file is never overwritten silently: it is kept next to the new one (".broken-<time>") and the user is told.
pub fn parse_or_keep<T: serde::de::DeserializeOwned + Default>(path: &std::path::Path, what: &str) -> (T, Option<String>) {
    let text = match std::fs::read(path) {
        Ok(t) => t,
        Err(_) => return (T::default(), None), // no file yet: the defaults, nothing to report
    };
    match serde_json::from_slice::<T>(&text) {
        Ok(v) => (v, None),
        Err(e) => {
            let backup = path.with_extension(format!("broken-{}.json", now()));
            let kept = std::fs::rename(path, &backup).is_ok();
            warn!("Settings: cannot read {} ({}), defaults are used", path.display(), e);
            (T::default(), Some(format!(
                "Не удалось прочитать {} — используются настройки по умолчанию.{}",
                what,
                if kept { format!(" Старый файл сохранён: {}", backup.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()) } else { String::new() }
            )))
        }
    }
}

fn load_settings() -> UiSettings {
    match file("ui_settings.json") {
        Some(p) => load_from(&p),
        None => UiSettings::default(),
    }
}

// the settings as the next start reads them from the file
pub fn load_from(path: &std::path::Path) -> UiSettings {
    let (mut s, problem): (UiSettings, Option<String>) = parse_or_keep(path, "файл настроек окна (ui_settings.json)");
    if let Some(p) = problem {
        add_load_problem(&p);
    }
    // the key is stored encrypted (DPAPI); an old plain key is encrypted at the next save
    let plain_on_disk = !s.ai_key.is_empty() && !s.ai_key.starts_with("dpapi:");
    s.ai_key = crate::secret::open(&s.ai_key);
    if plain_on_disk && cfg!(target_os = "windows") {
        write_settings(&s);
    }
    s
}

pub fn add_load_problem(text: &str) {
    let mut p = LOAD_PROBLEM.lock();
    if !p.is_empty() {
        p.push(' ');
    }
    p.push_str(text);
}

pub fn take_load_problem() -> String {
    std::mem::take(&mut *LOAD_PROBLEM.lock())
}

// the settings file: the key is encrypted. Written to a temporary file first, so a crash never leaves half a file.
fn write_settings(s: &UiSettings) {
    if let Some(p) = file("ui_settings.json") {
        write_to(&p, s);
    }
}

pub fn write_to(p: &std::path::Path, s: &UiSettings) {
    let mut v = serde_json::to_value(s).unwrap_or(Value::Null);
    v["ai_key"] = Value::String(crate::secret::seal(&s.ai_key));
    if let Ok(json) = serde_json::to_string_pretty(&v) {
        // (atomic, never through a link planted under the temporary name: lib.rs write_atomic)
        let _ = crate::write_atomic(p, json);
    }
}

// called once at start: restores the saved privacy choices (microphone) and the notification watcher
pub fn init() {
    let s = settings();
    apply_mic_threshold(&s);
    if s.mic_off {
        crate::MIC_MUTED.store(true, std::sync::atomic::Ordering::SeqCst);
        info!("Privacy: the microphone is switched off (saved setting)");
    }
    let problem = take_load_problem();
    if !problem.is_empty() {
        crate::notify::push("important", "Настройки восстановлены", &problem, false);
    }
}

pub fn settings() -> UiSettings {
    SETTINGS.lock().clone()
}

// tests that change the shared settings run one at a time and put them back
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
pub fn set_for_tests(s: UiSettings) {
    *SETTINGS.lock() = s;
}

pub fn voice_rate() -> f32 {
    SETTINGS.lock().voice_rate.clamp(0.5, 2.0)
}

pub fn voice_name() -> String {
    SETTINGS.lock().voice_name.clone()
}

// the microphone switch (window, tray, voice command, privacy section): one place, and it is remembered
pub fn set_mic_muted(muted: bool) {
    crate::MIC_MUTED.store(muted, std::sync::atomic::Ordering::SeqCst);
    let mut s = SETTINGS.lock();
    if s.mic_off != muted {
        s.mic_off = muted;
        write_settings(&s);
    }
}

// the name of a program that sent a notification (only the name) for the allowed list in the settings
pub fn remember_app(name: &str) {
    let name: String = name.trim().chars().take(80).collect();
    if name.is_empty() {
        return;
    }
    let mut s = SETTINGS.lock();
    if !s.ext_seen.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
        s.ext_seen.push(name);
        if s.ext_seen.len() > 40 {
            s.ext_seen.remove(0);
        }
        write_settings(&s);
    }
}

// "reset the settings": everything back to the defaults. The conversations, the timers and the AI key are kept
// (the key has its own "forget" button); the speech and the microphone are switched back on.
pub fn reset_settings() {
    let mut s = SETTINGS.lock();
    let key = std::mem::take(&mut s.ai_key);
    *s = UiSettings { ai_key: key, ..UiSettings::default() };
    write_settings(&s);
    drop(s);
    crate::MIC_MUTED.store(false, std::sync::atomic::Ordering::SeqCst);
    crate::tts::set_enabled(true);
    crate::ai::status_reset();
    crate::notify::ext_apply();
    log_event("info", "Настройки сброшены к значениям по умолчанию");
}

pub fn forget_ai_key() {
    let mut s = SETTINGS.lock();
    s.ai_key.clear();
    write_settings(&s);
    drop(s);
    crate::ai::status_reset();
    log_event("info", "Ключ ИИ удалён с этого компьютера");
}

fn str_list(v: &Value) -> Option<Vec<String>> {
    v.as_array().map(|a| {
        let mut out: Vec<String> = Vec::new();
        for x in a.iter().filter_map(|x| x.as_str()) {
            let n: String = x.trim().chars().take(80).collect();
            if !n.is_empty() && !out.iter().any(|o| o.eq_ignore_ascii_case(&n)) {
                out.push(n);
            }
        }
        out.truncate(60);
        out
    })
}

// merge the fields present in `patch` into the settings and save
pub fn update_settings(patch: &Value) {
    let mut s = SETTINGS.lock();
    if let Some(v) = patch.get("voice_rate").and_then(|v| v.as_f64()) { s.voice_rate = (v as f32).clamp(0.5, 2.0); }
    if let Some(v) = patch.get("reaction_voice").and_then(|v| v.as_str()) {
        s.reaction_voice = v.chars().take(60).collect();
        if crate::voices::get_voice(&s.reaction_voice).is_some() { crate::voices::set_current_voice(&s.reaction_voice); }
    }
    if let Some(v) = patch.get("city").and_then(|v| v.as_str()) {
        let city: String = v.trim().chars().take(80).collect();
        if city != s.city {
            s.city = city;
            crate::weather::city_changed();
        }
    }
    if let Some(v) = patch.get("voice_name").and_then(|v| v.as_str()) { s.voice_name = v.chars().take(120).collect(); }
    let before = (s.ai_provider.clone(), s.ai_model.clone(), s.ai_key.clone(), s.ai_url.clone(), s.ai_route.clone());
    if let Some(v) = patch.get("ai_provider").and_then(|v| v.as_str()) { s.ai_provider = v.chars().take(30).collect(); }
    if let Some(v) = patch.get("ai_model").and_then(|v| v.as_str()) { s.ai_model = v.chars().take(100).collect(); }
    if let Some(v) = patch.get("ai_key").and_then(|v| v.as_str()) { s.ai_key = v.chars().take(300).collect(); }
    if let Some(v) = patch.get("ai_url").and_then(|v| v.as_str()) { s.ai_url = v.chars().take(300).collect(); }
    if let Some(v) = patch.get("ai_route").and_then(|v| v.as_str()) { s.ai_route = v.chars().take(200).collect(); }
    if before != (s.ai_provider.clone(), s.ai_model.clone(), s.ai_key.clone(), s.ai_url.clone(), s.ai_route.clone()) {
        crate::ai::status_reset();
    }
    if let Some(v) = patch.get("ai_fallback").and_then(|v| v.as_bool()) { s.ai_fallback = v; }
    if let Some(v) = patch.get("ai_enabled").and_then(|v| v.as_bool()) {
        if v != s.ai_enabled {
            s.ai_enabled = v;
            crate::ai::status_reset();
            if !v {
                // what is being generated right now stops too: nothing more goes to the provider
                if let Some(id) = crate::ai::store().active() { crate::ai::stop(&id); }
            }
        }
    }
    let flag = |k: &str| patch.get(k).and_then(|v| v.as_bool());
    if let Some(v) = flag("notify_show") { s.notify_show = v; }
    if let Some(v) = flag("notify_speak") { s.notify_speak = v; }
    if let Some(v) = flag("notify_sound") { s.notify_sound = v; }
    if let Some(v) = flag("dnd") { s.dnd = v; }
    if let Some(v) = flag("ext_show") { s.ext_show = v; }
    if let Some(v) = flag("ext_speak") { s.ext_speak = v; }
    if let Some(v) = flag("ext_strict") { s.ext_strict = v; }
    if let Some(v) = patch.get("ext_allowed").and_then(str_list) { s.ext_allowed = v; }
    if let Some(v) = patch.get("ext_blocked").and_then(str_list) { s.ext_blocked = v; }
    // only the keys of known networks are kept
    let nets = |v: Vec<String>| -> Vec<String> { crate::social::NETWORKS.iter().filter(|n| v.iter().any(|x| x == n.key)).map(|n| n.key.to_string()).collect() };
    if let Some(v) = patch.get("ext_social").and_then(str_list) { s.ext_social = nets(v); }
    if let Some(v) = patch.get("ext_social_speak").and_then(str_list) { s.ext_social_speak = nets(v); }
    if let Some(v) = flag("ai_screen") { s.ai_screen = v; }
    if let Some(v) = patch.get("tts_volume").and_then(|v| v.as_u64()) { s.tts_volume = v.min(100) as u8; }
    if let Some(v) = patch.get("tts_warm").and_then(|v| v.as_bool()) {
        if v != s.tts_warm {
            s.tts_warm = v;
            if v { crate::tts::warm_up(); } else { crate::tts::warm_stop(); }
        }
    }
    if let Some(v) = patch.get("mic_sensitivity").and_then(|v| v.as_u64()) { s.mic_sensitivity = v.min(100) as u8; apply_mic_threshold(&s); }
    if let Some(v) = patch.get("browser").and_then(|v| v.as_str()) { s.browser = if browser_exe(v).is_some() { v.to_string() } else { String::new() }; }
    let ext_changed = match flag("ext_enabled") { Some(v) if v != s.ext_enabled => { s.ext_enabled = v; true } _ => false };
    if let Some(obj) = patch.get("ui").and_then(|v| v.as_object()) {
        for (k, v) in obj.iter().take(30) {
            let k: String = k.chars().take(40).collect();
            match v.as_str() {
                Some(val) => { s.ui.insert(k, val.chars().take(200).collect()); }
                None => { s.ui.remove(&k); }
            }
        }
    }
    write_settings(&s);
    drop(s);
    if ext_changed {
        crate::notify::ext_apply();
    }
}

// the settings page never gets the key back, only whether one is saved
pub fn settings_json() -> Value {
    let s = settings();
    json!({
        "voice_rate": s.voice_rate, "voice_name": s.voice_name, "city": s.city, "reaction_voice": if s.reaction_voice.is_empty() { crate::voices::get_current_voice().map(|v| v.voice.id.clone()).unwrap_or_default() } else { s.reaction_voice.clone() },
        "ai_provider": s.ai_provider, "ai_model": s.ai_model, "ai_url": s.ai_url, "ai_route": s.ai_route,
        "ai_key_saved": !s.ai_key.is_empty(),
        "ai_fallback": s.ai_fallback,
        "ai_enabled": s.ai_enabled,
        "mic_off": crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst),
        "notify_show": s.notify_show, "notify_speak": s.notify_speak, "notify_sound": s.notify_sound, "dnd": s.dnd,
        "ext_enabled": s.ext_enabled, "ext_show": s.ext_show, "ext_speak": s.ext_speak, "ext_strict": s.ext_strict,
        "ext_allowed": s.ext_allowed, "ext_blocked": s.ext_blocked, "ext_seen": s.ext_seen,
        "ext_social": s.ext_social, "ext_social_speak": s.ext_social_speak,
        "social_networks": crate::social::NETWORKS.iter().map(|n| json!({ "key": n.key, "title": n.title })).collect::<Vec<_>>(),
        "ext_status": crate::notify::ext_status_json(),
        "ui": s.ui,
        "ai_screen": s.ai_screen,
        "tts_volume": s.tts_volume, "tts_warm": s.tts_warm, "browser": s.browser, "mic_sensitivity": s.mic_sensitivity,
        "autostart": crate::autostart::state(),
        "engines": crate::DB.get().map(|d| { let d = d.read(); json!({ "wake": format!("{:?}", d.wake_word_engine), "stt": format!("{:?}", d.speech_to_text_engine), "intent": d.intent_backend, "slots": d.slots_backend }) }).unwrap_or(Value::Null),
        "log_dir": crate::APP_LOG_DIR.get().map(|d| d.display().to_string()).unwrap_or_default(),
        "internet": crate::diag::internet_uses(&s),
        "history": crate::ai::store().stats(),
        "tts_enabled": crate::tts::is_enabled(),
        "piper": crate::tts::piper_available(),
    })
}

// ---------------------------------------------------------------------------------------------
// command sections ("groups"): the commands page and the panel show commands by topic, and a whole
// section can be switched off
// ---------------------------------------------------------------------------------------------
// (key, emoji, title, short description). The keys of the sections that existed before are kept: the sections a user
// switched off stay off. The order follows the catalogue of the reference: browser, keyboard and mouse, media, sound ...
pub const GROUPS: &[(&str, &str, &str, &str)] = &[
    ("browser", "🌐", "Браузер, сайты и поиск", "открыть сайт, вкладки, назад/вперёд, история, закладки, поиск в интернете, Википедия, перевод, карты"),
    ("input", "⌨", "Клавиатура и мышь", "выделить, копировать, вставить, отменить, печать, клик, курсор, прокрутка"),
    ("media", "🎬", "Мультимедиа и экран", "пауза, следующий трек, снимок экрана"),
    ("sound", "🔊", "Звук и яркость", "громкость, без звука, яркость экрана"),
    ("timers", "⏱", "Таймеры и напоминания", "таймер, напоминание, будильник, «остановись через…»"),
    ("voice", "🎤", "Голос и ИИ", "микрофон, озвучка, чтение выделенного и буфера, проверка голоса"),
    ("apps", "🚀", "Приложения", "открыть и закрыть программу, калькулятор, Steam, обновить список программ"),
    ("files", "📁", "Файлы и папки", "найти файл по имени, типу, дате и тексту, открыть папку, диск, настройки Windows"),
    ("text", "📝", "Тексты и документы", "диктовка в Word и блокнот, таблицы Excel, новый документ, заметки"),
    ("windows", "🪟", "Окна", "свернуть, развернуть, переключить окно, раскладка"),
    ("power", "⏻", "Питание ПК", "выключить, перезагрузить, спящий режим, заблокировать"),
    ("system", "🖥", "Системные инструменты", "диспетчер задач, корзина"),
    ("info", "📊", "Информация", "время, дата, батарея, память, процессор, диск, IP, погода"),
    ("games", "🎮", "Игры", "игровой режим и таймер игры"),
    ("remote", "📱", "Пульт для телефона", "управление с телефона, найти телефон, ссылка и текст на телефон"),
    ("talk", "💬", "Разговор и помощь", "привет, «что ты умеешь», диагностика, шутки"),
    ("control", "⛔", "Управление Jarvis", "стоп, выключить Jarvis (отключить нельзя)"),
];

// categories of the reference that Jarvis cannot do (shown honestly as unavailable, with the reason; no commands)
pub const UNAVAILABLE_GROUPS: &[(&str, &str, &str, &str)] = &[
    ("gestures", "✋", "Жесты", "Недоступно: нужна камера и модель распознавания рук, которой в Jarvis нет. Команды жестами не выполняются."),
    ("lighting", "💡", "Подсветка и умный дом", "Недоступно: нет поддержки ламп, лент и розеток (нужна интеграция с конкретным устройством)."),
];

// What the catalog says about a command beyond "it runs": works only under a condition, cannot be done, or the
// same action as a newer command (its phrases go to that one; the old command is kept, nothing is removed).
// (status, note): status is "partial" | "unavailable" | "duplicate"; None = works as described.
pub fn command_status(id: &str) -> Option<(&'static str, &'static str)> {
    Some(match id {
        "assistant_unlock" => ("unavailable", "Windows не даёт разблокировать компьютер программой — Джарвис объясняет, как войти (пароль, PIN, Windows Hello)."),
        "assistant_phone_find" | "assistant_phone_stop" | "assistant_phone_link" | "assistant_phone_text" =>
            ("partial", "Работает, когда пульт включён и его страница открыта на телефоне."),
        "assistant_screen" => ("partial", "Нужен ИИ-провайдер с моделью, которая видит картинки, и разрешение «Анализ экрана» в настройках."),
        "assistant_chat_start" | "assistant_chat_end" => ("partial", "Нужен настроенный и включённый ИИ-провайдер."),
        "assistant_office_dictate" | "assistant_office_cell" | "assistant_office_new" | "assistant_object_start" =>
            ("partial", "Word и Excel должны быть установлены; Блокнот работает всегда."),
        "assistant_brightness_set" | "assistant_brightness_up" | "assistant_brightness_down" =>
            ("partial", "Яркость меняется на экране ноутбука; внешние мониторы Windows обычно не позволяют."),
        "assistant_info_battery" => ("partial", "Заряд есть только у ноутбука или планшета."),
        "open_google" => ("duplicate", "То же действие выполняет «открой сайт» (каталог сайтов): Google открывается так же."),
        "calculator_1" => ("duplicate", "Фразы обрабатывает «открой программу» — калькулятор запускается так же."),
        "calculator_2" => ("duplicate", "«Закрой калькулятор» обрабатывает «закрой программу»."),
        "windows_3" => ("duplicate", "Фразы обрабатывает «открой программу» — Диспетчер задач запускается так же."),
        "steam_1" => ("duplicate", "«Открой Steam» обрабатывает «открой программу», «игровой режим» — команда игрового режима."),
        "steam_2" => ("duplicate", "«Рабочий режим» обрабатывает команда игрового режима, «закрой Steam» — «закрой программу»."),
        "jarvis_5" => ("duplicate", "Те же фразы благодарности есть у соседней команды ответа на «спасибо»."),
        _ => return None,
    })
}

// the texts of the close dialog; the note says honestly whether the wake word will be listened to in the tray
pub fn close_dialog_json() -> serde_json::Value {
    let t = crate::i18n::t;
    let listening = crate::VOICE_ACTIVE.load(std::sync::atomic::Ordering::SeqCst) && !crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst);
    serde_json::json!({
        "title": t("close-dialog-title"),
        "tray": t("close-dialog-tray"),
        "exit": t("close-dialog-exit"),
        "cancel": t("close-dialog-cancel"),
        "open": t("close-dialog-open"),
        "browser_tray": t("close-dialog-browser-tray"),
        "exited": t("close-dialog-exited"),
        "note": t(if listening { "close-dialog-listening" } else { "close-dialog-not-listening" }),
    })
}

pub fn group_key(pack: &str, id: &str) -> &'static str {
    let id = id.strip_prefix("assistant_").unwrap_or(id);
    let starts = |p: &str| id.starts_with(p);
    if starts("web_") || pack == "browser" || id == "website" || id == "translate_clip" || id == "browser_set" || id == "speedtest" || id == "wiki" || id == "translate" || id == "maps" || id == "music_search" { return "browser"; }
    if starts("key_") || starts("mouse_") { return "input"; }
    if starts("media") || id == "windows_4" { return "media"; }
    if starts("volume") || starts("brightness") { return "sound"; }
    if starts("timer") || id == "reminder" { return "timers"; }
    if starts("win_") || id == "windows_1" || id == "windows_8" { return "windows"; }
    if starts("power") || id == "unlock" || id == "windows_5" || id == "windows_6" { return "power"; }
    if starts("file") || id == "place_open" || id == "disk_open" { return "files"; }
    if starts("note") || starts("task") || starts("office") || starts("object") || id == "session" { return "text"; }
    if starts("tts") || starts("read_") || starts("mic_") || starts("chat") || id == "screen" { return "voice"; }
    if pack == "windows" { return "system"; }
    if starts("info") || starts("sys_") || pack == "weather" { return "info"; }
    if starts("game") || pack == "steam" { return "games"; }
    if starts("remote") || starts("phone_") { return "remote"; }
    if pack == "stop" || pack == "terminate" { return "control"; }
    if starts("program") || starts("apps_") || pack == "calculator" { return "apps"; }
    "talk"
}

static DISABLED: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| {
    let set: HashSet<String> = file("disabled_groups.json")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
        .into_iter()
        .collect();
    Mutex::new(set)
});

pub fn is_group_disabled(group: &str) -> bool {
    group != "control" && DISABLED.lock().contains(group)
}

pub fn set_group_disabled(group: &str, disabled: bool) {
    if group == "control" {
        return;
    }
    let mut set = DISABLED.lock();
    if disabled { set.insert(group.to_string()); } else { set.remove(group); }
    if let Some(p) = file("disabled_groups.json") {
        let list: Vec<&String> = set.iter().collect();
        let _ = crate::write_atomic(&p, serde_json::to_string(&list).unwrap_or_default());
    }
}

pub fn group_title(key: &str) -> String {
    GROUPS.iter().find(|g| g.0 == key).map(|g| g.2.to_string()).unwrap_or_default()
}

// ---------------------------------------------------------------------------------------------
// computer state
// ---------------------------------------------------------------------------------------------
pub struct Stats {
    pub cpu: f32,        // percent
    pub mem_used_gb: f32,
    pub mem_total_gb: f32,
    pub disk_free_gb: f32,
    pub disk_total_gb: f32,
}

#[cfg(target_os = "windows")]
pub fn stats() -> Stats {
    use std::sync::atomic::{AtomicU64, Ordering};
    use winapi::shared::minwindef::FILETIME;
    use winapi::um::fileapi::GetDiskFreeSpaceExW;
    use winapi::um::sysinfoapi::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use winapi::um::processthreadsapi::GetSystemTimes;
    use winapi::shared::ntdef::ULARGE_INTEGER;

    static PREV_IDLE: AtomicU64 = AtomicU64::new(0);
    static PREV_TOTAL: AtomicU64 = AtomicU64::new(0);
    let gb = |b: u64| b as f32 / 1024.0 / 1024.0 / 1024.0;

    unsafe {
        let mut mem: MEMORYSTATUSEX = std::mem::zeroed();
        mem.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        GlobalMemoryStatusEx(&mut mem);

        let ft = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
        let (mut idle, mut kernel, mut user): (FILETIME, FILETIME, FILETIME) = std::mem::zeroed();
        GetSystemTimes(&mut idle, &mut kernel, &mut user);
        let (i, total) = (ft(idle), ft(kernel) + ft(user)); // kernel time includes idle time
        let (pi, pt) = (PREV_IDLE.swap(i, Ordering::SeqCst), PREV_TOTAL.swap(total, Ordering::SeqCst));
        let cpu = if pt > 0 && total > pt { (1.0 - (i - pi) as f32 / (total - pt) as f32) * 100.0 } else { 0.0 };

        let root: Vec<u16> = "C:\\\0".encode_utf16().collect();
        let (mut free, mut tot, mut _avail): (ULARGE_INTEGER, ULARGE_INTEGER, ULARGE_INTEGER) = std::mem::zeroed();
        GetDiskFreeSpaceExW(root.as_ptr(), &mut _avail, &mut tot, &mut free);

        Stats {
            cpu: cpu.clamp(0.0, 100.0),
            mem_used_gb: gb(mem.ullTotalPhys - mem.ullAvailPhys),
            mem_total_gb: gb(mem.ullTotalPhys),
            disk_free_gb: gb(*free.QuadPart()),
            disk_total_gb: gb(*tot.QuadPart()),
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn stats() -> Stats {
    let mut total = 0f32;
    let mut avail = 0f32;
    if let Ok(s) = std::fs::read_to_string("/proc/meminfo") {
        for line in s.lines() {
            let kb = |l: &str| l.split_whitespace().nth(1).and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
            if line.starts_with("MemTotal:") { total = kb(line); }
            if line.starts_with("MemAvailable:") { avail = kb(line); }
        }
    }
    let gb = |kb: f32| kb / 1024.0 / 1024.0;
    // processor load from /proc/stat between two calls
    use std::sync::atomic::{AtomicU64, Ordering};
    static PREV_IDLE: AtomicU64 = AtomicU64::new(0);
    static PREV_TOTAL: AtomicU64 = AtomicU64::new(0);
    let mut cpu = 0.0f32;
    if let Ok(s) = std::fs::read_to_string("/proc/stat") {
        if let Some(line) = s.lines().next() {
            let n: Vec<u64> = line.split_whitespace().skip(1).filter_map(|v| v.parse().ok()).collect();
            if n.len() >= 5 {
                let idle = n[3] + n[4];
                let total: u64 = n.iter().sum();
                let (pi, pt) = (PREV_IDLE.swap(idle, Ordering::SeqCst), PREV_TOTAL.swap(total, Ordering::SeqCst));
                if pt > 0 && total > pt {
                    cpu = (1.0 - (idle - pi) as f32 / (total - pt) as f32) * 100.0;
                }
            }
        }
    }
    // the system disk (df)
    let (mut free, mut size) = (0f32, 0f32);
    if let Ok(out) = std::process::Command::new("df").args(["-Pk", "/"]).output() {
        if let Some(line) = String::from_utf8_lossy(&out.stdout).lines().nth(1) {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() >= 4 {
                size = f[1].parse().unwrap_or(0.0);
                free = f[3].parse().unwrap_or(0.0);
            }
        }
    }
    Stats { cpu: cpu.clamp(0.0, 100.0), mem_used_gb: gb(total - avail), mem_total_gb: gb(total), disk_free_gb: gb(free), disk_total_gb: gb(size) }
}

pub fn state_json() -> Value {
    let st = stats();
    #[cfg(feature = "lua")]
    let timers: Vec<Value> = crate::lua::timers_json();
    #[cfg(not(feature = "lua"))]
    let timers: Vec<Value> = Vec::new();
    let actions: Vec<Value> = recent_actions()
        .into_iter()
        .take(12)
        .map(|e| json!({ "ago": now().saturating_sub(e.time), "text": e.text, "command": e.command, "ok": e.ok }))
        .collect();
    json!({
        "cpu": st.cpu, "mem_used": st.mem_used_gb, "mem_total": st.mem_total_gb,
        "disk_free": st.disk_free_gb, "disk_total": st.disk_total_gb,
        "timers": timers, "actions": actions,
        "messages_last": crate::social::last_id(),
        "session": crate::session::kind().unwrap_or_default(),
        "session_data": crate::session::data(),
        // the object Jarvis works with and the state of the work (only facts of the state machine, nothing invented)
        "object": crate::objects::snapshot(),
        "mic_muted": crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst),
        "voice_active": crate::VOICE_ACTIVE.load(std::sync::atomic::Ordering::SeqCst),
        "voice_error": crate::VOICE_ERROR.lock().clone(),
        "mic_level": crate::MIC_LEVEL.load(std::sync::atomic::Ordering::Relaxed),
        "tts_enabled": crate::tts::is_enabled(),
        "remote": crate::remote::is_running(),
        "phone": crate::remote::phone_json(),
        // the question of the cross of the window, in the language of Jarvis (the own window of Jarvis asks it)
        "close_dialog": close_dialog_json(),
        "close_req": crate::ui::browser_close_counter(),
        // something is still to be said (the queue, the next sentence being prepared): the "Стоп" button stays
        "speaking": crate::tts::is_speaking(),
        "voice_state": voice_state(),
        "rev": crate::ai::rev(),
        "ai": crate::ai::status_json(),
        "active_chat": crate::ai::store().active().unwrap_or_default(),
        "piper": crate::tts::piper_available(),
        "speech": json!({ "enabled": crate::tts::is_enabled(), "piper": crate::tts::piper_available(), "windows_voice": voice_name().starts_with("win:"), "problem": crate::tts::speech_problem() }),
        "ai_busy": crate::ai::store().active().map(|id| crate::ai::is_busy(&id)).unwrap_or(false),
        "health": crate::health::json(),
        "weather": crate::weather::json(),
        "events": recent_events(30),
        "notes": crate::notify::recent(20),
        "dnd": settings().dnd,
        "ai_enabled": settings().ai_enabled,
        "last_output": crate::dialog::last_output(),
        "commands_count": crate::COMMANDS_COUNT.load(std::sync::atomic::Ordering::Relaxed),
    })
}

// The state of the voice as the window shows it. READY only when the microphone really delivers audio.
pub fn voice_state() -> Value {
    use std::sync::atomic::Ordering::SeqCst;
    let err = crate::VOICE_ERROR.lock().clone();
    // SPEAKING only while a sound is really being played (not while Piper prepares it, not when it failed)
    let (state, detail): (&str, String) = if crate::tts::is_playing() {
        ("SPEAKING", "Jarvis говорит".into())
    } else if crate::VOICE_LISTENING.load(SeqCst) {
        ("LISTENING", "слушаю команду".into())
    } else if crate::VOICE_TEXT_ONLY.load(SeqCst) {
        ("DISABLED", "включён режим «только текст»".into())
    } else if !crate::VOICE_ACTIVE.load(SeqCst) {
        ("ERROR", if err.is_empty() { "голос не запущен".to_string() } else { err })
    } else if crate::MIC_MUTED.load(SeqCst) {
        ("DISABLED", "микрофон выключен".into())
    } else {
        match crate::recorder::last_frame_age_ms() {
            Some(age) if age < 3000 => ("READY", "микрофон работает".into()),
            Some(_) => ("ERROR", if err.is_empty() { "микрофон перестал отдавать звук".to_string() } else { err }),
            None => ("ERROR", "микрофон не отдаёт звук".into()),
        }
    };
    json!({ "state": state, "detail": detail, "tts": crate::tts::is_enabled() })
}


#[cfg(test)]
mod voice_tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    // the flags are global (the settings tests switch the microphone too): one test at a time
    use super::TEST_LOCK as LOCK;

    fn reset() {
        crate::VOICE_ACTIVE.store(false, SeqCst);
        crate::VOICE_LISTENING.store(false, SeqCst);
        crate::VOICE_TEXT_ONLY.store(false, SeqCst);
        crate::MIC_MUTED.store(false, SeqCst);
        crate::VOICE_ERROR.lock().clear();
    }
    fn st() -> String {
        voice_state()["state"].as_str().unwrap().to_string()
    }

    #[test]
    fn the_voice_indicator_shows_the_real_state() {
        let _g = LOCK.lock();
        reset();
        // not started and nothing explains it: ERROR, with the reason
        assert_eq!(st(), "ERROR");
        *crate::VOICE_ERROR.lock() = "микрофон не найден".into();
        assert!(voice_state()["detail"].as_str().unwrap().contains("микрофон не найден"));
        // started on purpose without the microphone: DISABLED, not an error
        crate::VOICE_TEXT_ONLY.store(true, SeqCst);
        assert_eq!(st(), "DISABLED");
        crate::VOICE_TEXT_ONLY.store(false, SeqCst);
        // the voice runs, but the microphone has delivered no audio: never READY
        crate::VOICE_ACTIVE.store(true, SeqCst);
        crate::VOICE_ERROR.lock().clear();
        assert_ne!(st(), "READY", "READY needs real audio frames");
        assert_eq!(st(), "ERROR");
        // muted by the user
        crate::MIC_MUTED.store(true, SeqCst);
        assert_eq!(st(), "DISABLED");
        crate::MIC_MUTED.store(false, SeqCst);
        // listening for a command
        crate::VOICE_LISTENING.store(true, SeqCst);
        assert_eq!(st(), "LISTENING");
        reset();
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn settings_of_an_installed_older_version_load_with_safe_defaults() {
        // a ui_settings.json written before notifications and privacy existed
        let old = r#"{"voice_rate":1.2,"reaction_voice":"","city":"Ташкент","voice_name":"piper","ai_provider":"gemini","ai_model":"gemini-flash-latest","ai_key":"dpapi:00","ai_url":"","ai_fallback":true}"#;
        let s: UiSettings = serde_json::from_str(old).unwrap();
        assert_eq!((s.city.as_str(), s.ai_provider.as_str(), s.voice_rate, s.ai_fallback), ("Ташкент", "gemini", 1.2, true), "nothing of the old settings is lost");
        assert!(s.ai_enabled && s.notify_show && s.notify_speak && !s.dnd && !s.mic_off);
        assert!(!s.ext_enabled, "notifications of other programs stay off after an update");
        assert!(s.ext_strict && s.ext_allowed.is_empty());
        // and a new file still has every old field (an older version can read it back)
        let v = serde_json::to_value(&s).unwrap();
        for k in ["voice_rate", "reaction_voice", "city", "voice_name", "ai_provider", "ai_model", "ai_key", "ai_url", "ai_fallback"] {
            assert!(v.get(k).is_some(), "{} missing", k);
        }
    }

    #[test]
    fn an_unreadable_settings_file_is_kept_and_reported_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ui_settings.json");
        std::fs::write(&p, "{ broken json").unwrap();
        let (s, problem): (UiSettings, Option<String>) = parse_or_keep(&p, "файл");
        assert_eq!(s.voice_rate, 1.0, "defaults are used");
        let problem = problem.expect("the user is told");
        assert!(problem.contains("broken-"), "{}", problem);
        let kept: Vec<_> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(kept.iter().any(|n| n.contains("broken-")), "the old file is kept: {:?}", kept);
        assert_eq!(std::fs::read_to_string(dir.path().join(&kept[0])).unwrap(), "{ broken json");
        // no file at all is not a problem
        let (_, none): (UiSettings, Option<String>) = parse_or_keep(&dir.path().join("missing.json"), "файл");
        assert!(none.is_none());
    }

    #[test]
    fn the_key_never_goes_back_to_the_window_and_reset_keeps_it() {
        let _g = TEST_LOCK.lock();
        set_for_tests(UiSettings { ai_key: "secret-key-777".into(), dnd: true, notify_speak: false, ..UiSettings::default() });
        let page = serde_json::to_string(&settings_json()).unwrap();
        assert!(!page.contains("secret-key-777"));
        assert!(page.contains("\"ai_key_saved\":true"));
        reset_settings();
        let s = settings();
        assert_eq!(s.ai_key, "secret-key-777", "the key has its own button, a reset keeps it");
        assert!(!s.dnd && s.notify_speak, "the rest is back to the defaults");
        forget_ai_key();
        assert!(settings().ai_key.is_empty());
        set_for_tests(UiSettings::default());
    }

    #[test]
    fn switches_change_the_real_state() {
        let _g = TEST_LOCK.lock();
        set_for_tests(UiSettings::default());
        set_mic_muted(true);
        assert!(crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst));
        assert!(settings().mic_off, "remembered for the next start");
        set_mic_muted(false);
        assert!(!settings().mic_off);
        update_settings(&json!({ "dnd": true, "notify_sound": false, "ext_allowed": ["Telegram", " telegram ", "", "Mail"], "ui": { "jarvis-mode": "avatar" } }));
        let s = settings();
        assert!(s.dnd && !s.notify_sound);
        assert_eq!(s.ext_allowed, vec!["Telegram".to_string(), "Mail".to_string()], "trimmed, no duplicates, no empty names");
        assert_eq!(s.ui.get("jarvis-mode").map(|s| s.as_str()), Some("avatar"));
        update_settings(&json!({ "ai_enabled": false }));
        assert_eq!(crate::ai::conf_from_settings().unwrap_err(), crate::ai::AiError::Disabled);
        set_for_tests(UiSettings::default());
    }

    #[test]
    fn every_setting_survives_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ui_settings.json");
        let mut ui = std::collections::BTreeMap::new();
        ui.insert("jarvis-mode".to_string(), "avatar".to_string());
        let saved = UiSettings {
            voice_rate: 1.3, voice_name: "piper".into(), reaction_voice: "jarvis-og".into(), city: "Самарканд".into(),
            ai_provider: "gemini".into(), ai_model: "gemini-flash-latest".into(), ai_key: "key-123".into(), ai_fallback: true,
            ai_enabled: false, mic_off: true, notify_show: false, notify_speak: false, notify_sound: false, dnd: true,
            ext_enabled: true, ext_show: false, ext_speak: true, ext_strict: false,
            ext_allowed: vec!["Telegram".into()], ext_blocked: vec!["Spam".into()], ext_seen: vec!["Mail".into()], ui, ..UiSettings::default()
        };
        write_to(&p, &saved);
        assert!(!p.with_extension("json.tmp").exists(), "the temporary file is renamed, not left behind");
        let back = load_from(&p);
        assert_eq!(serde_json::to_value(&back).unwrap(), serde_json::to_value(&saved).unwrap(), "the next start reads back exactly what was saved");
        #[cfg(target_os = "windows")]
        assert!(!std::fs::read_to_string(&p).unwrap().contains("key-123"), "on Windows the key is stored encrypted");
    }

    #[test]
    fn a_reset_of_the_settings_never_deletes_the_history() {
        let _g = TEST_LOCK.lock();
        let store = crate::ai::store();
        let chat = store.create();
        store.append(&chat.id, "user", "вопрос, который должен остаться");
        let before = store.stats();
        set_for_tests(UiSettings { dnd: true, ..UiSettings::default() });
        reset_settings();
        // other tests may add messages meanwhile: nothing may disappear
        assert!(store.stats()["chats"].as_u64() >= before["chats"].as_u64(), "the conversations are untouched by a reset");
        assert!(store.stats()["messages"].as_u64() >= before["messages"].as_u64());
        assert_eq!(store.load(&chat.id).unwrap().messages[0].content, "вопрос, который должен остаться");
        assert!(!settings().dnd);
        set_for_tests(UiSettings::default());
    }

    #[test]
    fn the_microphone_sensitivity_really_changes_the_speech_threshold() {
        assert_eq!(vad_threshold(50), 100.0, "50 % keeps the old behaviour");
        assert!(vad_threshold(100) < vad_threshold(80) && vad_threshold(80) < vad_threshold(50), "more sensitive = a lower threshold");
        assert!(vad_threshold(0) > vad_threshold(20) && vad_threshold(20) > vad_threshold(50));
        assert_eq!(vad_threshold(250), vad_threshold(100));
        let _g = TEST_LOCK.lock();
        set_for_tests(UiSettings::default());
        update_settings(&json!({ "mic_sensitivity": 90, "tts_volume": 140, "browser": "evil.exe" }));
        assert_eq!(mic_threshold(), vad_threshold(90), "applied at once to the microphone");
        let s = settings();
        assert_eq!((s.tts_volume, s.browser.as_str()), (100, ""), "out of range is limited, an unknown browser is refused");
        update_settings(&json!({ "browser": "firefox", "mic_sensitivity": 50 }));
        assert_eq!(settings().browser, "firefox");
        assert_eq!(browser_exe("yandex"), Some("browser.exe"));
        set_for_tests(UiSettings::default());
    }

    #[test]
    fn listening_and_speaking_are_never_stored() {
        let v = serde_json::to_string(&UiSettings::default()).unwrap();
        assert!(!v.contains("LISTENING") && !v.contains("SPEAKING") && !v.contains("listening"));
    }
}

#[cfg(test)]
mod status_tests {
    // every command named in the status table exists, so the catalog never describes a removed command
    #[test]
    fn the_status_table_names_only_real_commands() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let mut all = String::new();
        for e in std::fs::read_dir(&root).unwrap().flatten() {
            if let Ok(t) = std::fs::read_to_string(e.path().join("command.toml")) {
                all.push_str(&t);
            }
        }
        for id in ["assistant_unlock", "assistant_phone_find", "assistant_phone_stop", "assistant_phone_link", "assistant_phone_text", "assistant_screen",
            "assistant_chat_start", "assistant_chat_end", "assistant_office_dictate", "assistant_office_cell", "assistant_office_new", "assistant_object_start",
            "assistant_brightness_set", "assistant_brightness_up", "assistant_brightness_down", "assistant_info_battery", "open_google", "calculator_1",
            "calculator_2", "windows_3", "steam_1", "steam_2", "jarvis_5"] {
            assert!(all.contains(&format!("id = \"{}\"", id)), "{} is not a command", id);
            assert!(super::command_status(id).is_some(), "{} has no status", id);
        }
        assert_eq!(super::command_status("assistant_timer_set"), None, "an ordinary command just works");
        assert_eq!(super::command_status("assistant_unlock").unwrap().0, "unavailable");
    }
}

