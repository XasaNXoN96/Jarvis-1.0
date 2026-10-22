//! The history of what Jarvis did on its own, with an undo for the actions that CAN be undone: a remembered fact, a note, a job of the
//! scheduler, a picture that Jarvis created. Actions that cannot be taken back (a message that was opened in a messenger, a
//! program that was started) are written to the history as «нельзя отменить» and never offered for undo. An undo is applied once, and
//! only when the thing is still as Jarvis left it.

use crate::{memory, notes, scheduler, store};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const FILE: &str = "action_log.json";
const MAX: usize = 300;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", content = "arg", rename_all = "snake_case")]
pub enum Undo {
    // nothing to take back
    None,
    MemoryAdd(String),
    NoteAdd(String),
    JobAdd(String),
    // a file Jarvis created
    FileCreated(String),
    // a file Jarvis changed: (the file, the copy made before the change)
    FileRestore(String, String),
    // a fact / a note that was deleted: the whole entry, to put it back
    MemoryRestore(memory::Entry),
    NoteRestore(notes::Note),
    // a fact / a note that was edited: (id, the old text, the text the edit made); undone only while the text is still the edited one
    MemoryRevert(String, String, String),
    NoteRevert(String, String, String),
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub at: u64,
    pub what: String,
    pub undo: Undo,
    #[serde(default)]
    pub undone: bool,
}

pub fn list() -> Vec<Entry> {
    store::read::<Vec<Entry>>(FILE).0
}

pub fn record(what: &str, undo: Undo) -> String {
    let mut l = list();
    let id = store::new_id();
    l.push(Entry { id: id.clone(), at: store::now(), what: what.chars().take(200).collect(), undo, undone: false });
    let n = l.len();
    if n > MAX {
        l.drain(..n - MAX);
    }
    let _ = store::write(FILE, &l);
    id
}

pub fn can_undo(e: &Entry) -> bool {
    !e.undone && e.undo != Undo::None
}

fn apply(u: &Undo) -> Result<String, String> {
    match u {
        Undo::None => Err("Это действие нельзя отменить.".into()),
        Undo::MemoryAdd(id) => if memory::delete(id) { Ok("Убрал запись из памяти.".into()) } else { Err("Записи уже нет в памяти.".into()) },
        Undo::NoteAdd(id) => if notes::delete(id) { Ok("Убрал запись из дневника.".into()) } else { Err("Записи уже нет.".into()) },
        Undo::JobAdd(id) => if scheduler::remove(id) { Ok("Убрал задание из расписания.".into()) } else { Err("Задания уже нет.".into()) },
        Undo::MemoryRestore(e) => memory::restore(e.clone()).map(|_| "Вернул запись в память.".to_string()),
        Undo::NoteRestore(n) => notes::restore(n.clone()).map(|_| "Вернул запись в дневник.".to_string()),
        Undo::MemoryRevert(id, old, now) => memory::revert(id, old, now).map(|_| "Вернул прежний текст записи.".to_string()),
        Undo::NoteRevert(id, old, now) => notes::revert(id, old, now).map(|_| "Вернул прежний текст записи.".to_string()),
        Undo::FileRestore(p, b) => crate::codeassist::restore(p, b).map(|_| "Вернул файл как был.".to_string()),
        Undo::FileCreated(p) => {
            let path = PathBuf::from(p);
            // only inside the folder where Jarvis puts its own files
            if !path.starts_with(crate::imagegen::out_dir()) && !cfg!(test) {
                return Err("Этот файл не в папке Jarvis, его я не трогаю.".into());
            }
            std::fs::remove_file(&path).map(|_| "Удалил созданный файл.".to_string()).map_err(|_| "Файла уже нет или он занят.".to_string())
        }
    }
}

// undoes the entry (or the last one that can be undone when `id` is empty); marks it undone only when it worked
pub fn undo(id: &str) -> Result<String, String> {
    let mut l = list();
    let i = if id.is_empty() { l.iter().rposition(can_undo) } else { l.iter().position(|e| e.id == id) }.ok_or("Нечего отменять.")?;
    if l[i].undone {
        return Err("Это уже отменено.".into());
    }
    let r = apply(&l[i].undo)?;
    l[i].undone = true;
    store::write(FILE, &l)?;
    Ok(format!("{} ({})", r, l[i].what))
}

pub fn clear() -> bool {
    store::write(FILE, &Vec::<Entry>::new()).is_ok()
}

// «верни как было» / «откати последнее действие» / «undo last action»; «покажи историю действий»
pub fn parse(text: &str) -> Option<bool> {
    let n = crate::textnorm::normalize(text);
    match n.as_str() {
        "верни как было" | "откати последнее действие" | "откати последнее" | "undo last action" | "undo that" | "отмени последнее действие джарвиса" => Some(true),
        "покажи историю действий" | "что ты сделал" | "история действий" | "show action history" => Some(false),
        _ => None,
    }
}

pub fn summary(n: usize) -> String {
    let l = list();
    if l.is_empty() {
        return "История действий пуста.".into();
    }
    l.iter().rev().take(n).map(|e| format!("— {}{}", e.what, if e.undone { " (отменено)" } else if e.undo == Undo::None { " (нельзя отменить)" } else { "" })).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_takes_back_the_last_undoable_action_once() {
        store::with_temp(|| {
            let id = match memory::add("мой кот — Барсик", "t").unwrap() { memory::Added::Added(i) => i, _ => panic!() };
            record("Запомнил: кот", Undo::MemoryAdd(id));
            record("Открыл чат с Маликой", Undo::None);
            // the last action cannot be undone: the one before it is
            let r = undo("").unwrap();
            assert!(r.contains("Убрал запись из памяти"), "{}", r);
            assert!(memory::load().items.is_empty());
            assert!(undo("").is_err(), "nothing left, and the same one is not undone twice");
            assert!(summary(5).contains("Открыл чат") && summary(5).contains("(нельзя отменить)") && summary(5).contains("(отменено)"));
        });
    }

    #[test]
    fn a_job_and_a_file_can_be_undone_and_a_vanished_thing_is_reported() {
        store::with_temp(|| {
            let j = scheduler::add("открой почту", 10, scheduler::Repeat::Daily).unwrap();
            record("Задание", Undo::JobAdd(j.id.clone()));
            assert!(undo("").is_ok() && scheduler::list().is_empty());
            let d = tempfile::tempdir().unwrap();
            let f = d.path().join("a.png");
            std::fs::write(&f, b"x").unwrap();
            record("Картинка", Undo::FileCreated(f.to_string_lossy().into()));
            assert!(undo("").is_ok() && !f.exists());
            // already gone
            record("Картинка 2", Undo::FileCreated(f.to_string_lossy().into()));
            assert!(undo("").is_err());
            assert!(!list().last().unwrap().undone, "a failed undo is not marked as done");
        });
    }

    #[test]
    fn a_changed_file_is_restored_from_its_copy() {
        store::with_temp(|| {
            let d = tempfile::tempdir().unwrap();
            let (f, b) = (d.path().join("a.txt"), d.path().join("a.txt.jarvis-bak"));
            std::fs::write(&f, "new").unwrap();
            std::fs::write(&b, "old").unwrap();
            record("Правка a.txt", Undo::FileRestore(f.to_string_lossy().into(), b.to_string_lossy().into()));
            assert!(undo("").is_ok());
            assert_eq!(std::fs::read_to_string(&f).unwrap(), "old");
        });
    }

    #[test]
    fn phrases() {
        assert_eq!(parse("Верни как было"), Some(true));
        assert_eq!(parse("покажи историю действий"), Some(false));
        assert_eq!(parse("отмени последнее действие"), None, "that phrase belongs to the document sessions");
    }

    #[test]
    fn a_deleted_or_edited_fact_can_be_taken_back_once_and_only_while_it_is_as_left() {
        store::with_temp(|| {
            memory::add("Мой любимый цвет зелёный", "test").unwrap();
            let e = memory::load().items[0].clone();
            // delete → undo puts the same entry back
            assert!(memory::delete(&e.id));
            let id = record("Удалена запись памяти", Undo::MemoryRestore(e.clone()));
            assert!(memory::load().items.is_empty());
            assert!(undo(&id).unwrap().contains("Вернул"));
            assert_eq!(memory::load().items, vec![e.clone()]);
            assert!(undo(&id).is_err(), "a second undo of the same action is refused");
            // edit → undo returns the old text
            let old = memory::update(&e.id, "Мой любимый цвет синий").unwrap();
            let now = memory::get(&e.id).unwrap().text;
            let id2 = record("Правка", Undo::MemoryRevert(e.id.clone(), old, now));
            assert!(undo(&id2).is_ok());
            assert_eq!(memory::get(&e.id).unwrap().text, "Мой любимый цвет зелёный");
            // edit, then edited again: the first edit is NOT silently overwritten back
            let old = memory::update(&e.id, "Первая правка").unwrap();
            let id3 = record("Правка 1", Undo::MemoryRevert(e.id.clone(), old, "Первая правка".into()));
            memory::update(&e.id, "Вторая правка").unwrap();
            assert!(undo(&id3).unwrap_err().contains("уже изменена"));
            assert_eq!(memory::get(&e.id).unwrap().text, "Вторая правка");
        });
    }

    #[test]
    fn a_deleted_note_comes_back_and_a_note_edit_is_reverted() {
        store::with_temp(|| {
            let n = notes::add("купить хлеб", "note", 1000).unwrap();
            assert!(notes::delete(&n.id));
            let id = record("Удалена запись дневника", Undo::NoteRestore(n.clone()));
            assert!(undo(&id).is_ok());
            assert_eq!(notes::get(&n.id).unwrap().text, "купить хлеб");
            let old = notes::update(&n.id, "купить молоко").unwrap();
            let id = record("Правка", Undo::NoteRevert(n.id.clone(), old, "купить молоко".into()));
            assert!(undo(&id).is_ok());
            assert_eq!(notes::get(&n.id).unwrap().text, "купить хлеб");
        });
    }
}
