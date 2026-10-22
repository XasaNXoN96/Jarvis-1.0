//! «Проверить имя»: for a few seconds the wake-word recognizer does not wake Jarvis but only writes down what it heard and
//! how close it is to the chosen name, so the user can see whether the name is understood. Nothing is saved or sent.
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(12);
const MAX_HEARD: usize = 8;

#[derive(Default)]
struct State {
    until: Option<Instant>,
    heard: Vec<(String, f64)>, // (what was heard, the best similarity to the name in percent)
    unknown: u32,              // speech that the recognizer could not turn into a word of its list
    matched: bool,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));

pub fn start() {
    let mut s = STATE.lock();
    *s = State::default();
    s.until = Some(Instant::now() + WINDOW);
}

pub fn stop() {
    STATE.lock().until = None;
}

pub fn active() -> bool {
    STATE.lock().until.map(|u| Instant::now() < u).unwrap_or(false)
}

fn ratio(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    seqdiff::ratio(&a, &b)
}

// what the recognizer heard (lower case; "[unk]" or "" = speech it did not understand). True = the test took it, nobody wakes.
pub fn offer(recognized: &str, names: &[String]) -> bool {
    if !active() {
        return false;
    }
    let text = recognized.trim().to_lowercase();
    let mut s = STATE.lock();
    if text.is_empty() || text == "[unk]" {
        s.unknown += 1;
        return true;
    }
    let mut best = 0.0f64;
    for w in text.split_whitespace().filter(|w| *w != "[unk]") {
        for n in names {
            best = best.max(ratio(n, w));
        }
    }
    if best >= crate::config::VOSK_MIN_RATIO {
        s.matched = true;
    }
    if s.heard.len() < MAX_HEARD {
        s.heard.push((text.chars().take(60).collect(), (best * 10.0).round() / 10.0));
    }
    true
}

pub fn status_json() -> Value {
    let s = STATE.lock();
    let left = s.until.map(|u| u.saturating_duration_since(Instant::now()).as_secs()).unwrap_or(0);
    let name = crate::uistate::assistant_display_name();
    json!({
        "active": left > 0 && s.until.is_some(),
        "seconds_left": left,
        "name": name,
        "heard": s.heard.iter().map(|(t, r)| json!({ "text": t, "ratio": r })).collect::<Vec<_>>(),
        "unknown": s.unknown,
        "matched": s.matched,
        "threshold": crate::config::VOSK_MIN_RATIO,
        "engine": format!("{:?}", crate::DB.get().map(|d| d.read().wake_word_engine).unwrap_or(crate::config::DEFAULT_WAKE_WORD_ENGINE)).to_lowercase(),
        "voice": crate::uistate::voice_state(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_takes_speech_only_while_it_runs_and_tells_a_match() {
        let names = vec!["карен".to_string()];
        stop();
        assert!(!offer("карен", &names), "nothing is taken when no test runs");
        start();
        assert!(active());
        assert!(offer("[unk]", &names) && offer("привет карен", &names));
        let j = status_json();
        assert_eq!(j["matched"], true);
        assert_eq!(j["unknown"], 1);
        assert_eq!(j["heard"][0]["text"], "привет карен");
        start();
        assert!(offer("совсем другое", &names));
        assert_eq!(status_json()["matched"], false, "a new test starts clean");
        stop();
        assert!(!active());
    }
}
