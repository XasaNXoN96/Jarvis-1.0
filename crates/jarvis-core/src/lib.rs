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

pub mod remote;
pub mod ui;
pub mod uistate;
pub mod route;
pub mod apps;
pub mod dialog;
pub mod health;
pub mod weather;
pub mod session;
pub mod secret;
pub mod voice_install;
pub mod ai;
pub mod filesearch;
pub mod guard;
pub mod instance;
pub mod input;
pub mod scenario;
pub mod notify;
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