//! Voice control of a single-player game («Джарвис, включи голосовое управление для игр»).
//!
//! While the mode is on, short spoken phrases («иди вперёд», «смотри налево», «стой», «перезаряди», «стреляй») become the key
//! presses and mouse moves of a game profile. What it is and is not:
//!  * it only SENDS KEYS AND MOUSE MOVES through Windows (SendInput with scan codes, which games that read the keyboard
//!    directly also take). It never reads the memory of a game, never injects anything into it and never touches its files;
//!  * it works only while the window in front belongs to a program of the profile (single-player games: `iw3sp.exe`,
//!    `iw4sp.exe`, `iw5sp.exe` by default). In any other window the phrases do nothing and held keys are released;
//!  * «стой» releases everything at once; leaving the mode, leaving the game window and quitting release everything too;
//!  * only plain game keys are allowed in a profile (letters, digits, space, shift, ctrl, tab): no Alt, no Windows key, no Esc.
//! The phrases arrive as text, so it works the same from the keyboard, the phone remote and the microphone.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::BTreeSet;

// ---------------------------------------------------------------------------------------------
// the profile
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Keys {
    pub forward: u16,
    pub back: u16,
    pub left: u16,
    pub right: u16,
    pub jump: u16,
    pub crouch: u16,
    pub sprint: u16,
    pub reload: u16,
    pub grenade: u16,
    pub melee: u16,
    pub use_key: u16,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub title: String,
    // the program files (lower case) of the game windows where the mode works
    pub exes: Vec<String>,
    pub keys: Keys,
    // mouse counts for a quarter of a turn (90°): depends on the sensitivity of the game
    pub px_per_90: i32,
    // the experimental aim assist (aimassist.rs)
    pub aim: crate::aimassist::AimConfig,
}

impl Default for Profile {
    // the usual keys of the Call of Duty single-player campaigns; if the game has other keys, game_profile.json changes them
    fn default() -> Self {
        let v = |n: &str| crate::input::vk(n).unwrap_or(0);
        Profile {
            title: "Call of Duty (одиночная кампания)".into(),
            exes: vec!["iw3sp.exe".into(), "iw4sp.exe".into(), "iw5sp.exe".into()],
            keys: Keys { forward: v("w"), back: v("s"), left: v("a"), right: v("d"), jump: v("space"), crouch: v("c"), sprint: v("shift"), reload: v("r"), grenade: v("g"), melee: v("v"), use_key: v("f") },
            px_per_90: 450,
            aim: crate::aimassist::AimConfig::default(),
        }
    }
}

// plain game keys only: letters, digits, space, shift, ctrl, tab
pub fn allowed_key(vk: u16) -> bool {
    matches!(vk, 0x30..=0x39 | 0x41..=0x5A | 0x20 | 0x10 | 0x11 | 0x09)
}

// a profile from the optional file game_profile.json of the data folder; anything wrong in it: the default profile and the reason
pub fn profile_from_json(text: &str) -> Result<Profile, String> {
    let v: Value = serde_json::from_str(text).map_err(|_| "game_profile.json не читается (не JSON)".to_string())?;
    let mut p = Profile::default();
    if let Some(t) = v.get("title").and_then(|t| t.as_str()) {
        p.title = t.chars().take(60).collect();
    }
    if let Some(list) = v.get("exes").and_then(|e| e.as_array()) {
        let mut exes = Vec::new();
        for e in list.iter().take(10) {
            let name = e.as_str().unwrap_or("").trim().to_lowercase();
            if !name.ends_with(".exe") || name.len() < 5 || name.len() > 60 || name.contains(['\\', '/', ':']) {
                return Err(format!("в game_profile.json неверное имя программы «{}»: нужно имя файла вида game.exe", name));
            }
            exes.push(name);
        }
        if exes.is_empty() {
            return Err("в game_profile.json пустой список программ".into());
        }
        p.exes = exes;
    }
    if let Some(n) = v.get("px_per_90").and_then(|n| n.as_i64()) {
        if !(50..=3000).contains(&n) {
            return Err("px_per_90 должно быть от 50 до 3000".into());
        }
        p.px_per_90 = n as i32;
    }
    if let Some(aim) = v.get("aim") {
        p.aim = crate::aimassist::config_from_json(aim)?;
    }
    if let Some(keys) = v.get("keys").and_then(|k| k.as_object()) {
        for (slot, name) in keys {
            let code = name.as_str().and_then(crate::input::vk).filter(|c| allowed_key(*c)).ok_or_else(|| format!("в game_profile.json клавиша «{}» для «{}» не разрешена (можно буквы, цифры, space, shift, ctrl, tab)", name, slot))?;
            match slot.as_str() {
                "forward" => p.keys.forward = code,
                "back" => p.keys.back = code,
                "left" => p.keys.left = code,
                "right" => p.keys.right = code,
                "jump" => p.keys.jump = code,
                "crouch" => p.keys.crouch = code,
                "sprint" => p.keys.sprint = code,
                "reload" => p.keys.reload = code,
                "grenade" => p.keys.grenade = code,
                "melee" => p.keys.melee = code,
                "use" => p.keys.use_key = code,
                other => return Err(format!("в game_profile.json неизвестное действие «{}»", other)),
            }
        }
    }
    Ok(p)
}

// ---------------------------------------------------------------------------------------------
// the phrases
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Forward,
    Back,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tap {
    Jump,
    Crouch,
    Reload,
    Grenade,
    Melee,
    Use,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    On,
    Off,
    Help,
    // «стой»: every key and button is released
    ReleaseAll,
    // «иди вперёд»: the key is held until «стой» (the opposite one is released)
    Walk(Dir),
    // «беги»: forward and sprint
    Run,
    // degrees to the right (x) and down (y); negative = left / up
    Turn { yaw: i32, pitch: i32 },
    // a burst of fire, in milliseconds
    Fire(u64),
    CeaseFire,
    AimOn,
    AimOff,
    Tap(Tap),
    // the experimental aim assist on / off (aimassist.rs)
    AimAssist(bool),
}

fn normalize(text: &str) -> String {
    let mut s = String::new();
    for c in text.to_lowercase().chars() {
        s.push(match c {
            'ё' => 'е',
            c if c.is_alphanumeric() || c == ' ' => c,
            _ => ' ',
        });
    }
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn has(t: &str, stems: &[&str]) -> bool {
    t.split(' ').any(|w| stems.iter().any(|s| w.starts_with(s)))
}

// the number of degrees named in the phrase («на 90 градусов», «45»); None when there is none
fn degrees(t: &str) -> Option<i32> {
    let words: Vec<&str> = t.split(' ').collect();
    for (i, w) in words.iter().enumerate() {
        if let Ok(n) = w.parse::<i32>() {
            let near_unit = words.get(i + 1).map(|u| u.starts_with("градус")).unwrap_or(false);
            if near_unit || (1..=360).contains(&n) {
                return Some(n.clamp(1, 360));
            }
        }
    }
    None
}

// the phrase of the user -> the action (None: not a game phrase). The first question is whether the mode is on: switching it ON is
// understood always, everything else only while it is on.
pub fn parse(text: &str, active: bool) -> Option<Action> {
    let t = normalize(text);
    if t.is_empty() {
        return None;
    }
    let game = has(&t, &["игр"]);
    let wants_on = has(&t, &["включ", "запуст", "активир", "начн", "давай"]);
    let wants_off = has(&t, &["выключ", "отключ", "закончи", "выйди", "выход", "хватит", "заверш", "останови"]);
    if game && wants_on && !wants_off && (has(&t, &["управлен", "режим", "голосов"]) || t.contains("для игр")) {
        return Some(Action::On);
    }
    if !active {
        return None;
    }
    if (game && wants_off) || t.contains("хватит играть") || t.contains("выйди из игры") {
        return Some(Action::Off);
    }
    if has(&t, &["помощь", "подсказк"]) && game {
        return Some(Action::Help);
    }
    // stop and cease fire come before everything: they must work even inside a longer phrase
    if t.contains("прекрати огонь") || t.contains("не стреляй") || t.contains("отставить огонь") {
        return Some(Action::CeaseFire);
    }
    if has(&t, &["стой", "стоп", "остановис", "замри", "хватит", "отставить"]) {
        return Some(Action::ReleaseAll);
    }
    // the aim assist is switched by its own phrases; they come before «целься»
    if has(&t, &["помощ", "ассист", "автоприцел", "автонаведен"]) && has(&t, &["прицел", "прицелив", "ассист", "автоприцел", "автонаведен", "наведен"]) {
        if wants_off {
            return Some(Action::AimAssist(false));
        }
        if wants_on {
            return Some(Action::AimAssist(true));
        }
    }
    if t.contains("не целься") || t.contains("опусти оружие") || t.contains("отставить прицел") || t.contains("не прицеливайся") {
        return Some(Action::AimOff);
    }
    if has(&t, &["целься", "прицел", "прицелис"]) {
        return Some(Action::AimOn);
    }
    if has(&t, &["стреляй", "огонь", "пли", "выстрел", "стрельни", "пальни"]) {
        let long = has(&t, &["очеред", "долго", "непрерывн", "много"]);
        return Some(Action::Fire(if long { 1200 } else { 250 }));
    }
    // a turn of the view: a looking verb and a direction
    let look = has(&t, &["смотр", "посмотр", "поверн", "повор", "оглян", "разверн", "разворот", "глян", "взгляни"]);
    let right = has(&t, &["право", "направо", "вправо", "правее"]);
    let left = has(&t, &["лево", "налево", "влево", "левее"]);
    let up = has(&t, &["вверх", "наверх", "выше", "вверху"]);
    let down = has(&t, &["вниз", "ниже", "внизу"]);
    if look {
        if has(&t, &["разворот", "разверн", "оглян"]) || (has(&t, &["назад"]) && !left && !right && !up && !down) {
            return Some(Action::Turn { yaw: 180, pitch: 0 });
        }
        let a = degrees(&t).unwrap_or(if has(&t, &["чуть", "немного", "слегка", "капельк"]) { 15 } else if has(&t, &["сильно", "резко", "намного", "больше"]) { 60 } else { 30 });
        let (mut yaw, mut pitch) = (0, 0);
        if left {
            yaw = -a;
        }
        if right {
            yaw = a;
        }
        if up {
            pitch = -a;
        }
        if down {
            pitch = a;
        }
        if yaw != 0 || pitch != 0 {
            return Some(Action::Turn { yaw, pitch });
        }
    }
    if has(&t, &["перезаряд", "заряди", "обойм"]) {
        return Some(Action::Tap(Tap::Reload));
    }
    if has(&t, &["гранат"]) {
        return Some(Action::Tap(Tap::Grenade));
    }
    if has(&t, &["нож", "ударь", "удар", "врежь"]) {
        return Some(Action::Tap(Tap::Melee));
    }
    if has(&t, &["прыг", "перепрыг"]) {
        return Some(Action::Tap(Tap::Jump));
    }
    if has(&t, &["присяд", "пригнис", "пригнис", "сядь", "ляг"]) {
        return Some(Action::Tap(Tap::Crouch));
    }
    // «открой» alone is an ordinary command of the computer: here it is «use» only for a door, a hatch, a box
    if has(&t, &["используй", "подбери", "возьми"]) || (has(&t, &["открой", "открыть"]) && has(&t, &["двер", "люк", "ящик", "сейф"])) {
        return Some(Action::Tap(Tap::Use));
    }
    if has(&t, &["беги", "бегом", "спринт", "ускорьс"]) {
        return Some(Action::Run);
    }
    // walking: a direction word (with «иди» or alone)
    if has(&t, &["вперед", "прямо"]) {
        return Some(Action::Walk(Dir::Forward));
    }
    if has(&t, &["назад", "отступ", "пятис"]) {
        return Some(Action::Walk(Dir::Back));
    }
    if left && !right {
        return Some(Action::Walk(Dir::Left));
    }
    if right && !left {
        return Some(Action::Walk(Dir::Right));
    }
    if has(&t, &["иди", "шагай", "двигайс"]) {
        return Some(Action::Walk(Dir::Forward));
    }
    None
}

// ---------------------------------------------------------------------------------------------
// what is sent: a backend (the real Windows one, or a recording one in the tests)
// ---------------------------------------------------------------------------------------------
pub trait Backend {
    fn key(&mut self, vk: u16, down: bool);
    fn mouse_move(&mut self, dx: i32, dy: i32);
    fn button(&mut self, right: bool, down: bool);
    fn pause(&mut self, ms: u64);
}

#[derive(Default)]
pub struct Game {
    pub active: bool,
    pub profile: Profile,
    held: BTreeSet<u16>,
    left_button: bool,
    right_button: bool,
}

impl Game {
    fn press(&mut self, vk: u16, b: &mut dyn Backend) {
        if vk != 0 && self.held.insert(vk) {
            b.key(vk, true);
        }
    }
    fn release(&mut self, vk: u16, b: &mut dyn Backend) {
        if self.held.remove(&vk) {
            b.key(vk, false);
        }
    }
    pub fn release_all(&mut self, b: &mut dyn Backend) {
        for vk in std::mem::take(&mut self.held) {
            b.key(vk, false);
        }
        if std::mem::take(&mut self.left_button) {
            b.button(false, false);
        }
        if std::mem::take(&mut self.right_button) {
            b.button(true, false);
        }
    }
    pub fn anything_held(&self) -> bool {
        !self.held.is_empty() || self.left_button || self.right_button
    }

    // carries the action out; the short answer for the user (empty = no answer needed)
    pub fn apply(&mut self, a: &Action, b: &mut dyn Backend) -> String {
        let k = self.profile.keys.clone();
        match a {
            Action::On => {
                self.active = true;
                format!("Голосовое управление для игр включено ({}). Работает только в окне игры. «Стой» отпускает все клавиши, «выключи игровой режим» выходит.", self.profile.title)
            }
            Action::Off => {
                self.release_all(b);
                self.active = false;
                crate::aimassist::set_enabled(false, &self.profile.aim);
                "Игровой режим выключен, все клавиши отпущены.".into()
            }
            Action::Help => "Фразы: иди вперёд / назад / влево / вправо, беги, стой; смотри налево / направо / вверх / вниз (можно «на 45 градусов», «чуть», «разворот»); стреляй, очередью, прекрати огонь; целься, не целься; перезаряди, граната, нож, прыгай, присядь, используй. Помощь прицела (эксперимент): «включи помощь прицела» / «выключи помощь прицела».".into(),
            Action::ReleaseAll => {
                self.release_all(b);
                String::new()
            }
            Action::Walk(d) => {
                let (key, opposite) = match d {
                    Dir::Forward => (k.forward, k.back),
                    Dir::Back => (k.back, k.forward),
                    Dir::Left => (k.left, k.right),
                    Dir::Right => (k.right, k.left),
                };
                self.release(opposite, b);
                // plain walking is not sprinting
                if *d == Dir::Forward {
                    self.release(k.sprint, b);
                }
                self.press(key, b);
                String::new()
            }
            Action::Run => {
                self.release(k.back, b);
                self.press(k.forward, b);
                self.press(k.sprint, b);
                String::new()
            }
            Action::Turn { yaw, pitch } => {
                let per_degree = self.profile.px_per_90 as f32 / 90.0;
                let (dx, dy) = ((*yaw as f32 * per_degree).round() as i32, (*pitch as f32 * per_degree).round() as i32);
                // in small steps: a game that smooths its camera does not lose the move, and it looks like a hand
                let steps = (dx.abs().max(dy.abs()) / 25).clamp(1, 60);
                let (mut sx, mut sy) = (0, 0);
                for i in 1..=steps {
                    let (tx, ty) = (dx * i / steps, dy * i / steps);
                    b.mouse_move(tx - sx, ty - sy);
                    sx = tx;
                    sy = ty;
                    b.pause(6);
                }
                String::new()
            }
            Action::Fire(ms) => {
                if !self.left_button {
                    b.button(false, true);
                    b.pause(*ms);
                    b.button(false, false);
                }
                String::new()
            }
            Action::CeaseFire => {
                if std::mem::take(&mut self.left_button) {
                    b.button(false, false);
                }
                String::new()
            }
            Action::AimOn => {
                if !self.right_button {
                    self.right_button = true;
                    b.button(true, true);
                }
                String::new()
            }
            Action::AimOff => {
                if std::mem::take(&mut self.right_button) {
                    b.button(true, false);
                }
                String::new()
            }
            Action::AimAssist(on) => crate::aimassist::set_enabled(*on, &self.profile.aim),
            Action::Tap(t) => {
                let vk = match t {
                    Tap::Jump => k.jump,
                    Tap::Crouch => k.crouch,
                    Tap::Reload => k.reload,
                    Tap::Grenade => k.grenade,
                    Tap::Melee => k.melee,
                    Tap::Use => k.use_key,
                };
                if vk != 0 {
                    b.key(vk, true);
                    b.pause(40);
                    b.key(vk, false);
                }
                String::new()
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// the real thing
// ---------------------------------------------------------------------------------------------
static GAME: Lazy<Mutex<Game>> = Lazy::new(|| Mutex::new(Game { profile: Profile::default(), ..Default::default() }));

#[cfg(test)]
thread_local! {
    // the tests name the window in front themselves
    static TEST_FRONT: std::cell::RefCell<Option<Option<String>>> = std::cell::RefCell::new(None);
    static TEST_LOG: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
}

fn front_exe() -> Option<String> {
    #[cfg(test)]
    if let Some(v) = TEST_FRONT.with(|t| t.borrow().clone()) {
        return v;
    }
    crate::input::front_info().map(|(exe, _)| exe)
}

fn in_game(profile: &Profile) -> bool {
    front_exe().map(|e| profile.exes.iter().any(|x| *x == e)).unwrap_or(false)
}

#[cfg(target_os = "windows")]
struct WinBackend;

#[cfg(target_os = "windows")]
impl Backend for WinBackend {
    fn key(&mut self, vk: u16, down: bool) {
        use winapi::um::winuser::*;
        unsafe {
            let mut i: INPUT = std::mem::zeroed();
            i.type_ = INPUT_KEYBOARD;
            // the scan code: games that read the keyboard directly (DirectInput) ignore keys that carry only a virtual-key code
            let scan = MapVirtualKeyW(vk as u32, 0) as u16;
            let mut flags = KEYEVENTF_SCANCODE;
            if !down {
                flags |= KEYEVENTF_KEYUP;
            }
            *i.u.ki_mut() = KEYBDINPUT { wVk: 0, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 };
            SendInput(1, &mut i, std::mem::size_of::<INPUT>() as i32);
        }
    }
    fn mouse_move(&mut self, dx: i32, dy: i32) {
        use winapi::um::winuser::*;
        unsafe {
            let mut i: INPUT = std::mem::zeroed();
            i.type_ = INPUT_MOUSE;
            // RELATIVE movement: games that take the mouse for the camera read the changes, not the position of the cursor
            *i.u.mi_mut() = MOUSEINPUT { dx, dy, mouseData: 0, dwFlags: MOUSEEVENTF_MOVE, time: 0, dwExtraInfo: 0 };
            SendInput(1, &mut i, std::mem::size_of::<INPUT>() as i32);
        }
    }
    fn button(&mut self, right: bool, down: bool) {
        use winapi::um::winuser::*;
        unsafe {
            let mut i: INPUT = std::mem::zeroed();
            i.type_ = INPUT_MOUSE;
            let flags = match (right, down) {
                (false, true) => MOUSEEVENTF_LEFTDOWN,
                (false, false) => MOUSEEVENTF_LEFTUP,
                (true, true) => MOUSEEVENTF_RIGHTDOWN,
                (true, false) => MOUSEEVENTF_RIGHTUP,
            };
            *i.u.mi_mut() = MOUSEINPUT { dx: 0, dy: 0, mouseData: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 };
            SendInput(1, &mut i, std::mem::size_of::<INPUT>() as i32);
        }
    }
    fn pause(&mut self, ms: u64) {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
}

// where nothing can be sent (not Windows) and in the tests: it only writes down what would be sent
struct LogBackend;

impl Backend for LogBackend {
    fn key(&mut self, vk: u16, down: bool) {
        #[cfg(test)]
        TEST_LOG.with(|l| l.borrow_mut().push(format!("key {:#x} {}", vk, if down { "down" } else { "up" })));
        let _ = (vk, down);
    }
    fn mouse_move(&mut self, dx: i32, dy: i32) {
        #[cfg(test)]
        TEST_LOG.with(|l| l.borrow_mut().push(format!("move {} {}", dx, dy)));
        let _ = (dx, dy);
    }
    fn button(&mut self, right: bool, down: bool) {
        #[cfg(test)]
        TEST_LOG.with(|l| l.borrow_mut().push(format!("{} {}", if right { "right" } else { "left" }, if down { "down" } else { "up" })));
        let _ = (right, down);
    }
    fn pause(&mut self, _ms: u64) {}
}

fn with_backend<R>(f: impl FnOnce(&mut dyn Backend) -> R) -> R {
    #[cfg(all(target_os = "windows", not(test)))]
    {
        f(&mut WinBackend)
    }
    #[cfg(any(not(target_os = "windows"), test))]
    {
        f(&mut LogBackend)
    }
}

// the optional profile file of the data folder
fn load_profile() -> Result<Profile, String> {
    #[cfg(test)]
    {
        return Ok(Profile::default());
    }
    #[cfg(not(test))]
    {
        let path = crate::APP_DIR.join("game_profile.json");
        match std::fs::read_to_string(&path) {
            Ok(text) => profile_from_json(&text),
            Err(_) => Ok(Profile::default()),
        }
    }
}

// the watchdog: a key must never stay pressed when the user leaves the game window (alt-tab to Jarvis, a crash of the game)
static WATCHDOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn start_watchdog() {
    use std::sync::atomic::Ordering::SeqCst;
    if WATCHDOG.swap(true, SeqCst) {
        return;
    }
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut g = GAME.lock();
        if g.active && g.anything_held() && !in_game(&g.profile) {
            with_backend(|b| g.release_all(b));
            log::info!("Game mode: the game window is not in front, the keys are released");
        }
    });
}

// is this phrase for the game mode (so that the caller can ask for the right before it is carried out)
pub fn would_handle(text: &str) -> bool {
    parse(text, is_active()).is_some()
}

// the window in front is a game of the profile (the aim assist asks it in every frame)
pub fn in_game_now() -> bool {
    in_game(&GAME.lock().profile)
}

pub fn aim_config() -> crate::aimassist::AimConfig {
    GAME.lock().profile.aim.clone()
}

// a relative move of the mouse for the aim assist
pub fn mouse_rel(dx: i32, dy: i32) {
    with_backend(|b| b.mouse_move(dx, dy));
}

pub fn is_active() -> bool {
    GAME.lock().active
}

pub fn release_all() {
    let mut g = GAME.lock();
    with_backend(|b| g.release_all(b));
}

pub fn status_json() -> Value {
    let g = GAME.lock();
    json!({ "active": g.active, "game": g.profile.title, "exes": g.profile.exes, "in_game": g.active && in_game(&g.profile), "held": g.anything_held(), "aim": crate::aimassist::status_json(&g.profile.aim) })
}

// A phrase of the user: Some(the answer to show) when it belonged to the game mode (the caller stops there), None otherwise.
pub fn handle(text: &str) -> Option<String> {
    let active = is_active();
    let action = parse(text, active)?;
    let mut g = GAME.lock();
    match &action {
        Action::On => {
            // a bad profile file does not stop the mode: the usual keys are taken and the user is told why
            let (profile, problem) = match load_profile() {
                Ok(p) => (p, String::new()),
                Err(e) => (Profile::default(), format!(" Файл game_profile.json не применён: {}.", e)),
            };
            g.profile = profile;
            let mut reply = with_backend(|b| g.apply(&action, b));
            reply.push_str(&problem);
            drop(g);
            start_watchdog();
            Some(reply)
        }
        Action::Off | Action::Help | Action::ReleaseAll => Some(with_backend(|b| g.apply(&action, b))),
        _ => {
            // an action goes only to the window of the game
            if !in_game(&g.profile) {
                with_backend(|b| g.release_all(b));
                return Some(format!("Игра не в фокусе: откройте окно игры ({}), и я продолжу.", g.profile.exes.join(", ")));
            }
            let reply = with_backend(|b| g.apply(&action, b));
            Some(reply)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Rec(Vec<String>);
    impl Backend for Rec {
        fn key(&mut self, vk: u16, down: bool) {
            self.0.push(format!("{:#x}{}", vk, if down { "↓" } else { "↑" }));
        }
        fn mouse_move(&mut self, dx: i32, dy: i32) {
            self.0.push(format!("m{},{}", dx, dy));
        }
        fn button(&mut self, right: bool, down: bool) {
            self.0.push(format!("{}{}", if right { "R" } else { "L" }, if down { "↓" } else { "↑" }));
        }
        fn pause(&mut self, _: u64) {}
    }

    fn p(t: &str) -> Option<Action> {
        parse(t, true)
    }

    #[test]
    fn the_mode_is_switched_on_by_its_phrases_and_only_then_the_game_phrases_work() {
        for t in ["Джарвис, включи голосовое управление для игр", "включи игровой режим", "запусти голосовое управление для игры", "давай игровой режим"] {
            assert_eq!(parse(t, false), Some(Action::On), "{}", t);
        }
        // off: nothing else is a game phrase (a normal command goes on its normal way)
        assert_eq!(parse("иди вперёд", false), None);
        assert_eq!(parse("стой", false), None);
        assert_eq!(parse("открой браузер", true), None);
        for t in ["выключи игровой режим", "хватит играть", "выйди из игрового режима", "отключи голосовое управление для игр"] {
            assert_eq!(parse(t, true), Some(Action::Off), "{}", t);
        }
        assert_ne!(parse("выключи голосовое управление для игр", false), Some(Action::On));
    }

    #[test]
    fn movement_phrases() {
        assert_eq!(p("иди вперёд"), Some(Action::Walk(Dir::Forward)));
        assert_eq!(p("вперед!"), Some(Action::Walk(Dir::Forward)));
        assert_eq!(p("иди назад"), Some(Action::Walk(Dir::Back)));
        assert_eq!(p("иди влево"), Some(Action::Walk(Dir::Left)));
        assert_eq!(p("шагай направо"), Some(Action::Walk(Dir::Right)));
        assert_eq!(p("беги"), Some(Action::Run));
        assert_eq!(p("бегом вперёд"), Some(Action::Run));
        assert_eq!(p("стой"), Some(Action::ReleaseAll));
        assert_eq!(p("стоп, замри"), Some(Action::ReleaseAll));
    }

    #[test]
    fn looking_is_not_walking_and_the_angles_are_understood() {
        assert_eq!(p("смотри налево"), Some(Action::Turn { yaw: -30, pitch: 0 }));
        assert_eq!(p("посмотри направо"), Some(Action::Turn { yaw: 30, pitch: 0 }));
        assert_eq!(p("смотри вверх"), Some(Action::Turn { yaw: 0, pitch: -30 }));
        assert_eq!(p("посмотри вниз"), Some(Action::Turn { yaw: 0, pitch: 30 }));
        assert_eq!(p("поверни налево на 90 градусов"), Some(Action::Turn { yaw: -90, pitch: 0 }));
        assert_eq!(p("смотри чуть правее"), Some(Action::Turn { yaw: 15, pitch: 0 }));
        assert_eq!(p("резко поверни направо"), Some(Action::Turn { yaw: 60, pitch: 0 }));
        assert_eq!(p("разворот"), Some(Action::Turn { yaw: 180, pitch: 0 }));
        assert_eq!(p("посмотри назад"), Some(Action::Turn { yaw: 180, pitch: 0 }));
        assert_eq!(p("оглянись"), Some(Action::Turn { yaw: 180, pitch: 0 }));
    }

    #[test]
    fn fire_aim_and_the_other_actions() {
        assert_eq!(p("стреляй"), Some(Action::Fire(250)));
        assert_eq!(p("огонь очередью"), Some(Action::Fire(1200)));
        assert_eq!(p("прекрати огонь"), Some(Action::CeaseFire));
        assert_eq!(p("не стреляй"), Some(Action::CeaseFire));
        assert_eq!(p("целься"), Some(Action::AimOn));
        assert_eq!(p("опусти оружие"), Some(Action::AimOff));
        assert_eq!(p("перезаряди"), Some(Action::Tap(Tap::Reload)));
        assert_eq!(p("брось гранату"), Some(Action::Tap(Tap::Grenade)));
        assert_eq!(p("ударь ножом"), Some(Action::Tap(Tap::Melee)));
        assert_eq!(p("прыгай"), Some(Action::Tap(Tap::Jump)));
        assert_eq!(p("присядь"), Some(Action::Tap(Tap::Crouch)));
        assert_eq!(p("открой дверь"), Some(Action::Tap(Tap::Use)));
    }

    #[test]
    fn the_aim_assist_has_its_own_phrases_and_the_aiming_phrases_stay_what_they_were() {
        assert_eq!(p("включи помощь прицела"), Some(Action::AimAssist(true)));
        assert_eq!(p("включи помощь в прицеливании"), Some(Action::AimAssist(true)));
        assert_eq!(p("выключи помощь прицела"), Some(Action::AimAssist(false)));
        assert_eq!(p("отключи автоприцел"), Some(Action::AimAssist(false)));
        assert_eq!(p("целься"), Some(Action::AimOn));
        assert_eq!(p("не целься"), Some(Action::AimOff));
        // it is a phrase of the mode only
        assert_eq!(parse("включи помощь прицела", false), None);
    }

    #[test]
    fn keys_are_held_until_stop_and_the_opposite_key_is_released() {
        let mut g = Game { profile: Profile::default(), active: true, ..Default::default() };
        let (w, s, shift) = (0x57, 0x53, 0x10);
        let mut r = Rec::default();
        g.apply(&Action::Walk(Dir::Forward), &mut r);
        g.apply(&Action::Walk(Dir::Forward), &mut r);
        assert_eq!(r.0, vec![format!("{:#x}↓", w)], "a second «вперёд» presses nothing new");
        g.apply(&Action::Run, &mut r);
        assert!(g.held.contains(&shift) && g.held.contains(&w));
        g.apply(&Action::Walk(Dir::Back), &mut r);
        assert!(!g.held.contains(&w) && g.held.contains(&s), "back releases forward");
        g.apply(&Action::AimOn, &mut r);
        assert!(g.anything_held());
        g.apply(&Action::ReleaseAll, &mut r);
        assert!(!g.anything_held(), "«стой» releases keys and buttons");
        assert!(r.0.iter().any(|e| e == "R↑"));
        // every press has its release
        let downs = r.0.iter().filter(|e| e.ends_with('↓')).count();
        let ups = r.0.iter().filter(|e| e.ends_with('↑')).count();
        assert_eq!(downs, ups);
    }

    #[test]
    fn walking_forward_stops_the_sprint() {
        let mut g = Game { profile: Profile::default(), active: true, ..Default::default() };
        let mut r = Rec::default();
        g.apply(&Action::Run, &mut r);
        g.apply(&Action::Walk(Dir::Forward), &mut r);
        assert!(!g.held.contains(&0x10), "plain walking is not sprinting");
        assert!(g.held.contains(&0x57));
    }

    #[test]
    fn a_turn_moves_the_mouse_by_the_asked_amount_in_small_steps() {
        let mut g = Game { profile: Profile::default(), active: true, ..Default::default() };
        let mut r = Rec::default();
        g.apply(&Action::Turn { yaw: -90, pitch: 0 }, &mut r);
        let moves: Vec<(i32, i32)> = r.0.iter().filter_map(|e| e.strip_prefix('m')).map(|e| { let (a, b) = e.split_once(',').unwrap(); (a.parse().unwrap(), b.parse().unwrap()) }).collect();
        assert!(moves.len() > 5, "small steps: {}", moves.len());
        assert_eq!(moves.iter().map(|m| m.0).sum::<i32>(), -450, "90° = px_per_90 counts to the left");
        assert_eq!(moves.iter().map(|m| m.1).sum::<i32>(), 0);
        assert!(moves.iter().all(|m| m.0.abs() <= 25));
    }

    #[test]
    fn a_profile_file_changes_keys_but_only_to_plain_game_keys() {
        let ok = profile_from_json(r#"{"title":"Моя игра","exes":["Game.EXE"],"px_per_90":300,"keys":{"forward":"up","jump":"space","crouch":"ctrl"}}"#);
        // «up» (an arrow) is not a plain game key
        assert!(ok.is_err());
        let ok = profile_from_json(r#"{"title":"Моя игра","exes":["Game.EXE"],"px_per_90":300,"keys":{"forward":"i","crouch":"ctrl"}}"#).unwrap();
        assert_eq!((ok.exes, ok.px_per_90, ok.keys.forward, ok.keys.crouch), (vec!["game.exe".to_string()], 300, 0x49, 0x11));
        for bad in [
            r#"{"keys":{"jump":"alt"}}"#,
            r#"{"keys":{"jump":"win"}}"#,
            r#"{"keys":{"jump":"esc"}}"#,
            r#"{"keys":{"jump":"f4"}}"#,
            r#"{"keys":{"unknown":"w"}}"#,
            r#"{"exes":["C:\\Windows\\cmd.exe"]}"#,
            r#"{"exes":["notepad"]}"#,
            r#"{"exes":[]}"#,
            r#"{"px_per_90":5}"#,
            "not json",
        ] {
            assert!(profile_from_json(bad).is_err(), "{}", bad);
        }
        assert_eq!(profile_from_json("{}").unwrap(), Profile::default());
    }

    #[test]
    fn the_default_profile_is_the_single_player_games_with_plain_keys() {
        let d = Profile::default();
        assert!(d.exes.iter().all(|e| e.ends_with("sp.exe")), "single-player programs only: {:?}", d.exes);
        for k in [d.keys.forward, d.keys.back, d.keys.left, d.keys.right, d.keys.jump, d.keys.crouch, d.keys.sprint, d.keys.reload, d.keys.grenade, d.keys.melee, d.keys.use_key] {
            assert!(allowed_key(k), "{:#x}", k);
        }
    }

    #[test]
    fn actions_reach_only_the_window_of_the_game_and_the_keys_are_released_outside_it() {
        // the global state: this test is the only one that uses it
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("notepad.exe".into())));
        TEST_LOG.with(|l| l.borrow_mut().clear());
        assert!(handle("включи голосовое управление для игр").unwrap().contains("включено"));
        assert!(is_active());
        // another window in front: nothing is sent
        let m = handle("иди вперёд").unwrap();
        assert!(m.contains("не в фокусе"), "{}", m);
        assert!(TEST_LOG.with(|l| l.borrow().iter().all(|e| !(e.starts_with("key") && e.ends_with("down")))), "no key went down outside the game");
        assert!(!GAME.lock().anything_held());
        // the game in front: it goes
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("iw4sp.exe".into())));
        handle("иди вперёд");
        assert!(GAME.lock().anything_held());
        // the user leaves the window: «стой» (and the watchdog) release; a phrase of another window releases at once
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("chrome.exe".into())));
        handle("беги");
        assert!(!GAME.lock().anything_held(), "outside the game everything is released");
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("iw4sp.exe".into())));
        handle("беги");
        assert!(handle("стой").is_some());
        assert!(!GAME.lock().anything_held());
        assert!(handle("выключи игровой режим").unwrap().contains("выключен"));
        assert!(!is_active());
        assert_eq!(handle("иди вперёд"), None, "off: a normal phrase");
        TEST_FRONT.with(|t| *t.borrow_mut() = None);
    }
}
