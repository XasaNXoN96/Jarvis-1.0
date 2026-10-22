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
    let Ok(text) = std::fs::read_to_string(&p) else { return (T::default(), false) };
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

pub fn new_id() -> String {
    format!("{:x}{:04x}", now(), rand::random::<u16>())
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
