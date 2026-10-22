//! Long-term memory (what the user asked to remember): a local file, search, edit, delete, export. Secrets are never stored
//! (passwords, keys, tokens, card numbers are refused with a clear message). A conflicting fact (same subject, another value)
//! is reported instead of silently kept twice. A damaged file is moved aside and the memory starts empty.

use crate::{store, textnorm};
use serde::{Deserialize, Serialize};

const FILE: &str = "memory.json";
pub const MAX_ITEMS: usize = 2000;
pub const MAX_LEN: usize = 500;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub text: String,
    pub created: u64,
    pub updated: u64,
    #[serde(default)]
    pub source: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Memory {
    #[serde(default = "one")]
    pub version: u32,
    // the AI may remember by itself («запомни» is always allowed)
    #[serde(default)]
    pub auto_remember: bool,
    #[serde(default)]
    pub items: Vec<Entry>,
}
fn one() -> u32 {
    1
}
impl Default for Memory {
    fn default() -> Self {
        Memory { version: 1, auto_remember: false, items: Vec::new() }
    }
}

#[derive(Debug, PartialEq)]
pub enum Added {
    Added(String),
    Duplicate(String),
    // same subject, other value: the id and the old text
    Conflict(String, String),
}

pub fn looks_secret(text: &str) -> bool {
    let low = text.to_lowercase();
    let words = ["пароль", "password", "parol", "токен", "token", "api key", "api-key", "апи ключ", "секретный ключ", "secret key", "pin-код", "пин-код", "cvv", "cvc"];
    if words.iter().any(|w| low.contains(w)) {
        return true;
    }
    if low.contains("sk-") || low.contains("ghp_") || low.contains("xoxb-") {
        return true;
    }
    // a card number: 13-19 digits (spaces and dashes allowed)
    let digits: String = text.chars().filter(|c| c.is_ascii_digit() || *c == ' ' || *c == '-').collect();
    let run = digits.split(|c: char| c != ' ' && c != '-' && !c.is_ascii_digit()).map(|p| p.chars().filter(|c| c.is_ascii_digit()).count()).max().unwrap_or(0);
    run >= 13
}

// the part before «—», «:», «=», « это », « is » (the subject of a fact)
fn subject(text: &str) -> Option<String> {
    let t = text.to_lowercase();
    for sep in [" — ", " - ", ": ", " = ", " это ", " is ", " bu "] {
        if let Some(i) = t.find(sep) {
            let s = textnorm::normalize(&t[..i]);
            if !s.is_empty() {
                return Some(s);
            }
        }
    }
    None
}

pub fn load() -> Memory {
    store::read::<Memory>(FILE).0
}
fn save(m: &Memory) -> Result<(), String> {
    store::write(FILE, m)
}

pub fn add(text: &str, source: &str) -> Result<Added, String> {
    add_with(text, source, false)
}

// replace = true: a conflicting fact is overwritten
pub fn add_with(text: &str, source: &str, replace: bool) -> Result<Added, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Нечего запоминать.".into());
    }
    if text.chars().count() > MAX_LEN {
        return Err(format!("Слишком длинно: запоминаю до {} символов.", MAX_LEN));
    }
    if looks_secret(text) {
        return Err("Пароли, ключи, токены и номера карт я не запоминаю.".into());
    }
    let mut m = load();
    let norm = textnorm::normalize(text);
    if let Some(e) = m.items.iter().find(|e| textnorm::normalize(&e.text) == norm) {
        return Ok(Added::Duplicate(e.id.clone()));
    }
    if let Some(sub) = subject(text) {
        if let Some(i) = m.items.iter().position(|e| subject(&e.text).as_deref() == Some(sub.as_str())) {
            if !replace {
                return Ok(Added::Conflict(m.items[i].id.clone(), m.items[i].text.clone()));
            }
            m.items[i].text = text.to_string();
            m.items[i].updated = store::now();
            let id = m.items[i].id.clone();
            save(&m)?;
            return Ok(Added::Added(id));
        }
    }
    if m.items.len() >= MAX_ITEMS {
        return Err("Память заполнена: удалите ненужное.".into());
    }
    let id = store::new_id();
    let now = store::now();
    m.items.push(Entry { id: id.clone(), text: text.to_string(), created: now, updated: now, source: source.to_string() });
    save(&m)?;
    Ok(Added::Added(id))
}

pub fn edit(id: &str, text: &str) -> Result<(), String> {
    if looks_secret(text) {
        return Err("Пароли, ключи, токены и номера карт я не запоминаю.".into());
    }
    let mut m = load();
    let e = m.items.iter_mut().find(|e| e.id == id).ok_or("такой записи нет")?;
    e.text = text.trim().chars().take(MAX_LEN).collect();
    e.updated = store::now();
    save(&m)
}

pub fn delete(id: &str) -> bool {
    let mut m = load();
    let n = m.items.len();
    m.items.retain(|e| e.id != id);
    m.items.len() != n && save(&m).is_ok()
}

// the entries that mention the words of the query (all the words, typos allowed), best first
pub fn search(query: &str) -> Vec<Entry> {
    let q = textnorm::normalize(query);
    let words: Vec<&str> = q.split(' ').filter(|w| w.chars().count() > 2).collect();
    let m = load();
    if words.is_empty() {
        return m.items;
    }
    let mut hits: Vec<(usize, Entry)> = m
        .items
        .into_iter()
        .filter_map(|e| {
            let norm = textnorm::normalize(&e.text);
            let ew: Vec<&str> = norm.split(' ').collect();
            let n = words.iter().filter(|w| ew.iter().any(|x| textnorm::same_stem(w, x) || textnorm::similarity(w, x) >= 0.8)).count();
            (n == words.len()).then_some((n, e.clone()))
        })
        .collect();
    hits.sort_by(|a, b| b.1.updated.cmp(&a.1.updated));
    hits.into_iter().map(|(_, e)| e).collect()
}

pub fn set_auto(on: bool) -> Result<(), String> {
    let mut m = load();
    m.auto_remember = on;
    save(&m)
}

pub fn export_text() -> String {
    let m = load();
    m.items.iter().map(|e| format!("- {}", e.text)).collect::<Vec<_>>().join("\n")
}

// «запомни, что …» / «remember that …» / «eslab qol: …» → the fact
pub fn parse_remember(text: &str) -> Option<String> {
    let t = text.trim();
    let low = t.to_lowercase();
    for p in ["запомни, что ", "запомни что ", "запомни: ", "запомни ", "запиши в память: ", "запиши в память ", "remember that ", "remember: ", "remember ", "eslab qol: ", "eslab qol ", "eslab qol, "] {
        if low.starts_with(p) {
            return Some(t.chars().skip(p.chars().count()).collect::<String>().trim().to_string()).filter(|s| !s.is_empty());
        }
    }
    None
}

// «что ты помнишь про X» / «what do you remember about X»
pub fn parse_recall(text: &str) -> Option<String> {
    let low = textnorm::normalize(text);
    for p in ["что ты помнишь про ", "что ты помнишь о ", "что ты знаешь про ", "вспомни про ", "what do you remember about ", "do you remember ", "nima eslaysan "] {
        if let Some(rest) = low.strip_prefix(p) {
            return Some(rest.to_string());
        }
    }
    if matches!(low.as_str(), "что ты помнишь" | "что ты запомнил" | "what do you remember") {
        return Some(String::new());
    }
    None
}

// «забудь X» / «forget X»
pub fn parse_forget(text: &str) -> Option<String> {
    let low = textnorm::normalize(text);
    for p in ["забудь что ", "забудь про ", "забудь ", "forget about ", "forget that ", "forget "] {
        if let Some(rest) = low.strip_prefix(p) {
            return Some(rest.to_string()).filter(|s| !s.is_empty());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_search_edit_delete() {
        store::with_temp(|| {
            let Added::Added(id) = add("Мой любимый цвет — синий", "voice").unwrap() else { panic!() };
            assert!(matches!(add("мой любимый цвет — синий", "voice").unwrap(), Added::Duplicate(_)));
            assert_eq!(search("любимый цвет").len(), 1);
            assert_eq!(search("любимого цвета").len(), 1);
            assert!(search("машина").is_empty());
            edit(&id, "Мой любимый цвет — зелёный").unwrap();
            assert!(export_text().contains("зелёный"));
            assert!(delete(&id));
            assert!(load().items.is_empty());
        });
    }
    #[test]
    fn conflict_is_reported_not_doubled() {
        store::with_temp(|| {
            add("Мой город — Ташкент", "").unwrap();
            assert!(matches!(add("Мой город — Самарканд", "").unwrap(), Added::Conflict(_, old) if old.contains("Ташкент")));
            assert_eq!(load().items.len(), 1);
            add_with("Мой город — Самарканд", "", true).unwrap();
            assert_eq!(load().items.len(), 1);
            assert!(load().items[0].text.contains("Самарканд"));
        });
    }
    #[test]
    fn secrets_are_refused() {
        store::with_temp(|| {
            for s in ["мой пароль 12345", "api key: abc", "карта 4111 1111 1111 1111", "token=xyz", "ключ sk-abcdef"] {
                assert!(add(s, "").is_err(), "{}", s);
            }
            assert!(load().items.is_empty());
            assert!(!std::fs::read_to_string(store::file(FILE)).unwrap_or_default().contains("4111"));
        });
    }
    #[test]
    fn corrupt_file_is_moved_aside() {
        store::with_temp(|| {
            std::fs::write(store::file(FILE), "{not json").unwrap();
            assert!(load().items.is_empty());
            add("факт один", "").unwrap();
            assert_eq!(load().items.len(), 1);
            let dir = store::file(FILE).parent().unwrap().to_path_buf();
            assert!(std::fs::read_dir(dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("corrupt")));
        });
    }
    #[test]
    fn old_file_without_new_fields_loads() {
        store::with_temp(|| {
            std::fs::write(store::file(FILE), r#"{"items":[{"id":"1","text":"a b c","created":1,"updated":1}]}"#).unwrap();
            assert_eq!(load().items.len(), 1);
        });
    }
    #[test]
    fn phrases() {
        assert_eq!(parse_remember("Запомни, что мой день рождения 5 мая").unwrap(), "мой день рождения 5 мая");
        assert_eq!(parse_remember("remember that I like tea").unwrap(), "I like tea");
        assert_eq!(parse_recall("что ты помнишь про день рождения").unwrap(), "день рождения");
        assert_eq!(parse_forget("забудь про чай").unwrap(), "чай");
        assert!(parse_remember("привет").is_none());
    }
}
