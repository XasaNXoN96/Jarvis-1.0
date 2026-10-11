use jarvis_core::slots;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;

// include core
use jarvis_core::{
    audio, audio_processing, commands, config, db, listener, recorder, stt, intent,
    ipc::{self, IpcAction},
    i18n, voices, models,
    APP_CONFIG_DIR, APP_LOG_DIR, COMMANDS_LIST, DB,
};

// include log
#[macro_use]
extern crate simple_log;
mod log;

// include app
mod app;

// include tray
// @TODO. macOS currently not supported for tray functionality.
#[cfg(not(target_os = "macos"))]
mod tray;

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

fn main() -> Result<(), String> {
    // `--background`: no console window (used by autostart); the tray icon stays
    #[cfg(target_os = "windows")]
    if std::env::args().any(|a| a == "--background") {
        unsafe {
            winapi::um::wincon::FreeConsole();
        }
    }

    // initialize directories
    config::init_dirs()?;

    // initialize logging
    log::init_logging()?;

    // log some base info
    info!("Starting Jarvis v{} ...", config::APP_VERSION.unwrap());
    info!("Config directory is: {}", APP_CONFIG_DIR.get().unwrap().display());
    info!("Log directory is: {}", APP_LOG_DIR.get().unwrap().display());

    // initialize settings
    let settings = db::init();

    // set global DB (for core modules that read settings at init time)
    DB.set(settings.arc().clone())
            .expect("DB already initialized");

    // speech output and timers saved by the previous run
    jarvis_core::tts::init();
    jarvis_core::lua::init_timers();

    // init voices
    let voice_id = settings.lock().voice.clone();
    let language = settings.lock().language.clone();
    if let Err(e) = voices::init(&voice_id, &language) {
        warn!("Failed to init voices: {}", e);
    }

    // init i18n
    i18n::init(&settings.lock().language);

    // run mode: voice (microphone) or text-only. `--text` forces text mode.
    let force_text = std::env::args().any(|a| a == "--text");
    let mut voice_mode = !force_text;
    if force_text {
        info!("Text mode forced by --text flag.");
        *jarvis_core::VOICE_ERROR.lock() = "программа запущена с ключом --text (только текст)".to_string();
    }

    // init recorder (no microphone -> text mode)
    if voice_mode && recorder::init().is_err() {
        warn!("Microphone is not available, switching to TEXT mode.");
        *jarvis_core::VOICE_ERROR.lock() = "микрофон не найден или занят другой программой (проверьте Параметры Windows → Конфиденциальность → Микрофон: разрешите доступ классическим приложениям)".to_string();
        voice_mode = false;
    }

    // init models registry (scans available AI models)
    if let Err(e) = models::init() {
        warn!("Models registry init failed: {}", e);
    }

    // init stt engine (no speech model -> text mode)
    if voice_mode {
        if let Err(e) = stt::init() {
            warn!("Speech recognition is not available ({}), switching to TEXT mode.", e);
            *jarvis_core::VOICE_ERROR.lock() = format!("не загрузилась модель распознавания речи ({}). Нужен большой установщик или папка vosk\\model рядом с программой", e);
            voice_mode = false;
        }
    }

    // init commands
    info!("Initializing commands.");
    let cmds = match commands::parse_commands() {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to parse commands: {}. Starting with empty command list.", e);
            Vec::new()
        }
    };
    info!("Commands initialized. Count: {}, List: {:?}", cmds.len(), commands::list_paths(&cmds));
    COMMANDS_LIST.set(cmds).unwrap();

    // init audio (sounds are optional, continue without them)
    if audio::init().is_err() {
        warn!("Audio output is not available, continuing without sounds.");
    }

    // init wake-word engine (needed for voice mode only)
    if voice_mode {
        if let Err(e) = listener::init() {
            warn!("Wake-word engine init failed ({}), switching to TEXT mode.", e);
            *jarvis_core::VOICE_ERROR.lock() = format!("не запустился поиск слова «Джарвис» ({})", e);
            voice_mode = false;
        }
    }

    // shared async runtime for intent classification, IPC, etc.
    let rt = Arc::new(
        tokio::runtime::Runtime::new().expect("Failed to create tokio runtime")
    );

    // init intent-recognition engine
    // (if it fails, commands are still matched by text similarity)
    rt.block_on(async {
        if let Err(e) = intent::init(COMMANDS_LIST.get().unwrap()).await {
            warn!("Intent classifier is not available ({}), using simple text matching.", e);
        }
    });

    // init slots parsing engine
    slots::init().map_err(|e| error!("Slot extraction init failed: {}", e)).ok();

    // init audio processing
    info!("Initializing audio processing...");
    if let Err(e) = audio_processing::init() {
        warn!("Audio processing init failed: {}", e);
    }

    // init IPC
    info!("Initializing IPC...");
    ipc::init();

    // channel for text commands (manually written in the GUI)
    let (text_cmd_tx, text_cmd_rx) = mpsc::channel::<String>();
    let console_tx = text_cmd_tx.clone();
    // phone / remote control feeds commands through the same channel (off until enabled)
    jarvis_core::remote::set_sender(text_cmd_tx.clone());

    ipc::set_action_handler(move |action| {
        match action {
            IpcAction::Stop => {
                info!("Received stop command from GUI");
                SHOULD_STOP.store(true, Ordering::SeqCst);
            }
            IpcAction::ReloadCommands => {
                info!("Received reload commands request");
                // TODO: implement reload
            }
            IpcAction::SetMuted { muted } => {
                info!("Received mute request: {}", muted);
                jarvis_core::MIC_MUTED.store(muted, Ordering::SeqCst);
            }
            IpcAction::Listen => {
                info!("Received listen request (orb click)");
                jarvis_core::MIC_MUTED.store(false, Ordering::SeqCst);
                jarvis_core::LISTEN_NOW.store(true, Ordering::SeqCst);
            }
            IpcAction::TextCommand { text } => {
                info!("Received text command: {}", text);
                if let Err(e) = text_cmd_tx.send(text) {
                    error!("Failed to send text command to app: {}", e);
                }
            }
            IpcAction::Ping => {
                // handled internally by server
            }
            _ => {}
        }
    });

    // start WebSocket server on the shared runtime
    let ipc_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        ipc_rt.block_on(ipc::start_server());
    });
    
    // the Jarvis window (chat, orb, commands): a local page shown in an app-style Edge/Chrome window.
    // `--background`: no window at start (tray menu "Open window"), `--no-ui`: no window at all,
    // `--console`: keep the console window open
    jarvis_core::ui::start();
    {
        let args: Vec<String> = std::env::args().collect();
        let has = |flag: &str| args.iter().any(|a| a == flag);
        if !has("--background") && !has("--no-ui") && jarvis_core::ui::open_window() {
            #[cfg(target_os = "windows")]
            if !has("--console") {
                // the window replaces the console
                unsafe {
                    winapi::um::wincon::FreeConsole();
                }
            }
        }
    }

    // console input: type a command and press Enter (works in both modes)
    std::thread::spawn(move || app::console_input(console_tx));

    // start the app (in the background thread)
    let app_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        let _ = app::start(voice_mode, text_cmd_rx, &app_rt);
    });

    tray::init_blocking(settings);

    Ok(())
}

pub fn request_stop() {
    SHOULD_STOP.store(true, Ordering::SeqCst);
}

pub fn should_stop() -> bool {
    SHOULD_STOP.load(Ordering::SeqCst)
}
