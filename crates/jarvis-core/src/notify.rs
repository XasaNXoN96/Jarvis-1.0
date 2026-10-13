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
        // the phone and the Telegram bot of the user learn about it too (a timer rang, a task finished, an error)
        // (the result of a command reaches the phone as the answer to its command, not as one more event)
        if matches!(kind, "timer" | "alarm" | "reminder" | "important" | "system_error" | "ai_error") {
            crate::remote::announce_note(kind, &note.title, &note.text);
        }
        if matches!(kind, "timer" | "alarm" | "reminder") {
            crate::telegram::announce(&note.title, &note.text);
        }
        // the event list of the program (diagnostics) never gets the text of other programs' notifications
        crate::uistate::log_event(if kind.ends_with("error") { "error" } else { "info" }, &format!("Уведомление: {} — {}", note.title, note.text));
    }
    // the automatic tests never open windows or beep on the test machine; the voice they DO reach: the voice of the tests
    // records what was handed to it (tts::TEST_SPOKEN), so the whole way from a notification to the voice is checked
    if p.show && !cfg!(test) {
        display(&note);
    }
    if p.speak {
        if interrupts_speech(kind) {
            // the time has come (a timer, an alarm, a reminder): said at once
            crate::tts::speak(say);
        } else {
            // everything else (a message from another program, the result of a command, an error) waits for the
            // current speech: an AI answer that is being read is not cut off by a notification
            crate::tts::speak_queued(say);
        }
    } else if p.sound && !cfg!(test) {
        beep();
    }
    id
}

// only an event whose moment matters interrupts what Jarvis is saying
pub fn interrupts_speech(kind: &str) -> bool {
    matches!(kind, "timer" | "alarm" | "reminder")
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

// Does an entry of the allowed / excluded list name this program? Windows reports a display name
// ("Telegram Desktop") and an application id ("TelegramMessengerLLP.TelegramDesktop_...!Telegram.TelegramDesktop.Store",
// "Telegram.TelegramDesktop"): the user writes "Telegram". An entry matches whole words of the name (any case) or the
// beginning of a part of the id, never the middle of a word ("Mail" does not match "Gmail").
pub fn app_matches(entry: &str, name: &str, aumid: &str) -> bool {
    let words = |t: &str| -> Vec<String> { t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect() };
    let e = words(entry);
    if e.is_empty() {
        return false;
    }
    let n = words(name);
    // all the words of the entry appear in the name, in order and next to each other
    if n.windows(e.len()).any(|w| w == e.as_slice()) {
        return true;
    }
    let joined: String = e.concat();
    words(aumid).iter().any(|part| part.starts_with(&joined))
}

// is a notification of this program let through? (pure, tested)
pub fn ext_allowed(s: &UiSettings, app: &str) -> bool {
    ext_allowed_id(s, app, "")
}

pub fn ext_allowed_id(s: &UiSettings, app: &str, aumid: &str) -> bool {
    if !s.ext_enabled {
        return false;
    }
    let (app, aumid) = (app.trim(), aumid.trim());
    if app.is_empty() && aumid.is_empty() {
        return false; // an unrecognized program is never let through
    }
    let listed = |list: &[String]| list.iter().any(|x| app_matches(x, app, aumid));
    if listed(&s.ext_blocked) {
        return false;
    }
    if s.ext_strict {
        // a program that is not on the list is never let through automatically
        return listed(&s.ext_allowed);
    }
    true
}

// the texts of a notification without empty lines
pub fn ext_parts(texts: &[String]) -> Vec<String> {
    texts.iter().map(|t| t.split_whitespace().collect::<Vec<_>>().join(" ")).filter(|t| !t.is_empty()).collect()
}

// (title, text) of an external notification; a notification without any text still says where it comes from
pub fn ext_texts(app: &str, texts: &[String]) -> (String, String) {
    let parts = ext_parts(texts);
    let app = if app.trim().is_empty() { "Неизвестная программа".to_string() } else { app.trim().to_string() };
    match parts.len() {
        0 => (app, "Новое уведомление (без текста)".to_string()),
        1 => (app, parts[0].clone()),
        _ => (format!("{} · {}", app, parts[0]), parts[1..].join(" — ")),
    }
}

// a readable program name when Windows gives only the application id
pub fn app_label(name: &str, aumid: &str) -> String {
    if !name.trim().is_empty() {
        return name.trim().to_string();
    }
    // "TelegramMessengerLLP.TelegramDesktop_t4vj0pshhgkwm!Telegram.TelegramDesktop.Store" -> "TelegramDesktop"
    let main = aumid.split('!').next().unwrap_or("");
    let part = main.rsplit('.').next().unwrap_or(main);
    part.split('_').next().unwrap_or(part).trim().to_string()
}

// ---- what happened to the notifications of other programs: counts and the last stage, never their text ----
#[derive(Clone, Default, Serialize, Debug)]
pub struct ExtStats {
    pub polls: u64,         // times Windows was asked
    pub in_center: u64,     // notifications in the notification center at the last check
    pub received: u64,      // new notifications received
    pub unknown_app: u64,   // the program could not be recognized: not handled
    pub not_allowed: u64,   // not on the allowed list / excluded
    pub no_text: u64,       // the text is empty or hidden: shown, not spoken
    pub repeats: u64,
    pub shown: u64,
    pub queued_speech: u64, // handed to the voice queue (Piper)
    pub not_spoken: u64,    // should have been spoken but could not (see last_stage)
    pub last_stage: String,
    pub last_app: String,   // only the program name
    pub last_at: u64,
}

static EXT_STATS: Lazy<Mutex<ExtStats>> = Lazy::new(|| Mutex::new(ExtStats::default()));

fn stage(app: &str, text: &str, count: impl FnOnce(&mut ExtStats)) {
    let mut st = EXT_STATS.lock();
    count(&mut st);
    st.last_stage = text.to_string();
    if !app.is_empty() {
        st.last_app = app.chars().take(80).collect();
    }
    st.last_at = now();
    info!("Notifications of other programs: {} ({})", text, app); // the stage and the program, never the text
}

pub fn ext_stats() -> ExtStats {
    EXT_STATS.lock().clone()
}

static EXT_IDS: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));

// one notification reported by Windows. `initial`: it was already in the notification center when the watcher
// started (it is not shown again, only the program name is remembered for the list of the settings).
pub fn on_external(id: &str, app: &str, aumid: &str, texts: &[String], initial: bool) -> Option<u64> {
    let label = app_label(app, aumid);
    crate::uistate::remember_app(&label);
    {
        let mut ids = EXT_IDS.lock();
        if !ids.insert(id.to_string()) {
            drop(ids);
            stage(&label, "повтор — пропущено", |s| s.repeats += 1);
            return None; // the same notification again: never shown or read twice
        }
        if ids.len() > 2000 {
            ids.clear();
        }
    }
    if initial {
        return None;
    }
    if label.is_empty() {
        stage("", "уведомление получено, но программа не распознана — не обработано", |s| { s.received += 1; s.unknown_app += 1; });
        return None;
    }
    stage(&label, "уведомление получено, программа распознана", |s| s.received += 1);
    let s = crate::uistate::settings();
    // a social network: by its program, or a browser notification of its site (social.rs)
    let net = crate::social::detect(&label, aumid, texts);
    let from_browser = crate::social::is_browser(&label, aumid);
    let social_on = |n: &crate::social::Network| s.ext_enabled && s.ext_social.iter().any(|k| k == n.key) && !s.ext_blocked.iter().any(|b| app_matches(b, n.title, ""));
    let allowed = match net {
        Some(n) if social_on(n) => true,
        // a browser lets its own notifications through only when the browser itself is allowed
        _ => ext_allowed_id(&s, &label, aumid),
    };
    if !allowed {
        stage(&label, "программа не в списке разрешённых или исключена — не показано", |s| s.not_allowed += 1);
        return None; // not allowed: the content is dropped here, it goes nowhere
    }
    let texts: Vec<String> = match net {
        Some(n) if from_browser => crate::social::without_site_lines(n, texts),
        _ => texts.to_vec(),
    };
    // the network's name instead of "Google Chrome" / "Telegram Desktop"
    let shown = net.filter(|n| social_on(n)).map(|n| n.title.to_string()).unwrap_or_else(|| label.clone());
    let (title, text) = ext_texts(&shown, &texts);
    // a program that posts the same text twice with two ids
    if !crate::guard::first(&format!("ext:{}:{}", crate::guard::norm(&title), crate::guard::norm(&text)), crate::guard::NOTIFY_WINDOW) {
        stage(&label, "тот же текст повторно — пропущено", |s| s.repeats += 1);
        return None;
    }
    let has_text = !ext_parts(&texts).is_empty();
    let mut prefs = prefs_external(&s);
    // each network has its own "read aloud"; other programs follow the common switch
    if let Some(n) = net.filter(|n| social_on(n)) {
        prefs.speak = s.ext_social_speak.iter().any(|k| k == n.key);
    }
    let want_speak = prefs.speak;
    let mut p = plan(&prefs, false, false);
    if !has_text {
        // empty or hidden by the program ("show text" off in Telegram): shown, never spoken
        p.speak = false;
        stage(&label, "текст недоступен (пустой или скрыт программой) — показано без озвучки", |s| s.no_text += 1);
    } else if want_speak && !p.speak {
        let why = if s.dnd { "включён режим «Не беспокоить» — не озвучено" } else if !crate::tts::is_enabled() { "озвучка выключена — не озвучено" } else { "озвучивание уведомлений выключено" };
        stage(&label, why, |s| s.not_spoken += 1);
    } else if p.speak {
        if let Err(why) = crate::tts::can_speak() {
            p.speak = false;
            p.sound = s.notify_sound && !s.dnd;
            stage(&label, &format!("не удалось озвучить: {}", why), |s| s.not_spoken += 1);
        } else {
            stage(&label, "текст передан в очередь озвучки (Piper)", |s| s.queued_speech += 1);
        }
    }
    if p.show {
        EXT_STATS.lock().shown += 1;
    }
    // the page "Сообщения" (memory only)
    crate::social::push(net.filter(|n| social_on(n)).map(|n| n.key).unwrap_or(""), &shown, &title, &text);
    let say: String = if net.filter(|n| social_on(n)).is_some() {
        // "Telegram, Иван: привет, как дела?"
        format!("{}: {}", title.replace(" · ", ", "), text)
    } else {
        format!("Уведомление от {}. {}", shown, text)
    }
    .chars()
    .take(220)
    .collect();
    Some(deliver("external", &title, &text, &shown, false, p, &say))
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
    json!({ "state": st.state, "detail": st.detail, "stats": ext_stats(), "playback": crate::tts::last_play_json() })
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
    Emit @{ type = 'poll'; count = $list.Count }
    foreach ($n in $list) {
      $key = [string]$n.Id
      if ($seen.ContainsKey($key)) { continue }
      $seen[$key] = $true
      $app = ''
      $aumid = ''
      try { $aumid = [string]$n.AppInfo.AppUserModelId } catch {}
      try { $app = [string]$n.AppInfo.DisplayInfo.DisplayName } catch {}
      $texts = @()
      try {
        $b = $n.Notification.Visual.GetBinding([Windows.UI.Notifications.KnownNotificationBindings]::ToastGeneric)
        if ($b) { $texts = @($b.GetTextElements() | ForEach-Object { [string]$_.Text }) }
      } catch {}
      Emit @{ type = 'note'; id = $key; app = $app; aumid = $aumid; texts = $texts; initial = $first }
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
            on_external(v["id"].as_str().unwrap_or(""), v["app"].as_str().unwrap_or(""), v["aumid"].as_str().unwrap_or(""), &texts, v["initial"].as_bool().unwrap_or(false));
        }
        // one check of the notification center: only how many notifications are there, nothing of their content
        "poll" => {
            let n = v["count"].as_u64().unwrap_or(0);
            let mut st = EXT_STATS.lock();
            st.polls += 1;
            st.in_center = n;
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
