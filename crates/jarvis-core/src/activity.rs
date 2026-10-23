//! What Jarvis is doing right now, for the sphere of the window: recognizing speech, searching, executing a command. Each kind is a counter
//! that goes up when the work starts and down when the guard is dropped, so the sphere always returns to the normal state: after success, after
//! an error that is returned and after a panic (the guard is dropped while unwinding). Nothing here knows what was said or searched.

use std::cell::Cell;
use std::marker::PhantomData;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Recognizing,
    Searching,
    Executing,
}

impl Kind {
    fn index(self) -> usize {
        self as usize
    }
}

// one counter per kind. The program has ONE (`GLOBAL`); the tests make their own, so they do not disturb each other.
pub struct Tracker {
    counters: [AtomicUsize; 3],
}

static GLOBAL: Tracker = Tracker::new();

thread_local! {
    // the same counters for THIS thread only: what the thread that runs the work can see (the tests read it, the window reads the global ones)
    static HERE: [Cell<usize>; 3] = [Cell::new(0), Cell::new(0), Cell::new(0)];
}

impl Tracker {
    pub const fn new() -> Tracker {
        Tracker { counters: [AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0)] }
    }

    pub fn enter(&self, k: Kind) -> Guard<'_> {
        self.counters[k.index()].fetch_add(1, SeqCst);
        HERE.with(|h| h[k.index()].set(h[k.index()].get() + 1));
        Guard { tracker: self, kind: k, _same_thread: PhantomData }
    }

    // The word for the window; "" = nothing special. When several run together the one that the user waits for longest wins: executing, then searching.
    pub fn current(&self) -> &'static str {
        word(|k| self.counters[k.index()].load(SeqCst))
    }
}

fn word(count: impl Fn(Kind) -> usize) -> &'static str {
    if count(Kind::Executing) > 0 {
        "executing"
    } else if count(Kind::Searching) > 0 {
        "searching"
    } else if count(Kind::Recognizing) > 0 {
        "recognizing"
    } else {
        ""
    }
}

#[must_use = "the state ends when the guard is dropped"]
pub struct Guard<'a> {
    tracker: &'a Tracker,
    kind: Kind,
    // a guard ends on the thread that started it (so the counter of the thread stays right)
    _same_thread: PhantomData<*const ()>,
}

impl Drop for Guard<'_> {
    fn drop(&mut self) {
        self.tracker.counters[self.kind.index()].fetch_sub(1, SeqCst);
        HERE.with(|h| h[self.kind.index()].set(h[self.kind.index()].get().saturating_sub(1)));
    }
}

pub fn enter(k: Kind) -> Guard<'static> {
    GLOBAL.enter(k)
}

pub fn current() -> &'static str {
    GLOBAL.current()
}

// what THIS thread is doing now (the same words)
pub fn current_here() -> &'static str {
    HERE.with(|h| word(|k| h[k.index()].get()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_state_comes_and_goes_and_the_strongest_one_shows() {
        let t = Tracker::new();
        assert_eq!(t.current(), "");
        let r = t.enter(Kind::Recognizing);
        assert_eq!(t.current(), "recognizing");
        let s = t.enter(Kind::Searching);
        assert_eq!(t.current(), "searching");
        let e = t.enter(Kind::Executing);
        assert_eq!(t.current(), "executing");
        drop(e);
        assert_eq!(t.current(), "searching");
        drop(s);
        assert_eq!(t.current(), "recognizing");
        drop(r);
        assert_eq!(t.current(), "");
    }

    #[test]
    fn the_state_ends_after_an_error_after_a_panic_and_with_two_at_once() {
        let t = Tracker::new();
        // an error that is returned
        let work = || -> Result<(), String> {
            let _g = t.enter(Kind::Executing);
            assert_eq!(t.current(), "executing");
            Err("не получилось".into())
        };
        assert!(work().is_err());
        assert_eq!(t.current(), "");
        // a panic
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _g = t.enter(Kind::Searching);
            panic!("boom");
        }));
        assert!(r.is_err());
        assert_eq!(t.current(), "", "a panic does not leave the sphere in «ищу»");
        // two of the same kind: it ends only when both ended
        let a = t.enter(Kind::Recognizing);
        let b = t.enter(Kind::Recognizing);
        drop(a);
        assert_eq!(t.current(), "recognizing");
        drop(b);
        assert_eq!(t.current(), "");
    }

    // the real places are wired: this thread sees the state while the work runs and gets back to "" after it, also after an error
    #[test]
    fn the_places_that_work_set_the_state_and_give_it_back() {
        crate::store::with_temp(|| {
            assert_eq!(current_here(), "");
            // a command: refused by a missing pack path (an error), the state is gone after it
            let cmd: crate::commands::JCommand = serde_json::from_value(serde_json::json!({ "id": "activity_test", "type": "voice" })).unwrap();
            let _ = crate::commands::execute_command(&std::path::PathBuf::from("."), &cmd, None, None);
            assert_eq!(current_here(), "");
            // searches
            let _ = crate::kb::search("тест", 1);
            let _ = crate::memory::search("тест");
            let _ = crate::notes::search("тест");
            assert_eq!(current_here(), "");
            // a plan of the autopilot: «выполняю» is seen from the gate that is asked before every step; after a failing step it is over
            let seen = std::cell::RefCell::new(Vec::new());
            let gate = || {
                seen.borrow_mut().push(current_here());
                Ok(())
            };
            fn failing(_: &crate::scenario::Step, _: &mut Vec<isize>) -> Result<(), String> {
                Err("сбой".into())
            }
            fn quick(_: &crate::scenario::Step) -> std::time::Duration {
                std::time::Duration::from_secs(2)
            }
            let sc = crate::autopilot::parse_plan(r#"{"steps":[{"kind":"pause","param":"0.1"},{"kind":"pause","param":"0.1"}]}"#, "x").unwrap().scenario;
            let _l = crate::scenario::RUN_TEST_LOCK.lock();
            let r = crate::autopilot::execute_with(&sc, quick, failing, &gate);
            assert!(!r.ok);
            assert_eq!(*seen.borrow(), ["executing"]);
            assert_eq!(current_here(), "", "after the error the sphere is back to normal");
        });
    }
}
