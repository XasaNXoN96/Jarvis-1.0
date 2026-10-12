mod pvrecorder;
pub mod diag;

// mod cpal;
// mod portaudio;

use once_cell::sync::OnceCell;

use crate::{config, config::structs::RecorderType, DB};

static RECORDER_TYPE: OnceCell<RecorderType> = OnceCell::new();
static FRAME_LENGTH: OnceCell<u32> = OnceCell::new();

pub fn init() -> Result<(), ()> {
    // set default recorder type
    // @TODO. Make it configurable?
    let _ = RECORDER_TYPE.set(config::DEFAULT_RECORDER_TYPE);

    // some info
    info!("Loading recorder ...");
    info!("Available audio_devices are:\n{:?}", get_audio_devices());

    // load given recorder
    match RECORDER_TYPE.get().unwrap() {
        RecorderType::PvRecorder => {
            // Init Pv Recorder
            info!("Initializing PvRecorder recording backend.");
            let _ = FRAME_LENGTH.set(512u32); // pvrecorder requires frame buffer of 512
            let selected_microphone = get_selected_microphone_index();
            match pvrecorder::init_microphone(
                selected_microphone,
                FRAME_LENGTH.get().unwrap().to_owned(),
            ) {
                false => {
                    error!("Recorder initialization failed.");

                    return Err(());
                }
                _ => {
                    info!(
                        "Recorder initialization success. Listening to microphone ({}): {}",
                        pvrecorder::active_index(),
                        get_audio_device_name(pvrecorder::active_index())
                    );
                }
            }
        }
        RecorderType::PortAudio => {
            // Init PortAudio
            info!("Initializing PortAudio recording backend");
            todo!();
            // match portaudio::init_microphone(get_selected_microphone_index(), FRAME_LENGTH.load(Ordering::SeqCst)) {
            //     false => {
            //         // Switch to PortAudio recorder
            //         error!("PortAudio audio backend failed.");
            //     },
            //     _ => ()
            // }
        }
        RecorderType::Cpal => {
            // Init CPAL
            info!("Initializing CPAL recording backend");
            todo!();
            // match cpal::init_microphone(get_selected_microphone_index(), FRAME_LENGTH.load(Ordering::SeqCst)) {
            //     false => {
            //         // Switch to CPAL recorder
            //         error!("CPAL audio backend failed.");
            //     },
            //     _ => ()
            // }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------------------------
// the microphone came back: after the sleep of Windows or a reconnected device the stream is dead, it is opened
// again every few seconds until it works. Never when the user switched the microphone off (privacy) or Jarvis exits.
// ---------------------------------------------------------------------------------------------
static RECOVERING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static SHUT_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const RECOVER_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

// what to do on one attempt (kept apart for the tests): stop trying, wait, or open the microphone again
#[derive(Debug, PartialEq)]
pub enum Recover {
    // Jarvis exits: the attempts end
    Stop,
    // the user switched the microphone off: nothing is opened, the attempts wait (after "включи микрофон" the voice
    // loop finds the dead stream again otherwise)
    Wait,
    Try,
}

pub fn recover_step(shut_down: bool, muted: bool) -> Recover {
    if shut_down {
        Recover::Stop
    } else if muted {
        Recover::Wait
    } else {
        Recover::Try
    }
}

// 5 s, 10 s, 20 s, 40 s, then once a minute: a microphone that stays unplugged does not flood the log
pub fn recover_delay(attempt: u32) -> std::time::Duration {
    RECOVER_EVERY.saturating_mul(1 << attempt.min(4)).min(std::time::Duration::from_secs(60))
}

// A microphone that drops out again and again (a loose cable, a sleeping USB hub) is told to the user once in ten
// minutes; every loss still goes to the log and the event list.
pub const LOST_NOTICE_EVERY: std::time::Duration = std::time::Duration::from_secs(600);
static LOST_NOTICE_AT: parking_lot::Mutex<Option<std::time::Instant>> = parking_lot::Mutex::new(None);

pub fn lost_notice_due() -> bool {
    notice_due(&LOST_NOTICE_AT, std::time::Instant::now())
}

fn notice_due(last: &parking_lot::Mutex<Option<std::time::Instant>>, now: std::time::Instant) -> bool {
    let mut last = last.lock();
    if last.map(|t| now.duration_since(t) < LOST_NOTICE_EVERY).unwrap_or(false) {
        return false;
    }
    *last = Some(now);
    true
}

pub fn recover_in_background() {
    use std::sync::atomic::Ordering;
    if RECOVERING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut attempt = 0u32;
        loop {
            std::thread::sleep(recover_delay(attempt));
            // (the text mode has no microphone at all: nothing to bring back)
            let ends = SHUT_DOWN.load(Ordering::SeqCst) || crate::VOICE_TEXT_ONLY.load(Ordering::SeqCst);
            match recover_step(ends, crate::MIC_MUTED.load(Ordering::SeqCst)) {
                Recover::Stop => break,
                Recover::Wait => continue,
                Recover::Try => {}
            }
            // the microphone works again by another way (the user switched it off and on, which reopened it): a working
            // stream is never closed by the recovery
            if pvrecorder::delivering() {
                info!("Microphone recovery: the microphone already works, nothing to do");
                let mut err = crate::VOICE_ERROR.lock();
                if err.starts_with("микрофон перестал") {
                    err.clear();
                }
                break;
            }
            attempt += 1;
            release();
            if start_recording().is_ok() {
                // switched off or exiting while it was opening: it is closed again at once
                if crate::MIC_MUTED.load(Ordering::SeqCst) || SHUT_DOWN.load(Ordering::SeqCst) {
                    release();
                    continue;
                }
                info!("Microphone is back: recording again");
                let mut err = crate::VOICE_ERROR.lock();
                if err.starts_with("микрофон перестал") {
                    err.clear();
                }
                drop(err);
                crate::uistate::log_event("voice", "Микрофон снова работает");
                break;
            }
        }
        RECOVERING.store(false, Ordering::SeqCst);
    });
}

// "Полностью закрыть JARVIS": the microphone is released and never opened again by this process
pub fn shutdown() {
    SHUT_DOWN.store(true, std::sync::atomic::Ordering::SeqCst);
    release();
}

pub fn read_microphone(frame_buffer: &mut [i16]) {
    match RECORDER_TYPE.get().unwrap() {
        RecorderType::PvRecorder => {
            pvrecorder::read_microphone(frame_buffer);
        }
        RecorderType::PortAudio => {
            todo!();
            // portaudio::read_microphone(frame_buffer);
        }
        RecorderType::Cpal => {
            // cpal::read_microphone(frame_buffer);
            panic!("Cpal should be used via callback assignment");
        }
    }
}

pub fn start_recording() -> Result<(), ()> {
    match RECORDER_TYPE.get().unwrap() {
        RecorderType::PvRecorder => {
            return pvrecorder::start_recording(
                get_selected_microphone_index(),
                FRAME_LENGTH.get().unwrap().to_owned(),
            );
        }
        RecorderType::PortAudio => {
            todo!();
            // portaudio::start_recording(get_selected_microphone_index(), FRAME_LENGTH.load(Ordering::SeqCst));
        }
        RecorderType::Cpal => {
            todo!();
            // cpal::start_recording(get_selected_microphone_index(), FRAME_LENGTH.load(Ordering::SeqCst));
        }
    }
}

pub fn stop_recording() -> Result<(), ()> {
    match RECORDER_TYPE.get().unwrap() {
        RecorderType::PvRecorder => pvrecorder::stop_recording(),
        RecorderType::PortAudio => {
            todo!();
            // portaudio::stop_recording();
        }
        RecorderType::Cpal => {
            todo!();
            // cpal::stop_recording();
        }
    }
}

pub fn get_selected_microphone_index() -> i32 {
    let idx = DB.get().unwrap().read().microphone;
    let validated = diag::validate_index(idx, get_audio_devices().len());
    if validated != idx {
        warn!("Microphone index {} is not available, using the system default", idx);
    }
    validated
}

// stops and frees the audio stream (the device becomes available to other programs)
pub fn release() {
    pvrecorder::release();
}

// index of the device that is actually open (-1 = system default)
pub fn active_index() -> i32 {
    pvrecorder::active_index()
}

pub fn is_open() -> bool {
    pvrecorder::is_open()
}

// milliseconds since the last audio frame (None: no audio yet)
pub fn last_frame_age_ms() -> Option<u64> {
    pvrecorder::last_frame_age_ms()
}

// number of audio frames delivered so far
pub fn frames_received() -> u64 {
    pvrecorder::FRAMES.load(std::sync::atomic::Ordering::Relaxed)
}

// checks every device while the recorder is not running
pub fn probe_all() -> diag::Report {
    pvrecorder::probe_all()
}

pub fn get_audio_devices() -> Vec<String> {
    match RECORDER_TYPE.get() {
        Some(RecorderType::PvRecorder) => pvrecorder::list_audio_devices(),
        Some(RecorderType::PortAudio) => {
            todo!();
        }
        Some(RecorderType::Cpal) => {
            todo!();
        }
        None => {
            // not initialized yet, default to pvrecorder
            pvrecorder::list_audio_devices()
        }
    }
}

pub fn get_audio_device_name(idx: i32) -> String {
    match RECORDER_TYPE.get() {
        Some(RecorderType::PvRecorder) => pvrecorder::get_audio_device_name(idx),
        Some(RecorderType::PortAudio) => {
            todo!();
        }
        Some(RecorderType::Cpal) => {
            todo!();
        }
        None => {
            // not initialized yet, default to pvrecorder
            pvrecorder::get_audio_device_name(idx)
        }
    }
}

#[cfg(test)]
mod recover_tests {
    use super::*;

    // the microphone is opened again only when the user wants the voice and Jarvis is not exiting
    #[test]
    fn the_microphone_is_never_reopened_against_the_settings_or_after_exit() {
        assert_eq!(recover_step(false, false), Recover::Try);
        assert_eq!(recover_step(false, true), Recover::Wait, "switched off by the user: stays off, the attempts wait");
        assert_eq!(recover_step(true, false), Recover::Stop, "Jarvis exits: nothing is opened");
        assert_eq!(recover_step(true, true), Recover::Stop);
        let delays: Vec<u64> = (0..7).map(|a| recover_delay(a).as_secs()).collect();
        assert_eq!(delays, [5, 10, 20, 40, 60, 60, 60], "a missing microphone is not tried every 5 s forever");
    }

    #[test]
    fn a_microphone_that_drops_out_again_and_again_is_told_once_in_ten_minutes() {
        let last = parking_lot::Mutex::new(None);
        let t0 = std::time::Instant::now();
        assert!(notice_due(&last, t0), "the first loss is told");
        assert!(!notice_due(&last, t0 + std::time::Duration::from_secs(60)), "a second loss a minute later is not");
        assert!(!notice_due(&last, t0 + std::time::Duration::from_secs(599)));
        assert!(notice_due(&last, t0 + LOST_NOTICE_EVERY), "after ten minutes it is told again");
    }
}

