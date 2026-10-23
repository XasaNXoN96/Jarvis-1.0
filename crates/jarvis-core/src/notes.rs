//! The voice diary and notes: «запиши в дневник …», «заметка: …», «что я записывал вчера». Entries are dated, searchable and
//! deletable; only what the user said to be written is stored (nothing is recorded in the background).

use crate::{memory, store, textnorm};
use chrono::{Duration, NaiveDate};
use serde::{Deserialize, Serialize};

const FILE: &str = "notes.json";
pub const MAX_LEN: usize = 2000;
pub const MAX_ITEMS: usize = 5000;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Note {
    pub id: String,
    pub text: String,
    // unix seconds (UTC)
    pub at: u64,
    // "diary" | "note"
    pub kind: String,
    // the last change (unix seconds); older records have none and count from `at`
    #[serde(default)]
    pub updated: u64,
}

pub fn list() -> Vec<Note> {
    let mut v = store::read::<Vec<Note>>(FILE).0;
    for n in v.iter_mut() {
        if n.updated == 0 {
            n.updated = n.at;
        }
    }
    v
}

// the data comes back from a sync
pub fn replace_all(list: Vec<Note>) -> Result<(), String> {
    store::write(FILE, &list)
}

pub fn add(text: &str, kind: &str, at: u64) -> Result<Note, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Нечего записывать.".into());
    }
    if text.chars().count() > MAX_LEN {
        return Err(format!("Слишком длинно: до {} символов.", MAX_LEN));
    }
    if memory::looks_secret(text) {
        return Err("Пароли, ключи, токены и номера карт я не записываю.".into());
    }
    let mut all = list();
    if all.len() >= MAX_ITEMS {
        return Err("Дневник заполнен.".into());
    }
    let n = Note { id: store::new_id(), text: text.to_string(), at, updated: at, kind: if kind == "diary" { "diary" } else { "note" }.into() };
    all.push(n.clone());
    store::write(FILE, &all)?;
    Ok(n)
}

pub fn get(id: &str) -> Option<Note> {
    list().into_iter().find(|n| n.id == id)
}

// edits the text of a note; returns the old text (for the undo)
pub fn update(id: &str, text: &str) -> Result<String, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Запись не может быть пустой: чтобы убрать её, нажмите «Удалить».".into());
    }
    if text.chars().count() > MAX_LEN {
        return Err(format!("Слишком длинно: до {} символов.", MAX_LEN));
    }
    if memory::looks_secret(text) {
        return Err("Пароли, ключи, токены и номера карт я не записываю.".into());
    }
    let mut all = list();
    let n = all.iter_mut().find(|n| n.id == id).ok_or("такой записи нет")?;
    let old = std::mem::replace(&mut n.text, text.to_string());
    n.updated = store::now();
    store::write(FILE, &all)?;
    Ok(old)
}

// puts a deleted note back (the undo of a delete)
pub fn restore(n: Note) -> Result<(), String> {
    let mut all = list();
    if all.iter().any(|x| x.id == n.id) {
        return Err("Эта запись уже на месте.".into());
    }
    if all.len() >= MAX_ITEMS {
        return Err("Дневник заполнен.".into());
    }
    all.push(n);
    store::write(FILE, &all)
}

pub fn revert(id: &str, old: &str, expect_now: &str) -> Result<(), String> {
    let cur = get(id).ok_or("Записи уже нет.")?;
    if cur.text != expect_now {
        return Err("Запись уже изменена после этой правки, вернуть нельзя.".into());
    }
    update(id, old).map(|_| ())
}

// all notes as text, oldest first, with the date (UTC) and the kind
pub fn export_text() -> String {
    let mut v = list();
    v.sort_by_key(|n| n.at);
    v.iter()
        .map(|n| format!("[{}] {}: {}", chrono::DateTime::from_timestamp(n.at as i64, 0).map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default(), if n.kind == "diary" { "дневник" } else { "заметка" }, n.text))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn delete(id: &str) -> bool {
    let mut all = list();
    let n = all.len();
    all.retain(|x| x.id != id);
    all.len() != n && store::write(FILE, &all).is_ok()
}

pub fn search(q: &str) -> Vec<Note> {
    let _busy = crate::activity::enter(crate::activity::Kind::Searching);
    let q = textnorm::normalize(q);
    let words: Vec<&str> = q.split(' ').filter(|w| w.chars().count() > 2).collect();
    let mut v: Vec<Note> = list()
        .into_iter()
        .filter(|n| {
            let norm = textnorm::normalize(&n.text);
            let ew: Vec<&str> = norm.split(' ').collect();
            words.iter().all(|w| ew.iter().any(|x| textnorm::same_stem(w, x)))
        })
        .collect();
    v.sort_by(|a, b| b.at.cmp(&a.at));
    v
}

// the notes of a local day (`offset` = local minus UTC, seconds)
pub fn on_day(day: NaiveDate, offset: i64) -> Vec<Note> {
    list().into_iter().filter(|n| chrono::DateTime::from_timestamp(n.at as i64 + offset, 0).map(|d| d.date_naive() == day).unwrap_or(false)).collect()
}

#[derive(Debug, PartialEq)]
pub enum Cmd {
    Write { text: String, kind: &'static str },
    // today / yesterday / (days back)
    Read { days_back: i64 },
    Find(String),
}

pub fn parse(text: &str) -> Option<Cmd> {
    let t = text.trim();
    let low = t.to_lowercase();
    for (p, kind) in [("запиши в дневник: ", "diary"), ("запиши в дневник ", "diary"), ("дневник: ", "diary"), ("заметка: ", "note"), ("запиши заметку: ", "note"), ("запиши заметку ", "note"), ("add to diary: ", "diary"), ("note: ", "note"), ("kundalikka yoz: ", "diary"), ("eslatma: ", "note")] {
        if low.starts_with(p) {
            let body: String = t.chars().skip(p.chars().count()).collect();
            let body = body.trim().to_string();
            return if body.is_empty() { None } else { Some(Cmd::Write { text: body, kind }) };
        }
    }
    let n = textnorm::normalize(t);
    for (p, back) in [("что я записывал сегодня", 0), ("что я записывал вчера", 1), ("что в дневнике за сегодня", 0), ("что в дневнике за вчера", 1), ("what did i write today", 0), ("what did i write yesterday", 1)] {
        if n == p {
            return Some(Cmd::Read { days_back: back });
        }
    }
    for p in ["найди в дневнике ", "найди в заметках ", "search my diary for "] {
        if let Some(r) = n.strip_prefix(p) {
            return Some(Cmd::Find(r.to_string())).filter(|_| !r.is_empty());
        }
    }
    None
}

pub fn day_for(today: NaiveDate, days_back: i64) -> NaiveDate {
    today - Duration::days(days_back)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_search_read_delete() {
        store::with_temp(|| {
            let day = NaiveDate::from_ymd_opt(2026, 10, 22).unwrap();
            let at = day.and_hms_opt(12, 0, 0).unwrap().and_utc().timestamp() as u64;
            let n = add("Сегодня встретился с Иваном, обсудили поездку", "diary", at).unwrap();
            add("Купить хлеб", "note", at + 60).unwrap();
            assert_eq!(on_day(day, 0).len(), 2);
            assert!(on_day(day - Duration::days(1), 0).is_empty());
            assert_eq!(search("поездке с Иваном").len(), 1);
            assert!(add("мой пароль 123", "diary", at).is_err());
            assert!(delete(&n.id));
            assert_eq!(list().len(), 1);
        });
    }
    #[test]
    fn phrases() {
        assert_eq!(parse("Запиши в дневник: был хороший день"), Some(Cmd::Write { text: "был хороший день".into(), kind: "diary" }));
        assert_eq!(parse("что я записывал вчера"), Some(Cmd::Read { days_back: 1 }));
        assert_eq!(parse("найди в дневнике поездка"), Some(Cmd::Find("поездка".into())));
        assert_eq!(parse("открой браузер"), None);
        assert_eq!(parse("заметка: "), None);
    }

    #[test]
    fn notes_can_be_edited_exported_and_a_bad_edit_changes_nothing() {
        store::with_temp(|| {
            let a = add("первая", "note", 1_700_000_000).unwrap();
            add("вторая", "diary", 1_700_000_100).unwrap();
            assert!(update(&a.id, "").is_err());
            assert!(update(&a.id, "пароль: hunter2hunter2").is_err());
            assert_eq!(get(&a.id).unwrap().text, "первая", "a refused edit changes nothing");
            assert_eq!(update(&a.id, "первая, исправлено").unwrap(), "первая");
            assert!(get(&a.id).unwrap().updated >= a.updated);
            let t = export_text();
            assert!(t.starts_with("[2023-11-14") && t.contains("заметка: первая, исправлено") && t.lines().nth(1).unwrap().contains("дневник: вторая"), "{}", t);
        });
    }
}
