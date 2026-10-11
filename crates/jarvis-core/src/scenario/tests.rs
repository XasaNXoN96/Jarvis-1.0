use super::*;

static LOCK: Mutex<()> = Mutex::new(());

fn step(kind: &str, param: &str) -> Step {
    Step { kind: kind.into(), param: param.into(), window: String::new() }
}

fn sc(name: &str, phrase: &str, steps: Vec<Step>) -> Scenario {
    Scenario { name: name.into(), phrase: phrase.into(), steps, ..Default::default() }
}

#[test]
fn every_step_is_checked_before_it_is_saved() {
    assert!(check_step(&step("link", "https://example.com")).is_ok());
    assert!(check_step(&step("link", "example.com")).is_err(), "only http(s) links");
    assert!(check_step(&step("link", "javascript:alert(1)")).is_err());
    assert!(check_step(&step("keys", "ctrl+t")).is_ok());
    assert!(check_step(&step("keys", "ctrl+bogus")).is_err());
    assert!(check_step(&step("volume", "40")).is_ok());
    assert!(check_step(&step("volume", "140")).is_err());
    assert!(check_step(&step("pause", "1,5")).is_ok());
    assert!(check_step(&step("pause", "600")).is_err(), "no endless pauses");
    assert!(check_step(&step("mouse", "double")).is_ok());
    assert!(check_step(&step("mouse", "wiggle")).is_err());
    assert!(check_step(&step("text", "")).is_err());
    let shell = check_step(&step("shell", "del /q *")).unwrap_err();
    assert!(shell.contains("не поддерживаются"), "arbitrary commands are not a step: {}", shell);
    assert!(check(&sc("", "фраза", vec![step("volume", "10")])).is_err());
    assert!(check(&sc("Имя", "  ", vec![step("volume", "10")])).is_err());
    assert!(check(&sc("Имя", "фраза", vec![])).is_err());
    let bad = check(&sc("Имя", "фраза", vec![step("volume", "10"), step("keys", "??")])).unwrap_err();
    assert!(bad.starts_with("Шаг 2"), "{}", bad);
}

#[test]
fn scenarios_are_created_edited_found_by_phrase_and_deleted() {
    let _g = LOCK.lock();
    reset_for_tests();
    let a = upsert(sc("Работа", "Начать рабочий день!", vec![step("volume", "30"), step("pause", "0.1")])).unwrap();
    assert!(!a.id.is_empty() && a.enabled);
    assert_eq!(find("начать рабочий день").map(|s| s.id), Some(a.id.clone()), "punctuation and case do not matter");
    assert_eq!(find("Начать Рабочий День").map(|s| s.id), Some(a.id.clone()));
    assert!(find("начать рабочий").is_none(), "a part of the phrase is not the phrase");
    // another scenario cannot take the same phrase
    assert!(upsert(sc("Другой", "начать рабочий день", vec![step("volume", "10")])).unwrap_err().contains("уже занята"));
    // editing keeps the id; steps can be reordered and removed
    let mut b = a.clone();
    b.steps = vec![step("pause", "0.1"), step("keys", "win+d")];
    let b = upsert(b).unwrap();
    assert_eq!(b.id, a.id);
    assert_eq!(get(&a.id).unwrap().steps[1].kind, "keys");
    // switched off: the phrase does nothing
    assert!(set_enabled(&a.id, false));
    assert!(find("начать рабочий день").is_none());
    assert!(delete(&a.id));
    assert!(get(&a.id).is_none());
    assert!(!delete(&a.id));
}

#[test]
fn a_run_stops_at_the_step_that_fails_and_says_which_one() {
    let _g = LOCK.lock();
    reset_for_tests();
    let s = upsert(sc("Тест", "тестовый сценарий", vec![step("pause", "0.1"), step("file", "/no/such/file/ever.txt"), step("pause", "0.1")])).unwrap();
    let r = run(&s);
    assert!(!r.ok);
    assert_eq!((r.done, r.total), (1, 3));
    assert!(r.message.contains("Шаг 2") && r.message.contains("не найдены"), "{}", r.message);
    // the run is remembered (history of runs)
    assert_eq!(get(&s.id).unwrap().last_run.unwrap().done, 1);
    let ok = upsert(sc("Пауза", "просто пауза", vec![step("pause", "0.1"), step("pause", "0.1")])).unwrap();
    let r = run(&ok);
    assert!(r.ok && r.done == 2, "{:?}", r);
}

#[test]
fn export_import_and_templates() {
    let _g = LOCK.lock();
    reset_for_tests();
    upsert(sc("Кино", "режим кино", vec![step("volume", "70"), step("link", "https://www.youtube.com")])).unwrap();
    let doc = export_json();
    assert_eq!(doc["scenarios"].as_array().unwrap().len(), 1);
    assert!(doc["scenarios"][0].get("id").is_none(), "ids of this computer are not exported");
    // importing the same file again does not double it
    let (added, skipped) = import_json(&doc, false).unwrap();
    assert_eq!(added, 0);
    assert_eq!(skipped.len(), 1);
    // replace: the list becomes exactly the file
    let (added, _) = import_json(&doc, true).unwrap();
    assert_eq!((added, list().len()), (1, 1));
    // a broken or hostile file
    assert!(import_json(&json!({ "x": 1 }), false).is_err());
    let (added, skipped) = import_json(&json!({ "scenarios": [{ "name": "Зло", "phrase": "зло", "steps": [{ "kind": "shell", "param": "format c:" }] }] }), false).unwrap();
    assert_eq!(added, 0);
    assert!(skipped[0].contains("не поддерживаются"));
    // every template is a valid scenario
    for t in templates().as_array().unwrap() {
        let s: Scenario = serde_json::from_value(t.clone()).unwrap();
        assert!(check(&s).is_ok(), "{:?}", s);
    }
    reset_for_tests();
}

#[test]
fn a_program_step_finds_known_programs_by_name() {
    assert!(app_target("блокнот").to_lowercase().contains("notepad"));
    assert_eq!(app_target("какая-то своя программа"), "какая-то своя программа");
}
