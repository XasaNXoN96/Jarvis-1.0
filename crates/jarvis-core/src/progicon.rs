//! The icons of the programs on the page «Программы». Where an icon comes from:
//!   1. the picture made before, kept in `prog_icons/` of the data folder (named by a hash of the key);
//!   2. the icon of the program file (Windows reads it; the file is only read);
//!   3. nothing: the page draws the first letter of the name, as before.
//! The page asks by a KEY, never by a path: `ess:<id>` (a fixed tool of Windows), `sc:<id>` (a shortcut the user added) or `app:<launch>` (a
//! program of the list that Jarvis itself made). A key that is not in those lists gives nothing, so the page cannot ask for the icon of an
//! arbitrary file. Nothing is downloaded and no program is started.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

static NONE: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));
static MAKING: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn dir() -> PathBuf {
    #[cfg(test)]
    {
        return std::env::temp_dir().join(format!("jarvis-prog-icons-{}", std::process::id()));
    }
    #[cfg(not(test))]
    {
        crate::APP_DIR.join("prog_icons")
    }
}

// the file name of a key: a hash, so that nothing of the key reaches the file system
fn file_name(key: &str) -> String {
    use sha2::{Digest, Sha256};
    let h = Sha256::digest(key.as_bytes());
    h.iter().take(16).map(|b| format!("{:02x}", b)).collect::<String>() + ".png"
}

// the program file of a fixed tool of Windows (the folders come from Windows itself)
fn essential_file(id: &str) -> Option<PathBuf> {
    let root = PathBuf::from(std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into()));
    let sys = root.join("System32");
    Some(match id {
        "cmd" => sys.join("cmd.exe"),
        "powershell" => sys.join("WindowsPowerShell").join("v1.0").join("powershell.exe"),
        "settings" => root.join("ImmersiveControlPanel").join("SystemSettings.exe"),
        "taskmgr" => sys.join("Taskmgr.exe"),
        "explorer" => root.join("explorer.exe"),
        "control" => sys.join("control.exe"),
        "notepad" => sys.join("notepad.exe"),
        "calc" => sys.join("calc.exe"),
        "paint" => sys.join("mspaint.exe"),
        _ => return None,
    })
}

fn is_exe(p: &str) -> bool {
    p.to_lowercase().ends_with(".exe") && Path::new(p).is_file()
}

// the program file behind a key, only through the lists of the program itself
pub(crate) fn target_of(key: &str) -> Option<PathBuf> {
    if let Some(id) = key.strip_prefix("ess:") {
        if !crate::proglib::ESSENTIALS.iter().any(|e| e.0 == id) {
            return None;
        }
        return essential_file(id);
    }
    if let Some(id) = key.strip_prefix("sc:") {
        let s = crate::proglib::load().shortcuts.into_iter().find(|s| s.id == id)?;
        return is_exe(&s.path).then(|| PathBuf::from(s.path));
    }
    if let Some(launch) = key.strip_prefix("app:") {
        let a = crate::appindex::apps().into_iter().find(|a| a.launch == launch)?;
        return is_exe(&a.launch).then(|| PathBuf::from(a.launch));
    }
    None
}

fn cached(key: &str) -> Option<Vec<u8>> {
    let d = std::fs::read(dir().join(file_name(key))).ok()?;
    crate::gameicon::sniff(&d).map(|_| d)
}

fn keep(key: &str, data: &[u8]) {
    let d = dir();
    if std::fs::create_dir_all(&d).is_ok() {
        let _ = crate::write_atomic(&d.join(file_name(key)), data);
    }
}

// the PNG of the program (made once and kept); None: the page draws the letter
pub fn icon_for(key: &str) -> Option<Vec<u8>> {
    if key.len() > 600 || key.chars().any(|c| c.is_control()) {
        return None;
    }
    let path = target_of(key)?;
    if let Some(d) = cached(key) {
        return Some(d);
    }
    if NONE.lock().contains(key) {
        return None;
    }
    let _one = MAKING.lock();
    if let Some(d) = cached(key) {
        return Some(d);
    }
    match crate::gameicon::exe_icon_png(&path.to_string_lossy()) {
        Some(d) => {
            keep(key, &d);
            Some(d)
        }
        None => {
            NONE.lock().insert(key.to_string());
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_outside_the_lists_of_the_program_gives_nothing() {
        for k in ["", "ess:", "ess:../../etc/passwd", "ess:nobody", "sc:nobody", "app:C:\\Windows\\System32\\cmd.exe", "app:/etc/passwd", "C:\\Windows\\notepad.exe", "/etc/passwd", "ess:cmd\n"] {
            assert!(target_of(k).is_none(), "{:?}", k);
            assert!(icon_for(k).is_none(), "{:?}", k);
        }
        assert!(icon_for(&"a".repeat(700)).is_none());
    }

    #[test]
    fn the_fixed_tools_have_a_file_each_and_their_names_are_hashes() {
        for e in crate::proglib::ESSENTIALS.iter() {
            let p = target_of(&format!("ess:{}", e.0)).unwrap_or_else(|| panic!("{}", e.0));
            assert!(p.to_string_lossy().to_lowercase().ends_with(".exe"), "{}", e.0);
        }
        let n = file_name("ess:cmd");
        assert!(n.len() == 36 && n.ends_with(".png") && n[..32].bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(file_name("ess:cmd"), file_name("ess:calc"));
    }

    #[test]
    fn a_kept_picture_is_served_again_and_a_broken_one_is_not() {
        let key = "ess:calc";
        let png = crate::gameicon::encode_png(2, 2, &[255u8; 16]).unwrap();
        keep(key, &png);
        assert_eq!(cached(key), Some(png.clone()));
        assert_eq!(icon_for(key), Some(png));
        let broken = dir().join(file_name("ess:paint"));
        std::fs::create_dir_all(dir()).unwrap();
        std::fs::write(&broken, b"not a picture").unwrap();
        assert!(cached("ess:paint").is_none(), "a file that is not a picture is not served");
    }
}
