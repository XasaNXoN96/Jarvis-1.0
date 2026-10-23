//! One place for the small JSON files of the new features (memory, contacts, messages, notes…): they live in the settings folder of the
//! program, are written atomically, and a file that cannot be parsed is moved aside (never silently overwritten, never a crash).

use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;

#[cfg(test)]
pub static TEST_DIR: parking_lot::Mutex<Option<PathBuf>> = parking_lot::const_mutex(None);
#[cfg(test)]
pub static TEST_LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

pub fn file(name: &str) -> PathBuf {
    #[cfg(test)]
    if let Some(d) = TEST_DIR.lock().clone() {
        return d.join(name);
    }
    crate::APP_CONFIG_DIR.get().cloned().unwrap_or_else(|| PathBuf::from(".")).join(name)
}

// (the value, true when a damaged file was moved aside)
pub fn read<T: DeserializeOwned + Default>(name: &str) -> (T, bool) {
    let p = file(name);
    let Ok(text) = crate::retry_transient(|| std::fs::read_to_string(&p)) else { return (T::default(), false) };
    match serde_json::from_str::<T>(text.trim_start_matches('\u{feff}')) {
        Ok(v) => (v, false),
        Err(_) => {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let _ = std::fs::rename(&p, p.with_extension(format!("corrupt-{}", stamp)));
            (T::default(), true)
        }
    }
}

pub fn write<T: Serialize>(name: &str, v: &T) -> Result<(), String> {
    let p = file(name);
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(v).map_err(|e| e.to_string())?;
    crate::write_atomic(&p, text).map_err(|e| format!("не удалось сохранить {}: {}", name, e))
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// A number for a new item: the second, a counter of this run of the program and a random part. The counter makes two numbers of the
// same second differ for certain (the random part alone is 16 bits: among 80 items made in one second two were the same about once in twenty times).
pub fn new_id() -> String {
    static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    format!("{:x}{:04x}{:04x}", now(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) & 0xffff, rand::random::<u16>())
}

#[cfg(test)]
pub fn with_temp<R>(f: impl FnOnce() -> R) -> R {
    let _g = TEST_LOCK.lock();
    let dir = tempfile::tempdir().unwrap();
    *TEST_DIR.lock() = Some(dir.path().to_path_buf());
    let r = f();
    *TEST_DIR.lock() = None;
    r
}

#[cfg(test)]
mod id_tests {
    use super::*;

    #[test]
    fn numbers_made_in_one_run_never_repeat() {
        // (fewer than the counter can tell apart: inside one second two numbers can only be the same if the counter wrapped)
        let n = 20_000;
        let set: std::collections::HashSet<String> = (0..n).map(|_| new_id()).collect();
        assert_eq!(set.len(), n, "two items got the same number");
        assert!(set.iter().all(|i| i.len() <= 24 && i.chars().all(|c| c.is_ascii_hexdigit())), "a number stays short and hexadecimal");
    }

    #[test]
    fn many_numbers_from_many_threads_stay_different() {
        let handles: Vec<_> = (0..8).map(|_| std::thread::spawn(|| (0..2_000).map(|_| new_id()).collect::<Vec<_>>())).collect();
        let all: Vec<String> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
        let set: std::collections::HashSet<&String> = all.iter().collect();
        assert_eq!(set.len(), all.len());
    }
}
