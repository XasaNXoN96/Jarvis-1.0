use once_cell::sync::{Lazy, OnceCell};
use parking_lot::RwLock;
use std::{sync::Arc};
use platform_dirs::AppDirs;
use std::path::PathBuf;

#[macro_use]
extern crate log;

pub mod time;

pub mod audio;
pub mod commands;
pub mod config;
pub mod db;
pub mod i18n;

#[cfg(feature = "jarvis_app")]
pub mod listener;

pub mod recorder;

#[cfg(feature = "jarvis_app")]
pub mod stt;

#[cfg(feature = "intent")]
pub mod intent;

#[cfg(feature = "jarvis_app")]
pub mod slots;

pub mod models;

// re-exported from models/
pub use models::vosk_models;
pub use models::gliner_models;

#[cfg(feature = "jarvis_app")]
pub mod audio_processing;

#[cfg(feature = "jarvis_app")]
pub mod ipc;

pub mod voices;

pub mod audio_buffer;

pub mod tts;

pub mod origin;
pub mod confirm;
pub mod integrity;
pub mod policy;
pub mod remote;
pub mod ui;
pub mod ui_auth;
pub mod psrun;
pub mod acl;
pub mod safefs;
pub mod uistate;
pub mod route;
pub mod apps;
pub mod appindex;
pub mod dialog;
pub mod health;
pub mod devices;
pub mod weather;
pub mod session;
pub mod objects;
pub mod secret;
pub mod voice_install;
pub mod ai;
pub mod filesearch;
pub mod guard;
pub mod instance;
pub mod input;
pub mod scenario;
pub mod tasks;
pub mod telegram;
pub mod tgvoice;
pub mod ascii_path;
pub mod tgmenu;
pub mod tunnel;
pub mod autostart;
pub mod screen;
pub mod sites;
pub mod notify;
pub mod social;
#[cfg(target_os = "windows")]
pub mod notify_win;
pub mod diag;

#[cfg(feature = "lua")]
pub mod lua;

// shared statics
// pub static APP_DIR: Lazy<PathBuf> = Lazy::new(|| std::env::current_dir().unwrap());
pub static APP_DIR: Lazy<PathBuf> = Lazy::new(|| {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_else(|| std::env::current_dir().unwrap())
});
pub static SOUND_DIR: Lazy<PathBuf> = Lazy::new(|| APP_DIR.clone().join(config::SOUND_PATH));
pub static APP_DIRS: OnceCell<AppDirs> = OnceCell::new();
pub static APP_CONFIG_DIR: OnceCell<PathBuf> = OnceCell::new();
pub static APP_LOG_DIR: OnceCell<PathBuf> = OnceCell::new();
pub static DB: OnceCell<Arc<RwLock<db::structs::Settings>>> = OnceCell::new();
// microphone switched off from the tray / a command: Jarvis keeps working from text input
pub static MIC_MUTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// real state of the voice mode (the window shows it): active, or why it fell back to text mode
pub static VOICE_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static VOICE_ERROR: Lazy<parking_lot::Mutex<String>> = Lazy::new(|| parking_lot::Mutex::new(String::new()));
// peak of the last microphone frame, 0..32767 (live level meter in the settings)
// Jarvis is listening for a command right now (after the wake word / a click on the sphere)
pub static VOICE_LISTENING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// started with the "text only" key on purpose: the voice is switched off, that is not an error
pub static VOICE_TEXT_ONLY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// how many commands are loaded (for the "Intelligence hub" card)
pub static COMMANDS_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub static MIC_LEVEL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
// click on the orb: listen for a command at once (consumed by the voice loop)
pub static LISTEN_NOW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub static COMMANDS_LIST: OnceCell<Vec<JCommandsList>> = OnceCell::new();

// re-exports
pub use commands::JCommandsList;
pub use config::structs::*;
pub use db::structs::Settings;
pub use db::SettingsManager;

// use crate::commands::{JComandsList, JCommand};

#[cfg(test)]
mod installer_tests;

// Writes a file so that a crash or a power cut in the middle never leaves it cut off: the content goes to a
// temporary file next to it, which then replaces the old one in one step (settings, timers, tasks, notes).
// A child process that must not share the handles of Jarvis: no standard input, output or error of ours is passed on.
// Jarvis is a program without a console; a standard handle it was started with (by the installer, by Explorer, by a
// terminal) may be of a kind Windows cannot copy into a child, and the start of the child then fails with
// "os error 50" (the request is not supported). With the three streams closed (null) nothing is copied.
pub fn null_stdio(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::process::Stdio;
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
}

// A file for a download or a probe that is certainly NEW: whatever stood under that name (an old partial file, a file or a link
// somebody left there) is removed first (a link is removed itself, never followed) and the file is created with create_new,
// so the bytes cannot go through a link planted between the two steps.
pub fn create_fresh(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.is_dir() => return Err(std::io::Error::new(std::io::ErrorKind::Other, "на этом месте папка")),
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)
}

pub fn write_atomic(path: &std::path::Path, data: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::io::Write;
    // (a name of its own for every write: two writes of the same file at once never share the temporary file)
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    // The temporary file is made with create_new (O_EXCL / CREATE_NEW): a file or a link that somebody put under that name
    // in advance is never written through; a taken name is skipped. The rename then replaces the NAME of the target (a link
    // there is replaced, its destination is not touched).
    let mut last = std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free temporary name");
    for _ in 0..16 {
        let mut tmp = path.as_os_str().to_owned();
        tmp.push(format!(".{}-{}.tmp", std::process::id(), SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let tmp = std::path::PathBuf::from(tmp);
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                last = e;
                continue;
            }
            Err(e) => return Err(e),
        };
        let res = f.write_all(data.as_ref()).and_then(|_| f.flush());
        drop(f);
        let res = res.and_then(|_| std::fs::rename(&tmp, path));
        if let Err(e) = res {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        return Ok(());
    }
    Err(last)
}

#[cfg(test)]
mod write_atomic_tests {
    #[test]
    fn a_file_is_replaced_whole_and_no_temporary_file_stays() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Таймеры пользователя.json");
        super::write_atomic(&p, "[1]").unwrap();
        super::write_atomic(&p, "[1, 2]").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "[1, 2]");
        let names: Vec<String> = std::fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["Таймеры пользователя.json"], "no .tmp left behind");
    }

    // a link planted under the names the temporary file may get is never written through
    #[cfg(unix)]
    #[test]
    fn a_link_planted_as_the_temporary_file_is_not_written_through() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "original").unwrap();
        let p = dir.path().join("settings.json");
        for n in 0..400u64 {
            let name = format!("settings.json.{}-{}.tmp", std::process::id(), n);
            std::os::unix::fs::symlink(&victim, dir.path().join(name)).unwrap();
        }
        // (the counter is shared by the whole test run: some of the planted names may never come up, the point is that none is followed)
        let r = super::write_atomic(&p, "new data");
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original", "written through a planted link ({:?})", r);
        if r.is_ok() {
            assert_eq!(std::fs::read_to_string(&p).unwrap(), "new data");
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_fresh_file_is_never_written_through_a_link_or_an_old_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "original").unwrap();
        let p = dir.path().join("download.part");
        std::os::unix::fs::symlink(&victim, &p).unwrap();
        super::create_fresh(&p).unwrap().write_all(b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original", "written through a link");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new");
        // an old partial file is replaced, a folder is refused
        super::create_fresh(&p).unwrap().write_all(b"x").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "x");
        let d = dir.path().join("adir");
        std::fs::create_dir(&d).unwrap();
        assert!(super::create_fresh(&d).is_err());
    }

    // a link in the place of the target is replaced, its destination is untouched
    #[cfg(unix)]
    #[test]
    fn a_link_in_the_place_of_the_target_is_replaced_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim.txt");
        std::fs::write(&victim, "original").unwrap();
        let p = dir.path().join("settings.json");
        std::os::unix::fs::symlink(&victim, &p).unwrap();
        super::write_atomic(&p, "new data").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "original");
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "new data");
        assert!(!std::fs::symlink_metadata(&p).unwrap().file_type().is_symlink());
    }
}

