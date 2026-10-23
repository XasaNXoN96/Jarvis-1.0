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
    // after opening a link / program / file: "" (as it is) | "max" (the window on the whole screen) |
    // "grid" (the windows of the chain marked so are laid out side by side when the chain ends)
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
    // what starts it: "voice" = the phrase said after "Джарвис" or typed in the window / on the phone (the only kind
    // that exists; a scenario saved by an older version has no field and gets it on load)
    #[serde(default = "default_trigger")]
    pub trigger: String,
    pub enabled: bool,
    pub steps: Vec<Step>,
    pub updated: u64,
    pub last_run: Option<RunResult>,
}

fn default_trigger() -> String {
    "voice".to_string()
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
    if !sc.trigger.is_empty() && sc.trigger != "voice" {
        return Err("Этот тип запуска не поддерживается: сценарий запускается фразой (голосом или текстом)".into());
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

fn save(list: &[Scenario]) -> Result<(), String> {
    let p = file().ok_or("нет папки настроек")?;
    let text = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    crate::write_atomic(&p, text).map_err(|e| format!("файл сценариев не записан: {}", e))
}

pub fn list() -> Vec<Scenario> {
    LIST.lock().clone()
}

pub fn get(id: &str) -> Option<Scenario> {
    LIST.lock().iter().find(|s| s.id == id).cloned()
}

// creates (empty id) or replaces a scenario; returns the stored one
pub fn upsert(mut sc: Scenario) -> Result<Scenario, String> {
    if sc.trigger.is_empty() {
        sc.trigger = default_trigger();
    }
    sc.name = sc.name.trim().chars().take(80).collect();
    sc.phrase = sc.phrase.trim().chars().take(120).collect();
    for s in sc.steps.iter_mut() {
        s.param = s.param.trim().to_string();
        if s.window != "max" && s.window != "grid" {
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
    let before = list.clone();
    match list.iter_mut().find(|o| o.id == sc.id) {
        Some(o) => *o = sc.clone(),
        None => list.push(sc.clone()),
    }
    // the file could not be written (no space, no rights): the change is not kept in memory either, and the
    // user is told — never "saved" when it is not
    if let Err(e) = save_or_test(&list) {
        *list = before;
        return Err(format!("Не удалось сохранить: {}", e));
    }
    Ok(sc)
}

// (the tests have no config folder: there the list lives in memory only)
fn save_or_test(list: &[Scenario]) -> Result<(), String> {
    if cfg!(test) && file().is_none() {
        return Ok(());
    }
    save(list)
}

pub fn delete(id: &str) -> bool {
    let mut list = LIST.lock();
    let before = list.len();
    list.retain(|s| s.id != id);
    let changed = list.len() != before;
    if changed {
        let _ = save_or_test(&list);
    }
    changed
}

pub fn set_enabled(id: &str, on: bool) -> bool {
    let mut list = LIST.lock();
    match list.iter_mut().find(|s| s.id == id) {
        Some(s) => {
            s.enabled = on;
            let _ = save_or_test(&list);
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
    let list: Vec<Value> = list().into_iter().map(|s| json!({ "name": s.name, "phrase": s.phrase, "trigger": s.trigger, "enabled": s.enabled, "steps": s.steps })).collect();
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
        let _ = save_or_test(&list);
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
        json!({ "name": name, "phrase": phrase, "trigger": "voice", "steps": steps.into_iter().map(|(k, p, w)| json!({ "kind": k, "param": p, "window": w })).collect::<Vec<_>>() })
    };
    json!([
        t("Режим работы", "начать рабочий день", vec![("app", "телеграм", ""), ("link", "https://mail.google.com", "max"), ("volume", "30", "")]),
        t("Режим кино", "режим кино", vec![("volume", "70", ""), ("link", "https://www.youtube.com", "max")]),
        t("Режим учёбы", "режим учёбы", vec![("volume", "20", ""), ("app", "ворд", "max"), ("link", "https://translate.google.com", "")]),
        t("Игровой режим", "игровой вечер", vec![("app", "стим", ""), ("volume", "60", "")]),
        t("Я дома", "я дома", vec![("link", "https://www.youtube.com", "grid"), ("app", "телеграм", "grid"), ("app", "стим", "grid")]),
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
fn open(target: &str) -> Result<(), String> {
    // the rules of policy.rs are asked on every platform (the test of the rules runs on any computer)
    crate::policy::authorize_open(target)?;
    Err("открытие доступно только в Windows".into())
}

fn input_result(o: crate::input::Outcome) -> Result<(), String> {
    match o {
        crate::input::Outcome::Done => Ok(()),
        crate::input::Outcome::NoTarget => Err("нет окна, в котором можно выполнить действие".into()),
        crate::input::Outcome::Unknown(k) => Err(format!("неизвестное действие: {}", k)),
        crate::input::Outcome::Forbidden(why) => Err(format!("запрещено: {}", why)),
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

// the process an "app" step must produce: the file itself when a path to an .exe is given or the installed copy is
// started, else the known process of the program (calc.exe hands over to CalculatorApp.exe); None = not checked
pub fn app_process(param: &str) -> Option<String> {
    let p = param.trim();
    if std::path::Path::new(p).exists() {
        return exe_name(p);
    }
    let m = crate::apps::find_in_phrase(p)?;
    match crate::apps::locate(m.app) {
        Some(path) => exe_name(&path),
        None => m.app.process.map(|s| s.to_string()),
    }
}

fn exe_name(target: &str) -> Option<String> {
    let file = target.rsplit(['\\', '/']).next().unwrap_or("");
    file.to_lowercase().ends_with(".exe").then(|| file.to_string())
}

#[cfg(feature = "lua")]
fn process_running(exe: &str) -> bool {
    crate::lua::process_running(exe)
}

#[cfg(not(feature = "lua"))]
fn process_running(_exe: &str) -> bool {
    true
}

fn wait_process(exe: &str, max: Duration) -> bool {
    let until = std::time::Instant::now() + max;
    loop {
        if process_running(exe) {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

// one step on its own ("Тест шага"): a window marked "Авто сетка" takes the whole working area
pub fn run_step(s: &Step) -> Result<(), String> {
    let mut grid = Vec::new();
    let r = run_step_in(s, &mut grid);
    crate::input::arrange(&grid);
    r
}

fn run_step_in(s: &Step, grid: &mut Vec<isize>) -> Result<(), String> {
    // scenarios (and their steps) are a paid function: every way to run one comes through here
    crate::access::check("scenario_run").map_err(|d| d.message)?;
    check_step(s)?;
    // a scenario can do what a medium command does and no more: it cannot ask the user «да / нет» in the middle, so no
    // dangerous operation (a program from an unknown place, a forced kill) runs in it
    let _limits = crate::policy::enter_command("scenario", "medium", std::path::Path::new(""));
    let p = s.param.trim();
    let before = if s.window == "grid" { crate::input::foreground_window() } else { None };
    let opened = match s.kind.as_str() {
        "link" => open(p).map(|_| true),
        "search" => open(&format!("https://www.google.com/search?q={}", url_encode(p))).map(|_| true),
        "file" => {
            if !std::path::Path::new(p).exists() {
                return Err(format!("файл или папка не найдены: {}", p));
            }
            open(p).map(|_| true)
        }
        "app" => {
            let target = app_target(p);
            let exe = app_process(p);
            let was_running = exe.as_deref().map(process_running).unwrap_or(false);
            open(&target).map_err(|e| format!("программа «{}» не запустилась: {}", p, e))?;
            // the program must really appear (not only "Windows took the launch"); not checked when it was already
            // running or its program file is not known
            if let (Some(exe), false) = (exe, was_running) {
                if !wait_process(&exe, Duration::from_secs(8)) {
                    return Err(format!("программа «{}» не появилась после запуска", p));
                }
            }
            Ok(true)
        }
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
    if opened && s.window == "grid" {
        // the window of this step is remembered; two links in one browser window are one cell
        let link = matches!(s.kind.as_str(), "link" | "search");
        let expected = if s.kind == "app" { app_process(p) } else { None };
        if let Some(h) = crate::input::opened_window(before, link, expected.as_deref(), Duration::from_secs(10)) {
            if !grid.contains(&h) {
                grid.push(h);
            }
        }
    }
    Ok(())
}

// What runs now (the window shows "Шаг 2 из 4: Громкость"): (scenario id, step number from 1, steps, step title)
static RUNNING: Lazy<Mutex<Option<(String, usize, usize, String)>>> = Lazy::new(|| Mutex::new(None));

// the tests of the engine and of the autopilot share the one global «what runs now»: they take turns
#[cfg(test)]
pub(crate) static RUN_TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// «стоп» for a chain that is running: it stops before the next step (a step that has started finishes)
pub fn cancel() -> bool {
    if RUNNING.lock().is_some() {
        CANCEL.store(true, std::sync::atomic::Ordering::SeqCst);
        true
    } else {
        false
    }
}

pub fn running_json() -> Value {
    match RUNNING.lock().as_ref() {
        Some((id, step, total, title)) => json!({ "id": id, "step": step, "total": total, "title": title }),
        None => json!(null),
    }
}

// Everything that can be known before the first step: the parameters of all steps and the files that must exist.
// So a chain never starts and stops in the middle because of a typo in step 5.
pub fn preflight(sc: &Scenario) -> Result<(), String> {
    check(sc)?;
    for (i, s) in sc.steps.iter().enumerate() {
        if s.kind == "file" && !std::path::Path::new(s.param.trim()).exists() {
            return Err(format!("Шаг {} ({}): файл или папка не найдены: {}", i + 1, step_title(&s.kind), s.param.trim()));
        }
    }
    Ok(())
}

// The ONE way a scenario runs: its phrase said or typed, and the "Тест сценария" button. The steps run in their
// order, each one must report success; the first failure stops the chain with its number, kind and reason.
// The same chain never runs twice at once (a second start while it runs is refused, not queued).
pub fn run(sc: &Scenario) -> RunResult {
    run_inner(sc, true, None)
}

// a test of what the editor shows but is not saved: the same engine, but its result is not written into the history
// of the stored scenario (its steps differ from what was run)
pub fn run_draft(sc: &Scenario) -> RunResult {
    run_inner(sc, false, None)
}

// a plan made by the AI (autopilot): `gate` is asked BEFORE EVERY step, so a permission taken away or the owner lock
// closed in the middle of a long plan stops it at once instead of letting the rest run
pub fn run_gated(sc: &Scenario, gate: &dyn Fn() -> Result<(), String>) -> RunResult {
    run_inner(sc, false, Some(gate))
}

fn run_inner(sc: &Scenario, remember: bool, gate: Option<&dyn Fn() -> Result<(), String>>) -> RunResult {
    let total = sc.steps.len();
    {
        let mut running = RUNNING.lock();
        if let Some((id, step, n, _)) = running.as_ref() {
            let what = if *id == sc.id { "этот сценарий" } else { "другой сценарий" };
            return RunResult { at: now(), ok: false, done: 0, total, message: format!("Сценарий «{}» не запущен: {} уже выполняется (шаг {} из {})", sc.name, what, step, n) };
        }
        *running = Some((sc.id.clone(), 0, total, String::new()));
        // a stale «стоп» of an earlier chain must not stop this one; a «стоп» said from here on (also while the checks
        // before the first step run) is kept
        CANCEL.store(false, std::sync::atomic::Ordering::SeqCst);
    }
    struct Done;
    impl Drop for Done {
        fn drop(&mut self) {
            *RUNNING.lock() = None;
        }
    }
    let _done = Done;

    let mut done = 0;
    let mut message = String::new();
    let mut grid: Vec<isize> = Vec::new();
    if let Err(e) = preflight(sc) {
        message = e;
    } else {
        for (i, s) in sc.steps.iter().enumerate() {
            if CANCEL.swap(false, std::sync::atomic::Ordering::SeqCst) {
                message = format!("отменено перед шагом {}", i + 1);
                break;
            }
            if let Some(g) = gate {
                if let Err(e) = g() {
                    message = format!("остановлено перед шагом {}: {}", i + 1, e);
                    break;
                }
            }
            *RUNNING.lock() = Some((sc.id.clone(), i + 1, total, step_title(&s.kind).to_string()));
            match run_step_in(s, &mut grid) {
                Ok(()) => done += 1,
                Err(e) => {
                    message = format!("Шаг {} ({}): {}", i + 1, step_title(&s.kind), e);
                    break;
                }
            }
        }
    }
    // the windows marked "Авто сетка" side by side (also those opened before a step failed)
    crate::input::arrange(&grid);
    let ok = done == total;
    if ok {
        message = format!("Сценарий «{}» выполнен: шагов {}", sc.name, total);
    } else {
        message = format!("Сценарий «{}» остановлен. {}", sc.name, message);
    }
    let r = RunResult { at: now(), ok, done, total, message };
    if remember {
        let mut list = LIST.lock();
        if let Some(o) = list.iter_mut().find(|o| o.id == sc.id) {
            o.last_run = Some(r.clone());
            let _ = save_or_test(&list);
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
