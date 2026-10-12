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

// the own window of Jarvis on Windows (WebView2)
#[cfg(target_os = "windows")]
mod window;

static SHOULD_STOP: AtomicBool = AtomicBool::new(false);

// the report of --selftest-speech: "OK heard: ..." or "FAIL <step>: <reason>"
fn selftest_speech() -> String {
    let settings = db::init();
    let _ = DB.set(settings.arc().clone());
    i18n::init(&settings.lock().language);
    // the registry of the models: the recognizer takes its model from it (the program does this at its start)
    if let Err(e) = models::init() {
        return format!("FAIL models: {}", e);
    }
    let t0 = std::time::Instant::now();
    let phrase = "открой ютуб и поставь таймер на пять минут";
    let wav = match jarvis_core::tts::synth_to_wav(phrase) {
        Ok(w) => w,
        Err(e) => return format!("FAIL piper: {}", e),
    };
    // the sound itself: its rate and length (a phrase of seven words must not last ten seconds), and two short phrases
    let wav_info = |w: &[u8]| -> String {
        match jarvis_core::tgvoice::wav_info(w) {
            Ok((rate, secs)) => format!("{} Hz, {:.1} s", rate, secs),
            Err(e) => format!("({})", e),
        }
    };
    let mut shorts = String::new();
    for short in ["открой ютуб", "который час"] {
        match jarvis_core::tts::synth_to_wav(short) {
            Ok(w) => {
                let heard = jarvis_core::tgvoice::wav_to_16k(&w).and_then(|s| jarvis_core::tgvoice::recognize(&s)).unwrap_or_else(|e| format!("(error: {})", e));
                shorts.push_str(&format!(" [«{}»: {} -> «{}»]", short, wav_info(&w), heard.trim()));
            }
            Err(e) => shorts.push_str(&format!(" [«{}»: {}]", short, e)),
        }
    }
    let sound = wav_info(&wav);
    // what Vosk makes of the sound without any codec (to tell a weak recognition from a damaged sound)
    let direct = match jarvis_core::tgvoice::wav_to_16k(&wav).and_then(|s| jarvis_core::tgvoice::recognize(&s)) {
        Ok(t) => t.trim().to_string(),
        Err(e) => format!("(error: {})", e),
    };
    let ogg = match jarvis_core::tgvoice::encode_wav_to_ogg_opus(&wav) {
        Ok(o) => o,
        Err(e) => return format!("FAIL opus-encode: {}", e),
    };
    let samples = match jarvis_core::tgvoice::decode_ogg_opus(&ogg) {
        Ok(s) => s,
        Err(e) => return format!("FAIL opus-decode: {}", e),
    };
    match jarvis_core::tgvoice::recognize(&samples) {
        Ok(text) if !text.trim().is_empty() => {
            format!("OK heard: {} | without the codec: {} | sound: {} |{} | wav {} KB, ogg {} KB, {} samples, {} ms", text.trim(), direct, sound, shorts, wav.len() / 1024, ogg.len() / 1024, samples.len(), t0.elapsed().as_millis())
        }
        Ok(_) => format!("FAIL vosk: nothing recognized (without the codec: {})", direct),
        Err(e) => format!("FAIL vosk: {}", e),
    }
}

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

    // `--selftest-speech <file>`: the real engines of the voice of the Telegram bot end to end, no network and no
    // window: Piper makes a phrase, it is packed as a voice message (Opus), unpacked and recognized by Vosk. The result
    // is written to the file; nothing is installed or changed. Used by the CI of the full installer.
    if let Some(i) = std::env::args().position(|a| a == "--selftest-speech") {
        let out = std::env::args().nth(i + 1).unwrap_or_else(|| "selftest-speech.txt".into());
        // (a panic is reported in the file as well: the CI must see the reason, not only an exit code)
        let report = match std::panic::catch_unwind(selftest_speech) {
            Ok(r) => r,
            Err(e) => format!("FAIL panic: {}", e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()),
        };
        let _ = std::fs::write(&out, &report);
        println!("{}", report);
        std::process::exit(if report.starts_with("OK") { 0 } else { 1 });
    }

    // one Jarvis at a time: a second start shows the window of the running one and quits
    // (two copies ran every spoken command twice and wrote every question and answer twice)
    {
        let restarting = std::env::args().any(|a| a == "--wait-instance");
        // (the old copy needs a few seconds to end: the speech model and the libraries are unloaded)
        let wait = std::time::Duration::from_secs(if restarting { 30 } else { 0 });
        if !jarvis_core::instance::acquire_or_wait(jarvis_core::instance::NAME, wait) {
            if !std::env::args().any(|a| a == "--background" || a == "--no-ui") {
                // the running Jarvis shows its own window. A browser window on its address would show "cannot reach
                // this page" while it is still starting, so none is opened: the user is told instead.
                if !ask_running_to_show() {
                    tell_still_starting();
                }
            }
            return Ok(());
        }
    }

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
    // saved privacy choices (microphone off), a message about an unreadable settings file, the notification watcher
    jarvis_core::uistate::init();
    jarvis_core::notify::ext_apply();
    jarvis_core::lua::init_timers();

    // init voices
    let voice_id = settings.lock().voice.clone();
    let language = settings.lock().language.clone();
    if let Err(e) = voices::init(&voice_id, &language) {
        warn!("Failed to init voices: {}", e);
    }
    {
        let rv = jarvis_core::uistate::settings().reaction_voice;
        if !rv.is_empty() { voices::set_current_voice(&rv); }
    }

    // init i18n
    i18n::init(&settings.lock().language);

    // run mode: voice (microphone) or text-only. `--text` forces text mode.
    let force_text = std::env::args().any(|a| a == "--text");
    let mut voice_mode = !force_text;
    if force_text {
        jarvis_core::VOICE_TEXT_ONLY.store(true, std::sync::atomic::Ordering::SeqCst);
        info!("Text mode forced by --text flag.");
        *jarvis_core::VOICE_ERROR.lock() = "Джарвис запущен ярлыком «только текст» (ключ --text). Нажмите «Включить голос» или запустите обычный ярлык «Jarvis»".to_string();
    }

    // init recorder (no microphone -> text mode)
    if voice_mode && recorder::init().is_err() {
        warn!("Microphone is not available, switching to TEXT mode.");
        let summary = jarvis_core::recorder::diag::LAST_REPORT.lock().summary.clone();
        *jarvis_core::VOICE_ERROR.lock() = if summary.is_empty() { "микрофон не запустился (подробности в журнале)".to_string() } else { summary };
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
    jarvis_core::COMMANDS_COUNT.store(cmds.iter().map(|l| l.commands.iter().filter(|c| c.id != "assistant_session").count()).sum(), std::sync::atomic::Ordering::Relaxed);
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
    // the Telegram bot of the user feeds the same queue (only when the user switched it on and paired a phone)
    jarvis_core::telegram::start();

    ipc::set_action_handler(move |action| {
        match action {
            IpcAction::Stop => {
                info!("Received stop command from GUI");
                // a full exit: stopping only the voice loop left the window and the tray without anyone to answer
                std::thread::spawn(|| shutdown_and_exit());
            }
            IpcAction::ReloadCommands => {
                info!("Received reload commands request");
                // TODO: implement reload
            }
            IpcAction::SetMuted { muted } => {
                info!("Received mute request: {}", muted);
                jarvis_core::uistate::set_mic_muted(muted);
            }
            IpcAction::Listen => {
                info!("Received listen request (orb click)");
                // a click on the sphere is a clear wish to talk: the microphone is switched on (and that is remembered)
                jarvis_core::uistate::set_mic_muted(false);
                jarvis_core::LISTEN_NOW.store(true, Ordering::SeqCst);
            }
            IpcAction::TextCommand { text } => {
                info!("Received text command: {} chars", text.chars().count());
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
    
    // "Полностью закрыть JARVIS" of the browser window goes the same way as the tray and the own window
    jarvis_core::ui::set_exit_hook(Box::new(|| shutdown_and_exit()));
    // Windows: the own window of Jarvis (window.rs) answers "show" / "hide" from the first request of the page server
    // on (a request before its loop runs is done at its start), so no browser window opens next to it
    #[cfg(target_os = "windows")]
    {
        jarvis_core::ui::set_window_host(Box::new(|| window::send(window::UserEvent::Show)));
        jarvis_core::ui::set_window_hide(Box::new(|| window::send(window::UserEvent::Hide)));
    }
    // the Jarvis window (chat, orb, commands): a local page shown in an app-style Edge/Chrome window.
    // `--background`: no window at start (tray menu "Open window"), `--no-ui`: no window at all,
    // `--console`: keep the console window open
    jarvis_core::ui::start();
    // the list of installed programs ("открой Photoshop"): read from its file, rebuilt in the background when old
    jarvis_core::appindex::ensure_loaded();
    // (on Windows the own window is made by the window loop at the end of main)
    #[cfg(not(target_os = "windows"))]
    {
        let args: Vec<String> = std::env::args().collect();
        let has = |flag: &str| args.iter().any(|a| a == flag);
        if !has("--background") && !has("--no-ui") {
            jarvis_core::ui::open_window();
        }
    }

    // console input: type a command and press Enter (works in both modes)
    std::thread::spawn(move || app::console_input(console_tx));

    // start the app (in the background thread)
    let app_rt = Arc::clone(&rt);
    std::thread::spawn(move || {
        jarvis_core::uistate::log_event("start", &format!("Jarvis {} запущен", config::APP_VERSION.unwrap_or("")));
        if voice_mode {
            jarvis_core::uistate::log_event("voice", "Голосовой режим: запускаю микрофон");
        } else {
            let why = jarvis_core::VOICE_ERROR.lock().clone();
            jarvis_core::uistate::log_event(if jarvis_core::VOICE_TEXT_ONLY.load(std::sync::atomic::Ordering::SeqCst) { "info" } else { "error" }, &format!("Голос выключен: {}", why.chars().take(140).collect::<String>()));
        }
        let _ = app::start(voice_mode, text_cmd_rx, &app_rt);
    });

    tray::init_blocking(settings);

    Ok(())
}

pub fn request_stop() {
    SHOULD_STOP.store(true, Ordering::SeqCst);
}

// "Полностью закрыть JARVIS" / "Выход": the wake word stops, the microphone and the speech are released, the phone
// remote is closed, then the process ends (the settings and the history are saved as they change)
pub fn shutdown_and_exit() -> ! {
    info!("Exit requested: stopping the microphone, the speech and the remote");
    request_stop();
    // an AI answer still arriving is stopped (its part so far is kept in the conversation) and no new phrase
    // starts while Jarvis exits (a Piper process would outlive it)
    if let Some(id) = jarvis_core::ai::store().active() {
        jarvis_core::ai::stop(&id);
    }
    jarvis_core::tts::shutdown();
    // released and never reopened by the background recovery
    jarvis_core::recorder::shutdown();
    jarvis_core::remote::stop();
    jarvis_core::telegram::stop();
    jarvis_core::uistate::log_event("start", "Jarvis закрыт пользователем");
    // the voice loop sees the stop flag; a short pause lets it leave the microphone alone
    std::thread::sleep(std::time::Duration::from_millis(300));
    info!("Exit: done");
    ::log::logger().flush();
    // Everything Jarvis keeps is written already (settings, chats, timers: synchronous writes). The usual exit()
    // then unloads every library (the speech model, its thread pools, WebView2) after their threads are gone,
    // which took about ten seconds; the process is ended at once instead, with the same exit code.
    #[cfg(target_os = "windows")]
    unsafe {
        winapi::um::processthreadsapi::TerminateProcess(winapi::um::processthreadsapi::GetCurrentProcess(), 0);
    }
    std::process::exit(0)
}

// a second start: the running Jarvis is asked (through its local page server) to show its window
fn ask_running_to_show() -> bool {
    // the running copy may still be starting (the speech model loads for many seconds on a slow PC, more right after
    // the Windows login): it is asked again for up to two minutes; its own window opens by itself when it is ready
    let until = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        if ask_running_once() {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

// the running Jarvis did not answer for two minutes (it hangs, or it is still loading on a very slow PC)
fn tell_still_starting() {
    warn!("Second start: the running Jarvis does not answer");
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() };
        let text = wide("Джарвис уже запущен, но пока не отвечает (возможно, ещё загружается). Подождите немного и откройте его значком в трее. Если он не отвечает долго — закройте jarvis-app в диспетчере задач и запустите снова.");
        let title = wide("Джарвис");
        unsafe {
            winapi::um::winuser::MessageBoxW(std::ptr::null_mut(), text.as_ptr(), title.as_ptr(), winapi::um::winuser::MB_OK | winapi::um::winuser::MB_ICONINFORMATION);
        }
    }
}

fn ask_running_once() -> bool {
    use std::io::{Read, Write};
    let body = r#"{"name":"show_window"}"#;
    let Ok(mut s) = std::net::TcpStream::connect_timeout(&"127.0.0.1:9714".parse().unwrap(), std::time::Duration::from_secs(2)) else { return false };
    let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(4)));
    let req = format!(
        "POST /action HTTP/1.1\r\nHost: 127.0.0.1:9714\r\nX-Jarvis: 1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    if s.write_all(req.as_bytes()).is_err() {
        return false;
    }
    let mut answer = String::new();
    let _ = s.read_to_string(&mut answer);
    answer.starts_with("HTTP/1.1 200") && answer.contains("\"ok\":true")
}

pub fn should_stop() -> bool {
    SHOULD_STOP.load(Ordering::SeqCst)
}
