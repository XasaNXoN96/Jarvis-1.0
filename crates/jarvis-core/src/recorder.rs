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
