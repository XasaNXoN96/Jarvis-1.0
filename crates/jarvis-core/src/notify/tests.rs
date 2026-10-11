use super::*;
use crate::uistate::{self, UiSettings};

fn prefs() -> Prefs {
    Prefs { show: true, speak: true, sound: true, dnd: false, tts_on: true }
}

#[test]
fn a_notification_is_spoken_once_and_never_after_the_voice_already_said_it() {
    let p = plan(&prefs(), false, false);
    assert_eq!(p, Plan { show: true, speak: true, sound: false }, "spoken: the voice is the alert, no extra beep");
    let p = plan(&prefs(), false, true);
    assert!(p.show && !p.speak, "the reply was already said by the voice: the notification stays silent");
    let p = plan(&Prefs { tts_on: false, ..prefs() }, false, false);
    assert!(!p.speak && p.sound, "speech switched off: never spoken, a short sound instead");
    let p = plan(&Prefs { speak: false, sound: false, ..prefs() }, false, false);
    assert!(p.show && !p.speak && !p.sound);
    let p = plan(&Prefs { show: false, ..prefs() }, false, true);
    assert!(!p.show, "popups switched off");
}

#[test]
fn do_not_disturb_silences_everything_but_keeps_critical_errors_visible() {
    let dnd = Prefs { dnd: true, ..prefs() };
    let p = plan(&dnd, false, false);
    assert_eq!(p, Plan { show: false, speak: false, sound: false });
    let p = plan(&dnd, true, false);
    assert_eq!(p, Plan { show: true, speak: false, sound: false }, "a critical error is shown, silently");
    assert!(is_critical("system_error") && is_critical("important") && !is_critical("timer"));
}

#[test]
fn the_kind_of_a_timer_comes_from_its_title() {
    assert_eq!(timer_kind("Будильник"), "alarm");
    assert_eq!(timer_kind("Напоминание"), "reminder");
    assert_eq!(timer_kind("Таймер"), "timer");
    assert_eq!(timer_kind("Reminder"), "reminder");
}

#[test]
fn the_same_notification_twice_is_shown_once() {
    let _g = uistate::TEST_LOCK.lock();
    uistate::set_for_tests(UiSettings::default());
    let text = format!("Время вышло {}", rand::random::<u32>());
    assert!(push("timer", "Таймер", &text, false).is_some());
    assert!(push("timer", "Таймер", &text, false).is_none(), "a repeat within a few seconds is dropped");
    assert!(push("timer", "Таймер", &format!("{} (2)", text), false).is_some());
    let n = recent(5).into_iter().find(|n| n.text == text).expect("kept in the list of the window");
    assert_eq!(n.kind, "timer");
    assert!(n.spoken && n.shown);
}

#[test]
fn do_not_disturb_and_settings_are_applied_to_real_notifications() {
    let _g = uistate::TEST_LOCK.lock();
    uistate::set_for_tests(UiSettings { dnd: true, ..UiSettings::default() });
    let t = format!("тихо {}", rand::random::<u32>());
    push("reminder", "Напоминание", &t, false);
    let n = recent(5).into_iter().find(|n| n.text == t).unwrap();
    assert!(!n.shown && !n.spoken, "do not disturb: kept in the list, but not shown or spoken");
    let e = format!("ошибка {}", rand::random::<u32>());
    push("system_error", "Ошибка", &e, true);
    let n = recent(5).into_iter().find(|n| n.text == e).unwrap();
    assert!(n.shown && !n.spoken && n.critical);
    uistate::set_for_tests(UiSettings::default());
}

fn ext_settings() -> UiSettings {
    UiSettings { ext_enabled: true, ext_strict: true, ext_allowed: vec!["Telegram".into()], ext_blocked: vec!["Spam App".into()], ..UiSettings::default() }
}

#[test]
fn external_notifications_follow_the_allowed_and_excluded_lists() {
    let s = ext_settings();
    assert!(ext_allowed(&s, "Telegram"));
    assert!(ext_allowed(&s, "telegram"), "names are compared without case");
    assert!(!ext_allowed(&s, "Outlook"), "strict list: a program that is not on it is not let through");
    assert!(!ext_allowed(&s, ""), "an unknown program is never allowed automatically");
    let open = UiSettings { ext_strict: false, ..ext_settings() };
    assert!(ext_allowed(&open, "Outlook"));
    assert!(!ext_allowed(&open, "Spam App"), "an excluded program stays excluded");
    let both = UiSettings { ext_allowed: vec!["Spam App".into()], ..ext_settings() };
    assert!(!ext_allowed(&both, "Spam App"), "exclusion wins over the allowed list");
    let off = UiSettings { ext_enabled: false, ..ext_settings() };
    assert!(!ext_allowed(&off, "Telegram"), "switched off (the default): nothing is read");
    assert!(!UiSettings::default().ext_enabled, "off by default");
}

#[test]
fn an_external_notification_without_text_still_says_where_it_comes_from() {
    assert_eq!(ext_texts("Telegram", &[]), ("Telegram".to_string(), "Новое уведомление (без текста)".to_string()));
    assert_eq!(ext_texts("", &["  ".into()]).0, "Неизвестная программа");
    assert_eq!(ext_texts("Mail", &["Иван".into(), "Привет,\nкак дела?".into()]), ("Mail · Иван".to_string(), "Привет, как дела?".to_string()));
}

#[test]
fn external_notifications_are_shown_once_only_when_allowed_and_never_logged() {
    let _g = uistate::TEST_LOCK.lock();
    uistate::set_for_tests(ext_settings());
    let id = format!("n{}", rand::random::<u32>());
    let secret = format!("код подтверждения {}", rand::random::<u32>());
    // already in the notification center when the watcher started: not shown again
    assert!(on_external(&id, "Telegram", &[secret.clone()], true).is_none());
    assert!(on_external(&id, "Telegram", &[secret.clone()], false).is_none(), "the same id is never shown twice");
    let id2 = format!("{}b", id);
    assert!(on_external(&id2, "Telegram", &["Анна".into(), secret.clone()], false).is_some());
    assert!(on_external(&format!("{}c", id), "Outlook", &["письмо".into()], false).is_none(), "not on the allowed list");
    let n = recent(10).into_iter().find(|n| n.kind == "external" && n.text.contains(&secret)).unwrap();
    assert_eq!(n.source, "Telegram");
    assert!(!n.spoken, "external notifications are not read aloud unless that is switched on");
    // the text of another program never reaches the event log of the program (diagnostics) or the settings
    assert!(uistate::recent_events(80).iter().all(|e| !e.text.contains(&secret)));
    assert!(uistate::settings().ext_seen.iter().any(|a| a == "Telegram"), "only the program name is remembered");
    assert!(!serde_json::to_string(&uistate::settings_json()).unwrap().contains(&secret));
    uistate::set_for_tests(UiSettings::default());
}

#[test]
fn the_answer_of_windows_is_shown_as_it_is() {
    let _g = uistate::TEST_LOCK.lock();
    handle_line_for_tests(r#"{"type":"access","status":"Denied"}"#);
    let st = ext_status_json();
    assert_eq!(st["state"], "denied");
    assert!(st["detail"].as_str().unwrap().contains("Параметры Windows"), "the user is told where to allow it");
    handle_line_for_tests(r#"{"type":"error","message":"Element not found"}"#);
    assert_eq!(ext_status_json()["state"], "error");
    handle_line_for_tests("not json at all");
    handle_line_for_tests(r#"{"type":"access","status":"Allowed"}"#);
    assert_eq!(ext_status_json()["state"], "on");
    // a single text arrives from PowerShell as a string, not as a list
    uistate::set_for_tests(ext_settings());
    let t = format!("одна строка {}", rand::random::<u32>());
    handle_line_for_tests(&format!(r#"{{"type":"note","id":"x{}","app":"Telegram","texts":"{}","initial":false}}"#, rand::random::<u32>(), t));
    assert!(recent(5).iter().any(|n| n.text == t));
    uistate::set_for_tests(UiSettings::default());
    ext_stop();
    assert_eq!(ext_status_json()["state"], "off");
}

#[test]
fn a_refusal_of_windows_never_looks_like_working() {
    let _g = uistate::TEST_LOCK.lock();
    uistate::set_for_tests(ext_settings());
    for answer in ["Denied", "Unspecified", ""] {
        ext_stop();
        handle_line_for_tests(&format!(r#"{{"type":"access","status":"{}"}}"#, answer));
        let st = ext_status_json();
        assert_eq!(st["state"], "denied", "answer {:?}", answer);
        let detail = st["detail"].as_str().unwrap();
        assert!(detail.contains("не дала доступ") && detail.contains("Параметры Windows"), "{}", detail);
        // a notification line after a refusal is ignored: nothing is shown, nothing is read
        let t = format!("после отказа {}", rand::random::<u32>());
        handle_line_for_tests(&format!(r#"{{"type":"note","id":"d{}","app":"Telegram","texts":["{}"],"initial":false}}"#, rand::random::<u32>(), t));
        assert!(recent(10).iter().all(|n| n.text != t));
        // the process then exits: the refusal stays, it does not turn into "working" or a vague error
        ext_on_exit_for_tests();
        assert_eq!(ext_status_json()["state"], "denied");
    }
    // the process dies while starting or after the access: shown as an error, never as working
    ext_stop();
    handle_line_for_tests(r#"{"type":"access","status":"Allowed"}"#);
    assert_eq!(ext_status_json()["state"], "on");
    ext_on_exit_for_tests();
    assert_eq!(ext_status_json()["state"], "error");
    ext_stop();
    *EXT_STATE.lock() = ExtState { state: "starting".into(), detail: String::new() };
    ext_on_exit_for_tests();
    assert_eq!(ext_status_json()["state"], "error");
    // the diagnostics report names the state, not "working"
    assert!(crate::diag::report().contains("внешние вкл (error)"));
    ext_stop();
    uistate::set_for_tests(UiSettings::default());
}
