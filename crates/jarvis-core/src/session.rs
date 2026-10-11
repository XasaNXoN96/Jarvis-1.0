// A "session" is a conversation that goes on without the wake word: Jarvis asked what to do with a found file,
// or you are dictating into Word / Notepad / Excel. While a session is open every phrase (typed or spoken) goes
// to the session script (assistant_session) instead of being searched among the commands, until the script
// ends the session ("заканчивай") or it times out.

use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

struct Session {
    kind: String,
    data: String,
    last: Instant,
    timeout: Duration,
}

static SESSION: Lazy<Mutex<Option<Session>>> = Lazy::new(|| Mutex::new(None));

pub fn start(kind: &str, timeout_secs: u64) {
    info!("Session started: {} ({} s)", kind, timeout_secs);
    *SESSION.lock() = Some(Session { kind: kind.to_string(), data: String::new(), last: Instant::now(), timeout: Duration::from_secs(timeout_secs.max(5)) });
}

pub fn end() {
    if SESSION.lock().take().is_some() {
        info!("Session ended");
    }
}

// the kind of the open session (None when there is none or it has timed out)
pub fn kind() -> Option<String> {
    let mut guard = SESSION.lock();
    if let Some(s) = guard.as_ref() {
        if s.last.elapsed() > s.timeout {
            info!("Session '{}' timed out", s.kind);
            *guard = None;
            return None;
        }
    }
    guard.as_ref().map(|s| s.kind.clone())
}

pub fn touch() {
    if let Some(s) = SESSION.lock().as_mut() {
        s.last = Instant::now();
    }
}

pub fn data() -> String {
    SESSION.lock().as_ref().map(|s| s.data.clone()).unwrap_or_default()
}

pub fn set_data(value: &str) {
    if let Some(s) = SESSION.lock().as_mut() {
        s.data = value.to_string();
    }
}
