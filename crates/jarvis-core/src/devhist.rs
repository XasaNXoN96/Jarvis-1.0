//! «ПК» and «Телефон»: one history of what was asked, with two sections. A phrase that came by the microphone, the window or the
//! console is the PC section; one that came from the phone page or Telegram is the phone section. Both are kept in one file on the
//! computer, so the phone page and the window show the same thing. Only the phrase and the first answer are kept (never a password:
//! a phrase that talks about a password or a key is stored masked).
use crate::origin::Source;
use crate::store;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const FILE: &str = "device_history.json";
const MAX: usize = 300;
// an answer belongs to the question asked up to this long ago
const ANSWER_WINDOW: u64 = 120;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub at: u64,
    // "pc" | "phone"
    pub door: String,
    pub text: String,
    #[serde(default)]
    pub answer: String,
}

pub fn door_of(source: Source) -> Option<&'static str> {
    match source {
        Source::Voice | Source::Window | Source::Console | Source::Ipc => Some("pc"),
        Source::Phone | Source::Telegram => Some("phone"),
        // the program's own follow-ups and scripts are not something a person asked
        Source::Script | Source::Internal => None,
    }
}

fn masked(text: &str) -> String {
    let l = text.to_lowercase();
    if ["парол", "password", "токен", "token", "секрет", "api key", "ключ апи", "ключ api"].iter().any(|w| l.contains(w)) {
        return "(фраза скрыта: в ней пароль или ключ)".to_string();
    }
    text.trim().chars().take(300).collect()
}

fn load() -> Vec<Entry> {
    store::read::<Vec<Entry>>(FILE).0
}

// a phrase came in through a door
pub fn question(source: Source, text: &str) {
    let Some(door) = door_of(source) else { return };
    if text.trim().is_empty() {
        return;
    }
    let mut l = load();
    l.push(Entry { id: store::new_id(), at: store::now(), door: door.into(), text: masked(text), answer: String::new() });
    let n = l.len();
    if n > MAX {
        l.drain(..n - MAX);
    }
    let _ = store::write(FILE, &l);
}

// the first answer of Jarvis to the latest question that has none yet
pub fn answer(text: &str) {
    let t = text.trim();
    if t.is_empty() {
        return;
    }
    let mut l = load();
    let now = store::now();
    if let Some(e) = l.iter_mut().rev().find(|e| now.saturating_sub(e.at) <= ANSWER_WINDOW) {
        if e.answer.is_empty() {
            e.answer = masked(t).chars().take(240).collect();
            let _ = store::write(FILE, &l);
        }
    }
}

pub fn clear(door: &str) -> usize {
    let mut l = load();
    let before = l.len();
    l.retain(|e| door != "all" && e.door != door);
    let _ = store::write(FILE, &l);
    before - l.len()
}

// the newest first; `door` = "pc" | "phone" | "all"
pub fn list(door: &str, limit: usize) -> Vec<Entry> {
    load().into_iter().rev().filter(|e| door == "all" || e.door == door).take(limit.min(100)).collect()
}

pub fn json(door: &str, limit: usize) -> Value {
    let all = load();
    let count = |d: &str| all.iter().filter(|e| e.door == d).count();
    json!({
        "pc": count("pc"), "phone": count("phone"),
        "items": list(door, limit).iter().map(|e| json!({ "id": e.id, "at": e.at, "door": e.door, "text": e.text, "answer": e.answer })).collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_sections_by_the_door_and_the_answer_joins_the_question() {
        store::with_temp(|| {
            question(Source::Voice, "открой браузер");
            question(Source::Phone, "поставь таймер на пять минут");
            question(Source::Script, "сам");
            question(Source::Internal, "сам");
            answer("Таймер поставлен");
            let pc = list("pc", 10);
            let ph = list("phone", 10);
            assert_eq!((pc.len(), ph.len()), (1, 1), "scripts are nobody's question");
            assert_eq!(ph[0].text, "поставь таймер на пять минут");
            assert_eq!(ph[0].answer, "Таймер поставлен", "the answer goes to the latest question");
            assert_eq!(pc[0].answer, "");
            let j = json("all", 10);
            assert_eq!((j["pc"].as_u64(), j["phone"].as_u64()), (Some(1), Some(1)));
            assert_eq!(clear("phone"), 1);
            assert_eq!(list("all", 10).len(), 1);
        });
    }

    #[test]
    fn a_phrase_with_a_password_is_not_kept_and_the_list_is_bounded() {
        store::with_temp(|| {
            question(Source::Window, "мой пароль 12345");
            assert!(list("pc", 5)[0].text.contains("скрыта"));
            for i in 0..(MAX + 20) {
                question(Source::Voice, &format!("команда {}", i));
            }
            assert_eq!(load().len(), MAX);
            assert_eq!(list("pc", 1000).len(), 100, "a page is at most 100");
        });
    }
}
