// Timer Lua API: run something later, even after the command script has finished
//
//   jarvis.timer.after(seconds, title, message) -> id
//   jarvis.timer.cancel_all()                   -> number of cancelled timers
//   jarvis.timer.count()                        -> number of active timers
//   jarvis.timer.list()                         -> { {id=, remaining=, message=}, ... }
//
// When a timer fires: a notification is shown, a sound is played and the message
// is printed to the console. Timers live in memory only (lost when Jarvis exits).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mlua::{Lua, Table};
use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::lua::api::system::show_notification;
use crate::voices;

struct TimerEntry {
    id: u64,
    deadline: Instant,
    message: String,
    cancelled: Arc<AtomicBool>,
}

static TIMERS: Lazy<Mutex<Vec<TimerEntry>>> = Lazy::new(|| Mutex::new(Vec::new()));
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

pub fn start_timer(seconds: u64, title: String, message: String) -> u64 {
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    let cancelled = Arc::new(AtomicBool::new(false));
    let deadline = Instant::now() + Duration::from_secs(seconds);

    TIMERS.lock().push(TimerEntry {
        id,
        deadline,
        message: message.clone(),
        cancelled: Arc::clone(&cancelled),
    });

    info!("Timer #{} started: {} s ({})", id, seconds, message);

    std::thread::spawn(move || {
        // sleep in small steps so cancel_all() takes effect quickly
        while Instant::now() < deadline {
            if cancelled.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        if cancelled.load(Ordering::SeqCst) {
            return;
        }

        TIMERS.lock().retain(|t| t.id != id);

        info!("Timer #{} fired: {}", id, message);
        {
            use std::io::Write;
            let _ = writeln!(std::io::stdout(), "!!! {}: {}", title, message);
        }
        show_notification(&title, &message);

        // make sure the user notices: sound a few times
        for _ in 0..3 {
            voices::play_reply();
            std::thread::sleep(Duration::from_secs(2));
        }
    });

    id
}

pub fn cancel_all() -> usize {
    let mut timers = TIMERS.lock();
    let n = timers.len();
    for t in timers.iter() {
        t.cancelled.store(true, Ordering::SeqCst);
    }
    timers.clear();
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

    timer.set("cancel_all", lua.create_function(|_, ()| Ok(cancel_all()))?)?;
    timer.set("count", lua.create_function(|_, ()| Ok(count()))?)?;

    timer.set(
        "list",
        lua.create_function(|lua, ()| {
            let out = lua.create_table()?;
            let now = Instant::now();
            for (i, t) in TIMERS.lock().iter().enumerate() {
                let row = lua.create_table()?;
                row.set("id", t.id)?;
                row.set("remaining", t.deadline.saturating_duration_since(now).as_secs())?;
                row.set("message", t.message.as_str())?;
                out.set(i + 1, row)?;
            }
            Ok(out)
        })?,
    )?;

    jarvis.set("timer", timer)?;
    Ok(())
}
