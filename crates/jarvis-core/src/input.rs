// Keyboard shortcuts and mouse actions for the commands "новая вкладка", "выдели всё", "клик", "разверни окно" ...
//
// A shortcut goes to the window the user works in. When the Jarvis window itself is in front (the command was typed
// there), the window under it gets the focus first: Ctrl+T must open a tab in the browser, not in the Jarvis window.
// Only real key presses of Windows (SendInput) are used; nothing is injected into other programs in any other way.

// a virtual-key code of Windows for a key name ("ctrl", "t", "f5", "left", "page_down" ...)
pub fn vk(name: &str) -> Option<u16> {
    let n = name.trim().to_lowercase();
    let code = match n.as_str() {
        "ctrl" | "control" => 0x11,
        "shift" => 0x10,
        "alt" => 0x12,
        "win" | "windows" => 0x5B,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "tab" => 0x09,
        "space" => 0x20,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "page_up" | "pageup" => 0x21,
        "page_down" | "pagedown" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        "print_screen" => 0x2C,
        "browser_back" => 0xA6,
        "browser_forward" => 0xA7,
        "browser_refresh" => 0xA8,
        "plus" | "=" => 0xBB,
        "minus" | "-" => 0xBD,
        _ => {
            let mut chars = n.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => c.to_ascii_uppercase() as u16,
                (Some('f'), Some(_)) => match n[1..].parse::<u16>() {
                    Ok(k @ 1..=24) => 0x70 + k - 1,
                    _ => return None,
                },
                _ => return None,
            }
        }
    };
    Some(code)
}

// "ctrl+shift+t" -> [VK_CONTROL, VK_SHIFT, 'T']; None when a key is unknown (nothing is pressed then)
pub fn parse_combo(combo: &str) -> Option<Vec<u16>> {
    let keys: Option<Vec<u16>> = combo.split('+').filter(|k| !k.trim().is_empty()).map(vk).collect();
    keys.filter(|k| !k.is_empty())
}

// keys that extend to the "extended" set (arrows, home, end ...): they need the flag, or Windows sees the numpad keys
fn extended(code: u16) -> bool {
    matches!(code, 0x21..=0x28 | 0x2D | 0x2E | 0x5B | 0xA6 | 0xA7 | 0xA8)
}

#[derive(Debug, PartialEq)]
pub enum Outcome {
    Done,
    // the Jarvis window was in front and no other window could be found to work in
    NoTarget,
    Unknown(String),
}

#[cfg(target_os = "windows")]
mod win {
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::*;

    fn title(hwnd: HWND) -> String {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), 256) };
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }

    // the window of Jarvis itself: its page is called "Джарвис" (a browser adds " — Microsoft Edge" and the like);
    // a GitHub page or a folder "Jarvis-1.0" in another program is not it
    pub fn is_jarvis(hwnd: HWND) -> bool {
        let t = title(hwnd).to_lowercase();
        let t = t.trim();
        t == "джарвис" || t == "jarvis" || t.starts_with("джарвис - ") || t.starts_with("джарвис — ") || t.starts_with("jarvis - ") || t.starts_with("jarvis — ")
            || t.contains("127.0.0.1:9714")
    }

    // the next normal visible window under the Jarvis window (the one the user worked in before)
    unsafe fn window_below(start: HWND) -> HWND {
        let mut h = GetWindow(start, GW_HWNDNEXT);
        while !h.is_null() {
            let style = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            if IsWindowVisible(h) != 0 && IsIconic(h) == 0 && style & WS_EX_TOOLWINDOW == 0 && !title(h).trim().is_empty() && !is_jarvis(h) && title(h) != "Program Manager" {
                return h;
            }
            h = GetWindow(h, GW_HWNDNEXT);
        }
        std::ptr::null_mut()
    }

    // makes sure the keys go to the user's window; false when there is none
    pub unsafe fn focus_target() -> bool {
        let fg = GetForegroundWindow();
        if fg.is_null() || !is_jarvis(fg) {
            return true;
        }
        let target = window_below(fg);
        if target.is_null() {
            return false;
        }
        // Windows lets a background program bring a window to the front right after a key press
        keybd_event(0x12, 0, 0, 0);
        keybd_event(0x12, 0, KEYEVENTF_KEYUP, 0);
        SetForegroundWindow(target);
        std::thread::sleep(std::time::Duration::from_millis(120));
        true
    }

    fn key(code: u16, up: bool) -> INPUT {
        let mut i: INPUT = unsafe { std::mem::zeroed() };
        i.type_ = INPUT_KEYBOARD;
        let mut flags = if up { KEYEVENTF_KEYUP } else { 0 };
        if super::extended(code) {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        unsafe {
            *i.u.ki_mut() = KEYBDINPUT { wVk: code, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 };
        }
        i
    }

    pub unsafe fn press(codes: &[u16]) {
        let mut inputs: Vec<INPUT> = codes.iter().map(|&c| key(c, false)).collect();
        inputs.extend(codes.iter().rev().map(|&c| key(c, true)));
        SendInput(inputs.len() as u32, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32);
    }

    fn mouse(flags: u32) -> INPUT {
        let mut i: INPUT = unsafe { std::mem::zeroed() };
        i.type_ = INPUT_MOUSE;
        unsafe {
            *i.u.mi_mut() = MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 };
        }
        i
    }

    pub unsafe fn click(right: bool, times: u32) {
        let (down, up) = if right { (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP) } else { (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP) };
        let mut inputs: Vec<INPUT> = Vec::new();
        for _ in 0..times {
            inputs.push(mouse(down));
            inputs.push(mouse(up));
        }
        SendInput(inputs.len() as u32, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32);
    }

    pub unsafe fn move_by(dx: i32, dy: i32) {
        let mut p: winapi::shared::windef::POINT = std::mem::zeroed();
        GetCursorPos(&mut p);
        SetCursorPos(p.x + dx, p.y + dy);
    }

    pub unsafe fn scroll(lines: i32) {
        let mut i = mouse(MOUSEEVENTF_WHEEL);
        i.u.mi_mut().mouseData = (lines * WHEEL_DELTA as i32) as u32;
        SendInput(1, &mut i, std::mem::size_of::<INPUT>() as i32);
    }

    // the window the keys go to: maximize / restore / minimize
    pub unsafe fn window(cmd: i32) {
        let fg = GetForegroundWindow();
        if !fg.is_null() {
            ShowWindow(fg, cmd);
        }
    }

    pub const MAXIMIZE: i32 = SW_MAXIMIZE;
    pub const MINIMIZE: i32 = SW_MINIMIZE;
    pub const RESTORE: i32 = SW_RESTORE;
}

// presses a shortcut ("ctrl+t") in the user's window
pub fn keys(combo: &str) -> Outcome {
    let Some(codes) = parse_combo(combo) else { return Outcome::Unknown(combo.to_string()) };
    #[cfg(target_os = "windows")]
    unsafe {
        if !win::focus_target() {
            return Outcome::NoTarget;
        }
        win::press(&codes);
        Outcome::Done
    }
    #[cfg(not(target_os = "windows"))]
    {
        info!("Input: keys {:?} (Windows only)", codes);
        Outcome::Done
    }
}

// "click" | "double" | "right" | "up" | "down" | "left" | "right_move" | "scroll_up" | "scroll_down"
pub fn mouse(action: &str, amount: i32) -> Outcome {
    let amount = if amount == 0 { 100 } else { amount.clamp(-3000, 3000) };
    #[cfg(target_os = "windows")]
    unsafe {
        match action {
            "click" | "double" | "right" => {
                if !win::focus_target() {
                    return Outcome::NoTarget;
                }
                win::click(action == "right", if action == "double" { 2 } else { 1 });
            }
            "up" => win::move_by(0, -amount),
            "down" => win::move_by(0, amount),
            "left" => win::move_by(-amount, 0),
            "right_move" => win::move_by(amount, 0),
            "scroll_up" => win::scroll(amount.clamp(1, 50)),
            "scroll_down" => win::scroll(-amount.clamp(1, 50)),
            other => return Outcome::Unknown(other.to_string()),
        }
        Outcome::Done
    }
    #[cfg(not(target_os = "windows"))]
    {
        match action {
            "click" | "double" | "right" | "up" | "down" | "left" | "right_move" | "scroll_up" | "scroll_down" => {
                info!("Input: mouse {} {} (Windows only)", action, amount);
                Outcome::Done
            }
            other => Outcome::Unknown(other.to_string()),
        }
    }
}

// "maximize" | "minimize" | "restore" for the user's window
pub fn window(action: &str) -> Outcome {
    #[cfg(target_os = "windows")]
    unsafe {
        let cmd = match action {
            "maximize" => win::MAXIMIZE,
            "minimize" => win::MINIMIZE,
            "restore" => win::RESTORE,
            other => return Outcome::Unknown(other.to_string()),
        };
        if !win::focus_target() {
            return Outcome::NoTarget;
        }
        win::window(cmd);
        Outcome::Done
    }
    #[cfg(not(target_os = "windows"))]
    match action {
        "maximize" | "minimize" | "restore" => Outcome::Done,
        other => Outcome::Unknown(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_are_parsed_to_real_key_codes() {
        assert_eq!(parse_combo("ctrl+t"), Some(vec![0x11, 0x54]));
        assert_eq!(parse_combo("Ctrl+Shift+T"), Some(vec![0x11, 0x10, 0x54]));
        assert_eq!(parse_combo("alt+left"), Some(vec![0x12, 0x25]));
        assert_eq!(parse_combo("f5"), Some(vec![0x74]));
        assert_eq!(parse_combo("win+v"), Some(vec![0x5B, 0x56]));
        assert_eq!(parse_combo("ctrl+page_down"), Some(vec![0x11, 0x22]));
        assert_eq!(parse_combo("ctrl+home"), Some(vec![0x11, 0x24]));
        assert_eq!(parse_combo("win+shift+s"), Some(vec![0x5B, 0x10, 0x53]));
        assert_eq!(parse_combo("ctrl+bogus"), None, "an unknown key presses nothing");
        assert_eq!(parse_combo(""), None);
        assert_eq!(keys("ctrl+nope"), Outcome::Unknown("ctrl+nope".into()));
        assert!(matches!(mouse("wiggle", 0), Outcome::Unknown(_)));
        assert!(matches!(window("explode"), Outcome::Unknown(_)));
    }
}
