use parking_lot::RwLock;
use pv_recorder::{PvRecorder, PvRecorderBuilder};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use super::diag::{self, Attempt, Cause, Report, Stage};

use crate::APP_DIR;

// the open stream. Behind a lock (not a OnceCell) so it can be released and opened again.
static RECORDER: RwLock<Option<PvRecorder>> = RwLock::new(None);
static IS_RECORDING: AtomicBool = AtomicBool::new(false);
static ACTIVE_INDEX: AtomicI32 = AtomicI32::new(-1);
static READ_ERRORS: AtomicU32 = AtomicU32::new(0);
// frames delivered since start: lets the "Check" button prove that audio really arrives
pub static FRAMES: AtomicU64 = AtomicU64::new(0);
// milliseconds since the program start at the moment of the last audio frame (0 = none yet)
static LAST_FRAME_MS: AtomicU64 = AtomicU64::new(0);
static BASE: once_cell::sync::Lazy<std::time::Instant> = once_cell::sync::Lazy::new(std::time::Instant::now);

fn mark_frame() {
    LAST_FRAME_MS.store(BASE.elapsed().as_millis() as u64 + 1, Ordering::Relaxed);
}

// how long ago the microphone delivered audio (None: never)
pub fn last_frame_age_ms() -> Option<u64> {
    let last = LAST_FRAME_MS.load(Ordering::Relaxed);
    if last == 0 { None } else { Some((BASE.elapsed().as_millis() as u64 + 1).saturating_sub(last)) }
}

// how long we wait for the first audio frame from a freshly opened device
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_millis(1500);
// consecutive failed reads after which the microphone is reported as lost
const LOST_AFTER_ERRORS: u32 = 50;

// the library is also built into the program (Windows), so a lone jarvis-app.exe still has a microphone
#[cfg(target_os = "windows")]
static EMBEDDED_LIB: &[u8] = include_bytes!("../../../../lib/windows/amd64/libpv_recorder.dll");

// writes the built-in copy to a temp folder (the program folder may be read-only) and returns its path
#[cfg(target_os = "windows")]
fn embedded_library() -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("jarvis-pvrecorder-{}", EMBEDDED_LIB.len()));
    let path = dir.join("libpv_recorder.dll");
    let ok = std::fs::metadata(&path).map(|m| m.len() as usize == EMBEDDED_LIB.len()).unwrap_or(false);
    if !ok {
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, EMBEDDED_LIB)) {
            warn!("Could not unpack the built-in microphone library to {}: {}", path.display(), e);
            return None;
        }
    }
    Some(path)
}

#[cfg(not(target_os = "windows"))]
fn embedded_library() -> Option<PathBuf> {
    None
}

// pv_recorder looks for its library at a compile-time path (the build machine's folder),
// which does not exist on the user's PC. Look next to the program, then use the built-in copy.
fn library_path() -> Option<PathBuf> {
    let name = if cfg!(target_os = "windows") {
        "libpv_recorder.dll"
    } else if cfg!(target_os = "macos") {
        "libpv_recorder.dylib"
    } else {
        "libpv_recorder.so"
    };
    let candidates = [
        APP_DIR.join(name),
        APP_DIR.join("lib").join(name),
        APP_DIR.join("lib").join("windows").join("amd64").join(name),
    ];
    // the built-in copy always matches this build; a stale file next to the program (older installer) must not win
    let found = embedded_library().or_else(|| candidates.into_iter().find(|p| p.exists()));
    match &found {
        Some(p) => debug!("pvrecorder library: {}", p.display()),
        None => debug!("pvrecorder library not found next to the program, using the default path"),
    }
    found
}

fn builder(frame_length: i32) -> PvRecorderBuilder {
    let builder = PvRecorderBuilder::new(frame_length);
    match library_path() {
        Some(path) => builder.library_path(&path),
        None => builder,
    }
}

// same, but with the built-in copy (used when the file next to the program exists but cannot be loaded)
fn builder_embedded(frame_length: i32) -> Option<PvRecorderBuilder> {
    embedded_library().map(|p| PvRecorderBuilder::new(frame_length).library_path(&p))
}

fn device_name(devices: &[String], index: i32) -> String {
    if index < 0 {
        "системный по умолчанию".to_string()
    } else {
        devices.get(index as usize).cloned().unwrap_or_else(|| format!("устройство {}", index))
    }
}

// opens one device, starts it and waits for a real audio frame. On success the stream is returned running.
fn try_open(index: i32, frame_length: u32, devices: &[String]) -> Result<(PvRecorder, u32), Attempt> {
    let name = device_name(devices, index);
    let fail = |stage: Stage, cause: Cause, detail: String| Attempt { index, name: name.clone(), stage, cause: Some(cause), detail, peak: 0 };

    let mut opened = builder(frame_length as i32).device_index(index).init();
    // a broken or blocked library file next to the program: retry with the built-in copy
    if let Err(e) = &opened {
        if matches!(e.status(), pv_recorder::PvRecorderErrorStatus::LibraryLoadError) {
            warn!("Microphone library failed to load ({}), trying the built-in copy", e.message());
            if let Some(b) = builder_embedded(frame_length as i32) {
                opened = b.device_index(index).init();
            }
        }
    }
    let rec = match opened {
        Ok(r) => r,
        Err(e) => {
            let (cause, text) = diag::classify(e.status(), true);
            warn!("Microphone {} ({}): open failed: {:?}", index, name, e);
            let detail = if cause == Cause::Library { format!("{} [{}]", text, e.message()) } else { text.to_string() };
            return Err(fail(Stage::OpenFailed, cause, detail));
        }
    };
    if let Err(e) = rec.start() {
        let (cause, text) = diag::classify(e.status(), true);
        warn!("Microphone {} ({}): start failed: {:?}", index, name, e);
        return Err(fail(Stage::StartFailed, cause, text.to_string()));
    }

    // read a frame in a helper thread: a device that never delivers must not hang the program
    let (tx, rx) = std::sync::mpsc::channel();
    let reader = rec.clone();
    std::thread::spawn(move || {
        let _ = tx.send(reader.read());
    });
    match rx.recv_timeout(FIRST_FRAME_TIMEOUT) {
        Ok(Ok(frame)) => {
            let peak = frame.iter().map(|v| (*v as i32).unsigned_abs()).max().unwrap_or(0);
            Ok((rec, peak))
        }
        Ok(Err(e)) => {
            let (cause, text) = diag::classify(e.status(), false);
            warn!("Microphone {} ({}): read failed: {:?}", index, name, e);
            let _ = rec.stop();
            Err(fail(Stage::NoData, cause, text.to_string()))
        }
        Err(_) => {
            warn!("Microphone {} ({}): no audio frame within {:?}", index, name, FIRST_FRAME_TIMEOUT);
            let _ = rec.stop();
            Err(fail(Stage::NoData, Cause::NoData, "за 1,5 секунды не пришло ни одного блока звука".to_string()))
        }
    }
}

// Opens a microphone: the chosen device first, then the system default, then every other device.
// One device failing never means "no microphone": every candidate is tried and each result is kept in the report.
pub fn init_microphone(device_index: i32, frame_length: u32) -> bool {
    if RECORDER.read().is_some() {
        return true; // already open
    }
    let devices = list_audio_devices();
    let order = diag::candidate_order(device_index, devices.len());
    info!("Microphone: {} device(s) found, trying in order {:?}", devices.len(), order);

    let mut attempts: Vec<Attempt> = Vec::new();
    // with no devices listed we still try the system default once: the list itself may have failed
    for index in order {
        match try_open(index, frame_length, &devices) {
            Ok((rec, peak)) => {
                let name = device_name(&devices, index);
                let stage = diag::stage_from_peak(peak);
                info!("Microphone opened: {} (index {}), first frame peak {}", name, index, peak);
                attempts.push(Attempt { index, name, stage, cause: None, detail: String::new(), peak });
                *RECORDER.write() = Some(rec);
                ACTIVE_INDEX.store(index, Ordering::SeqCst);
                IS_RECORDING.store(true, Ordering::SeqCst);
                READ_ERRORS.store(0, Ordering::SeqCst);
                let summary = if index != device_index && device_index != -1 {
                    format!("выбранное устройство не открылось, использую «{}»", device_name(&devices, index))
                } else {
                    String::new()
                };
                diag::store(Report { devices, attempts, working: Some(index), summary });
                return true;
            }
            Err(a) => attempts.push(a),
        }
    }
    let summary = diag::failure_summary(&devices, &attempts);
    error!("Microphone: no device could be used. {}", summary);
    diag::store(Report { devices, attempts, working: None, summary });
    false
}

// stops and drops the stream so the device is free for another program or for a fresh start
pub fn release() {
    let rec = RECORDER.write().take();
    if let Some(rec) = rec {
        if IS_RECORDING.swap(false, Ordering::SeqCst) {
            let _ = rec.stop();
        }
        info!("Microphone released.");
    }
}

// the microphone is open and its last read gave audio (not a device that is gone and only fails)
pub fn delivering() -> bool {
    IS_RECORDING.load(Ordering::SeqCst) && READ_ERRORS.load(Ordering::Relaxed) == 0
}

pub fn read_microphone(frame_buffer: &mut [i16]) {
    let rec = RECORDER.read().clone();
    let Some(rec) = rec else {
        // no microphone (released, recovering): silence, never the last real frame again and again
        frame_buffer.fill(0);
        std::thread::sleep(Duration::from_millis(20));
        return;
    };
    match rec.read() {
        Ok(f) if f.len() == frame_buffer.len() => {
            frame_buffer.copy_from_slice(f.as_slice());
            FRAMES.fetch_add(1, Ordering::Relaxed);
            mark_frame();
            READ_ERRORS.store(0, Ordering::Relaxed);
        }
        Ok(_) => {
            frame_buffer.fill(0);
            std::thread::sleep(Duration::from_millis(20))
        }
        Err(msg) => {
            // a failed read gives silence (the old frame repeated would look like speech to the recognizer)
            frame_buffer.fill(0);
            // do not spin and flood the log: wait, and report once when the device is clearly gone
            let n = READ_ERRORS.fetch_add(1, Ordering::Relaxed) + 1;
            if n == 1 {
                error!("Failed to read audio frame. {:?}", msg);
            }
            if n == LOST_AFTER_ERRORS {
                let (_, text) = diag::classify(msg.status(), false);
                let why = format!("микрофон перестал отдавать звук: {} (отключён, занят другой программой или Windows вышла из сна)", text);
                *crate::VOICE_ERROR.lock() = why.clone();
                crate::uistate::log_event("error", &why);
                if crate::recorder::lost_notice_due() {
                    crate::notify::push("system_error", "Микрофон", &format!("{}. Джарвис пробует подключить его снова; команды текстом работают.", why), true);
                }
                // the device may come back (after sleep, a reconnected headset): it is opened again in the background
                crate::recorder::recover_in_background();
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

pub fn start_recording(device_index: i32, frame_length: u32) -> Result<(), ()> {
    if !init_microphone(device_index, frame_length) {
        error!("Cannot start recording: microphone is not available.");
        return Err(());
    }
    if IS_RECORDING.load(Ordering::SeqCst) {
        info!("START recording from microphone ...");
        return Ok(());
    }
    let rec = RECORDER.read().clone();
    match rec.map(|r| r.start()) {
        Some(Ok(_)) => {
            info!("START recording from microphone ...");
            IS_RECORDING.store(true, Ordering::SeqCst);
            Ok(())
        }
        Some(Err(msg)) => {
            error!("Failed to START audio recording: {}", msg);
            Err(())
        }
        None => Err(()),
    }
}

pub fn stop_recording() -> Result<(), ()> {
    release();
    Ok(())
}

pub fn active_index() -> i32 {
    ACTIVE_INDEX.load(Ordering::SeqCst)
}

pub fn is_open() -> bool {
    RECORDER.read().is_some()
}

// "Check" button while Jarvis is NOT listening: probe every device without keeping any of them open
pub fn probe_all() -> Report {
    let devices = list_audio_devices();
    let mut attempts = Vec::new();
    let mut order = diag::candidate_order(-1, devices.len());
    order.truncate(8);
    let mut working = None;
    for index in order {
        match try_open(index, 512, &devices) {
            Ok((rec, peak)) => {
                let _ = rec.stop();
                if working.is_none() {
                    working = Some(index);
                }
                attempts.push(Attempt { index, name: device_name(&devices, index), stage: diag::stage_from_peak(peak), cause: None, detail: String::new(), peak });
            }
            Err(a) => attempts.push(a),
        }
    }
    let summary = if working.is_some() { String::new() } else { diag::failure_summary(&devices, &attempts) };
    Report { devices, attempts, working, summary }
}

pub fn list_audio_devices() -> Vec<String> {
    let audio_devices = builder(512).get_available_devices();
    match audio_devices {
        Ok(audio_devices) => audio_devices,
        Err(err) => {
            error!("Failed to get audio devices: {}", err);
            Vec::new()
        },
    }
}

pub fn get_audio_device_name(idx: i32) -> String {
    if idx == -1 {
        return String::from("System Default");
    }

    let audio_devices = list_audio_devices();
    let mut first_device: String = String::new();

    for (_idx, device) in audio_devices.iter().enumerate() {
        if idx as usize == _idx {
            return device.to_string();
        }

        if _idx == 0 {
            first_device = device.to_string()
        }
    }

    // return first device as default, if none were matched
    first_device
}
