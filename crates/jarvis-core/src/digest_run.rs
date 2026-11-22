// The morning briefing at work: collects the data, asks the AI to retell it (or composes the text locally when no AI is
// connected), says it aloud and keeps the daily schedule. The rules and the texts are in digest.rs.
//
// Data sources (all local or already used by Jarvis; nothing new leaves the computer):
//   - weather: the same open-meteo reading as the home page (needs the city in the settings);
//   - reminders and timers of the next 24 hours: <config dir>/timers.json;
//   - the last notes: Documents/Jarvis-notes.txt.
// Gmail / Calendar connectors of OpenJarvis are not ported (they need a Google sign-in); the data model is ready for more sections.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{Datelike, Local, TimeZone, Timelike};

use crate::digest::{self, Cmd, Data, WeatherInfo};
use crate::origin::Source;
use crate::{ai, dialog, remote, tts, uistate, APP_CONFIG_DIR};

const HONORIFIC: &str = "сэр";
// the phrase the schedule feeds into the normal command pipeline (the same path as when the user says it)
pub const PHRASE: &str = "утренняя сводка";
const MAX_REMINDERS: usize = 6;
const MAX_NOTES: usize = 3;
const MAX_ITEM_CHARS: usize = 120;

pub fn weekday_ru(days_from_monday: u32) -> &'static str {
    ["понедельник", "вторник", "среда", "четверг", "пятница", "суббота", "воскресенье"][(days_from_monday % 7) as usize]
}

fn settings_path() -> Option<PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join("digest.json"))
}

fn short(s: &str) -> String {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    one.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(MAX_ITEM_CHARS).collect()
}

// the weather of the home page; a reading that is still loading is waited for a few seconds
fn weather_info() -> Option<WeatherInfo> {
    for _ in 0..16 {
        let v = crate::weather::json();
        match v["status"].as_str() {
            Some("OK") => {
                let w = &v["weather"];
                return Some(WeatherInfo {
                    place: w["place"].as_str()?.to_string(),
                    temperature: w["temperature"].as_f64()? as f32,
                    text: w["text"].as_str().unwrap_or("").to_string(),
                    wind: w["wind"].as_f64().map(|x| x as f32),
                });
            }
            Some("LOADING") => std::thread::sleep(Duration::from_millis(500)),
            _ => return None,
        }
    }
    None
}

// "в 09:00 — позвонить маме" for the timers and reminders that fall within the next 24 hours
pub fn reminders_from_json(text: &str, now_unix: i64) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    let Some(list) = v.as_array() else { return Vec::new() };
    let mut items: Vec<(i64, String)> = Vec::new();
    for t in list {
        let Some(due) = t["due"].as_i64() else { continue };
        if due < now_unix || due > now_unix + 24 * 3600 {
            continue;
        }
        let msg = t["message"].as_str().filter(|m| !m.trim().is_empty()).or_else(|| t["title"].as_str()).unwrap_or("");
        let msg = short(msg);
        if msg.is_empty() {
            continue;
        }
        items.push((due, msg));
    }
    items.sort();
    let today = Local.timestamp_opt(now_unix, 0).single().map(|d| d.date_naive());
    items
        .into_iter()
        .take(MAX_REMINDERS)
        .filter_map(|(due, msg)| {
            let when = Local.timestamp_opt(due, 0).single()?;
            let day = if Some(when.date_naive()) == today { "" } else { "завтра " };
            Some(format!("{}в {:02}:{:02} — {}", day, when.hour(), when.minute(), msg))
        })
        .collect()
}

fn reminders(now_unix: i64) -> Vec<String> {
    let Some(dir) = APP_CONFIG_DIR.get() else { return Vec::new() };
    std::fs::read_to_string(dir.join("timers.json")).map(|t| reminders_from_json(&t, now_unix)).unwrap_or_default()
}

// the last notes without the "[2026-10-10 14:05] " stamp
pub fn notes_from_text(text: &str) -> Vec<String> {
    let lines: Vec<String> = text
        .lines()
        .map(|l| {
            let l = l.trim();
            match (l.starts_with('['), l.find(']')) {
                (true, Some(i)) => l[i + 1..].trim().to_string(),
                _ => l.to_string(),
            }
        })
        .map(|l| short(&l))
        .filter(|l| !l.is_empty())
        .collect();
    let skip = lines.len().saturating_sub(MAX_NOTES);
    lines.into_iter().skip(skip).collect()
}

fn notes() -> Vec<String> {
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    if home.is_empty() {
        return Vec::new();
    }
    std::fs::read_to_string(PathBuf::from(home).join("Documents").join("Jarvis-notes.txt")).map(|t| notes_from_text(&t)).unwrap_or_default()
}

// the headlines of the feeds (only when the owner switched the news on); a feed that does not answer is skipped
fn news() -> Vec<String> {
    let Some(path) = settings_path() else { return Vec::new() };
    let s = digest::load_settings(&path);
    if !s.news {
        return Vec::new();
    }
    let mut all: Vec<String> = Vec::new();
    for feed in digest::feeds_of(&s) {
        match fetch_feed(&feed) {
            Ok(xml) => all.extend(digest::parse_rss_titles(&xml, 2)),
            Err(e) => {
                warn!("Digest: the feed {} was not read: {}", feed, e);
                uistate::log_event("digest", "Ленту новостей прочитать не удалось");
            }
        }
    }
    digest::safe_headlines(all)
}

// One feed: the address is checked (no local addresses, also after a redirect), the answer is cut at one megabyte.
#[cfg(feature = "lua")]
fn fetch_feed(url: &str) -> Result<String, String> {
    use std::io::Read;
    crate::shield::check_url_resolving(url)?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 3 {
                attempt.error("слишком много перенаправлений")
            } else if crate::shield::check_url(attempt.url().as_str()).is_err() {
                attempt.error("перенаправление на недопустимый адрес")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).header("User-Agent", "Jarvis-digest").send().map_err(|_| "нет связи с сервером ленты".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("сервер ответил {}", resp.status()));
    }
    let mut body = Vec::new();
    resp.take(1_000_000).read_to_end(&mut body).map_err(|_| "ответ оборвался".to_string())?;
    Ok(String::from_utf8_lossy(&body).to_string())
}

#[cfg(not(feature = "lua"))]
fn fetch_feed(_url: &str) -> Result<String, String> {
    Err("в этой сборке нет сетевого модуля".into())
}

pub fn collect() -> Data {
    let now = Local::now();
    Data {
        weekday: weekday_ru(now.weekday().num_days_from_monday()).to_string(),
        day: now.day(),
        month: now.month(),
        hour: now.hour(),
        weather: weather_info(),
        reminders: reminders(now.timestamp()),
        notes: notes(),
        news: news(),
    }
}

// the text of the briefing and whether the AI wrote it
pub fn compose(data: &Data) -> (String, bool) {
    if ai::is_configured() {
        let now = Local::now();
        let stamp = format!("{}, {:02}.{:02}.{}, {:02}:{:02}", data.weekday, now.day(), now.month(), now.year(), now.hour(), now.minute());
        match ai::complete_once(&digest::system_prompt(HONORIFIC, &stamp), &digest::user_message(data)) {
            Ok(text) => {
                let clean = digest::clean_for_speech(&text);
                if !clean.is_empty() {
                    return (clean, true);
                }
            }
            Err(e) => {
                warn!("Digest: the AI could not write the briefing: {:?}", e);
                uistate::log_event("digest", "ИИ недоступен: сводка составлена без него");
            }
        }
    }
    (digest::local_briefing(data, HONORIFIC), false)
}

// the briefing in the background: the app keeps listening meanwhile (the AI may take seconds)
pub fn run_in_background() {
    if !crate::guard::first("digest:run", Duration::from_secs(30)) {
        return;
    }
    std::thread::spawn(|| {
        let data = collect();
        let (text, by_ai) = compose(&data);
        uistate::log_event("digest", if by_ai { "Утренняя сводка (ИИ)" } else { "Утренняя сводка (без ИИ)" });
        // a briefing written by the AI belongs to the AI conversation (a follow-up question "а что с погодой?" sees it)
        dialog::reply(&text, if by_ai { "ai" } else { "local" });
        if tts::is_enabled() {
            tts::speak(&ai::speakable(&text));
        }
    });
}

fn say(text: &str) {
    dialog::reply(text, "local");
    if tts::is_enabled() {
        tts::speak(text);
    }
}

// a spoken or typed command about the briefing; the reply goes into the conversation of the request that is open now
pub fn handle(cmd: Cmd) {
    let Some(path) = settings_path() else {
        if cmd == Cmd::Now {
            dialog::reply("Секунду, сэр, собираю данные.", "local");
            run_in_background();
        } else {
            say("Не могу сохранить настройку: нет папки настроек.");
        }
        return;
    };
    let mut s = digest::load_settings(&path);
    let now = Local::now();
    match cmd {
        Cmd::Now => {
            dialog::reply("Секунду, сэр, собираю данные.", "local");
            run_in_background();
        }
        Cmd::Enable(time) => {
            digest::apply_enable(&mut s, time, now.hour(), now.minute(), &now.format("%Y-%m-%d").to_string());
            if digest::save_settings(&path, &s) {
                say(&digest::describe(&s));
            } else {
                say("Не удалось сохранить настройку сводки.");
            }
        }
        Cmd::Disable => {
            s.enabled = false;
            digest::save_settings(&path, &s);
            say(&digest::describe(&s));
        }
        Cmd::Status => say(&format!("{} {}", digest::describe(&s), digest::describe_news(&s))),
        Cmd::News(on) => {
            s.news = on;
            if digest::save_settings(&path, &s) {
                let more = if on { " Читаю заголовки с сайта BBC Русская служба; другие ленты можно указать в файле digest.json." } else { "" };
                say(&format!("{}{}", digest::describe_news(&s), more));
            } else {
                say("Не удалось сохранить настройку сводки.");
            }
        }
    }
}

// once a day at the chosen time: feeds the phrase into the normal pipeline (off until the user switches it on)
fn tick() {
    let Some(path) = settings_path() else { return };
    let mut s = digest::load_settings(&path);
    let now = Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    // the schedule is a program's own start: it comes in like a timer does (the door «Script»)
    if digest::due(&s, now.hour(), now.minute(), &today) && remote::submit(Source::Script, PHRASE) {
        s.last_day = today;
        digest::save_settings(&path, &s);
        info!("Digest: the morning briefing was started by the schedule");
    }
}

pub fn start_scheduler() {
    std::thread::spawn(|| {
        // let the app finish starting first
        std::thread::sleep(Duration::from_secs(20));
        loop {
            tick();
            std::thread::sleep(Duration::from_secs(30));
        }
    });
}

#[cfg(test)]
mod tests;
