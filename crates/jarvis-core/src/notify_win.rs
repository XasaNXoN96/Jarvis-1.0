// The notification window of Jarvis (Windows): a small graphite card "JARVIS // <ЗАГОЛОВОК>" in the top right corner
// of the screen (under the top edge of the work area), in the style of the Jarvis window. It fades and slides in, stays a few seconds
// (longer while the mouse is over it), fades out; a click closes it. It never takes the keyboard focus
// (WS_EX_NOACTIVATE, MA_NOACTIVATE): typing in another program or in the Jarvis window is not interrupted, and it
// has no buttons, so a click can never run a command again. Notifications are shown one after another.
//
// Only plain Win32/GDI calls that exist on Windows 10 and 11.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HDC, HWND, POINT, RECT};
use winapi::um::libloaderapi::GetModuleHandleW;
use winapi::um::wingdi::*;
use winapi::um::winuser::*;

#[derive(Clone, Debug)]
pub struct Popup {
    pub title: String,
    pub text: String,
    pub source: String,
    pub time: String,
    pub error: bool,
    pub critical: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    In(Instant),
    Hold(Instant),
    Out(Instant),
}

struct Ui {
    current: Option<Popup>,
    phase: Phase,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    scale: f32,
}

static QUEUE: Lazy<Mutex<VecDeque<Popup>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static UI: Lazy<Mutex<Ui>> = Lazy::new(|| Mutex::new(Ui { current: None, phase: Phase::Idle, x: 0, y: 0, w: 0, h: 0, scale: 1.0 }));
static STARTED: AtomicBool = AtomicBool::new(false);

const FADE_IN: Duration = Duration::from_millis(220);
const FADE_OUT: Duration = Duration::from_millis(260);
const ALPHA: f32 = 245.0;
const SLIDE: f32 = 14.0;

pub fn show(p: Popup) {
    {
        let mut q = QUEUE.lock();
        q.push_back(p);
        // a burst of notifications: the oldest waiting ones are dropped (they stay in the list of the window)
        while q.len() > 4 {
            q.pop_front();
        }
    }
    if !STARTED.swap(true, Ordering::SeqCst) {
        std::thread::spawn(|| unsafe { run() });
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn hold_time(p: &Popup) -> Duration {
    let base = if p.critical || p.error { 9 } else { 6 };
    Duration::from_secs(base + (p.text.chars().count() as u64 / 80).min(4))
}

unsafe fn run() {
    let class = wide("JarvisNotification");
    let hinst = GetModuleHandleW(std::ptr::null());
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as UINT,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: std::ptr::null_mut(),
        hCursor: LoadCursorW(std::ptr::null_mut(), IDC_HAND),
        hbrBackground: std::ptr::null_mut(),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    RegisterClassExW(&wc);
    let title = wide("Jarvis");
    let hwnd = CreateWindowExW(
        WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED | WS_EX_NOACTIVATE,
        class.as_ptr(),
        title.as_ptr(),
        WS_POPUP,
        0,
        0,
        10,
        10,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinst,
        std::ptr::null_mut(),
    );
    if hwnd.is_null() {
        warn!("Notifications: the window could not be created");
        STARTED.store(false, Ordering::SeqCst);
        return;
    }
    SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
    SetTimer(hwnd, 1, 30, None);
    let mut msg: MSG = std::mem::zeroed();
    while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe fn screen_scale() -> f32 {
    let hdc = GetDC(std::ptr::null_mut());
    let dpi = GetDeviceCaps(hdc, LOGPIXELSX);
    ReleaseDC(std::ptr::null_mut(), hdc);
    if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 }
}

// geometry of a new card (top right of the work area)
unsafe fn layout(ui: &mut Ui, p: Popup) {
    let s = screen_scale();
    let mut work: RECT = std::mem::zeroed();
    SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut work as *mut RECT as *mut _, 0);
    let n = p.text.chars().count();
    let lines = if n > 90 { 3.0 } else if n > 45 { 2.0 } else { 1.0 };
    let w = (372.0 * s) as i32;
    let h = ((58.0 + lines * 19.0) * s) as i32;
    let margin = (16.0 * s) as i32;
    ui.scale = s;
    ui.w = w;
    ui.h = h;
    ui.x = work.right - w - margin;
    ui.y = work.top + margin;
    ui.current = Some(p);
    ui.phase = Phase::In(Instant::now());
}

unsafe fn mouse_over(hwnd: HWND) -> bool {
    let mut pt: POINT = std::mem::zeroed();
    let mut r: RECT = std::mem::zeroed();
    GetCursorPos(&mut pt) != 0 && GetWindowRect(hwnd, &mut r) != 0 && pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
}

enum Act {
    Nothing,
    Show { x: i32, y: i32, w: i32, h: i32, radius: i32 },
    Frame { alpha: u8, x: i32, y: i32, w: i32, h: i32 },
    Hide,
}

// The state is decided under the lock; the window calls are made after it is released
// (a window call may send messages to wndproc, which takes the same lock).
unsafe fn tick(hwnd: HWND) {
    let over = mouse_over(hwnd);
    let act = {
        let mut ui = UI.lock();
        match ui.phase {
            Phase::Idle => {
                let next = QUEUE.lock().pop_front();
                match next {
                    Some(p) => {
                        layout(&mut ui, p);
                        Act::Show { x: ui.x, y: ui.y + (SLIDE * ui.scale) as i32, w: ui.w, h: ui.h, radius: (16.0 * ui.scale) as i32 }
                    }
                    None => Act::Nothing,
                }
            }
            Phase::In(t0) => {
                let k = (t0.elapsed().as_secs_f32() / FADE_IN.as_secs_f32()).min(1.0);
                let ease = 1.0 - (1.0 - k) * (1.0 - k);
                if k >= 1.0 {
                    let hold = ui.current.as_ref().map(hold_time).unwrap_or(Duration::from_secs(6));
                    ui.phase = Phase::Hold(Instant::now() + hold);
                }
                Act::Frame { alpha: (ALPHA * ease) as u8, x: ui.x, y: ui.y + (SLIDE * ui.scale * (1.0 - ease)) as i32, w: ui.w, h: ui.h }
            }
            Phase::Hold(until) => {
                if over {
                    // the user is reading it: it stays
                    ui.phase = Phase::Hold(until.max(Instant::now() + Duration::from_millis(1500)));
                } else if Instant::now() >= until {
                    ui.phase = Phase::Out(Instant::now());
                }
                Act::Nothing
            }
            Phase::Out(t0) => {
                let k = (t0.elapsed().as_secs_f32() / FADE_OUT.as_secs_f32()).min(1.0);
                if k >= 1.0 {
                    ui.current = None;
                    ui.phase = Phase::Idle;
                    Act::Hide
                } else {
                    Act::Frame { alpha: (ALPHA * (1.0 - k)) as u8, x: ui.x, y: ui.y + (SLIDE * ui.scale * k * 0.6) as i32, w: ui.w, h: ui.h }
                }
            }
        }
    };
    match act {
        Act::Nothing => {}
        Act::Show { x, y, w, h, radius } => {
            SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
            SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE);
            let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, radius, radius);
            SetWindowRgn(hwnd, rgn, 1); // the window owns the region from now on
            ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            InvalidateRect(hwnd, std::ptr::null(), 1);
        }
        Act::Frame { alpha, x, y, w, h } => {
            SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
            SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, h, SWP_NOACTIVATE | SWP_NOREDRAW);
        }
        Act::Hide => {
            ShowWindow(hwnd, SW_HIDE);
        }
    }
}

unsafe fn font(px: f32, weight: i32) -> winapi::shared::windef::HFONT {
    let face = wide("Segoe UI");
    CreateFontW(-(px as i32), 0, 0, 0, weight, 0, 0, 0, DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS, CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_DONTCARE, face.as_ptr())
}

unsafe fn text_out(hdc: HDC, text: &str, r: &mut RECT, flags: UINT) {
    let w = wide(text);
    DrawTextW(hdc, w.as_ptr(), -1, r, flags | DT_NOPREFIX);
}

// the header of the card, as in the window: "JARVIS // ТАЙМЕР"
pub fn header(title: &str) -> String {
    let t = title.trim();
    if t.is_empty() {
        "JARVIS".to_string()
    } else {
        format!("JARVIS // {}", t.to_uppercase())
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(hwnd, &mut ps);
    let snapshot = UI.try_lock().map(|ui| (ui.w, ui.h, ui.scale, ui.current.clone()));
    let Some((w, h, s, current)) = snapshot else {
        EndPaint(hwnd, &ps);
        InvalidateRect(hwnd, std::ptr::null(), 0);
        return;
    };
    if let Some(p) = current {
        // drawn off screen first: no flicker
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        let old_bmp = SelectObject(mem, bmp as _);

        let accent = if p.error || p.critical { RGB(255, 107, 122) } else { RGB(232, 232, 236) };
        // the card: the graphite background of the Jarvis window and a thin border
        let bg = CreateSolidBrush(RGB(17, 17, 19));
        let pen = CreatePen(PS_SOLID as i32, (1.0 * s).max(1.0) as i32, if p.error || p.critical { RGB(110, 46, 56) } else { RGB(52, 52, 58) });
        let old_brush = SelectObject(mem, bg as _);
        let old_pen = SelectObject(mem, pen as _);
        RoundRect(mem, 0, 0, w, h, (16.0 * s) as i32, (16.0 * s) as i32);
        // the accent line on the left
        let bar = CreateSolidBrush(accent);
        let r = RECT { left: (10.0 * s) as i32, top: (14.0 * s) as i32, right: (13.0 * s) as i32, bottom: h - (14.0 * s) as i32 };
        FillRect(mem, &r, bar);

        SetBkMode(mem, TRANSPARENT as i32);
        let f_title = font(12.5 * s, FW_SEMIBOLD);
        let f_small = font(12.0 * s, FW_NORMAL);
        let f_text = font(13.5 * s, FW_NORMAL);
        let left = (24.0 * s) as i32;
        let right = w - (16.0 * s) as i32;

        // time (top right)
        SelectObject(mem, f_small as _);
        SetTextColor(mem, RGB(139, 139, 147));
        let mut rt = RECT { left, top: (12.0 * s) as i32, right, bottom: (32.0 * s) as i32 };
        text_out(mem, &p.time, &mut rt, DT_RIGHT | DT_SINGLELINE | DT_VCENTER);
        // title
        SelectObject(mem, f_title as _);
        SetTextColor(mem, if p.error || p.critical { accent } else { RGB(160, 160, 168) });
        let mut rt = RECT { left, top: (11.0 * s) as i32, right: right - (48.0 * s) as i32, bottom: (33.0 * s) as i32 };
        text_out(mem, &header(&p.title), &mut rt, DT_LEFT | DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS);
        // message
        SelectObject(mem, f_text as _);
        SetTextColor(mem, RGB(236, 236, 238));
        let mut rt = RECT { left, top: (36.0 * s) as i32, right, bottom: h - (12.0 * s) as i32 };
        text_out(mem, &p.text, &mut rt, DT_LEFT | DT_WORDBREAK | DT_EDITCONTROL | DT_END_ELLIPSIS);

        BitBlt(hdc, 0, 0, w, h, mem, 0, 0, SRCCOPY);

        SelectObject(mem, old_brush);
        SelectObject(mem, old_pen);
        SelectObject(mem, old_bmp);
        for o in [bg as _, pen as _, bar as _, f_title as _, f_small as _, f_text as _, bmp as _] {
            DeleteObject(o);
        }
        DeleteDC(mem);
    }
    EndPaint(hwnd, &ps);
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: UINT, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            tick(hwnd);
            0
        }
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_ERASEBKGND => 1,
        // a click never activates the window (the focus stays where the user types)
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_LBUTTONUP | WM_RBUTTONUP => {
            if let Some(mut ui) = UI.try_lock() {
                if matches!(ui.phase, Phase::In(_) | Phase::Hold(_)) {
                    ui.phase = Phase::Out(Instant::now());
                }
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_card_header_is_jarvis_and_the_title_in_capitals() {
        assert_eq!(super::header("Таймер"), "JARVIS // ТАЙМЕР");
        assert_eq!(super::header("  "), "JARVIS");
    }
}
