//! The statistics of the game sessions: while a game is in front, once in two seconds the real numbers are read (the frame rate when
//! it can be read, the load of the processor, the memory, the video card and its memory and temperature) and added to the session of that game.
//! When the game leaves the front for good, the session is written to the history. Numbers that cannot be read stay empty («—»), they
//! are never estimated. Only the exe name of the game and the numbers are stored, nothing about what is on the screen.

use crate::{store, sysstat};
use serde::{Deserialize, Serialize};

const FILE: &str = "game_sessions.json";
const MAX_SESSIONS: usize = 300;
// the game must be away this long to end the session (alt-tab for a moment does not split it)
pub const AWAY_SECS: u64 = 60;
// the tracker is fed every 2 s: a longer silence with the same game «in front» means the computer slept (or Jarvis was frozen);
// the hours of sleep are not game time, so the session ends where it was last seen
pub const SLEEP_GAP_SECS: u64 = 30;
pub const MIN_SESSION_SECS: u64 = 30;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Stat {
    pub n: u32,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
}

impl Stat {
    pub fn add(&mut self, v: f64) {
        if self.n == 0 {
            self.min = v;
            self.max = v;
        } else {
            self.min = self.min.min(v);
            self.max = self.max.max(v);
        }
        self.n += 1;
        self.sum += v;
    }
    pub fn avg(&self) -> Option<f64> {
        (self.n > 0).then(|| self.sum / self.n as f64)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Session {
    pub exe: String,
    pub start: u64,
    pub end: u64,
    #[serde(default)]
    pub fps: Stat,
    #[serde(default)]
    pub cpu: Stat,
    #[serde(default)]
    pub ram_gb: Stat,
    #[serde(default)]
    pub gpu: Stat,
    #[serde(default)]
    pub vram_gb: Stat,
    #[serde(default)]
    pub gpu_temp: Stat,
    // frame-rate samples, to get the 1% low
    #[serde(default)]
    pub fps_samples: Vec<u16>,
}

impl Session {
    pub fn new(exe: &str, start: u64) -> Self {
        Session { exe: exe.to_string(), start, end: start, ..Default::default() }
    }

    pub fn add(&mut self, now: u64, s: &sysstat::Sample, fps: Option<u32>) {
        self.end = now;
        self.cpu.add(s.cpu_pct as f64);
        if s.ram_total_gb > 0.0 {
            self.ram_gb.add(s.ram_used_gb as f64);
        }
        if let Some(g) = s.gpu_pct {
            self.gpu.add(g as f64);
        }
        if let Some(v) = s.gpu_mem_used_gb {
            self.vram_gb.add(v as f64);
        }
        if let Some(t) = s.gpu_temp_c {
            self.gpu_temp.add(t as f64);
        }
        if let Some(f) = fps {
            self.fps.add(f as f64);
            if self.fps_samples.len() < 5400 {
                self.fps_samples.push(f.min(u16::MAX as u32) as u16);
            }
        }
    }

    pub fn secs(&self) -> u64 {
        self.end.saturating_sub(self.start)
    }

    // the average of the worst 1% of the samples
    pub fn fps_low1(&self) -> Option<f64> {
        if self.fps_samples.len() < 20 {
            return None;
        }
        let mut v = self.fps_samples.clone();
        v.sort_unstable();
        let k = (v.len() / 100).max(1);
        Some(v[..k].iter().map(|x| *x as f64).sum::<f64>() / k as f64)
    }
}

// the tracker: feed it what is in front once in a while
#[derive(Default)]
pub struct Tracker {
    pub current: Option<Session>,
    away_since: Option<u64>,
}

impl Tracker {
    // `front` = the exe of the game in front (None: no game). Returns a finished session, if one just ended.
    pub fn tick(&mut self, now: u64, front: Option<&str>, s: &sysstat::Sample, fps: Option<u32>) -> Option<Session> {
        match (&mut self.current, front) {
            (Some(cur), Some(exe)) if cur.exe.eq_ignore_ascii_case(exe) && now.saturating_sub(cur.end) > SLEEP_GAP_SECS && self.away_since.is_none() => {
                let done = self.current.take();
                let mut n = Session::new(exe, now);
                n.add(now, s, fps);
                self.current = Some(n);
                done
            }
            (Some(cur), Some(exe)) if cur.exe.eq_ignore_ascii_case(exe) => {
                self.away_since = None;
                cur.add(now, s, fps);
                None
            }
            (Some(_), Some(_)) => {
                // another game: the old session ends now
                let done = self.current.take();
                self.away_since = None;
                self.current = front.map(|e| {
                    let mut n = Session::new(e, now);
                    n.add(now, s, fps);
                    n
                });
                done
            }
            (Some(_), None) => {
                let since = *self.away_since.get_or_insert(now);
                if now.saturating_sub(since) >= AWAY_SECS {
                    self.away_since = None;
                    self.current.take()
                } else {
                    None
                }
            }
            (None, Some(exe)) => {
                let mut n = Session::new(exe, now);
                n.add(now, s, fps);
                self.current = Some(n);
                self.away_since = None;
                None
            }
            (None, None) => None,
        }
    }
}

pub fn history() -> Vec<Session> {
    store::read::<Vec<Session>>(FILE).0
}

pub fn record(s: Session) {
    if s.secs() < MIN_SESSION_SECS {
        return;
    }
    let mut h = history();
    h.push(s);
    let n = h.len();
    if n > MAX_SESSIONS {
        h.drain(..n - MAX_SESSIONS);
    }
    let _ = store::write(FILE, &h);
}

pub fn clear() -> bool {
    store::write(FILE, &Vec::<Session>::new()).is_ok()
}

fn f1(v: Option<f64>, unit: &str) -> String {
    v.map(|x| format!("{:.0}{}", x, unit)).unwrap_or_else(|| "—".into())
}

pub fn summary_json() -> serde_json::Value {
    use serde_json::json;
    let h = history();
    let mut games: std::collections::BTreeMap<String, (u64, u32)> = Default::default();
    for s in &h {
        let e = games.entry(s.exe.clone()).or_default();
        e.0 += s.secs();
        e.1 += 1;
    }
    let mut totals: Vec<_> = games.into_iter().map(|(exe, (secs, n))| json!({ "exe": exe, "secs": secs, "sessions": n })).collect();
    totals.sort_by(|a, b| b["secs"].as_u64().cmp(&a["secs"].as_u64()));
    json!({
        "games": totals,
        "sessions": h.iter().rev().take(30).map(|s| json!({
            "exe": s.exe, "start": s.start, "secs": s.secs(),
            "fps_avg": s.fps.avg().map(|v| v.round()), "fps_low1": s.fps_low1().map(|v| v.round()),
            "cpu_avg": s.cpu.avg().map(|v| v.round()), "gpu_avg": s.gpu.avg().map(|v| v.round()),
            "ram_max_gb": if s.ram_gb.n > 0 { Some((s.ram_gb.max * 10.0).round() / 10.0) } else { None },
            "vram_max_gb": if s.vram_gb.n > 0 { Some((s.vram_gb.max * 10.0).round() / 10.0) } else { None },
            "gpu_temp_max": if s.gpu_temp.n > 0 { Some(s.gpu_temp.max) } else { None },
        })).collect::<Vec<_>>(),
        "fps_note": crate::gamefps::note(),
    })
}

// a text report of the last sessions (the page copies it)
pub fn report_text() -> String {
    let h = history();
    if h.is_empty() {
        return "Сессий игр пока нет: статистика пишется, когда игра в фокусе дольше 30 секунд. FPS читается только при запуске Jarvis от администратора.".into();
    }
    h.iter()
        .rev()
        .take(20)
        .map(|s| {
            format!(
                "{} · {} мин · FPS {} (1% low {}) · ЦП {} · ВК {} · темп. {}",
                s.exe,
                s.secs() / 60,
                f1(s.fps.avg(), ""),
                f1(s.fps_low1(), ""),
                f1(s.cpu.avg(), "%"),
                f1(s.gpu.avg(), "%"),
                if s.gpu_temp.n > 0 { format!("до {:.0} °C", s.gpu_temp.max) } else { "—".into() }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(feature = "jarvis_app")]
pub fn start() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut t = Tracker::default();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            let front = crate::aimassist::front_game_any();
            // (nothing is read while no game is in front: the monitor costs nothing then)
            if front.is_none() && t.current.is_none() {
                continue;
            }
            let s = sysstat::sample();
            if let Some(done) = t.tick(store::now(), front.as_deref(), &s, crate::gamefps::fps_now()) {
                record(done);
            }
        }
    });
}

#[cfg(not(feature = "jarvis_app"))]
pub fn start() {}

#[cfg(test)]
mod tests {
    use super::*;

    fn smp(cpu: u32, gpu: Option<u32>) -> sysstat::Sample {
        sysstat::Sample { cpu_pct: cpu, ram_used_gb: 10.0, ram_total_gb: 32.0, gpu_pct: gpu, gpu_temp_c: gpu.map(|_| 65), ..Default::default() }
    }

    #[test]
    fn numbers_are_real_and_missing_ones_stay_empty() {
        let mut s = Session::new("game.exe", 100);
        s.add(102, &smp(40, None), None);
        s.add(104, &smp(60, None), None);
        assert_eq!(s.cpu.avg(), Some(50.0));
        assert_eq!(s.fps.avg(), None);
        assert_eq!(s.gpu.avg(), None);
        assert_eq!(s.fps_low1(), None);
        assert_eq!(s.secs(), 4);
    }

    #[test]
    fn one_percent_low_is_the_worst_slice() {
        let mut s = Session::new("g.exe", 0);
        for i in 0..200u32 {
            s.add(i as u64, &smp(10, Some(50)), Some(if i % 100 == 0 { 20 } else { 100 }));
        }
        assert_eq!(s.fps_low1(), Some(20.0));
        assert!((s.fps.avg().unwrap() - 99.2).abs() < 0.1);
        assert_eq!(s.gpu_temp.max, 65.0);
    }

    #[test]
    fn a_short_alt_tab_does_not_split_the_session_and_a_long_absence_ends_it() {
        let mut t = Tracker::default();
        let s = smp(10, None);
        assert!(t.tick(0, Some("a.exe"), &s, None).is_none());
        assert!(t.tick(2, Some("a.exe"), &s, None).is_none());
        // 30 s away, then back
        assert!(t.tick(10, None, &s, None).is_none());
        assert!(t.tick(40, None, &s, None).is_none());
        assert!(t.tick(41, Some("A.EXE"), &s, None).is_none());
        assert_eq!(t.current.as_ref().unwrap().start, 0);
        // gone for good
        assert!(t.tick(50, None, &s, None).is_none());
        let done = t.tick(120, None, &s, None).unwrap();
        assert_eq!((done.exe.as_str(), done.start), ("a.exe", 0));
        assert!(t.current.is_none());
    }

    #[test]
    fn hours_of_sleep_are_not_game_time() {
        let mut t = Tracker::default();
        let s = smp(10, None);
        t.tick(0, Some("a.exe"), &s, None);
        t.tick(2, Some("a.exe"), &s, None);
        t.tick(4, Some("a.exe"), &s, None);
        // the computer sleeps for 8 hours with the game in front, then wakes up
        let done = t.tick(4 + 8 * 3600, Some("a.exe"), &s, None).expect("the old session ends where it was last seen");
        assert_eq!((done.start, done.end, done.secs()), (0, 4, 4));
        assert_eq!(t.current.as_ref().unwrap().start, 4 + 8 * 3600, "a new session starts after the wake-up");
        // the ordinary ticks every 2 seconds (and a short stutter) do not split a session
        t.tick(4 + 8 * 3600 + 2, Some("a.exe"), &s, None);
        assert!(t.tick(4 + 8 * 3600 + 20, Some("a.exe"), &s, None).is_none());
    }

    #[test]
    fn another_game_closes_the_first_one() {
        let mut t = Tracker::default();
        let s = smp(10, None);
        t.tick(0, Some("a.exe"), &s, None);
        let done = t.tick(100, Some("b.exe"), &s, None).unwrap();
        assert_eq!(done.exe, "a.exe");
        assert_eq!(t.current.as_ref().unwrap().exe, "b.exe");
    }

    #[test]
    fn history_skips_short_sessions_and_reports() {
        store::with_temp(|| {
            assert!(report_text().contains("пока нет"));
            let mut a = Session::new("a.exe", 0);
            a.add(10, &smp(10, None), None);
            record(a);
            assert!(history().is_empty(), "10 seconds is not a session");
            let mut b = Session::new("b.exe", 0);
            b.add(3600, &smp(30, Some(80)), Some(90));
            record(b);
            assert_eq!(history().len(), 1);
            assert!(report_text().contains("b.exe · 60 мин · FPS 90"));
            let j = summary_json();
            assert_eq!(j["games"][0]["exe"], "b.exe");
            assert_eq!(j["sessions"][0]["gpu_temp_max"], 65.0);
            assert!(j["sessions"][0]["fps_low1"].is_null(), "too few samples for a 1% low");
        });
    }
}
