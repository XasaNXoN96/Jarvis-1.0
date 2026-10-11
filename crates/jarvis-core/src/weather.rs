// Weather for the home page: real data of open-meteo.com (no key, no account). The city is the one the user chose in the
// settings; the location of the computer is never guessed. Only the city NAME goes to the Internet, and only for this.
// The result is kept with the time of the update; when it cannot be obtained the state says why, nothing is invented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

const GEO: &str = "https://geocoding-api.open-meteo.com/v1/search";
const API: &str = "https://api.open-meteo.com/v1/forecast";
const REFRESH: Duration = Duration::from_secs(20 * 60);

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Weather {
    pub place: String,       // the place as the service names it ("Ташкент, Узбекистан")
    pub temperature: f32,    // °C
    pub text: String,        // "облачно"
    pub icon: String,
    pub wind: Option<f32>,   // m/s
    pub updated: u64,        // unix seconds
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    NoCity,
    Loading,
    Ok(Weather),
    Error(String),
}

struct Cache {
    city: String,
    state: State,
    last_try: Option<std::time::Instant>,
}

static CACHE: Lazy<Mutex<Cache>> = Lazy::new(|| Mutex::new(Cache { city: String::new(), state: State::NoCity, last_try: None }));
static BUSY: AtomicBool = AtomicBool::new(false);

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// WMO weather code -> (text, icon)
pub fn wmo(code: i64, day: bool) -> (&'static str, &'static str) {
    match code {
        0 => ("ясно", if day { "☀️" } else { "🌙" }),
        1 => ("в основном ясно", if day { "🌤️" } else { "🌙" }),
        2 => ("переменная облачность", "⛅"),
        3 => ("пасмурно", "☁️"),
        45 | 48 => ("туман", "🌫️"),
        51 | 53 | 55 | 56 | 57 => ("морось", "🌦️"),
        61 | 63 | 65 | 66 | 67 => ("дождь", "🌧️"),
        71 | 73 | 75 | 77 | 85 | 86 => ("снег", "🌨️"),
        80 | 81 | 82 => ("ливень", "🌧️"),
        95 | 96 | 99 => ("гроза", "⛈️"),
        _ => ("без описания", "🌡️"),
    }
}

// the first place of a geocoding answer: (name for the screen, latitude, longitude)
pub fn parse_geocode(v: &Value) -> Option<(String, f64, f64)> {
    let r = v.get("results")?.as_array()?.first()?;
    let name = r.get("name")?.as_str()?.to_string();
    let country = r.get("country").and_then(|c| c.as_str()).unwrap_or("");
    let place = if country.is_empty() { name } else { format!("{}, {}", name, country) };
    Some((place, r.get("latitude")?.as_f64()?, r.get("longitude")?.as_f64()?))
}

pub fn parse_forecast(v: &Value, place: &str) -> Option<Weather> {
    let c = v.get("current")?;
    let t = c.get("temperature_2m")?.as_f64()? as f32;
    let code = c.get("weather_code")?.as_i64()?;
    let day = c.get("is_day").and_then(|d| d.as_i64()).unwrap_or(1) == 1;
    let (text, icon) = wmo(code, day);
    Some(Weather { place: place.to_string(), temperature: (t * 10.0).round() / 10.0, text: text.into(), icon: icon.into(), wind: c.get("wind_speed_10m").and_then(|w| w.as_f64()).map(|w| w as f32), updated: now() })
}

// percent-encoding of a query value (the city name may be in any language)
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect()
}

#[cfg(feature = "lua")]
fn get_json(url: &str, query: &[(&str, String)]) -> Result<Value, String> {
    let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(8)).timeout(Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let qs: Vec<String> = query.iter().map(|(k, v)| format!("{}={}", k, enc(v))).collect();
    let resp = client.get(format!("{}?{}", url, qs.join("&"))).send().map_err(|e| if e.is_timeout() { "сервер погоды не отвечает".to_string() } else { "нет соединения с сервером погоды".to_string() })?;
    if !resp.status().is_success() {
        return Err(format!("сервер погоды ответил кодом {}", resp.status().as_u16()));
    }
    resp.json::<Value>().map_err(|_| "непонятный ответ сервера погоды".to_string())
}

#[cfg(not(feature = "lua"))]
fn get_json(_url: &str, _query: &[(&str, String)]) -> Result<Value, String> {
    Err("в этой сборке нет сетевого модуля".into())
}

// the whole request for one city (testable with other base addresses)
pub fn fetch_from(city: &str, geo: &str, api: &str) -> Result<Weather, String> {
    let g = get_json(geo, &[("name", city.to_string()), ("count", "1".into()), ("language", "ru".into()), ("format", "json".into())])?;
    let (place, lat, lon) = parse_geocode(&g).ok_or_else(|| format!("город «{}» не найден: проверьте название в настройках", city))?;
    let f = get_json(api, &[("latitude", lat.to_string()), ("longitude", lon.to_string()), ("current", "temperature_2m,weather_code,is_day,wind_speed_10m".into()), ("timezone", "auto".into())])?;
    parse_forecast(&f, &place).ok_or_else(|| "непонятный ответ сервера погоды".to_string())
}

fn refresh(city: String) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    {
        let mut c = CACHE.lock();
        c.last_try = Some(std::time::Instant::now());
        if !matches!(c.state, State::Ok(_)) || c.city != city {
            c.state = State::Loading;
        }
    }
    std::thread::spawn(move || {
        // other addresses only for the automated tests of the window (never set in normal use)
        let geo = std::env::var("JARVIS_WEATHER_GEO").unwrap_or_else(|_| GEO.to_string());
        let api = std::env::var("JARVIS_WEATHER_API").unwrap_or_else(|_| API.to_string());
        let result = fetch_from(&city, &geo, &api);
        let mut c = CACHE.lock();
        if c.city == city {
            match result {
                Ok(w) => c.state = State::Ok(w),
                // an older good reading stays on the screen with its own update time; a first failure is an error
                Err(e) => {
                    if !matches!(c.state, State::Ok(_)) {
                        c.state = State::Error(e);
                    }
                }
            }
        }
        BUSY.store(false, Ordering::SeqCst);
    });
}

// the state for the window; starts a refresh in the background when the city changed or the reading is old
pub fn json() -> Value {
    let city = crate::uistate::settings().city.trim().to_string();
    let mut start = false;
    {
        let mut c = CACHE.lock();
        if city.is_empty() {
            c.city.clear();
            c.state = State::NoCity;
        } else {
            if c.city != city {
                c.city = city.clone();
                c.state = State::Loading;
                start = true;
            } else if c.last_try.map(|t| t.elapsed() > REFRESH).unwrap_or(true) && !matches!(c.state, State::Loading) {
                start = true;
            }
        }
    }
    if start {
        refresh(city.clone());
    }
    let c = CACHE.lock();
    match &c.state {
        State::NoCity => json!({ "status": "NO_CITY", "city": "" }),
        State::Loading => json!({ "status": "LOADING", "city": city }),
        State::Error(e) => json!({ "status": "ERROR", "city": city, "message": e }),
        State::Ok(w) => json!({ "status": "OK", "city": city, "weather": w }),
    }
}

// the city was changed in the settings: the next reading is taken at once
pub fn city_changed() {
    let mut c = CACHE.lock();
    c.last_try = None;
}

#[cfg(test)]
mod tests;
