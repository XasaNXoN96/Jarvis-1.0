//! The wizard of the key of the aim: Jarvis finds out WHICH key or button of the mouse the user holds to aim in ONE game, and the user
//! confirms it. Nothing is guessed from the name of the game, nothing is kept before the confirmation.
//!
//! What it does and does not do (the rules of the wizard):
//! - it runs only after the user pressed «Определить клавишу» for a game, only while the window of THAT game is in front, for at most
//!   three minutes, and it stops at once on a confirmation, a cancel, a timeout or the exit of Jarvis;
//! - it asks Windows (GetAsyncKeyState, the same call the aim assist uses) only for the state of a short list of keys and buttons of a game
//!   (letters, digits, the mouse buttons, Shift, Ctrl, Alt, Space, Caps Lock, Tab, F1–F12); it keeps only «key → how many times held»,
//!   never the order of the keys, a text, a time of day or anything from another window; the counts are dropped when the wizard ends;
//! - a held key is NOT proof that it aims (it may be a shot, a jump, an action): the wizard only says «this key was held several times»,
//!   and the user confirms the meaning. When two keys were used about equally it asks, it does not choose;
//! - a key is saved for one game only after the user confirmed it (or typed it); until then it is not used by the aim assist.
//! Not Windows: the session can be started and fed by the tests, but nothing reads the keyboard.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

// holds of the same key that make a proposal
pub const NEEDED: u32 = 4;
// a press shorter than this (milliseconds) is a tap, not a hold of an aim key
pub const MIN_HOLD_MS: u64 = 80;
// the wizard ends by itself
pub const TIMEOUT: Duration = Duration::from_secs(180);
// the check of a saved key waits this long for the key
pub const VERIFY_WAIT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    // looking: the user is asked to hold the key several times in the game
    Observing,
    // one key is clear: waits for «Да, сохранить»
    Suggested(String),
    // several keys were used about equally: the user chooses (key, holds)
    Ambiguous(Vec<(String, u32)>),
    // the check of a saved key: waits for it to be pressed in the game
    Verifying { key: String, seen: bool },
    Cancelled,
    TimedOut,
}

pub struct Session {
    pub exe: String,
    pub phase: Phase,
    // is the window of the game in front now (set by the reader; the holds outside the game are never counted)
    pub in_game: bool,
    counts: HashMap<String, u32>,
    started: Instant,
}

impl Session {
    pub fn new(exe: &str) -> Session {
        Session { exe: exe.to_string(), phase: Phase::Observing, in_game: false, counts: HashMap::new(), started: Instant::now() }
    }

    pub fn verifying(exe: &str, key: &str) -> Session {
        Session { exe: exe.to_string(), phase: Phase::Verifying { key: key.to_string(), seen: false }, in_game: false, counts: HashMap::new(), started: Instant::now() }
    }

    // one hold of a key that ended (the name of the key, how long it was held)
    pub fn observe(&mut self, key: &str, held_ms: u64) {
        // a key outside the game, a tap, or a wizard that is not looking: nothing is counted
        if !self.in_game {
            return;
        }
        match self.phase.clone() {
            Phase::Observing => {
                if held_ms < MIN_HOLD_MS || crate::aimassist::trigger_vk(key).is_none() {
                    return;
                }
                *self.counts.entry(key.to_string()).or_insert(0) += 1;
                self.phase = self.decide();
            }
            Phase::Verifying { key: want, .. } => {
                if key == want && held_ms >= 20 {
                    self.phase = Phase::Verifying { key: want, seen: true };
                }
            }
            _ => {}
        }
    }

    // the keys by the number of holds, the most used first
    pub fn ranking(&self) -> Vec<(String, u32)> {
        let mut v: Vec<(String, u32)> = self.counts.iter().map(|(k, n)| (k.clone(), *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }

    // the most holds of one key, at most NEEDED (for «Наблюдение 2 из 4»)
    pub fn progress(&self) -> u32 {
        self.ranking().first().map(|(_, n)| (*n).min(NEEDED)).unwrap_or(0)
    }

    fn decide(&self) -> Phase {
        let r = self.ranking();
        let Some((top, n)) = r.first().cloned() else { return Phase::Observing };
        if n < NEEDED {
            return Phase::Observing;
        }
        let second = r.get(1).map(|x| x.1).unwrap_or(0);
        // the proposal needs a clear lead: the next key must have been held less than half as often
        if second * 2 <= n {
            Phase::Suggested(top)
        } else {
            Phase::Ambiguous(r.into_iter().filter(|x| x.1 >= 2).take(3).collect())
        }
    }

    pub fn expired(&self) -> bool {
        let limit = if matches!(self.phase, Phase::Verifying { .. }) { VERIFY_WAIT } else { TIMEOUT };
        self.started.elapsed() > limit
    }

    // the wizard stops looking: the counts are dropped
    pub fn end(&mut self, phase: Phase) {
        self.counts.clear();
        self.phase = phase;
    }
}

static SESSION: Lazy<Mutex<Option<Session>>> = Lazy::new(|| Mutex::new(None));
// the number of the current reader: an older reader ends by itself when it is not the current one
static GEN: AtomicU64 = AtomicU64::new(0);

// is a wizard looking (or waiting for a confirmation) now
pub fn is_active() -> bool {
    SESSION.lock().as_ref().map(|s| matches!(s.phase, Phase::Observing | Phase::Verifying { .. })).unwrap_or(false)
}

fn begin(s: Session) {
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    *SESSION.lock() = Some(s);
    reader::start(gen);
}

// the wizard for a game: only the game the user named, only after his request
pub fn start(exe: &str) -> Result<(), String> {
    crate::aimassist::game_key(exe).ok_or("игра не выбрана")?;
    begin(Session::new(&exe.trim().to_lowercase()));
    Ok(())
}

// the check of the saved key of a game: «нажмите её в игре»
pub fn verify(exe: &str) -> Result<(), String> {
    let b = crate::aimassist::game_binding(exe);
    let key = b.usable_key().ok_or("для этой игры нет подтверждённой клавиши")?.to_string();
    begin(Session::verifying(&exe.trim().to_lowercase(), &key));
    Ok(())
}

// stops looking at once, drops what was counted (nothing is saved)
pub fn cancel() {
    GEN.fetch_add(1, Ordering::SeqCst);
    let mut g = SESSION.lock();
    if let Some(s) = g.as_mut() {
        if matches!(s.phase, Phase::Observing | Phase::Suggested(_) | Phase::Ambiguous(_) | Phase::Verifying { .. }) {
            s.end(Phase::Cancelled);
        }
    }
}

fn finish_quietly() {
    GEN.fetch_add(1, Ordering::SeqCst);
    *SESSION.lock() = None;
}

// one hold of a key (called by the reader; the tests call it too)
pub fn feed(key: &str, held_ms: u64) {
    if let Some(s) = SESSION.lock().as_mut() {
        s.observe(key, held_ms);
    }
}

pub fn set_in_game(v: bool) {
    if let Some(s) = SESSION.lock().as_mut() {
        s.in_game = v;
    }
}

// the key the user confirms: the proposal of the wizard, or one he chose from the list, or one he typed; saved for THIS game only
pub fn confirm(exe: &str, key: Option<&str>, mode: &str) -> Result<String, String> {
    let from_wizard;
    let chosen: String = match key.map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty()) {
        Some(k) => {
            // a key from the list of the wizard counts as «wizard», a key the wizard never saw as «manual»
            from_wizard = SESSION.lock().as_ref().map(|s| s.exe == exe.trim().to_lowercase() && s.ranking().iter().any(|(n, _)| *n == k)).unwrap_or(false);
            k
        }
        None => {
            let g = SESSION.lock();
            match g.as_ref().map(|s| (&s.exe, &s.phase)) {
                Some((e, Phase::Suggested(k))) if *e == exe.trim().to_lowercase() => {
                    from_wizard = true;
                    k.clone()
                }
                _ => return Err("нет предложенной клавиши для этой игры: определите её заново или выберите вручную".into()),
            }
        }
    };
    crate::aimassist::save_game_binding(exe, &chosen, crate::aimassist::BindStatus::Confirmed, if from_wizard { "wizard" } else { "manual" }, mode)?;
    finish_quietly();
    Ok(chosen)
}

// the key typed by the user (it wins over any proposal)
pub fn manual(exe: &str, key: &str, mode: &str) -> Result<(), String> {
    crate::aimassist::save_game_binding(exe, key, crate::aimassist::BindStatus::Confirmed, "manual", mode)?;
    finish_quietly();
    Ok(())
}

pub fn reset(exe: &str) -> Result<(), String> {
    cancel();
    crate::aimassist::reset_game_binding(exe)
}

// «ПКМ», «клавиша Q» …
pub fn key_label(key: &str) -> String {
    match key {
        "rmb" => "правая кнопка мыши (ПКМ)".into(),
        "lmb" => "левая кнопка мыши (ЛКМ)".into(),
        "mmb" => "колесо мыши (нажатие)".into(),
        "x1" => "боковая кнопка мыши 1".into(),
        "x2" => "боковая кнопка мыши 2".into(),
        "shift" => "Shift".into(),
        "ctrl" => "Ctrl".into(),
        "alt" => "Alt".into(),
        "space" => "Пробел".into(),
        "caps" => "Caps Lock".into(),
        "tab" => "Tab".into(),
        k => format!("клавиша {}", k.to_uppercase()),
    }
}

pub fn short_label(key: &str) -> String {
    match key {
        "rmb" => "ПКМ".into(),
        "lmb" => "ЛКМ".into(),
        "mmb" => "колесо".into(),
        "x1" => "боковая 1".into(),
        "x2" => "боковая 2".into(),
        "space" => "Пробел".into(),
        "caps" => "Caps".into(),
        k => k.to_uppercase(),
    }
}

fn binding_json(exe: &str) -> Value {
    let b = crate::aimassist::game_binding(exe);
    json!({ "key": b.key, "label": if b.key.is_empty() { String::new() } else { short_label(&b.key) }, "status": b.status.name(), "how": b.how, "mode": b.mode, "input": b.input(), "at": b.at })
}

// what the page shows: the state of the wizard (of the game it asks about) and the saved key of that game
pub fn status_json(exe: &str) -> Value {
    let want = exe.trim().to_lowercase();
    let mut g = SESSION.lock();
    if let Some(s) = g.as_mut() {
        if s.expired() && matches!(s.phase, Phase::Observing | Phase::Verifying { .. }) {
            s.end(Phase::TimedOut);
        }
    }
    let saved = binding_json(&want);
    let Some(s) = g.as_ref().filter(|s| s.exe == want) else {
        return json!({ "state": "idle", "exe": want, "saved": saved });
    };
    let (state, extra) = match &s.phase {
        Phase::Idle => ("idle", json!({})),
        Phase::Observing => ("observing", json!({ "progress": s.progress(), "needed": NEEDED, "in_game": s.in_game, "seen": s.ranking().iter().take(3).map(|(k, n)| json!({ "key": k, "label": short_label(k), "count": n })).collect::<Vec<_>>() })),
        Phase::Suggested(k) => ("suggested", json!({ "key": k, "label": short_label(k), "label_long": key_label(k) })),
        Phase::Ambiguous(list) => ("ambiguous", json!({ "candidates": list.iter().map(|(k, n)| json!({ "key": k, "label": short_label(k), "count": n })).collect::<Vec<_>>() })),
        Phase::Verifying { key, seen } => ("verifying", json!({ "key": key, "label": short_label(key), "seen": seen, "in_game": s.in_game })),
        Phase::Cancelled => ("cancelled", json!({})),
        Phase::TimedOut => ("timeout", json!({})),
    };
    let mut v = json!({ "state": state, "exe": want, "saved": saved });
    if let (Some(o), Some(e)) = (v.as_object_mut(), extra.as_object()) {
        for (k, x) in e {
            o.insert(k.clone(), x.clone());
        }
    }
    v
}

// the reader: asks Windows about the short list of keys while the game is in front; ends when the wizard ends
#[cfg(not(target_os = "windows"))]
mod reader {
    pub fn start(_gen: u64) {}
}

#[cfg(target_os = "windows")]
mod reader {
    use super::*;

    // (the virtual key, the name): the keys of a game; nothing else is ever asked
    fn candidates() -> Vec<(i32, String)> {
        let mut v: Vec<(i32, String)> = vec![(0x01, "lmb".into()), (0x02, "rmb".into()), (0x04, "mmb".into()), (0x05, "x1".into()), (0x06, "x2".into()), (0x10, "shift".into()), (0x11, "ctrl".into()), (0x12, "alt".into()), (0x20, "space".into()), (0x14, "caps".into()), (0x09, "tab".into())];
        for c in b'A'..=b'Z' {
            v.push((c as i32, (c as char).to_ascii_lowercase().to_string()));
        }
        for c in b'0'..=b'9' {
            v.push((c as i32, (c as char).to_string()));
        }
        for n in 1..=12 {
            v.push((0x70 + n - 1, format!("f{}", n)));
        }
        v
    }

    pub fn start(gen: u64) {
        std::thread::spawn(move || {
            let keys = candidates();
            let mut down: HashMap<i32, Instant> = HashMap::new();
            let mut last_front = Instant::now() - Duration::from_secs(1);
            loop {
                std::thread::sleep(Duration::from_millis(8));
                if GEN.load(Ordering::SeqCst) != gen {
                    return;
                }
                let exe = match SESSION.lock().as_mut() {
                    Some(s) if matches!(s.phase, Phase::Observing | Phase::Verifying { .. }) => {
                        if s.expired() {
                            s.end(Phase::TimedOut);
                            return;
                        }
                        s.exe.clone()
                    }
                    _ => return,
                };
                // the window in front: only the game that was asked about counts; a press in any other window is never seen
                if last_front.elapsed() >= Duration::from_millis(60) {
                    last_front = Instant::now();
                    let front = crate::input::front_info().map(|(e, _)| e.to_lowercase()).unwrap_or_default();
                    let in_game = front == exe;
                    set_in_game(in_game);
                    if !in_game {
                        down.clear();
                    }
                }
                if !SESSION.lock().as_ref().map(|s| s.in_game).unwrap_or(false) {
                    continue;
                }
                for (vk, name) in &keys {
                    let pressed = unsafe { (winapi::um::winuser::GetAsyncKeyState(*vk) as u16) & 0x8000 != 0 };
                    match (pressed, down.contains_key(vk)) {
                        (true, false) => {
                            down.insert(*vk, Instant::now());
                        }
                        (false, true) => {
                            let t = down.remove(vk).map(|t| t.elapsed().as_millis() as u64).unwrap_or(0);
                            feed(name, t);
                        }
                        _ => {}
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::aimassist::TEST_SETTINGS_LOCK as LOCK;

    fn temp_file() {
        // (aimassist uses a file per test process: every test starts from an empty one)
        let _ = std::fs::remove_file(crate::aimassist::test_settings_file());
    }

    fn hold(s: &mut Session, key: &str, n: u32) {
        for _ in 0..n {
            s.observe(key, 300);
        }
    }

    #[test]
    fn the_wizard_starts_in_the_state_of_looking_and_counts_nothing_until_the_game_is_in_front() {
        let mut s = Session::new("game.exe");
        assert_eq!(s.phase, Phase::Observing);
        assert_eq!(s.progress(), 0);
        // the window of the game is not in front: holds are not counted
        hold(&mut s, "rmb", 10);
        assert_eq!((s.progress(), s.phase.clone()), (0, Phase::Observing));
        s.in_game = true;
        s.observe("rmb", 300);
        assert_eq!(s.progress(), 1);
    }

    #[test]
    fn the_same_key_held_several_times_makes_one_proposal_and_a_tap_does_not_count() {
        let mut s = Session::new("game.exe");
        s.in_game = true;
        hold(&mut s, "rmb", NEEDED - 1);
        assert_eq!(s.phase, Phase::Observing, "not enough yet");
        s.observe("rmb", 10);
        assert_eq!(s.phase, Phase::Observing, "a tap of 10 ms is not a hold");
        s.observe("rmb", 400);
        assert_eq!(s.phase, Phase::Suggested("rmb".into()));
    }

    #[test]
    fn different_keys_never_make_a_proposal_by_chance() {
        let mut s = Session::new("game.exe");
        s.in_game = true;
        // four different keys, once each: nothing
        for k in ["rmb", "q", "e", "x"] {
            s.observe(k, 300);
        }
        assert_eq!(s.phase, Phase::Observing);
        // two keys used about equally: the user is asked
        let mut t = Session::new("game.exe");
        t.in_game = true;
        for _ in 0..4 {
            t.observe("rmb", 300);
            t.observe("q", 300);
        }
        match &t.phase {
            Phase::Ambiguous(list) => assert!(list.iter().any(|x| x.0 == "rmb") && list.iter().any(|x| x.0 == "q")),
            other => panic!("expected a question, got {:?}", other),
        }
        // a stray key held once or twice does not spoil a clear key
        let mut u = Session::new("game.exe");
        u.in_game = true;
        u.observe("w", 300);
        hold(&mut u, "rmb", 4);
        assert_eq!(u.phase, Phase::Suggested("rmb".into()));
        // a key that is not a key of a game is not counted at all
        let mut v = Session::new("game.exe");
        v.in_game = true;
        hold(&mut v, "enter!", 9);
        assert_eq!(v.progress(), 0);
    }

    #[test]
    fn the_user_confirms_and_the_key_is_kept_for_that_game_only_and_a_second_game_has_its_own() {
        let _g = LOCK.lock();
        temp_file();
        cancel();
        assert_eq!(crate::aimassist::game_binding("a.exe").status, crate::aimassist::BindStatus::Unknown, "a new game has no key");
        start("a.exe").unwrap();
        assert!(is_active());
        set_in_game(true);
        for _ in 0..NEEDED {
            feed("q", 300);
        }
        assert!(matches!(status_json("a.exe")["state"].as_str(), Some("suggested")));
        // a proposal is not a saved key
        assert_eq!(crate::aimassist::game_binding("a.exe").status, crate::aimassist::BindStatus::Unknown);
        assert_eq!(confirm("a.exe", None, "hold").unwrap(), "q");
        assert!(!is_active(), "the wizard stopped after the confirmation");
        let a = crate::aimassist::game_binding("A.EXE");
        assert_eq!((a.key.as_str(), a.status, a.how.as_str()), ("q", crate::aimassist::BindStatus::Confirmed, "wizard"));
        // the second game: its own wizard, its own key; the first is not touched
        start("b.exe").unwrap();
        set_in_game(true);
        for _ in 0..NEEDED {
            feed("rmb", 300);
        }
        assert_eq!(confirm("b.exe", None, "hold").unwrap(), "rmb");
        assert_eq!(crate::aimassist::game_binding("a.exe").key, "q");
        assert_eq!(crate::aimassist::game_binding("b.exe").key, "rmb");
        // a manual key wins over a proposal and changes this game only
        start("a.exe").unwrap();
        set_in_game(true);
        for _ in 0..NEEDED {
            feed("e", 300);
        }
        manual("a.exe", "x1", "toggle").unwrap();
        let a = crate::aimassist::game_binding("a.exe");
        assert_eq!((a.key.as_str(), a.mode.as_str(), a.how.as_str()), ("x1", "toggle", "manual"));
        assert_eq!(crate::aimassist::game_binding("b.exe").key, "rmb");
        // a reset forgets one game and leaves the other
        reset("a.exe").unwrap();
        assert_eq!(crate::aimassist::game_binding("a.exe").status, crate::aimassist::BindStatus::Unknown);
        assert_eq!(crate::aimassist::game_binding("b.exe").key, "rmb");
        temp_file();
    }

    #[test]
    fn a_cancel_saves_nothing_changes_nothing_and_stops_looking() {
        let _g = LOCK.lock();
        temp_file();
        cancel();
        crate::aimassist::save_game_binding("a.exe", "rmb", crate::aimassist::BindStatus::Confirmed, "manual", "hold").unwrap();
        let before = crate::aimassist::game_binding("a.exe");
        start("a.exe").unwrap();
        set_in_game(true);
        for _ in 0..NEEDED {
            feed("q", 300);
        }
        cancel();
        assert!(!is_active());
        assert_eq!(status_json("a.exe")["state"], "cancelled");
        assert_eq!(crate::aimassist::game_binding("a.exe"), before, "the saved key is as it was");
        // after the stop the wizard counts nothing
        feed("q", 300);
        assert_eq!(status_json("a.exe")["state"], "cancelled");
        // a confirmation with no proposal is refused
        assert!(confirm("a.exe", None, "hold").is_err());
        temp_file();
    }

    #[test]
    fn the_check_of_a_saved_key_sees_that_key_and_only_in_the_game() {
        let _g = LOCK.lock();
        temp_file();
        cancel();
        assert!(verify("a.exe").is_err(), "no key, nothing to check");
        crate::aimassist::save_game_binding("a.exe", "rmb", crate::aimassist::BindStatus::Confirmed, "manual", "hold").unwrap();
        verify("a.exe").unwrap();
        feed("rmb", 300); // the game is not in front
        assert_eq!(status_json("a.exe")["seen"], false);
        set_in_game(true);
        feed("q", 300);
        assert_eq!(status_json("a.exe")["seen"], false, "another key");
        feed("rmb", 300);
        assert_eq!(status_json("a.exe")["seen"], true);
        cancel();
        temp_file();
    }
}
