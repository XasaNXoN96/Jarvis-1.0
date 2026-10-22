//! Protection of the requests of a phone that CHANGE something (a command, an action, a voice) against being played again.
//! Every such request carries the time of the phone (`X-Ts`) and a unique number (`X-Rid`, a nonce). The computer accepts a request only
//! when the time is within two minutes of its own clock and the number was never seen: the numbers it has accepted are kept in a file, so a
//! restart does not forget them. A request that arrives twice (a retry after a lost answer, or a copy of it that somebody recorded) is
//! NOT run the second time. A device also has a limit of requests per ten seconds.
//!
//! What this does NOT do: over plain http inside the home network an eavesdropper who sees the secret of the device can send NEW requests
//! with new numbers. The numbers stop the replay of a recorded request, not the use of a stolen secret: for that use the https address of
//! the remote or a VPN, and revoke the device in the window.
use crate::store;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};

const FILE: &str = "remote_nonces.json";
// a request is accepted when its time is this close to the clock of the computer (seconds)
pub const WINDOW: u64 = 120;
// a number is kept this long: longer than a request can be valid (the window on both sides, plus a margin)
const KEEP: u64 = WINDOW * 2 + 60;
const MAX_KEPT: usize = 2000;
// the limit of one device
const RATE_N: usize = 30;
const RATE_SECS: u64 = 10;

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Verdict {
    Accept,
    // the same number again: nothing runs, the answer says it was already done
    Duplicate,
    // no time or no number, or an unusable one
    Malformed,
    // the time is too far from the clock of the computer (the answer tells the time of the computer)
    Stale,
    TooMany,
}

#[derive(Serialize, Deserialize, Default)]
struct Kept {
    // (nonce, accepted at)
    seen: VecDeque<(String, u64)>,
}

struct State {
    seen: VecDeque<(String, u64)>,
    loaded: bool,
    rate: HashMap<String, VecDeque<u64>>,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State { seen: VecDeque::new(), loaded: false, rate: HashMap::new() }));

fn valid_nonce(n: &str) -> bool {
    (8..=64).contains(&n.len()) && n.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn load_into(st: &mut State) {
    if !st.loaded {
        st.loaded = true;
        st.seen = store::read::<Kept>(FILE).0.seen;
    }
}

fn save(st: &State) {
    #[cfg(test)]
    if store::TEST_DIR.lock().is_none() {
        return;
    }
    let _ = store::write(FILE, &Kept { seen: st.seen.clone() });
}

/// `who`: the id of the device (or "shared" for the shared link); `ts`/`nonce`: the headers as they came.
pub fn accept(who: &str, ts: &str, nonce: &str, now: u64) -> Verdict {
    // (the tests of the remote send many requests in a second from one "device"; the limit has its own test)
    #[cfg(test)]
    let limit = 10_000;
    #[cfg(not(test))]
    let limit = RATE_N;
    accept_with_limit(who, ts, nonce, now, limit)
}

pub fn accept_with_limit(who: &str, ts: &str, nonce: &str, now: u64, limit: usize) -> Verdict {
    let Ok(ts) = ts.trim().parse::<u64>() else { return Verdict::Malformed };
    let nonce = nonce.trim();
    if !valid_nonce(nonce) {
        return Verdict::Malformed;
    }
    if now.abs_diff(ts) > WINDOW {
        return Verdict::Stale;
    }
    let mut st = STATE.lock();
    load_into(&mut st);
    while st.seen.front().map(|(_, at)| now.saturating_sub(*at) > KEEP).unwrap_or(false) {
        st.seen.pop_front();
    }
    if st.seen.iter().any(|(n, _)| n == nonce) {
        return Verdict::Duplicate;
    }
    // the limit counts requests that were new
    let q = st.rate.entry(who.to_string()).or_default();
    while q.front().map(|t| now.saturating_sub(*t) >= RATE_SECS).unwrap_or(false) {
        q.pop_front();
    }
    if q.len() >= limit {
        return Verdict::TooMany;
    }
    q.push_back(now);
    if st.rate.len() > 200 {
        st.rate.retain(|_, v| !v.is_empty());
    }
    if st.seen.len() >= MAX_KEPT {
        st.seen.pop_front();
    }
    st.seen.push_back((nonce.to_string(), now));
    save(&st);
    Verdict::Accept
}

// a restart of the server: what is in memory is dropped and read again from the file (the tests use it; a real restart does the same by itself)
#[cfg(test)]
pub fn forget_memory_for_tests() {
    let mut st = STATE.lock();
    st.seen.clear();
    st.rate.clear();
    st.loaded = false;
}

#[cfg(test)]
pub fn reset_for_tests() {
    let mut st = STATE.lock();
    st.seen.clear();
    st.rate.clear();
    st.loaded = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_number_with_a_fresh_time_is_accepted_once() {
        store::with_temp(|| {
            reset_for_tests();
            let now = 1_000_000;
            assert_eq!(accept("d1", &now.to_string(), "nonce-aaaa-0001", now), Verdict::Accept);
            assert_eq!(accept("d1", &now.to_string(), "nonce-aaaa-0001", now + 1), Verdict::Duplicate, "the same request again is not run again");
            assert_eq!(accept("d2", &now.to_string(), "nonce-aaaa-0001", now + 1), Verdict::Duplicate, "another device cannot use the same number either");
            assert_eq!(accept("d1", &now.to_string(), "nonce-aaaa-0002", now + 1), Verdict::Accept);
        });
    }

    #[test]
    fn an_old_or_a_future_request_and_a_missing_or_broken_one_are_refused() {
        store::with_temp(|| {
            reset_for_tests();
            let now = 1_000_000;
            assert_eq!(accept("d", &(now - WINDOW - 1).to_string(), "nonce-aaaa-0003", now), Verdict::Stale);
            assert_eq!(accept("d", &(now + WINDOW + 1).to_string(), "nonce-aaaa-0004", now), Verdict::Stale);
            assert_eq!(accept("d", &(now - WINDOW).to_string(), "nonce-aaaa-0005", now), Verdict::Accept, "the edge of the window is inside");
            for (ts, n) in [("", "nonce-aaaa-0006"), ("abc", "nonce-aaaa-0006"), ("-5", "nonce-aaaa-0006"), ("1000000", ""), ("1000000", "short"), ("1000000", "has space in it 1"), ("1000000", "x;rm -rf /1234"), ("1000000", &"a".repeat(65)[..])] {
                assert_eq!(accept("d", ts, n, now), Verdict::Malformed, "{:?} {:?}", ts, n);
            }
        });
    }

    #[test]
    fn a_number_is_remembered_after_the_server_restarts_and_forgotten_when_it_can_no_longer_be_valid() {
        store::with_temp(|| {
            reset_for_tests();
            let now = 2_000_000;
            assert_eq!(accept("d", &now.to_string(), "nonce-restart-01", now), Verdict::Accept);
            forget_memory_for_tests();
            assert_eq!(accept("d", &now.to_string(), "nonce-restart-01", now + 5), Verdict::Duplicate, "the file keeps it");
            // much later the old request is refused as stale anyway, and its number is dropped from the file
            let later = now + KEEP + 10;
            assert_eq!(accept("d", &now.to_string(), "nonce-restart-01", later), Verdict::Stale);
            assert_eq!(accept("d", &later.to_string(), "nonce-restart-02", later), Verdict::Accept);
            forget_memory_for_tests();
            let raw = std::fs::read_to_string(store::file(FILE)).unwrap();
            assert!(!raw.contains("nonce-restart-01") && raw.contains("nonce-restart-02"), "{}", raw);
        });
    }

    #[test]
    fn a_device_has_a_limit_per_ten_seconds_and_other_devices_are_not_affected() {
        store::with_temp(|| {
            reset_for_tests();
            let now = 3_000_000;
            for i in 0..RATE_N {
                assert_eq!(accept_with_limit("busy", &now.to_string(), &format!("nonce-rate-{:04}", i), now, RATE_N), Verdict::Accept);
            }
            assert_eq!(accept_with_limit("busy", &now.to_string(), "nonce-rate-9999", now, RATE_N), Verdict::TooMany);
            assert_eq!(accept_with_limit("calm", &now.to_string(), "nonce-rate-9998", now, RATE_N), Verdict::Accept);
            assert_eq!(accept_with_limit("busy", &(now + RATE_SECS).to_string(), "nonce-rate-9997", now + RATE_SECS, RATE_N), Verdict::Accept, "after ten seconds");
        });
    }
}
