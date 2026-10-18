//! The frame rate of the game in front, read the way PresentMon reads it: the system's own event trace (ETW) of the graphics provider
//! (Microsoft-Windows-DXGI) tells every time a program presents a frame. The game is NOT touched: nothing is injected into it, no file
//! of it is read or changed, so no anti-cheat sees anything. Windows allows a trace session only to an administrator (or a member of
//! the group «Performance Log Users»): without that rights the line of the overlay says so instead of a number.
//! DirectX 11/12 games are counted (a game that presents through another path shows no number). Not Windows: the functions do nothing.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    NotStarted,
    NeedAdmin,
    Running,
    Failed(String),
}

static STATUS: Lazy<Mutex<Status>> = Lazy::new(|| Mutex::new(Status::NotStarted));

pub fn status() -> Status {
    STATUS.lock().clone()
}

// the moments of the frames of every process (only the last second is kept)
#[derive(Default)]
pub struct Counter {
    frames: HashMap<u32, VecDeque<Instant>>,
}

impl Counter {
    pub fn record(&mut self, pid: u32, at: Instant) {
        let q = self.frames.entry(pid).or_default();
        q.push_back(at);
        while q.front().map(|f| at.duration_since(*f) > Duration::from_secs(2)).unwrap_or(false) {
            q.pop_front();
        }
        // (a table of many processes is cut: the most of them present nothing)
        if self.frames.len() > 256 {
            self.frames.retain(|_, q| q.back().map(|b| at.duration_since(*b) < Duration::from_secs(5)).unwrap_or(false));
        }
    }

    // frames in the last second (0 when the process shows nothing now)
    pub fn fps(&self, pid: u32, now: Instant) -> u32 {
        self.frames.get(&pid).map(|q| q.iter().filter(|t| now.duration_since(**t) <= Duration::from_secs(1)).count() as u32).unwrap_or(0)
    }
}

static COUNTER: Lazy<Mutex<Counter>> = Lazy::new(|| Mutex::new(Counter::default()));

// frames per second of the program that has the window in front; None while there is nothing to show
pub fn fps_now() -> Option<u32> {
    if status() != Status::Running {
        return None;
    }
    let pid = foreground_pid()?;
    Some(COUNTER.lock().fps(pid, Instant::now())).filter(|n| *n > 0)
}

// what to write in the stats line when there is no number
pub fn note() -> Option<&'static str> {
    match status() {
        Status::NeedAdmin => Some("FPS игры: нужен запуск от администратора"),
        Status::Failed(_) => Some("FPS игры: недоступен"),
        _ => None,
    }
}

#[cfg(not(target_os = "windows"))]
fn foreground_pid() -> Option<u32> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn start() {}

#[cfg(not(target_os = "windows"))]
pub fn stop() {}

#[cfg(target_os = "windows")]
fn foreground_pid() -> Option<u32> {
    unsafe {
        let w = winapi::um::winuser::GetForegroundWindow();
        if w.is_null() {
            return None;
        }
        let mut pid = 0u32;
        winapi::um::winuser::GetWindowThreadProcessId(w, &mut pid);
        (pid != 0).then_some(pid)
    }
}

#[cfg(target_os = "windows")]
pub use win::{start, stop};

#[cfg(target_os = "windows")]
mod win {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use winapi::shared::evntrace::*;
    use winapi::shared::guiddef::GUID;
    use winapi::um::evntcons::*;

    static STARTED: AtomicBool = AtomicBool::new(false);
    static SESSION: AtomicU64 = AtomicU64::new(0);
    const NAME: &str = "JarvisFpsTrace";
    // Microsoft-Windows-DXGI; the event 42 is «Present::Start»
    const DXGI: GUID = GUID { Data1: 0xCA11C036, Data2: 0x0102, Data3: 0x4A2D, Data4: [0xA6, 0xAD, 0xF0, 0x3C, 0xFE, 0xD5, 0xD3, 0xC9] };
    const PRESENT_START: u16 = 42;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    // the block of the properties: the structure and the name of the session after it
    fn properties() -> Vec<u64> {
        let size = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() + (NAME.len() + 1) * 2 + 8;
        let mut buf = vec![0u64; size / 8 + 1];
        unsafe {
            let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
            (*p).Wnode.BufferSize = (buf.len() * 8) as u32;
            (*p).Wnode.Flags = 0x0002_0000; // WNODE_FLAG_TRACED_GUID
            (*p).Wnode.ClientContext = 1; // the clock of the events: the performance counter
            (*p).LogFileMode = EVENT_TRACE_REAL_TIME_MODE;
            (*p).LoggerNameOffset = std::mem::size_of::<EVENT_TRACE_PROPERTIES>() as u32;
        }
        buf
    }

    unsafe extern "system" fn on_event(rec: PEVENT_RECORD) {
        if rec.is_null() {
            return;
        }
        let h = &(*rec).EventHeader;
        if h.EventDescriptor.Id == PRESENT_START {
            COUNTER.lock().record(h.ProcessId, Instant::now());
        }
    }

    pub fn start() {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| unsafe {
            let name = wide(NAME);
            // a session left by an earlier run that did not end properly is closed first
            let mut old = properties();
            ControlTraceW(std::ptr::null_mut(), name.as_ptr(), old.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP);
            let mut props = properties();
            let mut handle: TRACEHANDLE = std::ptr::null_mut();
            let rc = StartTraceW(&mut handle, name.as_ptr(), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
            if rc != 0 {
                *STATUS.lock() = if rc == 5 { Status::NeedAdmin } else { Status::Failed(format!("StartTrace {}", rc)) };
                return;
            }
            SESSION.store(handle as usize as u64, Ordering::SeqCst);
            let rc = EnableTraceEx2(handle, &DXGI, EVENT_CONTROL_CODE_ENABLE_PROVIDER, TRACE_LEVEL_INFORMATION, 0, 0, 0, std::ptr::null_mut());
            if rc != 0 {
                *STATUS.lock() = Status::Failed(format!("EnableTrace {}", rc));
                stop();
                return;
            }
            let mut logger = wide(NAME);
            let mut file: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
            file.LoggerName = logger.as_mut_ptr();
            *file.u1.ProcessTraceMode_mut() = PROCESS_TRACE_MODE_REAL_TIME | PROCESS_TRACE_MODE_EVENT_RECORD;
            *file.u2.EventRecordCallback_mut() = Some(on_event);
            let h = OpenTraceW(&mut file);
            if h == INVALID_PROCESSTRACE_HANDLE {
                *STATUS.lock() = Status::Failed("OpenTrace".into());
                stop();
                return;
            }
            *STATUS.lock() = Status::Running;
            let mut hs = [h];
            // blocks until the session is stopped
            ProcessTrace(hs.as_mut_ptr(), 1, std::ptr::null_mut(), std::ptr::null_mut());
            CloseTrace(h);
        });
    }

    // the session must be stopped before the program ends: a real-time session of a dead process would keep its buffers until the restart
    pub fn stop() {
        let raw = SESSION.swap(0, Ordering::SeqCst);
        if raw != 0 {
            let handle = raw as usize as TRACEHANDLE;
            unsafe {
                let mut props = properties();
                ControlTraceW(handle, std::ptr::null(), props.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, EVENT_TRACE_CONTROL_STOP);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_of_the_last_second_are_counted_per_process() {
        let mut c = Counter::default();
        let t0 = Instant::now();
        for i in 0..60u64 {
            c.record(10, t0 + Duration::from_millis(i * 16));
        }
        c.record(20, t0);
        let now = t0 + Duration::from_millis(60 * 16);
        assert!(c.fps(10, now) >= 55 && c.fps(10, now) <= 61, "{}", c.fps(10, now));
        assert_eq!(c.fps(20, t0 + Duration::from_millis(1200)), 0, "an old frame is not counted");
        assert_eq!(c.fps(99, now), 0);
        // two seconds later nothing is shown
        assert_eq!(c.fps(10, now + Duration::from_secs(3)), 0);
    }

    #[test]
    fn the_status_note_says_why_there_is_no_number() {
        *STATUS.lock() = Status::NeedAdmin;
        assert!(note().unwrap().contains("администратора"));
        *STATUS.lock() = Status::Failed("x".into());
        assert!(note().unwrap().contains("недоступен"));
        *STATUS.lock() = Status::NotStarted;
        assert!(note().is_none());
        assert!(fps_now().is_none());
    }
}
