// Parse Lua API: numbers and durations spoken or typed in a phrase
//
//   jarvis.parse.number("громкость пятьдесят процентов")  -> 50
//   jarvis.parse.duration("поставь таймер на 5 минут")    -> 300, "поставь таймер"
//
// Understands digits ("5", "1.5"), Russian and English number words
// ("пять", "двадцать пять", "полтора", "five") and time units (sec/min/hour).

use mlua::{Lua, Table};

fn normalize(token: &str) -> String {
    token
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
        .replace('ё', "е")
}

// value of a number word, if the token is one
fn number_word(t: &str) -> Option<f64> {
    let v = match t {
        // russian
        "ноль" | "нуль" => 0.0,
        "один" | "одна" | "одну" | "одно" => 1.0,
        "два" | "две" => 2.0,
        "три" => 3.0,
        "четыре" => 4.0,
        "пять" => 5.0,
        "шесть" => 6.0,
        "семь" => 7.0,
        "восемь" => 8.0,
        "девять" => 9.0,
        "десять" => 10.0,
        "одиннадцать" => 11.0,
        "двенадцать" => 12.0,
        "тринадцать" => 13.0,
        "четырнадцать" => 14.0,
        "пятнадцать" => 15.0,
        "шестнадцать" => 16.0,
        "семнадцать" => 17.0,
        "восемнадцать" => 18.0,
        "девятнадцать" => 19.0,
        "двадцать" => 20.0,
        "тридцать" => 30.0,
        "сорок" => 40.0,
        "пятьдесят" => 50.0,
        "шестьдесят" => 60.0,
        "семьдесят" => 70.0,
        "восемьдесят" => 80.0,
        "девяносто" => 90.0,
        "сто" => 100.0,
        "двести" => 200.0,
        "триста" => 300.0,
        "четыреста" => 400.0,
        "пятьсот" => 500.0,
        "шестьсот" => 600.0,
        "семьсот" => 700.0,
        "восемьсот" => 800.0,
        "девятьсот" => 900.0,
        // english
        "zero" => 0.0,
        "one" => 1.0,
        "two" => 2.0,
        "three" => 3.0,
        "four" => 4.0,
        "five" => 5.0,
        "six" => 6.0,
        "seven" => 7.0,
        "eight" => 8.0,
        "nine" => 9.0,
        "ten" => 10.0,
        "eleven" => 11.0,
        "twelve" => 12.0,
        "thirteen" => 13.0,
        "fourteen" => 14.0,
        "fifteen" => 15.0,
        "sixteen" => 16.0,
        "seventeen" => 17.0,
        "eighteen" => 18.0,
        "nineteen" => 19.0,
        "twenty" => 20.0,
        "thirty" => 30.0,
        "forty" => 40.0,
        "fifty" => 50.0,
        "sixty" => 60.0,
        "seventy" => 70.0,
        "eighty" => 80.0,
        "ninety" => 90.0,
        "hundred" => 100.0,
        _ => return None,
    };
    Some(v)
}

// "полтора" / "half" are fractions, not part of a sum
fn fraction_word(t: &str) -> Option<f64> {
    match t {
        "полтора" | "полторы" => Some(1.5),
        "half" => Some(0.5),
        _ => None,
    }
}

// seconds in a unit, by word prefix
fn unit_seconds(t: &str) -> Option<f64> {
    // (Uzbek in Latin letters: soniya, sekund, daqiqa, minut, soat; only timers and reminders ask for a duration, «soat necha» is another command)
    if t.starts_with("сек") || t.starts_with("second") || t == "sec" || t == "secs" || t.starts_with("soniy") || t.starts_with("sekund") {
        Some(1.0)
    } else if t.starts_with("мин") || t.starts_with("minute") || t == "min" || t == "mins" || t.starts_with("daqiq") || t.starts_with("minut") {
        Some(60.0)
    } else if t.starts_with("час") || t.starts_with("hour") || t == "hr" || t == "hrs" || t.starts_with("soat") {
        Some(3600.0)
    } else {
        None
    }
}

// "полчаса" / "полминуты" -> seconds
fn half_unit(t: &str) -> Option<f64> {
    match t {
        "полчаса" => Some(1800.0),
        "полминуты" => Some(30.0),
        _ => None,
    }
}

fn digits_value(raw: &str) -> Option<f64> {
    let cleaned = raw.trim_matches(|c: char| !c.is_ascii_digit());
    if cleaned.is_empty() || !cleaned.chars().next()?.is_ascii_digit() {
        return None;
    }
    cleaned.replace(',', ".").parse::<f64>().ok()
}

// "50%" / "50 процентов" etc. are handled by the caller, we only need the number
pub fn parse_number(text: &str) -> Option<f64> {
    let mut acc: Option<f64> = None;
    for raw in text.split_whitespace() {
        // digits like "50", "50%", "1,5"
        if let Some(v) = digits_value(raw) {
            return Some(acc.map(|a| a + v).unwrap_or(v));
        }
        let t = normalize(raw);
        if let Some(v) = fraction_word(&t) {
            return Some(v);
        }
        if let Some(v) = number_word(&t) {
            acc = Some(acc.unwrap_or(0.0) + v);
        } else if acc.is_some() {
            break; // number words ended
        }
    }
    acc
}

// Returns (total seconds, text without the duration part)
pub fn parse_duration(text: &str) -> Option<(u64, String)> {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut used = vec![false; tokens.len()];

    let mut total = 0.0f64;
    let mut found = false;
    // number being built: (value, token indexes)
    let mut pending: Option<(f64, Vec<usize>)> = None;

    for (i, raw) in tokens.iter().enumerate() {
        let t = normalize(raw);

        if let Some(secs) = half_unit(&t) {
            total += secs;
            found = true;
            used[i] = true;
            pending = None;
            continue;
        }

        if let Some(v) = digits_value(raw) {
            // glued units: "5min", "10м"
            let tail: String = normalize(raw.trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ','));
            if !tail.is_empty() {
                if let Some(u) = unit_seconds(&tail) {
                    total += v * u;
                    found = true;
                    used[i] = true;
                    pending = None;
                    continue;
                }
            }
            pending = Some((v, vec![i]));
            continue;
        }

        if let Some(v) = fraction_word(&t) {
            pending = Some((v, vec![i]));
            continue;
        }

        if let Some(v) = number_word(&t) {
            pending = match pending.take() {
                Some((acc, mut idx)) if idx.last().map(|&j| j + 1 == i).unwrap_or(false) => {
                    idx.push(i);
                    Some((acc + v, idx))
                }
                _ => Some((v, vec![i])),
            };
            continue;
        }

        if let Some(u) = unit_seconds(&t) {
            // "через час" -> 1 hour, "5 минут" -> 5 min
            let (value, mut idx) = match pending.take() {
                Some((v, idx)) if idx.last().map(|&j| j + 1 == i).unwrap_or(false) => (v, idx),
                _ => (1.0, Vec::new()),
            };
            idx.push(i);
            total += value * u;
            found = true;
            for j in idx {
                used[j] = true;
            }
            continue;
        }

        pending = None;
    }

    if !found || total <= 0.0 {
        return None;
    }

    // drop connector words right before the removed duration ("через", "на", "in", "after")
    for i in 0..tokens.len() {
        if used[i] {
            let mut j = i;
            while j > 0 && !used[j - 1] {
                let w = normalize(tokens[j - 1]);
                if matches!(w.as_str(), "через" | "на" | "in" | "after" | "for" | "в") {
                    used[j - 1] = true;
                    j -= 1;
                } else {
                    break;
                }
            }
        }
    }

    let rest: Vec<&str> = tokens
        .iter()
        .enumerate()
        .filter(|(i, _)| !used[*i])
        .map(|(_, t)| *t)
        .collect();

    Some((total.round() as u64, rest.join(" ")))
}


// ---------- clock time: "в 7:30", "завтра в 9 утра", "в семь тридцать вечера" ----------

use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDateTime, TimeZone, Timelike};

fn genitive_hour(t: &str) -> Option<u32> {
    Some(match t {
        "первого" => 1,
        "второго" => 2,
        "третьего" => 3,
        "четвертого" => 4,
        "пятого" => 5,
        "шестого" => 6,
        "седьмого" => 7,
        "восьмого" => 8,
        "девятого" => 9,
        "десятого" => 10,
        "одиннадцатого" => 11,
        "двенадцатого" => 12,
        _ => return None,
    })
}

// number at tokens[i]: a digit token or number words ("двадцать три"); returns (value, tokens used)
fn number_at(tokens: &[String], i: usize) -> Option<(u32, usize)> {
    let t = tokens.get(i)?;
    if let Some(v) = digits_value(t) {
        if t.chars().all(|c| c.is_ascii_digit()) {
            return Some((v as u32, 1));
        }
        return None;
    }
    let v = number_word(t)? as u32;
    if (20..=90).contains(&v) && v % 10 == 0 {
        if let Some(next) = tokens.get(i + 1).and_then(|n| number_word(n)) {
            let next = next as u32;
            if (1..=9).contains(&next) {
                return Some((v + next, 2));
            }
        }
    }
    Some((v, 1))
}

// "7:30", "18.00" -> (hour, minute)
fn colon_time(raw: &str) -> Option<(u32, u32)> {
    let cleaned = raw.trim_matches(|c: char| !c.is_ascii_digit());
    let (h, m) = cleaned.split_once(|c| c == ':' || c == '.')?;
    if h.is_empty() || m.len() != 2 || !h.chars().all(|c| c.is_ascii_digit()) || !m.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    if h <= 24 && m < 60 { Some((h, m)) } else { None }
}

// Returns (moment, text without the time words). Without a day word the next such moment is used.
pub fn parse_clock_at(text: &str, now: NaiveDateTime) -> Option<(NaiveDateTime, String)> {
    let raw_tokens: Vec<&str> = text.split_whitespace().collect();
    let tokens: Vec<String> = raw_tokens.iter().map(|t| normalize(t)).collect();
    let mut used = vec![false; tokens.len()];

    // day words
    let mut day_offset: Option<i64> = None;
    for (i, t) in tokens.iter().enumerate() {
        let d = match t.as_str() {
            "сегодня" | "today" => 0,
            "завтра" | "tomorrow" => 1,
            "послезавтра" => 2,
            _ => continue,
        };
        day_offset = Some(d);
        used[i] = true;
    }

    // time itself
    let mut hm: Option<(u32, u32)> = None;
    let mut time_tokens: Vec<usize> = Vec::new();

    'find: for i in 0..tokens.len() {
        // "7:30"
        if let Some(t) = colon_time(raw_tokens[i]) {
            hm = Some(t);
            time_tokens = vec![i];
            break 'find;
        }
        // "полдень" / "полночь"
        if tokens[i] == "полдень" || tokens[i] == "noon" {
            hm = Some((12, 0));
            time_tokens = vec![i];
            break 'find;
        }
        if tokens[i] == "полночь" || tokens[i] == "midnight" {
            hm = Some((0, 0));
            time_tokens = vec![i];
            break 'find;
        }
        // "половина восьмого" -> 7:30
        if tokens[i].starts_with("половин") {
            if let Some(h) = tokens.get(i + 1).and_then(|n| genitive_hour(n)) {
                hm = Some(((h + 11) % 12 + if h > 12 { 12 } else { 0 }, 30));
                // "половина первого" = 12:30
                time_tokens = vec![i, i + 1];
                break 'find;
            }
        }
        // preposition + number: "в 9", "в семь тридцать", "at 7 30"
        if matches!(tokens[i].as_str(), "в" | "во" | "at" | "к") {
            if let Some((h, used_h)) = number_at(&tokens, i + 1) {
                if h > 24 {
                    continue;
                }
                let mut idxs: Vec<usize> = vec![i];
                idxs.extend((i + 1)..(i + 1 + used_h));
                let mut minute = 0u32;
                let next = i + 1 + used_h;
                // not a duration / date unit ("в 5 минут", "в 7 января")
                let unit_next = tokens.get(next).map(|t| unit_seconds(t).is_some()).unwrap_or(false);
                if unit_next {
                    continue;
                }
                if let Some((m, used_m)) = number_at(&tokens, next) {
                    if m < 60 && !tokens.get(next + used_m).map(|t| unit_seconds(t).is_some()).unwrap_or(false) {
                        minute = m;
                        idxs.extend(next..(next + used_m));
                    } else if m == 0 {
                        // "ноль пять"
                    }
                }
                hm = Some((h, minute));
                time_tokens = idxs;
                break 'find;
            }
        }
    }

    let (mut hour, minute) = hm?;
    for &i in &time_tokens {
        used[i] = true;
    }
    // the preposition right before the time ("в 18:00") belongs to it
    if let Some(&first) = time_tokens.first() {
        if first > 0 && matches!(tokens[first - 1].as_str(), "в" | "во" | "at" | "к") {
            used[first - 1] = true;
        }
    }

    // part of day right after the time
    let after = time_tokens.last().map(|i| i + 1).unwrap_or(0);
    if let Some(word) = tokens.get(after) {
        let adjust = match word.as_str() {
            "утра" | "утром" | "am" => Some("am"),
            "дня" | "днем" | "вечера" | "вечером" | "pm" => Some("pm"),
            "ночи" | "ночью" => Some("night"),
            _ => None,
        };
        if let Some(kind) = adjust {
            used[after] = true;
            match kind {
                "am" => {
                    if hour == 12 {
                        hour = 0;
                    }
                }
                "pm" => {
                    if hour < 12 {
                        hour += 12;
                    }
                }
                _ => {
                    if hour == 12 {
                        hour = 0;
                    } else if hour >= 6 && hour < 12 {
                        hour += 12;
                    }
                }
            }
        }
    }
    if hour == 24 {
        hour = 0;
        day_offset = Some(day_offset.unwrap_or(0) + 1);
    }
    if hour > 23 {
        return None;
    }

    let date = now.date() + ChronoDuration::days(day_offset.unwrap_or(0));
    let mut moment = date.and_hms_opt(hour, minute, 0)?;
    if day_offset.is_none() && moment <= now {
        moment += ChronoDuration::days(1);
    }

    let rest: Vec<&str> = raw_tokens
        .iter()
        .enumerate()
        .filter(|(i, _)| !used[*i])
        .map(|(_, t)| *t)
        .collect();
    Some((moment, rest.join(" ")))
}

// local time -> unix seconds
fn to_unix(moment: NaiveDateTime) -> Option<i64> {
    Local.from_local_datetime(&moment).earliest().map(|d| d.timestamp())
}

pub fn parse_clock(text: &str) -> Option<(i64, String, NaiveDateTime)> {
    let now = Local::now().naive_local();
    let (moment, rest) = parse_clock_at(text, now)?;
    Some((to_unix(moment)?, rest, moment))
}

pub fn register(lua: &Lua, jarvis: &Table) -> mlua::Result<()> {
    let parse = lua.create_table()?;

    // jarvis.parse.number(text) -> number or nil
    parse.set(
        "number",
        lua.create_function(|_, text: String| Ok(parse_number(&text)))?,
    )?;

    // jarvis.parse.duration(text) -> seconds, rest_of_text   (or nil)
    parse.set(
        "duration",
        lua.create_function(|_, text: String| {
            Ok(match parse_duration(&text) {
                Some((secs, rest)) => (Some(secs), Some(rest)),
                None => (None, None),
            })
        })?,
    )?;

    // jarvis.parse.clock(text) -> unix_time, rest_of_text, "HH:MM", days_ahead   (or nil)
    parse.set(
        "clock",
        lua.create_function(|_, text: String| {
            Ok(match parse_clock(&text) {
                Some((unix, rest, moment)) => {
                    let days = (moment.date() - Local::now().naive_local().date()).num_days();
                    (
                        Some(unix),
                        Some(rest),
                        Some(format!("{:02}:{:02}", moment.hour(), moment.minute())),
                        Some(days),
                    )
                }
                None => (None, None, None, None),
            })
        })?,
    )?;

    jarvis.set("parse", parse)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers() {
        assert_eq!(parse_number("громкость 50 процентов"), Some(50.0));
        assert_eq!(parse_number("громкость пятьдесят"), Some(50.0));
        assert_eq!(parse_number("яркость сто"), Some(100.0));
        assert_eq!(parse_number("сделай двадцать пять"), Some(25.0));
        assert_eq!(parse_number("звук на 70%"), Some(70.0));
        assert_eq!(parse_number("volume fifty"), Some(50.0));
        assert_eq!(parse_number("открой ютуб"), None);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("поставь таймер на 5 минут").unwrap().0, 300);
        assert_eq!(parse_duration("через пять минут").unwrap().0, 300);
        assert_eq!(parse_duration("выключи компьютер через 10 минут").unwrap().0, 600);
        assert_eq!(parse_duration("таймер на час").unwrap().0, 3600);
        assert_eq!(parse_duration("через полчаса").unwrap().0, 1800);
        assert_eq!(parse_duration("через полтора часа").unwrap().0, 5400);
        assert_eq!(parse_duration("на 1 час 30 минут").unwrap().0, 5400);
        assert_eq!(parse_duration("через двадцать пять минут").unwrap().0, 1500);
        assert_eq!(parse_duration("таймер 30 секунд").unwrap().0, 30);
        assert_eq!(parse_duration("timer for 2 minutes").unwrap().0, 120);
        assert_eq!(parse_duration("таймер 5min").unwrap().0, 300);
        assert_eq!(parse_duration("taymer 5 daqiqaga qo'y").unwrap().0, 300);
        assert_eq!(parse_duration("taymer 30 soniya").unwrap().0, 30);
        assert_eq!(parse_duration("2 soatdan keyin").unwrap().0, 7200);
        assert_eq!(parse_duration("выключи компьютер"), None);
        assert_eq!(parse_duration("через 0 минут"), None);
    }

    #[test]
    fn duration_rest_text() {
        let (secs, rest) = parse_duration("напомни через 5 минут позвонить маме").unwrap();
        assert_eq!(secs, 300);
        assert_eq!(rest, "напомни позвонить маме");
        let (_, rest) = parse_duration("поставь таймер на 5 минут").unwrap();
        assert_eq!(rest, "поставь таймер");
    }

    fn at(h: u32, m: u32) -> NaiveDateTime {
        chrono::NaiveDate::from_ymd_opt(2026, 10, 10).unwrap().and_hms_opt(h, m, 0).unwrap()
    }
    fn clock(text: &str) -> Option<(String, String)> {
        parse_clock_at(text, at(15, 0)).map(|(m, rest)| (m.format("%d %H:%M").to_string(), rest))
    }

    #[test]
    fn clock_times() {
        assert_eq!(clock("разбуди в 7:30").unwrap().0, "11 07:30"); // already past today -> tomorrow
        assert_eq!(clock("напомни в 18:00 про тренировку").unwrap(), ("10 18:00".into(), "напомни про тренировку".into()));
        assert_eq!(clock("завтра в 9 утра").unwrap().0, "11 09:00");
        assert_eq!(clock("в семь тридцать вечера").unwrap().0, "10 19:30");
        assert_eq!(clock("в половине восьмого вечера").unwrap().0, "10 19:30");
        assert_eq!(clock("послезавтра в 10").unwrap().0, "12 10:00");
        assert_eq!(clock("в 3 ночи").unwrap().0, "11 03:00");
        assert_eq!(clock("в двадцать три ноль ноль").map(|c| c.0), Some("10 23:00".into()));
        assert_eq!(clock("в полдень").unwrap().0, "11 12:00");
        assert_eq!(clock("at 7:30 pm").unwrap().0, "10 19:30");
    }

    #[test]
    fn clock_not_a_duration() {
        assert_eq!(clock("поставь таймер на 5 минут"), None);
        assert_eq!(clock("напомни через час"), None);
    }
}
