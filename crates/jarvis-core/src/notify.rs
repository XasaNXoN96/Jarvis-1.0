// Notifications.
//
// 1. Jarvis' own notifications: timers, alarms, reminders, finished tasks, results of commands, important messages,
//    errors of the AI and of computer functions. They come only from real events of the program (timer.rs, dialog.rs,
//    ai.rs, the settings loader) through push(). Each one is shown in the notification window of Jarvis (bottom right
//    of the screen, notify_win.rs), kept in the short list of the window and, if the settings allow it, spoken once.
//    The same notification twice in a few seconds is shown once (guard.rs). A reply that the voice has already said is
//    never said again by its notification.
//
// 2. Notifications of other Windows programs (messengers, mail ...): OFF by default. When the user switches them on,
//    Jarvis asks Windows for access (UserNotificationListener, the official API; Windows shows its own permission
//    question and may refuse, then the window says so). Only programs of the allowed list are let through when the
//    strict list is on; excluded programs never are. Their text stays on this computer: it is never stored on disk,
//    never put into the conversation, never sent to an AI or anywhere else.

use std::collections::{HashSet, VecDeque};
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

use crate::uistate::UiSettings;

#[derive(Clone, Serialize, Debug)]
pub struct Note {
    pub id: u64,
    pub time: u64,
    pub kind: String,   // timer | alarm | reminder | task | command | important | ai_error | system_error | external
    pub title: String,
    pub text: String,
    pub source: String, // the program of an external notification ("" for Jarvis)
    pub critical: bool,
    pub shown: bool,    // a popup was shown
    pub spoken: bool,   // its text was said by the notification
}

static NOTES: Lazy<Mutex<VecDeque<Note>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static NOTE_ID: AtomicU64 = AtomicU64::new(1);
// the window reported "I am visible" at this time (results of commands pop up only when nobody looks at the window)
static WINDOW_SEEN: AtomicU64 = AtomicU64::new(0);

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn window_seen() {
    WINDOW_SEEN.store(now(), Ordering::SeqCst);
}

pub fn window_visible() -> bool {
    now().saturating_sub(WINDOW_SEEN.load(Ordering::SeqCst)) <= 5
}

// critical messages are never hidden completely: even in "do not disturb" they are shown, silently
pub fn is_critical(kind: &str) -> bool {
    matches!(kind, "important" | "system_error")
}

// the kind of a timer by its title (the timer system has one kind of entry for timers, alarms and reminders)
pub fn timer_kind(title: &str) -> &'static str {
    let t = title.to_lowercase();
    if t.contains("будильн") || t.contains("alarm") || t.contains("разбуд") {
        "alarm"
    } else if t.contains("напомин") || t.contains("remind") {
        "reminder"
    } else {
        "timer"
    }
}

// ---------------------------------------------------------------------------------------------
// what happens with a notification: decided from the settings only (pure, tested)
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prefs {
    pub show: bool,
    pub speak: bool,
    pub sound: bool,
    pub dnd: bool,
    pub tts_on: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Plan {
    pub show: bool,
    pub speak: bool,
    pub sound: bool,
}

pub fn plan(p: &Prefs, critical: bool, already_spoken: bool) -> Plan {
    let show = p.show && (!p.dnd || critical);
    let speak = p.speak && !p.dnd && !already_spoken && p.tts_on;
    // a short sound when the notification is not spoken (the voice is the alert itself)
    let sound = p.sound && !p.dnd && !speak && (show || !already_spoken);
    Plan { show, speak, sound }
}

pub fn prefs_jarvis(s: &UiSettings) -> Prefs {
    Prefs { show: s.notify_show, speak: s.notify_speak, sound: s.notify_sound, dnd: s.dnd, tts_on: crate::tts::is_enabled() }
}

pub fn prefs_external(s: &UiSettings) -> Prefs {
    Prefs { show: s.ext_show, speak: s.ext_speak, sound: s.notify_sound, dnd: s.dnd, tts_on: crate::tts::is_enabled() }
}

// ---------------------------------------------------------------------------------------------
// Jarvis' own notifications
// ---------------------------------------------------------------------------------------------

// `spoken`: the voice has already said this (a reply of a command): the notification stays silent.
// Returns the id of the notification, None when it was a repeat.
pub fn push(kind: &str, title: &str, text: &str, spoken: bool) -> Option<u64> {
    let title = if title.trim().is_empty() { default_title(kind) } else { title.trim().to_string() };
    let text = text.trim();
    if text.is_empty() && title.is_empty() {
        return None;
    }
    // the same notification from two parts of the program (or twice in a row) is shown once
    if !crate::guard::first(&format!("note:{}:{}:{}", kind, crate::guard::norm(&title), crate::guard::norm(text)), crate::guard::NOTIFY_WINDOW) {
        return None;
    }
    let critical = is_critical(kind);
    let p = plan(&prefs_jarvis(&crate::uistate::settings()), critical, spoken);
    let say = if text.is_empty() { title.clone() } else { text.to_string() };
    Some(deliver(kind, &title, text, "", critical, p, &say))
}

fn default_title(kind: &str) -> String {
    match kind {
        "timer" => "Таймер",
        "alarm" => "Будильник",
        "reminder" => "Напоминание",
        "task" => "Задача выполнена",
        "command" => "Джарвис",
        "important" => "Важно",
        "ai_error" => "Ошибка ИИ",
        "system_error" => "Ошибка",
        "external" => "Уведомление",
        _ => "Джарвис",
    }
    .to_string()
}

fn deliver(kind: &str, title: &str, text: &str, source: &str, critical: bool, p: Plan, say: &str) -> u64 {
    let id = NOTE_ID.fetch_add(1, Ordering::SeqCst);
    let note = Note {
        id,
        time: now(),
        kind: kind.to_string(),
        title: title.chars().take(120).collect(),
        text: text.chars().take(600).collect(),
        source: source.chars().take(80).collect(),
        critical,
        shown: p.show,
        spoken: p.speak,
    };
    {
        let mut list = NOTES.lock();
        list.push_front(note.clone());
        list.truncate(40);
    }
    if kind != "external" {
        // the event list of the program (diagnostics) never gets the text of other programs' notifications
        crate::uistate::log_event(if kind.ends_with("error") { "error" } else { "info" }, &format!("Уведомление: {} — {}", note.title, note.text));
    }
    // the automatic tests decide and record, but never open windows, speak or beep on the test machine
    if cfg!(test) {
        return id;
    }
    if p.show {
        display(&note);
    }
    if p.speak {
        crate::tts::speak(say);
    } else if p.sound {
        beep();
    }
    id
}

pub fn recent(n: usize) -> Vec<Note> {
    NOTES.lock().iter().take(n).cloned().collect()
}

pub fn clear() {
    NOTES.lock().clear();
}

#[cfg(target_os = "windows")]
fn display(note: &Note) {
    crate::notify_win::show(crate::notify_win::Popup {
        title: note.title.clone(),
        text: note.text.clone(),
        source: note.source.clone(),
        time: chrono::Local::now().format("%H:%M").to_string(),
        error: note.kind.ends_with("error"),
        critical: note.critical,
    });
}

#[cfg(not(target_os = "windows"))]
fn display(note: &Note) {
    info!("Notification: {} — {}", note.title, note.text);
}

#[cfg(target_os = "windows")]
fn beep() {
    unsafe {
        winapi::um::winuser::MessageBeep(winapi::um::winuser::MB_ICONASTERISK);
    }
}

#[cfg(not(target_os = "windows"))]
fn beep() {}

// ---------------------------------------------------------------------------------------------
// notifications of other programs
// ---------------------------------------------------------------------------------------------

// is a notification of this program let through? (pure, tested)
pub fn ext_allowed(s: &UiSettings, app: &str) -> bool {
    if !s.ext_enabled {
        return false;
    }
    let app = app.trim();
    let listed = |list: &[String]| !app.is_empty() && list.iter().any(|x| x.trim().eq_ignore_ascii_case(app));
    if listed(&s.ext_blocked) {
        return false;
    }
    if s.ext_strict {
        // a program without a name, or one that is not on the list, is never let through automatically
        return listed(&s.ext_allowed);
    }
    true
}

// (title, text) of an external notification; a notification without any text still says where it comes from
pub fn ext_texts(app: &str, texts: &[String]) -> (String, String) {
    let parts: Vec<String> = texts.iter().map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|t| !t.is_empty()).collect();
    let app = if app.trim().is_empty() { "Неизвестная программа".to_string() } else { app.trim().to_string() };
    match parts.len() {
        0 => (app, "Новое уведомление (без текста)".to_string()),
        1 => (app, parts[0].clone()),
        _ => (format!("{} · {}", app, parts[0]), parts[1..].join(" — ")),
    }
}

static EXT_IDS: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

// one notification reported by Windows. `initial`: it was already in the notification center when the watcher
// started (it is not shown again, only the program name is remembered for the list of the settings).
pub fn on_external(id: &str, app: &str, texts: &[String], initial: bool) -> Option<u64> {
    crate::uistate::remember_app(app);
    {
        let mut ids = EXT_IDS.lock();
        if !ids.insert(id.to_string()) {
            return None; // the same notification again: never shown or read twice
        }
        if ids.len() > 2000 {
            ids.clear();
        }
    }
    if initial {
        return None;
    }
    let s = crate::uistate::settings();
    if !ext_allowed(&s, app) {
        return None; // not allowed: the content is dropped here, it goes nowhere
    }
    let (title, text) = ext_texts(app, texts);
    // a program that posts the same text twice with two ids
    if !crate::guard::first(&format!("ext:{}:{}", crate::guard::norm(&title), crate::guard::norm(&text)), crate::guard::NOTIFY_WINDOW) {
        return None;
    }
    let p = plan(&prefs_external(&s), false, false);
    let say: String = format!("Уведомление от {}. {}", app.trim(), text).chars().take(220).collect();
    Some(deliver("external", &title, &text, app, false, p, &say))
}

// ---- the watcher process (Windows): PowerShell with the official WinRT API, only while the switch is on ----
#[derive(Clone)]
struct ExtState {
    state: String, // off | starting | on | denied | error | unsupported
    detail: String,
}

static EXT_STATE: Lazy<Mutex<ExtState>> = Lazy::new(|| Mutex::new(ExtState { state: "off".into(), detail: String::new() }));
static EXT_CHILD: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));
static EXT_GEN: AtomicU64 = AtomicU64::new(0);

fn ext_set(state: &str, detail: &str) {
    *EXT_STATE.lock() = ExtState { state: state.into(), detail: detail.into() };
}

pub fn ext_status_json() -> Value {
    let st = EXT_STATE.lock().clone();
    json!({ "state": st.state, "detail": st.detail })
}

// start or stop the watcher as the settings say
pub fn ext_apply() {
    if crate::uistate::settings().ext_enabled {
        ext_start();
    } else {
        ext_stop();
    }
}

pub fn ext_stop() {
    EXT_GEN.fetch_add(1, Ordering::SeqCst);
    if let Some(mut c) = EXT_CHILD.lock().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    ext_set("off", "");
}

// The script uses only the documented API and only reads the notifications Windows lets it read.
// ASCII only (Windows PowerShell 5 reads a script without BOM in the ANSI code page); the output is UTF-8 JSON lines.
const EXT_SCRIPT: &str = r#"
param([int]$ParentPid = 0)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
function Emit($o) { [Console]::Out.WriteLine(($o | ConvertTo-Json -Compress -Depth 4)); [Console]::Out.Flush() }
try {
  Add-Type -AssemblyName System.Runtime.WindowsRuntime
  $asTask = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object { $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1' } | Select-Object -First 1
  function Await($op, [Type]$t) { $task = $asTask.MakeGenericMethod($t).Invoke($null, @($op)); $null = $task.Wait(-1); $task.Result }
  $null = [Windows.UI.Notifications.Management.UserNotificationListener, Windows.UI.Notifications, ContentType = WindowsRuntime]
  $null = [Windows.UI.Notifications.UserNotification, Windows.UI.Notifications, ContentType = WindowsRuntime]
  $null = [Windows.UI.Notifications.KnownNotificationBindings, Windows.UI.Notifications, ContentType = WindowsRuntime]
  $listener = [Windows.UI.Notifications.Management.UserNotificationListener]::Current
  $status = Await ($listener.RequestAccessAsync()) ([Windows.UI.Notifications.Management.UserNotificationListenerAccessStatus])
  Emit @{ type = 'access'; status = "$status" }
  if ("$status" -ne 'Allowed') { exit 0 }
  $listType = [System.Collections.Generic.IReadOnlyList[Windows.UI.Notifications.UserNotification]]
  $seen = @{}
  $first = $true
  while ($true) {
    if ($ParentPid -gt 0 -and -not (Get-Process -Id $ParentPid -ErrorAction SilentlyContinue)) { exit 0 }
    $list = Await ($listener.GetNotificationsAsync([Windows.UI.Notifications.NotificationKinds]::Toast)) $listType
    foreach ($n in $list) {
      $key = [string]$n.Id
      if ($seen.ContainsKey($key)) { continue }
      $seen[$key] = $true
      $app = ''
      try { $app = [string]$n.AppInfo.DisplayInfo.DisplayName } catch {}
      $texts = @()
      try {
        $b = $n.Notification.Visual.GetBinding([Windows.UI.Notifications.KnownNotificationBindings]::ToastGeneric)
        if ($b) { $texts = @($b.GetTextElements() | ForEach-Object { [string]$_.Text }) }
      } catch {}
      Emit @{ type = 'note'; id = $key; app = $app; texts = $texts; initial = $first }
    }
    $first = $false
    Start-Sleep -Seconds 2
  }
} catch {
  Emit @{ type = 'error'; message = $_.Exception.Message }
}
"#;

// the explanation shown when Windows does not give the access
pub fn ext_denied_text(status: &str) -> String {
    format!(
        "Windows не дала доступ к уведомлениям (ответ: {}). Откройте Параметры Windows → Конфиденциальность и защита → Уведомления \
         (в Windows 10: Конфиденциальность → Уведомления) и разрешите доступ к уведомлениям, затем выключите и включите этот переключатель. \
         Обычные (не из Microsoft Store) программы Windows может не пустить к уведомлениям совсем — тогда эта функция на этом компьютере недоступна.",
        status
    )
}

fn handle_line(line: &str) {
    let v: Value = match serde_json::from_str(line.trim()) {
        Ok(v) => v,
        Err(_) => return,
    };
    match v["type"].as_str().unwrap_or("") {
        "access" => {
            let status = v["status"].as_str().unwrap_or("");
            if status == "Allowed" {
                ext_set("on", "Windows разрешила доступ. Уведомления выбранных программ будут показаны.");
            } else {
                ext_set("denied", &ext_denied_text(status));
            }
        }
        "note" => {
            // a notification is handled only while Windows has really granted the access
            if EXT_STATE.lock().state != "on" {
                return;
            }
            let texts: Vec<String> = match &v["texts"] {
                Value::Array(a) => a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect(),
                Value::String(s) => vec![s.clone()],
                _ => Vec::new(),
            };
            on_external(v["id"].as_str().unwrap_or(""), v["app"].as_str().unwrap_or(""), &texts, v["initial"].as_bool().unwrap_or(false));
        }
        "error" => {
            let msg: String = v["message"].as_str().unwrap_or("").chars().take(200).collect();
            ext_set("error", &format!("Не удалось получить доступ к уведомлениям Windows: {}. На этой версии Windows или для этой программы функция может быть недоступна.", msg));
        }
        _ => {}
    }
}

#[cfg(target_os = "windows")]
fn ext_start() {
    use std::io::{BufRead, BufReader};
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    if EXT_CHILD.lock().is_some() {
        return;
    }
    let gen = EXT_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let Some(script) = crate::APP_CONFIG_DIR.get().map(|d| d.join("ext_notifications.ps1")) else {
        ext_set("error", "Нет папки настроек");
        return;
    };
    if let Err(e) = std::fs::write(&script, EXT_SCRIPT) {
        ext_set("error", &format!("Не удалось подготовить проверку уведомлений: {}", e));
        return;
    }
    ext_set("starting", "Запрашиваю у Windows доступ к уведомлениям…");
    let child = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File"])
        .arg(&script)
        .args(["-ParentPid", &std::process::id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            ext_set("error", &format!("PowerShell не запустился: {}", e));
            return;
        }
    };
    let out = child.stdout.take();
    *EXT_CHILD.lock() = Some(child);
    std::thread::spawn(move || {
        if let Some(out) = out {
            for line in BufReader::new(out).split(b'\n').flatten() {
                if EXT_GEN.load(Ordering::SeqCst) != gen {
                    return;
                }
                handle_line(&String::from_utf8_lossy(&line));
            }
        }
        // the process has ended
        if EXT_GEN.load(Ordering::SeqCst) == gen {
            if let Some(mut c) = EXT_CHILD.lock().take() {
                let _ = c.wait();
            }
            ext_on_exit();
        }
    });
}

#[cfg(not(target_os = "windows"))]
fn ext_start() {
    let _ = EXT_SCRIPT;
    ext_set("unsupported", "Уведомления других программ доступны только в Windows 10 и 11.");
}

// the watcher process ended: a refusal or an error stays as it is; "working" never stays shown for a dead process
fn ext_on_exit() {
    let st = EXT_STATE.lock().state.clone();
    if st == "on" || st == "starting" {
        ext_set("error", "Наблюдение за уведомлениями остановилось. Выключите и включите переключатель, чтобы запустить его снова.");
    }
}

pub fn ext_on_exit_for_tests() {
    ext_on_exit()
}

pub fn handle_line_for_tests(line: &str) {
    handle_line(line)
}

#[cfg(test)]
mod tests;
