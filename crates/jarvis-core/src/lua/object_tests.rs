// Automated tests of the document mode ("Работа с объектом"), dictation and the program names: the real Lua scripts
// and command triggers run against a fake Word / Excel / Notepad (testdata/fake_office.py), so the whole dialog is
// checked without Windows. What the fake cannot show (the real COM calls) needs a check on a real Windows.
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::commands::{self, JCommandsList};
use crate::lua::{execute, CommandContext, SandboxLevel};

// the session, the script state and the environment are global: one scenario at a time
static LOCK: Mutex<()> = Mutex::new(());

struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
    list: Vec<JCommandsList>,
    state: PathBuf,
    typed: PathBuf,
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

impl World {
    fn new(files: Value) -> World {
        crate::i18n::init("ru");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("commands");
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        copy_dir(&manifest.join("../../resources/commands"), &root);
        let state = dir.path().join("fake_state.json");
        let typed = dir.path().join("typed.txt");
        let initial = json!({
            "word": { "running": true, "docs": [], "active": -1 },
            "excel": { "running": true, "books": [], "active": -1 },
            "notepad": { "running": false, "title": "Безымянный", "text": "" },
            "files": files, "recycled": []
        });
        fs::write(&state, initial.to_string()).unwrap();
        let runner = manifest.join("src/lua/testdata/fake_office.py");
        std::env::set_var("FAKE_STATE", &state);
        std::env::set_var("JARVIS_TEST_TYPED", &typed);
        std::env::set_var("JARVIS_TEST_RUNNER", &runner);
        crate::session::end();

        let mut list = Vec::new();
        for e in fs::read_dir(&root).unwrap().flatten() {
            let toml_file = e.path().join("command.toml");
            if let Ok(text) = fs::read_to_string(&toml_file) {
                let file: JCommandsList = toml::from_str(&text).unwrap();
                list.push(JCommandsList { path: e.path(), commands: file.commands });
            }
        }
        World { _dir: dir, root, list, state, typed }
    }

    fn fake(&self) -> Value {
        serde_json::from_str(&fs::read_to_string(&self.state).unwrap()).unwrap()
    }

    fn set_fake(&self, f: impl FnOnce(&mut Value)) {
        let mut v = self.fake();
        f(&mut v);
        fs::write(&self.state, v.to_string()).unwrap();
    }

    fn typed(&self) -> String {
        fs::read_to_string(&self.typed).unwrap_or_default()
    }

    fn script_state(&self, key: &str) -> String {
        let p = self.root.join("assistant/.state.json");
        let v: Value = fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(json!({}));
        v.get(key).and_then(|x| x.as_str()).unwrap_or("").to_string()
    }

    // the id of the command a phrase would run (the same order as the application: session, program name, triggers)
    fn route(&self, text: &str) -> Option<String> {
        if crate::session::kind().is_some() {
            return Some("assistant_session".to_string());
        }
        commands::fetch_program(text, &self.list).or_else(|| commands::fetch_dictation(text, &self.list)).or_else(|| commands::fetch_by_triggers(text, &self.list)).map(|(_, c)| c.id.clone())
    }

    // says a phrase; returns the id of the command that handled it
    fn say(&self, text: &str) -> String {
        // like the application: the recognized / typed phrase arrives in lower case
        let text = text.to_lowercase();
        let text = text.as_str();
        let id = self.route(text).unwrap_or_else(|| panic!("no command for '{}'", text));
        let (path, cmd) = self
            .list
            .iter()
            .flat_map(|l| l.commands.iter().map(move |c| (&l.path, c)))
            .find(|(_, c)| c.id == id)
            .unwrap();
        if id == "assistant_session" {
            crate::session::touch();
        }
        let ctx = CommandContext { phrase: text.to_string(), command_id: cmd.id.clone(), command_path: path.clone(), language: "ru".into(), slots: None };
        let script = path.join(&cmd.script);
        execute(&script, ctx, SandboxLevel::Full, Duration::from_secs(25)).unwrap_or_else(|e| panic!("'{}' failed: {:?}", text, e));
        id
    }

    fn step(&self) -> String {
        format!("{}:{}", crate::session::kind().unwrap_or_default(), crate::session::data())
    }
}

fn doc_text(w: &World, app: &str, idx: usize) -> String {
    let key = if app == "word" { "docs" } else { "books" };
    w.fake()[app][key][idx]["text"].as_str().unwrap_or("").to_string()
}

#[test]
fn triggers_separate_program_names_questions_and_the_work_mode() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    assert_eq!(w.route("открой ворт").as_deref(), Some("assistant_program_open"));
    assert_eq!(w.route("открой эксел").as_deref(), Some("assistant_program_open"));
    assert_eq!(w.route("открой тотал командер").as_deref(), Some("assistant_program_open"));
    assert_eq!(w.route("закрой word").as_deref(), Some("assistant_program_close"));
    assert_eq!(w.route("начинаем работу с объектом").as_deref(), Some("assistant_object_start"));
    assert_eq!(w.route("начни работу с объектом").as_deref(), Some("assistant_object_start"));
    // an explicit search in Wikipedia still works, but a plain question is no longer a Wikipedia command
    assert!(w.route("найди в википедии Ташкент").is_some());
    for q in ["что такое программа", "кто такой Эйнштейн", "расскажи про Ташкент", "объясни как работает excel", "напиши программу на Python"] {
        assert_eq!(w.route(q), None, "'{}' must not be run by a command", q);
        assert!(crate::route::is_knowledge_question(q), "{}", q);
    }
    // small talk of Jarvis himself stays with the commands
    assert!(w.route("кто такой джарвис").is_some());
    assert!(w.route("что ты умеешь").is_some());
}

#[test]
fn work_mode_word_new_document_dictation_and_save() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    assert_eq!(w.say("открой ворт"), "assistant_program_open");
    assert_eq!(w.script_state("office_app"), "word");

    // Word is open without a document: the question "new or existing?"
    assert_eq!(w.say("начинаем работу с объектом"), "assistant_object_start");
    assert_eq!(w.step(), "object:doc_ask");
    w.say("создать новый документ");
    assert_eq!(w.step(), "object:typing");
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 1);

    // the dictated text goes into the document and nowhere else
    assert_eq!(w.say("Привет, как дела? Сегодня мы изучаем информатику"), "assistant_session");
    let text = doc_text(&w, "word", 0);
    assert!(text.contains("Привет, как дела?") && text.contains("информатику"), "{}", text);
    assert_eq!(w.typed(), "", "nothing may be typed into another window");

    // the end phrase asks, it does not save or delete by itself
    w.say("Заканчиваем работу с объектом");
    assert_eq!(w.step(), "object:ask_end");
    assert!(w.fake()["files"].as_object().unwrap().is_empty());
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 1);

    // an unclear answer repeats the question and is not typed into the document
    w.say("ну не знаю");
    assert_eq!(w.step(), "object:ask_end");
    assert_eq!(doc_text(&w, "word", 0), text);

    // save: a new document needs a name and a place
    w.say("сохранить");
    assert_eq!(w.step(), "object:save_name");
    w.say("отчёт по информатике на рабочем столе");
    assert_eq!(crate::session::kind(), None, "the work mode is finished after saving");
    let files = w.fake()["files"].clone();
    assert_eq!(files["/desktop/Отчёт по информатике.docx"].as_str().unwrap_or("MISSING"), text, "{}", files);
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 1, "the document stays open");
}

#[test]
fn work_mode_save_does_not_overwrite_without_asking() {
    let _g = LOCK.lock();
    let w = World::new(json!({ "/docs/План.docx": "старый текст" }));
    w.say("открой word");
    w.say("начинаем работу с объектом");
    w.say("новый");
    w.say("новый план");
    w.say("заканчиваем работу с объектом");
    w.say("сохранить");
    w.say("план");
    // the file exists: the question, nothing replaced
    assert_eq!(w.step(), "object:overwrite");
    assert_eq!(w.fake()["files"]["/docs/План.docx"].as_str().unwrap(), "старый текст");
    w.say("нет");
    assert_eq!(w.step(), "object:save_name");
    assert_eq!(w.fake()["files"]["/docs/План.docx"].as_str().unwrap(), "старый текст");
    w.say("план");
    w.say("да");
    assert!(w.fake()["files"]["/docs/План.docx"].as_str().unwrap().contains("Новый план"));
    assert_eq!(crate::session::kind(), None);
}

#[test]
fn work_mode_delete_needs_a_confirmation() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.say("открой ворд");
    w.say("начинаем работу с объектом");
    w.say("создать новый");
    w.say("важный текст");
    w.say("заканчиваем работу с объектом");

    // "delete" only asks; the document is still open and untouched
    w.say("удалить");
    assert_eq!(w.step(), "object:confirm_delete");
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 1);
    // a "no" goes back to editing
    w.say("нет");
    assert_eq!(w.step(), "object:typing");
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 1);
    // something that is not a clear "yes" does not delete either
    w.say("заканчиваем работу с объектом");
    w.say("удалить");
    w.say("может быть");
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 1);
    assert_eq!(w.step(), "object:typing");
    // a clear "yes": the document is closed without saving
    w.say("заканчиваем работу с объектом");
    w.say("удалить");
    w.say("да");
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 0);
    assert_eq!(crate::session::kind(), None);
    assert!(w.fake()["recycled"].as_array().unwrap().is_empty(), "an unsaved document has no file to remove");
}

#[test]
fn work_mode_deleting_a_saved_document_moves_its_file_to_the_recycle_bin() {
    let _g = LOCK.lock();
    let w = World::new(json!({ "/docs/Старый.docx": "текст" }));
    w.say("открой word");
    w.say("начинаем работу с объектом");
    w.say("открыть существующий");
    assert_eq!(w.step(), "object:doc_name");
    w.say("старый");
    assert_eq!(w.step(), "object:typing");
    assert_eq!(w.fake()["word"]["docs"][0]["name"].as_str().unwrap(), "Старый.docx");
    w.say("заканчиваем работу с объектом");
    w.say("удалить");
    w.say("подтверждаю");
    assert_eq!(w.fake()["recycled"][0].as_str().unwrap(), "/docs/Старый.docx");
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 0);
}

#[test]
fn work_mode_continue_and_cancel_return_to_the_document() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.say("открой word");
    w.say("начинаем работу с объектом");
    w.say("новый");
    w.say("первая часть");
    w.say("заканчиваем работу с объектом");
    w.say("продолжить редактирование");
    assert_eq!(w.step(), "object:typing");
    w.say("вторая часть");
    w.say("заканчиваем работу с объектом");
    w.say("отмена");
    assert_eq!(w.step(), "object:typing");
    w.say("третья часть");
    let text = doc_text(&w, "word", 0);
    assert!(text.contains("Первая часть") && text.contains("вторая часть") && text.contains("третья часть"), "{}", text);
}

#[test]
fn work_mode_save_of_a_document_that_already_has_a_file_asks_nothing() {
    let _g = LOCK.lock();
    let w = World::new(json!({ "/docs/Отчёт.docx": "старый" }));
    w.say("открой word");
    w.say("начинаем работу с объектом");
    w.say("существующий");
    w.say("отчёт");
    w.say("дополнение");
    w.say("заканчиваем работу с объектом");
    w.say("сохранить");
    assert_eq!(crate::session::kind(), None);
    assert!(w.fake()["files"]["/docs/Отчёт.docx"].as_str().unwrap().contains("Дополнение"));
}

#[test]
fn work_mode_never_types_into_a_closed_program_or_another_document() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.say("открой word");
    w.say("начинаем работу с объектом");
    w.say("новый");
    w.say("мой текст");
    let mine = doc_text(&w, "word", 0);

    // another document becomes the active one: the text is refused, neither document changes
    w.set_fake(|v| {
        v["word"]["docs"].as_array_mut().unwrap().push(json!({ "name": "Чужой.docx", "text": "", "fmt": [], "cells": {}, "saved": true }));
        v["word"]["active"] = json!(1);
    });
    w.say("этого не должно быть");
    assert_eq!(doc_text(&w, "word", 0), mine);
    assert_eq!(doc_text(&w, "word", 1), "");
    assert_eq!(w.step(), "object:typing", "the mode stays on");

    // Word is closed: it is not started again
    w.set_fake(|v| {
        v["word"]["running"] = json!(false);
        v["word"]["docs"] = json!([]);
    });
    w.say("и этого тоже");
    assert_eq!(w.fake()["word"]["running"], json!(false), "a closed program must not be started behind the user's back");
    assert_eq!(w.typed(), "");
}

#[test]
fn work_mode_word_formatting_commands() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.say("открой word");
    w.say("начинаем работу с объектом");
    w.say("новый");
    w.say("включи жирный");
    w.say("заголовок");
    w.say("по центру");
    w.say("выключи жирный");
    let fmt = w.fake()["word"]["docs"][0]["fmt"].clone();
    let fmt: Vec<&str> = fmt.as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(fmt.first(), Some(&"bold_on"));
    assert!(fmt.contains(&"bold_off"));
    // plain words are text, not a command
    assert!(doc_text(&w, "word", 0).contains("Заголовок") || doc_text(&w, "word", 0).contains("заголовок"));
}

#[test]
fn work_mode_excel_cells_formulas_and_the_active_cell() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    assert_eq!(w.say("открой эксел"), "assistant_program_open");
    assert_eq!(w.script_state("office_app"), "excel");
    w.say("начинаем работу с объектом");
    assert_eq!(w.step(), "object:doc_ask");
    w.say("новую таблицу");
    assert_eq!(w.step(), "object:typing");

    // existing cell commands keep working, formulas too
    w.say("ячейка а1 пиши 5");
    w.say("ячейка б1 пиши 7");
    w.say("ячейка c1 сделай а плюс б");
    let cells = w.fake()["excel"]["books"][0]["cells"].clone();
    assert_eq!(cells["A1"], json!("5"));
    assert_eq!(cells["B1"], json!("7"));
    assert_eq!(cells["C1"], json!("=A1+B1"));

    // values without a cell go into the active cell and move down
    w.say("итого");
    w.say("12");
    let cells = w.fake()["excel"]["books"][0]["cells"].clone();
    assert_eq!(cells["A1"], json!("Итого"), "the first free dictated value lands in the active cell");
    assert_eq!(cells["A2"], json!("12"));

    w.say("заканчиваем работу с объектом");
    w.say("сохранить");
    w.say("таблица на рабочем столе");
    assert!(w.fake()["files"]["/desktop/Таблица.xlsx"].is_string());
    assert_eq!(crate::session::kind(), None);
}

#[test]
fn work_mode_notepad_types_only_into_the_notepad_window_and_saves() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.set_fake(|v| v["notepad"]["running"] = json!(true));
    w.say("открой блокнот");
    assert_eq!(w.script_state("office_app"), "notepad");
    w.say("начинаем работу с объектом");
    assert_eq!(w.step(), "object:typing", "an open Notepad is used at once");
    w.say("первая строка точка");
    let typed = w.typed();
    assert!(typed.starts_with("Notepad\t"), "{}", typed);
    assert!(typed.contains("Первая строка."), "{}", typed);
    assert_eq!(w.typed().lines().count(), 1);

    w.say("заканчиваем работу с объектом");
    w.say("сохранить");
    assert_eq!(w.step(), "object:save_name");
    w.say("заметки");
    assert!(w.fake()["files"]["/docs/Заметки.txt"].is_string());
    assert_eq!(crate::session::kind(), None);
}

#[test]
fn work_mode_notepad_discard_needs_the_confirmation_too() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.set_fake(|v| v["notepad"]["running"] = json!(true));
    w.say("открой блокнот");
    w.say("начинаем работу с объектом");
    w.say("текст");
    w.say("заканчиваем работу с объектом");
    w.say("удалить");
    assert_eq!(w.fake()["notepad"]["running"], json!(true), "nothing is closed before the confirmation");
    w.say("нет");
    assert_eq!(w.fake()["notepad"]["running"], json!(true));
    w.say("заканчиваем работу с объектом");
    w.say("удалить");
    w.say("да");
    assert_eq!(w.fake()["notepad"]["running"], json!(false));
}

#[test]
fn text_after_the_write_command_is_typed_into_the_document_not_run_as_a_command() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.say("открой word");
    w.set_fake(|v| v["word"]["docs"] = json!([{ "name": "Document1", "text": "", "fmt": [], "cells": {}, "saved": false }]));
    w.set_fake(|v| v["word"]["active"] = json!(0));
    // "Пиши: привет, как дела?" is a dictation command with its text
    assert_eq!(w.say("пиши: привет, как дела?"), "assistant_office_dictate");
    assert!(doc_text(&w, "word", 0).contains("Привет, как дела?"), "{}", doc_text(&w, "word", 0));
    // the dictation goes on, and ends only on the end word
    w.say("это второе предложение");
    assert!(doc_text(&w, "word", 0).contains("второе предложение"));
    assert_eq!(w.say("заканчивай"), "assistant_session");
    assert_eq!(crate::session::kind(), None);
}

#[test]
fn the_old_dictation_into_excel_still_works() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.say("открой эксель");
    w.say("пиши в экселе");
    assert_eq!(w.step(), "dictation:excel");
    w.say("ячейка а1 пиши 5");
    w.say("ячейка б1 пиши 7");
    w.say("ячейка c1 сделай а плюс б");
    let cells = w.fake()["excel"]["books"][0]["cells"].clone();
    assert_eq!(cells["A1"], json!("5"));
    assert_eq!(cells["C1"], json!("=A1+B1"));
    w.say("заканчивай");
    assert_eq!(crate::session::kind(), None);
}

#[test]
fn the_work_mode_can_start_from_a_dictation_and_asks_for_the_program_when_it_is_unknown() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    // no program was opened before: Jarvis asks
    w.set_fake(|v| v["notepad"]["running"] = json!(false));
    w.say("начинаем работу с объектом");
    assert_eq!(w.step(), "object:app_ask");
    w.say("в ворде");
    assert_eq!(w.step(), "object:doc_ask");
    // "cancel" leaves the mode without touching anything
    w.say("отмена");
    assert_eq!(crate::session::kind(), None);
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 0);
}

#[test]
fn work_mode_does_not_trap_the_user_when_the_answer_is_never_understood() {
    let _g = LOCK.lock();
    let w = World::new(json!({}));
    w.say("открой word");
    w.say("начинаем работу с объектом");
    w.say("новый");
    w.say("мой текст");
    w.say("заканчиваем работу с объектом");
    w.say("эээ");
    w.say("хм");
    assert_eq!(w.step(), "object:ask_end", "two misses: the question is asked again");
    // a clear answer still works after the misses
    w.say("продолжить");
    assert_eq!(w.step(), "object:typing");
    w.say("заканчиваем работу с объектом");
    w.say("эээ");
    w.say("хм");
    w.say("угу ладно");
    // the third miss ends the mode; the document stays open and untouched, nothing is saved or deleted
    assert_eq!(crate::session::kind(), None);
    assert_eq!(w.fake()["word"]["docs"].as_array().unwrap().len(), 1);
    assert!(w.fake()["files"].as_object().unwrap().is_empty());
    assert!(w.fake()["recycled"].as_array().unwrap().is_empty());
}
