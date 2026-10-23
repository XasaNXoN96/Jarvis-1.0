//! The mouse and the keyboard of this computer, driven from the linked phone («Jarvis Remote», item 40.4 of the plan).
//!
//! What protects it (each point is a test below):
//! - the ability «Разрешить телефону управлять мышью и клавиатурой» is OFF by default and is switched on only by the owner in the permission
//!   centre; it is asked on EVERY action, so switching it off stops the phone at once;
//! - nothing happens without a session started on purpose by the phone (`start`); a session ends by itself after a silence or after the longest
//!   time, and by the «Выключить» button of the sign that is always shown on this computer while a session runs;
//! - only a short list of actions: a tap (click, double click, right click) at a place of the main screen, the wheel, a short list of keys and
//!   shortcuts (the rules of policy.rs apply too), and text up to 200 characters. No dragging, no raw key codes, no arbitrary combinations;
//! - the text that is typed is never written to the journal (the journal has the start, the stop, the number of actions and the refusals);
//! - Windows does not take input from Jarvis on the sign-in screen and in UAC windows: that is said as it is, nothing is bypassed.

use crate::live::{End, Live};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

pub const IDLE: Duration = Duration::from_secs(60);
pub const MAX: Duration = Duration::from_secs(10 * 60);
const MIN_GAP: Duration = Duration::from_millis(80);
pub const MAX_TEXT: usize = 200;

static LIVE: Lazy<Mutex<Live>> = Lazy::new(|| Mutex::new(Live::new(IDLE, MAX)));
static ACTIONS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static LAST_ACT: Lazy<Mutex<Option<Instant>>> = Lazy::new(|| Mutex::new(None));

// the keys the phone may press (single names), and the shortcuts it may press
const KEYS: [&str; 16] = ["enter", "escape", "esc", "tab", "space", "backspace", "delete", "up", "down", "left", "right", "home", "end", "page_up", "page_down", "f5"];
const COMBOS: [&str; 11] = ["ctrl+c", "ctrl+v", "ctrl+x", "ctrl+a", "ctrl+z", "ctrl+y", "ctrl+f", "ctrl+s", "alt+tab", "alt+left", "alt+right"];

#[derive(Debug, Clone, PartialEq)]
pub enum Act {
    // a tap at a place of the main screen (shares of width and height, 0..1)
    Click { x: f64, y: f64, right: bool, times: u32 },
    // lines of the wheel: positive = up
    Scroll(i32),
    Key(String),
    Text(String),
}

// What does the work. The program uses the real hands (input.rs); a test puts its own recorder here.
pub trait Hands: Send + Sync {
    fn click(&self, x: f64, y: f64, right: bool, times: u32) -> Result<(), String>;
    fn scroll(&self, lines: i32) -> Result<(), String>;
    fn key(&self, combo: &str) -> Result<(), String>;
    fn text(&self, text: &str) -> Result<(), String>;
}

struct SystemHands;
impl Hands for SystemHands {
    fn click(&self, x: f64, y: f64, right: bool, times: u32) -> Result<(), String> {
        crate::input::remote_click(x, y, right, times)
    }
    fn scroll(&self, lines: i32) -> Result<(), String> {
        crate::input::remote_scroll(lines)
    }
    fn key(&self, combo: &str) -> Result<(), String> {
        crate::input::remote_keys(combo)
    }
    fn text(&self, text: &str) -> Result<(), String> {
        crate::input::remote_text(text)
    }
}

static HANDS: Lazy<Mutex<Option<Box<dyn Hands>>>> = Lazy::new(|| Mutex::new(None));

#[cfg(test)]
pub(crate) fn set_hands_for_tests(h: Option<Box<dyn Hands>>) {
    *HANDS.lock() = h;
}

fn with_hands<R>(f: impl FnOnce(&dyn Hands) -> R) -> R {
    match HANDS.lock().as_ref() {
        Some(h) => f(h.as_ref()),
        None => f(&SystemHands),
    }
}

// what the phone sent → one checked action (nothing is run here)
pub fn parse(body: &Value) -> Result<Act, String> {
    let kind = body.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let num = |k: &str| body.get(k).and_then(|v| v.as_f64()).filter(|v| v.is_finite());
    match kind {
        "click" | "double" | "right" => {
            let (Some(x), Some(y)) = (num("x"), num("y")) else { return Err("нужны x и y — место на экране".into()) };
            if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
                return Err("место на экране вне экрана".into());
            }
            Ok(Act::Click { x, y, right: kind == "right", times: if kind == "double" { 2 } else { 1 } })
        }
        "scroll" => {
            let lines = num("lines").ok_or("нужно число строк")?;
            if lines.fract() != 0.0 || lines == 0.0 || lines.abs() > 20.0 {
                return Err("прокрутка: от 1 до 20 строк вверх или вниз".into());
            }
            Ok(Act::Scroll(lines as i32))
        }
        "key" => {
            let k = body.get("key").and_then(|v| v.as_str()).unwrap_or("").trim().to_lowercase();
            if KEYS.contains(&k.as_str()) || COMBOS.contains(&k.as_str()) {
                Ok(Act::Key(k))
            } else {
                Err(format!("клавиша или сочетание «{}» не разрешены для телефона", k.chars().take(30).collect::<String>()))
            }
        }
        "text" => {
            let t = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
            if t.is_empty() {
                return Err("нечего печатать".into());
            }
            if t.chars().count() > MAX_TEXT {
                return Err(format!("текст длиннее {} символов", MAX_TEXT));
            }
            if t.chars().any(|c| c.is_control()) {
                return Err("в тексте не должно быть управляющих символов (для Enter и Tab есть клавиши)".into());
            }
            Ok(Act::Text(t.to_string()))
        }
        other => Err(format!("неизвестное действие «{}»", other.chars().take(20).collect::<String>())),
    }
}

fn allowed() -> Result<(), String> {
    crate::perms::check(crate::perms::Cap::RemoteInput)
}

// the phone asks to start a session
pub fn start() -> Result<Value, String> {
    allowed().map_err(|e| {
        crate::audit::record_rate_limited("remote-input-denied", 60, "remote", "input_start", "refused", "управление с телефона выключено владельцем");
        e
    })?;
    let fresh = LIVE.lock().start(Instant::now());
    if fresh {
        ACTIONS.store(0, std::sync::atomic::Ordering::SeqCst);
        crate::audit::record_door("phone", "remote", "input_start", "medium", "allowed", true, "телефон начал управлять мышью и клавиатурой");
        crate::notify::push("important", "Телефон управляет компьютером", "Привязанный телефон начал управлять мышью и клавиатурой. Выключить: кнопка на красной полосе вверху окна Jarvis.", false);
    }
    Ok(json!({ "ok": true, "message": "Управление включено. Оно закончится само через минуту тишины или через 10 минут.", "idle_secs": IDLE.as_secs(), "max_secs": MAX.as_secs() }))
}

pub fn stop(reason: &str) -> bool {
    let was = LIVE.lock().stop();
    if was {
        crate::audit::record_door("phone", "remote", "input_stop", "medium", "allowed", true, &format!("{} (действий: {})", reason, ACTIONS.load(std::sync::atomic::Ordering::SeqCst)));
    }
    was
}

// is a session running? (an expired one is closed here, with a line in the journal)
pub fn live() -> bool {
    let r = LIVE.lock().check(Instant::now());
    match r {
        Ok(()) => {
            if allowed().is_err() {
                stop("разрешение выключено владельцем");
                return false;
            }
            true
        }
        Err(End::Stopped) => false,
        Err(e) => {
            crate::audit::record_door("phone", "remote", "input_stop", "medium", "allowed", true, &format!("{} (действий: {})", e.text(), ACTIONS.load(std::sync::atomic::Ordering::SeqCst)));
            false
        }
    }
}

// one action of the phone
pub fn act(a: &Act) -> Result<(), String> {
    if let Err(e) = allowed() {
        stop("разрешение выключено владельцем");
        return Err(e);
    }
    if let Err(e) = LIVE.lock().touch(Instant::now()) {
        return Err(if e == End::Stopped { "Управление не включено: нажмите «Включить управление» на телефоне.".to_string() } else { format!("Управление выключено: {}. Включите его снова.", e.text()) });
    }
    {
        let mut last = LAST_ACT.lock();
        let now = Instant::now();
        if last.map(|l| now.saturating_duration_since(l) < MIN_GAP).unwrap_or(false) {
            return Err("слишком часто: подождите долю секунды".into());
        }
        *last = Some(now);
    }
    let r = with_hands(|h| match a {
        Act::Click { x, y, right, times } => h.click(*x, *y, *right, *times),
        Act::Scroll(n) => h.scroll(*n),
        Act::Key(k) => h.key(k),
        Act::Text(t) => h.text(t),
    });
    match &r {
        Ok(()) => {
            ACTIONS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
        Err(e) => {
            // (no text of the request in the journal, only the reason and at most one line a minute)
            crate::audit::record_rate_limited("remote-input-failed", 60, "remote", "input", "refused", e);
        }
    }
    r
}

pub fn status_json() -> Value {
    json!({ "allowed": allowed().is_ok(), "live": live(), "seconds": LIVE.lock().seconds(Instant::now()) })
}

// hands that do nothing, for the tests of other modules (a test never moves the real mouse)
#[cfg(test)]
pub(crate) struct NoHands;
#[cfg(test)]
impl Hands for NoHands {
    fn click(&self, _: f64, _: f64, _: bool, _: u32) -> Result<(), String> {
        Ok(())
    }
    fn scroll(&self, _: i32) -> Result<(), String> {
        Ok(())
    }
    fn key(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
    fn text(&self, _: &str) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

#[cfg(test)]
pub(crate) fn reset_for_tests() {
    *LIVE.lock() = Live::new(IDLE, MAX);
    *LAST_ACT.lock() = None;
    ACTIONS.store(0, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    struct Recorder(Arc<Mutex<Vec<String>>>);
    impl Hands for Recorder {
        fn click(&self, x: f64, y: f64, right: bool, times: u32) -> Result<(), String> {
            self.0.lock().push(format!("click {:.2} {:.2} {} {}", x, y, right, times));
            Ok(())
        }
        fn scroll(&self, lines: i32) -> Result<(), String> {
            self.0.lock().push(format!("scroll {}", lines));
            Ok(())
        }
        fn key(&self, combo: &str) -> Result<(), String> {
            self.0.lock().push(format!("key {}", combo));
            Ok(())
        }
        fn text(&self, text: &str) -> Result<(), String> {
            self.0.lock().push(format!("text {}", text.chars().count()));
            Ok(())
        }
    }

    fn setup() -> Arc<Mutex<Vec<String>>> {
        reset_for_tests();
        let log = Arc::new(Mutex::new(Vec::new()));
        set_hands_for_tests(Some(Box::new(Recorder(log.clone()))));
        log
    }

    fn pause() {
        // (the pause between two actions is part of the rules)
        std::thread::sleep(MIN_GAP + Duration::from_millis(20));
    }

    #[test]
    fn what_the_phone_sends_is_checked_before_anything_is_run() {
        assert_eq!(parse(&json!({ "kind": "click", "x": 0.5, "y": 0.25 })), Ok(Act::Click { x: 0.5, y: 0.25, right: false, times: 1 }));
        assert_eq!(parse(&json!({ "kind": "double", "x": 0.0, "y": 1.0 })), Ok(Act::Click { x: 0.0, y: 1.0, right: false, times: 2 }));
        assert_eq!(parse(&json!({ "kind": "right", "x": 0.1, "y": 0.1 })), Ok(Act::Click { x: 0.1, y: 0.1, right: true, times: 1 }));
        for bad in [json!({ "kind": "click", "x": 1.2, "y": 0.5 }), json!({ "kind": "click", "x": -0.1, "y": 0.5 }), json!({ "kind": "click", "x": 0.5 }), json!({ "kind": "click", "x": "0.5", "y": 0.5 })] {
            assert!(parse(&bad).is_err(), "{}", bad);
        }
        assert_eq!(parse(&json!({ "kind": "scroll", "lines": -3 })), Ok(Act::Scroll(-3)));
        for bad in [json!({ "kind": "scroll", "lines": 0 }), json!({ "kind": "scroll", "lines": 21 }), json!({ "kind": "scroll", "lines": 1.5 }), json!({ "kind": "scroll" })] {
            assert!(parse(&bad).is_err(), "{}", bad);
        }
        assert_eq!(parse(&json!({ "kind": "key", "key": " Enter " })), Ok(Act::Key("enter".into())));
        assert_eq!(parse(&json!({ "kind": "key", "key": "CTRL+C" })), Ok(Act::Key("ctrl+c".into())));
        for bad in ["win+r", "ctrl+alt+delete", "alt+f4", "ctrl+shift+esc", "win", "f4", "ctrl+w", "a", ""] {
            assert!(parse(&json!({ "kind": "key", "key": bad })).is_err(), "{}", bad);
        }
        assert_eq!(parse(&json!({ "kind": "text", "text": "привет, мир" })), Ok(Act::Text("привет, мир".into())));
        assert!(parse(&json!({ "kind": "text", "text": "" })).is_err());
        assert!(parse(&json!({ "kind": "text", "text": "a\nb" })).is_err(), "no control characters");
        assert!(parse(&json!({ "kind": "text", "text": "я".repeat(201) })).is_err());
        assert!(parse(&json!({ "kind": "text", "text": "я".repeat(200) })).is_ok());
        assert!(parse(&json!({ "kind": "drag", "x": 0.1, "y": 0.1 })).is_err() && parse(&json!({ "kind": "move" })).is_err() && parse(&json!({})).is_err());
    }

    #[test]
    fn nothing_works_by_default_and_not_without_a_session_started_on_purpose() {
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            let log = setup();
            assert!(!crate::perms::allowed(crate::perms::Cap::RemoteInput), "OFF by default");
            assert!(start().is_err(), "the owner has not allowed it");
            assert!(act(&Act::Key("enter".into())).is_err());
            crate::perms::set(crate::perms::Cap::RemoteInput, true).unwrap();
            // allowed, but no session: still nothing
            let e = act(&Act::Key("enter".into())).unwrap_err();
            assert!(e.contains("не включено"), "{}", e);
            assert!(log.lock().is_empty(), "nothing reached the hands");
            assert!(start().is_ok());
            pause();
            act(&Act::Key("enter".into())).unwrap();
            assert_eq!(*log.lock(), vec!["key enter".to_string()]);
            set_hands_for_tests(None);
        });
    }

    #[test]
    fn the_owner_switching_the_ability_off_stops_a_running_session_at_once() {
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            let log = setup();
            crate::perms::set(crate::perms::Cap::RemoteInput, true).unwrap();
            start().unwrap();
            assert!(live());
            pause();
            act(&Act::Click { x: 0.5, y: 0.5, right: false, times: 1 }).unwrap();
            crate::perms::set(crate::perms::Cap::RemoteInput, false).unwrap();
            assert!(!live(), "the sign on the screen goes out");
            pause();
            assert!(act(&Act::Click { x: 0.5, y: 0.5, right: false, times: 1 }).is_err());
            assert_eq!(log.lock().len(), 1, "the second click never happened");
            // switching it on again does not bring the old session back
            crate::perms::set(crate::perms::Cap::RemoteInput, true).unwrap();
            pause();
            assert!(act(&Act::Scroll(1)).is_err());
            set_hands_for_tests(None);
        });
    }

    #[test]
    fn stop_ends_the_session_and_actions_come_not_too_often() {
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            let log = setup();
            crate::perms::set(crate::perms::Cap::RemoteInput, true).unwrap();
            start().unwrap();
            pause();
            act(&Act::Scroll(-2)).unwrap();
            assert!(act(&Act::Scroll(-2)).unwrap_err().contains("слишком часто"), "two actions in one breath");
            pause();
            act(&Act::Text("привет".into())).unwrap();
            assert_eq!(*log.lock(), vec!["scroll -2".to_string(), "text 6".to_string()]);
            assert!(stop("кнопка «Выключить» на компьютере"));
            assert!(!stop("ещё раз"), "nothing was running");
            pause();
            assert!(act(&Act::Scroll(1)).is_err());
            // the journal has the start and the stop, and no typed text
            let j = serde_json::to_string(&crate::audit::json_filtered("", "", "", "", 0, 0)).unwrap();
            assert!(j.contains("input_start") && j.contains("input_stop"), "{}", j);
            assert!(!j.contains("привет"), "the typed text is not in the journal");
            set_hands_for_tests(None);
        });
    }

    #[test]
    fn a_refusal_of_windows_is_told_as_it_is() {
        struct Blocked;
        impl Hands for Blocked {
            fn click(&self, _: f64, _: f64, _: bool, _: u32) -> Result<(), String> { Err("Windows не принял ввод".into()) }
            fn scroll(&self, _: i32) -> Result<(), String> { Err("Windows не принял ввод".into()) }
            fn key(&self, _: &str) -> Result<(), String> { Err("Windows не принял ввод".into()) }
            fn text(&self, _: &str) -> Result<(), String> { Err("Windows не принял ввод".into()) }
        }
        let _g = TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            reset_for_tests();
            set_hands_for_tests(Some(Box::new(Blocked)));
            crate::perms::set(crate::perms::Cap::RemoteInput, true).unwrap();
            start().unwrap();
            pause();
            assert_eq!(act(&Act::Key("enter".into())).unwrap_err(), "Windows не принял ввод");
            assert_eq!(ACTIONS.load(std::sync::atomic::Ordering::SeqCst), 0, "a refused action is not counted");
            set_hands_for_tests(None);
        });
    }
}
