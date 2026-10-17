//! An experimental aim assist for SINGLE-PLAYER games (it belongs to the game mode, gamemode.rs). Off at every start.
//!
//! What it does: while the user HOLDS THE RIGHT MOUSE BUTTON (aims down the sights) and the window of a game of the profile is in
//! front, it looks at a square of the screen around the crosshair, asks an ONNX detector of people where they are, takes the one nearest
//! to the crosshair and nudges the mouse a part of the way towards its head in every frame.
//! What it does not do: it does not read the memory of a game, does not inject anything into it and does not touch its files. It
//! only takes a picture of the screen (like a screenshot tool) and moves the mouse by relative steps (like a hand would).
//! It never runs when the game mode is off, when the window in front is not a program of the profile, when an anti-cheat program
//! runs, or without a model that loaded and passed the check.
//!
//! The model: installed from the Games page (aimmodel.rs: the official YOLOX-s of OpenCV Zoo with a checked SHA-256, or a file of the
//! user). Several YOLO families are understood (aimdetect.rs). The file lies at `<data folder>/aim/player.onnx` (or the path of
//! game_profile.json → aim.model). The model finds WHOLE PEOPLE (class «person»); «head» and «body» are points chosen in the box.
//! This part is split in two: the arithmetic (decode, NMS, target choice, the size of a step) and the settings are plain and tested; the
//! capture of the screen is Windows only; the run of the model is ONNX Runtime (tested with small fake models on any system).

use crate::aimdetect::{decode_output, preprocess, scale_back, ModelSpec, RawOut};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};

// the default side of the square around the crosshair that the detector looks at
pub const SIDE: usize = 640;

// where the model runs: Auto = the video card when this build has DirectML and it works, else the processor
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Device {
    Auto,
    Cpu,
    DirectMl,
}

impl Device {
    pub fn from_name(n: &str) -> Option<Device> {
        match n.trim().to_lowercase().as_str() {
            "auto" => Some(Device::Auto),
            "cpu" => Some(Device::Cpu),
            "directml" | "dml" | "gpu" => Some(Device::DirectMl),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Device::Auto => "auto",
            Device::Cpu => "cpu",
            Device::DirectMl => "directml",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Device::Auto => "Авто",
            Device::Cpu => "Процессор (CPU)",
            Device::DirectMl => "Видеокарта (DirectML)",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AimConfig {
    // the path of the model; None = <data folder>/aim/player.onnx
    pub model: Option<String>,
    // how much of the remaining way is made in one frame, percent
    pub strength_pct: u32,
    // where the head is in the box of a person: this part of the height from the top, percent
    pub head_pct: u32,
    // only a person whose head is nearer to the crosshair than this (pixels) is followed
    pub radius_px: u32,
    // the least score of the detector, percent
    pub conf_pct: u32,
    // the class of «person» in the model
    pub class: usize,
    // where the model runs
    pub device: Device,
    // the side of the square of the screen that is looked at, pixels (a smaller one is faster and sees less around the crosshair)
    pub side: u32,
    // the most frames a second (the program lowers it by itself when the computer cannot keep up)
    pub fps: u32,
    // a person whose box is lower than this (pixels of the screen) is not taken: far away or a part of something
    pub min_size_px: u32,
}

impl Default for AimConfig {
    fn default() -> Self {
        AimConfig { model: None, strength_pct: 25, head_pct: 12, radius_px: 260, conf_pct: 45, class: 0, device: Device::Auto, side: SIDE as u32, fps: 30, min_size_px: 24 }
    }
}

// the numbers named in `v` (a part of game_profile.json or the settings of the page) are put into `c`, each one checked
pub fn apply_json(c: &mut AimConfig, v: &Value) -> Result<(), String> {
    let num = |k: &str, lo: i64, hi: i64| -> Result<Option<u32>, String> {
        match v.get(k) {
            None => Ok(None),
            Some(x) => match x.as_i64() {
                Some(n) if (lo..=hi).contains(&n) => Ok(Some(n as u32)),
                _ => Err(format!("aim.{} должно быть числом от {} до {}", k, lo, hi)),
            },
        }
    };
    if let Some(n) = num("strength", 1, 100)? {
        c.strength_pct = n;
    }
    if let Some(n) = num("head", 3, 40)? {
        c.head_pct = n;
    }
    if let Some(n) = num("radius", 40, 320)? {
        c.radius_px = n;
    }
    if let Some(n) = num("conf", 10, 95)? {
        c.conf_pct = n;
    }
    if let Some(n) = num("class", 0, 1000)? {
        c.class = n as usize;
    }
    if let Some(n) = num("side", 160, 1024)? {
        c.side = n;
    }
    if let Some(n) = num("fps", 1, 60)? {
        c.fps = n;
    }
    if let Some(n) = num("min_size", 0, 400)? {
        c.min_size_px = n;
    }
    if let Some(d) = v.get("device") {
        c.device = d.as_str().and_then(Device::from_name).ok_or("aim.device: auto, cpu или directml")?;
    }
    if let Some(m) = v.get("model").and_then(|m| m.as_str()) {
        let m = m.trim();
        if !m.to_lowercase().ends_with(".onnx") || m.len() > 260 {
            return Err("aim.model: нужен путь к файлу .onnx".into());
        }
        c.model = Some(m.to_string());
    }
    Ok(())
}

pub fn config_from_json(v: &Value) -> Result<AimConfig, String> {
    let mut c = AimConfig::default();
    apply_json(&mut c, v)?;
    Ok(c)
}

// ---------------------------------------------------------------------------------------------
// the settings of the page (the device, the size of the picture, the frames a second, the limit of the score, the least size of a person):
// kept in `aim_settings.json` of the data folder and laid over the profile. Nothing else of the game files is touched.
// ---------------------------------------------------------------------------------------------
fn settings_file() -> std::path::PathBuf {
    #[cfg(test)]
    {
        return std::env::temp_dir().join(format!("jarvis-aim-settings-{}.json", std::process::id()));
    }
    #[cfg(not(test))]
    {
        crate::APP_DIR.join("aim_settings.json")
    }
}

fn read_overlay() -> Value {
    std::fs::read_to_string(settings_file()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(|v| v.is_object()).unwrap_or_else(|| json!({}))
}

// the settings of the page are checked as a whole, then kept; the answer is the configuration that results
pub fn save_settings(patch: &Value, base: &AimConfig) -> Result<AimConfig, String> {
    let Some(obj) = patch.as_object() else { return Err("настройки должны быть объектом".into()) };
    // only the keys of the page (the path of a model is chosen with the file window, not typed here)
    let allowed = ["device", "side", "fps", "conf", "min_size", "strength", "head", "radius"];
    if obj.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err("неизвестная настройка".into());
    }
    let mut now = read_overlay();
    for (k, v) in obj {
        now[k] = v.clone();
    }
    let mut c = base.clone();
    apply_json(&mut c, &now)?;
    let path = settings_file();
    let text = serde_json::to_string_pretty(&now).unwrap_or_default();
    crate::write_atomic(&path, text).map_err(|e| format!("не удалось сохранить настройки: {}", e))?;
    Ok(effective(base))
}

// the configuration that is used now: the profile, the settings of the page over it, the part of the body that was asked for by voice
pub fn effective(profile: &AimConfig) -> AimConfig {
    let mut c = profile.clone();
    // (a settings file that does not fit leaves the profile as it is; the page checks the numbers before it saves them)
    let _ = apply_json(&mut c, &read_overlay());
    if let Some(p) = point_override() {
        c.head_pct = p;
    }
    c.radius_px = c.radius_px.min(c.side / 2);
    c
}

// ---------------------------------------------------------------------------------------------
// the arithmetic
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Det {
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
    pub score: f32,
}

impl Det {
    fn area(&self) -> f32 {
        (self.x2 - self.x1).max(0.0) * (self.y2 - self.y1).max(0.0)
    }
}

// the output of a YOLOv8 model: `channels` rows (cx, cy, w, h, class scores…) of `anchors` values each, row after row.
// Boxes are in the pixels of the input of the model. None when the numbers do not fit the shape.
pub fn decode(out: &[f32], channels: usize, anchors: usize, class: usize, min_score: f32) -> Option<Vec<Det>> {
    if channels < 5 || anchors == 0 || out.len() != channels * anchors || 4 + class >= channels {
        return None;
    }
    let at = |c: usize, i: usize| out[c * anchors + i];
    let mut v = Vec::new();
    for i in 0..anchors {
        let score = at(4 + class, i);
        if !score.is_finite() || score < min_score {
            continue;
        }
        let (cx, cy, w, h) = (at(0, i), at(1, i), at(2, i), at(3, i));
        if !(cx.is_finite() && cy.is_finite() && w.is_finite() && h.is_finite()) || w <= 0.0 || h <= 0.0 {
            continue;
        }
        v.push(Det { x1: cx - w / 2.0, y1: cy - h / 2.0, x2: cx + w / 2.0, y2: cy + h / 2.0, score });
    }
    Some(v)
}

fn iou(a: &Det, b: &Det) -> f32 {
    let (ix1, iy1, ix2, iy2) = (a.x1.max(b.x1), a.y1.max(b.y1), a.x2.min(b.x2), a.y2.min(b.y2));
    let inter = (ix2 - ix1).max(0.0) * (iy2 - iy1).max(0.0);
    let union = a.area() + b.area() - inter;
    if union <= 0.0 { 0.0 } else { inter / union }
}

// the same person found twice leaves once (the better score)
pub fn nms(mut dets: Vec<Det>, iou_limit: f32) -> Vec<Det> {
    dets.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut kept: Vec<Det> = Vec::new();
    for d in dets {
        if kept.iter().all(|k| iou(k, &d) < iou_limit) {
            kept.push(d);
        }
    }
    kept
}

// the head of a person: the middle of the top of the box (head_pct of the height from the top)
pub fn head_of(d: &Det, head_pct: u32) -> (f32, f32) {
    ((d.x1 + d.x2) / 2.0, d.y1 + (d.y2 - d.y1) * head_pct as f32 / 100.0)
}

// the person whose head is nearest to the crosshair, and only within `radius`; the head point is returned too
pub fn pick(dets: &[Det], crosshair: (f32, f32), cfg: &AimConfig) -> Option<(Det, (f32, f32))> {
    dets.iter()
        .map(|d| (*d, head_of(d, cfg.head_pct)))
        .map(|(d, h)| (d, h, ((h.0 - crosshair.0).powi(2) + (h.1 - crosshair.1).powi(2)).sqrt()))
        .filter(|(_, _, dist)| *dist <= cfg.radius_px as f32)
        .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(d, h, _)| (d, h))
}

// the step of the mouse for one frame: a part of the remaining way, at most `max_step` pixels, nothing inside the dead zone
pub fn step(delta: (f32, f32), strength_pct: u32, max_step: f32) -> (i32, i32) {
    const DEAD: f32 = 3.0;
    let dist = (delta.0 * delta.0 + delta.1 * delta.1).sqrt();
    if dist <= DEAD {
        return (0, 0);
    }
    let want = (dist * strength_pct as f32 / 100.0).clamp(1.0, max_step);
    let k = want / dist;
    ((delta.0 * k).round() as i32, (delta.1 * k).round() as i32)
}


// ---------------------------------------------------------------------------------------------
// one frame
// ---------------------------------------------------------------------------------------------
// what one frame showed: nobody in reach, the crosshair is on the target already, or a step of the mouse towards it
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    NoTarget,
    OnTarget,
    Step(i32, i32),
}

// the crosshair is «on the target» when the step is no more than this (pixels, both axes together)
pub const LOCK_PX: i32 = 10;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    pub outcome: Outcome,
    // people found in the picture (after the score limit, NMS and the least size)
    pub objects: usize,
}

// a picture with (almost) nothing in it: black (a protected window, a minimised game) or one flat colour
pub fn frame_is_blank(bgra: &[u8]) -> bool {
    if bgra.len() < 16 {
        return true;
    }
    let first = [bgra[0], bgra[1], bgra[2]];
    // a few hundred sample points are enough: a real picture of a game is never flat
    let step = (bgra.len() / 4 / 400).max(1) * 4;
    bgra.chunks(4).step_by(step / 4).all(|p| p.len() == 4 && p[0].abs_diff(first[0]) < 3 && p[1].abs_diff(first[1]) < 3 && p[2].abs_diff(first[2]) < 3)
}

// A picture (BGRA, `side`×`side`) -> what to do with the mouse. The model is a closure, so that the tests can put a fake one in its place;
// the picture is prepared for the model, the answer is read by the layout of the model, the boxes are scaled back to the picture.
pub fn frame_outcome(bgra: &[u8], side: usize, cfg: &AimConfig, spec: &ModelSpec, detect: &mut dyn FnMut(&[f32]) -> Result<RawOut, String>) -> Result<Frame, String> {
    if side == 0 || bgra.len() != side * side * 4 {
        return Err("кадр неверного размера".into());
    }
    let rgb = preprocess(bgra, side, spec.input_w, spec.input_h, spec.layout.norm());
    let out = detect(&rgb)?;
    let dets = decode_output(&out, spec, cfg.class, cfg.conf_pct as f32 / 100.0).ok_or("ответ модели не подходит по форме")?;
    let dets = scale_back(nms(dets, 0.5), side, spec.input_w, spec.input_h);
    let dets: Vec<Det> = dets.into_iter().filter(|d| (d.y2 - d.y1) >= cfg.min_size_px as f32).collect();
    let centre = (side as f32 / 2.0, side as f32 / 2.0);
    let outcome = match pick(&dets, centre, cfg) {
        None => Outcome::NoTarget,
        Some((_, head)) => match step((head.0 - centre.0, head.1 - centre.1), cfg.strength_pct, 40.0) {
            (0, 0) => Outcome::OnTarget,
            (dx, dy) => Outcome::Step(dx, dy),
        },
    };
    Ok(Frame { outcome, objects: dets.len() })
}

// ---- the point of the body that is aimed at, and «the target is caught» (for «целься в голову» and «прицельный огонь») ----
static POINT: AtomicU32 = AtomicU32::new(0);
static LOCKED_MS: AtomicU64 = AtomicU64::new(0);
static CLOCK: Lazy<std::time::Instant> = Lazy::new(std::time::Instant::now);

// the part of the height (from the top, percent) that is aimed at instead of the head of the configuration; None = the head
pub fn set_point(pct: Option<u32>) {
    POINT.store(pct.unwrap_or(0).min(90), Ordering::SeqCst);
}

pub fn point_override() -> Option<u32> {
    Some(POINT.load(Ordering::SeqCst)).filter(|p| *p != 0)
}

#[cfg(test)]
pub fn mark_locked_for_test() {
    mark_locked();
}

#[allow(dead_code)]
fn mark_locked() {
    LOCKED_MS.store(CLOCK.elapsed().as_millis() as u64 + 1, Ordering::SeqCst);
}

// the crosshair was on a target within the last `ms` milliseconds
pub fn locked_within(ms: u64) -> bool {
    let at = LOCKED_MS.load(Ordering::SeqCst);
    at != 0 && (CLOCK.elapsed().as_millis() as u64).saturating_sub(at) <= ms
}

pub fn clear_locked() {
    LOCKED_MS.store(0, Ordering::SeqCst);
}

// ---------------------------------------------------------------------------------------------
// the state that the page and the diagnostics show
// ---------------------------------------------------------------------------------------------
static ENABLED: AtomicBool = AtomicBool::new(false);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static LAST_MS: AtomicU64 = AtomicU64::new(0);
static OBJECTS: AtomicU32 = AtomicU32::new(0);
// the number of the run: a thread of an older run ends by itself when it is not the current one (no thread is left behind)
static RUN_ID: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static PROBLEM: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static STATE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new("off".into()));
static CAPTURE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new("не снимался".into()));
static DEVICE: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

fn set_state(s: &str) {
    *STATE.lock() = s.to_string();
}

fn fail(msg: impl Into<String>) {
    *PROBLEM.lock() = msg.into();
    ENABLED.store(false, Ordering::SeqCst);
    set_state("error");
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

// a background thread of the aim assist is alive (the tests of the clean stop look at it)
pub fn thread_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn model_path(cfg: &AimConfig) -> std::path::PathBuf {
    match &cfg.model {
        Some(p) => std::path::PathBuf::from(p),
        None => crate::APP_DIR.join("aim").join("player.onnx"),
    }
}

// why the aim assist cannot be turned on (None = it can): the same words are shown on the page
pub fn enable_blockers(cfg: &AimConfig) -> Option<String> {
    let path = model_path(cfg);
    if !path.is_file() {
        return Some("нет модели: установите её на странице «Игры» → «Помощь прицела» (кнопка «Установить модель автоматически»)".into());
    }
    if !cfg!(all(target_os = "windows", feature = "ort")) {
        return Some("помощь прицела работает только в Windows-сборке с нейросетями".into());
    }
    None
}

// on / off; the answer for the user. Off at every start (never remembered).
pub fn set_enabled(on: bool, cfg: &AimConfig) -> String {
    if !on {
        let was = ENABLED.swap(false, Ordering::SeqCst);
        // the running thread sees the end by itself (it checks the flag and the number of the run on every turn)
        RUN_ID.fetch_add(1, Ordering::SeqCst);
        clear_locked();
        set_state("off");
        return if was { "Помощь прицела выключена.".into() } else { "Помощь прицела и так выключена.".into() };
    }
    if let Some(why) = enable_blockers(cfg) {
        ENABLED.store(false, Ordering::SeqCst);
        set_state("off");
        return format!("Помощь прицела не включена: {}.", why);
    }
    PROBLEM.lock().clear();
    FRAMES.store(0, Ordering::SeqCst);
    OBJECTS.store(0, Ordering::SeqCst);
    let id = RUN_ID.fetch_add(1, Ordering::SeqCst) + 1;
    ENABLED.store(true, Ordering::SeqCst);
    set_state("loading");
    #[cfg(all(target_os = "windows", feature = "ort"))]
    run::start(id);
    let _ = id;
    "Помощь прицела включена: работает, пока вы держите правую кнопку мыши в окне игры. «Выключи помощь прицела» или «стой» отключает.".into()
}

// the switch off from outside (the word «стой», the end of the game mode, the end of the program)
pub fn suspend() {
    if is_enabled() {
        set_enabled(false, &AimConfig::default());
    }
}

pub fn status_json(cfg: &AimConfig) -> Value {
    let eff = effective(cfg);
    json!({ "enabled": is_enabled(), "state": STATE.lock().clone(), "model": model_path(cfg).display().to_string(), "model_found": model_path(cfg).is_file(),
        "frames": FRAMES.load(Ordering::SeqCst), "last_frame_ms": LAST_MS.load(Ordering::SeqCst), "objects": OBJECTS.load(Ordering::SeqCst),
        "capture": CAPTURE.lock().clone(), "device_used": DEVICE.lock().clone(), "thread": thread_running(), "problem": PROBLEM.lock().clone(),
        "settings": { "device": eff.device.name(), "side": eff.side, "fps": eff.fps, "conf": eff.conf_pct, "min_size": eff.min_size_px } })
}

// how long to rest after a frame: the frame rate that was asked for, and when the computer is slow (a frame takes more than the budget) the
// rest is at least half of the time of the frame, so that the program never takes more than two thirds of a core from the game
pub fn rest_after(spent: std::time::Duration, fps: u32) -> std::time::Duration {
    let budget = std::time::Duration::from_millis(1000 / fps.clamp(1, 60) as u64);
    if spent < budget {
        budget - spent
    } else {
        spent / 2
    }
}

#[cfg(all(target_os = "windows", feature = "ort"))]
mod run {
    use super::*;
    use crate::aimdetect::ortrun;

    // the window in front: its client area on the screen (x, y, w, h); None when there is none, it is minimised or has no size
    unsafe fn window_area() -> Result<(i32, i32, i32, i32), String> {
        use winapi::shared::windef::{POINT, RECT};
        use winapi::um::winuser::*;
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return Err("нет окна переднего плана".into());
        }
        if IsIconic(hwnd) != 0 {
            return Err("окно игры свёрнуто".into());
        }
        let mut r: RECT = std::mem::zeroed();
        if GetClientRect(hwnd, &mut r) == 0 {
            return Err("окно игры закрыто".into());
        }
        let mut p = POINT { x: 0, y: 0 };
        if ClientToScreen(hwnd, &mut p) == 0 {
            return Err("окно игры закрыто".into());
        }
        Ok((p.x, p.y, r.right - r.left, r.bottom - r.top))
    }

    // the picture of the square around the middle of the game window (the crosshair); BGRA. `side` is cut down to the window.
    unsafe fn capture(want: i32) -> Result<(Vec<u8>, usize), String> {
        use winapi::um::wingdi::*;
        use winapi::um::winuser::*;
        let (wx, wy, ww, wh) = window_area()?;
        let side = want.min(ww).min(wh) & !1;
        if side < 64 {
            return Err(format!("окно игры слишком маленькое ({}×{})", ww, wh));
        }
        let (x, y) = (wx + (ww - side) / 2, wy + (wh - side) / 2);
        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return Err("экран не открывается для снимка".into());
        }
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, side, side);
        let old = SelectObject(mem, bmp as _);
        let mut ok = BitBlt(mem, 0, 0, side, side, hdc, x, y, SRCCOPY) != 0;
        let mut buf = vec![0u8; (side * side * 4) as usize];
        if ok {
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: side, biHeight: -side, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, biSizeImage: 0, biXPelsPerMeter: 0, biYPelsPerMeter: 0, biClrUsed: 0, biClrImportant: 0 };
            ok = GetDIBits(mem, bmp, 0, side as u32, buf.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS) != 0;
        }
        SelectObject(mem, old);
        DeleteObject(bmp as _);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), hdc);
        if ok { Ok((buf, side as usize)) } else { Err("снимок экрана не получился".into()) }
    }

    fn right_button_down() -> bool {
        unsafe { (winapi::um::winuser::GetAsyncKeyState(0x02) as u16) & 0x8000 != 0 }
    }

    // this run is the current one and still wanted
    fn alive(id: u64) -> bool {
        ENABLED.load(Ordering::SeqCst) && RUN_ID.load(Ordering::SeqCst) == id
    }

    pub fn start(id: u64) {
        // an older thread, if there is one, ends by itself (its number is not the current one); a new one starts at once
        std::thread::spawn(move || {
            // two threads never work together: wait for the older one to leave (it checks every few milliseconds)
            let mut waited = 0;
            while RUNNING.swap(true, Ordering::SeqCst) {
                if !alive(id) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
                waited += 1;
                if waited > 500 {
                    fail("прошлый запуск не завершился");
                    return;
                }
            }
            main_loop(id);
            RUNNING.store(false, Ordering::SeqCst);
        });
    }

    fn main_loop(id: u64) {
        let cfg0 = crate::gamemode::aim_config();
        // the model is loaded and tried once; any failure turns the module off with the reason (it does not wait for ever)
        let info = match ortrun::probe(&model_path(&cfg0), &cfg0) {
            Ok(i) => i,
            Err(e) => {
                fail(format!("модель не годится: {}", e));
                return;
            }
        };
        let mut session = match ortrun::session_for(&model_path(&cfg0), cfg0.device) {
            Ok((s, d, note)) => {
                *DEVICE.lock() = if note.is_empty() { d.title().to_string() } else { format!("{} ({})", d.title(), note) };
                s
            }
            Err(e) => {
                fail(e);
                return;
            }
        };
        let (mut cap_fail, mut blank) = (0u32, 0u32);
        while alive(id) {
            let cfg = crate::gamemode::aim_config();
            let ready = crate::gamemode::is_active() && crate::gamemode::in_game_now() && crate::gamemode::anticheat_running().is_none();
            if !ready {
                set_state("waiting");
                *CAPTURE.lock() = "ждёт окно игры".into();
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            if !right_button_down() {
                set_state("waiting");
                std::thread::sleep(std::time::Duration::from_millis(15));
                continue;
            }
            let started = std::time::Instant::now();
            let (frame, side) = match unsafe { capture(cfg.side as i32) } {
                Ok(f) => f,
                Err(e) => {
                    cap_fail += 1;
                    *CAPTURE.lock() = format!("ошибка: {}", e);
                    // a minimised or closed window is a pause, not a failure; a long row of failures stops the module with the reason
                    if cap_fail >= 120 {
                        fail(format!("экран не снимается: {} (игра в «настоящем» полноэкранном режиме? включите «окно без рамки»)", e));
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(250));
                    continue;
                }
            };
            cap_fail = 0;
            if frame_is_blank(&frame) {
                blank += 1;
                *CAPTURE.lock() = "кадр пустой (чёрный)".into();
                if blank >= 90 {
                    fail("экран снимается чёрным: окно игры защищено от снимков или игра в монопольном полноэкранном режиме (включите «окно без рамки»)");
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            blank = 0;
            *CAPTURE.lock() = format!("ок, {}×{}", side, side);
            set_state("working");
            let res = frame_outcome(&frame, side, &cfg, &info.spec, &mut |rgb| ortrun::run(&mut session, &info.input_name, rgb, info.spec.input_w, info.spec.input_h));
            match res {
                Ok(f) => {
                    OBJECTS.store(f.objects as u32, Ordering::SeqCst);
                    // the module may have been switched off while the model worked: no move of the mouse after that
                    if !alive(id) {
                        break;
                    }
                    match f.outcome {
                        Outcome::Step(dx, dy) => {
                            crate::gamemode::mouse_rel(dx, dy);
                            if dx.abs() + dy.abs() <= LOCK_PX {
                                mark_locked();
                            }
                        }
                        Outcome::OnTarget => mark_locked(),
                        Outcome::NoTarget => {}
                    }
                }
                Err(e) => {
                    fail(e);
                    return;
                }
            }
            FRAMES.fetch_add(1, Ordering::SeqCst);
            let spent = started.elapsed();
            LAST_MS.store(spent.as_millis() as u64, Ordering::SeqCst);
            std::thread::sleep(rest_after(spent, cfg.fps));
        }
        set_state(if ENABLED.load(Ordering::SeqCst) { "loading" } else { "off" });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det(x1: f32, y1: f32, x2: f32, y2: f32, score: f32) -> Det {
        Det { x1, y1, x2, y2, score }
    }

    #[test]
    fn the_output_of_yolo_is_decoded_by_class_and_score() {
        // 5 channels (box + one class), 3 anchors, row after row
        let out = [
            320.0, 330.0, 50.0, // cx
            300.0, 302.0, 50.0, // cy
            100.0, 100.0, 20.0, // w
            200.0, 200.0, 20.0, // h
            0.9, 0.8, 0.2, // score
        ];
        let v = decode(&out, 5, 3, 0, 0.45).unwrap();
        assert_eq!(v.len(), 2, "the weak one is dropped");
        assert_eq!(v[0], det(270.0, 200.0, 370.0, 400.0, 0.9));
        // a wrong shape, a class that does not exist, rubbish numbers
        assert!(decode(&out, 5, 4, 0, 0.1).is_none());
        assert!(decode(&out, 5, 3, 1, 0.1).is_none());
        let mut bad = out;
        bad[0] = f32::NAN;
        bad[12] = f32::NAN;
        assert_eq!(decode(&bad, 5, 3, 0, 0.25).unwrap().len(), 1, "the broken anchor is skipped, the weak one is below the limit");
        assert!(decode(&[], 5, 0, 0, 0.1).is_none());
    }

    #[test]
    fn the_same_person_found_twice_stays_once_with_the_better_score() {
        let kept = nms(vec![det(270.0, 200.0, 370.0, 400.0, 0.8), det(272.0, 202.0, 372.0, 402.0, 0.9), det(10.0, 10.0, 40.0, 80.0, 0.7)], 0.5);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].score, 0.9);
        assert!(kept.iter().any(|d| d.score == 0.7));
    }

    #[test]
    fn the_head_is_near_the_top_of_the_box_and_the_nearest_head_to_the_crosshair_is_taken_within_the_radius() {
        let cfg = AimConfig::default();
        let near = det(300.0, 250.0, 340.0, 400.0, 0.9); // head at (320, 250 + 150*0.12 = 268)
        let far = det(10.0, 10.0, 50.0, 160.0, 0.95);
        assert_eq!(head_of(&near, 12), (320.0, 268.0));
        let (d, head) = pick(&[far, near], (320.0, 320.0), &cfg).unwrap();
        assert_eq!(d, near);
        assert_eq!(head, (320.0, 268.0));
        // nobody within the radius: nothing is followed (a person far from the crosshair is not chased across the screen)
        assert!(pick(&[far], (320.0, 320.0), &cfg).is_none());
        assert!(pick(&[], (320.0, 320.0), &cfg).is_none());
    }

    #[test]
    fn a_step_is_a_part_of_the_way_never_more_than_the_limit_and_nothing_near_the_target() {
        assert_eq!(step((0.0, 0.0), 25, 40.0), (0, 0));
        assert_eq!(step((2.0, 1.0), 25, 40.0), (0, 0), "dead zone");
        assert_eq!(step((100.0, 0.0), 25, 40.0), (25, 0));
        assert_eq!(step((-100.0, 0.0), 50, 40.0), (-40, 0), "the limit");
        let (dx, dy) = step((30.0, 40.0), 20, 40.0);
        assert!((dx, dy) == (6, 8), "{:?}", (dx, dy));
        // never overshoots: a step is never longer than the way
        let (dx, dy) = step((4.0, 0.0), 100, 40.0);
        assert!(dx <= 4 && dy == 0);
    }

    #[test]
    fn the_config_is_checked_number_by_number() {
        let ok = config_from_json(&json!({ "strength": 40, "head": 15, "radius": 200, "conf": 50, "class": 0, "model": "D:\\m\\p.onnx", "device": "cpu", "side": 512, "fps": 20, "min_size": 30 })).unwrap();
        assert_eq!((ok.strength_pct, ok.head_pct, ok.radius_px, ok.conf_pct), (40, 15, 200, 50));
        assert_eq!((ok.device, ok.side, ok.fps, ok.min_size_px), (Device::Cpu, 512, 20, 30));
        assert_eq!(config_from_json(&json!({})).unwrap(), AimConfig::default());
        for bad in [json!({ "strength": 0 }), json!({ "strength": 500 }), json!({ "head": 90 }), json!({ "radius": 5000 }), json!({ "model": "x.exe" }), json!({ "conf": "high" }), json!({ "device": "tpu" }), json!({ "side": 10 }), json!({ "fps": 500 })] {
            assert!(config_from_json(&bad).is_err(), "{}", bad);
        }
    }

    fn spec8(side: usize) -> ModelSpec {
        ModelSpec { layout: crate::aimdetect::Layout::ChannelsFirst, input_w: side, input_h: side, classes: 1, anchors: 1 }
    }
    fn out8(v: Vec<f32>, anchors: usize) -> RawOut {
        RawOut { data: v, dims: vec![1, 5, anchors] }
    }

    #[test]
    fn a_frame_with_a_target_gives_a_step_towards_its_head_and_without_a_target_none() {
        let cfg = AimConfig { min_size_px: 0, ..AimConfig::default() };
        let side = 64usize;
        let frame = vec![0u8; side * side * 4];
        let spec = spec8(side);
        let mut seen_len = 0usize;
        let mut one = |rgb: &[f32]| -> Result<RawOut, String> {
            seen_len = rgb.len();
            // cx=20, cy=40, w=16, h=60 -> the box 12..28 × 10..70, the head at (20, 10 + 60*0.12 = 17.2)
            Ok(out8(vec![20.0, 40.0, 16.0, 60.0, 0.9], 1))
        };
        let f = frame_outcome(&frame, side, &cfg, &spec, &mut one).unwrap();
        assert_eq!(seen_len, 3 * side * side, "RGB planes");
        assert_eq!(f.objects, 1);
        match f.outcome {
            Outcome::Step(dx, dy) => assert!(dx < 0 && dy < 0, "left and up: {:?}", (dx, dy)),
            o => panic!("{:?}", o),
        }
        let mut nobody = |_: &[f32]| -> Result<RawOut, String> { Ok(out8(vec![0.0; 5], 1)) };
        let f = frame_outcome(&frame, side, &cfg, &spec, &mut nobody).unwrap();
        assert_eq!((f.outcome, f.objects), (Outcome::NoTarget, 0));
        // a failing detector and a frame of a wrong size are errors, not a move of the mouse
        let mut broken = |_: &[f32]| -> Result<RawOut, String> { Err("нет".into()) };
        assert!(frame_outcome(&frame, side, &cfg, &spec, &mut broken).is_err());
        assert!(frame_outcome(&frame[..100], side, &cfg, &spec, &mut nobody).is_err());
        // an answer of a wrong shape is an error too
        let mut wrong = |_: &[f32]| -> Result<RawOut, String> { Ok(RawOut { data: vec![0.0; 6], dims: vec![1, 5, 2] }) };
        assert!(frame_outcome(&frame, side, &cfg, &spec, &mut wrong).is_err());
    }

    #[test]
    fn the_boxes_are_scaled_back_to_the_picture_when_the_model_takes_another_size() {
        // the picture is 128×128, the model takes 64×64: a box 12..28 × 10..70 of the model is 24..56 × 20..140 on the screen,
        // the head at (40, 20 + 120*0.12 = 34.4): up-left of the centre (64, 64)
        let cfg = AimConfig { min_size_px: 0, ..AimConfig::default() };
        let frame = vec![9u8; 128 * 128 * 4];
        let mut one = |rgb: &[f32]| -> Result<RawOut, String> {
            assert_eq!(rgb.len(), 3 * 64 * 64, "the model gets its own size");
            Ok(out8(vec![20.0, 40.0, 16.0, 60.0, 0.9], 1))
        };
        let f = frame_outcome(&frame, 128, &cfg, &spec8(64), &mut one).unwrap();
        // delta = (40-64, 34.4-64) = (-24, -29.6); 25% of 38.1 = 9.5 pixels
        match f.outcome {
            Outcome::Step(dx, dy) => assert!(dx < 0 && dy < 0 && dx.abs() + dy.abs() >= 8, "{:?}", (dx, dy)),
            o => panic!("{:?}", o),
        }
    }

    #[test]
    fn a_small_object_is_not_taken_and_a_far_one_neither() {
        let frame = vec![9u8; 64 * 64 * 4];
        let mut tiny = |_: &[f32]| -> Result<RawOut, String> { Ok(out8(vec![32.0, 32.0, 6.0, 20.0, 0.9], 1)) };
        let cfg = AimConfig { min_size_px: 40, ..AimConfig::default() };
        let f = frame_outcome(&frame, 64, &cfg, &spec8(64), &mut tiny).unwrap();
        assert_eq!((f.outcome, f.objects), (Outcome::NoTarget, 0), "a box 20 pixels high is below the least size");
        let cfg = AimConfig { min_size_px: 10, ..AimConfig::default() };
        assert_eq!(frame_outcome(&frame, 64, &cfg, &spec8(64), &mut tiny).unwrap().objects, 1);
    }

    #[test]
    fn the_pixels_of_a_frame_become_rgb_planes() {
        let cfg = AimConfig::default();
        let side = 2usize;
        // B, G, R, A of four pixels
        let frame = vec![0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 10, 20, 30, 255];
        let mut got: Vec<f32> = Vec::new();
        let mut spy = |rgb: &[f32]| -> Result<RawOut, String> {
            got = rgb.to_vec();
            Ok(out8(vec![0.0; 5], 1))
        };
        frame_outcome(&frame, side, &cfg, &spec8(2), &mut spy).unwrap();
        assert!(got[0] == 1.0 && (got[3] - 30.0 / 255.0).abs() < 1e-6, "the red plane");
        assert!(got[5] == 1.0 && (got[7] - 20.0 / 255.0).abs() < 1e-6, "the green plane");
        assert!(got[10] == 1.0 && (got[11] - 10.0 / 255.0).abs() < 1e-6, "the blue plane");
    }

    #[test]
    fn a_black_or_flat_picture_is_blank_and_a_real_one_is_not() {
        assert!(frame_is_blank(&vec![0u8; 64 * 64 * 4]));
        assert!(frame_is_blank(&vec![200u8; 64 * 64 * 4]));
        let mut pic = vec![0u8; 64 * 64 * 4];
        for (i, b) in pic.iter_mut().enumerate() {
            *b = ((i / 4) % 251) as u8;
        }
        assert!(!frame_is_blank(&pic));
        assert!(frame_is_blank(&[]));
    }

    #[test]
    fn a_slow_computer_gets_a_rest_and_the_frame_rate_is_kept_otherwise() {
        use std::time::Duration;
        assert_eq!(rest_after(Duration::from_millis(10), 30), Duration::from_millis(23));
        // slower than the budget: half of the time of the frame is rest (never a busy loop)
        assert_eq!(rest_after(Duration::from_millis(100), 30), Duration::from_millis(50));
        assert!(rest_after(Duration::from_millis(1), 0) >= Duration::from_millis(16), "a nonsense frame rate is cut to 1..60");
    }

    #[test]
    fn the_settings_of_the_page_are_checked_kept_and_laid_over_the_profile() {
        let _l = SETTINGS_LOCK.lock();
        let _ = std::fs::remove_file(settings_file());
        set_point(None);
        let base = AimConfig::default();
        assert_eq!(effective(&base), AimConfig { radius_px: 260.min(base.side / 2), ..base.clone() });
        let c = save_settings(&json!({ "device": "cpu", "fps": 15, "conf": 60, "side": 480 }), &base).unwrap();
        assert_eq!((c.device, c.fps, c.conf_pct, c.side), (Device::Cpu, 15, 60, 480));
        assert_eq!(effective(&base).fps, 15, "kept on the disk");
        // a bad number or an unknown key changes nothing
        assert!(save_settings(&json!({ "fps": 9000 }), &base).is_err());
        assert!(save_settings(&json!({ "model": "C:\\x.onnx" }), &base).is_err());
        assert!(save_settings(&json!({ "evil": 1 }), &base).is_err());
        assert_eq!(effective(&base).fps, 15);
        // the point asked by voice wins over the settings
        set_point(Some(38));
        assert_eq!(effective(&base).head_pct, 38);
        set_point(None);
        let _ = std::fs::remove_file(settings_file());
    }

    static SETTINGS_LOCK: Mutex<()> = parking_lot::const_mutex(());

    #[test]
    fn it_does_not_turn_on_without_a_model_and_turning_off_is_always_possible() {
        let cfg = AimConfig { model: Some("Z:\\нет\\такого.onnx".into()), ..AimConfig::default() };
        let said = set_enabled(true, &cfg);
        assert!(said.contains("не включена"), "{}", said);
        assert!(!is_enabled());
        assert!(!thread_running());
        assert_eq!(status_json(&cfg)["state"], "off");
        assert!(set_enabled(false, &cfg).contains("выключена"));
        suspend();
        assert!(!is_enabled());
    }

    // the real ONNX Runtime with tiny fake models (a fixed answer of a detector): the whole way from a picture to a step of the mouse
    #[cfg(feature = "ort")]
    #[test]
    fn real_onnx_sessions_run_the_whole_way_with_fake_detectors_of_every_family() {
        use crate::aimdetect::ortrun;
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata");
        let cfg = AimConfig { device: Device::Cpu, min_size_px: 0, ..AimConfig::default() };
        // YOLOv8 fixture: a person (320, 300), 100×200; the head is at y = 200 + 200 × 0.12 = 224, the crosshair is at (320, 320):
        // 96 pixels up, a quarter of the way = 24. YOLOX fixture: the box 270..370 × 140..340, the head at y = 164: 156 up, a quarter = 39.
        for (file, want) in [("aim_dummy.onnx", -24), ("aim_yolox_dummy.onnx", -39)] {
            let path = dir.join(file);
            let info = ortrun::probe(&path, &cfg).unwrap_or_else(|e| panic!("{}: {}", file, e));
            let (mut session, _, _) = ortrun::session_for(&path, cfg.device).unwrap();
            let frame = vec![7u8; SIDE * SIDE * 4];
            let f = frame_outcome(&frame, SIDE, &cfg, &info.spec, &mut |rgb| ortrun::run(&mut session, &info.input_name, rgb, info.spec.input_w, info.spec.input_h)).unwrap();
            assert_eq!(f.outcome, Outcome::Step(0, want), "{}", file);
        }
        // a classifier and a file that is not a model are refused with a reason
        assert!(ortrun::probe(&dir.join("aim_classifier_dummy.onnx"), &cfg).is_err());
        let junk = std::env::temp_dir().join(format!("jarvis-junk-{}.onnx", std::process::id()));
        std::fs::write(&junk, vec![5u8; 5000]).unwrap();
        assert!(ortrun::probe(&junk, &cfg).is_err());
        std::fs::remove_file(&junk).ok();
    }
}
