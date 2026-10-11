// microphone diagnostics: which device was found, whether a stream opened, whether audio really arrives,
// and a human-readable reason when it did not. Pure logic here, so it is testable without a sound card.
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use pv_recorder::{PvRecorderErrorStatus, PvRecorderStatus};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Cause {
    NoDevices,   // Windows lists no capture device at all
    Library,     // libpv_recorder.dll could not be loaded
    BusyOrDenied, // the backend refused to open it: used exclusively by another program, disabled, or access denied
    AlreadyOpen, // this device is already open (another Jarvis instance)
    Io,          // the stream opened but reading failed
    NoData,      // the stream opened but no audio frame arrived in time
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    OpenFailed,  // the device could not be opened
    StartFailed, // opened, but the stream did not start
    NoData,      // started, but no audio arrived
    Silent,      // audio arrives, but only digital silence (muted / wrong input)
    Ok,          // audio arrives with a signal
}

#[derive(Clone, Debug, Serialize)]
pub struct Attempt {
    pub index: i32, // -1 = system default
    pub name: String,
    pub stage: Stage,
    pub cause: Option<Cause>,
    pub detail: String,
    pub peak: u32,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Report {
    pub devices: Vec<String>,
    pub attempts: Vec<Attempt>,
    pub working: Option<i32>,
    pub summary: String,
}

pub static LAST_REPORT: Lazy<Mutex<Report>> = Lazy::new(|| Mutex::new(Report::default()));

// signal below this peak (of 32768) over the probe is treated as silence
pub const SILENCE_PEAK: u32 = 8;

// order in which devices are tried: the one the user chose, then the system default, then every other device
pub fn candidate_order(preferred: i32, device_count: usize) -> Vec<i32> {
    let mut out: Vec<i32> = Vec::new();
    let mut push = |i: i32| {
        if !out.contains(&i) {
            out.push(i);
        }
    };
    if preferred >= -1 && (preferred < 0 || (preferred as usize) < device_count) {
        push(preferred);
    }
    push(-1);
    for i in 0..device_count {
        push(i as i32);
    }
    out
}

// a saved device index that no longer exists means "system default"
pub fn validate_index(idx: i32, device_count: usize) -> i32 {
    if idx >= 0 && (idx as usize) >= device_count {
        -1
    } else if idx < -1 {
        -1
    } else {
        idx
    }
}

pub fn classify(status: &PvRecorderErrorStatus, opening: bool) -> (Cause, &'static str) {
    match status {
        PvRecorderErrorStatus::LibraryLoadError => (
            Cause::Library,
            "не загрузилась библиотека записи звука libpv_recorder.dll (файл удалён или заблокирован антивирусом): переустановите Jarvis",
        ),
        PvRecorderErrorStatus::LibraryError(PvRecorderStatus::DEVICE_ALREADY_INITIALIZED) => {
            (Cause::AlreadyOpen, "устройство уже открыто (возможно, запущена вторая копия Jarvis)")
        }
        // while opening, a runtime/IO failure means the backend could not use the device at all
        PvRecorderErrorStatus::LibraryError(PvRecorderStatus::IO_ERROR)
        | PvRecorderErrorStatus::LibraryError(PvRecorderStatus::RUNTIME_ERROR)
            if opening =>
        {
            (Cause::BusyOrDenied, "не удалось открыть устройство: его нет, оно занято другой программой, отключено, либо доступ запрещён")
        }
        PvRecorderErrorStatus::LibraryError(PvRecorderStatus::BACKEND_ERROR) => (
            Cause::BusyOrDenied,
            "Windows не дал открыть устройство: оно занято другой программой (в том числе в монопольном режиме), отключено, либо доступ запрещён",
        ),
        PvRecorderErrorStatus::LibraryError(PvRecorderStatus::IO_ERROR)
        | PvRecorderErrorStatus::LibraryError(PvRecorderStatus::RUNTIME_ERROR) => {
            (Cause::Io, "ошибка при чтении звука с устройства")
        }
        PvRecorderErrorStatus::LibraryError(PvRecorderStatus::OUT_OF_MEMORY) => (Cause::Other, "не хватило памяти"),
        PvRecorderErrorStatus::LibraryError(_) | PvRecorderErrorStatus::ArgumentError | PvRecorderErrorStatus::OtherError => {
            (Cause::Other, "неожиданная ошибка аудиобиблиотеки")
        }
    }
}

pub fn stage_from_peak(peak: u32) -> Stage {
    if peak < SILENCE_PEAK {
        Stage::Silent
    } else {
        Stage::Ok
    }
}

pub fn stage_text(a: &Attempt) -> String {
    let who = if a.index < 0 { format!("{} (по умолчанию)", a.name) } else { a.name.clone() };
    match a.stage {
        Stage::OpenFailed => format!("✗ «{}»: не открылось — {}", who, a.detail),
        Stage::StartFailed => format!("✗ «{}»: поток не запустился — {}", who, a.detail),
        Stage::NoData => format!("✗ «{}»: поток открыт, но звук не поступает — {}", who, a.detail),
        Stage::Silent => format!("⚠ «{}»: звук поступает, но сигнала нет (тишина). Проверьте, что микрофон не выключен и выбран нужный вход", who),
        Stage::Ok => format!("✓ «{}»: звук поступает, сигнал есть", who),
    }
}

// the line shown to the user when no device could be used
pub fn failure_summary(devices: &[String], attempts: &[Attempt]) -> String {
    if devices.is_empty() && attempts.iter().all(|a| a.cause == Some(Cause::Library)) && !attempts.is_empty() {
        return format!("микрофон не запущен: {}", attempts[0].detail);
    }
    if devices.is_empty() {
        let lib = attempts.iter().find(|a| a.cause == Some(Cause::Library));
        if let Some(a) = lib {
            return format!("микрофон не запущен: {}", a.detail);
        }
        return "Windows не показывает ни одного устройства записи. Включите микрофон: Параметры → Система → Звук → Ввод, и проверьте, что он подключён".to_string();
    }
    let mut parts: Vec<String> = attempts
        .iter()
        .take(3)
        .map(|a| format!("«{}»: {}", a.name, if a.detail.is_empty() { "нет данных" } else { &a.detail }))
        .collect();
    if attempts.len() > 3 {
        parts.push(format!("и ещё {}", attempts.len() - 3));
    }
    format!(
        "ни одно из устройств ({}) не удалось использовать. {}. Проверьте Параметры Windows → Конфиденциальность → Микрофон (разрешить классическим приложениям) и закройте программы, которые держат микрофон",
        devices.len(),
        parts.join("; ")
    )
}

pub fn store(report: Report) {
    *LAST_REPORT.lock() = report;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_tries_chosen_then_default_then_all() {
        assert_eq!(candidate_order(2, 4), vec![2, -1, 0, 1, 3]);
        assert_eq!(candidate_order(-1, 3), vec![-1, 0, 1, 2]);
        // a stale index is skipped, nothing is lost
        assert_eq!(candidate_order(9, 2), vec![-1, 0, 1]);
        // no devices listed: the default is still tried once
        assert_eq!(candidate_order(-1, 0), vec![-1]);
    }

    #[test]
    fn stale_index_becomes_default() {
        assert_eq!(validate_index(0, 0), -1);
        assert_eq!(validate_index(3, 3), -1);
        assert_eq!(validate_index(2, 3), 2);
        assert_eq!(validate_index(-1, 3), -1);
        assert_eq!(validate_index(-5, 3), -1);
    }

    #[test]
    fn errors_get_specific_causes() {
        assert_eq!(classify(&PvRecorderErrorStatus::LibraryLoadError, true).0, Cause::Library);
        assert_eq!(classify(&PvRecorderErrorStatus::LibraryError(PvRecorderStatus::BACKEND_ERROR), true).0, Cause::BusyOrDenied);
        assert_eq!(classify(&PvRecorderErrorStatus::LibraryError(PvRecorderStatus::DEVICE_ALREADY_INITIALIZED), true).0, Cause::AlreadyOpen);
        assert_eq!(classify(&PvRecorderErrorStatus::LibraryError(PvRecorderStatus::IO_ERROR), false).0, Cause::Io);
        assert_eq!(classify(&PvRecorderErrorStatus::LibraryError(PvRecorderStatus::RUNTIME_ERROR), true).0, Cause::BusyOrDenied);
    }

    #[test]
    fn silence_is_not_reported_as_working_signal() {
        assert_eq!(stage_from_peak(0), Stage::Silent);
        assert_eq!(stage_from_peak(SILENCE_PEAK - 1), Stage::Silent);
        assert_eq!(stage_from_peak(500), Stage::Ok);
    }

    fn attempt(name: &str, cause: Cause, detail: &str) -> Attempt {
        Attempt { index: 0, name: name.into(), stage: Stage::OpenFailed, cause: Some(cause), detail: detail.into(), peak: 0 }
    }

    #[test]
    fn summary_names_the_real_reason() {
        let none = failure_summary(&[], &[attempt("системный по умолчанию", Cause::BusyOrDenied, "x")]);
        assert!(none.contains("не показывает ни одного устройства"), "{}", none);
        let lib = failure_summary(&[], &[attempt("д", Cause::Library, "не загрузилась библиотека записи звука")]);
        assert!(lib.contains("библиотека"), "{}", lib);
        let busy = failure_summary(&["Mic".to_string()], &[attempt("Mic", Cause::BusyOrDenied, "занято или доступ запрещён")]);
        assert!(busy.contains("Mic") && busy.contains("занято"), "{}", busy);
        assert!(!busy.contains("не найден"), "{}", busy);
    }

    #[test]
    fn stage_lines_do_not_claim_success_without_audio() {
        let mut a = attempt("Mic", Cause::NoData, "тишина");
        a.stage = Stage::NoData;
        assert!(stage_text(&a).starts_with('✗'));
        a.stage = Stage::Silent;
        assert!(stage_text(&a).starts_with('⚠'));
        a.stage = Stage::Ok;
        assert!(stage_text(&a).starts_with('✓'));
    }

    // no sound card in the build sandbox: opening must fail cleanly (no panic, a specific report), and release must be safe
    #[test]
    fn opening_without_hardware_reports_and_does_not_panic() {
        let ok = super::super::pvrecorder::init_microphone(-1, 512);
        let report = LAST_REPORT.lock().clone();
        if !ok {
            assert!(!report.attempts.is_empty());
            assert!(!report.summary.is_empty());
        }
        super::super::pvrecorder::release();
        super::super::pvrecorder::release();
        assert!(!super::super::pvrecorder::is_open());
    }
}

// the DLL we ship must export every function the Rust wrapper loads; an old DLL made every start fail with
// "library could not be loaded" (a missing symbol is reported by the wrapper as a load error)
#[cfg(test)]
mod shipped_library {
    #[test]
    fn windows_dll_exports_everything_the_wrapper_needs() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib/windows/amd64/libpv_recorder.dll");
        let bytes = std::fs::read(path).expect("lib/windows/amd64/libpv_recorder.dll");
        for name in [
            "pv_recorder_init", "pv_recorder_delete", "pv_recorder_start", "pv_recorder_stop", "pv_recorder_read",
            "pv_recorder_set_debug_logging", "pv_recorder_get_is_recording", "pv_recorder_get_selected_device",
            "pv_recorder_get_available_devices", "pv_recorder_free_available_devices", "pv_recorder_sample_rate",
            "pv_recorder_version",
        ] {
            let mut needle = vec![0u8];
            needle.extend_from_slice(name.as_bytes());
            needle.push(0);
            assert!(bytes.windows(needle.len()).any(|w| w == needle.as_slice()), "{} is missing from libpv_recorder.dll", name);
        }
    }
}
