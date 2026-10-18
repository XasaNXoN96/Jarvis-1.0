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
// the side that is taken by default: 320 is about 4 times faster than 640 on a processor (a person at the crosshair is still large enough)
pub const DEFAULT_SIDE: u32 = 320;

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
    // where the crosshair is in the window of the game, percent of its width / height from the middle (a game from the third person keeps it
    // higher than the middle, about -9 for the height); the square that is looked at is taken around that point
    pub cx_pct: i32,
    pub cy_pct: i32,
    // a person higher than this part of the picture is not taken (the character of the player himself, a companion at the camera): percent
    pub max_size_pct: u32,
    // look for the crosshair on the picture by itself (aimcross.rs); the numbers above are used while it is not found
    pub auto_cross: bool,
    // a box wider than this part of its height (percent) is not a standing person: a lying body, several people grown into one
    pub max_aspect_pct: u32,
    // inside this distance (pixels) from the head the mouse does not move (no trembling when the crosshair is almost there)
    pub dead_px: u32,
    // the longest step of the mouse for one frame, pixels
    pub max_step_px: u32,
    // keep a frame a second (with the boxes of the model as labels) in a folder, to train a model of this game later
    pub collect: bool,
    // the key or button that is held to aim («rmb», «lmb», «mmb», «x1», «x2», a letter, «shift», «ctrl», «alt», «space», «caps», «f1»…)
    pub trigger: String,
    // work in any game that is in front in the full screen (not only in the games of the list); a program that protects itself from cheats
    // still stops the module
    pub any_game: bool,
}

impl Default for AimConfig {
    fn default() -> Self {
        AimConfig { model: None, strength_pct: 40, head_pct: 12, radius_px: 260, conf_pct: 45, class: 0, device: Device::Auto, side: DEFAULT_SIDE, fps: 30, min_size_px: 24, cx_pct: 0, cy_pct: 0, max_size_pct: 65, auto_cross: true, max_aspect_pct: 150, dead_px: 3, max_step_px: 40, collect: false, trigger: "rmb".into(), any_game: true }
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
    if let Some(n) = num("max_size", 20, 100)? {
        c.max_size_pct = n;
    }
    if let Some(n) = num("max_aspect", 50, 500)? {
        c.max_aspect_pct = n;
    }
    if let Some(n) = num("dead", 0, 20)? {
        c.dead_px = n;
    }
    if let Some(n) = num("max_step", 5, 100)? {
        c.max_step_px = n;
    }
    if let Some(x) = v.get("trigger") {
        let name = x.as_str().ok_or("aim.trigger: название клавиши")?;
        if trigger_vk(name).is_none() {
            return Err(format!("aim.trigger: неизвестная клавиша «{}» (правая или средняя кнопка мыши, боковые x1/x2, буква, цифра, shift, ctrl, alt, space, caps, f1-f12)", name));
        }
        c.trigger = name.trim().to_lowercase();
    }
    if let Some(x) = v.get("any_game") {
        c.any_game = x.as_bool().ok_or("aim.any_game: true или false")?;
    }
    if let Some(x) = v.get("collect") {
        c.collect = x.as_bool().ok_or("aim.collect: true или false")?;
    }
    if let Some(x) = v.get("auto_cross") {
        c.auto_cross = x.as_bool().ok_or("aim.auto_cross: true или false")?;
    }
    for (k, slot) in [("cx", &mut c.cx_pct), ("cy", &mut c.cy_pct)] {
        if let Some(x) = v.get(k) {
            match x.as_i64() {
                Some(n) if (-40..=40).contains(&n) => *slot = n as i32,
                _ => return Err(format!("aim.{} должно быть числом от -40 до 40", k)),
            }
        }
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
    let allowed = ["device", "side", "fps", "conf", "min_size", "max_size", "cx", "cy", "auto_cross", "strength", "head", "radius", "max_aspect", "dead", "max_step", "collect", "trigger", "any_game"];
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

// the name of a game for the table of the games: the file name in small letters, nothing else
fn game_key(exe: &str) -> Option<String> {
    let e = exe.trim().to_lowercase();
    (e.ends_with(".exe") && e.len() <= 80 && !e.contains(['/', '\\', ':', '"', '\0'])).then_some(e)
}

// the crosshair that was kept for this game (percent from the middle of the window), if any
pub fn game_cross(exe: &str) -> Option<(i32, i32)> {
    let key = game_key(exe)?;
    let g = read_overlay();
    let e = g.get("games")?.get(&key)?;
    let (cx, cy) = (e.get("cx")?.as_i64()?, e.get("cy")?.as_i64()?);
    ((-40..=40).contains(&cx) && (-40..=40).contains(&cy)).then_some((cx as i32, cy as i32))
}

// keeps the crosshair for ONE game (the others and the general numbers are not touched)
pub fn save_game_cross(exe: &str, cx: i64, cy: i64) -> Result<(), String> {
    let key = game_key(exe).ok_or("игра не выбрана: включите помощь прицела и зажмите правую кнопку мыши в окне игры один раз")?;
    if !(-40..=40).contains(&cx) || !(-40..=40).contains(&cy) {
        return Err("положение перекрестия: числа от -40 до 40".into());
    }
    let mut now = read_overlay();
    if !now.get("games").map(|g| g.is_object()).unwrap_or(false) {
        now["games"] = json!({});
    }
    // (a table of a hundred games is more than enough; an older entry of the same game is replaced)
    if now["games"].as_object().map(|m| m.len()).unwrap_or(0) >= 100 && now["games"].get(&key).is_none() {
        return Err("слишком много игр в таблице".into());
    }
    if !now["games"][&key].is_object() {
        now["games"][&key] = json!({});
    }
    now["games"][&key]["cx"] = json!(cx);
    now["games"][&key]["cy"] = json!(cy);
    crate::write_atomic(&settings_file(), serde_json::to_string_pretty(&now).unwrap_or_default()).map_err(|e| format!("не удалось сохранить: {}", e))
}

// the numbers that a game may keep for itself (besides the crosshair)
pub const GAME_KEYS: [&str; 9] = ["strength", "head", "radius", "side", "conf", "max_step", "dead", "max_aspect", "trigger"];

// keeps the numbers of ONE game (only GAME_KEYS, each one checked); the others and the crosshair of the game stay
pub fn save_game_settings(exe: &str, patch: &Value) -> Result<usize, String> {
    let key = game_key(exe).ok_or("игра не выбрана: включите помощь прицела и зажмите правую кнопку мыши в окне игры один раз")?;
    let Some(obj) = patch.as_object() else { return Err("настройки должны быть объектом".into()) };
    let picked: serde_json::Map<String, Value> = obj.iter().filter(|(k, _)| GAME_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    // the check of the ranges is the one of the general settings
    apply_json(&mut AimConfig::default(), &Value::Object(picked.clone()))?;
    let mut now = read_overlay();
    if !now.get("games").map(|g| g.is_object()).unwrap_or(false) {
        now["games"] = json!({});
    }
    if now["games"].as_object().map(|m| m.len()).unwrap_or(0) >= 100 && now["games"].get(&key).is_none() {
        return Err("слишком много игр в таблице".into());
    }
    if !now["games"][&key].is_object() {
        now["games"][&key] = json!({});
    }
    for (k, v) in &picked {
        now["games"][&key][k] = v.clone();
    }
    crate::write_atomic(&settings_file(), serde_json::to_string_pretty(&now).unwrap_or_default()).map_err(|e| format!("не удалось сохранить: {}", e))?;
    Ok(picked.len())
}

// the configuration for the game that is in front: the numbers of that game laid over `effective`
pub fn effective_for(profile: &AimConfig, exe: Option<&str>) -> AimConfig {
    let mut c = effective(profile);
    if let Some((cx, cy)) = exe.and_then(game_cross) {
        c.cx_pct = cx;
        c.cy_pct = cy;
    }
    // the other numbers that this game keeps for itself lie over the general ones
    if let Some(e) = exe.and_then(game_key).and_then(|k| read_overlay().get("games").and_then(|g| g.get(&k)).cloned()) {
        if let Some(o) = e.as_object() {
            let mine: serde_json::Map<String, Value> = o.iter().filter(|(k, _)| GAME_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
            let _ = apply_json(&mut c, &Value::Object(mine));
            c.radius_px = c.radius_px.min(c.side / 2);
        }
    }
    c
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

// the virtual key code of a button or key that is held to aim (None = not a name that is known)
pub fn trigger_vk(name: &str) -> Option<i32> {
    let n = name.trim().to_lowercase();
    let code = match n.as_str() {
        "rmb" | "right" | "пкм" => 0x02,
        "lmb" | "left" | "лкм" => 0x01,
        "mmb" | "middle" => 0x04,
        "x1" => 0x05,
        "x2" => 0x06,
        "shift" => 0x10,
        "ctrl" | "control" => 0x11,
        "alt" => 0x12,
        "space" => 0x20,
        "caps" | "capslock" => 0x14,
        "tab" => 0x09,
        _ => {
            let c: Vec<char> = n.chars().collect();
            if c.len() == 1 && c[0].is_ascii_alphanumeric() {
                c[0].to_ascii_uppercase() as i32
            } else if let Some(num) = n.strip_prefix('f').and_then(|x| x.parse::<i32>().ok()).filter(|k| (1..=12).contains(k)) {
                0x70 + num - 1
            } else {
                return None;
            }
        }
    };
    Some(code)
}

// a program in the full screen that may be a game: its file name (None when the setting is off, the program is not a game or it is not full screen)
pub fn front_any_game(cfg: &AimConfig) -> Option<String> {
    if !cfg.any_game {
        return None;
    }
    #[cfg(target_os = "windows")]
    {
        let exe = crate::gamemode::front_exe_any()?;
        (!is_not_a_game(&exe) && run::front_fullscreen()).then_some(exe)
    }
    #[cfg(not(target_os = "windows"))]
    None
}

// programs that are never a game (the browser, the desktop, a player, a messenger): the full screen of these is not the full screen of a game
pub fn is_not_a_game(exe: &str) -> bool {
    const NOT: [&str; 52] = [
        "explorer.exe", "jarvis-app.exe", "chrome.exe", "msedge.exe", "firefox.exe", "opera.exe", "brave.exe", "vivaldi.exe", "browser.exe", "yandex.exe",
        "vlc.exe", "mpv.exe", "obs64.exe", "obs32.exe", "teams.exe", "zoom.exe", "slack.exe", "discord.exe", "telegram.exe", "spotify.exe",
        "code.exe", "devenv.exe", "steam.exe", "steamwebhelper.exe", "epicgameslauncher.exe", "applicationframehost.exe", "searchhost.exe",
        "shellexperiencehost.exe", "startmenuexperiencehost.exe", "lockapp.exe", "taskmgr.exe", "powerpnt.exe", "winword.exe", "excel.exe",
        "mstsc.exe", "vmware.exe", "virtualboxvm.exe", "photoshop.exe", "premiere pro.exe", "notepad.exe",
        "potplayermini64.exe", "potplayer64.exe", "potplayer.exe", "video.ui.exe", "wmplayer.exe", "mpc-hc64.exe", "mpc-hc.exe", "kmplayer.exe", "music.ui.exe", "aimp.exe", "foobar2000.exe", "teamviewer.exe",
    ];
    let e = exe.trim().to_lowercase();
    NOT.contains(&e.as_str())
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
    step_dead(delta, strength_pct, max_step, 3.0)
}

pub fn step_dead(delta: (f32, f32), strength_pct: u32, max_step: f32, dead: f32) -> (i32, i32) {
    let dist = (delta.0 * delta.0 + delta.1 * delta.1).sqrt();
    if dist <= dead || dist == 0.0 {
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

#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub outcome: Outcome,
    // people found in the picture (after the score limit, NMS and the least size)
    pub objects: usize,
    // the people themselves (the pixels of the picture), the one that was chosen and the point of it that is aimed at
    pub dets: Vec<Det>,
    pub picked: Option<(Det, (f32, f32))>,
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
pub fn frame_outcome(bgra: &[u8], side: usize, cfg: &AimConfig, spec: &ModelSpec, crosshair: (f32, f32), detect: &mut dyn FnMut(&[f32]) -> Result<RawOut, String>) -> Result<Frame, String> {
    if side == 0 || bgra.len() != side * side * 4 {
        return Err("кадр неверного размера".into());
    }
    let rgb = preprocess(bgra, side, spec.input_w, spec.input_h, spec.layout.norm());
    let out = detect(&rgb)?;
    let dets = decode_output(&out, spec, cfg.class, cfg.conf_pct as f32 / 100.0).ok_or("ответ модели не подходит по форме")?;
    let dets = scale_back(nms(dets, 0.5), side, spec.input_w, spec.input_h);
    let tallest = side as f32 * cfg.max_size_pct as f32 / 100.0;
    let widest = cfg.max_aspect_pct as f32 / 100.0;
    let dets: Vec<Det> = dets.into_iter().filter(|d| (d.y2 - d.y1) >= cfg.min_size_px as f32 && (d.y2 - d.y1) <= tallest && (d.x2 - d.x1) <= (d.y2 - d.y1) * widest).collect();
    let centre = crosshair;
    let picked = pick(&dets, centre, cfg);
    let outcome = match picked {
        None => Outcome::NoTarget,
        Some((_, head)) => match step_dead((head.0 - centre.0, head.1 - centre.1), cfg.strength_pct, cfg.max_step_px as f32, cfg.dead_px as f32) {
            (0, 0) => Outcome::OnTarget,
            (dx, dy) => Outcome::Step(dx, dy),
        },
    };
    Ok(Frame { outcome, objects: dets.len(), dets, picked })
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
// the snapshot of the last frame with the marks: what the program saw and decided (to find out why the aim goes where it goes). It is kept in
// the memory only, written to a file and shown only when the user asks for it.
// ---------------------------------------------------------------------------------------------
pub struct Snap {
    pub bgra: Vec<u8>,
    pub side: usize,
    pub cross: (f32, f32),
    pub frame: Frame,
    pub auto: bool,
}

static SNAP: Lazy<Mutex<Option<Snap>>> = Lazy::new(|| Mutex::new(None));

pub fn keep_snapshot(s: Snap) {
    *SNAP.lock() = Some(s);
}

fn px(img: &mut [u8], side: usize, x: i32, y: i32, c: [u8; 3]) {
    if x >= 0 && y >= 0 && (x as usize) < side && (y as usize) < side {
        let i = (y as usize * side + x as usize) * 4;
        img[i] = c[0];
        img[i + 1] = c[1];
        img[i + 2] = c[2];
        img[i + 3] = 255;
    }
}

fn line(img: &mut [u8], side: usize, a: (f32, f32), b: (f32, f32), c: [u8; 3], w: i32) {
    let n = ((b.0 - a.0).abs().max((b.1 - a.1).abs())).ceil().max(1.0) as i32;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let (x, y) = ((a.0 + (b.0 - a.0) * t).round() as i32, (a.1 + (b.1 - a.1) * t).round() as i32);
        for dx in -(w / 2)..=(w / 2) {
            for dy in -(w / 2)..=(w / 2) {
                px(img, side, x + dx, y + dy, c);
            }
        }
    }
}

fn rect(img: &mut [u8], side: usize, d: &Det, c: [u8; 3], w: i32) {
    let (a, b, cc, dd) = ((d.x1, d.y1), (d.x2, d.y1), (d.x2, d.y2), (d.x1, d.y2));
    for (p, q) in [(a, b), (b, cc), (cc, dd), (dd, a)] {
        line(img, side, p, q, c, w);
    }
}

fn disc(img: &mut [u8], side: usize, at: (f32, f32), r: i32, c: [u8; 3]) {
    for dx in -r..=r {
        for dy in -r..=r {
            if dx * dx + dy * dy <= r * r {
                px(img, side, at.0.round() as i32 + dx, at.1.round() as i32 + dy, c);
            }
        }
    }
}

// RGBA of the picture with the marks: green boxes = people that were found, yellow thick box = the chosen one, red point = where it is aimed
// at (the head of the box), red line = the way from the crosshair, cyan plus = the crosshair that the program used. Pure, tested.
pub fn render_snapshot(bgra: &[u8], side: usize, cross: (f32, f32), frame: &Frame) -> Vec<u8> {
    let mut img = vec![255u8; side * side * 4];
    for i in 0..side * side {
        img[i * 4] = bgra[i * 4 + 2];
        img[i * 4 + 1] = bgra[i * 4 + 1];
        img[i * 4 + 2] = bgra[i * 4];
    }
    for d in &frame.dets {
        rect(&mut img, side, d, [60, 255, 90], 2);
    }
    if let Some((d, head)) = &frame.picked {
        rect(&mut img, side, d, [255, 230, 40], 4);
        line(&mut img, side, cross, *head, [255, 50, 50], 3);
        disc(&mut img, side, *head, 6, [255, 50, 50]);
    }
    let c = [40, 240, 255];
    line(&mut img, side, (cross.0 - 14.0, cross.1), (cross.0 + 14.0, cross.1), c, 3);
    line(&mut img, side, (cross.0, cross.1 - 14.0), (cross.0, cross.1 + 14.0), c, 3);
    img
}

// the PNG of the last frame with the marks (None: the aim assist has not looked at anything yet)
pub fn snapshot_png() -> Option<Vec<u8>> {
    let g = SNAP.lock();
    let s = g.as_ref()?;
    crate::gameicon::encode_png(s.side as u32, s.side as u32, &render_snapshot(&s.bgra, s.side, s.cross, &s.frame))
}

// ---- frames for the training of a model of one game: a frame a second, with the boxes of the model as labels (YOLO text files) ----
pub const COLLECT_MAX: usize = 500;

pub fn dataset_dir() -> std::path::PathBuf {
    #[cfg(test)]
    {
        return std::env::temp_dir().join(format!("jarvis-aim-dataset-{}", std::process::id()));
    }
    #[cfg(not(test))]
    crate::APP_DIR.join("aim").join("dataset")
}

// the label file of YOLO: «class cx cy w h» of every box, normalised to the side of the picture
pub fn yolo_labels(dets: &[Det], side: usize) -> String {
    let s = side as f32;
    dets.iter().map(|d| format!("0 {:.6} {:.6} {:.6} {:.6}\n", ((d.x1 + d.x2) / 2.0 / s).clamp(0.0, 1.0), ((d.y1 + d.y2) / 2.0 / s).clamp(0.0, 1.0), ((d.x2 - d.x1) / s).clamp(0.0, 1.0), ((d.y2 - d.y1) / s).clamp(0.0, 1.0))).collect()
}

// one frame into the folder (the picture and the labels); Ok(false) when the folder is full
pub fn save_training_frame(bgra: &[u8], side: usize, dets: &[Det]) -> Result<bool, String> {
    let dir = dataset_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let n = std::fs::read_dir(&dir).map(|r| r.filter_map(|e| e.ok()).filter(|e| e.path().extension().map(|x| x == "png").unwrap_or(false)).count()).unwrap_or(0);
    if n >= COLLECT_MAX {
        return Ok(false);
    }
    let mut rgba = vec![255u8; side * side * 4];
    for i in 0..side * side {
        rgba[i * 4] = bgra[i * 4 + 2];
        rgba[i * 4 + 1] = bgra[i * 4 + 1];
        rgba[i * 4 + 2] = bgra[i * 4];
    }
    let png = crate::gameicon::encode_png(side as u32, side as u32, &rgba).ok_or("кадр не кодируется")?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
    crate::write_atomic(&dir.join(format!("f{}.png", stamp)), &png).map_err(|e| e.to_string())?;
    crate::write_atomic(&dir.join(format!("f{}.txt", stamp)), yolo_labels(dets, side)).map_err(|e| e.to_string())?;
    Ok(true)
}

// the side taken: the wished one, lowered by the governor while frames are slow
fn governed_side(wanted: u32, now: &mut u32) -> u32 {
    if *now == 0 || *now > wanted {
        *now = wanted;
    }
    *now
}

pub const SLOW_MS: f32 = 90.0;
pub const MIN_SIDE: u32 = 256;

pub fn lower_side(side: u32, avg_ms: f32) -> u32 {
    if avg_ms > SLOW_MS && side > MIN_SIDE {
        (side - 32).max(MIN_SIDE)
    } else {
        side
    }
}

static LAST_FRAME_AT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
static PERIOD_MS: AtomicU64 = AtomicU64::new(0);

// the time between frames is averaged for the stats line of the game overlay
fn note_frame() {
    let now = std::time::Instant::now();
    let mut g = LAST_FRAME_AT.lock();
    if let Some(prev) = *g {
        let ms = now.duration_since(prev).as_millis() as u64;
        // a pause between two presses of the aim button is not a slow frame
        if ms <= 500 {
            let old = PERIOD_MS.load(Ordering::SeqCst);
            PERIOD_MS.store(if old == 0 { ms } else { (old * 4 + ms) / 5 }, Ordering::SeqCst);
        }
    }
    *g = Some(now);
}

// (frames per second, ms of the last frame) while the aim works, else None
pub fn aim_rate() -> Option<(u32, u64)> {
    if !ENABLED.load(Ordering::SeqCst) {
        return None;
    }
    let at = (*LAST_FRAME_AT.lock())?;
    let period = PERIOD_MS.load(Ordering::SeqCst);
    if at.elapsed().as_millis() > 1500 || period == 0 {
        return None;
    }
    Some(((1000 / period.max(1)) as u32, LAST_MS.load(Ordering::SeqCst)))
}

// the numbers of the last frame for the report (boxes in the pixels of the square, the chosen one, the point aimed at, the step): with them the
// report alone shows where the aim goes and why, without a picture
pub fn snapshot_details() -> String {
    let g = SNAP.lock();
    let Some(s) = g.as_ref() else { return "последний кадр: ещё не было".into() };
    let mut t = format!("Последний кадр {}×{}, перекрестие ({:.0}, {:.0}){}\n", s.side, s.side, s.cross.0, s.cross.1, if s.auto { " найдено автоматически" } else { "" });
    for (i, d) in s.frame.dets.iter().enumerate() {
        let chosen = s.frame.picked.map(|(p, _)| p == *d).unwrap_or(false);
        t.push_str(&format!("  человек {}: рамка ({:.0},{:.0})–({:.0},{:.0}), высота {:.0} пикс., уверенность {:.0}%{}\n", i + 1, d.x1, d.y1, d.x2, d.y2, d.y2 - d.y1, d.score * 100.0, if chosen { "  <- ЦЕЛЬ" } else { "" }));
    }
    if s.frame.dets.is_empty() {
        t.push_str("  людей в кадре нет (после порога уверенности, размера и отбора)\n");
    }
    if let Some((_, head)) = s.frame.picked {
        t.push_str(&format!("  целимся в точку ({:.0}, {:.0}); от перекрестия: {:+.0}, {:+.0} пикс.\n", head.0, head.1, head.0 - s.cross.0, head.1 - s.cross.1));
    }
    t.push_str(&format!("  итог: {}", match s.frame.outcome {
        Outcome::NoTarget => "цели нет".to_string(),
        Outcome::OnTarget => "перекрестие уже на цели".to_string(),
        Outcome::Step(dx, dy) => format!("шаг мыши ({:+}, {:+}) пикс.", dx, dy),
    }));
    t
}

// the words about what the snapshot shows (for the card)
pub fn snapshot_text() -> String {
    match SNAP.lock().as_ref() {
        None => "Снимка ещё нет: включите помощь прицела и зажмите правую кнопку мыши в игре.".into(),
        Some(s) => format!(
            "Кадр {}×{}, перекрестие ({:.0}, {:.0}){}, людей найдено {}, итог: {}",
            s.side,
            s.side,
            s.cross.0,
            s.cross.1,
            if s.auto { " (найдено автоматически)" } else { "" },
            s.frame.dets.len(),
            match s.frame.outcome {
                Outcome::NoTarget => "цели нет".to_string(),
                Outcome::OnTarget => "перекрестие уже на цели".to_string(),
                Outcome::Step(dx, dy) => format!("шаг мыши ({:+}, {:+}) пикселей", dx, dy),
            }
        ),
    }
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
// the game that the aim assist saw last in front, and where it takes the crosshair from now
static LAST_GAME: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
static CROSS_SRC: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new("ещё не определялось".into()));

pub fn last_game() -> String {
    LAST_GAME.lock().clone()
}

pub fn cross_source() -> String {
    CROSS_SRC.lock().clone()
}

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
    *SNAP.lock() = None;
    FRAMES.store(0, Ordering::SeqCst);
    LAST_MS.store(0, Ordering::SeqCst);
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
        "capture": CAPTURE.lock().clone(), "last_game": last_game(), "cross": cross_source(), "device_used": DEVICE.lock().clone(), "thread": thread_running(), "problem": PROBLEM.lock().clone(),
        "settings": { "device": eff.device.name(), "side": eff.side, "fps": eff.fps, "conf": eff.conf_pct, "min_size": eff.min_size_px, "max_size": eff.max_size_pct, "cx": eff.cx_pct, "cy": eff.cy_pct, "auto_cross": eff.auto_cross, "max_aspect": eff.max_aspect_pct, "dead": eff.dead_px, "max_step": eff.max_step_px, "collect": eff.collect, "trigger": eff.trigger, "any_game": eff.any_game, "strength": eff.strength_pct, "radius": eff.radius_px, "head": eff.head_pct } })
}

// how long to rest after a frame: the frame rate that was asked for, and when the computer is slow (a frame takes more than the budget) the
// rest is a quarter of the time of the frame, so that the program never takes more than four fifths of a core from the game (a longer
// pause would only make the aim later; the mouse itself is moved smoothly by the mover, see below)
pub fn rest_after(spent: std::time::Duration, fps: u32) -> std::time::Duration {
    let budget = std::time::Duration::from_millis(1000 / fps.clamp(1, 60) as u64);
    if spent < budget {
        budget - spent
    } else {
        spent / 4
    }
}

// ---- the smooth move of the mouse: the detector gives a place about six times a second, the mouse is moved many times a second ----
// `pending` is the way that is left to the target (pixels); every tick of the mover takes a part of it. Pure, tested.
pub const MOVER_GAIN: f32 = 0.30;

pub fn mover_tick(pending: &mut (f32, f32), carry: &mut (f32, f32)) -> (i32, i32) {
    let take = (pending.0 * MOVER_GAIN, pending.1 * MOVER_GAIN);
    pending.0 -= take.0;
    pending.1 -= take.1;
    carry.0 += take.0;
    carry.1 += take.1;
    let send = (carry.0.trunc(), carry.1.trunc());
    carry.0 -= send.0;
    carry.1 -= send.1;
    // what is left under a pixel is let go (it would never be sent)
    if pending.0.abs() < 0.4 && pending.1.abs() < 0.4 {
        *pending = (0.0, 0.0);
    }
    (send.0 as i32, send.1 as i32)
}

// a new place came from the detector: the way to it, less what the mouse has already moved since the picture was taken
pub fn pending_after(step: (i32, i32), moved_since_capture: (f32, f32)) -> (f32, f32) {
    let left = (step.0 as f32 - moved_since_capture.0, step.1 as f32 - moved_since_capture.1);
    // (never turn back because of the delay: a way that has changed its sign is finished)
    (if left.0 * step.0 as f32 > 0.0 { left.0 } else { 0.0 }, if left.1 * step.1 as f32 > 0.0 { left.1 } else { 0.0 })
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
    struct Cap {
        buf: Vec<u8>,
        side: usize,
        // where the crosshair is in the square
        cross: (f32, f32),
        // the square in the window, and the size of the window
        origin: (i32, i32),
        win: (i32, i32),
    }

    // `auto`: the crosshair that was found on the pictures, pixels from the middle of the window; without it the numbers of the settings (percent)
    unsafe fn capture(want: i32, auto: Option<(i32, i32)>, cx_pct: i32, cy_pct: i32) -> Result<Cap, String> {
        use winapi::um::wingdi::*;
        use winapi::um::winuser::*;
        let (wx, wy, ww, wh) = window_area()?;
        let side = want.min(ww).min(wh) & !1;
        if side < 64 {
            return Err(format!("окно игры слишком маленькое ({}×{})", ww, wh));
        }
        // the crosshair: the middle of the window moved by the settings; the square is taken around it and kept inside the window
        let (px, py) = match auto {
            Some((ax, ay)) => (ww / 2 + ax, wh / 2 + ay),
            None => (ww / 2 + ww * cx_pct / 100, wh / 2 + wh * cy_pct / 100),
        };
        let (ox, oy) = ((px - side / 2).clamp(0, ww - side), (py - side / 2).clamp(0, wh - side));
        let cross = ((px - ox) as f32, (py - oy) as f32);
        let (x, y) = (wx + ox, wy + oy);
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
        if ok { Ok(Cap { buf, side: side as usize, cross, origin: (ox, oy), win: (ww, wh) }) } else { Err("снимок экрана не получился".into()) }
    }

    // the way to the target that is left, and what the mouse has moved in all (the detector subtracts it for the delay of its picture)
    static PENDING: Mutex<(f32, f32)> = parking_lot::const_mutex((0.0, 0.0));
    static MOVED: Mutex<(f32, f32)> = parking_lot::const_mutex((0.0, 0.0));

    // sends small moves of the mouse about 200 times a second while the right button is held; it ends with the run
    fn mover(id: u64) {
        let mut carry = (0.0f32, 0.0f32);
        while alive(id) {
            std::thread::sleep(std::time::Duration::from_millis(5));
            if !right_button_down() {
                *PENDING.lock() = (0.0, 0.0);
                carry = (0.0, 0.0);
                continue;
            }
            let (dx, dy) = {
                let mut p = PENDING.lock();
                mover_tick(&mut p, &mut carry)
            };
            if dx != 0 || dy != 0 {
                crate::gamemode::mouse_rel(dx, dy);
                let mut m = MOVED.lock();
                m.0 += dx as f32;
                m.1 += dy as f32;
            }
        }
        *PENDING.lock() = (0.0, 0.0);
    }

    static TRIGGER: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0x02);

    // the key or button of the aim (the setting «trigger») is held
    fn right_button_down() -> bool {
        unsafe { (winapi::um::winuser::GetAsyncKeyState(TRIGGER.load(Ordering::SeqCst)) as u16) & 0x8000 != 0 }
    }

    // the program in front covers the whole screen of its monitor (a game in the full screen or the window without a frame)
    pub fn front_fullscreen() -> bool {
        use winapi::um::winuser::*;
        unsafe {
            let w = GetForegroundWindow();
            if w.is_null() {
                return false;
            }
            let mut r: winapi::shared::windef::RECT = std::mem::zeroed();
            if GetWindowRect(w, &mut r) == 0 {
                return false;
            }
            let m = MonitorFromWindow(w, MONITOR_DEFAULTTONEAREST);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if m.is_null() || GetMonitorInfoW(m, &mut mi) == 0 {
                return false;
            }
            let a = mi.rcMonitor;
            r.left <= a.left + 2 && r.top <= a.top + 2 && r.right >= a.right - 2 && r.bottom >= a.bottom - 2
        }
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
        *PENDING.lock() = (0.0, 0.0);
        std::thread::spawn(move || mover(id));
        let (mut cap_fail, mut blank) = (0u32, 0u32);
        let mut tracker = crate::aimcross::CrossTracker::default();
        let mut auto_px: Option<(i32, i32)> = None;
        // the last believed place of the crosshair in this game: used while the lock is lost for a moment (the crosshair of the third person is
        // drawn only while aiming), so the mouse never goes to the middle of the window instead of the crosshair
        let mut last_good: Option<(i32, i32)> = None;
        let mut frame_no = 0u32;
        let mut last_collect = std::time::Instant::now();
        let mut last_dets: Vec<Det> = Vec::new();
        // the side that is really taken: lowered by itself when the frame is slow, so the game keeps its processor
        let mut side_now = 0u32;
        let mut avg_ms = 0.0f32;
        while alive(id) {
            let cfg = crate::gamemode::aim_config();
            if let Some(vk) = trigger_vk(&cfg.trigger) {
                TRIGGER.store(vk, Ordering::SeqCst);
            }
            // a game of the list, or (when allowed) any program in the full screen that is not a browser, a player and the like
            let game = crate::gamemode::front_game_exe().or_else(|| front_any_game(&cfg));
            let ready = crate::gamemode::is_active() && game.is_some() && crate::gamemode::anticheat_running().is_none();
            if !ready {
                set_state("waiting");
                *CAPTURE.lock() = format!("ждёт: впереди должна быть игра на весь экран (или из списка) и зажата клавиша «{}»", cfg.trigger);
                std::thread::sleep(std::time::Duration::from_millis(100));
                continue;
            }
            if let Some(g) = &game {
                if *LAST_GAME.lock() != *g {
                    *LAST_GAME.lock() = g.clone();
                    // another game: the crosshair is looked for from the start
                    tracker.reset();
                    auto_px = None;
                    last_good = None;
                }
            }
            if !right_button_down() {
                set_state("waiting");
                std::thread::sleep(std::time::Duration::from_millis(15));
                continue;
            }
            let started = std::time::Instant::now();
            let moved_before = *MOVED.lock();
            let cap = match unsafe { capture(governed_side(cfg.side, &mut side_now) as i32, auto_px.or(last_good), cfg.cx_pct, cfg.cy_pct) } {
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
            let (frame, side) = (&cap.buf, cap.side);
            if frame_is_blank(frame) {
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
            // the crosshair: looked for on the picture (cheap while it is not yet believed, rarely afterwards); its place is believed after
            // several equal frames and then the square is cut around it
            frame_no = frame_no.wrapping_add(1);
            if cfg.auto_cross {
                // a locked place is checked every few frames over the wide area, so a switch first/third person is noticed
                let verify = !tracker.is_locked() || frame_no % 4 == 0;
                let reach = 120.min(side / 2 - crate::aimcross::MARGIN.min(side / 2 - 1));
                if verify {
                    let found = crate::aimcross::find(frame, side, cap.cross, reach).map(|f| ((cap.origin.0 as f32 + f.x) - cap.win.0 as f32 / 2.0, (cap.origin.1 as f32 + f.y) - cap.win.1 as f32 / 2.0));
                    auto_px = tracker.feed(found);
                } else {
                    auto_px = tracker.current();
                }
            } else {
                tracker.reset();
                auto_px = None;
                last_good = None;
            }
            if auto_px.is_some() {
                last_good = auto_px;
            }
            *CROSS_SRC.lock() = match (auto_px, game.as_deref().and_then(game_cross)) {
                (Some((x, y)), _) => format!("найдено на экране автоматически: {:+}, {:+} пикс. от середины окна", x, y),
                (None, Some((x, y))) => format!("запомнено для игры {}: {:+}%, {:+}%", game.as_deref().unwrap_or(""), x, y),
                (None, None) if last_good.is_some() => format!("последнее найденное место (перекрестие на миг потеряно): {:+}, {:+} пикс.", last_good.unwrap().0, last_good.unwrap().1),
                (None, None) => format!("из общих настроек: {:+}%, {:+}% (на экране перекрестие из четырёх чёрточек не найдено)", cfg.cx_pct, cfg.cy_pct),
            };
            *CAPTURE.lock() = format!("ок, {}×{}", side, side);
            set_state("working");
            let res = frame_outcome(frame, side, &cfg, &info.spec, cap.cross, &mut |rgb| ortrun::run(&mut session, &info.input_name, rgb, info.spec.input_w, info.spec.input_h));
            match res {
                Ok(f) => {
                    OBJECTS.store(f.objects as u32, Ordering::SeqCst);
                    last_dets = f.dets.clone();
                    keep_snapshot(Snap { bgra: frame.clone(), side, cross: cap.cross, frame: f.clone(), auto: auto_px.is_some() });
                    // the module may have been switched off while the model worked: no move of the mouse after that
                    if !alive(id) {
                        break;
                    }
                    match f.outcome {
                        Outcome::Step(dx, dy) => {
                            // the mover makes the way in small steps; what it has moved since the picture was taken is taken off
                            let now = *MOVED.lock();
                            *PENDING.lock() = pending_after((dx, dy), (now.0 - moved_before.0, now.1 - moved_before.1));
                            if dx.abs() + dy.abs() <= LOCK_PX {
                                mark_locked();
                            }
                        }
                        Outcome::OnTarget => {
                            *PENDING.lock() = (0.0, 0.0);
                            mark_locked()
                        }
                        Outcome::NoTarget => *PENDING.lock() = (0.0, 0.0),
                    }
                }
                Err(e) => {
                    fail(e);
                    return;
                }
            }
            FRAMES.fetch_add(1, Ordering::SeqCst);
            if cfg.collect && last_collect.elapsed() >= std::time::Duration::from_secs(1) {
                last_collect = std::time::Instant::now();
                let _ = save_training_frame(frame, side, &last_dets);
            }
            let spent = started.elapsed();
            LAST_MS.store(spent.as_millis() as u64, Ordering::SeqCst);
            note_frame();
            avg_ms = if avg_ms == 0.0 { spent.as_millis() as f32 } else { avg_ms * 0.8 + spent.as_millis() as f32 * 0.2 };
            side_now = lower_side(side_now, avg_ms);
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
        let cfg = AimConfig { min_size_px: 0, max_size_pct: 100, ..AimConfig::default() };
        let side = 64usize;
        let frame = vec![0u8; side * side * 4];
        let spec = spec8(side);
        let mut seen_len = 0usize;
        let mut one = |rgb: &[f32]| -> Result<RawOut, String> {
            seen_len = rgb.len();
            // cx=20, cy=40, w=16, h=60 -> the box 12..28 × 10..70, the head at (20, 10 + 60*0.12 = 17.2)
            Ok(out8(vec![20.0, 40.0, 16.0, 60.0, 0.9], 1))
        };
        let f = frame_outcome(&frame, side, &cfg, &spec, ((side) as f32 / 2.0, (side) as f32 / 2.0), &mut one).unwrap();
        assert_eq!(seen_len, 3 * side * side, "RGB planes");
        assert_eq!(f.objects, 1);
        match f.outcome {
            Outcome::Step(dx, dy) => assert!(dx < 0 && dy < 0, "left and up: {:?}", (dx, dy)),
            o => panic!("{:?}", o),
        }
        let mut nobody = |_: &[f32]| -> Result<RawOut, String> { Ok(out8(vec![0.0; 5], 1)) };
        let f = frame_outcome(&frame, side, &cfg, &spec, ((side) as f32 / 2.0, (side) as f32 / 2.0), &mut nobody).unwrap();
        assert_eq!((f.outcome, f.objects), (Outcome::NoTarget, 0));
        // a failing detector and a frame of a wrong size are errors, not a move of the mouse
        let mut broken = |_: &[f32]| -> Result<RawOut, String> { Err("нет".into()) };
        assert!(frame_outcome(&frame, side, &cfg, &spec, ((side) as f32 / 2.0, (side) as f32 / 2.0), &mut broken).is_err());
        assert!(frame_outcome(&frame[..100], side, &cfg, &spec, ((side) as f32 / 2.0, (side) as f32 / 2.0), &mut nobody).is_err());
        // an answer of a wrong shape is an error too
        let mut wrong = |_: &[f32]| -> Result<RawOut, String> { Ok(RawOut { data: vec![0.0; 6], dims: vec![1, 5, 2] }) };
        assert!(frame_outcome(&frame, side, &cfg, &spec, ((side) as f32 / 2.0, (side) as f32 / 2.0), &mut wrong).is_err());
    }

    #[test]
    fn the_boxes_are_scaled_back_to_the_picture_when_the_model_takes_another_size() {
        // the picture is 128×128, the model takes 64×64: a box 12..28 × 10..70 of the model is 24..56 × 20..140 on the screen,
        // the head at (40, 20 + 120*0.12 = 34.4): up-left of the centre (64, 64)
        let cfg = AimConfig { min_size_px: 0, max_size_pct: 100, ..AimConfig::default() };
        let frame = vec![9u8; 128 * 128 * 4];
        let mut one = |rgb: &[f32]| -> Result<RawOut, String> {
            assert_eq!(rgb.len(), 3 * 64 * 64, "the model gets its own size");
            Ok(out8(vec![20.0, 40.0, 16.0, 60.0, 0.9], 1))
        };
        let f = frame_outcome(&frame, 128, &cfg, &spec8(64), ((128) as f32 / 2.0, (128) as f32 / 2.0), &mut one).unwrap();
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
        let cfg = AimConfig { min_size_px: 40, max_size_pct: 100, ..AimConfig::default() };
        let f = frame_outcome(&frame, 64, &cfg, &spec8(64), ((64) as f32 / 2.0, (64) as f32 / 2.0), &mut tiny).unwrap();
        assert_eq!((f.outcome, f.objects), (Outcome::NoTarget, 0), "a box 20 pixels high is below the least size");
        let cfg = AimConfig { min_size_px: 10, max_size_pct: 100, ..AimConfig::default() };
        assert_eq!(frame_outcome(&frame, 64, &cfg, &spec8(64), ((64) as f32 / 2.0, (64) as f32 / 2.0), &mut tiny).unwrap().objects, 1);
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
        frame_outcome(&frame, side, &cfg, &spec8(2), ((side) as f32 / 2.0, (side) as f32 / 2.0), &mut spy).unwrap();
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
        // slower than the budget: a quarter of the time of the frame is rest (never a busy loop)
        assert_eq!(rest_after(Duration::from_millis(100), 30), Duration::from_millis(25));
        assert!(rest_after(Duration::from_millis(1), 0) >= Duration::from_millis(16), "a nonsense frame rate is cut to 1..60");
    }

    #[test]
    fn the_mover_makes_the_way_in_small_steps_without_overshooting_and_loses_nothing() {
        let (mut pending, mut carry) = ((100.0f32, -60.0f32), (0.0f32, 0.0f32));
        let (mut sx, mut sy, mut ticks) = (0i32, 0i32, 0);
        let mut biggest = 0;
        while pending != (0.0, 0.0) && ticks < 200 {
            let (dx, dy) = mover_tick(&mut pending, &mut carry);
            biggest = biggest.max(dx.abs()).max(dy.abs());
            sx += dx;
            sy += dy;
            ticks += 1;
            assert!(sx <= 100 && sy >= -60, "never beyond the target: {:?}", (sx, sy));
        }
        assert!(ticks < 60, "it arrives in a fraction of a second at 200 ticks a second: {}", ticks);
        assert!((100 - sx).abs() <= 1 && (-60 - sy).abs() <= 1, "arrived: {:?}", (sx, sy));
        assert!(biggest <= 30, "no jump: the biggest step is {} pixels", biggest);
        // nothing to do, nothing sent
        assert_eq!(mover_tick(&mut (0.0, 0.0), &mut (0.0, 0.0)), (0, 0));
    }

    #[test]
    fn a_new_place_takes_off_what_the_mouse_moved_since_the_picture_and_never_turns_back() {
        assert_eq!(pending_after((20, -10), (0.0, 0.0)), (20.0, -10.0));
        assert_eq!(pending_after((20, -10), (8.0, -4.0)), (12.0, -6.0));
        // the mouse has already gone further than the new step: nothing is left, and it does not go back
        assert_eq!(pending_after((20, -10), (30.0, -2.0)), (0.0, -8.0));
        assert_eq!(pending_after((0, 5), (3.0, 1.0)), (0.0, 4.0));
    }

    #[test]
    fn the_crosshair_offset_and_the_biggest_person_are_checked_and_used() {
        let ok = config_from_json(&json!({ "cx": 0, "cy": -9, "max_size": 70 })).unwrap();
        assert_eq!((ok.cx_pct, ok.cy_pct, ok.max_size_pct), (0, -9, 70));
        for bad in [json!({ "cy": 60 }), json!({ "cx": -41 }), json!({ "cy": "up" }), json!({ "max_size": 5 }), json!({ "max_size": 150 })] {
            assert!(config_from_json(&bad).is_err(), "{}", bad);
        }
        // the aim goes to the crosshair that is given, not to the middle: a person right on a crosshair that is above the middle is «on target»
        let side = 64usize;
        let cfg = AimConfig { min_size_px: 0, max_size_pct: 100, strength_pct: 25, ..AimConfig::default() };
        let frame = vec![9u8; side * side * 4];
        // the box 20..44 × 8..60 (cx 32, cy 34, w 24, h 52): head at y = 8 + 52 × 0.12 = 14.2
        let mut one = |_: &[f32]| -> Result<RawOut, String> { Ok(out8(vec![32.0, 34.0, 24.0, 52.0, 0.9], 1)) };
        let from_middle = frame_outcome(&frame, side, &cfg, &spec8(side), (32.0, 32.0), &mut one).unwrap();
        assert!(matches!(from_middle.outcome, Outcome::Step(0, dy) if dy < 0), "{:?}", from_middle);
        let from_crosshair = frame_outcome(&frame, side, &cfg, &spec8(side), (32.0, 14.0), &mut one).unwrap();
        assert_eq!(from_crosshair.outcome, Outcome::OnTarget);
        // a person taller than the allowed part of the picture (the player himself at the camera) is not taken
        let big = AimConfig { max_size_pct: 65, ..cfg.clone() };
        let tall = frame_outcome(&frame, side, &big, &spec8(side), (32.0, 32.0), &mut one).unwrap();
        assert_eq!((tall.outcome, tall.objects), (Outcome::NoTarget, 0), "52 of 64 is 81% of the picture");
    }

    #[test]
    fn the_snapshot_marks_the_people_the_choice_the_way_and_the_crosshair() {
        let side = 64usize;
        let bgra = vec![10u8; side * side * 4];
        let cfg = AimConfig { min_size_px: 0, max_size_pct: 100, ..AimConfig::default() };
        let mut one = |_: &[f32]| -> Result<RawOut, String> { Ok(out8(vec![20.0, 40.0, 16.0, 40.0, 0.9], 1)) };
        let f = frame_outcome(&bgra, side, &cfg, &spec8(side), (32.0, 32.0), &mut one).unwrap();
        assert_eq!(f.dets.len(), 1);
        let (d, head) = f.picked.expect("a chosen person");
        assert_eq!((d.x1, d.y1, d.x2, d.y2), (12.0, 20.0, 28.0, 60.0));
        assert_eq!(head, (20.0, 24.8));
        let img = render_snapshot(&bgra, side, (32.0, 32.0), &f);
        assert_eq!(img.len(), side * side * 4);
        let at = |x: usize, y: usize| [img[(y * side + x) * 4], img[(y * side + x) * 4 + 1], img[(y * side + x) * 4 + 2]];
        assert_eq!(at(1, 1), [10, 10, 10], "the picture itself (grey: B, G, R are equal)");
        assert_eq!(at(32, 32), [40, 240, 255], "the crosshair is cyan");
        assert_eq!(at(20, 25), [255, 50, 50], "the head point is red");
        assert_eq!(at(12, 40), [255, 230, 40], "the chosen box is yellow");
        // the channels come in the right order: a pure blue pixel of the screen is blue in the picture
        let mut b = vec![0u8; side * side * 4];
        b[0] = 255;
        let plain = Frame { outcome: Outcome::NoTarget, objects: 0, dets: vec![], picked: None };
        let img2 = render_snapshot(&b, side, (50.0, 50.0), &plain);
        assert_eq!(&img2[0..3], &[0, 0, 255]);
        // a png can be made of it
        assert!(crate::gameicon::encode_png(side as u32, side as u32, &img).map(|p| p.starts_with(&[0x89, b'P', b'N', b'G'])).unwrap_or(false));
    }

    #[test]
    fn a_wide_box_is_a_lying_body_and_the_dead_zone_and_the_longest_step_are_settings() {
        let side = 64usize;
        let bgra = vec![10u8; side * side * 4];
        // a lying body: 40 wide, 12 high (aspect 330%)
        let mut lying = |_: &[f32]| -> Result<RawOut, String> { Ok(out8(vec![32.0, 40.0, 40.0, 12.0, 0.9], 1)) };
        let base = AimConfig { min_size_px: 0, max_size_pct: 100, ..AimConfig::default() };
        let f = frame_outcome(&bgra, side, &base, &spec8(side), (10.0, 10.0), &mut lying).unwrap();
        assert_eq!(f.outcome, Outcome::NoTarget);
        let loose = AimConfig { max_aspect_pct: 500, ..base.clone() };
        let f = frame_outcome(&bgra, side, &loose, &spec8(side), (10.0, 10.0), &mut lying).unwrap();
        assert_ne!(f.outcome, Outcome::NoTarget);
        // the dead zone and the longest step
        assert_eq!(step_dead((6.0, 0.0), 40, 40.0, 8.0), (0, 0));
        assert_ne!(step_dead((6.0, 0.0), 40, 40.0, 2.0), (0, 0));
        assert_eq!(step_dead((200.0, 0.0), 100, 12.0, 3.0), (12, 0));
        let mut c = AimConfig::default();
        assert!(apply_json(&mut c, &json!({"max_aspect": 200, "dead": 5, "max_step": 25, "collect": true})).is_ok());
        assert_eq!((c.max_aspect_pct, c.dead_px, c.max_step_px, c.collect), (200, 5, 25, true));
        assert!(apply_json(&mut c, &json!({"dead": 99})).is_err());
        assert!(apply_json(&mut c, &json!({"max_aspect": 10})).is_err());
    }

    #[test]
    fn the_key_of_the_aim_is_chosen_by_name_and_any_full_screen_game_is_allowed() {
        assert_eq!(trigger_vk("RMB"), Some(0x02));
        assert_eq!(trigger_vk(" q "), Some(0x51));
        assert_eq!(trigger_vk("5"), Some(0x35));
        assert_eq!(trigger_vk("x2"), Some(0x06));
        assert_eq!(trigger_vk("F5"), Some(0x74));
        assert_eq!(trigger_vk("shift"), Some(0x10));
        assert_eq!(trigger_vk("f13"), None);
        assert_eq!(trigger_vk("enter!"), None);
        assert_eq!(trigger_vk(""), None);
        let mut c = AimConfig::default();
        assert_eq!((c.trigger.as_str(), c.any_game), ("rmb", true));
        assert!(apply_json(&mut c, &json!({"trigger": "Q", "any_game": false})).is_ok());
        assert_eq!((c.trigger.as_str(), c.any_game), ("q", false));
        assert!(apply_json(&mut c, &json!({"trigger": "banana"})).is_err());
        assert!(is_not_a_game("Chrome.EXE") && is_not_a_game("explorer.exe") && !is_not_a_game("crimsondesert.exe"));
        // with the setting off nothing is taken (and on a computer without windows nothing either)
        assert_eq!(front_any_game(&AimConfig { any_game: false, ..AimConfig::default() }), None);
    }

    #[test]
    fn a_game_keeps_its_own_numbers_over_the_general_ones() {
        let _l = SETTINGS_LOCK.lock();
        let _ = std::fs::remove_file(settings_file());
        let profile = AimConfig::default();
        save_game_cross("shooter.exe", 0, -9).unwrap();
        assert_eq!(save_game_settings("Shooter.EXE", &json!({"strength": 70, "side": 256, "unknown": 1})).unwrap(), 2);
        let c = effective_for(&profile, Some("shooter.exe"));
        assert_eq!((c.strength_pct, c.side, c.cy_pct), (70, 256, -9), "its own numbers and the crosshair that was kept before");
        let other = effective_for(&profile, Some("other.exe"));
        assert_eq!((other.strength_pct, other.side), (40, DEFAULT_SIDE));
        assert!(save_game_settings("shooter.exe", &json!({"strength": 500})).is_err());
        assert!(save_game_settings("not a game", &json!({"strength": 50})).is_err());
        let _ = std::fs::remove_file(settings_file());
    }

    #[test]
    fn frames_for_training_are_written_with_yolo_labels_up_to_a_limit() {
        let side = 32usize;
        let bgra = vec![50u8; side * side * 4];
        let dir = dataset_dir();
        let _ = std::fs::remove_dir_all(&dir);
        let d = Det { x1: 8.0, y1: 4.0, x2: 24.0, y2: 28.0, score: 0.9 };
        assert_eq!(yolo_labels(&[d], side), "0 0.500000 0.500000 0.500000 0.750000\n");
        assert_eq!(save_training_frame(&bgra, side, &[d]), Ok(true));
        let files: Vec<String> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with(".png")) && files.iter().any(|f| f.ends_with(".txt")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slow_frame_lowers_the_side_down_to_a_floor() {
        assert_eq!(lower_side(320, 200.0), 288);
        assert_eq!(lower_side(264, 200.0), 256);
        assert_eq!(lower_side(256, 400.0), 256);
        assert_eq!(lower_side(320, 90.0), 320);
        let mut n = 0;
        assert_eq!(governed_side(320, &mut n), 320);
        n = 288;
        assert_eq!(governed_side(320, &mut n), 288);
        assert_eq!(governed_side(256, &mut n), 256);
    }

    #[test]
    fn the_report_of_the_last_frame_has_the_boxes_the_choice_and_the_step() {
        let side = 64usize;
        let bgra = vec![10u8; side * side * 4];
        let cfg = AimConfig { min_size_px: 0, max_size_pct: 100, ..AimConfig::default() };
        let mut one = |_: &[f32]| -> Result<RawOut, String> { Ok(out8(vec![20.0, 40.0, 16.0, 40.0, 0.9], 1)) };
        let f = frame_outcome(&bgra, side, &cfg, &spec8(side), (32.0, 32.0), &mut one).unwrap();
        keep_snapshot(Snap { bgra: bgra.clone(), side, cross: (32.0, 32.0), frame: f, auto: true });
        let t = snapshot_details();
        for want in ["64×64", "найдено автоматически", "человек 1: рамка (12,20)–(28,60)", "высота 40", "уверенность 90%", "ЦЕЛЬ", "целимся в точку (20, 25)", "-12, -7", "итог: шаг мыши"] {
            assert!(t.contains(want), "{}\n{}", want, t);
        }
        // no frame yet
        *SNAP.lock() = None;
        assert!(snapshot_details().contains("ещё не было"));
    }

    #[test]
    fn the_crosshair_is_kept_per_game_and_laid_over_the_general_numbers_only_for_that_game() {
        let _l = SETTINGS_LOCK.lock();
        let _ = std::fs::remove_file(settings_file());
        set_point(None);
        let base = AimConfig::default();
        save_settings(&json!({ "cy": -3 }), &base).unwrap();
        save_game_cross("Wonder.EXE", 1, -9).unwrap();
        save_game_cross("shooter.exe", 0, 0).unwrap();
        assert_eq!(game_cross("wonder.exe"), Some((1, -9)), "names are compared in small letters");
        // the game in front has its own numbers; another game and no game take the general ones
        let c = effective_for(&base, Some("wonder.exe"));
        assert_eq!((c.cx_pct, c.cy_pct), (1, -9));
        assert_eq!(effective_for(&base, Some("shooter.exe")).cy_pct, 0, "an own zero is an own number");
        assert_eq!(effective_for(&base, Some("other.exe")).cy_pct, -3);
        assert_eq!(effective_for(&base, None).cy_pct, -3);
        // the general settings that are saved later do not erase the table of the games
        save_settings(&json!({ "fps": 20 }), &base).unwrap();
        assert_eq!(game_cross("wonder.exe"), Some((1, -9)));
        // names that are not a program, paths and numbers out of range are refused
        for bad in ["", "wonder", "..\\x.exe", "a/b.exe", "c:x.exe"] {
            assert!(save_game_cross(bad, 0, 0).is_err(), "{}", bad);
        }
        assert!(save_game_cross("wonder.exe", 0, 99).is_err());
        assert_eq!(game_cross("wonder.exe"), Some((1, -9)), "a refusal changes nothing");
        // auto search can be switched off in the settings, and only with a true / false
        assert!(!save_settings(&json!({ "auto_cross": false }), &base).unwrap().auto_cross);
        assert!(save_settings(&json!({ "auto_cross": "no" }), &base).is_err());
        let _ = std::fs::remove_file(settings_file());
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
        let cfg = AimConfig { device: Device::Cpu, min_size_px: 0, side: SIDE as u32, strength_pct: 25, ..AimConfig::default() };
        // YOLOv8 fixture: a person (320, 300), 100×200; the head is at y = 200 + 200 × 0.12 = 224, the crosshair is at (320, 320):
        // 96 pixels up, a quarter of the way = 24. YOLOX fixture: the box 270..370 × 140..340, the head at y = 164: 156 up, a quarter = 39.
        for (file, want) in [("aim_dummy.onnx", -24), ("aim_yolox_dummy.onnx", -39)] {
            let path = dir.join(file);
            let info = ortrun::probe(&path, &cfg).unwrap_or_else(|e| panic!("{}: {}", file, e));
            let (mut session, _, _) = ortrun::session_for(&path, cfg.device).unwrap();
            let frame = vec![7u8; SIDE * SIDE * 4];
            let f = frame_outcome(&frame, SIDE, &cfg, &info.spec, ((SIDE) as f32 / 2.0, (SIDE) as f32 / 2.0), &mut |rgb| ortrun::run(&mut session, &info.input_name, rgb, info.spec.input_w, info.spec.input_h)).unwrap();
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

// A check against a REAL model and a REAL screenshot (not run in the normal tests: the files are not in the repository).
// AIM_REAL_MODEL = the .onnx file, AIM_REAL_BGRA = a square picture in raw BGRA, AIM_REAL_SIDE = its side.
#[cfg(all(test, feature = "ort"))]
mod real_screenshot {
    use super::*;

    #[test]
    fn a_real_model_on_a_real_screenshot_gives_the_right_places() {
        let (Ok(model), Ok(raw), Ok(side)) = (std::env::var("AIM_REAL_MODEL"), std::env::var("AIM_REAL_BGRA"), std::env::var("AIM_REAL_SIDE")) else { return };
        let side: usize = side.parse().unwrap();
        let frame = std::fs::read(raw).unwrap();
        let cfg = AimConfig { device: Device::Cpu, side: side as u32, ..AimConfig::default() };
        let info = crate::aimdetect::ortrun::probe(std::path::Path::new(&model), &cfg).unwrap();
        let (mut session, _, _) = crate::aimdetect::ortrun::session_for(std::path::Path::new(&model), cfg.device).unwrap();
        let out = crate::aimdetect::ortrun::run(&mut session, &info.input_name, &crate::aimdetect::preprocess(&frame, side, info.spec.input_w, info.spec.input_h, info.spec.layout.norm()), info.spec.input_w, info.spec.input_h).unwrap();
        let dets = crate::aimdetect::decode_output(&out, &info.spec, cfg.class, cfg.conf_pct as f32 / 100.0).unwrap();
        let dets = crate::aimdetect::scale_back(nms(dets, 0.5), side, info.spec.input_w, info.spec.input_h);
        for d in &dets {
            let head = head_of(d, cfg.head_pct);
            eprintln!("REAL box ({:.1},{:.1})-({:.1},{:.1}) score {:.2} head ({:.1},{:.1}) delta ({:.1},{:.1})", d.x1, d.y1, d.x2, d.y2, d.score, head.0, head.1, head.0 - side as f32 / 2.0, head.1 - side as f32 / 2.0);
        }
        let f = frame_outcome(&frame, side, &cfg, &info.spec, ((side) as f32 / 2.0, (side) as f32 / 2.0), &mut |rgb| crate::aimdetect::ortrun::run(&mut session, &info.input_name, rgb, info.spec.input_w, info.spec.input_h)).unwrap();
        eprintln!("REAL outcome {:?} objects {}", f.outcome, f.objects);
    }
}
