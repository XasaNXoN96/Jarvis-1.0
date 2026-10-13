use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use jarvis_core::{audio_buffer::AudioRingBuffer, audio_processing, commands, config, listener, recorder, stt, COMMANDS_LIST, intent, voices, ipc::{self, IpcEvent}, i18n, slots};
use jarvis_core::origin::{Incoming, Source};
use rand::seq::SliceRandom;

use crate::should_stop;

// VAD state machine
#[derive(Debug, Clone, Copy, PartialEq)]
enum VadState {
    WaitingForVoice,
    VoiceActive,
}

// print a line for the user (console), ignoring errors when there is no console
fn say(msg: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{}", msg);
}

pub fn start(voice_mode: bool, text_cmd_rx: Receiver<Incoming>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    // from here on the queue of typed / remote / Telegram phrases is read
    jarvis_core::remote::mark_queue_ready();
    if voice_mode {
        match main_loop(&text_cmd_rx, rt) {
            // microphone failed to start: don't die, keep working in text mode
            Err(()) => {
                warn!("Voice mode failed, switching to TEXT mode.");
                let mut e = jarvis_core::VOICE_ERROR.lock();
                if e.is_empty() { *e = "микрофон не запустился".to_string(); }
            }
            Ok(()) => return Ok(()),
        }
    }
    text_loop(&text_cmd_rx, rt)
}

// read commands typed in the console and pass them to the main loop
pub fn console_input(tx: std::sync::mpsc::Sender<Incoming>) {
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        match stdin.read_line(&mut line) {
            Ok(0) | Err(_) => return, // no console / stdin closed
            Ok(_) => {
                let text = line.trim();
                if text.is_empty() {
                    continue;
                }
                if matches!(text, "exit" | "quit" | "выход") {
                    // the whole Jarvis ends (not only the voice loop with the window and the tray left behind)
                    crate::shutdown_and_exit();
                }
                if tx.send(Incoming::new(Source::Console, text)).is_err() {
                    return;
                }
            }
        }
    }
}

// text-only mode: no microphone / speech recognition, commands come from console or GUI
fn text_loop(text_cmd_rx: &Receiver<Incoming>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    info!("TEXT mode: type a command and press Enter ('exit' to quit).");
    say("Jarvis: text mode. Type a command and press Enter ('exit' to quit).");
    voices::play_greet();
    ipc::send(IpcEvent::Idle);

    loop {
        // run all queued commands first, stop only when the queue is empty
        match text_cmd_rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(incoming) => process_text_command(incoming, rt),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if should_stop() {
                    info!("Stop signal received, shutting down...");
                    voices::play_goodbye();
                    ipc::send(IpcEvent::Stopping);
                    break;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn main_loop(text_cmd_rx: &Receiver<Incoming>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    let frame_length: usize = 512;
    let sample_rate: usize = 16000;
    let mut frame_buffer: Vec<i16> = vec![0; frame_length];
    
    // ring buffer: keeps last 5 seconds of audio (pre-roll)
    let mut audio_buffer = AudioRingBuffer::new(5.0, frame_length, sample_rate);

    // VAD state
    let mut vad_state = VadState::WaitingForVoice;
    let mut silence_frames: u32 = 0;
    
    // how many frames of silence before we consider speech ended
    // 1.5 seconds = 1.5 * (16000 / 512) ≈ 47 frames
    let silence_threshold: u32 = ((1.5 * sample_rate as f32) / frame_length as f32) as u32;
    
    voices::play_greet();

    match recorder::start_recording() {
        Ok(_) => info!("Recording started. Microphone: {}", 
            recorder::get_audio_device_name(recorder::get_selected_microphone_index())),
        Err(_) => {
            error!("Cannot start recording.");
            return Err(());
        }
    }

    jarvis_core::VOICE_ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
    jarvis_core::VOICE_ERROR.lock().clear();
    ipc::send(IpcEvent::Idle);
    let mut mic_closed = false;

    // ### WAKE WORD DETECTION LOOP
    'wake_word: loop {
        if should_stop() {
            info!("Stop signal received, shutting down...");
            voices::play_goodbye();
            ipc::send(IpcEvent::Stopping);
            break;
        }

        if let Ok(incoming) = text_cmd_rx.try_recv() {
            process_text_command(incoming, &rt);
            continue 'wake_word;
        }

        // microphone switched off (tray / command): the device itself is closed (Windows no longer shows it in use),
        // text commands still work; switched on again: it is opened again
        if jarvis_core::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst) {
            if !mic_closed {
                recorder::release();
                mic_closed = true;
                info!("Microphone switched off: the device is closed");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
            continue 'wake_word;
        }
        if mic_closed {
            mic_closed = false;
            match recorder::start_recording() {
                Ok(_) => info!("Microphone switched on: the device is open again"),
                // (cannot be opened now: the background recovery keeps trying, the window shows why)
                Err(_) => recorder::recover_in_background(),
            }
            stt::reset_wake_recognizer();
            stt::reset_speech_recognizer();
            audio_processing::reset();
            audio_buffer.clear();
            vad_state = VadState::WaitingForVoice;
            silence_frames = 0;
        }

        // a session is open (dictation, a question about a file): keep listening without the wake word,
        // but never while Jarvis itself is speaking
        if jarvis_core::session::kind().is_some() && !jarvis_core::tts::is_speaking() {
            jarvis_core::LISTEN_NOW.store(true, std::sync::atomic::Ordering::SeqCst);
        }

        // click on the orb: listen for a command right away, no wake word needed
        if jarvis_core::LISTEN_NOW.swap(false, std::sync::atomic::Ordering::SeqCst) {
            info!("Listening on request (orb click)");
            ipc::send(IpcEvent::WakeWordDetected);
            stt::reset_wake_recognizer();
            stt::reset_speech_recognizer();
            audio_processing::reset();
            audio_buffer.clear();
            ipc::send(IpcEvent::Listening);
            recognize_command(&mut frame_buffer, &rt, frame_length, sample_rate, false);
            vad_state = VadState::WaitingForVoice;
            silence_frames = 0;
            audio_buffer.clear();
            stt::reset_wake_recognizer();
            stt::reset_speech_recognizer();
            audio_processing::reset();
            ipc::send(IpcEvent::Idle);
            continue 'wake_word;
        }

        recorder::read_microphone(&mut frame_buffer);
        let peak = frame_buffer.iter().map(|v| (*v as i32).unsigned_abs()).max().unwrap_or(0);
        jarvis_core::MIC_LEVEL.store(peak, std::sync::atomic::Ordering::Relaxed);
        let processed = audio_processing::process(&frame_buffer);
        
        match vad_state {
            VadState::WaitingForVoice => {
                // always buffer audio
                audio_buffer.push(&frame_buffer);
                
                if processed.is_voice {
                    // voice started! flush buffer to Vosk
                    info!("VAD: Voice started, flushing {} buffered frames", audio_buffer.len());
                    
                    for buffered_frame in audio_buffer.drain_all() {
                        listener::data_callback(&buffered_frame);
                    }
                    
                    vad_state = VadState::VoiceActive;
                    silence_frames = 0;
                }
            }
            
            VadState::VoiceActive => {
                // dual-feed: speech recognizer gets frames in parallel with wake word detector
                let _ = stt::recognize(&frame_buffer, false);

                // feed to wake word detector
                if let Some(_keyword_index) = listener::data_callback(&frame_buffer) {
                    // WAKE WORD DETECTED!
                    info!("Wake word activated!");
                    // the user calls Jarvis while he is speaking: he stops and listens ("Джарвис, стоп" works too)
                    if jarvis_core::tts::is_speaking() {
                        // (an AI answer still arriving stops too, or its next sentence would start the voice again)
                        if let Some(id) = jarvis_core::ai::store().active() {
                            jarvis_core::ai::stop(&id);
                        }
                        jarvis_core::tts::stop();
                    }
                    ipc::send(IpcEvent::WakeWordDetected);
                    
                    stt::reset_wake_recognizer();
                    audio_processing::reset();

                    // brief sniff to keep feeding STT while transitioning
                    let sniff_frames = ((0.3 * sample_rate as f32) / frame_length as f32) as u32;
                    for _ in 0..sniff_frames {
                        recorder::read_microphone(&mut frame_buffer);
                        audio_processing::process(&frame_buffer);
                        stt::recognize(&frame_buffer, false);
                    }

                    ipc::send(IpcEvent::Listening);
                    recognize_command(&mut frame_buffer, &rt, frame_length, sample_rate, true);

                    // reset state after command
                    vad_state = VadState::WaitingForVoice;
                    silence_frames = 0;
                    audio_buffer.clear();
                    stt::reset_wake_recognizer();
                    stt::reset_speech_recognizer(); // NOW reset, after command is done
                    audio_processing::reset();
                    ipc::send(IpcEvent::Idle);
                    
                    continue 'wake_word;
                }
                
                // track silence
                if processed.is_voice {
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                    
                    if silence_frames > silence_threshold {
                        debug!("VAD: Silence timeout, returning to wait state");
                        vad_state = VadState::WaitingForVoice;
                        silence_frames = 0;
                        stt::reset_wake_recognizer();
                        stt::reset_speech_recognizer(); // reset since we were dual-feeding
                    }
                }
            }
        }
    }

    recorder::stop_recording().ok();
    ipc::send(IpcEvent::Stopping);

    Ok(())
}


// Voice recognition for command after wake word
// "Jarvis is listening for a command" for the whole time of recognize_command
struct ListeningGuard;
impl ListeningGuard {
    fn new() -> Self {
        jarvis_core::VOICE_LISTENING.store(true, std::sync::atomic::Ordering::SeqCst);
        ListeningGuard
    }
}
impl Drop for ListeningGuard {
    fn drop(&mut self) {
        jarvis_core::VOICE_LISTENING.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

fn recognize_command(
    frame_buffer: &mut [i16],
    rt: &tokio::runtime::Runtime,
    frame_length: usize,
    sample_rate: usize,
    prefed_audio: bool
) {
    let _listening = ListeningGuard::new();
    let mut audio_buffer = AudioRingBuffer::new(2.0, frame_length, sample_rate);
    let mut vad_state = if prefed_audio {
        VadState::VoiceActive
    } else {
        VadState::WaitingForVoice
    };
    let mut silence_frames: u32 = 0;
    let mut start = SystemTime::now();
    let mut first_recognition = prefed_audio;
    let mut heard_own_voice = false;
    
    // longer silence threshold for commands (user might pause to think)
    // 5 seconds
    let silence_threshold: u32 = ((5.0 * sample_rate as f32) / frame_length as f32) as u32;
    
    loop {
        if crate::should_stop() {
            return;
        }
        // the microphone was switched off while Jarvis was listening: stop at once, nothing more is recognized
        if jarvis_core::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst) {
            info!("Microphone switched off: listening stopped");
            return;
        }
        
        recorder::read_microphone(frame_buffer);
        // Jarvis is speaking (an AI answer, a reply, a prompt of a session): the microphone hears that voice, so it
        // is not recognized as the user's next words; listening goes on with a fresh start when the voice is silent
        if jarvis_core::tts::is_speaking() {
            heard_own_voice = true;
            continue;
        }
        if heard_own_voice {
            heard_own_voice = false;
            stt::reset_speech_recognizer();
            audio_processing::reset();
            audio_buffer.clear();
            vad_state = VadState::WaitingForVoice;
            silence_frames = 0;
            start = SystemTime::now();
        }
        let processed = audio_processing::process(frame_buffer);
        
        match vad_state {
            VadState::WaitingForVoice => {
                audio_buffer.push(frame_buffer);
                
                if processed.is_voice {
                    // flush buffer to STT
                    for buffered_frame in audio_buffer.drain_all() {
                        stt::recognize(&buffered_frame, false);
                    }
                    vad_state = VadState::VoiceActive;
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                    
                    if silence_frames > silence_threshold {
                        info!("Long silence detected, returning to wake word mode.");
                        return;
                    }
                }
            }
            
            VadState::VoiceActive => {
                // feed to STT
                if let Some(mut recognized_voice) = stt::recognize(frame_buffer, false) {
                    // what was said can be private (a message, a note): only its size goes to the log, the window shows the text
                    info!("Recognized voice: {} chars", recognized_voice.chars().count());
                    
                    ipc::send(IpcEvent::SpeechRecognized {
                        text: recognized_voice.clone(),
                    });
                    
                    recognized_voice = recognized_voice.to_lowercase();
                    
                    // check if wake word repeated (reactivate)
                    let wake_phrases = config::get_wake_phrases(&i18n::get_language());
                    let contains_wake = wake_phrases.iter().any(|wp| recognized_voice.contains(wp));

                    if contains_wake {
                        // strip the wake word
                        let mut remaining = recognized_voice.clone();
                        for wp in wake_phrases {
                            remaining = remaining.replace(wp, "");
                        }
                        let remaining = remaining.trim();

                        if remaining.is_empty() {
                            if first_recognition {
                                // leftover wake word from dual-feed, just discard it
                                info!("Discarding initial wake word from prefed audio");
                                first_recognition = false;
                                stt::reset_speech_recognizer();
                                voices::play_reply();
                                vad_state = VadState::WaitingForVoice;
                                silence_frames = 0;
                                start = SystemTime::now();
                                audio_buffer.clear();
                                continue;
                            }

                            // just wake word, no command - reactivate
                            info!("Wake word repeated during chaining, reactivating...");
                            voices::play_reply();
                            stt::reset_speech_recognizer();
                            ipc::send(IpcEvent::Listening);
                            
                            vad_state = VadState::WaitingForVoice;
                            silence_frames = 0;
                            start = SystemTime::now();
                            audio_buffer.clear();
                            continue;
                        } else {
                            // wake word + command in one phrase - execute the command part
                            info!("Wake word + command during chaining: '{}'", remaining);
                            recognized_voice = remaining.to_string();
                            // fall through to command execution below
                        }
                    }

                    first_recognition = false;
                    
                    // filter activation phrases
                    // for tbr in config::ASSISTANT_PHRASES_TBR {
                    //     recognized_voice = recognized_voice.replace(tbr, "");
                    // }
                    let in_session = jarvis_core::session::kind().is_some();
                    if !in_session {
                        recognized_voice = remove_phrases(&recognized_voice, config::get_phrases_to_remove(&i18n::get_language()));
                    }

                    recognized_voice = recognized_voice.trim().to_string();
                    
                    if recognized_voice.len() < 5 && !in_session {
                        debug!("Ignoring too short recognition: '{}'", recognized_voice);
                        continue;
                    }

                    if recognized_voice.is_empty() {
                        continue;
                    }
                    
                    // execute command and check if we should chain
                    VOICE_INPUT.store(true, std::sync::atomic::Ordering::SeqCst);
                    *ORIGINAL.lock() = recognized_voice.clone();
                    // nothing listens while the command runs: the window does not say "слушаю"
                    jarvis_core::VOICE_LISTENING.store(false, std::sync::atomic::Ordering::SeqCst);
                    let should_chain = execute_command(&recognized_voice, Source::Voice, rt);
                    jarvis_core::VOICE_LISTENING.store(should_chain, std::sync::atomic::Ordering::SeqCst);
                    VOICE_INPUT.store(false, std::sync::atomic::Ordering::SeqCst);
                    
                    if should_chain {
                        // chain: reset and continue listening
                        info!("Chaining enabled, continuing to listen...");
                        stt::reset_speech_recognizer();
                        vad_state = VadState::WaitingForVoice;
                        silence_frames = 0;
                        start = SystemTime::now();
                        audio_buffer.clear();
                        ipc::send(IpcEvent::Listening);
                        continue;
                    } else {
                        // no chain: return to wake word
                        info!("No chain, returning to wake word mode.");
                        return;
                    }
                }
                
                // track silence
                if processed.is_voice {
                    silence_frames = 0;
                } else {
                    silence_frames += 1;
                    
                    if silence_frames > silence_threshold {
                        info!("Long silence detected, returning to wake word mode.");
                        return;
                    }
                }
            }
        }
        
        // timeout
        if let Ok(elapsed) = start.elapsed() {
            if elapsed > config::CMS_WAIT_DELAY {
                info!("Command timeout, returning to wake word mode.");
                return;
            }
        }
    }
}


// Removes the given words / phrases ("джарвис", "сэр", "скажи" ...) as WHOLE words only,
// so that "скажи" does not cut a piece out of "расскажи".
fn remove_phrases(text: &str, phrases: &[&str]) -> String {
    let mut padded = format!(" {} ", text);
    for phrase in phrases {
        let needle = format!(" {} ", phrase);
        // twice: removing one phrase can make the neighbour match ("а  а")
        for _ in 0..2 {
            padded = padded.replace(&needle, " ");
        }
        // phrase glued to punctuation ("джарвис," / "сэр.")
        for punct in [",", ".", "!", "?"] {
            padded = padded.replace(&format!(" {}{} ", phrase, punct), " ");
        }
    }
    padded.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn process_text_command(incoming: Incoming, rt: &tokio::runtime::Runtime) {
    // an internal follow-up of the previous request (the search after an AI failure): no new question is stored,
    // its replies join the request it belongs to. It is internal only when the program itself queued it
    // (the door is set by the entrance, never read from the text)
    let internal = incoming.source == Source::Internal;
    let source = incoming.source;
    let text = incoming.text.as_str();
    info!("Processing text command: {} chars", text.chars().count());
    
    if internal {
        jarvis_core::dialog::continue_last();
    } else {
        ipc::send(IpcEvent::SpeechRecognized { text: text.to_string() });
    }
    INTERNAL.store(internal, std::sync::atomic::Ordering::SeqCst);
    VOICE_INPUT.store(false, std::sync::atomic::Ordering::SeqCst);
    *ORIGINAL.lock() = text.trim().to_string();
    
    let mut filtered = text.to_lowercase();
    // for tbr in config::ASSISTANT_PHRASES_TBR {
    //     filtered = filtered.replace(tbr, "");
    // }
    if jarvis_core::session::kind().is_none() {
        filtered = remove_phrases(&filtered, config::get_phrases_to_remove(&i18n::get_language()));
    }

    let filtered = filtered.trim();
    
    if filtered.is_empty() {
        ipc::send(IpcEvent::Idle);
        return;
    }
    
    // text commands never chain
    execute_command(filtered, source, rt);
    INTERNAL.store(false, std::sync::atomic::Ordering::SeqCst);
}


// the phrase being handled is an internal follow-up (its question is already in the conversation)
static INTERNAL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

// the phrase came from the microphone (not from the keyboard)
static VOICE_INPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// what the user actually said or typed (capitals, punctuation); `text` of execute_command is lower-cased and without
// the wake word, which is right for matching and wrong for showing and for the AI
static ORIGINAL: once_cell::sync::Lazy<parking_lot::Mutex<String>> = once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(String::new()));

// the user's words for the conversation and for the AI: the original text without the wake word in front of it
fn shown_text(matched: &str) -> String {
    jarvis_core::route::original_without_wake(&ORIGINAL.lock(), matched)
}

// the question of the user goes into the one conversation (unless it is an internal follow-up)
fn open_request(text: &str, src: &str) {
    if INTERNAL.load(std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    jarvis_core::dialog::begin(&shown_text(text), src);
}

// Execute command, returns true if chaining should continue
fn execute_command(text: &str, source: Source, rt: &tokio::runtime::Runtime) -> bool {
    // every command, script and scenario that runs below knows its door (jarvis_core::origin::current())
    let _door = jarvis_core::origin::enter(source);
    info!("Command door: {}", source.as_str());

    // a question «Подтвердить? да / нет» is open: this phrase answers it (only from the door the request came through)
    match jarvis_core::confirm::reply(text, source) {
        jarvis_core::confirm::Reply::Rerun { text: asked, source: asked_source } => {
            info!("Confirmed from the {} door: the command is run", asked_source.as_str());
            return execute_command(&asked, asked_source, rt);
        }
        jarvis_core::confirm::Reply::Cancelled => {
            open_request(text, "command");
            let msg = "Отменено.";
            say(msg);
            ipc::send(IpcEvent::Output { text: msg.to_string() });
            jarvis_core::dialog::reply(msg, "local");
            jarvis_core::dialog::end(true);
            jarvis_core::uistate::log_action(text, "подтверждение: нет", true);
            ipc::send(IpcEvent::Idle);
            return false;
        }
        jarvis_core::confirm::Reply::OtherDoor => info!("A phrase from the {} door does not answer the open question", source.as_str()),
        jarvis_core::confirm::Reply::NotAnAnswer => {}
    }
    let commands_list = match COMMANDS_LIST.get() {
        Some(c) => c,
        None => {
            ipc::send(IpcEvent::Error { message: "Commands not loaded".to_string() });
            ipc::send(IpcEvent::Idle);
            return false;
        }
    };
    
    // an open session (dictation, "what to do with the file?") takes every phrase before the command search
    if let Some(kind) = jarvis_core::session::kind() {
        // a new file search while the question about the previous file is still open: leave the question
        if kind == "file" {
            if let Some((_, c)) = commands::fetch_by_triggers(text, commands_list) {
                if c.id == "assistant_file_search" {
                    jarvis_core::session::end();
                }
            }
        }
    }
    // the talk mode ("чат"): every phrase goes to the AI without the wake word, until "закончи чат"
    if jarvis_core::session::kind().as_deref() == Some("chat") {
        if jarvis_core::route::ends_chat(text) {
            jarvis_core::session::end();
            open_request(text, "command");
            jarvis_core::dialog::reply("Режим разговора выключен.", "local");
            jarvis_core::dialog::end(true);
            if jarvis_core::tts::is_enabled() {
                jarvis_core::tts::speak("Режим разговора выключен.");
            }
            ipc::send(IpcEvent::Idle);
            return false;
        }
        jarvis_core::session::touch();
        ask_ai(text);
        return jarvis_core::session::kind().is_some();
    }
    if let Some(kind) = jarvis_core::session::kind() {
        let session_cmd = commands_list.iter().flat_map(|l| l.commands.iter().map(move |c| (&l.path, c))).find(|(_, c)| c.id == "assistant_session");
        if let Some((path, cfg)) = session_cmd {
            info!("Session '{}' input: {}", kind, text);
            jarvis_core::session::touch();
            // the questions and answers of a dialog (what to do with the file, save / delete / continue) are part of the
            // conversation; the dictated text itself is not (it goes into the document)
            let record = kind == "file" || kind == "confirm" || kind == "app_choice" || (kind == "object" && (jarvis_core::session::data() != "typing" || text.contains("объект")));
            if record {
                open_request(text, "command");
            }
            let ok = match commands::execute_command(path, cfg, Some(text), None) {
                Ok(_) => true,
                Err(msg) => {
                    error!("Session script failed: {}", msg);
                    ipc::send(IpcEvent::Error { message: msg });
                    false
                }
            };
            jarvis_core::uistate::log_action(text, &format!("диалог: {}", kind), ok);
            if record {
                jarvis_core::dialog::end(ok);
            }
            ipc::send(IpcEvent::Idle);
            return jarvis_core::session::kind().is_some(); // keep listening while the session is open
        }
    }

    // a scenario of the "Конструктор": its activation phrase runs its chain of steps (before the usual commands:
    // the user chose this phrase on purpose)
    if let Some(sc) = jarvis_core::scenario::find(text) {
        if !jarvis_core::guard::first(&format!("scenario:{}", sc.id), jarvis_core::guard::ACTION_WINDOW) {
            ipc::send(IpcEvent::Idle);
            return false;
        }
        info!("Scenario: {}", sc.name);
        open_request(text, "command");
        let r = jarvis_core::scenario::run(&sc);
        if r.ok {
            jarvis_core::dialog::reply(&r.message, "command");
        } else {
            jarvis_core::dialog::fail(&r.message, "command");
        }
        jarvis_core::dialog::end(r.ok);
        jarvis_core::uistate::log_action(text, &format!("сценарий «{}»", sc.name), r.ok);
        if jarvis_core::tts::is_enabled() {
            // the number of the step that failed (also one found before the start), as the message says it
            let step: Option<String> = r.message.split("Шаг ").nth(1).map(|t| t.chars().take_while(|c| c.is_ascii_digit()).collect()).filter(|n: &String| !n.is_empty());
            let spoken = match (r.ok, step) {
                (true, _) => "Сценарий выполнен.".to_string(),
                (false, Some(n)) => format!("Сценарий остановлен на шаге {}. Подробности в окне.", n),
                (false, None) => "Сценарий не запущен, подробности в окне.".to_string(),
            };
            jarvis_core::tts::speak(&spoken);
        }
        ipc::send(IpcEvent::Idle);
        return false;
    }

    // a program by its spoken name ("открой ворт", "запусти тотал командер"): before the word triggers,
    // because the recognizer rarely writes the name the way the trigger list has it
    // then a website by name ("открой YouTube" is YouTube, never a search)
    let program_cmd = commands::fetch_program(text, commands_list)
        .or_else(|| commands::fetch_dictation(text, commands_list))
        .or_else(|| commands::fetch_site(text, commands_list))
        .or_else(|| commands::fetch_indexed_program(text, commands_list));

    // a question for the AI ("что такое программа", "объясни, как работает Excel"): the commands do not run it
    if program_cmd.is_none() && jarvis_core::route::decide(text, jarvis_core::ai::is_configured()) == jarvis_core::route::Route::Ai {
        info!("Knowledge question, sending to the AI ({} chars)", text.chars().count());
        ask_ai(text);
        return false;
    }

    let cmd_result = if let Some(found) = program_cmd {
        info!("Program command: {}", found.1.id);
        Some(found)
    } else if let Some(found) = commands::fetch_by_triggers(text, commands_list) {
        info!("Keyword trigger matched: {}", found.1.id);
        Some(found)
    } else if let Some((intent_id, confidence)) = 
        rt.block_on(intent::classify(text)) 
    {
        info!("Intent recognized: {} (confidence: {:.2})", intent_id, confidence);
        intent::get_command_by_intent(commands_list, &intent_id)
    } else {
        info!("Intent not recognized, trying levenshtein fallback...");
        let sentence = text.split_whitespace().count() >= 3;
        if sentence && jarvis_core::ai::is_configured() && !VOICE_INPUT.load(std::sync::atomic::Ordering::SeqCst) {
            // a typed sentence that matches no command by words: only a near-exact command phrase beats the AI,
            // a merely similar-looking command is not guessed
            commands::fetch_command_min(text, commands_list, config::CMD_RATIO_THRESHOLD.max(88.0))
        } else {
            commands::fetch_command(text, commands_list)
        }
    };
    
    if let Some((cmd_path, cmd_config)) = cmd_result {
        info!("Command found: {:?}", cmd_path);
        say(&format!("> {}", cmd_config.id));

        // a module switched off in the Jarvis window
        let pack = cmd_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let group = jarvis_core::uistate::group_key(&pack, &cmd_config.id);
        if jarvis_core::uistate::is_group_disabled(group) {
            let msg = format!("Раздел «{}» отключён (включить: окно Jarvis → Команды)", jarvis_core::uistate::group_title(group));
            say(&msg);
            ipc::send(IpcEvent::Output { text: msg.clone() });
            open_request(text, "command");
            jarvis_core::dialog::reply(&msg, "local");
            jarvis_core::dialog::end(false);
            jarvis_core::uistate::log_action(text, &cmd_config.id, false);
            ipc::send(IpcEvent::Idle);
            return false;
        }
        
        // a dangerous (high-risk) command is asked about first; only «да» from the same door runs this very command
        // (confirm.rs). What counts is the risk of a command whose files are the ones of this build.
        let risk = commands::effective_risk(&cmd_path, &cmd_config);
        let confirmed = match jarvis_core::confirm::gate(&cmd_config.id, risk, text, source) {
            jarvis_core::confirm::Gate::Run { confirmed } => confirmed,
            jarvis_core::confirm::Gate::Ask { question } => {
                info!("Command {} (high risk) waits for the confirmation from the {} door", cmd_config.id, source.as_str());
                say(&question);
                open_request(text, "command");
                ipc::send(IpcEvent::Output { text: question.clone() });
                jarvis_core::dialog::reply(&question, "local");
                jarvis_core::dialog::end(true);
                if jarvis_core::tts::can_speak().is_ok() {
                    jarvis_core::tts::speak("Это опасное действие. Подтвердить? Да или нет?");
                }
                jarvis_core::uistate::log_action(text, &format!("{}: ждёт подтверждения", cmd_config.id), true);
                ipc::send(IpcEvent::Idle);
                // by voice the answer is heard without the wake word
                return source == Source::Voice;
            }
        };
        let _confirmed_run = jarvis_core::confirm::confirmed_run(&cmd_config.id, confirmed);

        // extract slots if needed
        let extracted_slots = if !cmd_config.slots.is_empty() {
            let s = slots::extract(text, &cmd_config.slots);
            if !s.is_empty() {
                info!("Extracted slots: {:?}", s);
            }
            Some(s)
        } else {
            None
        };

        open_request(text, "command");
        jarvis_core::uistate::log_event("command", &format!("Команда «{}» ({}): выполняется", text, cmd_config.id));
        let outcome = commands::execute_command(&cmd_path, &cmd_config, Some(&text), extracted_slots.as_ref());
        match &outcome {
            Ok(_) => jarvis_core::uistate::log_event("command", &format!("Команда {}: выполнена", cmd_config.id)),
            Err(msg) => {
                jarvis_core::uistate::log_event("error", &format!("Команда {}: ошибка — {}", cmd_config.id, msg.chars().take(120).collect::<String>()));
                jarvis_core::dialog::fail(&format!("Не получилось выполнить команду: {}", msg.chars().take(200).collect::<String>()), "command");
            }
        }
        jarvis_core::dialog::end(outcome.is_ok());
        match outcome {
            Ok(chain) => {
                info!("Command executed successfully");
                jarvis_core::uistate::log_action(text, &cmd_config.id, true);
                // the short reaction sound only when the command did not answer aloud itself: never two voices at once
                // (the spoken answer "Сейчас 10 часов" is not covered by a recorded "Есть, сэр")
                if !jarvis_core::tts::is_speaking() {
                    voices::play_random_from(cmd_config.get_sounds(&i18n::get_language()).as_slice());
                }
                ipc::send(IpcEvent::CommandExecuted {
                    id: cmd_config.id.clone(),
                    success: true,
                });
                ipc::send(IpcEvent::Idle);
                return chain; // return chain status from command
            }
            Err(msg) => {
                error!("Error executing command: {}", msg);
                jarvis_core::uistate::log_action(text, &cmd_config.id, false);
                say(&format!("! error: {}", msg));
                // a failure is heard, not only shown (the voice packs have no error sound of their own);
                // a command that already said why stays the only voice
                if !jarvis_core::tts::is_speaking() {
                    // (a voice that cannot speak — no Piper in the Lite edition — gets the short sound instead)
                    if jarvis_core::tts::can_speak().is_ok() {
                        jarvis_core::tts::speak("Не получилось выполнить команду. Подробности в окне.");
                    } else {
                        voices::play_error();
                    }
                }
                ipc::send(IpcEvent::CommandExecuted {
                    id: cmd_config.id.clone(),
                    success: false,
                });
                ipc::send(IpcEvent::Error { message: msg.to_string() });
            }
        }
    } else {
        info!("No command found ({} chars)", text.chars().count());
        // not a command: with "answer phrases that are not commands" switched on, the AI answers (only then)
        let ai_ready = jarvis_core::ai::is_configured();
        let question = jarvis_core::route::is_open_question(text);
        // typed messages that are not commands are a talk with the AI; spoken ones only when the switch is on
        // (a misheard word in the room should not travel to the Internet)
        let typed = !VOICE_INPUT.load(std::sync::atomic::Ordering::SeqCst);
        // AI switched off in the privacy section: the phrase needs the AI, so the user is told and nothing is sent anywhere
        // (not to the provider and not to a web search either)
        if !jarvis_core::uistate::settings().ai_enabled && (typed || question) {
            let msg = jarvis_core::ai::AiError::Disabled.user_text();
            say(&msg);
            open_request(text, "local");
            jarvis_core::dialog::reply(&msg, "local");
            jarvis_core::dialog::end(true);
            if jarvis_core::tts::is_enabled() {
                jarvis_core::tts::speak("Запросы к ИИ выключены в настройках приватности.");
            }
            jarvis_core::uistate::log_action(text, "ИИ выключен", false);
            ipc::send(IpcEvent::Idle);
            return false;
        }
        if ai_ready && (typed || jarvis_core::uistate::settings().ai_fallback || question) {
            ask_ai(text);
            return false;
        }
        if question && !typed {
            // a spoken question and no AI: a misheard phrase in the room is not sent to a search engine by itself;
            // the user is told how to search on purpose
            let msg = "ИИ не подключён (Настройки → ИИ-провайдер). Чтобы искать в интернете, скажите «найди в интернете …».".to_string();
            say(&msg);
            open_request(text, "local");
            jarvis_core::dialog::reply(&msg, "local");
            jarvis_core::dialog::end(true);
            if jarvis_core::tts::is_enabled() {
                jarvis_core::tts::speak("ИИ не подключён. Чтобы искать в интернете, скажите: найди в интернете.");
            }
            jarvis_core::uistate::log_action(text, "ИИ не подключён", false);
            ipc::send(IpcEvent::Idle);
            return false;
        }
        if question {
            // a typed question and no AI is connected: say so and look it up on the Internet instead of staying silent.
            // The note says plainly that this is a search, not an answer of the AI.
            let msg = "ИИ не подключён (Настройки → ИИ-провайдер), поэтому ищу ответ в интернете. Это результат поиска, а не ответ нейросети.".to_string();
            say(&msg);
            ipc::send(IpcEvent::Output { text: msg.clone() });
            open_request(text, "local");
            jarvis_core::dialog::reply(&msg, "search");
            jarvis_core::dialog::end(true);
            if jarvis_core::tts::is_enabled() {
                jarvis_core::tts::speak("ИИ не подключён. Ищу ответ в интернете.");
            }
            jarvis_core::uistate::log_event("info", "ИИ не настроен: поиск в интернете");
            jarvis_core::uistate::log_action(text, "поиск в интернете", true);
            jarvis_core::remote::push_internal(&format!("найди в интернете {}", jarvis_core::route::search_query(text)));
            ipc::send(IpcEvent::Idle);
            return false;
        }
        say(&format!("? command not found: {}", text));
        let unknown = format!("Не знаю такой команды: «{}». Скажите «что ты умеешь» или задайте вопрос.", text);
        open_request(text, "local");
        jarvis_core::dialog::reply(&unknown, "local");
        jarvis_core::dialog::end(false);
        if jarvis_core::tts::can_speak().is_ok() {
            jarvis_core::tts::speak("Не знаю такой команды.");
        } else {
            voices::play_not_found();
        }
        jarvis_core::uistate::log_action(text, "—", false);
        ipc::send(IpcEvent::Error { 
            message: format!("Command not found: {}", text) 
        });
    }
    
    ipc::send(IpcEvent::Idle);
    false // no chain on error or not found
}


// sends a phrase to the connected AI; the answer appears on the "Диалог" page and is read aloud. When the AI
// cannot answer (no internet, limit, server error) the reason is said and, where it makes sense, the Internet is searched.
fn ask_ai(text: &str) {
    let query = text.to_string();
    let shown = shown_text(text);
    let text = shown.as_str();
    let hook: jarvis_core::ai::ErrorHook = Box::new(move |e| {
        // the error itself is already stored in the conversation by the AI module (never hidden)
        let msg = e.user_text();
        ipc::send(IpcEvent::Output { text: msg.clone() });
        jarvis_core::uistate::log_event("error", &format!("ИИ: {}", msg.split(". ").next().unwrap_or(&msg)));
        if jarvis_core::tts::is_enabled() {
            jarvis_core::tts::speak(msg.split(". ").next().unwrap_or(&msg));
        }
        if e.allows_web_search() {
            // plan B, clearly named as a search; the next question goes to the provider again
            jarvis_core::dialog::adopt_ai_request();
            jarvis_core::dialog::reply("ИИ сейчас недоступен, поэтому ищу в интернете. Это результат поиска, а не ответ нейросети.", "search");
            jarvis_core::uistate::log_event("info", "ИИ недоступен: поиск в интернете");
            jarvis_core::remote::push_internal(&format!("найди в интернете {}", jarvis_core::route::search_query(&query)));
        }
    });
    if INTERNAL.load(std::sync::atomic::Ordering::SeqCst) {
        // the follow-up of a request is never asked again
        ipc::send(IpcEvent::Idle);
        return;
    }
    match jarvis_core::ai::send_question(text, hook) {
        Ok(_) => {
            jarvis_core::uistate::log_action(text, "ИИ-чат", true);
            jarvis_core::uistate::log_event("ai", &format!("Вопрос отправлен: {}", jarvis_core::ai::status_json()["provider"].as_str().unwrap_or("ИИ")));
        }
        Err(jarvis_core::ai::AiError::Duplicate) => {
            // the same question a moment ago: it is already in the conversation and being answered
            jarvis_core::uistate::log_event("info", &format!("Повтор вопроса пропущен: {}", text.chars().take(80).collect::<String>()));
        }
        Err(e) => {
            // refused before anything was sent (busy, empty, not configured): the question and the reason stay together
            jarvis_core::uistate::log_action(text, "ИИ-чат", false);
            jarvis_core::dialog::begin(text, "ai");
            jarvis_core::dialog::fail(&e.user_text(), "ai");
            jarvis_core::dialog::end(false);
            ipc::send(IpcEvent::Output { text: e.user_text() });
            if jarvis_core::tts::is_enabled() {
                let m = e.user_text();
                jarvis_core::tts::speak(m.split(". ").next().unwrap_or(&m));
            }
        }
    }
    ipc::send(IpcEvent::Idle);
}

pub fn close(code: i32) {
    info!("Closing application.");
    voices::play_goodbye();
    ipc::send(IpcEvent::Stopping);
    std::process::exit(code);
}