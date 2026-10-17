//! An experimental aim assist for SINGLE-PLAYER games (it belongs to the game mode, gamemode.rs). Off at every start.
//!
//! What it does: while the user HOLDS THE RIGHT MOUSE BUTTON (aims down the sights) and the window of a game of the profile is in
//! front, it looks at a square of the screen around the crosshair, asks an ONNX detector (YOLO-style, supplied by the user) where the
//! people are, takes the one nearest to the crosshair and nudges the mouse a part of the way towards its head in every frame.
//! What it does not do: it does not read the memory of a game, does not inject anything into it and does not touch its files. It
//! only takes a picture of the screen (like a screenshot tool) and moves the mouse by relative steps (like a hand would).
//! It never runs when the game mode is off, when the window in front is not a program of the profile, or without a model file.
//!
//! The detector: the default is the YOLOv8 export (`yolo export model=yolov8n.pt format=onnx imgsz=640`): input `images`
//! (1×3×640×640, RGB, 0..1), output `output0` (1×(4+classes)×anchors: centre x, centre y, width, height, then the class scores).
//! Class 0 is «person». The file lies at `<data folder>/aim/player.onnx` (or the path of game_profile.json → aim.model).
//! This part of the code is split in two: the arithmetic below (decode, NMS, target choice, the size of a step) is plain and tested; the
//! capture of the screen and the run of the model are Windows / ONNX Runtime only.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

// the side of the square around the crosshair that the detector looks at
pub const SIDE: usize = 640;

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
}

impl Default for AimConfig {
    fn default() -> Self {
        AimConfig { model: None, strength_pct: 25, head_pct: 12, radius_px: 260, conf_pct: 45, class: 0 }
    }
}

pub fn config_from_json(v: &Value) -> Result<AimConfig, String> {
    let mut c = AimConfig::default();
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
    if let Some(m) = v.get("model").and_then(|m| m.as_str()) {
        let m = m.trim();
        if !m.to_lowercase().ends_with(".onnx") || m.len() > 260 {
            return Err("aim.model: нужен путь к файлу .onnx".into());
        }
        c.model = Some(m.to_string());
    }
    Ok(c)
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
// the running part
// ---------------------------------------------------------------------------------------------
static ENABLED: AtomicBool = AtomicBool::new(false);
static FRAMES: AtomicU64 = AtomicU64::new(0);
static LAST_MS: AtomicU64 = AtomicU64::new(0);
static PROBLEM: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub fn model_path(cfg: &AimConfig) -> std::path::PathBuf {
    match &cfg.model {
        Some(p) => std::path::PathBuf::from(p),
        None => crate::APP_DIR.join("aim").join("player.onnx"),
    }
}

// on / off; the answer for the user. Off at every start (never remembered).
pub fn set_enabled(on: bool, cfg: &AimConfig) -> String {
    if !on {
        ENABLED.store(false, Ordering::SeqCst);
        return "Помощь прицела выключена.".into();
    }
    let path = model_path(cfg);
    if !path.is_file() {
        ENABLED.store(false, Ordering::SeqCst);
        return format!("Помощь прицела не включена: нет файла модели {} (см. docs/GAME-MODE.md, как его получить).", path.display());
    }
    if !cfg!(all(target_os = "windows", feature = "ort")) {
        ENABLED.store(false, Ordering::SeqCst);
        return "Помощь прицела работает только в Windows-сборке с нейросетями.".into();
    }
    PROBLEM.lock().clear();
    ENABLED.store(true, Ordering::SeqCst);
    #[cfg(all(target_os = "windows", feature = "ort"))]
    run::start();
    "Помощь прицела включена: работает, пока вы держите правую кнопку мыши в окне игры. «Выключи помощь прицела» отключает.".into()
}

pub fn status_json(cfg: &AimConfig) -> Value {
    json!({ "enabled": is_enabled(), "model": model_path(cfg).display().to_string(), "model_found": model_path(cfg).is_file(),
        "frames": FRAMES.load(Ordering::SeqCst), "last_frame_ms": LAST_MS.load(Ordering::SeqCst), "problem": PROBLEM.lock().clone() })
}

// one frame: a picture (BGRA, `side`×`side`) -> the step of the mouse (None: no target). The session is a closure, so that the
// tests can put a fake detector in its place.
pub fn frame_step(bgra: &[u8], side: usize, cfg: &AimConfig, detect: &mut dyn FnMut(&[f32], usize) -> Result<(Vec<f32>, usize, usize), String>) -> Result<Option<(i32, i32)>, String> {
    if bgra.len() != side * side * 4 {
        return Err("кадр неверного размера".into());
    }
    // BGRA -> RGB planes, 0..1
    let plane = side * side;
    let mut rgb = vec![0f32; plane * 3];
    for i in 0..plane {
        rgb[i] = bgra[i * 4 + 2] as f32 / 255.0;
        rgb[plane + i] = bgra[i * 4 + 1] as f32 / 255.0;
        rgb[2 * plane + i] = bgra[i * 4] as f32 / 255.0;
    }
    let (out, channels, anchors) = detect(&rgb, side)?;
    let dets = decode(&out, channels, anchors, cfg.class, cfg.conf_pct as f32 / 100.0).ok_or("ответ модели не подходит по форме")?;
    let dets = nms(dets, 0.5);
    let centre = (side as f32 / 2.0, side as f32 / 2.0);
    Ok(pick(&dets, centre, cfg).map(|(_, head)| step((head.0 - centre.0, head.1 - centre.1), cfg.strength_pct, 40.0)).filter(|s| *s != (0, 0)))
}

#[cfg(all(target_os = "windows", feature = "ort"))]
mod run {
    use super::*;
    use ort::value::Tensor;

    static STARTED: AtomicBool = AtomicBool::new(false);

    // the model: loaded once, when it is first needed
    pub fn load(path: &std::path::Path) -> Result<ort::session::Session, String> {
        ort::session::Session::builder()
            .map_err(|e| format!("ONNX Runtime: {}", e))?
            .commit_from_file(path)
            .map_err(|e| format!("модель {} не загрузилась: {}", path.display(), e))
    }

    // runs the detector on an RGB picture: the output tensor with its shape (channels, anchors)
    pub fn detect(session: &mut ort::session::Session, rgb: &[f32], side: usize) -> Result<(Vec<f32>, usize, usize), String> {
        let arr = ndarray::Array4::from_shape_vec((1, 3, side, side), rgb.to_vec()).map_err(|e| e.to_string())?;
        let input = Tensor::from_array(arr).map_err(|e| format!("tensor: {}", e))?;
        let outputs = session.run(ort::inputs! { "images" => input }).map_err(|e| format!("ort: {}", e))?;
        let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| format!("ответ модели: {}", e))?;
        let dims: Vec<usize> = shape.iter().map(|&d| d as usize).collect();
        if dims.len() != 3 || dims[0] != 1 {
            return Err(format!("ответ модели формы {:?}: нужна форма 1×(4+классы)×боксы", dims));
        }
        Ok((data.to_vec(), dims[1], dims[2]))
    }

    // the picture of the square around the crosshair (the centre of the main screen); BGRA
    unsafe fn capture(side: i32) -> Option<Vec<u8>> {
        use winapi::um::wingdi::*;
        use winapi::um::winuser::*;
        let (sw, sh) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        if sw < side || sh < side {
            return None;
        }
        let (x, y) = ((sw - side) / 2, (sh - side) / 2);
        let hdc = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return None;
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
        if ok { Some(buf) } else { None }
    }

    fn right_button_down() -> bool {
        unsafe { (winapi::um::winuser::GetAsyncKeyState(0x02) as u16) & 0x8000 != 0 }
    }

    pub fn start() {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }
        std::thread::spawn(|| {
            let mut session: Option<ort::session::Session> = None;
            loop {
                let cfg = crate::gamemode::aim_config();
                let ready = ENABLED.load(Ordering::SeqCst) && crate::gamemode::is_active() && crate::gamemode::in_game_now() && crate::gamemode::anticheat_running().is_none();
                if !ready {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                    continue;
                }
                if !right_button_down() {
                    std::thread::sleep(std::time::Duration::from_millis(15));
                    continue;
                }
                if session.is_none() {
                    match load(&model_path(&cfg)) {
                        Ok(s) => session = Some(s),
                        Err(e) => {
                            *PROBLEM.lock() = e;
                            ENABLED.store(false, Ordering::SeqCst);
                            continue;
                        }
                    }
                }
                let started = std::time::Instant::now();
                let Some(frame) = (unsafe { capture(SIDE as i32) }) else {
                    *PROBLEM.lock() = "экран не снимается (игра в «настоящем» полноэкранном режиме? включите «окно без рамки»)".into();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    continue;
                };
                let s = session.as_mut().unwrap();
                match frame_step(&frame, SIDE, &cfg, &mut |rgb, side| detect(s, rgb, side)) {
                    Ok(Some((dx, dy))) => crate::gamemode::mouse_rel(dx, dy),
                    Ok(None) => {}
                    Err(e) => {
                        *PROBLEM.lock() = e;
                        ENABLED.store(false, Ordering::SeqCst);
                        continue;
                    }
                }
                FRAMES.fetch_add(1, Ordering::SeqCst);
                LAST_MS.store(started.elapsed().as_millis() as u64, Ordering::SeqCst);
                // about sixty frames a second at the most: the game keeps its time
                let spent = started.elapsed();
                if spent < std::time::Duration::from_millis(16) {
                    std::thread::sleep(std::time::Duration::from_millis(16) - spent);
                }
            }
        });
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
        let ok = config_from_json(&json!({ "strength": 40, "head": 15, "radius": 200, "conf": 50, "class": 0, "model": "D:\\m\\p.onnx" })).unwrap();
        assert_eq!((ok.strength_pct, ok.head_pct, ok.radius_px, ok.conf_pct), (40, 15, 200, 50));
        assert_eq!(config_from_json(&json!({})).unwrap(), AimConfig::default());
        for bad in [json!({ "strength": 0 }), json!({ "strength": 500 }), json!({ "head": 90 }), json!({ "radius": 5000 }), json!({ "model": "x.exe" }), json!({ "conf": "high" })] {
            assert!(config_from_json(&bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn a_frame_with_a_target_gives_a_step_towards_its_head_and_without_a_target_none() {
        let cfg = AimConfig::default();
        let side = 64usize;
        let frame = vec![0u8; side * side * 4];
        // a fake detector: one person whose head is up and to the left of the centre of a 64×64 picture
        let mut seen_len = 0usize;
        let mut one = |rgb: &[f32], s: usize| -> Result<(Vec<f32>, usize, usize), String> {
            seen_len = rgb.len();
            assert_eq!(s, side);
            // cx=20, cy=40, w=16, h=60 -> the box 12..28 × 10..70, the head at (20, 10 + 60*0.12 = 17.2)
            Ok((vec![20.0, 40.0, 16.0, 60.0, 0.9], 5, 1))
        };
        let step_found = frame_step(&frame, side, &cfg, &mut one).unwrap().expect("a target");
        assert_eq!(seen_len, 3 * side * side, "RGB planes");
        assert!(step_found.0 < 0 && step_found.1 < 0, "left and up: {:?}", step_found);
        let mut nobody = |_: &[f32], _: usize| -> Result<(Vec<f32>, usize, usize), String> { Ok((vec![0.0; 5], 5, 1)) };
        assert_eq!(frame_step(&frame, side, &cfg, &mut nobody).unwrap(), None);
        // a failing detector and a frame of a wrong size are errors, not a move of the mouse
        let mut broken = |_: &[f32], _: usize| -> Result<(Vec<f32>, usize, usize), String> { Err("нет".into()) };
        assert!(frame_step(&frame, side, &cfg, &mut broken).is_err());
        assert!(frame_step(&frame[..100], side, &cfg, &mut nobody).is_err());
    }

    #[test]
    fn the_pixels_of_a_frame_become_rgb_planes() {
        let cfg = AimConfig::default();
        let side = 2usize;
        // B, G, R, A of four pixels
        let frame = vec![0, 0, 255, 255, 0, 255, 0, 255, 255, 0, 0, 255, 10, 20, 30, 255];
        let mut got: Vec<f32> = Vec::new();
        let mut spy = |rgb: &[f32], _: usize| -> Result<(Vec<f32>, usize, usize), String> {
            got = rgb.to_vec();
            Ok((vec![0.0; 5], 5, 1))
        };
        frame_step(&frame, side, &cfg, &mut spy).unwrap();
        assert_eq!(&got[0..4], &[1.0, 0.0, 0.0, 30.0 / 255.0], "the red plane");
        assert_eq!(&got[4..8], &[0.0, 1.0, 0.0, 20.0 / 255.0], "the green plane");
        assert_eq!(&got[8..12], &[0.0, 0.0, 1.0, 10.0 / 255.0], "the blue plane");
    }

    // the real ONNX Runtime with a tiny fake model (a fixed answer of a detector): the whole way from a picture to a step of the mouse
    #[cfg(feature = "ort")]
    #[test]
    fn a_real_onnx_session_runs_the_whole_way_with_a_fake_detector() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata/aim_dummy.onnx");
        let mut session = ort::session::Session::builder().unwrap().commit_from_file(&path).expect("the fake model loads");
        let frame = vec![7u8; SIDE * SIDE * 4];
        let cfg = AimConfig::default();
        let mut run = |rgb: &[f32], s: usize| -> Result<(Vec<f32>, usize, usize), String> {
            use ort::value::Tensor;
            let arr = ndarray::Array4::from_shape_vec((1, 3, s, s), rgb.to_vec()).map_err(|e| e.to_string())?;
            let outputs = session.run(ort::inputs! { "images" => Tensor::from_array(arr).map_err(|e| e.to_string())? }).map_err(|e| e.to_string())?;
            let (shape, data) = outputs[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
            let d: Vec<usize> = shape.iter().map(|&x| x as usize).collect();
            Ok((data.to_vec(), d[1], d[2]))
        };
        // the fake detector always says: a person (320, 300), 100×200 (twice, nearly the same, and a weak one far away).
        // Its head is at y = 200 + 200 × 0.12 = 224, the crosshair is at (320, 320): 96 pixels up, a quarter of the way = 24
        let r = frame_step(&frame, SIDE, &cfg, &mut run).unwrap();
        assert_eq!(r, Some((0, -24)));
    }
}
