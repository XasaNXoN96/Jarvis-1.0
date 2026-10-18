//! A small label over the game while the game mode is on («● ИГРОВОЙ РЕЖИМ · W · ПРИЦЕЛ») and a short sound when a phrase of the game was
//! carried out. Both are switched on the Games page (off by default).
//!
//! The label is a tiny window of its own, always on top, that the mouse and the keyboard pass through (it never takes the focus and never
//! catches a click). It is shown only while the mode is on and a game of the list is in front. A game in the «real» full screen mode
//! may hide it: use «окно без рамки» in the game. Windows only; elsewhere the functions do nothing.

use crate::gamemode;
use once_cell::sync::Lazy;
use parking_lot::Mutex;

// the text of the label
pub fn label_text(held: &[String], aim: bool, recording: Option<&str>, heard: Option<(&str, bool)>) -> String {
    let mut s = String::from("● ИГРОВОЙ РЕЖИМ");
    if !held.is_empty() {
        s.push_str(" · ");
        s.push_str(&held.join("+"));
    }
    if aim {
        s.push_str(" · ПРИЦЕЛ");
    }
    if let Some(name) = recording {
        s.push_str(&format!(" · ЗАПИСЬ «{}»", name));
    }
    // what was heard last (for a few seconds): so that the user sees what the microphone made of his words
    if let Some((words, ok)) = heard {
        s.push_str(&format!(" · «{}» {}", words, if ok { "✓" } else { "✗ не понял" }));
    }
    s
}

pub fn current_label() -> String {
    let held: Vec<String> = gamemode::held_names();
    let heard = gamemode::last_heard();
    label_text(&held, crate::aimassist::is_enabled(), crate::gamemacro::recording_name().as_deref(), heard.as_ref().map(|(w, ok)| (w.as_str(), *ok)))
}

// a 40 ms tone (48 kHz, mono, 16 bit) with a soft start and end: a click that says «done»
pub fn confirm_wav() -> Vec<u8> {
    let n = 1920usize;
    let mut pcm = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = i as f32 / 48_000.0;
        let env = (i as f32 / 200.0).min(1.0) * ((n - i) as f32 / 400.0).min(1.0);
        let v = (t * 1200.0 * std::f32::consts::TAU).sin() * 5000.0 * env;
        pcm.extend_from_slice(&(v as i16).to_le_bytes());
    }
    crate::yandex::wav_of(&pcm)
}

#[allow(dead_code)]
static WAV: Lazy<Vec<u8>> = Lazy::new(confirm_wav);

// a short sound after a phrase of the game (if it is switched on)
pub fn confirm() {
    if !crate::gamelist::options().sound {
        return;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use winapi::um::playsoundapi::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
        // the bytes stay in the static for the whole run: the sound is played asynchronously from this memory
        PlaySoundW(WAV.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
    }
}

static TEXT: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

#[cfg(target_os = "windows")]
mod win {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
    use winapi::shared::windef::{HWND, RECT};
    use winapi::um::libloaderapi::GetModuleHandleW;
    use winapi::um::wingdi::*;
    use winapi::um::winuser::*;

    static STARTED: AtomicBool = AtomicBool::new(false);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn proc_(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            WM_ERASEBKGND => 1,
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc: RECT = std::mem::zeroed();
                GetClientRect(hwnd, &mut rc);
                // black is the transparent colour of the window: the text floats over the game
                FillRect(hdc, &rc, GetStockObject(BLACK_BRUSH as i32) as _);
                SetBkMode(hdc, TRANSPARENT as i32);
                let full = TEXT.lock().clone();
                let (head, stats) = full.split_once('\n').unwrap_or((full.as_str(), ""));
                // the label, and under it a small dim line with the load of the computer
                let font = CreateFontW(22, 0, 0, 0, 700, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, wide("Segoe UI").as_ptr());
                let old = SelectObject(hdc, font as _);
                SetTextColor(hdc, 0x0080FF00); // a light green (BGR)
                let mut top = RECT { left: 0, top: 0, right: rc.right, bottom: 34 };
                let text = wide(head);
                DrawTextW(hdc, text.as_ptr(), -1, &mut top, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                SelectObject(hdc, old);
                DeleteObject(font as _);
                if !stats.is_empty() {
                    let small = CreateFontW(16, 0, 0, 0, 600, 0, 0, 0, DEFAULT_CHARSET as u32, 0, 0, CLEARTYPE_QUALITY as u32, 0, wide("Segoe UI").as_ptr());
                    let old = SelectObject(hdc, small as _);
                    SetTextColor(hdc, 0x00C8C8C8);
                    let mut low = RECT { left: 0, top: 34, right: rc.right, bottom: 58 };
                    let text = wide(stats);
                    DrawTextW(hdc, text.as_ptr(), -1, &mut low, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
                    SelectObject(hdc, old);
                    DeleteObject(small as _);
                }
                EndPaint(hwnd, &ps);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    pub fn start() {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| unsafe {
            let hinst = GetModuleHandleW(std::ptr::null());
            let class = wide("JarvisGameLabel");
            let wc = WNDCLASSEXW { cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32, style: 0, lpfnWndProc: Some(proc_), cbClsExtra: 0, cbWndExtra: 0, hInstance: hinst, hIcon: std::ptr::null_mut(), hCursor: std::ptr::null_mut(), hbrBackground: std::ptr::null_mut(), lpszMenuName: std::ptr::null(), lpszClassName: class.as_ptr(), hIconSm: std::ptr::null_mut() };
            RegisterClassExW(&wc);
            // always on top, passes the mouse through, never takes the focus, not in the task bar
            let hwnd = CreateWindowExW(WS_EX_TOPMOST | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE, class.as_ptr(), wide("Jarvis game").as_ptr(), WS_POPUP, 24, 24, 860, 58, std::ptr::null_mut(), std::ptr::null_mut(), hinst, std::ptr::null_mut());
            if hwnd.is_null() {
                return;
            }
            SetLayeredWindowAttributes(hwnd, 0, 255, LWA_COLORKEY);
            let mut shown = false;
            let mut last = String::new();
            let mut stats = String::new();
            // (a program in the full screen is looked for twice a second, not at every turn of the loop)
            let (mut any_game, mut any_at) = (false, std::time::Instant::now() - std::time::Duration::from_secs(5));
            let mut stats_at = std::time::Instant::now() - std::time::Duration::from_secs(5);
            loop {
                if any_at.elapsed() >= std::time::Duration::from_millis(500) {
                    any_game = crate::aimassist::front_any_game(&crate::aimassist::effective(&gamemode::profile_aim())).is_some();
                    any_at = std::time::Instant::now();
                }
                let want = crate::gamelist::options().overlay && gamemode::is_active() && (gamemode::in_game_now() || any_game);
                if want && stats_at.elapsed() >= std::time::Duration::from_secs(1) {
                    stats = {
                        crate::gamefps::start();
                        crate::sysstat::line_with_fps(&crate::sysstat::sample(), crate::aimassist::aim_rate(), crate::gamefps::fps_now(), crate::gamefps::note())
                    };
                    stats_at = std::time::Instant::now();
                }
                let text = if want { format!("{}\n{}", current_label(), stats) } else { String::new() };
                if want != shown {
                    ShowWindow(hwnd, if want { SW_SHOWNOACTIVATE } else { SW_HIDE });
                    shown = want;
                    if want {
                        SetWindowPos(hwnd, HWND_TOPMOST, 24, 24, 860, 58, SWP_NOACTIVATE);
                    }
                }
                if want && text != last {
                    *TEXT.lock() = text.clone();
                    InvalidateRect(hwnd, std::ptr::null(), 1);
                    last = text;
                }
                let mut m: MSG = std::mem::zeroed();
                while PeekMessageW(&mut m, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                    TranslateMessage(&m);
                    DispatchMessageW(&m);
                }
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
        });
    }
}

// starts the label (once); it shows and hides itself
pub fn start() {
    #[cfg(target_os = "windows")]
    win::start();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_tells_the_state() {
        assert_eq!(label_text(&[], false, None, None), "● ИГРОВОЙ РЕЖИМ");
        assert_eq!(label_text(&["W".into(), "SHIFT".into()], false, None, None), "● ИГРОВОЙ РЕЖИМ · W+SHIFT");
        assert_eq!(label_text(&[], true, None, None), "● ИГРОВОЙ РЕЖИМ · ПРИЦЕЛ");
        assert_eq!(label_text(&["W".into()], true, Some("комбо"), None), "● ИГРОВОЙ РЕЖИМ · W · ПРИЦЕЛ · ЗАПИСЬ «комбо»");
        assert_eq!(label_text(&["W".into()], false, None, Some(("иди вперёд", true))), "● ИГРОВОЙ РЕЖИМ · W · «иди вперёд» ✓");
        assert_eq!(label_text(&[], false, None, Some(("иди в перёт", false))), "● ИГРОВОЙ РЕЖИМ · «иди в перёт» ✗ не понял");
    }

    #[test]
    fn the_confirmation_sound_is_a_short_valid_wav() {
        let w = confirm_wav();
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(w.len(), 44 + 1920 * 2);
        let samples: Vec<i16> = w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        assert!(samples.iter().any(|s| s.abs() > 3000), "a tone, not silence");
        assert!(samples[0].abs() < 200 && samples[samples.len() - 1].abs() < 200, "soft start and end (no click)");
    }
}
