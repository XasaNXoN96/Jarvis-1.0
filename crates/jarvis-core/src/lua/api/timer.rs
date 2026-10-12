// Timer Lua API: run something later, even after the command script has finished
//
//   jarvis.timer.after(seconds, title, message) -> id
//   jarvis.timer.at(unix_time, title, message)  -> id   (alarm at a clock time)
//   jarvis.timer.cancel_all()                   -> number of cancelled timers
//   jarvis.timer.count()                        -> number of active timers
//   jarvis.timer.list()                         -> { {id=, remaining=, message=}, ... }
//
// When a timer fires: the notification of Jarvis is shown (bottom right of the screen), the message is spoken and/or
// a sound is played, as the notification settings say (notify.rs), and the message is printed to the console.
// Timers are saved to <config dir>/timers.json and restored on the next start
// (init()), so closing Jarvis does not lose them.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use mlua::{Lua, Table};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::APP_CONFIG_DIR;

#[derive(Clone, Serialize, Deserialize)]
struct TimerEntry {
    id: u64,
    due: u64, // unix seconds
    title: String,
    message: String,
}

static TIMERS: Lazy<Mutex<Vec<TimerEntry>>> = Lazy::new(|| Mutex::new(Vec::new()));

// overdue timers older than this are dropped on start instead of fired
const MAX_OVERDUE_SECS: u64 = 12 * 3600;

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn file_path() -> Option<std::path::PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join("timers.json"))
}

fn save(timers: &[TimerEntry]) {
    if let Some(p) = file_path() {
        match serde_json::to_string_pretty(timers) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&p, json) {
                    warn!("Cannot save timers to {}: {}", p.display(), e);
                }
            }
            Err(e) => warn!("Cannot serialize timers: {}", e),
        }
    }
}

fn next_id(timers: &[TimerEntry]) -> u64 {
    timers.iter().map(|t| t.id).max().unwrap_or(0) + 1
}

fn fire(entry: &TimerEntry, late: bool) {
    let message = if late {
        format!("{} ({})", entry.message, if crate::i18n::get_language() == "en" { "missed while Jarvis was off" } else { "пропущено, пока Jarvis был выключен" })
    } else {
        entry.message.clone()
    };
    info!("Timer #{} fired: {}", entry.id, message);
    {
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "!!! {}: {}", entry.title, message);
    }
    #[cfg(feature = "jarvis_app")]
    crate::ipc::send(crate::ipc::IpcEvent::Output { text: format!("{}: {}", entry.title, message) });
    // one notification: the window of Jarvis, the voice (said once) or a short sound, "do not disturb" respected
    crate::notify::push(crate::notify::timer_kind(&entry.title), &entry.title, &message, false);
}

// run the timer in the background; the entry disappears if it is cancelled meanwhile
fn spawn(entry: TimerEntry) {
    std::thread::spawn(move || {
        loop {
            // still registered? (cancel_all removes it)
            if !TIMERS.lock().iter().any(|t| t.id == entry.id) {
                return;
            }
            if now_unix() >= entry.due {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        {
            let mut timers = TIMERS.lock();
            timers.retain(|t| t.id != entry.id);
            save(&timers);
        }
        fire(&entry, false);
    });
}

pub fn start_at(due: u64, title: String, message: String) -> u64 {
    let entry = {
        let mut timers = TIMERS.lock();
        // the same timer asked for twice in a row (a phrase heard twice, a double press): one timer
        let key = format!("timer:{}:{}", title, message);
        if !crate::guard::first(&key, crate::guard::ACTION_WINDOW) {
            let window = crate::guard::ACTION_WINDOW.as_secs();
            if let Some(same) = timers.iter().find(|t| t.title == title && t.message == message && t.due.abs_diff(due) <= window) {
                info!("Timer #{} already set, the repeat is not a new timer", same.id);
                return same.id;
            }
        }
        let entry = TimerEntry { id: next_id(&timers), due, title, message };
        timers.push(entry.clone());
        save(&timers);
        entry
    };
    info!("Timer #{} set for unix {} ({})", entry.id, entry.due, entry.message);
    let id = entry.id;
    spawn(entry);
    id
}

pub fn start_timer(seconds: u64, title: String, message: String) -> u64 {
    start_at(now_unix() + seconds, title, message)
}

// restore timers saved by the previous run; call once after the config dir is known
// for the Jarvis window: active timers as JSON rows
pub fn list_json() -> Vec<serde_json::Value> {
    let now = now_unix();
    let mut timers = TIMERS.lock().clone();
    timers.sort_by_key(|t| t.due);
    timers
        .iter()
        .map(|t| serde_json::json!({ "id": t.id, "remaining": t.due.saturating_sub(now), "title": t.title, "message": t.message }))
        .collect()
}

pub fn cancel_one(id: u64) -> bool {
    let mut timers = TIMERS.lock();
    let before = timers.len();
    timers.retain(|t| t.id != id);
    let changed = timers.len() != before;
    if changed {
        save(&timers);
    }
    changed
}

pub fn init() {
    let path = match file_path() {
        Some(p) => p,
        None => return,
    };
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    let saved: Vec<TimerEntry> = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            warn!("Cannot read {}: {}", path.display(), e);
            return;
        }
    };

    let now = now_unix();
    let mut late = Vec::new();
    let mut waiting = Vec::new();
    for t in saved {
        if t.due > now {
            waiting.push(t);
        } else if now - t.due <= MAX_OVERDUE_SECS {
            late.push(t);
        }
    }
    info!("Timers restored: {} waiting, {} missed", waiting.len(), late.len());

    {
        let mut timers = TIMERS.lock();
        *timers = waiting.clone();
        save(&timers);
    }
    for t in waiting {
        spawn(t);
    }
    if !late.is_empty() {
        std::thread::spawn(move || {
            for t in late {
                fire(&t, true);
            }
        });
    }
}

pub fn cancel_all() -> usize {
    let mut timers = TIMERS.lock();
    let n = timers.len();
    timers.clear();
    save(&timers);
    n
}

// only the timers of one kind ("Игровой режим"): the reminders of the user stay
pub fn cancel_titled(title: &str) -> usize {
    let mut timers = TIMERS.lock();
    let before = timers.len();
    timers.retain(|t| t.title != title);
    let n = before - timers.len();
    save(&timers);
    n
}

pub fn count() -> usize {
    TIMERS.lock().len()
}

pub fn register(lua: &Lua, jarvis: &Table) -> mlua::Result<()> {
    let timer = lua.create_table()?;

    timer.set(
        "after",
        lua.create_function(|_, (seconds, title, message): (f64, String, String)| {
            if !(seconds > 0.0) {
                return Err(mlua::Error::runtime("timer: seconds must be positive"));
            }
            Ok(start_timer(seconds.round() as u64, title, message))
        })?,
    )?;

    timer.set(
        "at",
        lua.create_function(|_, (unix, title, message): (f64, String, String)| {
            if unix <= now_unix() as f64 {
                return Err(mlua::Error::runtime("timer: time is in the past"));
            }
            Ok(start_at(unix as u64, title, message))
        })?,
    )?;

    timer.set("cancel_all", lua.create_function(|_, ()| Ok(cancel_all()))?)?;
    // jarvis.timer.cancel_titled(title) -> how many were cancelled
    timer.set("cancel_titled", lua.create_function(|_, title: String| Ok(cancel_titled(&title)))?)?;
    timer.set("count", lua.create_function(|_, ()| Ok(count()))?)?;

    timer.set(
        "list",
        lua.create_function(|lua, ()| {
            let out = lua.create_table()?;
            let now = now_unix();
            let mut timers = TIMERS.lock().clone();
            timers.sort_by_key(|t| t.due);
            for (i, t) in timers.iter().enumerate() {
                let row = lua.create_table()?;
                row.set("id", t.id)?;
                row.set("remaining", t.due.saturating_sub(now))?;
                row.set("due", t.due)?;
                row.set("message", t.message.as_str())?;
                out.set(i + 1, row)?;
            }
            Ok(out)
        })?,
    )?;

    jarvis.set("timer", timer)?;
    Ok(())
}
