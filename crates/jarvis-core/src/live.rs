//! A live session of the linked phone on this computer: the camera of the computer shown on the phone, or the mouse and the keyboard of the
//! computer driven from the phone. A session is started on purpose (a request of the phone, with the permission of the owner switched on),
//! lasts only while the phone keeps using it, and ends by itself: after a silence (`idle`), after the longest time (`max`), or when somebody
//! stops it. The clock is given by the caller, so the rules are tested without waiting.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum End {
    // nothing runs (never started, or stopped)
    Stopped,
    // the phone was silent for too long
    Idle,
    // the longest time of one session is over
    TooLong,
}

impl End {
    pub fn text(self) -> &'static str {
        match self {
            End::Stopped => "сеанс не запущен",
            End::Idle => "сеанс закончился: телефон долго не отвечал",
            End::TooLong => "сеанс закончился: достигнуто наибольшее время одного сеанса",
        }
    }
}

pub struct Live {
    since: Option<Instant>,
    last: Option<Instant>,
    idle: Duration,
    max: Duration,
}

impl Live {
    pub const fn new(idle: Duration, max: Duration) -> Self {
        Live { since: None, last: None, idle, max }
    }

    // starts a session; true when it is a NEW one (false: it was already running, and only its silence timer is renewed)
    pub fn start(&mut self, now: Instant) -> bool {
        let fresh = self.check(now).is_err();
        if fresh {
            self.since = Some(now);
        }
        self.last = Some(now);
        fresh
    }

    // does the session still run at `now`? An expired one is cleared here, and the reason is returned.
    pub fn check(&mut self, now: Instant) -> Result<(), End> {
        let (Some(since), Some(last)) = (self.since, self.last) else { return Err(End::Stopped) };
        if now.saturating_duration_since(since) >= self.max {
            self.since = None;
            self.last = None;
            return Err(End::TooLong);
        }
        if now.saturating_duration_since(last) >= self.idle {
            self.since = None;
            self.last = None;
            return Err(End::Idle);
        }
        Ok(())
    }

    // a request of the phone: renews the silence timer if the session runs
    pub fn touch(&mut self, now: Instant) -> Result<(), End> {
        self.check(now)?;
        self.last = Some(now);
        Ok(())
    }

    pub fn stop(&mut self) -> bool {
        let was = self.since.is_some();
        self.since = None;
        self.last = None;
        was
    }

    pub fn active(&mut self, now: Instant) -> bool {
        self.check(now).is_ok()
    }

    pub fn seconds(&self, now: Instant) -> u64 {
        self.since.map(|s| now.saturating_duration_since(s).as_secs()).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn a_session_runs_while_the_phone_keeps_using_it_and_ends_by_silence() {
        let t0 = Instant::now();
        let mut l = Live::new(secs(20), secs(600));
        assert_eq!(l.check(t0), Err(End::Stopped), "never started");
        assert!(l.start(t0), "a new session");
        assert!(!l.start(t0 + secs(5)), "starting again renews it, it is not a second session");
        assert_eq!(l.touch(t0 + secs(15)), Ok(()));
        assert_eq!(l.touch(t0 + secs(30)), Ok(()), "each use renews the silence timer");
        assert_eq!(l.touch(t0 + secs(51)), Err(End::Idle), "21 s of silence");
        assert_eq!(l.check(t0 + secs(52)), Err(End::Stopped), "an ended session stays ended");
    }

    #[test]
    fn a_session_has_a_longest_time_however_busy_the_phone_is() {
        let t0 = Instant::now();
        let mut l = Live::new(secs(20), secs(60));
        l.start(t0);
        for i in 1..6 {
            assert_eq!(l.touch(t0 + secs(i * 10)), Ok(()));
        }
        assert_eq!(l.touch(t0 + secs(60)), Err(End::TooLong));
        assert!(l.start(t0 + secs(61)), "after the end a new session can be started on purpose");
    }

    #[test]
    fn stop_ends_at_once_and_says_whether_anything_ran() {
        let t0 = Instant::now();
        let mut l = Live::new(secs(20), secs(600));
        assert!(!l.stop(), "nothing ran");
        l.start(t0);
        assert!(l.active(t0 + secs(1)));
        assert_eq!(l.seconds(t0 + secs(7)), 7);
        assert!(l.stop());
        assert!(!l.active(t0 + secs(2)));
        assert_eq!(l.touch(t0 + secs(3)), Err(End::Stopped));
    }
}
