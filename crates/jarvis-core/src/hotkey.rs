//! The global hotkey (Windows): one key combination that works from any program, also when the window of Jarvis is hidden in the tray.
//! A press does what a click on the orb does: Jarvis listens for a command at once, without the wake word. While Jarvis speaks, a press
//! silences him (and an AI answer that is still arriving) instead. The microphone switched off by the user is respected: nothing is
//! listened to then. The combination is chosen on the settings page; it must have Ctrl, Alt or Win (a key with Shift alone would
//! take the letter away from every program), and a combination that another program already owns is reported, not taken.

use once_cell::sync::Lazy;
use parking_lot::Mutex;

pub const DEFAULT: &str = "Ctrl+Alt+J";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Combo {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    // the virtual-key code of the main key
    pub vk: u32,
}

// "ctrl+alt+j", "Ctrl + Shift + F9", "win+space" -> the combination, or why not
pub fn parse(text: &str) -> Result<Combo, String> {
    let (mut ctrl, mut alt, mut shift, mut win) = (false, false, false, false);
    let mut key: Option<u32> = None;
    for part in text.split('+').map(|p| p.trim().to_lowercase()).filter(|p| !p.is_empty()) {
        match part.as_str() {
            "ctrl" | "control" | "ктрл" => ctrl = true,
            "alt" | "альт" => alt = true,
            "shift" | "шифт" => shift = true,
            "win" | "windows" | "super" => win = true,
            "space" | "пробел" => set_key(&mut key, 0x20)?,
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) => set_key(&mut key, k.to_ascii_uppercase().as_bytes()[0] as u32)?,
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => set_key(&mut key, k.as_bytes()[0] as u32)?,
            k if k.starts_with('f') && k[1..].parse::<u32>().map(|n| (1..=12).contains(&n)).unwrap_or(false) => set_key(&mut key, 0x70 + k[1..].parse::<u32>().unwrap() - 1)?,
            _ => return Err(format!("«{}» не подходит: нужны Ctrl, Alt, Win, Shift и одна клавиша (буква, цифра, F1–F12, Space)", part)),
        }
    }
    let Some(vk) = key else { return Err("не выбрана основная клавиша".into()) };
    if !(ctrl || alt || win) {
        return Err("нужен хотя бы один из Ctrl, Alt или Win (одна клавиша или Shift+клавиша мешали бы набору текста)".into());
    }
    Ok(Combo { ctrl, alt, shift, win, vk })
}

fn set_key(slot: &mut Option<u32>, vk: u32) -> Result<(), String> {
    if slot.is_some() {
        return Err("основная клавиша должна быть одна".into());
    }
    *slot = Some(vk);
    Ok(())
}

// the way a combination is shown and kept: "Ctrl+Alt+J"
pub fn describe(c: &Combo) -> String {
    let mut s = String::new();
    for (on, name) in [(c.ctrl, "Ctrl"), (c.alt, "Alt"), (c.shift, "Shift"), (c.win, "Win")] {
        if on {
            s.push_str(name);
            s.push('+');
        }
    }
    s.push_str(&match c.vk {
        0x20 => "Space".to_string(),
        0x70..=0x7B => format!("F{}", c.vk - 0x70 + 1),
        v => (v as u8 as char).to_string(),
    });
    s
}

// "" = the hotkey is off; a text that does not parse is also off (the page checks it before it saves)
pub fn normalize(text: &str) -> Result<String, String> {
    if text.trim().is_empty() {
        return Ok(String::new());
    }
    parse(text).map(|c| describe(&c))
}

// what a press does (pure on the inputs, so that it is tested): the answer is for the log
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Press {
    // the microphone is off by the user: nothing is listened to
    Ignored,
    // Jarvis was speaking: silenced
    Silenced,
    // listens for a command now
    Listen,
}

pub fn decide(mic_muted: bool, speaking: bool) -> Press {
    if speaking {
        // silencing is allowed also with the microphone off: it only makes Jarvis quiet
        Press::Silenced
    } else if mic_muted {
        Press::Ignored
    } else {
        Press::Listen
    }
}

pub fn on_press() -> Press {
    let p = decide(crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst), crate::tts::is_speaking());
    match p {
        Press::Silenced => {
            if let Some(id) = crate::ai::store().active() {
                crate::ai::stop(&id);
            }
            crate::tts::stop();
        }
        Press::Listen => crate::LISTEN_NOW.store(true, std::sync::atomic::Ordering::SeqCst),
        Press::Ignored => {}
    }
    p
}

// ---------------------------------------------------------------------------------------------
// the system part
// ---------------------------------------------------------------------------------------------
// "off" | "ok" | "error: why" | "unsupported"
static STATUS: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new("off".into()));
static WANTED: Lazy<Mutex<Option<Combo>>> = Lazy::new(|| Mutex::new(None));

pub fn status() -> String {
    STATUS.lock().clone()
}

// the combination that is asked for ("" = none); the thread registers it and writes the result into `status()`
pub fn apply(text: &str) {
    *WANTED.lock() = parse(text).ok();
    if WANTED.lock().is_none() {
        *STATUS.lock() = "off".into();
    }
    #[cfg(target_os = "windows")]
    sys::poke();
    #[cfg(not(target_os = "windows"))]
    {
        if WANTED.lock().is_some() {
            *STATUS.lock() = "unsupported".into();
        }
    }
}

#[cfg(target_os = "windows")]
mod sys {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use winapi::um::processthreadsapi::GetCurrentThreadId;
    use winapi::um::winuser::*;

    static TID: AtomicU32 = AtomicU32::new(0);
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    const ID: i32 = 0x4A52; // «JR»
    const WM_REAPPLY: u32 = WM_APP + 1;

    pub fn poke() {
        if !STARTED.swap(true, Ordering::SeqCst) {
            std::thread::spawn(run);
            return;
        }
        let tid = TID.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe { PostThreadMessageW(tid, WM_REAPPLY, 0, 0) };
        }
    }

    fn register() {
        unsafe { UnregisterHotKey(std::ptr::null_mut(), ID) };
        let Some(c) = *WANTED.lock() else { return };
        let mods = (if c.alt { MOD_ALT } else { 0 }) | (if c.ctrl { MOD_CONTROL } else { 0 }) | (if c.shift { MOD_SHIFT } else { 0 }) | (if c.win { MOD_WIN } else { 0 }) | MOD_NOREPEAT;
        let ok = unsafe { RegisterHotKey(std::ptr::null_mut(), ID, mods as u32, c.vk) } != 0;
        *STATUS.lock() = if ok { "ok".into() } else { format!("error: сочетание {} уже занято другой программой или Windows — выберите другое", describe(&c)) };
    }

    fn run() {
        unsafe {
            let mut msg: MSG = std::mem::zeroed();
            // the queue of this thread exists from here on (messages posted to it are kept)
            PeekMessageW(&mut msg, std::ptr::null_mut(), WM_USER, WM_USER, PM_NOREMOVE);
            TID.store(GetCurrentThreadId(), Ordering::SeqCst);
            register();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                match msg.message {
                    WM_HOTKEY if msg.wParam as i32 == ID => {
                        let p = on_press();
                        log::info!("Hotkey pressed: {:?}", p);
                    }
                    WM_REAPPLY => register(),
                    _ => {}
                }
            }
            UnregisterHotKey(std::ptr::null_mut(), ID);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combinations_are_read_in_any_spelling_and_shown_in_one() {
        let c = parse("ctrl+alt+j").unwrap();
        assert_eq!((c.ctrl, c.alt, c.shift, c.win, c.vk), (true, true, false, false, b'J' as u32));
        assert_eq!(describe(&c), "Ctrl+Alt+J");
        assert_eq!(normalize(" Control + Shift + f9 ").unwrap(), "Ctrl+Shift+F9");
        assert_eq!(normalize("win+space").unwrap(), "Win+Space");
        assert_eq!(normalize("Ctrl+Alt+5").unwrap(), "Ctrl+Alt+5");
        assert_eq!(normalize("").unwrap(), "", "empty = the hotkey is off");
        assert_eq!(parse(DEFAULT).unwrap(), c);
    }

    #[test]
    fn a_combination_that_would_take_a_letter_from_every_program_or_is_nonsense_is_refused() {
        for bad in ["j", "shift+j", "ctrl+alt", "ctrl+j+k", "ctrl+f13", "ctrl+ё", "ctrl+enter", "alt+tab+x+"] {
            assert!(parse(bad).is_err(), "{}", bad);
        }
        assert!(normalize("ctrl+").is_err());
    }

    #[test]
    fn a_press_silences_speech_listens_or_respects_the_muted_microphone() {
        assert_eq!(decide(false, false), Press::Listen);
        assert_eq!(decide(false, true), Press::Silenced);
        assert_eq!(decide(true, true), Press::Silenced, "making Jarvis quiet is allowed with the microphone off");
        assert_eq!(decide(true, false), Press::Ignored, "the microphone is off: nothing is listened to");
    }

    #[test]
    fn applying_a_bad_or_empty_combination_leaves_the_hotkey_off() {
        apply("");
        assert_eq!(status(), "off");
        apply("j");
        assert_eq!(status(), "off");
    }
}
