// A token bucket: a door that lets a burst through and then only so many requests a minute. Used for the doors that are not the keyboard
// of this computer (the phone, the Telegram bot): a broken page or a flooding client must not start a hundred commands a second.
//
// The idea is the rate limiter of OpenJarvis (https://github.com/open-jarvis/OpenJarvis, src/openjarvis/security/rate_limiter.py,
// Apache-2.0); this is our own code. The clock is given by the caller (milliseconds), so everything is tested without waiting.

use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Bucket {
    capacity: f64,
    per_ms: f64,
    tokens: f64,
    last_ms: u64,
}

impl Bucket {
    // `burst` requests at once, then `per_minute` a minute
    pub fn new(burst: u32, per_minute: u32, now_ms: u64) -> Self {
        let capacity = burst.max(1) as f64;
        Bucket { capacity, per_ms: per_minute.max(1) as f64 / 60_000.0, tokens: capacity, last_ms: now_ms }
    }

    // Ok: let it pass. Err(ms): refused, so many milliseconds until one more request is allowed
    pub fn take(&mut self, now_ms: u64) -> Result<(), u64> {
        let gone = now_ms.saturating_sub(self.last_ms) as f64;
        self.tokens = (self.tokens + gone * self.per_ms).min(self.capacity);
        self.last_ms = now_ms.max(self.last_ms);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err((((1.0 - self.tokens) / self.per_ms).ceil() as u64).max(1))
        }
    }
}

// one bucket per key (a door, a sender); a key that was quiet for long is forgotten
#[derive(Debug)]
pub struct Limiter {
    burst: u32,
    per_minute: u32,
    buckets: HashMap<String, Bucket>,
}

const MAX_KEYS: usize = 256;
const FORGET_AFTER_MS: u64 = 10 * 60_000;

impl Limiter {
    pub fn new(burst: u32, per_minute: u32) -> Self {
        Limiter { burst, per_minute, buckets: HashMap::new() }
    }

    pub fn allow(&mut self, key: &str, now_ms: u64) -> Result<(), u64> {
        if self.buckets.len() >= MAX_KEYS && !self.buckets.contains_key(key) {
            self.buckets.retain(|_, b| now_ms.saturating_sub(b.last_ms) < FORGET_AFTER_MS);
            if self.buckets.len() >= MAX_KEYS {
                // a flood of new names: the oldest are dropped, the new one still gets a bucket
                if let Some(oldest) = self.buckets.iter().min_by_key(|(_, b)| b.last_ms).map(|(k, _)| k.clone()) {
                    self.buckets.remove(&oldest);
                }
            }
        }
        let (burst, per_minute) = (self.burst, self.per_minute);
        self.buckets.entry(key.to_string()).or_insert_with(|| Bucket::new(burst, per_minute, now_ms)).take(now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_burst_passes_then_the_rate_holds() {
        let mut b = Bucket::new(3, 60, 0);
        assert!(b.take(0).is_ok() && b.take(0).is_ok() && b.take(0).is_ok());
        let wait = b.take(0).unwrap_err();
        assert_eq!(wait, 1000, "60 a minute is one a second");
        assert!(b.take(500).is_err());
        assert!(b.take(1000).is_ok());
        assert!(b.take(1000).is_err());
    }

    #[test]
    fn the_bucket_never_holds_more_than_the_burst() {
        let mut b = Bucket::new(2, 600, 0);
        // a long quiet time does not save up more than two
        assert!(b.take(10_000_000).is_ok() && b.take(10_000_000).is_ok());
        assert!(b.take(10_000_000).is_err());
    }

    #[test]
    fn a_clock_that_goes_back_does_not_break_it() {
        let mut b = Bucket::new(1, 60, 5000);
        assert!(b.take(5000).is_ok());
        assert!(b.take(1000).is_err(), "earlier time gives nothing back");
        assert!(b.take(6000).is_ok());
    }

    #[test]
    fn doors_are_counted_apart() {
        let mut l = Limiter::new(1, 60);
        assert!(l.allow("phone", 0).is_ok());
        assert!(l.allow("phone", 0).is_err());
        assert!(l.allow("telegram", 0).is_ok(), "another door has its own bucket");
    }

    #[test]
    fn many_names_do_not_grow_the_table_without_end() {
        let mut l = Limiter::new(1, 60);
        for i in 0..2000 {
            let _ = l.allow(&format!("k{}", i), i as u64);
        }
        assert!(l.buckets.len() <= MAX_KEYS);
    }
}
