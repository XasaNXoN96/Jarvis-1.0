// «Лагает»: how much of the time of one microphone frame (512 samples = 32 ms) the program spends to process it. The voice loop (jarvis-app) reports the time
// of every frame here; the settings show the result. When the work takes longer than the frame, the recognition falls behind the voice and everything
// said comes late: that is what the user feels as a lag. Nothing is guessed: the numbers are measured on the computer of the user.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

// one frame of the voice loop, in microseconds
pub const FRAME_US: u32 = 32_000;

static EMA_US: AtomicU32 = AtomicU32::new(0);
static PEAK_US: AtomicU32 = AtomicU32::new(0);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static LATE: AtomicU64 = AtomicU64::new(0);

// the work for one frame (the waiting for the microphone is not a work and is not counted)
pub fn record(work: Duration) {
    let us = work.as_micros().min(u32::MAX as u128) as u32;
    let old = EMA_US.load(Ordering::Relaxed);
    // a moving average: a new frame counts 5 %
    let new = if old == 0 { us } else { ((old as u64 * 95 + us as u64 * 5) / 100) as u32 };
    EMA_US.store(new, Ordering::Relaxed);
    let peak = PEAK_US.load(Ordering::Relaxed);
    // the peak falls slowly (1 % a frame), so that a rare long frame is still seen a few seconds later
    PEAK_US.store(us.max(peak - peak / 100), Ordering::Relaxed);
    FRAMES.fetch_add(1, Ordering::Relaxed);
    if us > FRAME_US {
        LATE.fetch_add(1, Ordering::Relaxed);
    }
}

pub fn reset() {
    EMA_US.store(0, Ordering::Relaxed);
    PEAK_US.store(0, Ordering::Relaxed);
    FRAMES.store(0, Ordering::Relaxed);
    LATE.store(0, Ordering::Relaxed);
}

// the verdict in words, for a load in percent of the frame
pub fn verdict(load_pct: u32) -> &'static str {
    match load_pct {
        0..=69 => "в норме: распознавание успевает за голосом",
        70..=99 => "на пределе: при нагрузке на компьютер (игра, браузер) речь может обрабатываться с задержкой",
        _ => "не успевает: речь обрабатывается медленнее, чем вы говорите, поэтому ответ приходит с опозданием",
    }
}

pub fn status_json() -> Value {
    let frames = FRAMES.load(Ordering::Relaxed);
    if frames < 50 {
        return json!({ "ok": false, "text": "Нагрузка на распознавание: пока мало данных (скажите несколько слов)." });
    }
    let ema = EMA_US.load(Ordering::Relaxed);
    let peak = PEAK_US.load(Ordering::Relaxed);
    let load = (ema as u64 * 100 / FRAME_US as u64) as u32;
    let late = LATE.load(Ordering::Relaxed);
    json!({
        "ok": true,
        "load_pct": load,
        "avg_ms": (ema as f64 / 100.0).round() / 10.0,
        "peak_ms": (peak as f64 / 100.0).round() / 10.0,
        "late_pct": (late * 1000 / frames) as f64 / 10.0,
        "text": format!(
            "Нагрузка на распознавание: {}% времени кадра ({:.1} мс из 32; пик {:.1} мс; кадров дольше 32 мс: {:.1}%): {}.",
            load, ema as f64 / 1000.0, peak as f64 / 1000.0, (late * 1000 / frames) as f64 / 10.0, verdict(load)
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_load_is_measured_against_the_frame_and_told_in_words() {
        reset();
        assert_eq!(status_json()["ok"], false, "not enough frames: no verdict");
        for _ in 0..200 {
            record(Duration::from_millis(8));
        }
        let s = status_json();
        assert_eq!(s["ok"], true);
        let load = s["load_pct"].as_u64().unwrap();
        assert!((23..=27).contains(&load), "8 ms of 32 ms is a quarter: {}", s);
        assert!(s["text"].as_str().unwrap().contains("в норме"), "{}", s);
        reset();
        for _ in 0..400 {
            record(Duration::from_millis(45));
        }
        let s = status_json();
        assert!(s["load_pct"].as_u64().unwrap() >= 100 && s["text"].as_str().unwrap().contains("не успевает"), "{}", s);
        assert!(s["late_pct"].as_f64().unwrap() > 99.0, "{}", s);
        assert_eq!((verdict(69), verdict(70)), (verdict(0), verdict(99)));
        assert_ne!(verdict(69), verdict(70));
        reset();
    }
}
