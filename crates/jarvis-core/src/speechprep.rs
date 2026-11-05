// A text made ready for the voice, so that Jarvis speaks like a person reads and not like a program prints: numbers, times, percents and degrees become
// words (the voices read digits badly and in the wrong form), emoji and signs that no one pronounces are dropped, a link is «ссылка», a dash and a bracket
// become a pause, a sentence that has no end gets one (the voice then lowers the tone at the end). Pure and tested. Only for Russian text: another
// language is left as it is.

const ONES: [&str; 20] = [
    "ноль", "один", "два", "три", "четыре", "пять", "шесть", "семь", "восемь", "девять", "десять", "одиннадцать", "двенадцать", "тринадцать", "четырнадцать",
    "пятнадцать", "шестнадцать", "семнадцать", "восемнадцать", "девятнадцать",
];
const TENS: [&str; 10] = ["", "", "двадцать", "тридцать", "сорок", "пятьдесят", "шестьдесят", "семьдесят", "восемьдесят", "девяносто"];
const HUNDREDS: [&str; 10] = ["", "сто", "двести", "триста", "четыреста", "пятьсот", "шестьсот", "семьсот", "восемьсот", "девятьсот"];

// the number as words (masculine, the nominative): 0 .. 999 999 999
pub fn words_ru(n: u64) -> String {
    if n >= 1_000_000_000 {
        return digits_one_by_one(&n.to_string());
    }
    if n == 0 {
        return "ноль".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    let groups = [(n / 1_000_000, ["миллион", "миллиона", "миллионов"], false), ((n / 1000) % 1000, ["тысяча", "тысячи", "тысяч"], true), (n % 1000, ["", "", ""], false)];
    for (g, names, fem) in groups {
        if g == 0 {
            continue;
        }
        parts.push(below_thousand(g, fem));
        if !names[0].is_empty() {
            parts.push(plural(g, names).to_string());
        }
    }
    parts.join(" ")
}

fn below_thousand(g: u64, fem: bool) -> String {
    let mut p: Vec<String> = Vec::new();
    if g >= 100 {
        p.push(HUNDREDS[(g / 100) as usize].to_string());
    }
    let r = g % 100;
    if r >= 20 {
        p.push(TENS[(r / 10) as usize].to_string());
        if r % 10 > 0 {
            p.push(unit(r % 10, fem));
        }
    } else if r > 0 {
        p.push(unit(r, fem));
    }
    p.join(" ")
}

fn unit(u: u64, fem: bool) -> String {
    if fem && u == 1 {
        return "одна".to_string();
    }
    if fem && u == 2 {
        return "две".to_string();
    }
    ONES[u as usize].to_string()
}

// «1 минута, 2 минуты, 5 минут»
pub fn plural(n: u64, forms: [&'static str; 3]) -> &'static str {
    let (m10, m100) = (n % 10, n % 100);
    if (11..=14).contains(&m100) {
        forms[2]
    } else if m10 == 1 {
        forms[0]
    } else if (2..=4).contains(&m10) {
        forms[1]
    } else {
        forms[2]
    }
}

fn digits_one_by_one(s: &str) -> String {
    s.chars().filter_map(|c| c.to_digit(10)).map(|d| ONES[d as usize]).collect::<Vec<_>>().join(" ")
}

fn is_emoji_or_symbol(c: char) -> bool {
    let u = c as u32;
    (0x1F000..=0x1FAFF).contains(&u) || (0x2600..=0x27BF).contains(&u) || (0x2B00..=0x2BFF).contains(&u) || u == 0xFE0F || u == 0x200D || (0x2190..=0x21FF).contains(&u)
}

// the numbers of the text (digits, with a sign, a decimal comma, a time, a percent, a degree) become words
fn numbers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !c.is_ascii_digit() {
            out.push(c);
            i += 1;
            continue;
        }
        // the digits of a word with letters («A4», «mp3», «COVID19») are left as they are
        let glued_before = i > 0 && (chars[i - 1].is_alphabetic());
        let mut j = i;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        let int: String = chars[i..j].iter().collect();
        let glued_after = j < chars.len() && chars[j].is_alphabetic() && !matches!(chars[j], '°');
        if glued_before || glued_after && !chars[j..].iter().take(3).collect::<String>().chars().all(|c| c == '°') {
            out.extend(chars[i..j].iter());
            i = j;
            continue;
        }
        // a time 23:40
        if j + 2 < chars.len() + 0 && chars[j] == ':' && j + 2 < chars.len() && chars[j + 1].is_ascii_digit() && chars[j + 2].is_ascii_digit() && int.len() <= 2 {
            let mut k = j + 1;
            while k < chars.len() && chars[k].is_ascii_digit() {
                k += 1;
            }
            let mins: String = chars[j + 1..k].iter().collect();
            if let (Ok(h), Ok(m)) = (int.parse::<u64>(), mins.parse::<u64>()) {
                if h < 24 && m < 60 && mins.len() == 2 {
                    out.push_str(&words_ru(h));
                    out.push(' ');
                    if m == 0 {
                        out.push_str("ровно");
                    } else if m < 10 {
                        out.push_str("ноль ");
                        out.push_str(&words_ru(m));
                    } else {
                        out.push_str(&words_ru(m));
                    }
                    i = k;
                    continue;
                }
            }
        }
        // a decimal: 36,6 / 3.5
        let mut end = j;
        let mut frac: Option<String> = None;
        if j + 1 < chars.len() && (chars[j] == ',' || chars[j] == '.') && chars[j + 1].is_ascii_digit() {
            let mut k = j + 1;
            while k < chars.len() && chars[k].is_ascii_digit() {
                k += 1;
            }
            frac = Some(chars[j + 1..k].iter().collect());
            end = k;
        }
        let value = int.parse::<u64>().ok().filter(|_| int.len() <= 9);
        let int_words = match value {
            Some(v) if int.len() == 1 || !int.starts_with('0') => words_ru(v),
            _ => digits_one_by_one(&int),
        };
        let mut piece = int_words;
        if let Some(f) = &frac {
            piece.push_str(" запятая ");
            piece.push_str(&if f.len() <= 2 && !f.starts_with('0') { words_ru(f.parse::<u64>().unwrap_or(0)) } else { digits_one_by_one(f) });
        }
        // a percent / a degree right after the number
        let rest: String = chars[end..].iter().take(2).collect();
        let n_for_unit = value.unwrap_or(0);
        if rest.starts_with('%') && frac.is_none() {
            piece.push(' ');
            piece.push_str(plural(n_for_unit, ["процент", "процента", "процентов"]));
            end += 1;
        } else if rest.starts_with('%') {
            piece.push_str(" процента");
            end += 1;
        } else if rest.starts_with('°') && frac.is_none() {
            piece.push(' ');
            piece.push_str(plural(n_for_unit, ["градус", "градуса", "градусов"]));
            end += 1;
            if chars.get(end) == Some(&'C') || chars.get(end) == Some(&'С') {
                end += 1;
            }
        }
        // a sign in front of a number: «+11» / «−5»
        let had_sign = out.ends_with('+') || out.ends_with('-') || out.ends_with('−');
        if had_sign && (out.len() < 2 || !out[..out.len() - 1].chars().last().map(|p| p.is_alphanumeric()).unwrap_or(false)) {
            let sign = out.pop().unwrap_or(' ');
            out.push_str(if sign == '+' { "плюс " } else { "минус " });
        }
        out.push_str(&piece);
        i = end;
    }
    out
}

/// The text as it should be spoken. `lang` is the language of the voice («ru»): the other languages are only cleaned of signs.
pub fn prepare(text: &str, lang: &str) -> String {
    // links
    let mut t = String::new();
    for word in text.split_whitespace() {
        let w = word.trim_matches(|c: char| matches!(c, '(' | ')' | '<' | '>' | '[' | ']'));
        if w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.") {
            t.push_str("ссылка ");
        } else {
            t.push_str(word);
            t.push(' ');
        }
    }
    // signs that nobody pronounces, emoji; dashes and brackets are pauses
    let mut s = String::new();
    for c in t.chars() {
        if is_emoji_or_symbol(c) {
            continue;
        }
        match c {
            '*' | '`' | '#' | '_' | '~' | '|' | '^' | '<' | '>' | '=' => s.push(' '),
            '—' | '–' | '(' | ')' | '[' | ']' | '«' | '»' | '"' => s.push_str(if matches!(c, '«' | '»' | '"') { " " } else { ", " }),
            '→' => s.push_str(", "),
            '&' => s.push_str(if lang == "ru" { " и " } else { " and " }),
            _ => s.push(c),
        }
    }
    let mut s = if lang == "ru" { numbers(&s) } else { s };
    // the pauses that came out doubled: «, ,» and several marks in a row
    loop {
        let before = s.clone();
        s = s.replace(" ,", ",").replace(",,", ",").replace(", ,", ",").replace("!!", "!").replace("??", "?").replace("..", ".").replace(" .", ".").replace(",.", ".").replace(" !", "!").replace(" ?", "?").replace("  ", " ");
        if s == before {
            break;
        }
    }
    let s = s.trim().trim_start_matches(|c: char| matches!(c, ',' | ';' | ':')).trim();
    // a sentence without an end gets one: the voice then falls at the end instead of hanging
    match s.chars().last() {
        Some(c) if c.is_alphanumeric() => format!("{}.", s),
        Some(_) => s.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_read_as_words_with_the_right_form() {
        for (n, w) in [(0, "ноль"), (5, "пять"), (11, "одиннадцать"), (21, "двадцать один"), (40, "сорок"), (100, "сто"), (205, "двести пять"), (1000, "одна тысяча"), (2026, "две тысячи двадцать шесть"),
            (12_345, "двенадцать тысяч триста сорок пять"), (1_000_000, "один миллион"), (2_500_000, "два миллиона пятьсот тысяч")] {
            assert_eq!(words_ru(n), w, "{}", n);
        }
        assert_eq!((plural(1, ["минута", "минуты", "минут"]), plural(3, ["минута", "минуты", "минут"]), plural(5, ["минута", "минуты", "минут"]), plural(11, ["минута", "минуты", "минут"]), plural(22, ["минута", "минуты", "минут"])),
            ("минута", "минуты", "минут", "минут", "минуты"));
    }

    #[test]
    fn a_text_is_made_ready_for_the_voice() {
        assert_eq!(prepare("Сейчас 23:40.", "ru"), "Сейчас двадцать три сорок.");
        assert_eq!(prepare("Встреча в 9:05", "ru"), "Встреча в девять ноль пять.");
        assert_eq!(prepare("Будильник на 7:00", "ru"), "Будильник на семь ровно.");
        assert_eq!(prepare("Заряд 50% и 21%", "ru"), "Заряд пятьдесят процентов и двадцать один процент.");
        assert_eq!(prepare("Ташкент: +11° ясно", "ru"), "Ташкент: плюс одиннадцать градусов ясно.");
        assert_eq!(prepare("Минус 5°C на улице", "ru"), "Минус пять градусов на улице.");
        assert_eq!(prepare("Сегодня 5 ноября 2026 года", "ru"), "Сегодня пять ноября две тысячи двадцать шесть года.");
        assert_eq!(prepare("Температура 36,6", "ru"), "Температура тридцать шесть запятая шесть.");
        assert_eq!(prepare("Позвоните по 998901234567", "ru"), "Позвоните по девять девять восемь девять ноль один два три четыре пять шесть семь.");
        // digits that belong to a word are kept
        assert_eq!(prepare("Формат A4 и mp3", "ru"), "Формат A4 и mp3.");
        // another language: only the signs are cleaned
        assert_eq!(prepare("It is 23:40 **now** 😀", "en"), "It is 23:40 now.");
    }

    #[test]
    fn signs_links_and_pauses() {
        assert_eq!(prepare("Смотрите https://example.com/a?b=1 — там всё есть", "ru"), "Смотрите ссылка, там всё есть.");
        assert_eq!(prepare("Готово 😀🎉!!!", "ru"), "Готово!");
        assert_eq!(prepare("## Заголовок\n* пункт один\n* пункт два", "ru"), "Заголовок пункт один пункт два.");
        assert_eq!(prepare("Тест (проверка) «в кавычках»", "ru"), "Тест, проверка, в кавычках.");
        assert_eq!(prepare("Прибыль и убытки & налоги", "ru"), "Прибыль и убытки и налоги.");
        assert_eq!(prepare("", "ru"), "");
        assert_eq!(prepare("   ", "ru"), "");
        assert_eq!(prepare("Да, сэр.", "ru"), "Да, сэр.");
    }
}
