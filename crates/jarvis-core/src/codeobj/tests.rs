use super::*;
use std::sync::atomic::Ordering;
use std::sync::mpsc;

use crate::objects::{Front, Verify};

// ---------- pure helpers ----------
#[test]
fn file_names_from_speech() {
    assert_eq!(file_name_from_speech("calculator.py", "py").as_deref(), Some("calculator.py"));
    assert_eq!(file_name_from_speech("назови calculator.py", "py").as_deref(), Some("calculator.py"));
    assert_eq!(file_name_from_speech("Назови файл калькулятор точка пи вай", "py").as_deref(), Some("kalkulyator.py"));
    assert_eq!(file_name_from_speech("мой калькулятор", "py").as_deref(), Some("moy_kalkulyator.py"));
    assert_eq!(file_name_from_speech("notes точка md", "txt").as_deref(), Some("notes.md"));
    assert_eq!(file_name_from_speech("назови файл", "py"), None);
    // nothing of a path survives
    let n = file_name_from_speech("..\\..\\windows\\evil", "py").unwrap();
    assert!(!n.contains('\\') && !n.contains('/') && !n.contains(".."), "{}", n);
}

#[test]
fn places_from_speech() {
    assert_eq!(place_from_speech("в документах"), Some(Place { folder: "documents", sub: None }));
    assert_eq!(place_from_speech("на рабочем столе папка калькулятор"), Some(Place { folder: "desktop", sub: Some("калькулятор".into()) }));
    assert_eq!(place_from_speech("загрузки"), Some(Place { folder: "downloads", sub: None }));
    assert_eq!(place_from_speech("по умолчанию"), Some(Place { folder: "documents", sub: None }));
    assert_eq!(place_from_speech("куда-нибудь"), None);
    // a folder name never climbs out
    let p = place_from_speech("в документах папка ..\\..\\windows").unwrap();
    assert!(p.sub.as_deref().map_or(true, |s| !s.contains('\\') && !s.contains('/')), "{:?}", p);
}

#[test]
fn code_is_taken_from_an_answer() {
    assert_eq!(extract_code("Вот код:\n```python\nprint(1)\n```\nГотово").as_deref(), Some("print(1)\n"));
    assert_eq!(extract_code("```\na = 1\nb = 2\n```").as_deref(), Some("a = 1\nb = 2\n"));
    assert_eq!(extract_code("print('без блока')").as_deref(), Some("print('без блока')\n"));
    assert_eq!(extract_code("```python\n\n```"), None);
    assert_eq!(extract_code("   "), None);
}

#[test]
fn the_diff_counts_real_changes() {
    assert_eq!(diff_lines("a\nb\nc\n", "a\nb\nc\n"), Diff { added: 0, removed: 0 });
    assert_eq!(diff_lines("a\nb\nc\n", "a\nX\nc\nd\n"), Diff { added: 2, removed: 1 });
    assert_eq!(diff_lines("", "x\ny\n"), Diff { added: 2, removed: 0 });
    let old: String = (0..100).map(|i| format!("line {}\n", i)).collect();
    assert!(!is_large_change(&old, diff_lines(&old, &format!("{}new\n", old))), "an addition is small");
    assert!(is_large_change(&old, diff_lines(&old, "совсем другой код\n")), "everything replaced is large");
    assert!(!is_large_change("a\nb\n", diff_lines("a\nb\n", "c\n")), "a tiny file changes freely");
}

#[test]
fn secrets_are_recognized() {
    assert!(contains_secret("API_KEY = \"sk-abcdefghijklmnop12345\""));
    assert!(contains_secret("password: hunter22"));
    assert!(contains_secret("x = 'AIzaSyA1234567890abcdefghijk'"));
    assert!(!contains_secret("def add(a, b):\n    return a + b\n"));
    assert!(!contains_secret("# the token of the lexer is a word\nclass Token: pass\n"));
}

#[test]
fn syntax_output_is_parsed() {
    assert_eq!(parse_syntax_output("3|invalid syntax\n"), Some((3, "invalid syntax".into())));
    assert_eq!(parse_syntax_output("мусор"), None);
}

#[test]
fn the_syntax_check_finds_errors_and_never_runs_the_code() {
    let dir = tempfile::tempdir().unwrap();
    let marker = dir.path().join("ran.txt");
    let good = dir.path().join("good.py");
    // a file that would leave a trace if it were EXECUTED: it is only parsed
    std::fs::write(&good, format!("open(r'{}', 'w').write('x')\nprint(1)\n", marker.display())).unwrap();
    let bad = dir.path().join("bad.py");
    std::fs::write(&bad, "def f(:\n    pass\n").unwrap();
    crate::policy::set_test_allowed(vec![std::fs::canonicalize(dir.path()).unwrap()]);
    if find_python().is_none() {
        // reported, not hidden: this machine has no Python, the check cannot run here
        eprintln!("NOT RUN: no Python on this machine");
        assert_eq!(syntax_check(&good), Syntax::NoPython);
        crate::policy::set_test_allowed(Vec::new());
        return;
    }
    assert_eq!(syntax_check(&good), Syntax::Ok);
    assert!(!marker.exists(), "the checked code was run!");
    match syntax_check(&bad) {
        Syntax::Error { line, .. } => assert_eq!(line, 1),
        other => panic!("{:?}", other),
    }
    crate::policy::set_test_allowed(Vec::new());
}

#[test]
fn intents() {
    use Intent::*;
    // with the code file open
    assert!(matches!(parse("создай python файл", true, false), Some(CreateFile { name: None, ext: "py", special: None })));
    assert!(matches!(parse("создай питон файл", false, false), Some(CreateFile { .. })), "the word Python is enough");
    assert!(matches!(parse("Создай файл", true, false), Some(CreateFile { .. })));
    assert_eq!(parse("создай файл", false, false), None, "not about code: not ours");
    assert!(matches!(parse("создай python файл и назови calc.py", true, false), Some(CreateFile { name: Some(n), .. }) if n == "calc.py"));
    assert!(matches!(parse("создай проект калькулятор", true, false), Some(CreateProject { .. })));
    assert!(matches!(parse("создай файл requirements.txt", true, true), Some(CreateFile { special: Some("requirements"), .. })));
    assert!(matches!(parse("создай readme", true, true), Some(CreateFile { special: Some("readme"), .. })));
    assert!(matches!(parse("создай тесты", true, true), Some(CreateFile { special: Some("tests"), .. })));
    assert!(matches!(parse("напиши калькулятор на python", true, true), Some(Edit(_))));
    assert!(matches!(parse("добавь обработку деления на ноль", true, true), Some(Edit(_))));
    assert!(matches!(parse("исправь ошибку в функции", true, true), Some(Edit(_))));
    assert!(matches!(parse("объясни ошибку", true, true), Some(Explain(_))));
    assert!(matches!(parse("проверь синтаксис", true, true), Some(Check)));
    assert!(matches!(parse("верни как было", true, true), Some(Undo)));
    assert!(matches!(parse("запусти код", true, true), Some(Run)));
    assert!(matches!(parse("запусти файл", true, true), Some(Run)));
    assert!(matches!(parse("повтори генерацию", true, true), Some(Retry)));
    // without a code object «напиши …» is a question for the AI, as before
    assert_eq!(parse("напиши калькулятор на python", false, false), None);
    assert_eq!(parse("добавь обработку деления на ноль", true, false), None, "no file chosen: nothing to change");
    assert_eq!(parse("сколько времени", true, true), None);
}

// ---------- the dialog against a fake world ----------
struct World {
    _dir: tempfile::TempDir,
    root: PathBuf,
    notes: mpsc::Receiver<(String, String)>,
    launched: Arc<Mutex<Vec<PathBuf>>>,
    running: Arc<Mutex<bool>>,
}

struct FakeProbe(Arc<Mutex<bool>>);

impl Probe for FakeProbe {
    fn front(&self) -> Option<Front> {
        None
    }
    fn running(&self, process: &str) -> bool {
        process == "code.exe" && *self.0.lock()
    }
}

impl World {
    fn new() -> World {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("documents")).unwrap();
        crate::policy::set_test_allowed(vec![root.clone()]);
        *PLACES_ROOT.lock() = Some(root.clone());
        forget();
        objects::with(|e| e.reset());
        // VS Code is open, as after «Открой VS Code»
        objects::with(|e| e.open_app("vscode").unwrap());
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        set_notifier(Box::new(move |t, s| {
            let _ = tx.lock().send((t.to_string(), s.to_string()));
        }));
        let launched = Arc::new(Mutex::new(Vec::new()));
        let l2 = launched.clone();
        *LAUNCH.lock() = Some(Box::new(move |p| {
            l2.lock().push(p.to_path_buf());
            Ok(())
        }));
        let running = Arc::new(Mutex::new(true));
        *PROBE.lock() = Box::new(FakeProbe(running.clone()));
        *GENERATOR.lock() = None;
        World { _dir: dir, root, notes: rx, launched, running }
    }

    fn say(&self, text: &str) -> Option<Reply> {
        handle(&text.to_lowercase(), text, Source::Voice)
    }

    fn generator(&self, f: impl Fn(&str, &str, &AtomicBool) -> Result<String, String> + Send + Sync + 'static) {
        *GENERATOR.lock() = Some(Box::new(f));
    }

    fn next_note(&self) -> (String, String) {
        self.notes.recv_timeout(Duration::from_secs(5)).expect("the answer of the task did not arrive")
    }

    fn no_note(&self) {
        std::thread::sleep(Duration::from_millis(300));
        assert!(self.notes.try_recv().is_err(), "an answer that must not be said");
    }

    // «создай python файл» → «назови …» → «в документах»
    fn create(&self, name: &str) -> PathBuf {
        let r = self.say("создай python файл").unwrap();
        assert!(r.text.contains("Как назвать"), "{}", r.text);
        let r = self.say(&format!("назови {}", name)).unwrap();
        assert!(r.text.contains("В какой папке"), "{}", r.text);
        let r = self.say("в документах").unwrap();
        assert!(r.text.contains("создан"), "{}", r.text);
        self.root.join("documents").join(name)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        crate::policy::set_test_allowed(Vec::new());
        *PLACES_ROOT.lock() = None;
        *LAUNCH.lock() = None;
        *GENERATOR.lock() = None;
        *PROBE.lock() = Box::new(objects::SysProbe);
        forget();
        objects::with(|e| e.reset());
    }
}

use crate::session::TEST_LOCK as LOCK;

const CALC: &str = "def add(a, b):\n    return a + b\n\n\ndef div(a, b):\n    return a / b\n";
const CALC2: &str = "def add(a, b):\n    return a + b\n\n\ndef div(a, b):\n    if b == 0:\n        raise ValueError('деление на ноль')\n    return a / b\n";

#[test]
fn scenario_b_python_in_vscode() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("calculator.py");
    assert_eq!(std::fs::read(&path).unwrap(), b"", "an empty new file");
    assert_eq!(w.launched.lock().as_slice(), &[path.clone()], "the file is shown in VS Code");
    let snap = objects::snapshot();
    assert_eq!((snap["state"].as_str().unwrap(), snap["kind"].as_str().unwrap(), snap["name"].as_str().unwrap()), ("OBJECT_ACTIVE", "code_file", "calculator.py"));

    // «напиши калькулятор»: the code comes from the AI and is written by Jarvis, not typed
    w.generator(|_, user, _| {
        assert!(user.contains("калькулятор"));
        Ok(format!("Вот код:\n```python\n{}```", CALC))
    });
    let r = w.say("Напиши калькулятор на Python").unwrap();
    assert!(r.text.contains("Пишу код"), "{}", r.text);
    let (done, _) = w.next_note();
    assert!(done.contains("писано и проверено чтением") && done.contains("Код не запускался"), "{}", done);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CALC);
    assert_eq!(objects::snapshot()["state"], "OBJECT_ACTIVE");
    assert_eq!(objects::snapshot()["saved"], true, "the save is confirmed by reading the file back");

    // «добавь обработку деления на ноль»: the AI gets the current file and the change is applied to it
    let seen = Arc::new(Mutex::new(String::new()));
    let s2 = seen.clone();
    w.generator(move |_, user, _| {
        *s2.lock() = user.to_string();
        Ok(format!("```python\n{}```", CALC2))
    });
    w.say("Добавь обработку деления на ноль").unwrap();
    let (done, _) = w.next_note();
    assert!(seen.lock().contains("return a / b") && seen.lock().contains("деления на ноль"), "the AI saw the current file");
    assert!(done.contains("добавлено строк: 2"), "{}", done);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CALC2);

    // «верни как было»
    let r = w.say("верни как было").unwrap();
    assert!(r.text.contains("Вернул прошлую версию"), "{}", r.text);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CALC);
}

#[test]
fn stop_during_generation_cancels_it_and_a_late_answer_is_never_written() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("calc.py");
    std::fs::write(&path, CALC).unwrap();
    let (tx, rx) = mpsc::channel::<()>();
    let tx = Mutex::new(tx);
    let observed = Arc::new(AtomicBool::new(false));
    let o2 = observed.clone();
    w.generator(move |_, _, cancel| {
        // the generation runs until it is told to stop, and then still «answers» (a response that was already on the way)
        let t0 = Instant::now();
        while !cancel.load(Ordering::SeqCst) && t0.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(10));
        }
        o2.store(cancel.load(Ordering::SeqCst), Ordering::SeqCst);
        let _ = tx.lock().send(());
        Ok("```python\nprint('ПОЗДНИЙ ОТВЕТ')\n```".to_string())
    });
    w.say("перепиши калькулятор").unwrap();
    assert_eq!(objects::state(), State::AiTask);
    let r = w.say("Стоп").unwrap();
    assert!(r.text.contains("Генерация отменена") && r.text.contains("Продолжим?"), "{}", r.text);
    assert_eq!(objects::state(), State::Paused);
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(observed.load(Ordering::SeqCst), "the generation really saw the stop flag");
    w.no_note();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CALC, "the late answer was not written");
    assert_eq!(objects::snapshot()["state"], "PAUSED");
}

#[test]
fn continue_checks_vscode_and_the_file_again() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("calc.py");
    std::fs::write(&path, CALC).unwrap();
    // a pause with no generation
    let r = w.say("Пауза");
    assert!(r.is_none(), "nothing runs: «пауза» is not intercepted outside a task, the engine is not paused: {:?}", r);
    objects::with(|e| e.pause());
    assert_eq!(objects::state(), State::Paused);

    // VS Code was closed meanwhile
    *w.running.lock() = false;
    let r = w.say("Продолжим с объектом").unwrap();
    assert!(r.text.contains("VS Code закрыт") && r.text.contains("нельзя"), "{}", r.text);
    assert_eq!(objects::state(), State::ErrorRecovery);

    // opened again: the work goes on; the file was changed by someone: it is read again
    *w.running.lock() = true;
    std::fs::write(&path, format!("{}# правка в редакторе\n", CALC)).unwrap();
    let r = w.say("продолжай писать код").unwrap();
    assert!(r.text.contains("Продолжаем") && r.text.contains("изменился"), "{}", r.text);
    assert_eq!(objects::state(), State::ObjectActive);
}

#[test]
fn continue_with_a_deleted_file_is_reported() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("calc.py");
    objects::with(|e| e.pause());
    std::fs::remove_file(&path).unwrap();
    let r = w.say("Продолжим").unwrap();
    assert!(r.text.contains("не найден"), "{}", r.text);
    assert_eq!(objects::state(), State::ErrorRecovery);
}

#[test]
fn a_no_to_continue_closes_nothing() {
    let _g = LOCK.lock();
    let w = World::new();
    w.create("calc.py");
    objects::with(|e| e.stop());
    let r = w.say("нет").unwrap();
    assert!(r.text.contains("остался открытым"), "{}", r.text);
    assert_eq!(objects::state(), State::Paused);
    assert!(objects::with(|e| e.object().is_some()));
}

#[test]
fn an_existing_file_is_never_overwritten() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.root.join("documents").join("calculator.py");
    std::fs::write(&path, "# мой старый код\n").unwrap();
    w.say("создай python файл").unwrap();
    w.say("назови calculator.py").unwrap();
    let r = w.say("в документах").unwrap();
    assert!(r.text.contains("уже есть") && r.text.contains("не менял"), "{}", r.text);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# мой старый код\n");
}

#[test]
fn the_name_is_asked_when_missing_and_the_folder_when_there_is_no_project() {
    let _g = LOCK.lock();
    let w = World::new();
    let r = w.say("создай python файл").unwrap();
    assert!(r.text.contains("Как назвать файл"));
    // an unclear folder is asked again, nothing is created
    w.say("назови calc.py").unwrap();
    let r = w.say("куда-нибудь").unwrap();
    assert!(r.text.contains("В какой папке"), "{}", r.text);
    assert!(!w.root.join("documents").join("calc.py").exists());
    w.say("в документах папка проект").unwrap();
    assert!(w.root.join("documents").join("проект").join("calc.py").is_file());
    // the project is known now: the next file goes there without a question
    let r = w.say("создай python файл назови второй.py").unwrap();
    assert!(r.text.contains("создан"), "{}", r.text);
    assert!(w.root.join("documents").join("проект").join("vtoroy.py").is_file(), "{:?}", std::fs::read_dir(w.root.join("documents").join("проект")).unwrap().flatten().map(|e| e.file_name()).collect::<Vec<_>>());
}

#[test]
fn cancel_leaves_the_dialog_of_creation() {
    let _g = LOCK.lock();
    let w = World::new();
    w.say("создай python файл").unwrap();
    let r = w.say("отмена").unwrap();
    assert!(r.text.contains("не создаю"));
    assert!(w.say("calc.py").is_none(), "the question is closed: a name is not taken any more");
}

#[test]
fn a_path_out_of_the_allowed_places_is_refused() {
    let _g = LOCK.lock();
    let w = World::new();
    let docs = w.root.join("documents");
    // a file name that climbs out of the folder
    let r = create_file(&docs, "..\\..\\outside.py", "py");
    let r = r.unwrap();
    assert!(!w.root.parent().unwrap().join("outside.py").exists());
    let _ = r;
    // a folder that is not allowed at all
    let denied = tempfile::tempdir().unwrap();
    let r = create_file(denied.path(), "x.py", "py").unwrap();
    assert!(r.text.contains("Не удалось") || r.text.contains("Запрещено") || r.text.contains("не разрешена"), "{}", r.text);
    assert!(!denied.path().join("x.py").exists());
}

#[test]
fn generated_code_is_never_run() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("evil.py");
    let marker = w.root.join("pwned.txt");
    let code = format!("open(r'{}', 'w').write('x')\n", marker.display());
    w.generator(move |_, _, _| Ok(format!("```python\n{}```", code)));
    w.say("напиши код").unwrap();
    w.next_note();
    assert!(std::fs::read_to_string(&path).unwrap().contains("pwned.txt"));
    assert!(!marker.exists(), "the generated code was executed");
    // a request to run it is refused with a reason
    let r = w.say("запусти код").unwrap();
    assert!(r.text.contains("не запускается автоматически"), "{}", r.text);
    assert!(!marker.exists());
}

#[test]
fn a_file_changed_while_the_ai_wrote_is_not_overwritten() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("calc.py");
    std::fs::write(&path, CALC).unwrap();
    let p2 = path.clone();
    w.generator(move |_, _, _| {
        // the user edits the file in VS Code while the AI is writing
        std::fs::write(&p2, "# правка пользователя\n").unwrap();
        Ok(format!("```python\n{}```", CALC2))
    });
    w.say("добавь проверку деления на ноль").unwrap();
    let (note, _) = w.next_note();
    assert!(note.contains("изменился") && note.contains("Ничего не записал"), "{}", note);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# правка пользователя\n");
}

#[test]
fn a_big_change_waits_for_yes_and_a_no_writes_nothing() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("big.py");
    let old: String = (0..60).map(|i| format!("x{} = {}\n", i, i)).collect();
    std::fs::write(&path, &old).unwrap();
    w.generator(|_, _, _| Ok("```python\nprint('всё заменено')\n```".to_string()));
    w.say("перепиши всё").unwrap();
    let (q, _) = w.next_note();
    assert!(q.contains("Записать?"), "{}", q);
    assert_eq!(objects::state(), State::AwaitingConfirmation);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), old, "nothing is written before «да»");
    let r = w.say("нет").unwrap();
    assert!(r.text.contains("ничего не записал"), "{}", r.text);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), old);

    // the same again, now «да»
    w.say("перепиши всё").unwrap();
    w.next_note();
    let r = w.say("да").unwrap();
    assert!(r.text.contains("писано и проверено"), "{}", r.text);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "print('всё заменено')\n");
    // an old «да» does nothing now
    assert!(w.say("да").is_none());
}

#[test]
fn a_question_does_not_survive_a_stop() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("big.py");
    let old: String = (0..60).map(|i| format!("x{} = {}\n", i, i)).collect();
    std::fs::write(&path, &old).unwrap();
    w.generator(|_, _, _| Ok("```python\nprint('x')\n```".to_string()));
    w.say("перепиши всё").unwrap();
    w.next_note();
    let r = w.say("стоп").unwrap();
    assert!(r.text.contains("Вопрос отменён"), "{}", r.text);
    assert_eq!(objects::state(), State::Paused);
    w.say("продолжим").unwrap();
    assert!(w.say("да").is_none(), "a «да» after the pause has no question to answer");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), old);
}

#[test]
fn unavailable_ai_keeps_the_request_for_a_retry() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("calc.py");
    w.generator(|_, _, _| Err("ИИ недоступен: нет сети.".to_string()));
    w.say("напиши калькулятор").unwrap();
    let (note, _) = w.next_note();
    assert!(note.contains("недоступен") && note.contains("повтори"), "{}", note);
    assert_eq!(objects::state(), State::ErrorRecovery);
    assert_eq!(std::fs::read(&path).unwrap(), b"");
    // the AI is back: «повтори» sends the same request again
    w.generator(|_, user, _| {
        assert!(user.contains("калькулятор"));
        Ok(format!("```python\n{}```", CALC))
    });
    w.say("повтори").unwrap();
    w.next_note();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CALC);
    assert_eq!(objects::state(), State::ObjectActive);
}

#[test]
fn a_secret_in_the_file_or_in_the_request_is_not_sent_to_the_ai() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("cfg.py");
    let called = Arc::new(AtomicBool::new(false));
    let c2 = called.clone();
    w.generator(move |_, _, _| {
        c2.store(true, Ordering::SeqCst);
        Ok("```python\nx=1\n```".to_string())
    });
    std::fs::write(&path, "API_KEY = \"sk-abcdefghijklmnop12345\"\n").unwrap();
    let r = w.say("добавь функцию").unwrap();
    assert!(r.text.contains("ключ") && r.text.contains("не отправляю"), "{}", r.text);
    let r = w.say("добавь пароль hunter2 = \"supersecret\" в код").unwrap();
    assert!(r.text.contains("не отправляю"), "{}", r.text);
    assert!(!called.load(Ordering::SeqCst), "the secret reached the AI");
    assert_eq!(objects::state(), State::ObjectActive);
}

#[test]
fn a_phrase_from_the_phone_or_telegram_is_not_taken() {
    let _g = LOCK.lock();
    let w = World::new();
    w.create("calc.py");
    for door in [Source::Phone, Source::Telegram, Source::Ipc, Source::Script, Source::Internal] {
        assert!(handle("напиши калькулятор", "напиши калькулятор", door).is_none(), "{:?}", door);
        assert!(handle("создай python файл", "создай python файл", door).is_none(), "{:?}", door);
    }
    for door in [Source::Voice, Source::Window, Source::Console] {
        assert!(handle("запусти код", "запусти код", door).is_some(), "{:?}", door);
    }
}

#[test]
fn nothing_is_taken_without_a_code_object() {
    let _g = LOCK.lock();
    let w = World::new();
    objects::with(|e| e.reset());
    assert!(w.say("Напиши калькулятор на Python").is_none(), "an ordinary question stays with the AI chat");
    assert!(w.say("стоп").is_none(), "the global «стоп» stays with its command");
    assert!(w.say("продолжим с объектом").is_none());
}

#[test]
fn the_global_stop_word_stays_free_when_nothing_runs() {
    let _g = LOCK.lock();
    let w = World::new();
    w.create("calc.py");
    // an active object but no generation, no question: «стоп» is the ordinary command (it stops the voice)
    assert!(w.say("стоп").is_none());
    assert!(w.say("отмена").is_none());
    assert_eq!(objects::state(), State::ObjectActive);
}

#[test]
fn a_new_file_replaces_the_previous_object_but_not_during_a_task() {
    let _g = LOCK.lock();
    let w = World::new();
    w.create("a.py");
    objects::with(|e| e.pause());
    let r = w.say("создай python файл назови b.py").unwrap();
    assert!(r.text.contains("создан"), "{}", r.text);
    assert_eq!(objects::snapshot()["name"], "b.py");
    // during a task a new file is refused
    w.generator(|_, _, cancel| {
        while !cancel.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        Err("cancelled".to_string())
    });
    w.say("напиши что-нибудь").unwrap();
    let r = w.say("создай python файл назови c.py").unwrap();
    assert!(r.text.contains("Сначала нужно завершить"), "{}", r.text);
    assert!(!w.root.join("documents").join("c.py").exists());
    w.say("стоп").unwrap();
}

#[test]
fn explain_reads_but_never_writes() {
    let _g = LOCK.lock();
    let w = World::new();
    let path = w.create("calc.py");
    std::fs::write(&path, CALC).unwrap();
    w.generator(|sys, _, _| {
        assert!(!sys.contains("полным новым содержимым"));
        Ok("Функция div делит a на b.".to_string())
    });
    w.say("объясни код").unwrap();
    let (note, _) = w.next_note();
    assert!(note.contains("делит"), "{}", note);
    assert_eq!(std::fs::read_to_string(&path).unwrap(), CALC);
    assert_eq!(objects::state(), State::ObjectActive);
}

#[test]
fn the_check_of_syntax_is_reported_after_a_write() {
    let _g = LOCK.lock();
    let w = World::new();
    w.create("calc.py");
    w.generator(|_, _, _| Ok("```python\ndef f(:\n    pass\n```".to_string()));
    w.say("напиши функцию").unwrap();
    let (note, _) = w.next_note();
    if find_python().is_some() {
        assert!(note.contains("Ошибка синтаксиса в строке 1"), "{}", note);
    } else {
        assert!(note.contains("Python не найден"), "{}", note);
    }
}

#[test]
fn the_object_verification_helper_matches_the_vscode_file() {
    // the file-based object is verified by the program and the title, like any window of the engine
    let o = objects::Object { id: "o".into(), kind: Kind::CodeFile, app: "vscode".into(), name: "calculator.py".into(), path: None, project: None, detect: Detect::ProcessOnly };
    struct P(Option<Front>);
    impl Probe for P {
        fn front(&self) -> Option<Front> {
            self.0.clone()
        }
        fn running(&self, p: &str) -> bool {
            p == "code.exe"
        }
    }
    assert_eq!(objects::verify(&o, &P(Some(Front { process: "code.exe".into(), title: "calculator.py - proj - Visual Studio Code".into() }))), Verify::Ok(Detect::ProcessTitle));
    assert!(matches!(objects::verify(&o, &P(Some(Front { process: "code.exe".into(), title: "other.py - Visual Studio Code".into() }))), Verify::WrongWindow { .. }));
}
