use std::sync::mpsc::Receiver;
use std::time::SystemTime;

use jarvis_core::{audio_buffer::AudioRingBuffer, audio, audio_processing, commands, config,  listener, recorder, stt, COMMANDS_LIST, intent, voices, ipc::{self, IpcEvent}};
use rand::prelude::*;

use crate::should_stop;

// VAD state machine
#[derive(Debug, Clone, Copy, PartialEq)]
enum VadState {
    WaitingForVoice,
    VoiceActive,
}

pub fn start(text_cmd_rx: Receiver<String>) -> Result<(), ()> {
    // start the loop
    main_loop(text_cmd_rx)
}

fn main_loop(text_cmd_rx: Receiver<String>) -> Result<(), ()> {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    let mut start: SystemTime;
    // let sounds_directory = audio::get_sound_directory().unwrap();
    let frame_length: usize = 512; // default for every wake-word engine
    let sample_rate: usize = 16000;
    let mut frame_buffer: Vec<i16> = vec![0; frame_length];

    // ring buffer: keep last 2 seconds of audio
    let mut audio_buffer = AudioRingBuffer::new(2.0, frame_length, sample_rate);

    // VAD state
    let mut vad_state = VadState::WaitingForVoice;
    let mut silence_frames: u32 = 0;

    // how many frames of silence before we consider speech ended
    // 1.5 seconds = 1.5 * (16000 / 512) ≈ 47 frames
    // @TODO: Put this to config
    let silence_threshold: u32 = ((1.5 * sample_rate as f32) / frame_length as f32) as u32;

    // play some startup phrase
    // audio::play_sound(&sounds_directory.join("run.wav"));
    voices::play_greet();

    // start recording
    match recorder::start_recording() {
        Ok(_) => info!("Recording started."),
        Err(_) => {
            error!("Cannot start recording.");
            return Err(()); // quit
        }
    }

    // notify GUI we're ready
    ipc::send(IpcEvent::Idle);

    // DEBUG counter
    let mut frame_count: u32 = 0;

    // the loop
    'wake_word: loop {
        // check for stop signal
        if should_stop() {
            info!("Stop signal received, shutting down...");
            voices::play_goodbye();
            ipc::send(IpcEvent::Stopping);
            break;
        }

        // check for text commands
        if let Ok(text) = text_cmd_rx.try_recv() {
            process_text_command(&text, &rt);
            continue 'wake_word;
        }

        // read from microphone
        recorder::read_microphone(&mut frame_buffer);

        // DEBUG: check raw audio
        frame_count += 1;
        let raw_rms = calculate_rms(&frame_buffer);

        if frame_count % 100 == 0 {
            info!("DEBUG [{}]: raw_rms={:.0}", frame_count, raw_rms);
        }

        // check if we're getting any audio at all
        if frame_count == 100 && raw_rms < 10.0 {
            warn!("WARNING: Microphone appears to be silent! RMS={:.0}", raw_rms);
        }

        // process audio (gain -> noise suppression -> VAD)
        let processed = audio_processing::process(&frame_buffer);

        if frame_count % 100 == 0 {
            info!("DEBUG [{}]: is_voice={}, vad_conf={:.2}, processed_rms={:.0}", 
                frame_count,
                processed.is_voice, 
                processed.vad_confidence,
                calculate_rms(&processed.samples)
            );
        }

        // skip if no voice detected (vad)
        if !processed.is_voice {
            continue 'wake_word;
        }

        // DEBUG: we passed VAD
        if frame_count % 50 == 0 {
            info!("DEBUG: Voice detected, checking wake word...");
        }

        // recognize wake-word
        match listener::data_callback(&frame_buffer) {
            Some(_keyword_index) => {
                // notify GUI
                ipc::send(IpcEvent::WakeWordDetected);

                // reset some things
                stt::reset_wake_recognizer();
                stt::reset_speech_recognizer();
                audio_processing::reset();

                // wake-word activated, process further commands
                // capture current time
                start = SystemTime::now();
                silence_frames = 0;

                // play some reply phrase
                // @TODO. Make it via commands or upcoming events system.
                voices::play_reply();


                // notify GUI we're listening
                ipc::send(IpcEvent::Listening);

                // wait for voice commands
                'voice_recognition: loop {
                    // check for stop
                    if should_stop() {
                        break 'wake_word;
                    }

                    // read from microphone
                    recorder::read_microphone(&mut frame_buffer);

                    // process first
                    let processed = audio_processing::process(&frame_buffer);

                    // detect silence, return to wake-word if silence
                    if processed.is_voice {
                        silence_frames = 0;
                    } else {
                        silence_frames += 1;
                        if silence_frames > config::VAD_SILENCE_FRAMES * 2 {
                            info!("Long silence detected, returning to wake word mode.");
                            break 'voice_recognition;
                        }
                    }

                    // stt part (without partials)
                    if let Some(mut recognized_voice) = stt::recognize(&frame_buffer, false) {
                        // something was recognized
                        info!("Recognized voice: {}", recognized_voice);

                        // notify GUI
                        ipc::send(IpcEvent::SpeechRecognized {
                            text: recognized_voice.clone(),
                        });

                        // filter recognized voice
                        // @TODO. Better recognized voice filtration.
                        recognized_voice = recognized_voice.to_lowercase();

                        // answer again if it's activation phrase repeated
                        if recognized_voice.contains(config::VOSK_FETCH_PHRASE) {
                            info!("Wake word detected during chaining, reactivating...");
                            
                            // play greet sound
                            // audio::play_sound(&sounds_directory.join(format!(
                            //     "{}.wav",
                            //     config::ASSISTANT_GREET_PHRASES
                            //         .choose(&mut rand::thread_rng())
                            //         .unwrap()
                            // )));
                            voices::play_reply();
                            
                            // reset timer and continue listening
                            start = SystemTime::now();
                            silence_frames = 0;
                            stt::reset_speech_recognizer();

                            ipc::send(IpcEvent::Listening);
                            continue 'voice_recognition;
                        }

                        // filter out activation phrase from command
                        for tbr in config::ASSISTANT_PHRASES_TBR {
                            recognized_voice = recognized_voice.replace(tbr, "");
                        }
                        recognized_voice = recognized_voice.trim().into();

                        // skip if nothing left after filtering (*evil laugh*)
                        if recognized_voice.is_empty() {
                            continue 'voice_recognition;
                        }

                        // execute command (shared executor)
                        execute_command(&recognized_voice, &rt);

                        // return to wake-word listening after command execution (no matter successful or not)
                        break 'voice_recognition;
                    }

                    // only recognize voice for a certain period of time
                    match start.elapsed() {
                        Ok(elapsed) if elapsed > config::CMS_WAIT_DELAY => {
                            // return to wake-word listening after N seconds
                            break 'voice_recognition;
                        }
                        _ => (),
                    }

                    // reset things
                    stt::reset_wake_recognizer();
                    audio_processing::reset();
                    ipc::send(IpcEvent::Idle);
                }
            }
            None => (),
        }
    }

    // cleanup
    recorder::stop_recording().ok();
    ipc::send(IpcEvent::Stopping);

    Ok(())
}


// process text command from GUI
fn process_text_command(text: &str, rt: &tokio::runtime::Runtime) {
    info!("Processing text command: {}", text);
    
    ipc::send(IpcEvent::SpeechRecognized { text: text.to_string() });
    
    // filter text same as voice
    let mut filtered = text.to_lowercase();
    for tbr in config::ASSISTANT_PHRASES_TBR {
        filtered = filtered.replace(tbr, "");
    }
    let filtered = filtered.trim();
    
    if filtered.is_empty() {
        ipc::send(IpcEvent::Idle);
        return;
    }
    
    execute_command(filtered, rt);
}

// shared command execution logic (manual & voice)
fn execute_command(text: &str, rt: &tokio::runtime::Runtime) {
    let commands_list = match COMMANDS_LIST.get() {
        Some(c) => c,
        None => {
            ipc::send(IpcEvent::Error { message: "Commands not loaded".to_string() });
            ipc::send(IpcEvent::Idle);
            return;
        }
    };
    
    // let sounds_directory = audio::get_sound_directory().unwrap();
    
    // try intent recognition first, fallback to levenshtein
    let cmd_result = if let Some((intent_id, confidence)) = 
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
        
        match commands::execute_command(&cmd_path, &cmd_config) {
            Ok(_) => {
                info!("Command executed successfully");
                voices::play_ok(); // command executed sound
                ipc::send(IpcEvent::CommandExecuted {
                    id: cmd_config.id.clone(),
                    success: true,
                });
            }
            Err(msg) => {
                error!("Error executing command: {}", msg);
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
        // play "not understood" sound
        // audio::play_sound(&sounds_directory.join("not_understand.wav"));
        voices::play_not_found();
        ipc::send(IpcEvent::Error { 
            message: format!("Command not found: {}", text) 
        });
    }
    
    ipc::send(IpcEvent::Idle);
}


fn keyword_callback(keyword_index: i32) {}

pub fn close(code: i32) {
    info!("Closing application.");
    voices::play_goodbye();
    ipc::send(IpcEvent::Stopping);
    std::process::exit(code);
}

fn calculate_rms(samples: &[i16]) -> f32 {
    if samples.is_empty() { return 0.0; }
    let sum: f64 = samples.iter().map(|&s| (s as f64).powi(2)).sum();
    (sum / samples.len() as f64).sqrt() as f32
}
