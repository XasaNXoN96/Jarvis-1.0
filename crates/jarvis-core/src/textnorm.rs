//! Tolerant matching of what the user typed or the recogniser heard (Russian / English / Uzbek): case, «ё», punctuation, a wrong
//! keyboard layout («ghbdtn» = «привет»), typos and endings. It never decides on its own when two answers are close: it says so
//! (`Match::Ambiguous`) and the caller asks a short clarifying question.

pub fn normalize(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        let c = match c {
            'ё' => 'е',
            '’' | '‘' | 'ʻ' | 'ʼ' | '`' => '\'',
            c => c,
        };
        if c.is_alphanumeric() || c == '\'' {
            out.push(c);
        } else if !out.ends_with(' ') {
            out.push(' ');
        }
    }
    out.trim().to_string()
}

const EN: &str = "qwertyuiop[]asdfghjkl;'zxcvbnm,.`";
const RU: &str = "йцукенгшщзхъфывапролджэячсмитьбюё";

fn swap(s: &str, from: &str, to: &str) -> String {
    let f: Vec<char> = from.chars().collect();
    let t: Vec<char> = to.chars().collect();
    s.chars().map(|c| f.iter().position(|x| *x == c).map(|i| t[i]).unwrap_or(c)).collect()
}

// the text typed with the Latin layout that was meant to be Russian («ghbdtn» → «привет»), and back
pub fn en_to_ru(s: &str) -> String {
    swap(&s.to_lowercase(), EN, RU)
}
pub fn ru_to_en(s: &str) -> String {
    swap(&s.to_lowercase(), RU, EN)
}

pub fn has_cyrillic(s: &str) -> bool {
    s.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c))
}
pub fn has_latin(s: &str) -> bool {
    s.chars().any(|c| c.is_ascii_alphabetic())
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur.push((prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost));
        }
        prev = cur;
    }
    prev[b.len()]
}

// 0.0 .. 1.0
pub fn similarity(a: &str, b: &str) -> f32 {
    let (a, b) = (normalize(a), normalize(b));
    let n = a.chars().count().max(b.chars().count());
    if n == 0 {
        return 1.0;
    }
    1.0 - levenshtein(&a, &b) as f32 / n as f32
}

#[derive(Debug, PartialEq)]
pub enum Match {
    Exact(usize),
    Fuzzy(usize, f32),
    // several candidates are about equally good: ask which one
    Ambiguous(Vec<usize>),
    None,
}

pub const FUZZY_MIN: f32 = 0.72;
const TIE: f32 = 0.06;

// the best candidate for the input, also trying the other keyboard layout of the input
pub fn best_match(input: &str, candidates: &[&str]) -> Match {
    let norm = normalize(input);
    if norm.is_empty() || candidates.is_empty() {
        return Match::None;
    }
    if let Some(i) = candidates.iter().position(|c| normalize(c) == norm) {
        return Match::Exact(i);
    }
    let variants = [norm.clone(), normalize(&en_to_ru(&norm)), normalize(&ru_to_en(&norm))];
    let mut scored: Vec<(usize, f32)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| (i, variants.iter().map(|v| similarity(v, c)).fold(0.0, f32::max)))
        .filter(|(_, s)| *s >= FUZZY_MIN)
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    match scored.as_slice() {
        [] => Match::None,
        [(i, s)] => Match::Fuzzy(*i, *s),
        [(i, s), (_, s2), ..] if s - s2 > TIE => Match::Fuzzy(*i, *s),
        all => Match::Ambiguous(all.iter().filter(|(_, s)| all[0].1 - s <= TIE).map(|(i, _)| *i).collect()),
    }
}

// «жене» ~ «жена», «маме» ~ «мама»: same beginning, endings may differ
pub fn same_stem(a: &str, b: &str) -> bool {
    let (a, b) = (normalize(a), normalize(b));
    if a == b {
        return true;
    }
    let (ac, bc): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let common = ac.iter().zip(bc.iter()).take_while(|(x, y)| x == y).count();
    let shortest = ac.len().min(bc.len());
    (common >= 3 && common + 2 >= shortest) || (common >= 2 && shortest <= 4 && common + 1 >= shortest)
}

// Russian, English or Uzbek (Latin / Cyrillic): the language of a text; "" when it cannot be said
pub fn detect_lang(s: &str) -> &'static str {
    let low = s.to_lowercase();
    if low.chars().any(|c| "ўқғҳ".contains(c)) {
        return "uz";
    }
    let cyr = low.chars().filter(|c| ('\u{400}'..='\u{4ff}').contains(c)).count();
    let lat = low.chars().filter(|c| c.is_ascii_alphabetic()).count();
    if cyr == 0 && lat == 0 {
        return "";
    }
    if cyr >= lat {
        return "ru";
    }
    let words: Vec<String> = normalize(&low).split(' ').map(|w| w.to_string()).collect();
    let uz = ["salom", "qanday", "yoz", "menga", "uchun", "bilan", "emas", "yo'q", "ha", "xabar", "kerak", "xotinimga", "onamga", "aka", "rahmat", "iltimos", "va", "bu", "men", "sen", "nima", "bor"];
    let en = ["the", "is", "are", "and", "to", "of", "you", "me", "send", "write", "what", "how", "please", "my", "a", "in", "it", "this", "that", "message", "wife", "tell"];
    let u = words.iter().filter(|w| uz.contains(&w.as_str())).count() + low.matches("o'").count() + low.matches("g'").count();
    let e = words.iter().filter(|w| en.contains(&w.as_str())).count();
    if u > e { "uz" } else if e > 0 || u == 0 { "en" } else { "uz" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes() {
        assert_eq!(normalize("  Ёлка,  ЁЖ!! "), "елка еж");
        assert_eq!(normalize("O‘zbek"), "o'zbek");
    }
    #[test]
    fn layout() {
        assert_eq!(en_to_ru("ghbdtn"), "привет");
        assert_eq!(ru_to_en("руддщ"), "hello");
    }
    #[test]
    fn typo_matches_one() {
        let c = ["открой браузер", "закрой браузер", "включи музыку"];
        assert_eq!(best_match("включи музику", &c), Match::Fuzzy(2, similarity("включи музику", "включи музыку")));
        assert_eq!(best_match("открой браузер", &c), Match::Exact(0));
    }
    #[test]
    fn wrong_layout_matches() {
        let c = ["привет", "пока"];
        assert!(matches!(best_match("ghbdtn", &c), Match::Exact(0) | Match::Fuzzy(0, _)));
    }
    #[test]
    fn close_candidates_are_ambiguous() {
        let c = ["мама", "маша"];
        match best_match("маа", &c) {
            Match::Ambiguous(v) => assert_eq!(v.len(), 2),
            Match::None => {}
            other => panic!("{:?}", other),
        }
        assert_eq!(best_match("ничего общего", &c), Match::None);
    }
    #[test]
    fn stems() {
        assert!(same_stem("жене", "жена"));
        assert!(same_stem("маме", "мама"));
        assert!(!same_stem("жена", "мама"));
        assert!(same_stem("Ане", "Аня"));
        assert!(!same_stem("мама", "маша"));
    }
    #[test]
    fn languages() {
        assert_eq!(detect_lang("Привет, как дела"), "ru");
        assert_eq!(detect_lang("Please send the message to my wife"), "en");
        assert_eq!(detect_lang("Salom, qanday yoz menga"), "uz");
        assert_eq!(detect_lang("Салом, қандай"), "uz");
        assert_eq!(detect_lang("1234"), "");
    }
}
