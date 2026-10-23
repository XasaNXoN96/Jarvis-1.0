//! The watchdog of the microphone loop: the loop that listens for the wake word reports each frame it read (`beat`). If the frames stop
//! (the device was unplugged or the driver hung) the watchdog asks the recorder to reopen the device, with a pause between tries and
//! a limit; if the frames keep coming but are digital silence (muted in Windows, a dead device) it says so instead of looking «alive».
//! It never claims that the wake word is heard: it only reports whether audio flows, what it did about it, and what it could not do.

use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub const STALL_MS: u64 = 10_000;
pub const SILENT_MS: u64 = 20_000;
pub const RETRY_MS: u64 = 20_000;
pub const MAX_RESTARTS: u32 = 5;
// after the fast tries are used up: one more try this often (a microphone plugged in later is found again without a restart)
pub const SLOW_RETRY_MS: u64 = 300_000;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    Ok,
    // the loop is paused on purpose (microphone switched off)
    Paused,
    // no frames for this long
    Stalled(u64),
    // frames are all zero for this long
    Silent(u64),
}

#[derive(Default, Debug)]
pub struct Watch {
    pub last_beat: Option<u64>,
    pub zero_since: Option<u64>,
    pub paused: bool,
    // the loop is busy with a command (it does not read the microphone meanwhile): not a stall
    pub busy: u32,
    pub restarts: u32,
    pub last_restart: Option<u64>,
    pub gave_up: bool,
    // Windows woke up from sleep / the session was unlocked: the device is reopened once, with fresh counters (see `resumed`)
    pub resume_pending: bool,
}

// what the watchdog does at one tick
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Nothing,
    // reopen the device; `n` = the number of this try; `after_resume` = the reason is a return from sleep / unlock, not a measured stall
    Reopen { n: u32, after_resume: bool, verdict: Verdict },
}

impl Watch {
    pub fn beat(&mut self, now: u64, peak: u32) {
        self.last_beat = Some(now);
        if peak == 0 {
            self.zero_since.get_or_insert(now);
        } else {
            self.zero_since = None;
            // audio flows again: the count of tries starts over
            self.restarts = 0;
            self.gave_up = false;
        }
    }

    pub fn verdict(&self, now: u64) -> Verdict {
        if self.paused {
            return Verdict::Paused;
        }
        if self.busy > 0 {
            return Verdict::Ok;
        }
        match self.last_beat {
            None => Verdict::Ok,
            Some(b) if now.saturating_sub(b) >= STALL_MS => Verdict::Stalled(now - b),
            Some(_) => match self.zero_since {
                Some(z) if now.saturating_sub(z) >= SILENT_MS => Verdict::Silent(now - z),
                _ => Verdict::Ok,
            },
        }
    }

    // should the device be reopened now? (a stall only; a silent device is not helped by a reopening more than a few times)
    pub fn should_restart(&self, now: u64, v: Verdict) -> bool {
        if self.gave_up || self.restarts >= MAX_RESTARTS {
            // slow tries: only for a real stall (a silent device is not helped by reopening)
            return matches!(v, Verdict::Stalled(_)) && self.last_restart.map(|t| now.saturating_sub(t) >= SLOW_RETRY_MS).unwrap_or(true);
        }
        if !matches!(v, Verdict::Stalled(_) | Verdict::Silent(_)) {
            return false;
        }
        self.last_restart.map(|t| now.saturating_sub(t) >= RETRY_MS).unwrap_or(true)
    }

    // Windows returned from sleep (or the session was unlocked). A device that was open before the sleep is often dead after it, and the
    // fast tries may have been used up while the computer was still waking up (then the next try would wait 5 minutes). So: the counters
    // start over (still limited: MAX_RESTARTS), the clocks start from now (the time asleep is not a «stall»), and ONE reopening is asked for.
    pub fn resumed(&mut self, now: u64) {
        self.restarts = 0;
        self.gave_up = false;
        self.last_restart = None;
        self.zero_since = None;
        self.last_beat = Some(now);
        self.resume_pending = true;
    }

    // one tick of the watchdog
    pub fn next_action(&mut self, now: u64) -> Action {
        if self.paused {
            // the user switched the microphone off: nothing to bring back (switching it on reopens it by itself)
            self.resume_pending = false;
            return Action::Nothing;
        }
        if self.resume_pending && self.busy == 0 {
            self.resume_pending = false;
            self.restarted(now);
            return Action::Reopen { n: self.restarts, after_resume: true, verdict: Verdict::Ok };
        }
        let v = self.verdict(now);
        if self.should_restart(now, v) {
            self.restarted(now);
            return Action::Reopen { n: self.restarts, after_resume: false, verdict: v };
        }
        Action::Nothing
    }

    pub fn restarted(&mut self, now: u64) {
        self.restarts = (self.restarts + 1).min(MAX_RESTARTS);
        self.last_restart = Some(now);
        if self.restarts >= MAX_RESTARTS {
            self.gave_up = true;
        }
        // the clock of the silence starts again after a reopening
        self.zero_since = None;
        self.last_beat = Some(now);
    }
}

static WATCH: Mutex<Watch> = parking_lot::const_mutex(Watch { last_beat: None, zero_since: None, paused: false, busy: 0, restarts: 0, last_restart: None, gave_up: false, resume_pending: false });
static STARTED: AtomicBool = AtomicBool::new(false);
static T0: once_cell::sync::Lazy<Instant> = once_cell::sync::Lazy::new(Instant::now);

pub(crate) fn now_ms() -> u64 {
    T0.elapsed().as_millis() as u64
}

// called by the reading of every frame
pub fn beat(peak: u32) {
    WATCH.lock().beat(now_ms(), peak);
}

// kept while a command runs: the loop does not read the microphone then, which is not a stall
pub struct Busy;
pub fn busy() -> Busy {
    WATCH.lock().busy += 1;
    Busy
}
impl Drop for Busy {
    fn drop(&mut self) {
        let mut w = WATCH.lock();
        w.busy = w.busy.saturating_sub(1);
        w.last_beat = Some(now_ms());
        w.zero_since = None;
    }
}

// Windows woke up / the session was unlocked (see powerwatch.rs)
pub fn on_resume() {
    WATCH.lock().resumed(now_ms());
}

pub fn set_paused(p: bool) {
    let mut w = WATCH.lock();
    if w.paused != p {
        crate::voicediag::note("device", if p { "микрофон выключен" } else { "микрофон включён" });
    }
    w.paused = p;
    if !p {
        w.last_beat = Some(now_ms());
        w.zero_since = None;
    }
}

pub fn status_json() -> Value {
    let w = WATCH.lock();
    let v = w.verdict(now_ms());
    let (state, text) = match v {
        Verdict::Ok => ("ok", "Звук с микрофона идёт".to_string()),
        Verdict::Paused => ("paused", "Микрофон выключен".to_string()),
        Verdict::Stalled(ms) => ("stalled", format!("Звук не приходит уже {} с.{}", ms / 1000, if w.gave_up { " Переоткрыть микрофон не удалось: проверьте, что он подключён." } else { " Пробую открыть микрофон заново." })),
        Verdict::Silent(ms) => ("silent", format!("Микрофон открыт, но {} с тишина без единого звука: проверьте, не выключен ли он в Windows.", ms / 1000)),
    };
    json!({ "state": state, "text": text, "restarts": w.restarts, "gave_up": w.gave_up })
}

#[cfg(feature = "jarvis_app")]
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    crate::powerwatch::start();
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let now = now_ms();
        let mut w = WATCH.lock();
        let v = w.verdict(now);
        match v {
            Verdict::Stalled(ms) => crate::voicediag::note("stream", &format!("звук не приходит {} с", ms / 1000 / 10 * 10)),
            Verdict::Silent(ms) => crate::voicediag::note("stream", &format!("тишина без звука {} с", ms / 1000 / 10 * 10)),
            Verdict::Ok if w.restarts > 0 || w.gave_up => crate::voicediag::note("stream", "звук идёт"),
            _ => {}
        }
        if let Action::Reopen { n, after_resume, verdict } = w.next_action(now) {
            drop(w);
            if after_resume {
                crate::voicediag::note("device", "после сна или блокировки открываю микрофон заново");
                log::warn!("Microphone watchdog: Windows resumed, reopening the device once");
            } else {
                crate::voicediag::note("device", &format!("переоткрываю микрофон (попытка {})", n));
                log::warn!("Microphone watchdog: {:?}, reopening the device (try {} of {})", verdict, n, MAX_RESTARTS);
            }
            // (one recovery thread at a time: `recover_in_background` refuses a second one while the first works)
            crate::recorder::recover_in_background();
        }
    });
}

#[cfg(not(feature = "jarvis_app"))]
pub fn start() {
    let _ = &STARTED;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_living_loop_is_ok_and_a_paused_one_is_not_blamed() {
        let mut w = Watch::default();
        assert_eq!(w.verdict(0), Verdict::Ok, "nothing read yet");
        w.beat(1000, 300);
        assert_eq!(w.verdict(1500), Verdict::Ok);
        w.paused = true;
        assert_eq!(w.verdict(60_000), Verdict::Paused);
    }

    #[test]
    fn a_stall_is_found_and_the_device_is_reopened_with_pauses_and_a_limit() {
        let mut w = Watch::default();
        w.beat(0, 100);
        assert_eq!(w.verdict(STALL_MS - 1_000), Verdict::Ok);
        let t0 = STALL_MS + 2_000;
        let v = w.verdict(t0);
        assert_eq!(v, Verdict::Stalled(t0));
        assert!(w.should_restart(t0, v));
        w.restarted(t0);
        // right after a reopening the clock starts again: no second reopening at once
        assert_eq!(w.verdict(t0 + 1_000), Verdict::Ok);
        assert!(!w.should_restart(t0 + 1_000, Verdict::Stalled(STALL_MS)));
        let mut now = t0;
        let mut tries = 1;
        while tries < 10 {
            now += RETRY_MS + STALL_MS;
            let v = w.verdict(now);
            if w.should_restart(now, v) {
                w.restarted(now);
                tries += 1;
            } else {
                break;
            }
        }
        assert_eq!(w.restarts, MAX_RESTARTS, "it stops after the limit");
        assert!(w.gave_up);
        // audio came back: the count starts again
        w.beat(now + 100, 500);
        assert_eq!((w.restarts, w.gave_up), (0, false));
    }

    #[test]
    fn after_giving_up_it_still_tries_slowly_but_not_for_silence() {
        let mut w = Watch::default();
        w.beat(0, 100);
        let mut now = 20_000;
        while !w.gave_up {
            w.restarted(now);
            now += 1;
        }
        let t = now;
        assert_eq!(w.restarts, MAX_RESTARTS);
        assert!(!w.should_restart(t + 60_000, Verdict::Stalled(60_000)), "not at once");
        assert!(w.should_restart(t + SLOW_RETRY_MS, Verdict::Stalled(60_000)), "a slow try after the pause");
        assert!(!w.should_restart(t + SLOW_RETRY_MS, Verdict::Silent(60_000)), "silence is not helped by reopening");
        w.restarted(t + SLOW_RETRY_MS);
        assert_eq!(w.restarts, MAX_RESTARTS, "the count does not grow without a limit");
        assert!(w.gave_up);
    }

    #[test]
    fn digital_silence_is_reported_as_silence() {
        let mut w = Watch::default();
        for t in (0..=25_000).step_by(100) {
            w.beat(t, 0);
        }
        assert_eq!(w.verdict(25_000), Verdict::Silent(25_000));
        w.beat(25_100, 40);
        assert_eq!(w.verdict(25_200), Verdict::Ok);
    }

    #[test]
    fn a_command_that_runs_for_a_long_time_is_not_a_stall() {
        let mut w = Watch::default();
        w.beat(0, 100);
        w.busy = 1;
        assert_eq!(w.verdict(120_000), Verdict::Ok);
        w.busy = 0;
        assert!(matches!(w.verdict(120_000), Verdict::Stalled(_)));
    }

    #[test]
    fn the_status_text_is_honest() {
        set_paused(true);
        assert_eq!(status_json()["state"], "paused");
        set_paused(false);
        beat(10);
        assert_eq!(status_json()["state"], "ok");
    }
}
