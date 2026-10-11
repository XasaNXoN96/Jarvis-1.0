// Scenarios of the "Конструктор": a phrase said after "Джарвис" runs a chain of steps.
//
// Steps reuse what the commands already use (no second automation engine): opening links, files and programs
// (lua::shell_open, apps.rs), web search, typing text, shortcuts and mouse (input.rs), the volume, the clipboard and
// pauses. A step that cannot be done stops the chain and says which step and why; nothing is reported as done when it
// was not. Arbitrary shell commands are deliberately not a step type: a scenario could then run anything.
//
// Stored in <config dir>/scenarios.json (an unreadable file is kept as .broken-<time>, never overwritten silently).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const STEP_KINDS: &[(&str, &str, &str)] = &[
    // (kind, title, what the parameter is)
    ("link", "Ссылка", "адрес сайта: https://example.com"),
    ("app", "Приложение", "название программы («телеграм», «ворд») или путь к .exe"),
    ("file", "Файл", "полный путь к файлу или папке"),
    ("text", "Текст", "текст, который будет напечатан в активном окне"),
    ("keys", "Клавиши", "сочетание: ctrl+t, alt+tab, win+d, f5"),
    ("mouse", "Мышь", "click | double | right | scroll_up | scroll_down"),
    ("search", "Поиск", "что найти в интернете"),
    ("volume", "Громкость", "число от 0 до 100"),
    ("pause", "Пауза", "секунды (до 60)"),
    ("clipboard", "Буфер", "текст, который окажется в буфере обмена"),
];

const MAX_STEPS: usize = 30;
const MAX_PAUSE_MS: u64 = 60_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Step {
    pub kind: String,
    pub param: String,
    // after opening a link / program / file: "" (as it is) | "max" (the window on the whole screen)
    pub window: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct RunResult {
    pub at: u64,
    pub ok: bool,
    pub done: usize,
    pub total: usize,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct Scenario {
    pub id: String,
    pub name: String,
    pub phrase: String, // said after "Джарвис"
    pub enabled: bool,
    pub steps: Vec<Step>,
    pub updated: u64,
    pub last_run: Option<RunResult>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// "Начать рабочий день!" -> "начать рабочий день"
pub fn norm(text: &str) -> String {
    text.to_lowercase().replace('ё', "е").split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------------------------
// checking (pure, tested)
// ---------------------------------------------------------------------------------------------
pub fn check_step(s: &Step) -> Result<(), String> {
    let p = s.param.trim();
    match s.kind.as_str() {
        "link" => {
            let l = p.to_lowercase();
            if !(l.starts_with("https://") || l.starts_with("http://")) || p.len() < 10 || p.contains(char::is_whitespace) {
                return Err("ссылка должна начинаться с https:// или http:// и не содержать пробелов".into());
            }
        }
        "app" | "file" | "text" | "search" | "clipboard" => {
            if p.is_empty() {
                return Err("параметр не заполнен".into());
            }
        }
        "keys" => {
            if crate::input::parse_combo(p).is_none() {
                return Err(format!("непонятное сочетание клавиш «{}» (пример: ctrl+t)", p));
            }
        }
        "mouse" => {
            if !matches!(p, "click" | "double" | "right" | "scroll_up" | "scroll_down") {
                return Err("для мыши: click, double, right, scroll_up или scroll_down".into());
            }
        }
        "volume" => match p.parse::<i32>() {
            Ok(0..=100) => {}
            _ => return Err("громкость — число от 0 до 100".into()),
        },
        "pause" => match p.replace(',', ".").parse::<f64>() {
            Ok(v) if v > 0.0 && v <= 60.0 => {}
            _ => return Err("пауза — от 0.1 до 60 секунд".into()),
        },
        "shell" => return Err("команды оболочки не поддерживаются: так сценарий мог бы запустить что угодно".into()),
        other => return Err(format!("неизвестный тип шага «{}»", other)),
    }
    if p.chars().count() > 2000 {
        return Err("параметр слишком длинный".into());
    }
    Ok(())
}

pub fn check(sc: &Scenario) -> Result<(), String> {
    if sc.name.trim().is_empty() {
        return Err("Дайте сценарию название".into());
    }
    if norm(&sc.phrase).is_empty() {
        return Err("Впишите фразу активации, например: начать рабочий день".into());
    }
    if sc.steps.is_empty() {
        return Err("Добавьте хотя бы один шаг".into());
    }
    if sc.steps.len() > MAX_STEPS {
        return Err(format!("Не больше {} шагов в одном сценарии", MAX_STEPS));
    }
    for (i, s) in sc.steps.iter().enumerate() {
        check_step(s).map_err(|e| format!("Шаг {}: {}", i + 1, e))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// storage
// ---------------------------------------------------------------------------------------------
static LIST: Lazy<Mutex<Vec<Scenario>>> = Lazy::new(|| Mutex::new(load()));

fn file() -> Option<std::path::PathBuf> {
    crate::APP_CONFIG_DIR.get().map(|d| d.join("scenarios.json"))
}

fn load() -> Vec<Scenario> {
    match file() {
        Some(p) => {
            let (list, problem): (Vec<Scenario>, Option<String>) = crate::uistate::parse_or_keep(&p, "файл сценариев (scenarios.json)");
            if let Some(pr) = problem {
                crate::uistate::add_load_problem(&pr);
            }
            list
        }
        None => Vec::new(),
    }
}

fn save(list: &[Scenario]) {
    if let Some(p) = file() {
        if let Ok(text) = serde_json::to_string_pretty(list) {
            let tmp = p.with_extension("json.tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &p);
            }
        }
    }
}

pub fn list() -> Vec<Scenario> {
    LIST.lock().clone()
}

pub fn get(id: &str) -> Option<Scenario> {
    LIST.lock().iter().find(|s| s.id == id).cloned()
}

// creates (empty id) or replaces a scenario; returns the stored one
pub fn upsert(mut sc: Scenario) -> Result<Scenario, String> {
    sc.name = sc.name.trim().chars().take(80).collect();
    sc.phrase = sc.phrase.trim().chars().take(120).collect();
    for s in sc.steps.iter_mut() {
        s.param = s.param.trim().to_string();
        if s.window != "max" {
            s.window.clear();
        }
    }
    check(&sc)?;
    let mut list = LIST.lock();
    let key = norm(&sc.phrase);
    if list.iter().any(|o| o.id != sc.id && norm(&o.phrase) == key) {
        return Err(format!("Фраза «{}» уже занята другим сценарием", sc.phrase));
    }
    if sc.id.is_empty() {
        sc.id = format!("s{}-{}", now(), rand::random::<u16>());
        sc.enabled = true;
    } else if let Some(old) = list.iter().find(|o| o.id == sc.id) {
        if sc.last_run.is_none() {
            sc.last_run = old.last_run.clone();
        }
    }
    sc.updated = now();
    match list.iter_mut().find(|o| o.id == sc.id) {
        Some(o) => *o = sc.clone(),
        None => list.push(sc.clone()),
    }
    save(&list);
    Ok(sc)
}

pub fn delete(id: &str) -> bool {
    let mut list = LIST.lock();
    let before = list.len();
    list.retain(|s| s.id != id);
    let changed = list.len() != before;
    if changed {
        save(&list);
    }
    changed
}

pub fn set_enabled(id: &str, on: bool) -> bool {
    let mut list = LIST.lock();
    match list.iter_mut().find(|s| s.id == id) {
        Some(s) => {
            s.enabled = on;
            save(&list);
            true
        }
        None => false,
    }
}

// the scenario a phrase starts (the phrase itself, or the phrase followed by nothing but filler)
pub fn find(text: &str) -> Option<Scenario> {
    let t = norm(text);
    if t.is_empty() {
        return None;
    }
    LIST.lock().iter().find(|s| s.enabled && !norm(&s.phrase).is_empty() && norm(&s.phrase) == t).cloned()
}

// export: the scenarios as a JSON document (no ids of this computer needed to import them elsewhere)
pub fn export_json() -> Value {
    let list: Vec<Value> = list().into_iter().map(|s| json!({ "name": s.name, "phrase": s.phrase, "enabled": s.enabled, "steps": s.steps })).collect();
    json!({ "jarvis_scenarios": 1, "scenarios": list })
}

// import: `replace` = the imported list replaces all; otherwise added (phrases already used are skipped, not doubled)
pub fn import_json(doc: &Value, replace: bool) -> Result<(usize, Vec<String>), String> {
    let items = doc.get("scenarios").and_then(|v| v.as_array()).ok_or("В файле нет списка сценариев")?;
    let mut parsed = Vec::new();
    let mut skipped = Vec::new();
    for it in items.iter().take(200) {
        let mut sc: Scenario = serde_json::from_value(it.clone()).map_err(|_| "Файл сценариев повреждён")?;
        sc.id.clear();
        sc.last_run = None;
        match check(&sc) {
            Ok(()) => parsed.push(sc),
            Err(e) => skipped.push(format!("«{}»: {}", sc.name, e)),
        }
    }
    if replace {
        let mut list = LIST.lock();
        list.clear();
        save(&list);
    }
    let mut added = 0;
    for sc in parsed {
        let name = sc.name.clone();
        match upsert(sc) {
            Ok(_) => added += 1,
            Err(e) => skipped.push(format!("«{}»: {}", name, e)),
        }
    }
    Ok((added, skipped))
}

// ready-made scenarios to start from (the user edits them before saving)
pub fn templates() -> Value {
    let t = |name: &str, phrase: &str, steps: Vec<(&str, &str, &str)>| {
        json!({ "name": name, "phrase": phrase, "steps": steps.into_iter().map(|(k, p, w)| json!({ "kind": k, "param": p, "window": w })).collect::<Vec<_>>() })
    };
    json!([
        t("Режим работы", "начать рабочий день", vec![("app", "телеграм", ""), ("link", "https://mail.google.com", "max"), ("volume", "30", "")]),
        t("Режим кино", "режим кино", vec![("volume", "70", ""), ("link", "https://www.youtube.com", "max")]),
        t("Режим учёбы", "режим учёбы", vec![("volume", "20", ""), ("app", "ворд", "max"), ("link", "https://translate.google.com", "")]),
        t("Игровой режим", "игровой вечер", vec![("app", "стим", ""), ("volume", "60", "")]),
    ])
}

// ---------------------------------------------------------------------------------------------
// running
// ---------------------------------------------------------------------------------------------
fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect()
}

#[cfg(all(feature = "lua", target_os = "windows"))]
fn open(target: &str) -> Result<(), String> {
    crate::lua::shell_open(target)
}

#[cfg(not(all(feature = "lua", target_os = "windows")))]
fn open(_target: &str) -> Result<(), String> {
    Err("открытие доступно только в Windows".into())
}

fn input_result(o: crate::input::Outcome) -> Result<(), String> {
    match o {
        crate::input::Outcome::Done => Ok(()),
        crate::input::Outcome::NoTarget => Err("нет окна, в котором можно выполнить действие".into()),
        crate::input::Outcome::Unknown(k) => Err(format!("неизвестное действие: {}", k)),
    }
}

// the program of an "app" step: an existing path, else a known program name (apps.rs), else the name as it is
pub fn app_target(param: &str) -> String {
    let p = param.trim();
    if std::path::Path::new(p).exists() {
        return p.to_string();
    }
    match crate::apps::find_in_phrase(p) {
        Some(m) => crate::apps::locate(m.app).unwrap_or_else(|| m.app.start.to_string()),
        None => p.to_string(),
    }
}

pub fn run_step(s: &Step) -> Result<(), String> {
    check_step(s)?;
    let p = s.param.trim();
    let opened = match s.kind.as_str() {
        "link" => open(p).map(|_| true),
        "search" => open(&format!("https://www.google.com/search?q={}", url_encode(p))).map(|_| true),
        "file" => {
            if !std::path::Path::new(p).exists() {
                return Err(format!("файл или папка не найдены: {}", p));
            }
            open(p).map(|_| true)
        }
        "app" => open(&app_target(p)).map(|_| true).map_err(|e| format!("программа «{}» не запустилась: {}", p, e)),
        "text" => {
            #[cfg(feature = "lua")]
            {
                crate::lua::type_text(p, None).map(|_| false)
            }
            #[cfg(not(feature = "lua"))]
            Err("ввод текста недоступен в этой сборке".to_string())
        }
        "keys" => input_result(crate::input::keys(p)).map(|_| false),
        "mouse" => input_result(crate::input::mouse(p, 0)).map(|_| false),
        "volume" => {
            #[cfg(feature = "lua")]
            {
                crate::lua::volume_control("set", p.parse().unwrap_or(50)).map(|_| false)
            }
            #[cfg(not(feature = "lua"))]
            Err("громкость недоступна в этой сборке".to_string())
        }
        "clipboard" => {
            #[cfg(all(feature = "lua", target_os = "windows"))]
            {
                crate::lua::set_clipboard_text(p).map(|_| false)
            }
            #[cfg(not(all(feature = "lua", target_os = "windows")))]
            Err("буфер обмена доступен только в Windows".to_string())
        }
        "pause" => {
            let ms = (p.replace(',', ".").parse::<f64>().unwrap_or(1.0) * 1000.0) as u64;
            std::thread::sleep(Duration::from_millis(ms.min(MAX_PAUSE_MS)));
            Ok(false)
        }
        other => Err(format!("неизвестный тип шага «{}»", other)),
    }?;
    if opened && s.window == "max" {
        // the opened window needs a moment to appear before it can be maximized
        std::thread::sleep(Duration::from_millis(1500));
        let _ = crate::input::window("maximize");
    }
    Ok(())
}

// runs the whole chain; stops at the first step that fails. The result is remembered (history of runs).
pub fn run(sc: &Scenario) -> RunResult {
    let total = sc.steps.len();
    let mut done = 0;
    let mut message = String::new();
    for (i, s) in sc.steps.iter().enumerate() {
        match run_step(s) {
            Ok(()) => done += 1,
            Err(e) => {
                message = format!("Шаг {} ({}): {}", i + 1, step_title(&s.kind), e);
                break;
            }
        }
    }
    let ok = done == total;
    if ok {
        message = format!("Сценарий «{}» выполнен: шагов {}", sc.name, total);
    } else {
        message = format!("Сценарий «{}» остановлен. {}", sc.name, message);
    }
    let r = RunResult { at: now(), ok, done, total, message };
    {
        let mut list = LIST.lock();
        if let Some(o) = list.iter_mut().find(|o| o.id == sc.id) {
            o.last_run = Some(r.clone());
            save(&list);
        }
    }
    crate::uistate::log_event(if ok { "command" } else { "error" }, &r.message);
    r
}

pub fn step_title(kind: &str) -> &'static str {
    STEP_KINDS.iter().find(|k| k.0 == kind).map(|k| k.1).unwrap_or("шаг")
}

pub fn kinds_json() -> Value {
    json!(STEP_KINDS.iter().map(|(k, t, h)| json!({ "kind": k, "title": t, "hint": h })).collect::<Vec<_>>())
}

#[cfg(test)]
pub fn reset_for_tests() {
    LIST.lock().clear();
}

#[cfg(test)]
mod tests;
