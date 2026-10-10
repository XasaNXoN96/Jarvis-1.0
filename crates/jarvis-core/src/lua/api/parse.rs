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
    if t.starts_with("сек") || t.starts_with("second") || t == "sec" || t == "secs" {
        Some(1.0)
    } else if t.starts_with("мин") || t.starts_with("minute") || t == "min" || t == "mins" {
        Some(60.0)
    } else if t.starts_with("час") || t.starts_with("hour") || t == "hr" || t == "hrs" {
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
}
