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
    // one of CATEGORIES ("" = none: all the records made before the categories, and the ones nobody sorted)
    #[serde(default)]
    pub category: String,
}

// the fixed set of categories: (id kept in the file, name shown)
pub const CATEGORIES: [(&str, &str); 4] = [("work", "Работа"), ("personal", "Личное"), ("ideas", "Идеи"), ("study", "Учёба")];

pub fn category_title(id: &str) -> Option<&'static str> {
    CATEGORIES.iter().find(|c| c.0 == id).map(|c| c.1)
}

pub fn categories_json() -> serde_json::Value {
    serde_json::json!(CATEGORIES.iter().map(|c| serde_json::json!({ "id": c.0, "title": c.1 })).collect::<Vec<_>>())
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
    let n = Note { id: store::new_id(), text: text.to_string(), at, updated: at, kind: if kind == "diary" { "diary" } else { "note" }.into(), category: String::new() };
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

// assigns or changes the category ("" takes it off); an unknown one is refused and nothing changes. Returns the previous category.
pub fn set_category(id: &str, category: &str) -> Result<String, String> {
    let category = category.trim();
    if !category.is_empty() && category_title(category).is_none() {
        return Err("Такой категории нет.".into());
    }
    let mut all = list();
    let n = all.iter_mut().find(|n| n.id == id).ok_or("такой записи нет")?;
    let old = std::mem::replace(&mut n.category, category.to_string());
    if old != n.category {
        n.updated = store::now();
        store::write(FILE, &all)?;
    }
    Ok(old)
}

// the notes of one category ("" = those without a category)
pub fn in_category(category: &str) -> Vec<Note> {
    list().into_iter().filter(|n| n.category == category).collect()
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
        .map(|n| format!("[{}] {}{}: {}", chrono::DateTime::from_timestamp(n.at as i64, 0).map(|d| d.format("%Y-%m-%d %H:%M").to_string()).unwrap_or_default(), if n.kind == "diary" { "дневник" } else { "заметка" }, category_title(&n.category).map(|t| format!(" ({})", t)).unwrap_or_default(), n.text))
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
    // `category` is one of the ids of CATEGORIES, or "" when the phrase names none
    Write { text: String, kind: &'static str, category: &'static str },
    // «что в дневнике по работе» / «прочитай записи категории идеи»
    ReadCategory(&'static str),
    // today / yesterday / (days back)
    Read { days_back: i64 },
    Find(String),
}

// the category a word names (the word as it is said: «работа», «по работе», «идеи», «учёба»); None when it is none of the four
pub fn category_of(word: &str) -> Option<&'static str> {
    let w = textnorm::normalize(word);
    let w = w.trim_start_matches("по ").trim();
    if w.starts_with("работ") {
        Some("work")
    } else if w.starts_with("личн") {
        Some("personal")
    } else if w.starts_with("иде") {
        Some("ideas")
    } else if w.starts_with("учеб") || w.starts_with("учёб") {
        Some("study")
    } else {
        None
    }
}

// «в категорию работа: купить кофе» / «в категории идеи купить кофе» / «(работа) купить кофе» → (the category, the text); no category named: ("", the text)
fn split_category(body: &str) -> (&'static str, String) {
    let low = body.to_lowercase();
    for p in ["в категорию ", "в категории ", "категория "] {
        if let Some(rest) = low.strip_prefix(p) {
            let rest_orig: String = body.chars().skip(low.chars().count() - rest.chars().count()).collect();
            let word_end = rest_orig.find(|c: char| c == ':' || c == ',' || c == ' ').unwrap_or(rest_orig.len());
            let (word, tail) = rest_orig.split_at(word_end);
            if let Some(cat) = category_of(word) {
                return (cat, tail.trim_start_matches([':', ',', ' ']).trim().to_string());
            }
            return ("", body.to_string());
        }
    }
    ("", body.to_string())
}

pub fn parse(text: &str) -> Option<Cmd> {
    let t = text.trim();
    let low = t.to_lowercase();
    for (p, kind) in [("запиши в дневник: ", "diary"), ("запиши в дневник ", "diary"), ("дневник: ", "diary"), ("заметка: ", "note"), ("запиши заметку: ", "note"), ("запиши заметку ", "note"), ("add to diary: ", "diary"), ("note: ", "note"), ("kundalikka yoz: ", "diary"), ("eslatma: ", "note")] {
        if low.starts_with(p) {
            let body: String = t.chars().skip(p.chars().count()).collect();
            let (category, body) = split_category(body.trim());
            let body = body.trim().to_string();
            return if body.is_empty() { None } else { Some(Cmd::Write { text: body, kind, category }) };
        }
    }
    let n = textnorm::normalize(t);
    for (p, back) in [("что я записывал сегодня", 0), ("что я записывал вчера", 1), ("что в дневнике за сегодня", 0), ("что в дневнике за вчера", 1), ("what did i write today", 0), ("what did i write yesterday", 1)] {
        if n == p {
            return Some(Cmd::Read { days_back: back });
        }
    }
    for p in ["прочитай записи категории ", "что в дневнике по категории ", "покажи записи категории ", "read my diary category "] {
        if let Some(r) = n.strip_prefix(p) {
            return category_of(r.trim()).map(Cmd::ReadCategory);
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
        assert_eq!(parse("Запиши в дневник: был хороший день"), Some(Cmd::Write { text: "был хороший день".into(), kind: "diary", category: "" }));
        assert_eq!(parse("запиши в дневник в категорию Работа: созвон в пять"), Some(Cmd::Write { text: "созвон в пять".into(), kind: "diary", category: "work" }));
        assert_eq!(parse("запиши в дневник в категории идеи сделать сайт"), Some(Cmd::Write { text: "сделать сайт".into(), kind: "diary", category: "ideas" }));
        assert_eq!(parse("запиши в дневник категория учёба, выучить урок").map(|c| matches!(c, Cmd::Write { category: "study", .. })), Some(true));
        assert_eq!(parse("запиши в дневник в категорию космос: привет"), Some(Cmd::Write { text: "в категорию космос: привет".into(), kind: "diary", category: "" }), "an unknown category is not taken: the whole phrase is the text");
        assert_eq!(parse("запиши в дневник в категорию работа:"), None, "no text");
        assert_eq!(parse("Прочитай записи категории работа"), Some(Cmd::ReadCategory("work")));
        assert_eq!(parse("прочитай записи категории космос"), None);
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

    #[test]
    fn a_category_can_be_assigned_changed_and_taken_off_and_a_wrong_one_changes_nothing() {
        store::with_temp(|| {
            let n = add("созвон с Маликой в пятницу", "note", 1000).unwrap();
            assert_eq!(n.category, "", "a new record has none");
            assert_eq!(set_category(&n.id, "work").unwrap(), "");
            assert_eq!(get(&n.id).unwrap().category, "work");
            assert!(get(&n.id).unwrap().updated >= n.updated);
            assert_eq!(set_category(&n.id, "ideas").unwrap(), "work");
            assert_eq!(in_category("ideas").len(), 1);
            assert!(in_category("work").is_empty());
            // wrong values: not in the set, other case, a path, the name instead of the id, too long
            for bad in ["Работа", "WORK", "../etc", "работа", &"x".repeat(500), "work;drop", "all"] {
                assert_eq!(set_category(&n.id, bad).unwrap_err(), "Такой категории нет.", "{}", bad);
                assert_eq!(get(&n.id).unwrap().category, "ideas", "{} changed it", bad);
            }
            // a record that does not exist
            assert!(set_category("нет-такой", "work").is_err());
            // taken off ("" and spaces)
            assert_eq!(set_category(&n.id, "  ").unwrap(), "ideas");
            assert_eq!(get(&n.id).unwrap().category, "");
            // the text and the kind are not touched
            assert_eq!(get(&n.id).unwrap().text, "созвон с Маликой в пятницу");
        });
    }

    #[test]
    fn records_made_before_the_categories_still_load_and_the_category_is_kept_in_the_file_and_the_export() {
        store::with_temp(|| {
            // an old file: no "category" field at all
            store::write("notes.json", &serde_json::json!([{ "id": "old1", "text": "старая запись", "at": 500, "kind": "diary" }])).unwrap();
            let n = get("old1").unwrap();
            assert_eq!((n.category.as_str(), n.updated), ("", 500));
            assert!(export_text().contains("дневник: старая запись"), "{}", export_text());
            set_category("old1", "personal").unwrap();
            // after a new start the category is read from the file
            assert_eq!(get("old1").unwrap().category, "personal");
            assert!(export_text().contains("дневник (Личное): старая запись"), "{}", export_text());
            // a deleted record comes back with its category (the undo of a delete)
            let n = get("old1").unwrap();
            assert!(delete("old1"));
            restore(n).unwrap();
            assert_eq!(get("old1").unwrap().category, "personal");
            // a file from another computer with an unknown category: it loads, the record is kept, the unknown id is shown as «без категории»
            store::write("notes.json", &serde_json::json!([{ "id": "x", "text": "t", "at": 1, "kind": "note", "category": "future" }])).unwrap();
            assert_eq!(get("x").unwrap().text, "t");
            assert!(export_text().contains("заметка: t"));
            assert_eq!(categories_json().as_array().unwrap().len(), CATEGORIES.len());
        });
    }
}
