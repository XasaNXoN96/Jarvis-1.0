// Protection from doing the same thing twice by accident: a double click, a phone that sends a request again,
// a phrase heard twice, a script that asks for the same timer twice in a row.
//
// - first(key, window): true the first time a key is seen within the window, false for a repeat inside it.
//   The callers choose the key (what counts as "the same action") and the window; nothing is delayed,
//   a repeat is only skipped.
// - request_seen(rid): a request id sent by the window or the phone page; the same id is handled once.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

// the window for a repeated question to the AI and a repeated typed / spoken request
pub const REPEAT_WINDOW: Duration = Duration::from_millis(1200);
// the same program started again, the same timer set again
pub const ACTION_WINDOW: Duration = Duration::from_secs(4);
// the same notification shown again
pub const NOTIFY_WINDOW: Duration = Duration::from_secs(5);

const MAX_KEYS: usize = 512;
const MAX_REQUEST_IDS: usize = 256;

static RECENT: Lazy<Mutex<HashMap<String, Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));
static REQUEST_IDS: Lazy<Mutex<VecDeque<String>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

// the text of a request as a key: case, extra spaces and the punctuation at the end do not make it different
pub fn norm(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches(|c: char| c == '.' || c == '!' || c == '?' || c == ',' || c == '…')
        .to_string()
}

pub fn first(key: &str, window: Duration) -> bool {
    let now = Instant::now();
    let mut recent = RECENT.lock();
    if let Some(at) = recent.get(key) {
        if now.duration_since(*at) < window {
            info!("Repeat skipped: {}", key.chars().take(120).collect::<String>());
            return false;
        }
    }
    if recent.len() >= MAX_KEYS {
        // nothing here is older than a minute in use: drop the old keys
        recent.retain(|_, at| now.duration_since(*at) < Duration::from_secs(60));
    }
    recent.insert(key.to_string(), now);
    true
}

// forgets a key: the action did not happen after all (it failed), so trying again at once is not a repeat
pub fn forget(key: &str) {
    RECENT.lock().remove(key);
}

// forgets every key that begins with `prefix` (the script tests start each scenario from a clean state)
#[cfg(test)]
pub fn forget_prefix(prefix: &str) {
    RECENT.lock().retain(|k, _| !k.starts_with(prefix));
}

// true when this request id was already handled (an empty id is never "seen": old pages send none)
pub fn request_seen(rid: &str) -> bool {
    let rid = rid.trim();
    if rid.is_empty() {
        return false;
    }
    let mut ids = REQUEST_IDS.lock();
    if ids.iter().any(|r| r == rid) {
        info!("Repeated request id skipped: {}", rid);
        return true;
    }
    if ids.len() >= MAX_REQUEST_IDS {
        ids.pop_front();
    }
    ids.push_back(rid.to_string());
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_inside_window_is_skipped() {
        assert!(first("test:a", Duration::from_secs(10)));
        assert!(!first("test:a", Duration::from_secs(10)));
        // another key is not a repeat
        assert!(first("test:b", Duration::from_secs(10)));
    }

    #[test]
    fn repeat_after_window_is_allowed() {
        assert!(first("test:c", Duration::from_millis(30)));
        std::thread::sleep(Duration::from_millis(60));
        assert!(first("test:c", Duration::from_millis(30)));
    }

    #[test]
    fn forget_allows_again() {
        assert!(first("test:d", Duration::from_secs(10)));
        forget("test:d");
        assert!(first("test:d", Duration::from_secs(10)));
    }

    #[test]
    fn request_ids_once() {
        assert!(!request_seen("rid-1"));
        assert!(request_seen("rid-1"));
        assert!(!request_seen("rid-2"));
        // no id: never treated as a repeat
        assert!(!request_seen(""));
        assert!(!request_seen(""));
    }

    #[test]
    fn norm_ignores_case_spaces_and_end_punctuation() {
        assert_eq!(norm("  Сколько   будет 2 + 2? "), "сколько будет 2 + 2");
        assert_eq!(norm("Открой Word!"), norm("открой word"));
    }
}
