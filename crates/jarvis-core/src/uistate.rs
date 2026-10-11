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
    pub voice_name: String,  // "" = automatic (Piper, else a female Windows voice)
    pub ai_provider: String, // none | anthropic | openai | ollama | custom
    pub ai_model: String,
    pub ai_key: String,
    pub ai_url: String,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self { voice_rate: 1.0, voice_name: String::new(), ai_provider: "none".into(), ai_model: String::new(), ai_key: String::new(), ai_url: String::new() }
    }
}

static SETTINGS: Lazy<Mutex<UiSettings>> = Lazy::new(|| Mutex::new(load_settings()));

fn file(name: &str) -> Option<PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join(name))
}

fn load_settings() -> UiSettings {
    file("ui_settings.json")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn settings() -> UiSettings {
    SETTINGS.lock().clone()
}

pub fn voice_rate() -> f32 {
    SETTINGS.lock().voice_rate.clamp(0.5, 2.0)
}

pub fn voice_name() -> String {
    SETTINGS.lock().voice_name.clone()
}

// merge the fields present in `patch` into the settings and save
pub fn update_settings(patch: &Value) {
    let mut s = SETTINGS.lock();
    if let Some(v) = patch.get("voice_rate").and_then(|v| v.as_f64()) { s.voice_rate = (v as f32).clamp(0.5, 2.0); }
    if let Some(v) = patch.get("voice_name").and_then(|v| v.as_str()) { s.voice_name = v.chars().take(120).collect(); }
    if let Some(v) = patch.get("ai_provider").and_then(|v| v.as_str()) { s.ai_provider = v.chars().take(30).collect(); }
    if let Some(v) = patch.get("ai_model").and_then(|v| v.as_str()) { s.ai_model = v.chars().take(100).collect(); }
    if let Some(v) = patch.get("ai_key").and_then(|v| v.as_str()) { s.ai_key = v.chars().take(300).collect(); }
    if let Some(v) = patch.get("ai_url").and_then(|v| v.as_str()) { s.ai_url = v.chars().take(300).collect(); }
    if let Some(p) = file("ui_settings.json") {
        if let Ok(json) = serde_json::to_string_pretty(&*s) {
            let _ = std::fs::write(p, json);
        }
    }
}

// the settings page never gets the key back, only whether one is saved
pub fn settings_json() -> Value {
    let s = settings();
    json!({
        "voice_rate": s.voice_rate, "voice_name": s.voice_name,
        "ai_provider": s.ai_provider, "ai_model": s.ai_model, "ai_url": s.ai_url,
        "ai_key_saved": !s.ai_key.is_empty(),
        "tts_enabled": crate::tts::is_enabled(),
        "piper": crate::tts::piper_available(),
    })
}

// ---------------------------------------------------------------------------------------------
// command sections ("groups"): the commands page and the panel show commands by topic, and a whole
// section can be switched off
// ---------------------------------------------------------------------------------------------
// (key, emoji, title, short description)
pub const GROUPS: &[(&str, &str, &str, &str)] = &[
    ("sound", "🔊", "Звук и экран", "громкость, яркость, музыка: пауза, следующий трек"),
    ("timers", "⏱", "Таймеры и напоминания", "таймер, напоминание, будильник, «остановись через…»"),
    ("apps", "🌐", "Программы и сайты", "открыть сайт, браузер, блокнот, калькулятор, закрыть программу"),
    ("files", "📁", "Файлы и папки", "найти файл, открыть папку, диск, настройки Windows"),
    ("text", "📝", "Тексты и документы", "диктовка в Word и блокнот, таблицы Excel, новый документ, заметки"),
    ("voice", "🎤", "Голос и озвучка", "микрофон, озвучка, чтение выделенного текста, проверка голоса"),
    ("system", "🖥", "Компьютер и питание", "выключить, перезагрузить, заблокировать, свернуть окна, скриншот"),
    ("info", "📊", "Информация", "время, дата, батарея, память, процессор, диск, IP"),
    ("web", "🌍", "Интернет", "погода, Википедия, перевод, карты, музыка"),
    ("games", "🎮", "Игры", "игровой режим и таймер игры, Steam"),
    ("remote", "📱", "Пульт для телефона", "управление с телефона"),
    ("talk", "💬", "Разговор и помощь", "привет, «что ты умеешь», диагностика, шутки"),
    ("control", "⛔", "Управление Jarvis", "стоп, выключить Jarvis (отключить нельзя)"),
];

pub fn group_key(pack: &str, id: &str) -> &'static str {
    let id = id.strip_prefix("assistant_").unwrap_or(id);
    let starts = |p: &str| id.starts_with(p);
    if starts("volume") || starts("brightness") || starts("media") { return "sound"; }
    if starts("timer") || id == "reminder" { return "timers"; }
    if starts("file") || id == "place_open" || id == "disk_open" { return "files"; }
    if starts("note") || starts("office") || id == "session" { return "text"; }
    if starts("tts") || starts("read_") || starts("mic_") { return "voice"; }
    if starts("power") || id == "unlock" || pack == "windows" { return "system"; }
    if starts("info") || starts("sys_") { return "info"; }
    if id == "wiki" || id == "translate" || id == "maps" || id == "music_search" || pack == "weather" { return "web"; }
    if starts("game") || pack == "steam" { return "games"; }
    if starts("remote") { return "remote"; }
    if pack == "stop" || pack == "terminate" { return "control"; }
    if id == "website" || starts("program") || pack == "browser" || pack == "calculator" { return "apps"; }
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
        let _ = std::fs::write(p, serde_json::to_string(&list).unwrap_or_default());
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
    Stats { cpu: 0.0, mem_used_gb: gb(total - avail), mem_total_gb: gb(total), disk_free_gb: 0.0, disk_total_gb: 0.0 }
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
        "session": crate::session::kind().unwrap_or_default(),
        "session_data": crate::session::data(),
        "mic_muted": crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst),
        "voice_active": crate::VOICE_ACTIVE.load(std::sync::atomic::Ordering::SeqCst),
        "voice_error": crate::VOICE_ERROR.lock().clone(),
        "mic_level": crate::MIC_LEVEL.load(std::sync::atomic::Ordering::Relaxed),
        "tts_enabled": crate::tts::is_enabled(),
        "remote": crate::remote::is_running(),
    })
}
