//! Windows tells a program when the computer goes to sleep, wakes up and when the session is locked / unlocked. After a sleep the microphone
//! that was open before it is often dead; the watchdog of the microphone (wakewatch.rs) used to find that only by missing frames and could
//! have used its fast tries while the computer was still waking up. Here a wake-up / an unlock is turned into ONE request to the watchdog:
//! «start over and reopen the device once». Events that come in a flock (Windows sends several for one wake-up) are merged by a gate, and the
//! gate also limits how many such requests are accepted in ten minutes, so the microphone is never reopened in a loop.
//! The part that listens to Windows is small and exists only on Windows; the decisions are plain functions that the tests run everywhere.

use std::collections::VecDeque;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Event {
    // going to sleep (nothing to do, only noted)
    Suspend,
    // woke up (automatically or by the user)
    Resume,
    // the session was unlocked
    Unlock,
}

// a flock of events of one wake-up is one request
pub const MIN_GAP_MS: u64 = 15_000;
// and not more than this many requests in the window below
pub const MAX_PER_WINDOW: usize = 6;
pub const WINDOW_MS: u64 = 600_000;

#[derive(Default, Debug)]
pub struct Gate {
    accepted: VecDeque<u64>,
}

impl Gate {
    pub fn allow(&mut self, now: u64) -> bool {
        while self.accepted.front().map(|t| now.saturating_sub(*t) >= WINDOW_MS).unwrap_or(false) {
            self.accepted.pop_front();
        }
        if self.accepted.back().map(|t| now.saturating_sub(*t) < MIN_GAP_MS).unwrap_or(false) || self.accepted.len() >= MAX_PER_WINDOW {
            return false;
        }
        self.accepted.push_back(now);
        true
    }
}

// What to do with one event: true = the watchdog was asked to start over and reopen the microphone.
pub fn react(gate: &mut Gate, watch: &mut crate::wakewatch::Watch, now: u64, ev: Event) -> bool {
    match ev {
        Event::Suspend => false,
        Event::Resume | Event::Unlock => {
            if !gate.allow(now) {
                return false;
            }
            watch.resumed(now);
            true
        }
    }
}

static GATE: parking_lot::Mutex<Option<Gate>> = parking_lot::const_mutex(None);

// called by the thread that listens to Windows
pub fn notify(ev: Event) {
    match ev {
        Event::Suspend => crate::voicediag::note("device", "компьютер засыпает"),
        Event::Resume | Event::Unlock => {
            let allowed = GATE.lock().get_or_insert_with(Gate::default).allow(crate::wakewatch::now_ms());
            if allowed {
                crate::voicediag::note("device", if ev == Event::Resume { "компьютер проснулся: проверю микрофон" } else { "сеанс разблокирован: проверю микрофон" });
                crate::wakewatch::on_resume();
            }
        }
    }
}

pub fn start() {
    #[cfg(target_os = "windows")]
    sys::start();
}

#[cfg(target_os = "windows")]
mod sys {
    use super::{notify, Event};
    use std::sync::atomic::{AtomicBool, Ordering};
    use winapi::shared::minwindef::{BOOL, DWORD, LPARAM, LRESULT, UINT, WPARAM};
    use winapi::shared::windef::HWND;
    use winapi::um::libloaderapi::GetModuleHandleW;
    use winapi::um::winuser::{CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, TranslateMessage, MSG, WNDCLASSW};

    const WM_POWERBROADCAST: UINT = 0x0218;
    const PBT_APMSUSPEND: WPARAM = 0x0004;
    const PBT_APMRESUMESUSPEND: WPARAM = 0x0007;
    const PBT_APMRESUMEAUTOMATIC: WPARAM = 0x0012;
    const WM_WTSSESSION_CHANGE: UINT = 0x02B1;
    const WTS_SESSION_UNLOCK: WPARAM = 0x8;
    const NOTIFY_FOR_THIS_SESSION: DWORD = 0;

    #[link(name = "wtsapi32")]
    extern "system" {
        fn WTSRegisterSessionNotification(hwnd: HWND, flags: DWORD) -> BOOL;
    }

    static STARTED: AtomicBool = AtomicBool::new(false);

    unsafe extern "system" fn proc_(hwnd: HWND, msg: UINT, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_POWERBROADCAST => match w {
                PBT_APMSUSPEND => notify(Event::Suspend),
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => notify(Event::Resume),
                _ => {}
            },
            WM_WTSSESSION_CHANGE if w == WTS_SESSION_UNLOCK => notify(Event::Unlock),
            _ => {}
        }
        DefWindowProcW(hwnd, msg, w, l)
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn start() {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        let _ = std::thread::Builder::new().name("power-watch".into()).spawn(|| unsafe {
            // (an invisible window of its own: the broadcast of power messages goes to windows, not to a thread)
            let class = wide("JarvisPowerWatch");
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(proc_);
            wc.hInstance = GetModuleHandleW(std::ptr::null());
            wc.lpszClassName = class.as_ptr();
            if RegisterClassW(&wc) == 0 {
                log::warn!("Power watch: the window class was not registered; no wake-up events");
                return;
            }
            let title = wide("Jarvis power watch");
            let hwnd = CreateWindowExW(0, class.as_ptr(), title.as_ptr(), 0, 0, 0, 0, 0, std::ptr::null_mut(), std::ptr::null_mut(), wc.hInstance, std::ptr::null_mut());
            if hwnd.is_null() {
                log::warn!("Power watch: the window was not created; no wake-up events");
                return;
            }
            if WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION) == 0 {
                log::warn!("Power watch: the unlock of the session is not watched (the wake-up still is)");
            }
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wakewatch::{Action, Verdict, Watch, MAX_RESTARTS};

    #[test]
    fn a_flock_of_events_of_one_wake_up_is_one_request() {
        let (mut g, mut w) = (Gate::default(), Watch::default());
        assert!(react(&mut g, &mut w, 1_000, Event::Resume));
        assert!(!react(&mut g, &mut w, 1_500, Event::Resume), "Windows sends several messages for one wake-up");
        assert!(!react(&mut g, &mut w, 3_000, Event::Unlock), "the unlock a moment later is the same return");
        assert!(react(&mut g, &mut w, 1_000 + MIN_GAP_MS, Event::Unlock), "a later one is a new request");
        // going to sleep only notes
        assert!(!react(&mut g, &mut w, 900_000, Event::Suspend));
    }

    #[test]
    fn requests_are_limited_so_the_microphone_is_never_reopened_in_a_loop() {
        let (mut g, mut w) = (Gate::default(), Watch::default());
        let mut accepted = 0;
        for k in 0..40u64 {
            if react(&mut g, &mut w, k * MIN_GAP_MS, Event::Resume) {
                accepted += 1;
            }
        }
        // 40 events 15 s apart = 10 minutes: not more than MAX_PER_WINDOW get through
        assert_eq!(accepted, MAX_PER_WINDOW);
        // after the window has passed it works again
        assert!(react(&mut g, &mut w, 40 * MIN_GAP_MS + WINDOW_MS, Event::Resume));
    }

    #[test]
    fn after_a_wake_up_the_watchdog_starts_over_and_reopens_once() {
        let mut w = Watch::default();
        // before the sleep: audio flowed, then the tries were used up while the computer was waking (5 fast tries, the next one is 5 minutes away)
        w.beat(0, 300);
        let mut now = 20_000;
        while !w.gave_up {
            now += 25_000;
            if let Action::Reopen { .. } = w.next_action(now) {}
        }
        assert!(w.gave_up && w.restarts == MAX_RESTARTS);
        let just = now + 1_000;
        assert_eq!(w.next_action(just), Action::Nothing, "given up: the next try is 5 minutes away");
        // the wake-up event: fresh counters and one reopening at the next tick, not in 5 minutes
        let mut g = Gate::default();
        assert!(react(&mut g, &mut w, just, Event::Resume));
        assert_eq!((w.restarts, w.gave_up, w.resume_pending), (0, false, true));
        assert_eq!(w.next_action(just + 2_000), Action::Reopen { n: 1, after_resume: true, verdict: Verdict::Ok });
        // once: the next tick does not reopen again
        assert_eq!(w.next_action(just + 4_000), Action::Nothing);
        // the time asleep is not a stall: the clocks started at the wake-up
        assert_eq!(w.verdict(just + 4_000), Verdict::Ok);
        // if audio does not come back, the usual stall logic goes on, still with the limit: MAX_RESTARTS fast tries in all (the one after the
        // wake-up included), then it gives up and only the slow tries (every 5 minutes) remain
        let mut tries = 1;
        let mut t = just + 4_000;
        for _ in 0..40 {
            t += 25_000;
            if let Action::Reopen { after_resume, .. } = w.next_action(t) {
                assert!(!after_resume);
                tries += 1;
            }
            if w.gave_up {
                break;
            }
        }
        assert_eq!(tries, MAX_RESTARTS, "the whole episode is limited by the same number of tries");
        assert!(w.gave_up);
        assert_eq!(w.next_action(t + 1_000), Action::Nothing, "no immediate sixth try");
    }

    #[test]
    fn a_muted_microphone_or_a_running_command_is_not_reopened_by_a_wake_up() {
        let mut w = Watch::default();
        w.paused = true;
        w.resumed(10_000);
        assert_eq!(w.next_action(12_000), Action::Nothing);
        assert!(!w.resume_pending, "nothing is kept for later: switching the microphone on reopens it by itself");
        // a command is running: the reopening waits for its end
        let mut w = Watch::default();
        w.busy = 1;
        w.resumed(10_000);
        assert_eq!(w.next_action(12_000), Action::Nothing);
        w.busy = 0;
        assert!(matches!(w.next_action(14_000), Action::Reopen { after_resume: true, .. }));
    }

    #[test]
    fn a_wake_up_while_audio_flows_still_reopens_only_once_and_audio_clears_the_counters() {
        let mut w = Watch::default();
        w.beat(0, 300);
        w.resumed(5_000);
        assert!(matches!(w.next_action(7_000), Action::Reopen { n: 1, after_resume: true, .. }));
        w.beat(8_000, 400);
        assert_eq!(w.restarts, 0, "audio is back: the count starts again");
        w.beat(12_000, 400);
        assert_eq!(w.next_action(13_000), Action::Nothing);
    }

    #[test]
    fn the_global_entry_notes_and_asks_the_watchdog_without_panicking() {
        // (the global gate is shared with other tests: only that the call is safe and that sleeping asks for nothing)
        notify(Event::Suspend);
        notify(Event::Resume);
        notify(Event::Resume);
    }
}
