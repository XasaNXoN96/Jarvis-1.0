//! Smart home through the REST API of Home Assistant (the user's own server: «http://homeassistant.local:8123» and a long-lived access
//! token from their profile page). There are no invented devices: the list is what the server returns, and a command is reported
//! as done only when the server answers that it accepted it. Without the address and the token the feature says so and does nothing.
//! Doors, locks and alarms are NOT controlled by voice: only lights, switches, fans, covers and media players.

use crate::{perms, store, textnorm};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const FILE: &str = "smarthome.json";
pub const DOMAINS: [&str; 5] = ["light", "switch", "fan", "cover", "media_player"];

#[derive(Serialize, Deserialize, Default, Clone)]
struct Conf {
    #[serde(default)]
    url: String,
    // sealed (DPAPI on Windows)
    #[serde(default)]
    access_token: String,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Device {
    pub entity_id: String,
    pub name: String,
    pub domain: String,
    pub state: String,
}

pub fn valid_url(u: &str) -> bool {
    let u = u.trim();
    (u.starts_with("http://") || u.starts_with("https://")) && !u.contains(char::is_whitespace) && u.len() < 200 && u.split("://").nth(1).map(|h| !h.is_empty()).unwrap_or(false)
}

pub fn configure(url: &str, token: &str) -> Result<(), String> {
    crate::owner::guard()?;
    let url = url.trim().trim_end_matches('/');
    if !valid_url(url) {
        return Err("Адрес должен начинаться с http:// или https://".into());
    }
    let mut c: Conf = store::read::<Conf>(FILE).0;
    c.url = url.to_string();
    // an empty token field keeps the saved token
    if !token.trim().is_empty() {
        c.access_token = crate::secret::seal(token.trim());
    }
    if c.access_token.is_empty() {
        return Err("Нужен долгосрочный токен доступа из профиля Home Assistant.".into());
    }
    store::write(FILE, &c)
}

pub fn forget() -> bool {
    store::write(FILE, &Conf::default()).is_ok()
}

pub fn status_json() -> Value {
    let c: Conf = store::read::<Conf>(FILE).0;
    json!({ "configured": !c.url.is_empty() && !c.access_token.is_empty(), "url": c.url, "has_token": !c.access_token.is_empty() })
}

pub fn is_configured() -> bool {
    status_json()["configured"].as_bool().unwrap_or(false)
}

// the devices that may be controlled, out of the answer of GET /api/states
pub fn parse_states(v: &Value) -> Vec<Device> {
    let Some(arr) = v.as_array() else { return Vec::new() };
    let mut out: Vec<Device> = arr
        .iter()
        .filter_map(|e| {
            let id = e.get("entity_id")?.as_str()?;
            let domain = id.split('.').next()?;
            if !DOMAINS.contains(&domain) {
                return None;
            }
            let state = e.get("state")?.as_str()?.to_string();
            let name = e.get("attributes").and_then(|a| a.get("friendly_name")).and_then(|n| n.as_str()).unwrap_or(id).to_string();
            Some(Device { entity_id: id.to_string(), name, domain: domain.to_string(), state })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    On,
    Off,
}

// «включи свет в гостиной» → (On, "свет в гостиной")
pub fn parse(text: &str) -> Option<(Act, String)> {
    let n = textnorm::normalize(text);
    for (p, a) in [("включи ", Act::On), ("выключи ", Act::Off), ("погаси ", Act::Off), ("turn on ", Act::On), ("turn off ", Act::Off), ("switch on ", Act::On), ("switch off ", Act::Off), ("yoq ", Act::On), ("o'chir ", Act::Off)] {
        if let Some(r) = n.strip_prefix(p) {
            let r = r.trim_start_matches("the ").to_string();
            return Some((a, r)).filter(|(_, r)| !r.is_empty());
        }
    }
    None
}

#[derive(Debug, PartialEq)]
pub enum Pick {
    One(Device),
    Many(Vec<Device>),
    None,
}

// the device the phrase is about: only a close match counts (the word «свет» alone is not a device), so ordinary commands
// like «включи музыку» are left to the normal command search
pub fn pick(devices: &[Device], phrase: &str) -> Pick {
    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
    let p = textnorm::normalize(phrase);
    // the phrase may be longer than the name («свет в гостиной» for «Гостиная»): a name inside the phrase also counts
    let pw: Vec<&str> = p.split(' ').collect();
    let inside: Vec<usize> = devices
        .iter()
        .enumerate()
        .filter(|(_, d)| {
            let n = textnorm::normalize(&d.name);
            let words: Vec<&str> = n.split(' ').filter(|w| w.chars().count() >= 3).collect();
            !words.is_empty() && words.iter().all(|w| pw.iter().any(|x| textnorm::same_stem(w, x)))
        })
        .map(|(i, _)| i)
        .collect();
    if inside.len() == 1 {
        return Pick::One(devices[inside[0]].clone());
    }
    if inside.len() > 1 {
        return Pick::Many(inside.into_iter().map(|i| devices[i].clone()).collect());
    }
    match textnorm::best_match(&p, &names) {
        textnorm::Match::Exact(i) | textnorm::Match::Fuzzy(i, _) => Pick::One(devices[i].clone()),
        textnorm::Match::Ambiguous(v) => Pick::Many(v.into_iter().map(|i| devices[i].clone()).collect()),
        textnorm::Match::None => Pick::None,
    }
}

pub fn service(d: &Device, a: Act) -> (&'static str, &'static str) {
    // (domain, service)
    let dom = match d.domain.as_str() {
        "light" => "light",
        "switch" => "switch",
        "fan" => "fan",
        "cover" => "cover",
        _ => "media_player",
    };
    let svc = match (d.domain.as_str(), a) {
        ("cover", Act::On) => "open_cover",
        ("cover", Act::Off) => "close_cover",
        (_, Act::On) => "turn_on",
        (_, Act::Off) => "turn_off",
    };
    (dom, svc)
}

#[cfg(feature = "lua")]
fn http(method: &str, path: &str, body: Option<Value>) -> Result<(u16, String), String> {
    let c: Conf = store::read::<Conf>(FILE).0;
    let token = crate::secret::open(&c.access_token);
    if c.url.is_empty() || token.is_empty() {
        return Err("Умный дом не настроен: укажите адрес Home Assistant и токен в Настройках.".into());
    }
    let client = reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(4)).timeout(std::time::Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    let url = format!("{}{}", c.url, path);
    let req = if method == "POST" { client.post(&url).json(&body.unwrap_or(json!({}))) } else { client.get(&url) };
    let resp = req.bearer_auth(token).send().map_err(|e| format!("Home Assistant не ответил: {}", e.without_url()))?;
    let code = resp.status().as_u16();
    Ok((code, resp.text().unwrap_or_default()))
}

#[cfg(feature = "lua")]
pub fn devices() -> Result<Vec<Device>, String> {
    perms::check(perms::Cap::SmartHome)?;
    let (code, body) = http("GET", "/api/states", None)?;
    match code {
        200 => Ok(parse_states(&serde_json::from_str(&body).map_err(|_| "Home Assistant вернул не JSON".to_string())?)),
        401 | 403 => Err("Home Assistant не принял токен. Создайте новый в профиле.".into()),
        c => Err(format!("Home Assistant ответил кодом {}", c)),
    }
}

// the server accepted the command (it does not promise that the lamp really lit: the next list shows the state)
#[cfg(feature = "lua")]
pub fn act(d: &Device, a: Act) -> Result<(), String> {
    perms::check(perms::Cap::SmartHome)?;
    let (dom, svc) = service(d, a);
    let (code, _) = http("POST", &format!("/api/services/{}/{}", dom, svc), Some(json!({ "entity_id": d.entity_id })))?;
    match code {
        200 | 201 => Ok(()),
        401 | 403 => Err("Home Assistant не принял токен.".into()),
        404 => Err("Home Assistant не знает такое устройство или действие.".into()),
        c => Err(format!("Home Assistant ответил кодом {}", c)),
    }
}

#[cfg(not(feature = "lua"))]
pub fn devices() -> Result<Vec<Device>, String> {
    Err("нет сети в этой сборке".into())
}
#[cfg(not(feature = "lua"))]
pub fn act(_d: &Device, _a: Act) -> Result<(), String> {
    Err("нет сети в этой сборке".into())
}


// «яркость света в гостиной 50 процентов» / «сделай свет на кухне на 30 процентов» / «set the kitchen light to 40 percent» → (50, "света в гостиной")
pub fn parse_brightness(text: &str) -> Option<(u8, String)> {
    // (the normalizer drops the sign «%»: it is turned into the word first)
    let n = textnorm::normalize(&text.replace('%', " процентов "));
    let mut rest = None;
    for p in ["установи яркость ", "поставь яркость ", "яркость ", "сделай ", "set brightness of ", "set brightness ", "set "] {
        if let Some(r) = n.strip_prefix(p) {
            rest = Some(r.to_string());
            break;
        }
    }
    let rest = rest?;
    let words: Vec<&str> = rest.split(' ').collect();
    // the number must be followed by «процент…» / «%» / «percent» (a bare number is not a brightness: «включи 5 свет» is not this)
    let pos = words.iter().enumerate().find_map(|(i, w)| {
        let digits = w.trim_end_matches('%');
        let v: u32 = digits.parse().ok()?;
        let followed = w.ends_with('%') || words.get(i + 1).map(|x| x.starts_with("процент") || *x == "percent" || *x == "%").unwrap_or(false);
        (followed && v <= 100).then_some((i, v as u8, w.ends_with('%')))
    })?;
    let (i, v, glued) = pos;
    let skip = if glued { 1 } else { 2 };
    // the number and its «процентов», and the little word before the number («на 30 процентов», «to 40 percent»), are not the name of the device
    let before = if i > 0 && matches!(words[i - 1], "на" | "до" | "to" | "at") { i - 1 } else { i };
    let name: Vec<&str> = words.iter().enumerate().filter(|(k, w)| !(*k >= before && *k < i + skip) && !matches!(**w, "the" | "яркость")).map(|(_, w)| *w).collect();
    let name = name.join(" ");
    if name.is_empty() {
        return None;
    }
    Some((v, name))
}

// «включен ли свет в гостиной» / «горит ли лампа» / «is the kitchen light on» → "свет в гостиной"
pub fn parse_state_query(text: &str) -> Option<String> {
    let n = textnorm::normalize(text);
    for p in ["включен ли ", "включена ли ", "выключен ли ", "выключена ли ", "горит ли ", "работает ли ", "открыта ли ", "открыт ли ", "is the ", "is "] {
        if let Some(r) = n.strip_prefix(p) {
            let r = r.trim_end_matches(" on").trim_end_matches(" off").trim_end_matches(" включен").trim_end_matches(" включена").trim().to_string();
            // «is the …» only when it ends with on / off (else it is any question)
            if (p == "is the " || p == "is ") && !(n.ends_with(" on") || n.ends_with(" off")) {
                return None;
            }
            return Some(r).filter(|r| !r.is_empty());
        }
    }
    None
}

// what the server says about a device, in words (the words of the state are the ones Home Assistant uses)
pub fn state_words(d: &Device) -> String {
    let s = d.state.as_str();
    match (d.domain.as_str(), s) {
        (_, "on") => "включено".into(),
        (_, "off") => "выключено".into(),
        ("cover", "open") => "открыто".into(),
        ("cover", "closed") => "закрыто".into(),
        ("cover", "opening") => "открывается".into(),
        ("cover", "closing") => "закрывается".into(),
        ("media_player", "playing") => "играет".into(),
        ("media_player", "paused") => "на паузе".into(),
        ("media_player", "idle") => "в ожидании".into(),
        (_, "unavailable") => "недоступно (устройство не отвечает серверу)".into(),
        (_, other) => format!("состояние «{}»", other),
    }
}

#[cfg(feature = "lua")]
pub fn set_brightness(d: &Device, pct: u8) -> Result<(), String> {
    perms::check(perms::Cap::SmartHome)?;
    if d.domain != "light" {
        return Err(format!("У «{}» нет яркости: это не лампа.", d.name));
    }
    let (code, _) = http("POST", "/api/services/light/turn_on", Some(json!({ "entity_id": d.entity_id, "brightness_pct": pct.clamp(1, 100) })))?;
    match code {
        200 | 201 => Ok(()),
        401 | 403 => Err("Home Assistant не принял токен.".into()),
        404 => Err("Home Assistant не знает такое устройство или действие.".into()),
        c => Err(format!("Home Assistant ответил кодом {}", c)),
    }
}
#[cfg(not(feature = "lua"))]
pub fn set_brightness(_d: &Device, _pct: u8) -> Result<(), String> {
    Err("нет сети в этой сборке".into())
}

// the devices the server listed last time (to recognise a home phrase when the server does not answer)
static LAST_DEVICES: parking_lot::Mutex<Vec<Device>> = parking_lot::const_mutex(Vec::new());

// words of things at home: «выключи свет» must get the honest error even when no device list is known yet
const HOME_WORDS: [&str; 12] = ["свет", "лампа", "люстра", "розетка", "вентилятор", "штора", "жалюзи", "light", "lamp", "fan", "curtain", "chiroq"];

// Some(error) when the phrase is about the home (a known device or a word of the home), None: it is an ordinary command
fn failure_for(known: &[Device], what: &str, err: String) -> Option<String> {
    let n = textnorm::normalize(what);
    let about_home = !matches!(pick(known, what), Pick::None) || n.split(' ').any(|w| HOME_WORDS.iter().any(|h| textnorm::same_stem(h, w)));
    if about_home {
        Some(err)
    } else {
        None
    }
}

// the whole voice path: None = this phrase is not about the smart home (the normal commands go on)
pub fn handle(text: &str) -> Option<String> {
    if !is_configured() {
        return None;
    }
    // «яркость света в гостиной 50 процентов»: a lamp only; 0 means off
    if let Some((pct, what)) = parse_brightness(text) {
        return handle_picked(&what, |d| {
            if pct == 0 {
                act(d, Act::Off).map(|_| format!("{}: сервер принял команду «выключить».", d.name))
            } else {
                set_brightness(d, pct).map(|_| format!("{}: сервер принял яркость {}%.", d.name, pct))
            }
        });
    }
    // «включен ли свет в гостиной»: only reads what the server says
    if let Some(what) = parse_state_query(text) {
        return handle_picked(&what, |d| Ok(format!("{}: {}.", d.name, state_words(d))));
    }
    let (a, what) = parse(text)?;
    let list = match devices() {
        Ok(l) => {
            *LAST_DEVICES.lock() = l.clone();
            l
        }
        // the server cannot be asked: the phrase is taken over ONLY when it is about the home, else «включи музыку» would get the error of Home Assistant
        Err(e) => return failure_for(&LAST_DEVICES.lock(), &what, e),
    };
    match pick(&list, &what) {
        Pick::None => None,
        Pick::Many(v) => Some(format!("Какое именно: {}?", v.iter().take(4).map(|d| format!("«{}»", d.name)).collect::<Vec<_>>().join(" или "))),
        Pick::One(d) => Some(match act(&d, a) {
            Ok(()) => format!("{}: сервер принял команду «{}».", d.name, if a == Act::On { "включить" } else { "выключить" }),
            Err(e) => e,
        }),
    }
}

// the device a phrase is about, asked of the server; `f` does the work. None: the phrase is not about a known device (the normal commands go on)
fn handle_picked(what: &str, f: impl FnOnce(&Device) -> Result<String, String>) -> Option<String> {
    let list = match devices() {
        Ok(l) => {
            *LAST_DEVICES.lock() = l.clone();
            l
        }
        Err(e) => return failure_for(&LAST_DEVICES.lock(), what, e),
    };
    match pick(&list, what) {
        Pick::None => None,
        Pick::Many(v) => Some(format!("Какое именно: {}?", v.iter().take(4).map(|d| format!("«{}»", d.name)).collect::<Vec<_>>().join(" или "))),
        Pick::One(d) => Some(f(&d).unwrap_or_else(|e| e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    fn dev(id: &str, name: &str) -> Device {
        Device { entity_id: id.into(), name: name.into(), domain: id.split('.').next().unwrap().into(), state: "off".into() }
    }

    #[test]
    fn only_safe_domains_are_listed() {
        let v = json!([
            { "entity_id": "light.living", "state": "on", "attributes": { "friendly_name": "Гостиная" } },
            { "entity_id": "lock.front", "state": "locked", "attributes": { "friendly_name": "Входная дверь" } },
            { "entity_id": "alarm_control_panel.home", "state": "armed", "attributes": {} },
            { "entity_id": "switch.kettle", "state": "off", "attributes": {} },
            { "state": "x" }
        ]);
        let d = parse_states(&v);
        assert_eq!(d.iter().map(|x| x.entity_id.as_str()).collect::<Vec<_>>(), vec!["switch.kettle", "light.living"]);
        assert_eq!(d[0].name, "switch.kettle");
        assert!(parse_states(&json!({})).is_empty());
    }

    #[test]
    fn phrases_and_picks() {
        assert_eq!(parse("Включи свет в гостиной"), Some((Act::On, "свет в гостиной".into())));
        assert_eq!(parse("turn off the kitchen light"), Some((Act::Off, "kitchen light".into())));
        assert_eq!(parse("открой браузер"), None);
        let all = vec![dev("light.a", "Гостиная"), dev("light.b", "Кухня"), dev("switch.c", "Чайник")];
        assert!(matches!(pick(&all, "свет в гостиной"), Pick::One(d) if d.entity_id == "light.a"));
        assert!(matches!(pick(&all, "чайник"), Pick::One(d) if d.entity_id == "switch.c"));
        assert_eq!(pick(&all, "музыку"), Pick::None, "an ordinary command is not taken over");
        let two = vec![dev("light.a", "Свет спальня"), dev("light.b", "Свет спальня 2")];
        assert!(matches!(pick(&two, "свет спальня"), Pick::One(_) | Pick::Many(_)));
        assert_eq!(service(&dev("cover.x", "Штора"), Act::On), ("cover", "open_cover"));
        assert_eq!(service(&dev("light.x", "Л"), Act::Off), ("light", "turn_off"));
    }

    #[test]
    fn configuration_is_checked_and_the_token_is_not_shown() {
        store::with_temp(|| {
            crate::owner::reset_for_tests();
            assert!(configure("ftp://x", "t").is_err());
            assert!(configure("http://ha.local:8123", "").is_err());
            configure("http://ha.local:8123/", "secret-token").unwrap();
            let s = status_json().to_string();
            assert!(!s.contains("secret-token"));
            assert!(is_configured());
            assert!(handle("открой браузер").is_none());
            forget();
            assert!(!is_configured());
            assert!(handle("включи свет").is_none(), "not configured: not taken over");
        });
    }

    // a tiny HTTP server that plays Home Assistant
    fn serve(responses: Vec<(u16, String)>) -> (String, std::sync::mpsc::Receiver<String>) {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for (code, body) in responses {
                let Ok((mut s, _)) = l.accept() else { return };
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                tx.send(String::from_utf8_lossy(&buf[..n]).to_string()).ok();
                let _ = write!(s, "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", code, body.len(), body);
            }
        });
        (format!("http://{}", addr), rx)
    }

    #[test]
    fn the_command_goes_to_the_server_and_its_answer_decides() {
        store::with_temp(|| {
            crate::owner::reset_for_tests();
            let states = json!([{ "entity_id": "light.living", "state": "off", "attributes": { "friendly_name": "Гостиная" } }]).to_string();
            let (url, rx) = serve(vec![(200, states.clone()), (200, "[]".into()), (200, states.clone()), (401, "{}".into())]);
            configure(&url, "tok123").unwrap();
            // permission off (smart home is off by default): refused without any request
            assert!(handle("включи свет в гостиной").unwrap().contains("центре разрешений"));
            perms::set(perms::Cap::SmartHome, true).unwrap();
            let r = handle("включи свет в гостиной").unwrap();
            assert!(r.contains("сервер принял"), "{}", r);
            let get = rx.recv().unwrap();
            assert!(get.starts_with("GET /api/states") && get.to_lowercase().contains("authorization: bearer tok123"), "{}", get);
            let post = rx.recv().unwrap();
            assert!(post.starts_with("POST /api/services/light/turn_on") && post.contains("light.living"), "{}", post);
            // the server refuses the token
            let r = handle("выключи свет в гостиной").unwrap();
            assert!(r.contains("не принял токен"), "{}", r);
        });
    }

    #[test]
    fn brightness_and_state_phrases_are_understood_exactly() {
        assert_eq!(parse_brightness("Яркость света в гостиной 50 процентов"), Some((50, "света в гостиной".into())));
        assert_eq!(parse_brightness("сделай свет на кухне на 30 процентов"), Some((30, "свет на кухне".into())));
        assert_eq!(parse_brightness("сделай лампу 70%"), Some((70, "лампу".into())));
        assert_eq!(parse_brightness("set the kitchen light to 40 percent"), Some((40, "kitchen light".into())));
        assert_eq!(parse_brightness("сделай 50 процентов"), None, "no device");
        assert_eq!(parse_brightness("сделай громкость 50"), None, "a bare number is not a brightness");
        assert_eq!(parse_brightness("яркость 150 процентов"), None, "more than 100");
        assert_eq!(parse_brightness("открой браузер"), None);
        assert_eq!(parse_state_query("Включен ли свет в гостиной?"), Some("свет в гостиной".into()));
        assert_eq!(parse_state_query("горит ли лампа"), Some("лампа".into()));
        assert_eq!(parse_state_query("is the kitchen light on"), Some("kitchen light".into()));
        assert_eq!(parse_state_query("is it raining"), None, "any other question");
        assert_eq!(parse_state_query("включи свет"), None);
        let d = |dom: &str, st: &str| Device { entity_id: format!("{}.x", dom), name: "Х".into(), domain: dom.into(), state: st.into() };
        assert_eq!(state_words(&d("light", "on")), "включено");
        assert_eq!(state_words(&d("cover", "closed")), "закрыто");
        assert!(state_words(&d("light", "unavailable")).contains("недоступно"));
        assert!(state_words(&d("fan", "weird")).contains("weird"), "an unknown state is shown as the server said it");
    }

    #[test]
    fn brightness_goes_to_a_lamp_only_and_a_question_only_reads() {
        store::with_temp(|| {
            crate::owner::reset_for_tests();
            let states = json!([
                { "entity_id": "light.living", "state": "on", "attributes": { "friendly_name": "Гостиная" } },
                { "entity_id": "fan.vent", "state": "off", "attributes": { "friendly_name": "Вентилятор" } }
            ]).to_string();
            // 1 a question (list), 2 a brightness (list + post), 3 a fan (list: refused before any post), 4 off at 0% (list + post)
            let (url, rx) = serve(vec![(200, states.clone()), (200, states.clone()), (200, "[]".into()), (200, states.clone()), (200, states.clone()), (200, "[]".into())]);
            configure(&url, "tok").unwrap();
            perms::set(perms::Cap::SmartHome, true).unwrap();
            let r = handle("включен ли свет в гостиной").unwrap();
            assert_eq!(r, "Гостиная: включено.", "{}", r);
            assert!(rx.recv().unwrap().starts_with("GET /api/states"));
            let r = handle("яркость света в гостиной 40 процентов").unwrap();
            assert!(r.contains("яркость 40%"), "{}", r);
            assert!(rx.recv().unwrap().starts_with("GET /api/states"));
            let post = rx.recv().unwrap();
            assert!(post.starts_with("POST /api/services/light/turn_on") && post.contains("\"brightness_pct\":40") && post.contains("light.living"), "{}", post);
            let r = handle("яркость вентилятора 40 процентов").unwrap();
            assert!(r.contains("нет яркости"), "a fan has no brightness: {}", r);
            assert!(rx.recv().unwrap().starts_with("GET /api/states"));
            let r = handle("яркость света в гостиной 0 процентов").unwrap();
            assert!(r.contains("выключить"), "{}", r);
            assert!(rx.recv().unwrap().starts_with("GET /api/states"));
            assert!(rx.recv().unwrap().starts_with("POST /api/services/light/turn_off"));
        });
    }

    #[test]
    fn an_ordinary_phrase_is_not_taken_over_when_the_server_cannot_be_asked() {
        let err = "Home Assistant не ответил".to_string();
        let known = vec![dev("light.a", "Гостиная")];
        assert_eq!(failure_for(&[], "музыку", err.clone()), None, "«включи музыку» goes on to the normal commands");
        assert_eq!(failure_for(&known, "браузер", err.clone()), None);
        assert_eq!(failure_for(&[], "свет на кухне", err.clone()), Some(err.clone()), "a word of the home gets the honest error");
        assert_eq!(failure_for(&known, "гостиную", err.clone()), Some(err.clone()), "a known device gets it too");
        assert_eq!(failure_for(&[], "the kitchen light", err.clone()), Some(err));
    }

    #[test]
    fn with_the_server_down_an_ordinary_phrase_is_left_to_the_normal_commands() {
        store::with_temp(|| {
            crate::owner::reset_for_tests();
            // a port nobody listens on
            let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", free.local_addr().unwrap());
            drop(free);
            configure(&url, "tok").unwrap();
            perms::set(perms::Cap::SmartHome, true).unwrap();
            LAST_DEVICES.lock().clear();
            assert!(handle("включи музыку").is_none());
            assert!(handle("включи свет").unwrap().contains("не ответил"));
        });
    }
}
