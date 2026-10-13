use super::*;

const ALL_EVENTS: [Event; 18] = [
    Event::OpenApp, Event::BindObject, Event::StartDictation, Event::EndDictation, Event::StartAi, Event::AiDone, Event::AiCancelled, Event::AiFailed,
    Event::AskConfirm, Event::ConfirmYes, Event::ConfirmNo, Event::Pause, Event::Resume, Event::ObjectLost, Event::AppClosed, Event::RecoverOk, Event::Release, Event::Reset,
];

// the whole table, written out once more: any difference from `next` is a failing test
fn allowed() -> Vec<(State, Event, State)> {
    use Event::*;
    use State::*;
    vec![
        (Idle, OpenApp, AppOpen), (Idle, BindObject, ObjectActive),
        (AppOpen, OpenApp, AppOpen), (AppOpen, BindObject, ObjectActive), (AppOpen, AppClosed, Idle), (AppOpen, Release, Idle),
        (ObjectActive, BindObject, ObjectActive), (ObjectActive, StartDictation, Dictation), (ObjectActive, StartAi, AiTask), (ObjectActive, AskConfirm, AwaitingConfirmation),
        (ObjectActive, Pause, Paused), (ObjectActive, ObjectLost, ErrorRecovery), (ObjectActive, AppClosed, ErrorRecovery), (ObjectActive, Release, AppOpen),
        (Dictation, EndDictation, ObjectActive), (Dictation, StartAi, AiTask), (Dictation, AskConfirm, AwaitingConfirmation), (Dictation, Pause, Paused),
        (Dictation, ObjectLost, ErrorRecovery), (Dictation, AppClosed, ErrorRecovery), (Dictation, Release, AppOpen),
        (AiTask, AiDone, ObjectActive), (AiTask, AiCancelled, ObjectActive), (AiTask, AiFailed, ErrorRecovery), (AiTask, AskConfirm, AwaitingConfirmation),
        (AiTask, Pause, Paused), (AiTask, ObjectLost, ErrorRecovery), (AiTask, AppClosed, ErrorRecovery),
        (AwaitingConfirmation, ConfirmYes, ObjectActive), (AwaitingConfirmation, ConfirmNo, ObjectActive), (AwaitingConfirmation, Pause, Paused),
        (AwaitingConfirmation, ObjectLost, ErrorRecovery), (AwaitingConfirmation, AppClosed, ErrorRecovery),
        (Paused, Resume, ObjectActive), (Paused, ObjectLost, ErrorRecovery), (Paused, AppClosed, ErrorRecovery), (Paused, Release, AppOpen),
        (ErrorRecovery, RecoverOk, ObjectActive), (ErrorRecovery, OpenApp, AppOpen), (ErrorRecovery, BindObject, ObjectActive), (ErrorRecovery, Release, AppOpen),
    ]
}

#[test]
fn every_allowed_transition_goes_where_the_table_says() {
    for (from, ev, to) in allowed() {
        assert_eq!(next(from, ev), Ok(to), "{:?} + {:?}", from, ev);
    }
}

#[test]
fn every_other_transition_is_forbidden() {
    let table = allowed();
    let mut forbidden = 0;
    for from in State::ALL {
        for ev in ALL_EVENTS {
            if ev == Event::Reset {
                assert_eq!(next(from, ev), Ok(State::Idle));
                continue;
            }
            if table.iter().any(|(f, e, _)| *f == from && *e == ev) {
                continue;
            }
            assert_eq!(next(from, ev), Err(Forbidden { from, ev }), "{:?} + {:?} must be forbidden", from, ev);
            forbidden += 1;
        }
    }
    assert!(forbidden > 80, "the table is meant to be strict, got {}", forbidden);
}

#[test]
fn incompatible_operations_do_not_mix() {
    use Event::*;
    use State::*;
    // no dictation during a generation or a question, no second generation, no confirm without a question
    assert!(next(AiTask, StartDictation).is_err());
    assert!(next(AiTask, StartAi).is_err());
    assert!(next(AwaitingConfirmation, StartDictation).is_err());
    assert!(next(AwaitingConfirmation, StartAi).is_err());
    assert!(next(Dictation, StartDictation).is_err());
    assert!(next(ObjectActive, ConfirmYes).is_err());
    assert!(next(Paused, StartDictation).is_err());
    assert!(next(Paused, StartAi).is_err());
    assert!(next(Paused, ConfirmYes).is_err());
    assert!(next(Idle, StartDictation).is_err());
}

#[test]
fn no_stuck_states() {
    // from every state, without the Reset event, the work can get back to a state with no running operation
    for start in State::ALL {
        let mut seen = vec![start];
        let mut stack = vec![start];
        while let Some(s) = stack.pop() {
            for ev in ALL_EVENTS {
                if ev == Event::Reset {
                    continue;
                }
                if let Ok(n) = next(s, ev) {
                    if !seen.contains(&n) {
                        seen.push(n);
                        stack.push(n);
                    }
                }
            }
        }
        assert!(seen.contains(&State::ObjectActive) || start == State::Idle || start == State::AppOpen, "{:?} cannot reach OBJECT_ACTIVE", start);
        assert!(seen.contains(&State::Idle), "{:?} cannot reach IDLE without Reset", start);
    }
}

fn bound() -> Engine {
    let mut e = Engine::new();
    e.open_app("word").unwrap();
    e.bind("word", Kind::WordDoc, "Отчёт.docx", Some("C:\\Users\\u\\Documents\\Отчёт.docx"), None, Detect::ComDocument).unwrap();
    e
}

#[test]
fn context_keeps_the_object_facts() {
    let e = bound();
    let o = e.object().unwrap();
    assert_eq!(o.kind, Kind::WordDoc);
    assert_eq!(o.app, "word");
    assert_eq!(o.detect, Detect::ComDocument);
    assert!(o.path.as_deref().unwrap().ends_with("Отчёт.docx"));
    let s = e.snapshot();
    assert_eq!(s["state"], "OBJECT_ACTIVE");
    assert_eq!(s["saved"], false);
    assert_eq!(s["kind"], "word_doc");
}

#[test]
fn dictation_stop_keeps_the_object_and_asks_to_continue() {
    let mut e = bound();
    e.start_dictation().unwrap();
    assert_eq!(e.state(), State::Dictation);
    let rep = e.stop();
    assert!(rep.paused && rep.dictation_stopped && !rep.ai_cancelled);
    assert_eq!(e.state(), State::Paused);
    assert_eq!(e.object().unwrap().name, "Отчёт.docx", "the object stays");
    assert!(e.context().resume_prompted);
    let say = stop_reply(&rep, true);
    assert!(say.contains("Продолжим?") && say.contains("Отчёт.docx"), "{}", say);
    e.resume().unwrap();
    assert_eq!(e.state(), State::ObjectActive);
    assert!(!e.context().resume_prompted);
}

#[test]
fn pause_does_not_ask_and_decline_keeps_the_context() {
    let mut e = bound();
    e.start_dictation().unwrap();
    let rep = e.pause();
    assert!(!e.context().resume_prompted);
    assert!(!stop_reply(&rep, false).contains("Продолжим?"));
    e.decline_resume();
    assert_eq!(e.state(), State::Paused);
    assert!(e.object().is_some(), "a «нет» closes and deletes nothing");
}

#[test]
fn stop_cancels_the_generation_and_its_late_result_is_dropped() {
    let mut e = bound();
    let (task, flag) = e.start_ai("добавь обработку деления на ноль").unwrap();
    assert_eq!(e.state(), State::AiTask);
    assert!(!flag.load(Ordering::SeqCst));
    let rep = e.stop();
    assert!(rep.ai_cancelled && rep.paused);
    assert!(flag.load(Ordering::SeqCst), "the generation sees the cancel flag");
    assert_eq!(e.context().ai, AiStatus::Cancelled);
    // the answer arrives anyway: it is not accepted and the state does not move
    assert!(!e.ai_finished(task, Ok("def f(): pass".into())));
    assert_eq!(e.state(), State::Paused);
    assert!(e.context().results.is_empty());
}

#[test]
fn a_result_of_an_old_task_is_not_accepted_by_a_new_one() {
    let mut e = bound();
    let (old, _) = e.start_ai("первое").unwrap();
    e.cancel().unwrap();
    let (new, _) = e.start_ai("второе").unwrap();
    assert!(!e.ai_finished(old, Ok("старый ответ".into())));
    assert_eq!(e.state(), State::AiTask, "the new task is untouched");
    assert!(e.ai_finished(new, Ok("готово".into())));
    assert_eq!(e.state(), State::ObjectActive);
    assert_eq!(e.context().ai, AiStatus::Done);
}

#[test]
fn an_ai_failure_goes_to_error_recovery_and_back() {
    let mut e = bound();
    let (t, _) = e.start_ai("код").unwrap();
    assert!(!e.ai_finished(t, Err("ИИ недоступен".into())));
    assert_eq!(e.state(), State::ErrorRecovery);
    assert_eq!(e.context().ai, AiStatus::Failed);
    assert!(e.context().error.contains("недоступен"));
    e.recovered().unwrap();
    assert_eq!(e.state(), State::ObjectActive);
}

#[test]
fn only_one_generation_and_no_dictation_during_it() {
    let mut e = bound();
    e.start_ai("a").unwrap();
    assert!(matches!(e.start_ai("b"), Err(Error::Busy(_))));
    assert!(e.start_dictation().is_err());
    assert_eq!(e.state(), State::AiTask);
}

#[test]
fn starting_ai_during_dictation_ends_the_dictation() {
    let mut e = bound();
    e.start_dictation().unwrap();
    e.start_ai("перепиши официально").unwrap();
    assert_eq!(e.state(), State::AiTask);
    assert!(!e.context().operation.contains("dictation"));
}

#[test]
fn a_question_runs_the_operation_once_and_never_survives_a_pause() {
    let mut e = bound();
    e.ask("overwrite", "Файл уже есть. Заменить?").unwrap();
    assert_eq!(e.state(), State::AwaitingConfirmation);
    assert!(matches!(e.ask("delete", "Удалить?"), Err(Error::Busy(_))), "one question at a time");
    assert_eq!(e.answer(true).unwrap().as_deref(), Some("overwrite"));
    assert_eq!(e.state(), State::ObjectActive);
    // a second «да» has nothing to confirm
    assert!(e.answer(true).is_err());

    e.ask("delete", "Удалить?").unwrap();
    let rep = e.stop();
    assert!(rep.question_dropped);
    e.resume().unwrap();
    assert!(e.answer(true).is_err(), "the old «да» must not run a stale action after the pause");
    assert!(e.context().pending.is_none());
}

#[test]
fn no_means_nothing_runs() {
    let mut e = bound();
    e.ask("replace_text", "Заменить весь текст?").unwrap();
    assert_eq!(e.answer(false).unwrap(), None);
    assert_eq!(e.state(), State::ObjectActive);
}

#[test]
fn cancel_ends_only_the_operation() {
    let mut e = bound();
    e.start_dictation().unwrap();
    assert!(e.cancel().is_some());
    assert_eq!(e.state(), State::ObjectActive, "the object stays active, not paused");
    assert!(e.cancel().is_none(), "nothing to cancel");
}

#[test]
fn closing_the_program_during_work_goes_to_recovery() {
    let mut e = bound();
    e.start_dictation().unwrap();
    e.app_closed().unwrap();
    assert_eq!(e.state(), State::ErrorRecovery);
    assert!(e.context().error.contains("закрыта"));
    assert!(e.object().is_some(), "the context stays to offer the restore");
    e.open_app("word").unwrap();
    e.bind("word", Kind::WordDoc, "Отчёт.docx", None, None, Detect::ProcessTitle).unwrap();
    assert_eq!(e.state(), State::ObjectActive);
}

#[test]
fn a_lost_object_is_reported_and_cancels_the_generation() {
    let mut e = bound();
    let (task, flag) = e.start_ai("текст").unwrap();
    e.lost("документ закрыт").unwrap();
    assert_eq!(e.state(), State::ErrorRecovery);
    assert!(flag.load(Ordering::SeqCst));
    assert!(!e.ai_finished(task, Ok("поздно".into())));
}

#[test]
fn another_object_cannot_replace_the_object_during_a_task() {
    let mut e = bound();
    e.start_ai("текст").unwrap();
    assert!(matches!(e.bind("word", Kind::WordDoc, "Другой.docx", None, None, Detect::ProcessTitle), Err(Error::Busy(_))));
    assert_eq!(e.object().unwrap().name, "Отчёт.docx");
}

#[test]
fn a_save_is_a_fact_only_until_the_next_change() {
    let mut e = bound();
    assert!(!e.context().saved_confirmed);
    e.saved("C:\\x\\Отчёт.docx");
    assert!(e.context().saved_confirmed);
    assert_eq!(e.snapshot()["saved_path"], "C:\\x\\Отчёт.docx");
    e.changed();
    assert!(!e.context().saved_confirmed);
}

#[test]
fn history_is_short_and_without_secrets() {
    let mut e = bound();
    for i in 0..30 {
        e.note("user", &format!("фраза {}", i));
    }
    assert_eq!(e.context().recent.len(), MAX_RECENT);
    e.note("user", "мой пароль hunter2 и ключ AIzaSyA1234567890abcdefghijk");
    let last = &e.context().recent.back().unwrap().text;
    assert!(!last.contains("hunter2") && !last.contains("AIza"), "{}", last);
    let long = "слово ".repeat(500);
    e.note("jarvis", &long);
    assert!(e.context().recent.back().unwrap().text.chars().count() <= MAX_TEXT + 1);
    for i in 0..20 {
        e.saved(&format!("p{}", i));
    }
    assert_eq!(e.context().results.len(), MAX_RESULTS);
}

#[test]
fn sanitize_hides_tokens_and_keys() {
    for secret in ["sk-abcdefghijklmnop12345", "123456789:AAH_abcdefghijklmnopqrstuvwxyz1234", "ghp_abcdefghijklmnopqrstuv"] {
        let out = sanitize(&format!("используй {} сейчас", secret));
        assert!(!out.contains(secret), "{}", out);
    }
    assert_eq!(sanitize("пароль 12345 дальше"), "пароль [скрыто] дальше");
    assert_eq!(sanitize("обычный текст про калькулятор"), "обычный текст про калькулятор");
}

#[test]
fn reset_forgets_everything_and_cancels_the_task() {
    let mut e = bound();
    let (_, flag) = e.start_ai("x").unwrap();
    e.reset();
    assert_eq!(e.state(), State::Idle);
    assert!(e.object().is_none());
    assert!(flag.load(Ordering::SeqCst));
}

#[test]
fn bind_needs_a_free_engine_and_dictation_needs_an_object() {
    let mut e = Engine::new();
    assert_eq!(e.start_dictation(), Err(Error::NoObject));
    assert_eq!(e.start_ai("x").map(|_| ()), Err(Error::NoObject));
    assert_eq!(e.ask("x", "?"), Err(Error::NoObject));
    assert_eq!(e.resume(), Err(Error::NoObject));
}

#[test]
fn the_object_of_another_program_is_not_kept() {
    let mut e = bound();
    e.release().unwrap();
    assert_eq!(e.state(), State::AppOpen);
    e.open_app("excel").unwrap();
    assert!(e.object().is_none());
}

#[test]
fn finish_works_from_every_state_and_leaves_nothing_running() {
    let setups: [fn(&mut Engine); 6] = [
        |_| {},
        |e| e.start_dictation().unwrap(),
        |e| { e.start_ai("x").unwrap(); },
        |e| e.ask("delete", "Удалить?").unwrap(),
        |e| { e.stop(); },
        |e| e.lost("пропал").unwrap(),
    ];
    for setup in setups {
        let mut e = bound();
        setup(&mut e);
        e.finish().unwrap();
        assert_eq!(e.state(), State::AppOpen);
        assert!(e.context().pending.is_none() && e.context().operation.is_empty());
    }
}

// ---- the window in front ----
struct Fake {
    front: Option<Front>,
    running: Vec<&'static str>,
}

impl Probe for Fake {
    fn front(&self) -> Option<Front> {
        self.front.clone()
    }
    fn running(&self, process: &str) -> bool {
        self.running.contains(&process)
    }
}

fn doc() -> Object {
    bound().object().unwrap().clone()
}

#[test]
fn the_right_window_is_accepted_with_the_method_named() {
    let p = Fake { front: Some(Front { process: "winword.exe".into(), title: "отчёт - word".into() }), running: vec!["winword.exe"] };
    assert_eq!(verify(&doc(), &p), Verify::Ok(Detect::ProcessTitle));
}

#[test]
fn another_window_in_front_is_never_a_target() {
    let p = Fake { front: Some(Front { process: "chrome.exe".into(), title: "YouTube".into() }), running: vec!["winword.exe", "chrome.exe"] };
    assert_eq!(verify(&doc(), &p), Verify::WrongWindow { found: "chrome.exe".into() });
    // the right program but another document
    let p = Fake { front: Some(Front { process: "winword.exe".into(), title: "Письмо - Word".into() }), running: vec!["winword.exe"] };
    assert!(matches!(verify(&doc(), &p), Verify::WrongWindow { .. }));
    // nothing normal in front
    let p = Fake { front: None, running: vec!["winword.exe"] };
    assert!(matches!(verify(&doc(), &p), Verify::WrongWindow { .. }));
}

#[test]
fn a_closed_program_is_a_lost_object() {
    let p = Fake { front: Some(Front { process: "chrome.exe".into(), title: "x".into() }), running: vec!["chrome.exe"] };
    assert_eq!(verify(&doc(), &p), Verify::Lost);
    let mut o = doc();
    o.app = "unknown".into();
    assert_eq!(verify(&o, &p), Verify::Lost, "an object of an unknown program is never guessed");
}

#[test]
fn an_unnamed_notepad_is_checked_by_the_program_only() {
    let mut o = doc();
    o.app = "notepad".into();
    o.name = String::new();
    let p = Fake { front: Some(Front { process: "notepad.exe".into(), title: "Безымянный — Блокнот".into() }), running: vec!["notepad.exe"] };
    assert_eq!(verify(&o, &p), Verify::Ok(Detect::ProcessOnly));
}

// ---- the voice words ----
#[test]
fn stop_words_in_every_state() {
    for (text, want) in [("Стоп", Control::Stop), ("остановись", Control::Stop), ("Хватит", Control::Stop), ("Прекрати ввод", Control::Stop), ("Пока остановись", Control::Stop),
        ("Не закрывай, просто остановись", Control::Stop), ("Джарвис, стоп", Control::Stop), ("пауза", Control::Pause), ("Поставь на паузу", Control::Pause), ("отмена", Control::Cancel)] {
        assert_eq!(classify(text, State::Dictation), Some(want), "{}", text);
        assert_eq!(classify(text, State::AiTask), Some(want), "{}", text);
    }
}

#[test]
fn the_word_stop_inside_dictated_text_is_text() {
    for text in [
        "он сказал стоп и ушёл",
        "на светофоре стоп сигнал",
        "хватит ли нам времени",
        "остановись на этом пункте и перейди к следующему",
        "пауза между словами",
        "отмена заказа вступает в силу завтра",
        "да нет возможно",
        "продолжим с объектом",
    ] {
        assert_eq!(classify(text, State::Dictation), None, "{}", text);
    }
}

#[test]
fn resume_phrases_apply_to_a_paused_work() {
    for text in ["Продолжим с объектом", "Продолжай с этим файлом", "Вернись к документу", "Продолжай писать код", "Возобнови работу", "продолжим", "Продолжай"] {
        assert_eq!(classify(text, State::Paused), Some(Control::Resume), "{}", text);
    }
    // another command with the same verb is not ours
    for text in ["продолжай воспроизведение", "продолжи загрузку файла из интернета", "вернись на рабочий стол"] {
        assert_eq!(classify(text, State::Paused), None, "{}", text);
    }
    assert_eq!(classify("да", State::Paused), Some(Control::Yes));
    assert_eq!(classify("нет", State::Paused), Some(Control::No));
    assert_eq!(classify("Продолжим с объектом", State::Idle), None, "nothing to continue when there is no work");
}

#[test]
fn answers_apply_only_to_a_question() {
    assert_eq!(classify("да", State::AwaitingConfirmation), Some(Control::Yes));
    assert_eq!(classify("не надо", State::AwaitingConfirmation), Some(Control::No));
    assert_eq!(classify("отмена", State::AwaitingConfirmation), Some(Control::Cancel));
    assert_eq!(classify("да", State::ObjectActive), None, "a lone «да» with no question is not ours");
    assert_eq!(classify("да", State::Dictation), None);
    assert_eq!(classify("да", State::AiTask), None);
}

#[test]
fn idle_never_takes_a_word() {
    for t in ["стоп", "пауза", "отмена", "продолжим", "да"] {
        assert_eq!(classify(t, State::Idle), None);
    }
}

#[test]
fn the_undo_of_the_last_action_stays_a_document_command() {
    // «отмени последнее действие» is the undo of Word (office.lua), not the cancel of the operation
    assert_eq!(classify("отмени последнее действие", State::Dictation), None);
    assert_eq!(classify("отмени последнее действие", State::ObjectActive), None);
}

#[test]
fn honest_replies() {
    let none = StopReport { was: State::Idle, dictation_stopped: false, ai_cancelled: false, question_dropped: false, paused: false, object_name: String::new() };
    assert_eq!(stop_reply(&none, true), "Сейчас нечего останавливать.");
    let already = StopReport { was: State::Paused, ..none.clone() };
    assert_eq!(stop_reply(&already, true), "Работа уже приостановлена.");
    let mut e = bound();
    let _ = e.start_ai("x").unwrap();
    let rep = e.stop();
    let say = stop_reply(&rep, true);
    assert!(say.contains("Генерация отменена") && say.contains("не будет вставлен"), "{}", say);
}

#[test]
fn the_engine_is_shared_and_resettable() {
    let _g = crate::session::TEST_LOCK.lock();
    with(|e| e.reset());
    assert_eq!(state(), State::Idle);
    with(|e| {
        e.open_app("notepad").unwrap();
        e.bind("notepad", Kind::TextFile, "", None, None, Detect::ProcessOnly).unwrap();
    });
    assert_eq!(snapshot()["state"], "OBJECT_ACTIVE");
    assert_eq!(control_for("стоп"), Some(Control::Stop));
    with(|e| e.reset());
}
