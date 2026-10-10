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
    }

    // init recorder (no microphone -> text mode)
    if voice_mode && recorder::init().is_err() {
        warn!("Microphone is not available, switching to TEXT mode.");
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
                // TODO: implement mute
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
