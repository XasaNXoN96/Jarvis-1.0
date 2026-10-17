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
    // racing
    pub handbrake: u16,
    pub nitro: u16,
    pub horn: u16,
    pub camera: u16,
    // action / adventure
    pub dodge: u16,
    pub inventory: u16,
    pub map: u16,
    pub heal: u16,
}

// the kind of a game: the phrases that are understood besides the common ones (walk, look, jump, stop ...)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Genre {
    // shooters (the default, and the one that knows the most phrases)
    #[default]
    Shooter,
    // racing: gas, brake, steering, handbrake, nitro, horn, camera
    Racing,
    // action / adventure / RPG / platformers: attack, block, dodge, inventory, map, heal
    Action,
}

impl Genre {
    pub fn from_name(n: &str) -> Option<Genre> {
        match n.trim().to_lowercase().as_str() {
            "shooter" | "fps" => Some(Genre::Shooter),
            "racing" | "race" => Some(Genre::Racing),
            "action" | "adventure" | "rpg" => Some(Genre::Action),
            _ => None,
        }
    }
    pub fn name(self) -> &'static str {
        match self {
            Genre::Shooter => "shooter",
            Genre::Racing => "racing",
            Genre::Action => "action",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Genre::Shooter => "шутер",
            Genre::Racing => "гонки",
            Genre::Action => "экшен / приключения",
        }
    }
}

// a step of a phrase of the user's own («газуй»: hold w, wait, tap space ...): any game, any genre
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Step {
    Tap(u16),
    Hold(u16),
    Release(u16),
    Click(bool),
    Wait(u64),
    Move(i32, i32),
}

// "tap:space" | "hold:w" | "release:w" | "click:left" | "click:right" | "wait:300" | "move:30,0"
pub fn parse_step(text: &str) -> Result<Step, String> {
    let (kind, arg) = text.split_once(':').ok_or_else(|| format!("шаг «{}» неверный: нужно вид:значение, например tap:space", text))?;
    let key = |a: &str| crate::input::vk(a).filter(|c| allowed_key(*c)).ok_or_else(|| format!("клавиша «{}» не разрешена (можно буквы, цифры, space, shift, ctrl, tab)", a));
    match kind.trim() {
        "tap" => Ok(Step::Tap(key(arg.trim())?)),
        "hold" => Ok(Step::Hold(key(arg.trim())?)),
        "release" => Ok(Step::Release(key(arg.trim())?)),
        "click" => match arg.trim() {
            "left" => Ok(Step::Click(false)),
            "right" => Ok(Step::Click(true)),
            _ => Err("click: left или right".into()),
        },
        "wait" => arg.trim().parse::<u64>().ok().filter(|n| (1..=2000).contains(n)).map(Step::Wait).ok_or_else(|| "wait: от 1 до 2000 миллисекунд".to_string()),
        "move" => {
            let (a, b) = arg.split_once(',').ok_or("move: dx,dy")?;
            let (dx, dy) = (a.trim().parse::<i32>().map_err(|_| "move: dx,dy числа")?, b.trim().parse::<i32>().map_err(|_| "move: dx,dy числа")?);
            if dx.abs() > 1500 || dy.abs() > 1500 {
                return Err("move: не больше 1500 по каждой оси".into());
            }
            Ok(Step::Move(dx, dy))
        }
        other => Err(format!("шаг «{}» неизвестен (tap, hold, release, click, wait, move)", other)),
    }
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
    // games the user added by voice («добавь эту игру»): they take the same keys unless `games` says otherwise
    pub extra: Vec<String>,
    // other keys / sensitivity for one program
    pub games: std::collections::BTreeMap<String, GameOverride>,
    // the kind of the game (the phrases beyond the common ones)
    pub genre: Genre,
    // the kinds of the games the user added («добавь эту игру как гонку»)
    pub genres: std::collections::BTreeMap<String, Genre>,
    // the user's own phrases (any genre): the words of the phrase -> steps
    pub phrases: Vec<(String, Vec<Step>)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameOverride {
    pub keys: Keys,
    pub px_per_90: i32,
    pub genre: Option<Genre>,
    pub phrases: Vec<(String, Vec<Step>)>,
}

impl Default for Profile {
    // the usual keys of the Call of Duty single-player campaigns; if the game has other keys, game_profile.json changes them
    fn default() -> Self {
        let v = |n: &str| crate::input::vk(n).unwrap_or(0);
        Profile {
            title: "Call of Duty (одиночная кампания)".into(),
            exes: vec!["iw3sp.exe".into(), "iw4sp.exe".into(), "iw5sp.exe".into()],
            keys: Keys { forward: v("w"), back: v("s"), left: v("a"), right: v("d"), jump: v("space"), crouch: v("c"), sprint: v("shift"), reload: v("r"), grenade: v("g"), melee: v("v"), use_key: v("f"),
                handbrake: v("space"), nitro: v("shift"), horn: v("h"), camera: v("c"), dodge: v("ctrl"), inventory: v("i"), map: v("m"), heal: v("x") },
            px_per_90: 450,
            aim: crate::aimassist::AimConfig::default(),
            extra: Vec::new(),
            games: Default::default(),
            genre: Genre::Shooter,
            genres: Default::default(),
            phrases: Vec::new(),
        }
    }
}

// plain game keys only: letters, digits, space, shift, ctrl, tab
pub fn allowed_key(vk: u16) -> bool {
    matches!(vk, 0x30..=0x39 | 0x41..=0x5A | 0x20 | 0x10 | 0x11 | 0x09)
}

// the words of a phrase of the user (or the name of a macro) as they are compared: not empty, not long, and never the words that stop
// everything or switch the mode off (a phrase of the user must not take their place)
pub(crate) fn valid_phrase_words(phrase: &str) -> Result<String, String> {
    let words = normalize(phrase);
    if words.is_empty() || words.chars().count() > 40 {
        return Err(format!("фраза «{}» пустая или длиннее 40 знаков", phrase));
    }
    if has(&words, &["стой", "стоп", "остановис", "остановк", "замри", "выключ", "отключ", "хватит", "макрос", "запис", "сохран"]) {
        return Err(format!("фраза «{}» не может содержать слова остановки, выключения и записи", phrase));
    }
    Ok(words)
}

// "phrases": { "газуй": ["hold:w", "wait:300", "tap:space"] }
fn phrases_from_json(v: &Value) -> Result<Vec<(String, Vec<Step>)>, String> {
    let map = v.as_object().ok_or("phrases должно быть объектом: «фраза»: [шаги]")?;
    let mut out = Vec::new();
    for (phrase, steps) in map.iter().take(60) {
        let words = valid_phrase_words(phrase)?;
        let list = steps.as_array().filter(|a| !a.is_empty() && a.len() <= 30).ok_or_else(|| format!("для «{}» нужен список из 1–30 шагов", phrase))?;
        let mut parsed = Vec::new();
        for st in list {
            parsed.push(parse_step(st.as_str().unwrap_or("")).map_err(|e| format!("«{}»: {}", phrase, e))?);
        }
        out.push((words, parsed));
    }
    Ok(out)
}

fn valid_exe_name(name: &str) -> bool {
    name.ends_with(".exe") && name.len() >= 5 && name.len() <= 60 && !name.contains(['\\', '/', ':'])
}

fn apply_keys(keys: &mut Keys, map: &serde_json::Map<String, Value>) -> Result<(), String> {
    for (slot, name) in map {
        let code = name.as_str().and_then(crate::input::vk).filter(|c| allowed_key(*c)).ok_or_else(|| format!("в game_profile.json клавиша «{}» для «{}» не разрешена (можно буквы, цифры, space, shift, ctrl, tab)", name, slot))?;
        match slot.as_str() {
            "forward" => keys.forward = code,
            "back" => keys.back = code,
            "left" => keys.left = code,
            "right" => keys.right = code,
            "jump" => keys.jump = code,
            "crouch" => keys.crouch = code,
            "sprint" => keys.sprint = code,
            "reload" => keys.reload = code,
            "grenade" => keys.grenade = code,
            "melee" => keys.melee = code,
            "use" => keys.use_key = code,
            "handbrake" => keys.handbrake = code,
            "nitro" => keys.nitro = code,
            "horn" => keys.horn = code,
            "camera" => keys.camera = code,
            "dodge" => keys.dodge = code,
            "inventory" => keys.inventory = code,
            "map" => keys.map = code,
            "heal" => keys.heal = code,
            other => return Err(format!("в game_profile.json неизвестное действие «{}»", other)),
        }
    }
    Ok(())
}

impl Profile {
    // the programs where the mode works: the profile's own and the ones the user added
    pub fn all_exes(&self) -> Vec<String> {
        let mut v = self.exes.clone();
        for e in &self.extra {
            if !v.contains(e) {
                v.push(e.clone());
            }
        }
        v
    }

    // the profile for the window in front (its own keys and sensitivity when `games` names it)
    pub fn effective(&self, exe: &str) -> Profile {
        let mut p = self.clone();
        let mut own_genre = None;
        if let Some(o) = self.games.get(exe) {
            p.keys = o.keys.clone();
            p.px_per_90 = o.px_per_90;
            own_genre = o.genre;
            // the phrases of the game come first, then the common ones
            let mut list = o.phrases.clone();
            list.extend(self.phrases.iter().cloned());
            p.phrases = list;
        }
        if let Some(g) = own_genre.or_else(|| self.genres.get(exe).copied()) {
            p.genre = g;
        }
        p
    }
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
        apply_keys(&mut p.keys, keys)?;
    }
    if let Some(g) = v.get("genre").and_then(|g| g.as_str()) {
        p.genre = Genre::from_name(g).ok_or_else(|| format!("в game_profile.json неизвестный жанр «{}» (shooter, racing, action)", g))?;
    }
    if let Some(ph) = v.get("phrases") {
        p.phrases = phrases_from_json(ph)?;
    }
    // other keys / sensitivity for one program: "games": { "mygame.exe": { "px_per_90": 300, "keys": { "forward": "w" } } }
    if let Some(games) = v.get("games").and_then(|g| g.as_object()) {
        for (exe, spec) in games.iter().take(30) {
            let exe = exe.trim().to_lowercase();
            if !valid_exe_name(&exe) {
                return Err(format!("в game_profile.json неверное имя программы «{}» в games", exe));
            }
            let mut o = GameOverride { keys: p.keys.clone(), px_per_90: p.px_per_90, genre: None, phrases: Vec::new() };
            if let Some(g) = spec.get("genre").and_then(|g| g.as_str()) {
                o.genre = Some(Genre::from_name(g).ok_or_else(|| format!("в game_profile.json неизвестный жанр «{}» (shooter, racing, action)", g))?);
            }
            if let Some(ph) = spec.get("phrases") {
                o.phrases = phrases_from_json(ph)?;
            }
            if let Some(n) = spec.get("px_per_90").and_then(|n| n.as_i64()) {
                if !(50..=3000).contains(&n) {
                    return Err(format!("px_per_90 для {} должно быть от 50 до 3000", exe));
                }
                o.px_per_90 = n as i32;
            }
            if let Some(keys) = spec.get("keys").and_then(|k| k.as_object()) {
                apply_keys(&mut o.keys, keys)?;
            }
            p.games.insert(exe, o);
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
    Handbrake,
    Nitro,
    Horn,
    Camera,
    Dodge,
    Inventory,
    Map,
    Heal,
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
    // «добавь эту игру» / «забудь эту игру»: the program of the window in front joins the list of games (or leaves it)
    // the kind of the game may be named: «как гонку», «как экшен»
    AddGame(Option<Genre>),
    ForgetGame,
    // the steps of a phrase of the user's own
    Custom(Vec<Step>),
    // racing: the wheel straight (both steering keys are released)
    Straight,
    // macros: «запиши макрос комбо» … «сохрани макрос»; «удали макрос комбо»
    MacroStart(String),
    MacroStop,
    MacroDelete(String),
    // a question about the screen of the game: a hint / the text read aloud (and translated)
    Hint,
    ReadText(bool),
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

// «как гонку», «как экшен»: the kind of the game named in a phrase
fn genre_named(t: &str) -> Option<Genre> {
    if has(t, &["гонк", "гоночн", "автосим", "машин"]) {
        Some(Genre::Racing)
    } else if has(t, &["экшен", "экшн", "приключен", "адвенчур", "рпг", "платформер", "action"]) {
        Some(Genre::Action)
    } else if has(t, &["шутер", "стрелялк", "фпс", "fps"]) {
        Some(Genre::Shooter)
    } else {
        None
    }
}

// the phrases of a kind of game (besides the common ones of `parse`); they come first, because some words mean other things there:
// «газ» is walking forward, «прямо» in racing is a straight wheel, «блок» in action is the right button held
pub fn genre_parse(text: &str, genre: Genre) -> Option<Action> {
    let t = normalize(text);
    match genre {
        Genre::Shooter => None,
        Genre::Racing => {
            if has(&t, &["газ", "гони", "поехали", "разгоняйс", "жми"]) {
                Some(Action::Walk(Dir::Forward))
            } else if has(&t, &["тормоз", "притормоз"]) || t.contains("задний ход") {
                Some(Action::Walk(Dir::Back))
            } else if has(&t, &["прямо", "выровняй"]) || t.contains("не поворачивай") {
                Some(Action::Straight)
            } else if has(&t, &["ручник", "дрифт", "занос"]) {
                Some(Action::Tap(Tap::Handbrake))
            } else if has(&t, &["нитро", "турбо", "закис", "ускорени"]) {
                Some(Action::Tap(Tap::Nitro))
            } else if has(&t, &["сигнал", "бибикни", "посигналь", "гудок"]) {
                Some(Action::Tap(Tap::Horn))
            } else if has(&t, &["камер"]) {
                Some(Action::Tap(Tap::Camera))
            } else {
                None
            }
        }
        Genre::Action => {
            if t.contains("сними блок") || t.contains("опусти блок") || t.contains("отпусти блок") || t.contains("не блокируй") {
                Some(Action::AimOff)
            } else if has(&t, &["блок", "защит", "закройся"]) {
                Some(Action::AimOn)
            } else if t.contains("тяжел") || t.contains("сильный удар") || t.contains("мощный удар") {
                Some(Action::Fire(700))
            } else if has(&t, &["атакуй", "бей", "руби", "режь", "ударь", "удар"]) {
                Some(Action::Fire(150))
            } else if has(&t, &["перекат", "уворот", "увернис", "уклонис", "кувырок"]) {
                Some(Action::Tap(Tap::Dodge))
            } else if has(&t, &["инвентар", "рюкзак", "сумк"]) {
                Some(Action::Tap(Tap::Inventory))
            } else if has(&t, &["карту", "карта", "карты"]) {
                Some(Action::Tap(Tap::Map))
            } else if has(&t, &["лечис", "аптечк", "хил", "исцел", "полечис"]) {
                Some(Action::Tap(Tap::Heal))
            } else if has(&t, &["садис", "вылезай", "вылези", "залезай", "залезь", "лезь"]) {
                Some(Action::Tap(Tap::Use))
            } else {
                None
            }
        }
    }
}

// the user's own phrase: its words stand in the sentence as whole words
pub fn custom_match(text: &str, phrases: &[(String, Vec<Step>)]) -> Option<Action> {
    let padded = format!(" {} ", normalize(text));
    phrases.iter().find(|(words, _)| padded.contains(&format!(" {} ", words))).map(|(_, steps)| Action::Custom(steps.clone()))
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
    // the list of games: the window in front joins it (or leaves it)
    if game && has(&t, &["эту", "этот", "эта", "текущ", "сейчас"]) {
        if has(&t, &["добав", "запомн", "внеси", "занеси"]) {
            return Some(Action::AddGame(genre_named(&t)));
        }
        if has(&t, &["забуд", "удали", "убери", "исключи"]) {
            return Some(Action::ForgetGame);
        }
    }
    if has(&t, &["помощь", "подсказк"]) && game {
        return Some(Action::Help);
    }
    // macros: «запиши макрос комбо», «сохрани макрос», «удали макрос комбо»
    if has(&t, &["макрос"]) || t.contains("останови запись") || t.contains("закончи запись") || t.contains("хватит записывать") || t.contains("сохрани запись") {
        let name = t.split(' ').skip_while(|w| !w.starts_with("макрос")).skip(1).collect::<Vec<_>>().join(" ");
        if has(&t, &["удали", "удалить", "забудь"]) && has(&t, &["макрос"]) {
            return Some(Action::MacroDelete(name));
        }
        if has(&t, &["сохран", "останов", "законч", "хватит", "стоп"]) && (has(&t, &["макрос"]) || has(&t, &["запис"])) {
            return Some(Action::MacroStop);
        }
        if has(&t, &["запиш", "записыв", "начни", "начать"]) && has(&t, &["макрос"]) {
            return Some(Action::MacroStart(name));
        }
    }
    // a question about the screen of the game
    if has(&t, &["прочитай", "прочти", "перескажи", "озвучь"]) && has(&t, &["текст", "субтитр", "диалог", "задани", "надпис", "письм"]) {
        return Some(Action::ReadText(false));
    }
    if has(&t, &["переведи", "перевод"]) && has(&t, &["текст", "субтитр", "диалог", "задани", "надпис"]) {
        return Some(Action::ReadText(true));
    }
    if has(&t, &["подскаж", "подсказк"]) || t.contains("что делать") || t.contains("куда идти") || t.contains("как пройти") || t.contains("куда мне") {
        return Some(Action::Hint);
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
    // asked before every step of a phrase with several steps (a macro): false = the game is not in front any more, an anti-cheat program
    // started, or the user pressed Esc; the rest of the steps is not sent and the keys are released
    fn may_continue(&mut self, _profile: &Profile) -> bool {
        true
    }
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
            Action::Help => "Фразы: иди вперёд / назад / влево / вправо, беги, стой; смотри налево / направо / вверх / вниз (можно «на 45 градусов», «чуть», «разворот»); стреляй, очередью, прекрати огонь; целься, не целься; перезаряди, граната, нож, прыгай, присядь, используй. Помощь прицела (эксперимент): «включи помощь прицела» / «выключи помощь прицела». Своя игра: «добавь эту игру» (можно «как гонку» или «как экшен»). Гонки: газ, тормоз, прямо, ручник, нитро, сигнал. Экшен: атакуй, блок, перекат, инвентарь, карта, полечись.".into(),
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
            // done by `handle` (it needs the window in front and the file of the list)
            Action::AddGame(_) | Action::ForgetGame | Action::MacroStart(_) | Action::MacroStop | Action::MacroDelete(_) | Action::Hint | Action::ReadText(_) => String::new(),
            Action::Straight => {
                self.release(k.left, b);
                self.release(k.right, b);
                String::new()
            }
            Action::Custom(steps) => {
                for st in steps {
                    if !b.may_continue(&self.profile) {
                        self.release_all(b);
                        log::info!("Game mode: a phrase with several steps was stopped (the game left the front, an anti-cheat program, or Esc)");
                        break;
                    }
                    match st {
                        Step::Tap(vk) => {
                            b.key(*vk, true);
                            b.pause(40);
                            b.key(*vk, false);
                        }
                        Step::Hold(vk) => self.press(*vk, b),
                        Step::Release(vk) => self.release(*vk, b),
                        Step::Click(right) => {
                            b.button(*right, true);
                            b.pause(60);
                            b.button(*right, false);
                        }
                        Step::Wait(ms) => b.pause(*ms),
                        Step::Move(dx, dy) => {
                            let n = (dx.abs().max(dy.abs()) / 25).clamp(1, 60);
                            let (mut sx, mut sy) = (0, 0);
                            for i in 1..=n {
                                let (tx, ty) = (dx * i / n, dy * i / n);
                                b.mouse_move(tx - sx, ty - sy);
                                sx = tx;
                                sy = ty;
                                b.pause(6);
                            }
                        }
                    }
                }
                String::new()
            }
            Action::Tap(t) => {
                let vk = match t {
                    Tap::Jump => k.jump,
                    Tap::Crouch => k.crouch,
                    Tap::Reload => k.reload,
                    Tap::Grenade => k.grenade,
                    Tap::Melee => k.melee,
                    Tap::Use => k.use_key,
                    Tap::Handbrake => k.handbrake,
                    Tap::Nitro => k.nitro,
                    Tap::Horn => k.horn,
                    Tap::Camera => k.camera,
                    Tap::Dodge => k.dodge,
                    Tap::Inventory => k.inventory,
                    Tap::Map => k.map,
                    Tap::Heal => k.heal,
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
    front_exe().map(|e| profile.all_exes().iter().any(|x| *x == e)).unwrap_or(false)
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
    fn may_continue(&mut self, profile: &Profile) -> bool {
        use winapi::um::winuser::{GetAsyncKeyState, VK_ESCAPE};
        let esc = unsafe { (GetAsyncKeyState(VK_ESCAPE) as u16) & 0x8000 != 0 };
        !esc && in_game(profile) && anticheat_running().is_none()
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

// ---------------------------------------------------------------------------------------------
// which programs may be added, and the anti-cheat guard
// ---------------------------------------------------------------------------------------------
// well-known ONLINE games: never added (the mode is for single-player games)
const ONLINE_GAMES: &[&str] = &[
    "valorant.exe", "valorant-win64-shipping.exe", "cs2.exe", "csgo.exe", "fortniteclient-win64-shipping.exe", "r5apex.exe", "overwatch.exe",
    "rainbowsix.exe", "rainbowsix_vulkan.exe", "tslgame.exe", "dota2.exe", "leagueoflegends.exe", "league of legends.exe", "escapefromtarkov.exe",
    "destiny2.exe", "paladins.exe", "apexlegends.exe", "warzone.exe", "hunt.exe", "deadbydaylight-win64-shipping.exe", "pubg.exe",
];
// programs that are not games: keys and mouse moves are never sent to them
const NOT_GAMES: &[&str] = &[
    "explorer.exe", "cmd.exe", "powershell.exe", "pwsh.exe", "windowsterminal.exe", "wt.exe", "regedit.exe", "taskmgr.exe", "mmc.exe", "chrome.exe",
    "msedge.exe", "firefox.exe", "browser.exe", "opera.exe", "brave.exe", "vivaldi.exe", "code.exe", "devenv.exe", "notepad.exe", "winword.exe",
    "excel.exe", "outlook.exe", "powerpnt.exe", "telegram.exe", "discord.exe", "steam.exe", "jarvis-app.exe", "jarvis.exe", "applicationframehost.exe",
    "systemsettings.exe", "searchhost.exe", "shellexperiencehost.exe", "lockapp.exe", "mstsc.exe", "teamviewer.exe", "anydesk.exe",
];
// the helper programs of anti-cheat systems: while one of them runs, the game mode does not start (and stops)
const ANTICHEAT: &[&str] = &["easyanticheat.exe", "easyanticheat_eos.exe", "beservice.exe", "bedaisy.exe", "vgc.exe", "faceitclient.exe", "faceit.exe", "esea.exe", "xigncode.exe", "gameguard.exe", "npggnt.exe", "punkbuster.exe", "pbsvc.exe"];

// why this program must not be added (None: it may be)
pub fn refuse_reason(exe: &str) -> Option<String> {
    if ONLINE_GAMES.contains(&exe) {
        return Some(format!("{} — сетевая игра: игровой режим только для одиночных игр", exe));
    }
    if NOT_GAMES.contains(&exe) {
        return Some(format!("{} — не игра: в неё я клавиши и мышь не отправляю", exe));
    }
    if !valid_exe_name(exe) {
        return Some("не удалось определить программу игры".into());
    }
    None
}

#[cfg(test)]
thread_local! {
    static TEST_PROCS: std::cell::RefCell<Option<Vec<String>>> = std::cell::RefCell::new(None);
}

// the names (lower case) of the running programs
fn running_programs() -> Vec<String> {
    #[cfg(test)]
    if let Some(v) = TEST_PROCS.with(|t| t.borrow().clone()) {
        return v;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
        use winapi::um::tlhelp32::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
        let mut out = Vec::new();
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                out.push(String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase());
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        out
    }
    #[cfg(not(target_os = "windows"))]
    Vec::new()
}

// the name of a running anti-cheat helper, if there is one (looked up at most every three seconds)
pub fn anticheat_running() -> Option<String> {
    static CACHE: Lazy<Mutex<Option<(std::time::Instant, Option<String>)>>> = Lazy::new(|| Mutex::new(None));
    #[cfg(not(test))]
    {
        let g = CACHE.lock();
        if let Some((t, r)) = g.as_ref() {
            if t.elapsed() < std::time::Duration::from_secs(3) {
                return r.clone();
            }
        }
    }
    let found = running_programs().into_iter().find(|p| ANTICHEAT.contains(&p.as_str()));
    #[cfg(not(test))]
    {
        *CACHE.lock() = Some((std::time::Instant::now(), found.clone()));
    }
    #[cfg(test)]
    let _ = &CACHE;
    found
}

// the list of games (gamelist.rs) -> the profile: the programs and the kinds of the games the user has
fn apply_list(p: &mut Profile) {
    let list = crate::gamelist::load();
    p.extra = list.iter().map(|e| e.exe.clone()).collect();
    p.genres = list.iter().map(|e| (e.exe.clone(), e.genre)).collect();
}

// the Games page changed the list: the mode that runs takes it at once
pub fn reload_list() {
    let mut g = GAME.lock();
    apply_list(&mut g.profile);
}

// the questions about the screen of a game
pub fn hint_question(game: &str) -> String {
    format!("Я играю в одиночную игру ({}). Посмотри на снимок экрана и коротко, в два-три предложения, подскажи, что можно сделать дальше или как пройти эту ситуацию. Не придумывай то, чего не видно на картинке.", game)
}

pub fn read_question(translate: bool) -> String {
    if translate {
        "Найди на снимке экрана игры текст (диалог, субтитры, задание, подсказку, надпись) и переведи его на русский. Ответь только переводом, без пояснений. Если текста не видно, скажи об этом.".to_string()
    } else {
        "Найди на снимке экрана игры текст (диалог, субтитры, задание, подсказку, надпись) и перескажи его по-русски коротко и точно. Если текст уже на русском, просто прочитай его. Если текста не видно, скажи об этом.".to_string()
    }
}

// the question goes to the AI with a picture of the screen (only when «Анализ экрана» is allowed in the settings); the answer is shown in
// the conversation and said aloud. In the background: the game must not wait for it.
fn ask_screen(question: String, shown: String) {
    std::thread::spawn(move || {
        crate::dialog::begin(&shown, "command");
        match crate::ai::ask_about_screen(&question) {
            Ok(answer) => {
                crate::dialog::reply(&answer, "local");
                crate::dialog::end(true);
                if crate::tts::is_enabled() {
                    crate::tts::speak(&crate::ai::speakable(&answer));
                }
            }
            Err(e) => {
                crate::dialog::reply(&e.user_text(), "local");
                crate::dialog::end(false);
            }
        }
    });
}

// A phrase of several steps (a macro, a phrase of game_profile.json) is played in a thread of its own, step by step: the lock of the game is
// taken for one step only, the pauses are waited outside it. So «стой», any other phrase, the watchdog and the anti-cheat guard reach the
// program while it plays. Every new action (or «стой») makes the number below bigger, and a macro that sees another number stops.
static PLAY_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

// the macro that plays (if any) is told to stop after its current step
pub fn cancel_playing() {
    PLAY_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

// Is a macro playing now (the Games page and the report show it)
static PLAYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn is_playing() -> bool {
    PLAYING.load(std::sync::atomic::Ordering::SeqCst)
}

// plays the steps; `wait` waits outside the lock (and returns early when the macro is cancelled)
fn play_steps(exe: &str, steps: &[Step], gen: u64, wait: &dyn Fn(u64)) {
    use std::sync::atomic::Ordering::SeqCst;
    for st in steps {
        if PLAY_GEN.load(SeqCst) != gen {
            return;
        }
        if let Step::Wait(ms) = st {
            wait(*ms);
            continue;
        }
        let mut g = GAME.lock();
        if !g.active || PLAY_GEN.load(SeqCst) != gen {
            return;
        }
        let base = g.profile.clone();
        g.profile = base.effective(exe);
        let go = with_backend(|b| {
            if b.may_continue(&g.profile) {
                g.apply(&Action::Custom(vec![st.clone()]), b);
                true
            } else {
                g.release_all(b);
                false
            }
        });
        g.profile = base;
        if !go {
            note_event(&format!("макрос остановлен: игра {} не впереди, защита от читов или Esc", exe));
            return;
        }
    }
}

fn play(exe: String, steps: Vec<Step>) {
    use std::sync::atomic::Ordering::SeqCst;
    let gen = PLAY_GEN.fetch_add(1, SeqCst) + 1;
    #[cfg(test)]
    {
        play_steps(&exe, &steps, gen, &|_| {});
    }
    #[cfg(not(test))]
    {
        std::thread::spawn(move || {
            PLAYING.store(true, SeqCst);
            let wait = |ms: u64| {
                let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
                while std::time::Instant::now() < end && PLAY_GEN.load(SeqCst) == gen {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            };
            play_steps(&exe, &steps, gen, &wait);
            // only the newest macro puts the flag down
            if PLAY_GEN.load(SeqCst) == gen {
                PLAYING.store(false, SeqCst);
            }
        });
    }
}

// the last things the game mode did (for the report of the Games page): a time and a short text, no phrases of the user
static EVENTS: Lazy<Mutex<std::collections::VecDeque<(u64, String)>>> = Lazy::new(|| Mutex::new(std::collections::VecDeque::new()));

pub fn note_event(text: &str) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut e = EVENTS.lock();
    if e.len() >= 40 {
        e.pop_front();
    }
    e.push_back((t, text.chars().take(200).collect()));
}

pub fn recent_events() -> Vec<(u64, String)> {
    EVENTS.lock().iter().cloned().collect()
}

// the watchdog: a key must never stay pressed when the user leaves the game window (alt-tab to Jarvis, a crash of the game)
static WATCHDOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn start_watchdog() {
    use std::sync::atomic::Ordering::SeqCst;
    if WATCHDOG.swap(true, SeqCst) {
        return;
    }
    crate::gameoverlay::start();
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let mut g = GAME.lock();
        if g.active && anticheat_running().is_some() {
            with_backend(|b| g.release_all(b));
            g.active = false;
            crate::aimassist::set_enabled(false, &g.profile.aim);
            crate::gamemacro::cancel();
            log::warn!("Game mode: an anti-cheat program is running, the mode is switched off");
            note_event("режим выключен: запущена защита от читов");
            continue;
        }
        if g.active && g.anything_held() && !in_game(&g.profile) {
            with_backend(|b| g.release_all(b));
            log::info!("Game mode: the game window is not in front, the keys are released");
        }
    });
}

// what a phrase means in the game mode right now: the user's own phrases first, then the phrases of the kind of the game in front,
// then the common ones. The words that control the mode itself (on, off, stop, add a game ...) are never taken by the first two.
pub fn decide(text: &str) -> Option<Action> {
    let active = is_active();
    let builtin = parse(text, active);
    if !active || matches!(builtin, Some(Action::On | Action::Off | Action::Help | Action::ReleaseAll | Action::AddGame(_) | Action::ForgetGame | Action::AimAssist(_) | Action::MacroStart(_) | Action::MacroStop | Action::MacroDelete(_))) {
        return builtin;
    }
    let profile = {
        let g = GAME.lock();
        match front_exe().filter(|e| g.profile.all_exes().contains(e)) {
            Some(exe) => g.profile.effective(&exe),
            None => g.profile.clone(),
        }
    };
    // the phrases of the game, then the macros the user recorded (for every game), then the kind of the game, then the common ones
    let mut phrases = profile.phrases.clone();
    phrases.extend(crate::gamemacro::load());
    custom_match(text, &phrases).or_else(|| genre_parse(text, profile.genre)).or(builtin)
}

// is this phrase for the game mode (so that the caller can ask for the right before it is carried out)
pub fn would_handle(text: &str) -> bool {
    decide(text).is_some()
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

// the keys that are held now, by name (for the label over the game)
pub fn held_names() -> Vec<String> {
    GAME.lock().held.iter().filter_map(|k| crate::gamemacro::key_name(*k)).map(|n| n.to_uppercase()).collect()
}

pub fn is_active() -> bool {
    GAME.lock().active
}

pub fn release_all() {
    cancel_playing();
    let mut g = GAME.lock();
    with_backend(|b| g.release_all(b));
}

// what the diagnostics (and the report of the Games page) need to know, without anything personal
pub struct Facts {
    pub active: bool,
    pub in_game: bool,
    pub anticheat: Option<String>,
    pub games: usize,
    pub macros: usize,
    pub recording: bool,
    pub playing: bool,
    pub overlay: bool,
    pub sound: bool,
    pub scan_running: bool,
    // (found, added, skipped)
    pub scan: Option<(usize, usize, usize)>,
}

pub fn facts() -> Facts {
    let (active, in_game_now) = {
        let g = GAME.lock();
        (g.active, g.active && in_game(&g.profile))
    };
    let o = crate::gamelist::options();
    let sc = crate::gamescan::status_json();
    let found = sc["found"].as_array().cloned().unwrap_or_default();
    Facts {
        active,
        in_game: in_game_now,
        anticheat: anticheat_running(),
        games: crate::gamelist::load().len(),
        macros: crate::gamemacro::load().len(),
        recording: crate::gamemacro::is_recording(),
        playing: is_playing(),
        overlay: o.overlay,
        sound: o.sound,
        scan_running: sc["running"].as_bool().unwrap_or(false),
        scan: if sc["done"].as_bool().unwrap_or(false) { Some((found.len(), sc["added"].as_u64().unwrap_or(0) as usize, found.iter().filter(|f| !f["skipped"].is_null()).count())) } else { None },
    }
}

// the text of the button «Копировать отчёт по играм»: the state, the list, the last scan, the last events. No keys, no phrases.
pub fn report_text() -> String {
    let f = facts();
    let yn = |b: bool| if b { "да" } else { "нет" };
    let mut r = String::from("=== Отчёт по играм Jarvis ===\n");
    r.push_str(&format!("Версия Jarvis: {}\n", crate::config::APP_VERSION.unwrap_or("?")));
    r.push_str(&format!("Режим включён: {} · окно игры впереди: {} · защита от читов: {}\n", yn(f.active), yn(f.in_game), f.anticheat.clone().unwrap_or_else(|| "не найдена".into())));
    r.push_str(&format!("Метка на экране: {} · звук подтверждения: {} · запись макроса идёт: {} · макрос играет: {}\n", yn(f.overlay), yn(f.sound), yn(f.recording), yn(f.playing)));
    r.push_str(&format!("Помощь прицела: {}\n", if crate::aimassist::is_enabled() { "включена" } else { "выключена" }));
    r.push_str(&format!("\nИгры в списке ({}):\n", f.games));
    for e in crate::gamelist::load() {
        r.push_str(&format!("  {} · {} · {} · {}\n", e.exe, e.name, e.genre.name(), e.source));
    }
    r.push_str(&format!("\nМакросы ({}):\n", f.macros));
    for (n, steps) in crate::gamemacro::load() {
        r.push_str(&format!("  {} · шагов: {}\n", n, steps.len()));
    }
    let sc = crate::gamescan::status_json();
    r.push_str("\nПоследний поиск игр: ");
    if f.scan_running {
        r.push_str("идёт\n");
    } else if let Some((found, added, skipped)) = f.scan {
        r.push_str(&format!("найдено {}, добавлено новых {}, не добавлено {}\n", found, added, skipped));
        for it in sc["found"].as_array().into_iter().flatten().filter(|i| !i["skipped"].is_null()) {
            r.push_str(&format!("  не добавлена: {} ({}) — {}\n", it["name"].as_str().unwrap_or(""), it["store"].as_str().unwrap_or(""), it["skipped"].as_str().unwrap_or("")));
        }
    } else {
        r.push_str("ещё не запускался\n");
    }
    let ev = recent_events();
    r.push_str("\nПоследние события режима:\n");
    if ev.is_empty() {
        r.push_str("  пока нет\n");
    }
    for (t, text) in ev.iter().rev().take(20) {
        r.push_str(&format!("  unix {} · {}\n", t, text));
    }
    r.push_str("\nВ отчёт не входят: ключи, токены, сказанные фразы и снимки экрана. Проверено только тестами разбора, если в этом отчёте нет событий из живой игры.\n");
    r
}

// everything the Games page shows: the list, the scan, the macros, the options
pub fn games_json() -> Value {
    let games: Vec<Value> = crate::gamelist::load().iter().map(|e| json!({ "exe": e.exe, "name": e.name, "genre": e.genre.name(), "source": e.source })).collect();
    let macros: Vec<Value> = crate::gamemacro::load().iter().map(|(n, s)| json!({ "name": n, "steps": s.iter().filter_map(crate::gamemacro::step_text).collect::<Vec<_>>() })).collect();
    let o = crate::gamelist::options();
    json!({ "games": games, "scan": crate::gamescan::status_json(), "macros": macros, "recording": crate::gamemacro::recording_name(), "options": { "overlay": o.overlay, "sound": o.sound }, "builtin": Profile::default().exes })
}

// a program the user typed or picked: the file name only («C:\\Games\\x\\game.exe» -> «game.exe»)
pub fn exe_of_input(raw: &str) -> String {
    raw.trim().trim_matches('"').rsplit(|c| c == '\\' || c == '/').next().unwrap_or("").trim().to_lowercase()
}

pub fn status_json() -> Value {
    let g = GAME.lock();
    json!({ "active": g.active, "game": g.profile.title, "exes": g.profile.all_exes(), "in_game": g.active && in_game(&g.profile), "held": g.anything_held(), "added": g.profile.extra, "genres": g.profile.genres.iter().map(|(e, gn)| (e.clone(), gn.name())).collect::<std::collections::BTreeMap<_, _>>(), "anticheat": anticheat_running(), "aim": crate::aimassist::status_json(&g.profile.aim) })
}

// A phrase of the user: Some(the answer to show) when it belonged to the game mode (the caller stops there), None otherwise.
pub fn handle(text: &str) -> Option<String> {
    let action = decide(text)?;
    // a new action of the game (or «стой») ends the macro that plays; the answers about the screen, the list and the macros do not
    if !matches!(action, Action::Hint | Action::ReadText(_) | Action::MacroStart(_) | Action::MacroStop | Action::MacroDelete(_) | Action::AddGame(_) | Action::ForgetGame | Action::Help | Action::AimAssist(_)) {
        cancel_playing();
    }
    let mut g = GAME.lock();
    // an anti-cheat program is running: the mode is not started and is switched off (the keys are released first)
    if let Some(name) = anticheat_running() {
        if matches!(action, Action::On) || g.active {
            with_backend(|b| g.release_all(b));
            g.active = false;
            crate::aimassist::set_enabled(false, &g.profile.aim);
            crate::gamemacro::cancel();
            note_event(&format!("режим не включён или выключен: защита от читов ({})", name));
            return Some(format!("Игровой режим не включён: запущена защита от читов ({}). Режим только для одиночных игр без неё: закройте игру с защитой.", name));
        }
    }
    match &action {
        Action::On => {
            // a bad profile file does not stop the mode: the usual keys are taken and the user is told why
            let (profile, problem) = match load_profile() {
                Ok(p) => (p, String::new()),
                Err(e) => (Profile::default(), format!(" Файл game_profile.json не применён: {}.", e)),
            };
            g.profile = profile;
            apply_list(&mut g.profile);
            let mut reply = with_backend(|b| g.apply(&action, b));
            reply.push_str(&problem);
            if !g.profile.extra.is_empty() {
                reply.push_str(&format!(" Добавленные игры: {}.", g.profile.extra.join(", ")));
            }
            reply.push_str(" Своя игра: откройте её и скажите «добавь эту игру».");
            drop(g);
            start_watchdog();
            note_event("режим включён");
            Some(reply)
        }
        Action::Off | Action::Help | Action::ReleaseAll => {
            if matches!(action, Action::Off) {
                crate::gamemacro::cancel();
                note_event("режим выключен");
            }
            Some(with_backend(|b| g.apply(&action, b)))
        }
        Action::MacroStart(name) => Some(match crate::gamemacro::start(name) {
            Ok(m) => {
                note_event("запись макроса началась");
                m
            }
            Err(m) => m,
        }),
        Action::MacroStop => Some(match crate::gamemacro::stop() {
            Ok(m) => {
                note_event("макрос сохранён");
                m
            }
            Err(m) => m,
        }),
        Action::MacroDelete(name) => Some(if crate::gamemacro::delete(name) { format!("Макрос «{}» удалён.", name) } else { format!("Макроса «{}» нет.", name) }),
        Action::Hint | Action::ReadText(_) => {
            let title = front_exe().filter(|e| g.profile.all_exes().contains(e)).unwrap_or_else(|| g.profile.title.clone());
            let (question, shown) = match &action {
                Action::ReadText(tr) => (read_question(*tr), if *tr { "Переведи текст на экране игры" } else { "Прочитай текст на экране игры" }),
                _ => (hint_question(&title), "Подсказка по игре"),
            };
            drop(g);
            ask_screen(question, shown.to_string());
            Some("Смотрю на экран игры…".to_string())
        }
        Action::AddGame(genre) => {
            let Some(exe) = front_exe() else {
                return Some("Не вижу окна игры: откройте игру (она должна быть впереди) и повторите.".into());
            };
            if let Some(why) = refuse_reason(&exe) {
                return Some(format!("Не добавил: {}.", why));
            }
            let in_extra = g.profile.extra.contains(&exe);
            if g.profile.all_exes().contains(&exe) && !in_extra {
                return Some(format!("{} уже в списке игр (он в списке по умолчанию).", exe));
            }
            // a well-known game: its kind and name are taken from the table (the user may name another kind)
            let known = crate::gamescan::known_game(&exe);
            let entry = crate::gamelist::Entry { exe: exe.clone(), name: known.map(|k| k.0.to_string()).unwrap_or_else(|| exe.clone()), genre: genre.or(known.map(|k| k.1)).unwrap_or_default(), source: "voice".into() };
            let outcome = crate::gamelist::add(entry, genre.is_some());
            apply_list(&mut g.profile);
            match outcome {
                Err(why) => Some(format!("Не добавил: {}.", why)),
                Ok(crate::gamelist::Added::Known) => match genre {
                    Some(gn) => Some(format!("Жанр игры {}: {}.", exe, gn.title())),
                    None => Some(format!("{} уже в списке игр.", exe)),
                },
                Ok(crate::gamelist::Added::New) => {
                    note_event(&format!("игра добавлена голосом: {}", exe));
                    let kind = genre.or(known.map(|k| k.1)).map(|gn| format!(" Жанр: {}.", gn.title())).unwrap_or_else(|| " Жанр не назван: понимаю общие фразы и фразы шутера; для гонки или экшена скажите «добавь эту игру как гонку» / «как экшен».".to_string());
                    Some(format!("Добавил игру {}. Обычные клавиши (WASD, пробел, C, Shift, R, G, V, F); другие клавиши — game_profile.json. Только одиночные игры!{}", exe, kind))
                }
            }
        }
        Action::ForgetGame => {
            let Some(exe) = front_exe() else {
                return Some("Не вижу окна игры: откройте игру и повторите.".into());
            };
            if !g.profile.extra.contains(&exe) {
                return Some(format!("{} нет среди добавленных вами игр.", exe));
            }
            crate::gamelist::remove(&exe);
            note_event(&format!("игра убрана голосом: {}", exe));
            apply_list(&mut g.profile);
            with_backend(|b| g.release_all(b));
            Some(format!("Игра {} убрана из списка.", exe))
        }
        _ => {
            // an action goes only to the window of a game of the list, with the keys of that game
            let Some(exe) = front_exe().filter(|e| g.profile.all_exes().contains(e)) else {
                with_backend(|b| g.release_all(b));
                return Some(format!("Игра не в фокусе: откройте окно игры ({}) или добавьте её («добавь эту игру»), и я продолжу.", g.profile.all_exes().join(", ")));
            };
            // a phrase with steps plays in its own thread (the lock is let go first), the rest is a matter of a moment
            if let Action::Custom(steps) = &action {
                let steps = steps.clone();
                drop(g);
                play(exe, steps);
                crate::gameoverlay::confirm();
                return Some(String::new());
            }
            let base = g.profile.clone();
            g.profile = base.effective(&exe);
            let reply = with_backend(|b| g.apply(&action, b));
            g.profile = base;
            // a short sound says «done» (if it is switched on)
            crate::gameoverlay::confirm();
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

    // the tests that use the global state (the mode, the profile) run one after another
    // (the same lock as the tests of the recorder: switching the mode off throws a recording away)
    use crate::gamemacro::REC_TESTS as GLOBAL;

    fn reset_global() {
        let mut g = GAME.lock();
        g.active = false;
        g.profile = Profile::default();
        g.left_button = false;
        g.right_button = false;
        g.held.clear();
        crate::gamelist::test_reset();
        TEST_PROCS.with(|t| *t.borrow_mut() = Some(vec!["explorer.exe".into()]));
    }

    #[test]
    fn macros_hints_and_reading_have_their_own_phrases() {
        assert_eq!(parse("запиши макрос комбо", true), Some(Action::MacroStart("комбо".into())));
        assert_eq!(parse("сохрани макрос", true), Some(Action::MacroStop));
        assert_eq!(parse("удали макрос комбо", true), Some(Action::MacroDelete("комбо".into())));
        assert_eq!(parse("прочитай текст", true), Some(Action::ReadText(false)));
        assert_eq!(parse("переведи текст", true), Some(Action::ReadText(true)));
        for t in ["подскажи что делать", "куда идти", "как пройти дальше"] {
            assert_eq!(parse(t, true), Some(Action::Hint), "{}", t);
        }
        // with the mode off none of them is a game phrase
        for t in ["запиши макрос комбо", "прочитай текст", "подскажи что делать"] {
            assert_eq!(parse(t, false), None, "{}", t);
        }
    }

    // a backend that says «stop» after `allow` steps: the game left the front, an anti-cheat program started, or Esc was pressed
    struct Guarded {
        log: Vec<String>,
        allow: usize,
    }
    impl Backend for Guarded {
        fn key(&mut self, vk: u16, down: bool) {
            self.log.push(format!("{:#x}{}", vk, if down { "↓" } else { "↑" }));
        }
        fn mouse_move(&mut self, _: i32, _: i32) {}
        fn button(&mut self, _: bool, _: bool) {}
        fn pause(&mut self, _: u64) {}
        fn may_continue(&mut self, _: &Profile) -> bool {
            if self.allow == 0 {
                return false;
            }
            self.allow -= 1;
            true
        }
    }

    #[test]
    fn a_macro_stops_and_lets_go_of_the_keys_when_the_game_is_not_in_front_any_more() {
        let mut g = Game { profile: Profile::default(), active: true, ..Default::default() };
        let steps = vec![Step::Hold(0x57), Step::Wait(500), Step::Tap(0x20), Step::Tap(0x20), Step::Release(0x57)];
        let mut b = Guarded { log: Vec::new(), allow: 2 };
        g.apply(&Action::Custom(steps.clone()), &mut b);
        // the hold and the wait were done; at the third step the guard said stop: no tap, and W is let go
        assert_eq!(b.log, vec!["0x57↓", "0x57↑"]);
        assert!(!g.anything_held());
        // with the guard saying yes all steps are done
        let mut g = Game { profile: Profile::default(), active: true, ..Default::default() };
        let mut b = Guarded { log: Vec::new(), allow: 99 };
        g.apply(&Action::Custom(steps), &mut b);
        assert_eq!(b.log.len(), 6);
    }

    #[test]
    fn switching_the_mode_off_or_an_anti_cheat_program_throws_a_recording_away() {
        let _l = GLOBAL.lock();
        reset_global();
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("iw4sp.exe".into())));
        crate::gamemacro::cancel();
        handle("включи игровой режим").unwrap();
        assert!(handle("запиши макрос комбо").unwrap().contains("Записываю"));
        assert!(crate::gamemacro::is_recording());
        handle("выключи игровой режим").unwrap();
        assert!(!crate::gamemacro::is_recording(), "off: the recording is gone");
        // an anti-cheat program
        handle("включи игровой режим").unwrap();
        handle("запиши макрос комбо").unwrap();
        assert!(crate::gamemacro::is_recording());
        TEST_PROCS.with(|t| *t.borrow_mut() = Some(vec!["easyanticheat.exe".into()]));
        let _ = handle("иди вперёд");
        assert!(!crate::gamemacro::is_recording(), "anti-cheat: the recording is gone");
        crate::gamemacro::cancel();
        TEST_FRONT.with(|t| *t.borrow_mut() = None);
        reset_global();
    }

    #[test]
    fn a_stop_during_a_macro_ends_it_after_the_current_step_and_stop_lets_go_of_the_keys() {
        let _l = GLOBAL.lock();
        reset_global();
        {
            let mut g = GAME.lock();
            g.active = true;
        }
        let steps = vec![Step::Hold(0x57), Step::Wait(500), Step::Tap(0x20), Step::Hold(0x10)];
        let gen = PLAY_GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        // «стой» arrives while the macro waits: the wait returns, the macro sees the new number and does nothing more
        play_steps("iw4sp.exe", &steps, gen, &|_| cancel_playing());
        {
            let g = GAME.lock();
            assert!(g.held.contains(&0x57), "the step before the wait was done");
            assert!(!g.held.contains(&0x10), "the steps after the stop were not");
        }
        // the phrase «стой» itself: it bumps the number and lets go of everything
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("iw4sp.exe".into())));
        let before = PLAY_GEN.load(std::sync::atomic::Ordering::SeqCst);
        handle("стой").unwrap();
        assert!(PLAY_GEN.load(std::sync::atomic::Ordering::SeqCst) > before);
        assert!(GAME.lock().held.is_empty());
        // a macro whose number is old does not start a step at all
        let old = PLAY_GEN.load(std::sync::atomic::Ordering::SeqCst) - 1;
        play_steps("iw4sp.exe", &[Step::Hold(0x57)], old, &|_| {});
        assert!(GAME.lock().held.is_empty());
        TEST_FRONT.with(|t| *t.borrow_mut() = None);
        reset_global();
    }

    #[test]
    fn the_report_of_the_games_has_the_state_the_list_the_scan_and_the_events_and_nothing_personal() {
        let _l = GLOBAL.lock();
        reset_global();
        crate::gamelist::add(crate::gamelist::Entry { exe: "fallout3.exe".into(), name: "Fallout 3".into(), genre: Genre::Action, source: "scan".into() }, false).unwrap();
        note_event("проверка отчёта");
        let r = report_text();
        for part in ["Отчёт по играм", "Режим включён: нет", "защита от читов: не найдена", "fallout3.exe · Fallout 3 · action · scan", "Игры в списке (1)", "Последний поиск игр", "проверка отчёта", "не входят: ключи"] {
            assert!(r.contains(part), "{}: {}", part, r);
        }
        reset_global();
    }

    #[test]
    fn what_the_games_page_types_becomes_a_program_name() {
        assert_eq!(exe_of_input("C:\\Games\\Fallout 3\\Fallout3.EXE"), "fallout3.exe");
        assert_eq!(exe_of_input("  \"D:/x/y/Game.exe\" "), "game.exe");
        assert_eq!(exe_of_input("game.exe"), "game.exe");
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
    fn the_phrases_that_add_and_forget_a_game() {
        assert_eq!(p("добавь эту игру"), Some(Action::AddGame(None)));
        assert_eq!(p("запомни эту игру"), Some(Action::AddGame(None)));
        assert_eq!(p("добавь эту игру как гонку"), Some(Action::AddGame(Some(Genre::Racing))));
        assert_eq!(p("добавь эту игру как экшен"), Some(Action::AddGame(Some(Genre::Action))));
        assert_eq!(p("запомни эту игру как приключение"), Some(Action::AddGame(Some(Genre::Action))));
        assert_eq!(p("добавь эту игру как шутер"), Some(Action::AddGame(Some(Genre::Shooter))));
        assert_eq!(p("забудь эту игру"), Some(Action::ForgetGame));
        assert_eq!(p("удали эту игру из списка"), Some(Action::ForgetGame));
        // only inside the mode, and only «this» game: a normal request about games is not taken
        assert_eq!(parse("добавь эту игру", false), None);
        assert_eq!(p("добавь игру в избранное"), None);
    }

    #[test]
    fn any_offline_game_can_be_added_with_its_own_keys_but_not_an_online_game_or_a_non_game() {
        let _lock = GLOBAL.lock();
        reset_global();
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("mygame.exe".into())));
        handle("включи игровой режим");
        // a game that is not in the list yet: nothing is sent, and the user is told how to add it
        let m = handle("иди вперёд").unwrap();
        assert!(m.contains("добавьте её"), "{}", m);
        assert!(!GAME.lock().anything_held());
        // added by voice: it works at once, with the usual keys
        assert!(handle("добавь эту игру").unwrap().contains("Добавил игру mygame.exe"));
        assert!(handle("добавь эту игру").unwrap().contains("уже в списке"));
        handle("иди вперёд");
        assert!(GAME.lock().held.contains(&0x57), "W is held in the added game");
        handle("стой");
        // remembered for the next time
        assert_eq!(crate::gamelist::load().iter().map(|e| e.exe.clone()).collect::<Vec<_>>(), vec!["mygame.exe".to_string()]);
        // an online game and a program that is not a game are refused
        for (exe, word) in [("valorant.exe", "сетевая игра"), ("cs2.exe", "сетевая игра"), ("chrome.exe", "не игра"), ("cmd.exe", "не игра")] {
            TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some(exe.into())));
            let m = handle("добавь эту игру").unwrap();
            assert!(m.contains("Не добавил") && m.contains(word), "{}: {}", exe, m);
        }
        // no window: told so
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(None));
        assert!(handle("добавь эту игру").unwrap().contains("Не вижу окна игры"));
        // forgotten: it is outside the list again
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("mygame.exe".into())));
        assert!(handle("забудь эту игру").unwrap().contains("убрана"));
        assert!(handle("иди вперёд").unwrap().contains("Игра не в фокусе"));
        assert!(handle("забудь эту игру").unwrap().contains("нет среди добавленных"));
        // a game of the default list cannot be «forgotten» (it is not the user's addition)
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("iw4sp.exe".into())));
        assert!(handle("забудь эту игру").unwrap().contains("нет среди добавленных"));
        handle("выключи игровой режим");
        TEST_FRONT.with(|t| *t.borrow_mut() = None);
        reset_global();
    }

    #[test]
    fn racing_phrases() {
        let r = |t: &str| genre_parse(t, Genre::Racing);
        assert_eq!(r("газ"), Some(Action::Walk(Dir::Forward)));
        assert_eq!(r("гони"), Some(Action::Walk(Dir::Forward)));
        assert_eq!(r("тормози"), Some(Action::Walk(Dir::Back)));
        assert_eq!(r("задний ход"), Some(Action::Walk(Dir::Back)));
        assert_eq!(r("прямо"), Some(Action::Straight), "in racing «прямо» is a straight wheel, not walking");
        assert_eq!(r("ручник"), Some(Action::Tap(Tap::Handbrake)));
        assert_eq!(r("включи нитро"), Some(Action::Tap(Tap::Nitro)));
        assert_eq!(r("посигналь"), Some(Action::Tap(Tap::Horn)));
        assert_eq!(r("смени камеру"), Some(Action::Tap(Tap::Camera)));
        // steering is the common «влево / вправо»
        assert_eq!(r("влево"), None);
        assert_eq!(p("влево"), Some(Action::Walk(Dir::Left)));
        assert_eq!(genre_parse("газ", Genre::Shooter), None);
    }

    #[test]
    fn action_adventure_phrases() {
        let a = |t: &str| genre_parse(t, Genre::Action);
        assert_eq!(a("атакуй"), Some(Action::Fire(150)));
        assert_eq!(a("тяжёлый удар"), Some(Action::Fire(700)));
        assert_eq!(a("блок"), Some(Action::AimOn));
        assert_eq!(a("сними блок"), Some(Action::AimOff));
        assert_eq!(a("перекат"), Some(Action::Tap(Tap::Dodge)));
        assert_eq!(a("открой инвентарь"), Some(Action::Tap(Tap::Inventory)));
        assert_eq!(a("покажи карту"), Some(Action::Tap(Tap::Map)));
        assert_eq!(a("полечись"), Some(Action::Tap(Tap::Heal)));
        assert_eq!(a("садись в машину"), Some(Action::Tap(Tap::Use)));
        assert_eq!(a("иди вперёд"), None);
    }

    #[test]
    fn a_phrase_of_the_user_is_any_list_of_steps_in_any_genre() {
        let p = profile_from_json(r#"{"genre":"racing","phrases":{"Газуй!":["hold:w","wait:300","tap:space"],"прицельный выстрел":["click:right","wait:50","click:left"],"поверни":["move:50,-20"]}}"#).unwrap();
        assert_eq!(p.genre, Genre::Racing);
        assert_eq!(p.phrases.len(), 3);
        assert_eq!(p.phrases.iter().find(|(w, _)| w == "газуй").unwrap().1, vec![Step::Hold(0x57), Step::Wait(300), Step::Tap(0x20)]);
        // the words stand in a sentence as whole words
        assert!(matches!(custom_match("давай газуй сейчас", &p.phrases), Some(Action::Custom(_))));
        assert_eq!(custom_match("газуйте", &p.phrases), None);
        // run: every step is carried out, held keys are tracked and released by «стой»
        let mut g = Game { profile: Profile::default(), active: true, ..Default::default() };
        let mut r = Rec::default();
        let Some(Action::Custom(steps)) = custom_match("газуй", &p.phrases) else { panic!() };
        g.apply(&Action::Custom(steps), &mut r);
        assert_eq!(r.0, vec!["0x57↓".to_string(), "0x20↓".into(), "0x20↑".into()]);
        assert!(g.held.contains(&0x57));
        g.apply(&Action::ReleaseAll, &mut r);
        assert!(!g.anything_held());
        // wrong things are refused with a reason
        for bad in [
            r#"{"phrases":{"стой тут":["tap:w"]}}"#,
            r#"{"phrases":{"выключи":["tap:w"]}}"#,
            r#"{"phrases":{"a":["tap:alt"]}}"#,
            r#"{"phrases":{"a":["tap:f4"]}}"#,
            r#"{"phrases":{"a":["fly:w"]}}"#,
            r#"{"phrases":{"a":["wait:99999"]}}"#,
            r#"{"phrases":{"a":["move:9999,0"]}}"#,
            r#"{"phrases":{"a":[]}}"#,
            r#"{"phrases":{"":["tap:w"]}}"#,
            r#"{"phrases":["tap:w"]}"#,
            r#"{"genre":"puzzle"}"#,
        ] {
            assert!(profile_from_json(bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn a_game_added_as_racing_or_action_understands_the_phrases_of_that_kind() {
        let _lock = GLOBAL.lock();
        reset_global();
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("racer.exe".into())));
        handle("включи игровой режим");
        // no kind named: the common and shooter phrases only («газ» is nothing for a shooter, the phrase goes on to the usual commands)
        assert!(handle("добавь эту игру").unwrap().contains("Жанр не назван"));
        assert_eq!(handle("газ"), None);
        // the kind is named later: changed (the game stays in the list)
        assert!(handle("добавь эту игру как гонку").unwrap().contains("Жанр игры racer.exe: гонки"));
        assert!(would_handle("газ"));
        handle("газ");
        assert!(GAME.lock().held.contains(&0x57), "gas holds W");
        handle("тормоз");
        assert!(GAME.lock().held.contains(&0x53) && !GAME.lock().held.contains(&0x57), "the brake releases the gas");
        handle("влево");
        assert!(GAME.lock().held.contains(&0x41));
        handle("прямо");
        assert!(!GAME.lock().held.contains(&0x41), "straight releases the steering");
        handle("стой");
        assert!(!GAME.lock().anything_held());
        assert_eq!(crate::gamelist::load().iter().find(|e| e.exe == "racer.exe").map(|e| e.genre), Some(Genre::Racing), "the kind is remembered");
        // an action game added at once with its kind
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("adventure.exe".into())));
        assert!(handle("запомни эту игру как приключение").unwrap().contains("экшен / приключения"));
        handle("блок");
        assert!(GAME.lock().right_button, "block holds the right button");
        handle("сними блок");
        assert!(!GAME.lock().right_button);
        // the phrases of a kind do not leak to another game
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("racer.exe".into())));
        assert_eq!(decide("блок"), parse("блок", true));
        // the user's own phrases of the profile work in the game in front
        GAME.lock().profile.phrases = vec![("на ура".to_string(), vec![Step::Tap(0x45)])];
        assert!(matches!(decide("давай на ура"), Some(Action::Custom(_))));
        // forgotten: the kind goes with the game
        handle("забудь эту игру");
        assert!(!crate::gamelist::load().iter().any(|e| e.exe == "racer.exe"));
        handle("выключи игровой режим");
        TEST_FRONT.with(|t| *t.borrow_mut() = None);
        reset_global();
    }

    #[test]
    fn a_game_can_have_its_own_keys_and_sensitivity() {
        let p = profile_from_json(r#"{"games":{"Other.EXE":{"px_per_90":300,"keys":{"forward":"i","jump":"ctrl"}}}}"#).unwrap();
        let e = p.effective("other.exe");
        assert_eq!((e.px_per_90, e.keys.forward, e.keys.jump), (300, 0x49, 0x11));
        // another program keeps the common keys and sensitivity
        let c = p.effective("iw4sp.exe");
        assert_eq!((c.px_per_90, c.keys.forward, c.keys.jump), (450, 0x57, 0x20));
        for bad in [r#"{"games":{"x":{}}}"#, r#"{"games":{"C:\\a.exe":{}}}"#, r#"{"games":{"a.exe":{"px_per_90":1}}}"#, r#"{"games":{"a.exe":{"keys":{"jump":"alt"}}}}"#] {
            assert!(profile_from_json(bad).is_err(), "{}", bad);
        }
    }

    #[test]
    fn an_anti_cheat_program_stops_the_mode_and_keeps_it_from_starting() {
        let _lock = GLOBAL.lock();
        reset_global();
        TEST_FRONT.with(|t| *t.borrow_mut() = Some(Some("iw4sp.exe".into())));
        TEST_PROCS.with(|t| *t.borrow_mut() = Some(vec!["explorer.exe".into(), "EasyAntiCheat.exe".to_lowercase()]));
        let m = handle("включи игровой режим").unwrap();
        assert!(m.contains("защита от читов") && m.contains("easyanticheat.exe"), "{}", m);
        assert!(!is_active());
        // the mode was on, an anti-cheat appears: the next phrase switches it off and releases the keys
        TEST_PROCS.with(|t| *t.borrow_mut() = Some(vec!["explorer.exe".into()]));
        handle("включи игровой режим");
        handle("иди вперёд");
        assert!(GAME.lock().anything_held());
        TEST_PROCS.with(|t| *t.borrow_mut() = Some(vec!["explorer.exe".into(), "beservice.exe".into()]));
        let m = handle("беги").unwrap();
        assert!(m.contains("защита от читов"), "{}", m);
        assert!(!is_active() && !GAME.lock().anything_held());
        for name in ["easyanticheat.exe", "beservice.exe", "vgc.exe", "faceitclient.exe"] {
            assert!(ANTICHEAT.contains(&name), "{}", name);
        }
        TEST_FRONT.with(|t| *t.borrow_mut() = None);
        reset_global();
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
        let _lock = GLOBAL.lock();
        reset_global();
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
