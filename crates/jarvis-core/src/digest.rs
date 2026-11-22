// The morning briefing of Jarvis: the data is collected FIRST (weather, reminders, notes), then the model only retells it.
//
// The scheme and the rules of the prompt follow the `morning_digest` agent of OpenJarvis (https://github.com/open-jarvis/OpenJarvis,
// src/openjarvis/agents/morning_digest.py, Apache-2.0): the model gets the collected data in the message, fetches nothing
// itself, may state only facts from the data, silently omits what is missing, no Markdown (it is spoken), a hard word limit.
// This file is the pure part (prompts, a local text without the AI, phrases, schedule); digest_run.rs does the work.
//
// Without a connected AI the briefing is composed locally from the same data, so it also works offline.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MAX_WORDS: usize = 120;
// a briefing that was due earlier than this long ago is skipped (the computer was off all morning)
const LATE_MINUTES: u32 = 180;
pub const DEFAULT_HOUR: u32 = 7;
pub const DEFAULT_MINUTE: u32 = 30;

#[derive(Clone, Debug, PartialEq)]
pub struct WeatherInfo {
    pub place: String,
    pub temperature: f32,
    pub text: String,
    pub wind: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Data {
    pub weekday: String, // "вторник"
    pub day: u32,
    pub month: u32,
    pub hour: u32,
    pub weather: Option<WeatherInfo>,
    pub reminders: Vec<String>, // "в 09:00 — позвонить маме"
    pub notes: Vec<String>,
}

impl Data {
    pub fn is_empty(&self) -> bool {
        self.weather.is_none() && self.reminders.is_empty() && self.notes.is_empty()
    }
}

// ---------------------------------------------------------------------------------------------
// words
// ---------------------------------------------------------------------------------------------
pub fn month_ru_gen(m: u32) -> &'static str {
    ["января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября", "октября", "ноября", "декабря"][((m + 11) % 12) as usize]
}

pub fn greeting(hour: u32) -> &'static str {
    match hour {
        5..=11 => "Доброе утро",
        12..=17 => "Добрый день",
        18..=22 => "Добрый вечер",
        _ => "Доброй ночи",
    }
}

fn plural(n: u32, one: &'static str, few: &'static str, many: &'static str) -> &'static str {
    let (n100, n10) = (n % 100, n % 10);
    if (11..=14).contains(&n100) {
        many
    } else if n10 == 1 {
        one
    } else if (2..=4).contains(&n10) {
        few
    } else {
        many
    }
}

// "плюс 12 градусов", "минус 1 градус", "ноль градусов"
pub fn degrees_ru(t: f32) -> String {
    let n = t.round() as i32;
    let abs = n.unsigned_abs();
    let unit = plural(abs, "градус", "градуса", "градусов");
    match n {
        0 => "ноль градусов".to_string(),
        n if n > 0 => format!("плюс {} {}", abs, unit),
        _ => format!("минус {} {}", abs, unit),
    }
}

fn wind_ru(w: f32) -> String {
    let n = w.round().max(0.0) as u32;
    format!("{} {} в секунду", n, plural(n, "метр", "метра", "метров"))
}

// ---------------------------------------------------------------------------------------------
// prompts
// ---------------------------------------------------------------------------------------------
pub fn system_prompt(honorific: &str, stamp: &str) -> String {
    format!(
"Ты — ДЖАРВИС, личный ИИ-дворецкий пользователя: спокойный, безупречно вежливый, с сухим британским юмором. Сейчас ты зачитываешь устную утреннюю сводку.

Данные уже собраны и приведены в сообщении пользователя: ты ничего не ищешь и не запрашиваешь сам.
Сейчас: {stamp}. Обращение к пользователю: «{honorific}».

ЖЁСТКИЕ ПРАВИЛА (нарушать нельзя):
— Только факты из данных. Ничего не выдумывай: ни дел, ни планов, ни самочувствия, ни событий.
— Если по какой-то теме данных нет, молча пропусти её. Не говори, что данных нет, и не упоминай недоступные источники.
— Начни кратко с приветствия и обращения, закончи сразу после последнего пункта. Обращение используй два-три раза за всю сводку, не в каждой фразе.
— Порядок: сначала важное и срочное (напоминания на ближайшие часы), потом погода, потом заметки.
— Без Markdown, списков, заголовков и эмодзи: текст озвучивается голосом.
— Не предлагай действий и не задавай вопросов.
— Допустима одна короткая сухая шутка, если она не искажает факты.
— Не больше {words} слов.",
        stamp = stamp,
        honorific = honorific,
        words = MAX_WORDS
    )
}

pub fn user_message(d: &Data) -> String {
    let mut out = String::from("Эти данные — единственный источник фактов для сводки:\n\n<данные>\n");
    out.push_str(&format!("ДАТА: {}, {} {}\n", d.weekday, d.day, month_ru_gen(d.month)));
    if let Some(w) = &d.weather {
        out.push_str(&format!("ПОГОДА: {}: {}, {}", w.place, degrees_ru(w.temperature), w.text));
        if let Some(wind) = w.wind {
            out.push_str(&format!(", ветер {}", wind_ru(wind)));
        }
        out.push('\n');
    }
    if !d.reminders.is_empty() {
        out.push_str("НАПОМИНАНИЯ БЛИЖАЙШИХ 24 ЧАСОВ:\n");
        for r in &d.reminders {
            out.push_str(&format!("- {}\n", r));
        }
    }
    if !d.notes.is_empty() {
        out.push_str("ПОСЛЕДНИЕ ЗАМЕТКИ ПОЛЬЗОВАТЕЛЯ:\n");
        for n in &d.notes {
            out.push_str(&format!("- {}\n", n));
        }
    }
    out.push_str("</данные>\n\n");
    if d.is_empty() {
        out.push_str("Кроме даты, данных нет: поприветствуй, назови дату и закончи.");
    } else {
        out.push_str("Используй только эти данные, ничего личного не добавляй. Обращение не больше трёх раз.");
    }
    out.push_str(&format!(" Не больше {} слов.", MAX_WORDS));
    out
}

// the briefing without the AI: the same data, plain sentences
pub fn local_briefing(d: &Data, honorific: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    parts.push(format!("{}, {}. Сегодня {}, {} {}.", greeting(d.hour), honorific, d.weekday, d.day, month_ru_gen(d.month)));
    if !d.reminders.is_empty() {
        parts.push(format!("Ближайшие напоминания: {}.", d.reminders.join("; ")));
    }
    if let Some(w) = &d.weather {
        let mut s = format!("Погода, {}: {}, {}", w.place, degrees_ru(w.temperature), w.text);
        if let Some(wind) = w.wind {
            s.push_str(&format!(", ветер {}", wind_ru(wind)));
        }
        s.push('.');
        parts.push(s);
    }
    if !d.notes.is_empty() {
        parts.push(format!("Последние заметки: {}.", d.notes.join("; ")));
    }
    if d.is_empty() {
        parts.push("Новостей для вас нет, можно начинать день.".to_string());
    }
    parts.join(" ")
}

// Markdown that slipped into a spoken text (headers, bullets, **bold**), as OpenJarvis strips it before the speech
pub fn clean_for_speech(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let mut l = line.trim_start().to_string();
        let hashes = l.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && hashes <= 6 && l.chars().nth(hashes) == Some(' ') {
            l = l[hashes + 1..].to_string();
        }
        for b in ["- ", "* ", "• "] {
            if let Some(rest) = l.strip_prefix(b) {
                l = rest.to_string();
                break;
            }
        }
        lines.push(l.replace("**", "").replace('*', ""));
    }
    lines.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------------------------
// phrases
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cmd {
    Now,
    Enable(Option<(u32, u32)>),
    Disable,
    Status,
}

fn norm(text: &str) -> String {
    text.to_lowercase().replace('ё', "е")
}

pub fn parse_command(text: &str) -> Option<Cmd> {
    let t = norm(text);
    let has = |words: &[&str]| words.iter().any(|w| t.contains(w));
    // "расскажи сводку новостей" is not the morning briefing
    if has(&["новост"]) {
        return None;
    }
    let topic = t.contains("сводк") || t.contains("брифинг") || t.contains("доложи обстановку") || t.contains("доклад на сегодня") || t.contains("что у нас на сегодня") || t.contains("что у меня на сегодня");
    if !topic {
        return None;
    }
    if has(&["выключи", "отключи", "отмени", "убери", "не нужна", "не надо", "останови", "хватит"]) {
        return Some(Cmd::Disable);
    }
    if has(&["когда", "во сколько", "статус"]) {
        return Some(Cmd::Status);
    }
    if has(&["включи", "подключи", "настрой", "каждое утро", "каждый день", "ежедневн", "поставь", "установи", "по утрам"]) {
        return Some(Cmd::Enable(parse_time(&t)));
    }
    Some(Cmd::Now)
}

// "в 7:30", "7.30", "в 8 вечера", "в 7 30", and (when the script engine is built in) "в семь тридцать"
pub fn parse_time(text: &str) -> Option<(u32, u32)> {
    let t = norm(text);
    let tokens: Vec<&str> = t.split_whitespace().collect();
    // a token like 7:30 / 07.30
    for tok in &tokens {
        let tok = tok.trim_matches(|c: char| !c.is_ascii_digit());
        if let Some((h, m)) = tok.split_once(|c| c == ':' || c == '.') {
            if let (Ok(h), Ok(m)) = (h.parse::<u32>(), m.parse::<u32>()) {
                if h < 24 && m < 60 && tok.len() <= 5 {
                    return Some((apply_day_part(h, &tokens), m));
                }
            }
        }
    }
    // "в 7", "в 7 30", "в 8 вечера"
    for (i, tok) in tokens.iter().enumerate() {
        if *tok != "в" {
            continue;
        }
        let Some(h) = tokens.get(i + 1).and_then(|s| s.parse::<u32>().ok()) else { continue };
        if h > 24 {
            continue;
        }
        let m = tokens.get(i + 2).and_then(|s| s.parse::<u32>().ok()).filter(|m| *m < 60 && s_is_minutes(tokens[i + 2])).unwrap_or(0);
        return Some((apply_day_part(h % 24, &tokens), m));
    }
    #[cfg(feature = "lua")]
    {
        if let Some((moment, _)) = crate::lua::api::parse::parse_clock_at(text, chrono::Local::now().naive_local()) {
            use chrono::Timelike;
            return Some((moment.hour(), moment.minute()));
        }
    }
    None
}

// a bare number after the hour is minutes ("в 7 30"), not the start of something else ("в 7 утра")
fn s_is_minutes(s: &str) -> bool {
    s.chars().all(|c| c.is_ascii_digit()) && !s.is_empty() && s.len() <= 2
}

fn apply_day_part(h: u32, tokens: &[&str]) -> u32 {
    let has = |w: &str| tokens.iter().any(|t| t.starts_with(w));
    if (has("вечер") || has("дня") || has("днем")) && h < 12 {
        h + 12
    } else if has("ночи") && h == 12 {
        0
    } else {
        h
    }
}

// ---------------------------------------------------------------------------------------------
// schedule
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub enabled: bool,
    pub hour: u32,
    pub minute: u32,
    #[serde(default)]
    pub last_day: String, // the day the briefing was last started: "2026-11-17"
}

impl Default for Settings {
    fn default() -> Self {
        Settings { enabled: false, hour: DEFAULT_HOUR, minute: DEFAULT_MINUTE, last_day: String::new() }
    }
}

pub fn load_settings(path: &Path) -> Settings {
    let Ok(text) = std::fs::read_to_string(path) else { return Settings::default() };
    let mut s: Settings = serde_json::from_str(&text).unwrap_or_default();
    if s.hour > 23 || s.minute > 59 {
        s.hour = DEFAULT_HOUR;
        s.minute = DEFAULT_MINUTE;
    }
    s
}

pub fn save_settings(path: &Path, s: &Settings) -> bool {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(s) {
        Ok(text) => std::fs::write(path, text).is_ok(),
        Err(_) => false,
    }
}

// is it time? once a day, from the chosen time up to LATE_MINUTES later (a computer switched on at noon does not read the morning briefing)
pub fn due(s: &Settings, hour: u32, minute: u32, today: &str) -> bool {
    if !s.enabled || s.last_day == today {
        return false;
    }
    let (now, at) = (hour * 60 + minute, s.hour * 60 + s.minute);
    now >= at && now - at <= LATE_MINUTES
}

// switches the schedule on (optionally with a new time). When the chosen time of today has passed already, the first
// briefing is tomorrow's, not one in the middle of the day.
pub fn apply_enable(s: &mut Settings, time: Option<(u32, u32)>, now_hour: u32, now_minute: u32, today: &str) {
    if let Some((h, m)) = time {
        s.hour = h;
        s.minute = m;
    }
    s.enabled = true;
    if now_hour * 60 + now_minute >= s.hour * 60 + s.minute {
        s.last_day = today.to_string();
    } else {
        s.last_day.clear();
    }
}

pub fn describe(s: &Settings) -> String {
    if s.enabled {
        format!("Утренняя сводка включена: каждый день в {:02}:{:02}.", s.hour, s.minute)
    } else {
        format!("Утренняя сводка выключена. Скажите «включи утреннюю сводку в {:02}:{:02}», чтобы получать её каждый день.", s.hour, s.minute)
    }
}

#[cfg(test)]
mod tests;
