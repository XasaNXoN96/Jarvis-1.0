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
    // a job that was edited: (the job before, the job after the edit); undone only while the job is still as the edit left it
    JobRevert(scheduler::Job, scheduler::Job),
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

// The list is read, changed and written back: two changes at the same moment (a command from the voice and one from the phone, an undo and a new
// record) must not lose each other's line. Only the short read-change-write is under the lock, never the undo itself.
static LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

pub fn record(what: &str, undo: Undo) -> String {
    let _g = LOCK.lock();
    let mut l = list();
    let id = store::new_id();
    l.push(Entry { id: id.clone(), at: store::now(), what: what.chars().take(200).collect(), undo, undone: false });
    let n = l.len();
    if n > MAX {
        l.drain(..n - MAX);
    }
    if let Err(e) = store::write(FILE, &l) {
        log::warn!("action history: a line could not be saved: {}", e);
    }
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
        Undo::JobRevert(old, now) => scheduler::restore(old, now).map(|_| "Вернул прежнее время и фразу задания.".to_string()),
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

// undoes the entry (or the last one that can be undone when `id` is empty); marks it undone only when it worked.
// The entry is CLAIMED first (marked under the lock), so two undos at once cannot both apply it; if the undo fails the mark is taken off.
pub fn undo(id: &str) -> Result<String, String> {
    let (entry_id, what, action) = {
        let _g = LOCK.lock();
        let mut l = list();
        let i = if id.is_empty() { l.iter().rposition(can_undo) } else { l.iter().position(|e| e.id == id) }.ok_or("Нечего отменять.")?;
        if l[i].undone {
            return Err("Это уже отменено.".into());
        }
        if l[i].undo == Undo::None {
            return Err("Это действие нельзя отменить.".into());
        }
        l[i].undone = true;
        store::write(FILE, &l)?;
        (l[i].id.clone(), l[i].what.clone(), l[i].undo.clone())
    };
    match apply(&action) {
        Ok(r) => Ok(format!("{} ({})", r, what)),
        Err(e) => {
            let _g = LOCK.lock();
            let mut l = list();
            if let Some(x) = l.iter_mut().find(|x| x.id == entry_id) {
                x.undone = false;
            }
            let _ = store::write(FILE, &l);
            Err(e)
        }
    }
}

pub fn clear() -> bool {
    let _g = LOCK.lock();
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

// The list for the page, newest first: `q` (a part of the text, case does not matter) and `state`: "" / "all", "undoable" (can be taken back
// now), "undone", "irreversible" (cannot be taken back). An unknown state shows nothing wrong: it is the same as all.
pub fn filtered(q: &str, state: &str, n: usize) -> Vec<Entry> {
    let q = crate::textnorm::normalize(q);
    list()
        .into_iter()
        .rev()
        .filter(|e| match state {
            "undoable" => can_undo(e),
            "undone" => e.undone,
            "irreversible" => !e.undone && e.undo == Undo::None,
            _ => true,
        })
        .filter(|e| q.is_empty() || crate::textnorm::normalize(&e.what).contains(&q))
        .take(n)
        .collect()
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
    fn an_edited_job_is_put_back_by_the_undo() {
        store::with_temp(|| {
            let j = scheduler::add("открой почту", 10, scheduler::Repeat::Once).unwrap();
            let now = chrono::NaiveDate::from_ymd_opt(2026, 10, 22).unwrap().and_hms_opt(8, 0, 0).unwrap();
            let (old, new) = scheduler::edit(&j.id, "каждый день в 10:00 открой блокнот", now, 0).unwrap();
            record("Изменено задание", Undo::JobRevert(old, new));
            assert!(undo("").unwrap().contains("прежнее время"));
            assert_eq!(scheduler::list()[0].text, "открой почту");
            assert!(undo("").is_err());
        });
    }

    #[test]
    fn the_history_can_be_filtered_by_a_word_and_by_state() {
        store::with_temp(|| {
            let id = match memory::add("мой кот — Барсик", "t").unwrap() { memory::Added::Added(i) => i, _ => panic!() };
            record("Запомнил: кот Барсик", Undo::MemoryAdd(id));
            record("Открыл чат с Маликой", Undo::None);
            let j = scheduler::add("открой почту", 10, scheduler::Repeat::Daily).unwrap();
            record("Задание: почта", Undo::JobAdd(j.id));
            assert_eq!(filtered("", "", 10).len(), 3);
            assert_eq!(filtered("", "all", 10).len(), 3);
            assert_eq!(filtered("БАРСИК", "", 10).len(), 1, "a word, any case");
            assert_eq!(filtered("чат", "irreversible", 10).len(), 1);
            assert_eq!(filtered("чат", "undoable", 10).len(), 0);
            assert_eq!(filtered("", "undoable", 10).len(), 2);
            assert_eq!(filtered("", "garbage", 10).len(), 3, "an unknown state is the same as all");
            assert_eq!(filtered("", "", 2).len(), 2, "cut to n");
            assert!(filtered("", "", 10)[0].what.starts_with("Задание"), "newest first");
            undo("").unwrap();
            assert_eq!(filtered("", "undone", 10).len(), 1);
            assert_eq!(filtered("", "irreversible", 10).len(), 1, "an undone entry is not «irreversible»");
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

    // two records at once keep both lines; two undos of one entry apply it once
    #[test]
    fn simultaneous_records_are_all_kept_and_one_entry_is_undone_only_once() {
        store::with_temp(|| {
            let threads: Vec<_> = (0..8).map(|t| std::thread::spawn(move || for k in 0..10 { record(&format!("действие {}-{}", t, k), Undo::None); })).collect();
            for h in threads {
                h.join().unwrap();
            }
            assert_eq!(list().len(), 80, "no line was lost");
            let mut ids: Vec<String> = list().into_iter().map(|e| e.id).collect();
            ids.sort();
            ids.dedup();
            assert_eq!(ids.len(), 80);
            // one added fact, undone from 6 threads at once: it is taken back exactly once
            let added = match memory::add("факт для гонки", "t").unwrap() { memory::Added::Added(i) => i, _ => panic!() };
            let id = record("Запомнил", Undo::MemoryAdd(added));
            let results: Vec<_> = (0..6).map(|_| { let id = id.clone(); std::thread::spawn(move || undo(&id)) }).collect::<Vec<_>>().into_iter().map(|h| h.join().unwrap()).collect();
            assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1, "{:?}", results);
            assert!(memory::load().items.is_empty());
            assert!(list().iter().find(|e| e.id == id).unwrap().undone);
        });
    }

    #[test]
    fn an_action_that_cannot_be_undone_says_so_and_a_failed_undo_stays_open() {
        store::with_temp(|| {
            let id = record("Открыл чат", Undo::None);
            assert_eq!(undo(&id).unwrap_err(), "Это действие нельзя отменить.");
            assert!(!list()[0].undone);
            let id = record("Запись, которой уже нет", Undo::MemoryAdd("нет-такой".into()));
            assert!(undo(&id).is_err());
            assert!(!list().last().unwrap().undone, "a failed undo can be tried again later");
        });
    }
}
