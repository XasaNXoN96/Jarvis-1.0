use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use jarvis_core::{audio_buffer::AudioRingBuffer, audio_processing, commands, config, listener, recorder, stt, COMMANDS_LIST, intent, voices, ipc::{self, IpcEvent}, i18n, slots};
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

pub fn start(voice_mode: bool, text_cmd_rx: Receiver<String>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
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
pub fn console_input(tx: std::sync::mpsc::Sender<String>) {
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
                    crate::request_stop();
                    return;
                }
                if tx.send(text.to_string()).is_err() {
                    return;
                }
            }
        }
    }
}

// text-only mode: no microphone / speech recognition, commands come from console or GUI
fn text_loop(text_cmd_rx: &Receiver<String>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
    info!("TEXT mode: type a command and press Enter ('exit' to quit).");
    say("Jarvis: text mode. Type a command and press Enter ('exit' to quit).");
    voices::play_greet();
    ipc::send(IpcEvent::Idle);

    loop {
        // run all queued commands first, stop only when the queue is empty
        match text_cmd_rx.recv_timeout(std::time::Duration::from_millis(200)) {
            Ok(text) => process_text_command(&text, rt),
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

fn main_loop(text_cmd_rx: &Receiver<String>, rt: &tokio::runtime::Runtime) -> Result<(), ()> {
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

    // ### WAKE WORD DETECTION LOOP
    'wake_word: loop {
        if should_stop() {
            info!("Stop signal received, shutting down...");
            voices::play_goodbye();
            ipc::send(IpcEvent::Stopping);
            break;
        }

        if let Ok(text) = text_cmd_rx.try_recv() {
            process_text_command(&text, &rt);
            continue 'wake_word;
        }

        // microphone switched off (tray / command): wait for text commands only
        if jarvis_core::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(100));
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
fn recognize_command(
    frame_buffer: &mut [i16],
    rt: &tokio::runtime::Runtime,
    frame_length: usize,
    sample_rate: usize,
    prefed_audio: bool
) {
    let mut audio_buffer = AudioRingBuffer::new(2.0, frame_length, sample_rate);
    let mut vad_state = if prefed_audio {
        VadState::VoiceActive
    } else {
        VadState::WaitingForVoice
    };
    let mut silence_frames: u32 = 0;
    let mut start = SystemTime::now();
    let mut first_recognition = prefed_audio;
    
    // longer silence threshold for commands (user might pause to think)
    // 5 seconds
    let silence_threshold: u32 = ((5.0 * sample_rate as f32) / frame_length as f32) as u32;
    
    loop {
        if crate::should_stop() {
            return;
        }
        
        recorder::read_microphone(frame_buffer);
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
                    info!("Recognized voice: {}", recognized_voice);
                    
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
                    recognized_voice = remove_phrases(&recognized_voice, config::get_phrases_to_remove(&i18n::get_language()));

                    recognized_voice = recognized_voice.trim().to_string();
                    
                    if recognized_voice.len() < 5 {
                        debug!("Ignoring too short recognition: '{}'", recognized_voice);
                        continue;
                    }

                    if recognized_voice.is_empty() {
                        continue;
                    }
                    
                    // execute command and check if we should chain
                    let should_chain = execute_command(&recognized_voice, rt);
                    
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

fn process_text_command(text: &str, rt: &tokio::runtime::Runtime) {
    info!("Processing text command: {}", text);
    
    ipc::send(IpcEvent::SpeechRecognized { text: text.to_string() });
    
    let mut filtered = text.to_lowercase();
    // for tbr in config::ASSISTANT_PHRASES_TBR {
    //     filtered = filtered.replace(tbr, "");
    // }
    filtered = remove_phrases(&filtered, config::get_phrases_to_remove(&i18n::get_language()));

    let filtered = filtered.trim();
    
    if filtered.is_empty() {
        ipc::send(IpcEvent::Idle);
        return;
    }
    
    // text commands never chain
    execute_command(filtered, rt);
}


// Execute command, returns true if chaining should continue
fn execute_command(text: &str, rt: &tokio::runtime::Runtime) -> bool {
    let commands_list = match COMMANDS_LIST.get() {
        Some(c) => c,
        None => {
            ipc::send(IpcEvent::Error { message: "Commands not loaded".to_string() });
            ipc::send(IpcEvent::Idle);
            return false;
        }
    };
    
    let cmd_result = if let Some(found) = commands::fetch_by_triggers(text, commands_list) {
        info!("Keyword trigger matched: {}", found.1.id);
        Some(found)
    } else if let Some((intent_id, confidence)) = 
        rt.block_on(intent::classify(text)) 
    {
        info!("Intent recognized: {} (confidence: {:.2})", intent_id, confidence);
        intent::get_command_by_intent(commands_list, &intent_id)
    } else {
        info!("Intent not recognized, trying levenshtein fallback...");
        commands::fetch_command(text, commands_list)
    };
    
    if let Some((cmd_path, cmd_config)) = cmd_result {
        info!("Command found: {:?}", cmd_path);
        say(&format!("> {}", cmd_config.id));

        // a module switched off in the Jarvis window
        let pack = cmd_path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if jarvis_core::uistate::is_pack_disabled(&pack) {
            let msg = format!("Модуль «{}» отключён (включить: окно Jarvis → Панель → Модули)", pack);
            say(&msg);
            ipc::send(IpcEvent::Output { text: msg });
            jarvis_core::uistate::log_action(text, &cmd_config.id, false);
            ipc::send(IpcEvent::Idle);
            return false;
        }
        
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

        match commands::execute_command(&cmd_path, &cmd_config, Some(&text), extracted_slots.as_ref()) {
            Ok(chain) => {
                info!("Command executed successfully");
                jarvis_core::uistate::log_action(text, &cmd_config.id, true);
                // voices::play_ok();
                voices::play_random_from(cmd_config.get_sounds(&i18n::get_language()).as_slice());
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
                voices::play_error();
                ipc::send(IpcEvent::CommandExecuted {
                    id: cmd_config.id.clone(),
                    success: false,
                });
                ipc::send(IpcEvent::Error { message: msg.to_string() });
            }
        }
    } else {
        info!("No command found for: {}", text);
        say(&format!("? command not found: {}", text));
        jarvis_core::uistate::log_action(text, "—", false);
        voices::play_not_found();
        ipc::send(IpcEvent::Error { 
            message: format!("Command not found: {}", text) 
        });
    }
    
    ipc::send(IpcEvent::Idle);
    false // no chain on error or not found
}


pub fn close(code: i32) {
    info!("Closing application.");
    voices::play_goodbye();
    ipc::send(IpcEvent::Stopping);
    std::process::exit(code);
}