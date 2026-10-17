//! Voice macros of the game mode: «запиши макрос комбо» … play … «сохрани макрос», and later the phrase «комбо» does the same.
//!
//! While a recording runs and the window of a game of the list is in front, the program reads the state of the plain game keys (letters,
//! digits, space, shift, ctrl, tab) and of the two mouse buttons — nothing else, no text, no other key — and turns it into steps
//! (`tap:w`, `hold:shift`, `wait:300`, `click:left` …, the same steps as in game_profile.json). A macro is kept in game_macros.json.
//! Mouse MOVEMENT is not recorded (a game keeps taking the cursor back to the centre); use «move:dx,dy» in a phrase of game_profile.json.

use crate::gamemode::{allowed_key, parse_step, Step};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

pub const MAX_MACROS: usize = 60;
pub const MAX_STEPS: usize = 30;
const SHORT_PRESS_MS: u64 = 150;
const MAX_RECORD_MS: u64 = 60_000;

// what the recorder saw: a key or a mouse button went down or up, at this time (milliseconds from the start)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ev {
    Key(u16, bool),
    Button(bool, bool),
}

// the name of a key for the file («w», «space») — the opposite of input::vk for the allowed keys
pub fn key_name(vk: u16) -> Option<String> {
    Some(match vk {
        0x20 => "space".into(),
        0x10 => "shift".into(),
        0x11 => "ctrl".into(),
        0x09 => "tab".into(),
        0x30..=0x39 | 0x41..=0x5A if allowed_key(vk) => (vk as u8 as char).to_ascii_lowercase().to_string(),
        _ => return None,
    })
}

pub fn step_text(s: &Step) -> Option<String> {
    Some(match s {
        Step::Tap(k) => format!("tap:{}", key_name(*k)?),
        Step::Hold(k) => format!("hold:{}", key_name(*k)?),
        Step::Release(k) => format!("release:{}", key_name(*k)?),
        Step::Click(right) => format!("click:{}", if *right { "right" } else { "left" }),
        Step::Wait(ms) => format!("wait:{}", ms),
        Step::Move(dx, dy) => format!("move:{},{}", dx, dy),
    })
}

// the events of a recording -> steps: a short press is a tap, a long one is a hold with its release, a button is a click; the pauses
// between them are waits. More than MAX_STEPS steps are cut (the second value says so). A key still held at the end is released.
pub fn events_to_steps(events: &[(u64, Ev)], end_ms: u64) -> (Vec<Step>, bool) {
    // (time, order, step)
    let mut actions: Vec<(u64, Step)> = Vec::new();
    let mut down_at: std::collections::BTreeMap<u16, u64> = Default::default();
    let mut btn_down: [Option<u64>; 2] = [None, None];
    for (t, ev) in events {
        match *ev {
            Ev::Key(vk, true) => {
                down_at.entry(vk).or_insert(*t);
            }
            Ev::Key(vk, false) => {
                if let Some(d) = down_at.remove(&vk) {
                    if t - d < SHORT_PRESS_MS {
                        actions.push((d, Step::Tap(vk)));
                    } else {
                        actions.push((d, Step::Hold(vk)));
                        actions.push((*t, Step::Release(vk)));
                    }
                }
            }
            Ev::Button(right, true) => btn_down[right as usize] = btn_down[right as usize].or(Some(*t)),
            Ev::Button(right, false) => {
                if let Some(d) = btn_down[right as usize].take() {
                    actions.push((d, Step::Click(right)));
                }
            }
        }
    }
    // keys held when the recording ended
    for (vk, d) in down_at {
        actions.push((d, Step::Hold(vk)));
        actions.push((end_ms.max(d), Step::Release(vk)));
    }
    actions.sort_by_key(|(t, _)| *t);
    let mut steps = Vec::new();
    let mut last = actions.first().map(|(t, _)| *t).unwrap_or(0);
    let mut cut = false;
    for (t, a) in actions {
        let gap = t.saturating_sub(last);
        let need = if gap >= 30 { 2 } else { 1 };
        if steps.len() + need > MAX_STEPS {
            cut = true;
            break;
        }
        if gap >= 30 {
            steps.push(Step::Wait(gap.min(2000)));
        }
        steps.push(a);
        last = t;
    }
    // a hold that was cut off would stay pressed: it is released at the end of the macro
    let mut held: Vec<u16> = Vec::new();
    for s in &steps {
        match s {
            Step::Hold(k) if !held.contains(k) => held.push(*k),
            Step::Release(k) => held.retain(|h| h != k),
            _ => {}
        }
    }
    for k in held {
        if steps.len() < MAX_STEPS {
            steps.push(Step::Release(k));
        } else {
            steps.pop();
            steps.push(Step::Release(k));
        }
    }
    (steps, cut)
}

// ---------------------------------------------------------------------------------------------
// the store: game_macros.json
// ---------------------------------------------------------------------------------------------
#[cfg(test)]
thread_local! {
    static TEST_STORE: std::cell::RefCell<Vec<(String, Vec<Step>)>> = std::cell::RefCell::new(Vec::new());
}

static WRITE: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub fn parse_store(text: &str) -> Vec<(String, Vec<Step>)> {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Vec::new() };
    let mut out = Vec::new();
    for m in v.get("macros").and_then(|m| m.as_array()).into_iter().flatten().take(MAX_MACROS) {
        let name = m.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let steps: Option<Vec<Step>> = m.get("steps").and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).map(parse_step).collect::<Result<Vec<_>, _>>().ok()).flatten();
        if let (false, Some(steps)) = (name.is_empty(), steps) {
            if !steps.is_empty() && steps.len() <= MAX_STEPS && !out.iter().any(|(n, _): &(String, Vec<Step>)| *n == name) {
                out.push((name, steps));
            }
        }
    }
    out
}

pub fn store_json(list: &[(String, Vec<Step>)]) -> String {
    let macros: Vec<Value> = list.iter().map(|(n, s)| json!({ "name": n, "steps": s.iter().filter_map(step_text).collect::<Vec<_>>() })).collect();
    serde_json::to_string_pretty(&json!({ "macros": macros })).unwrap_or_default()
}

pub fn load() -> Vec<(String, Vec<Step>)> {
    #[cfg(test)]
    {
        return TEST_STORE.with(|t| t.borrow().clone());
    }
    #[cfg(not(test))]
    {
        std::fs::read_to_string(crate::APP_DIR.join("game_macros.json")).map(|t| parse_store(&t)).unwrap_or_default()
    }
}

fn save(list: &[(String, Vec<Step>)]) {
    #[cfg(test)]
    {
        TEST_STORE.with(|t| *t.borrow_mut() = list.to_vec());
    }
    #[cfg(not(test))]
    {
        let path = crate::APP_DIR.join("game_macros.json");
        if let Ok(old) = std::fs::read_to_string(&path) {
            crate::keep_copy_if_repaired(&path, &old, &store_json(&parse_store(&old)));
        }
        let _ = crate::write_atomic(&path, store_json(list));
    }
}

// the name of a macro as it is spoken: checked like a phrase of the profile (no words of stopping or switching off)
pub fn clean_name(raw: &str) -> Result<String, String> {
    crate::gamemode::valid_phrase_words(raw)
}

pub fn put(name: &str, steps: Vec<Step>) -> Result<(), String> {
    let name = clean_name(name)?;
    if steps.is_empty() {
        return Err("в записи не было ни одного нажатия".into());
    }
    let _g = WRITE.lock();
    let mut list = load();
    if let Some(m) = list.iter_mut().find(|(n, _)| *n == name) {
        m.1 = steps;
    } else if list.len() >= MAX_MACROS {
        return Err(format!("уже {} макросов: удалите ненужные («удали макрос …»)", MAX_MACROS));
    } else {
        list.push((name, steps));
    }
    save(&list);
    Ok(())
}

pub fn delete(name: &str) -> bool {
    let Ok(name) = clean_name(name) else { return false };
    let _g = WRITE.lock();
    let mut list = load();
    let before = list.len();
    list.retain(|(n, _)| *n != name);
    let changed = list.len() != before;
    if changed {
        save(&list);
    }
    changed
}

// ---------------------------------------------------------------------------------------------
// the recording
// ---------------------------------------------------------------------------------------------
struct Recording {
    name: String,
    started: std::time::Instant,
    events: Vec<(u64, Ev)>,
}

static REC: Lazy<Mutex<Option<Recording>>> = Lazy::new(|| Mutex::new(None));

pub fn is_recording() -> bool {
    REC.lock().is_some()
}

pub fn recording_name() -> Option<String> {
    REC.lock().as_ref().map(|r| r.name.clone())
}

// the recorder reports a change of a key or a button (the reader below, and the tests)
pub fn note(ev: Ev) {
    let mut g = REC.lock();
    if let Some(r) = g.as_mut() {
        let t = r.started.elapsed().as_millis() as u64;
        if t <= MAX_RECORD_MS && r.events.len() < 400 {
            r.events.push((t, ev));
        }
    }
}

pub fn start(name: &str) -> Result<String, String> {
    let name = clean_name(name).map_err(|e| format!("Не могу записать: {}.", e))?;
    let mut g = REC.lock();
    if g.is_some() {
        return Err("Запись уже идёт: скажите «сохрани макрос».".into());
    }
    *g = Some(Recording { name: name.clone(), started: std::time::Instant::now(), events: Vec::new() });
    drop(g);
    #[cfg(target_os = "windows")]
    reader::start();
    Ok(format!("Записываю макрос «{}»: делайте нажатия в игре, потом скажите «сохрани макрос» (запись остановится сама через минуту).", name))
}

// throws the recording away (the mode was switched off, an anti-cheat program started): nothing is kept
pub fn cancel() {
    if REC.lock().take().is_some() {
        log::info!("Game macro: the recording was cancelled");
    }
}

// stops and keeps the macro; the answer for the user
pub fn stop() -> Result<String, String> {
    let Some(r) = REC.lock().take() else { return Err("Запись не идёт.".into()) };
    let end = r.started.elapsed().as_millis() as u64;
    let (steps, cut) = events_to_steps(&r.events, end.min(MAX_RECORD_MS));
    let n = steps.len();
    put(&r.name, steps)?;
    Ok(format!("Макрос «{}» сохранён ({} шагов){}. Скажите «{}», и он повторится.", r.name, n, if cut { ", длинная запись обрезана до 30 шагов" } else { "" }, r.name))
}

#[cfg(target_os = "windows")]
mod reader {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    static RUNNING: AtomicBool = AtomicBool::new(false);

    // the plain game keys the recorder looks at: letters, digits, space, shift, ctrl, tab
    fn keys() -> Vec<u16> {
        (0x30u16..=0x39).chain(0x41..=0x5A).chain([0x20, 0x10, 0x11, 0x09]).filter(|k| allowed_key(*k)).collect()
    }

    pub fn start() {
        if RUNNING.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| {
            use winapi::um::winuser::GetAsyncKeyState;
            let list = keys();
            let mut down: std::collections::HashSet<u16> = Default::default();
            let mut buttons = [false, false];
            while is_recording() {
                // the mode is off or an anti-cheat program runs: nothing is looked at, the recording is thrown away
                if !crate::gamemode::is_active() || crate::gamemode::anticheat_running().is_some() {
                    cancel();
                    break;
                }
                // only while a game of the list is in front: what is done in other windows is not looked at
                if crate::gamemode::in_game_now() {
                    for &k in &list {
                        let now = unsafe { (GetAsyncKeyState(k as i32) as u16) & 0x8000 != 0 };
                        if now != down.contains(&k) {
                            if now {
                                down.insert(k);
                            } else {
                                down.remove(&k);
                            }
                            note(Ev::Key(k, now));
                        }
                    }
                    for (i, vk) in [0x01i32, 0x02].iter().enumerate() {
                        let now = unsafe { (GetAsyncKeyState(*vk) as u16) & 0x8000 != 0 };
                        if now != buttons[i] {
                            buttons[i] = now;
                            note(Ev::Button(i == 1, now));
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(8));
                // the end of the time: the recording stops by itself and is kept
                let over = REC.lock().as_ref().map(|r| r.started.elapsed().as_millis() as u64 > MAX_RECORD_MS).unwrap_or(false);
                if over {
                    let _ = stop();
                }
            }
            RUNNING.store(false, Ordering::SeqCst);
        });
    }
}

#[cfg(test)]
pub(crate) static REC_TESTS: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(list: &[(u64, Ev)]) -> Vec<(u64, Ev)> {
        list.to_vec()
    }

    #[test]
    fn short_presses_are_taps_long_ones_are_holds_and_the_pauses_are_waits() {
        // w down 0..60 (a tap), a pause, shift down 500..1200 (a hold), a click at 1300..1340; the waits are the gaps between the starts of actions
        let ev = rec(&[(0, Ev::Key(0x57, true)), (60, Ev::Key(0x57, false)), (500, Ev::Key(0x10, true)), (1200, Ev::Key(0x10, false)), (1300, Ev::Button(false, true)), (1340, Ev::Button(false, false))]);
        let (steps, cut) = events_to_steps(&ev, 1400);
        assert!(!cut);
        assert_eq!(steps, vec![Step::Tap(0x57), Step::Wait(500), Step::Hold(0x10), Step::Wait(700), Step::Release(0x10), Step::Wait(100), Step::Click(false)]);
    }

    #[test]
    fn a_key_still_held_at_the_end_is_released_and_a_long_record_is_cut_with_its_holds_closed() {
        let (steps, _) = events_to_steps(&rec(&[(0, Ev::Key(0x57, true))]), 900);
        assert_eq!(steps, vec![Step::Hold(0x57), Step::Wait(900), Step::Release(0x57)]);
        // forty taps: cut at the limit, no hold stays open
        let mut ev = Vec::new();
        for i in 0..40u64 {
            ev.push((i * 100, Ev::Key(0x41, true)));
            ev.push((i * 100 + 40, Ev::Key(0x41, false)));
        }
        let (steps, cut) = events_to_steps(&ev, 4000);
        assert!(cut && steps.len() <= MAX_STEPS, "{}", steps.len());
        // a hold in the cut part
        let (steps, _) = events_to_steps(&rec(&[(0, Ev::Key(0x57, true)), (10, Ev::Key(0x41, true)), (20, Ev::Key(0x41, false))]), 5000);
        assert!(matches!(steps.last(), Some(Step::Release(0x57))), "{:?}", steps);
        assert_eq!(events_to_steps(&[], 100), (Vec::new(), false));
    }

    #[test]
    fn steps_are_written_as_text_and_read_back_the_same() {
        let steps = vec![Step::Tap(0x57), Step::Hold(0x10), Step::Wait(300), Step::Release(0x10), Step::Click(true), Step::Tap(0x20), Step::Tap(0x31)];
        let text = store_json(&[("комбо".to_string(), steps.clone())]);
        assert!(text.contains("tap:w") && text.contains("hold:shift") && text.contains("click:right") && text.contains("tap:space") && text.contains("tap:1"), "{}", text);
        assert_eq!(parse_store(&text), vec![("комбо".to_string(), steps)]);
        // broken entries are dropped, not the whole file
        let mixed = r#"{"macros":[{"name":"ok","steps":["tap:w"]},{"name":"bad","steps":["tap:alt"]},{"name":"","steps":["tap:w"]},{"name":"empty","steps":[]}]}"#;
        assert_eq!(parse_store(mixed).len(), 1);
        assert!(parse_store("не json").is_empty());
        assert_eq!(key_name(0x1B), None);
        assert_eq!(key_name(0x12), None);
    }

    #[test]
    fn a_recording_keeps_a_macro_under_a_spoken_name_and_the_name_cannot_be_a_stop_word() {
        let _l = REC_TESTS.lock();
        TEST_STORE.with(|t| t.borrow_mut().clear());
        assert!(start("Стой").is_err());
        assert!(start("выключи всё").is_err());
        assert!(stop().is_err(), "nothing is recorded");
        assert!(start("Комбо Один!").unwrap().contains("комбо один"));
        assert!(is_recording() && recording_name().as_deref() == Some("комбо один"));
        assert!(start("другое").unwrap_err().contains("уже идёт"));
        note(Ev::Key(0x57, true));
        std::thread::sleep(std::time::Duration::from_millis(60));
        note(Ev::Key(0x57, false));
        let msg = stop().unwrap();
        assert!(msg.contains("сохранён") && msg.contains("комбо один"), "{}", msg);
        assert!(!is_recording());
        let list = load();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "комбо один");
        assert_eq!(list[0].1, vec![Step::Tap(0x57)]);
        // an empty record is not a macro
        start("пустой").unwrap();
        assert!(stop().is_err());
        assert_eq!(load().len(), 1);
        // delete
        assert!(delete("Комбо один"));
        assert!(!delete("комбо один"));
        assert!(load().is_empty());
    }
}
