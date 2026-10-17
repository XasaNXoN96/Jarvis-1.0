use super::*;
use crate::uistate::{self, UiSettings};

fn mic(voice_active: bool, frame_age_ms: Option<u64>) -> MicFacts {
    MicFacts { voice_active, text_only: false, muted: false, frame_age_ms, error: String::new(), devices: 1 }
}

#[test]
fn the_microphone_is_working_only_when_real_audio_arrives() {
    assert_eq!(mic_check(&mic(true, Some(200))).status, Status::Ok);
    assert_eq!(mic_check(&mic(true, None)).status, Status::Error, "an open stream without audio is not 'working'");
    assert_eq!(mic_check(&mic(true, Some(9000))).status, Status::Error);
    assert_eq!(mic_check(&MicFacts { error: "микрофон не найден".into(), ..mic(false, None) }).detail.contains("микрофон не найден"), true);
    assert_eq!(mic_check(&MicFacts { muted: true, ..mic(true, Some(10)) }).status, Status::Warn);
    assert_eq!(mic_check(&MicFacts { text_only: true, ..mic(false, None) }).status, Status::Warn);
    assert!(!mic_check(&mic(true, None)).fix.is_empty(), "a problem comes with what to do");
}

#[test]
fn piper_is_working_only_after_a_real_synthesis() {
    assert_eq!(piper_check(true, false, PiperFacts::NotInstalled, None).status, Status::Error);
    assert_eq!(piper_check(true, true, PiperFacts::NotInstalled, None).status, Status::Warn);
    assert_eq!(piper_check(true, false, PiperFacts::Ok(30_000), None).status, Status::Unchecked, "a made sound file is not proof that it was heard");
    assert_eq!(piper_check_play(true, false, PiperFacts::Ok(30_000), None, Some((now(), Ok(())))).status, Status::Ok);
    let not_heard = piper_check_play(true, false, PiperFacts::Ok(30_000), None, Some((now(), Err("нет устройства".into()))));
    assert_eq!(not_heard.status, Status::Error);
    assert!(not_heard.detail.contains("не воспроизводится"));
    assert_eq!(piper_check(false, false, PiperFacts::Ok(30_000), None).status, Status::Warn, "speech switched off by the user");
    let failed = piper_check(true, false, PiperFacts::Failed("onnx error".into()), Some((1, Ok(()))));
    assert_eq!(failed.status, Status::Error);
    assert!(failed.detail.contains("onnx error"));
    assert_eq!(piper_check(true, false, PiperFacts::Interrupted, None).status, Status::Unchecked);
}

#[test]
fn a_saved_key_does_not_prove_that_the_ai_is_connected() {
    let unverified = json!({ "configured": true, "state": "DISCONNECTED", "provider": "Google Gemini", "model": "gemini-flash-latest", "detail": "ключ задан" });
    assert_eq!(ai_check(true, &unverified).status, Status::Unchecked);
    let ok = json!({ "configured": true, "state": "CONNECTED", "provider": "Google Gemini", "model": "m", "last_ok": now() });
    assert_eq!(ai_check(true, &ok).status, Status::Ok);
    let err = json!({ "configured": true, "state": "ERROR", "provider": "OpenAI", "model": "m", "detail": "Провайдер не принял ключ" });
    assert_eq!(ai_check(true, &err).status, Status::Error);
    assert_eq!(ai_check(true, &json!({ "configured": false, "detail": "не выбран" })).status, Status::Warn);
    assert_eq!(ai_check(false, &ok).status, Status::Warn, "switched off in the privacy section");
    assert_eq!(latency_check(None).status, Status::Unchecked);
    assert_eq!(latency_check(Some(&json!({ "total_ms": 900, "first_chunk_ms": 300, "streamed": true }))).status, Status::Ok);
}

#[test]
fn a_local_server_does_not_prove_that_a_phone_can_connect() {
    assert_eq!(remote_check(false, None, None).status, Status::Unchecked);
    assert_eq!(remote_check(true, Some(Ok(())), None).status, Status::Unchecked, "nobody has connected yet");
    assert_eq!(remote_check(true, Some(Ok(())), Some(now() - 30)).status, Status::Ok);
    assert_eq!(remote_check(true, Some(Err("refused".into())), None).status, Status::Error);
}

#[test]
fn internet_resources_and_commands() {
    assert_eq!(internet_check(false, Ok(()), Ok(10)).status, Status::Error);
    assert_eq!(internet_check(true, Err("timeout".into()), Err("x".into())).status, Status::Error);
    assert_eq!(internet_check(true, Ok(()), Err("blocked".into())).status, Status::Warn);
    assert_eq!(internet_check(true, Ok(()), Ok(40)).status, Status::Ok);
    assert_eq!(resources_check(0, &[], true).status, Status::Error);
    assert_eq!(resources_check(120, &["звуки".into()], true).status, Status::Error);
    assert_eq!(resources_check(120, &[], false).status, Status::Warn);
    assert_eq!(resources_check(120, &[], true).status, Status::Ok);
    assert_eq!(commands_check(&[]).status, Status::Unchecked);
    assert_eq!(commands_check(&[("PowerShell", true), ("cmd", false)]).status, Status::Error);
    assert_eq!(monitor_check(0.0, 0.0, 0.0).status, Status::Error);
    assert_eq!(audio_check(1, Some(Err("нет".into()))).status, Status::Error);
    assert_eq!(audio_check(1, None).status, Status::Unchecked);
}

#[test]
fn secrets_are_removed_from_any_text_of_the_report() {
    let _g = uistate::TEST_LOCK.lock();
    uistate::set_for_tests(UiSettings { ai_key: "my-very-secret-key-12345".into(), ..UiSettings::default() });
    let t = sanitize("ключ my-very-secret-key-12345 и sk-abcdefghijklmnop, http://192.168.1.5:9713/?t=deadbeefcafe Bearer qwerty123 key=zzz AIzaSyXXXXXXXXXXXX");
    for secret in ["my-very-secret-key-12345", "sk-abcdefghijklmnop", "deadbeefcafe", "qwerty123", "zzz", "AIzaSyXXXXXXXXXXXX"] {
        assert!(!t.contains(secret), "{} leaked: {}", secret, t);
    }
    assert!(t.contains("ключ"), "the rest of the text stays: {}", t);
    uistate::set_for_tests(UiSettings::default());
}

#[test]
fn the_report_has_no_secrets_no_conversations_and_copying_it_changes_nothing() {
    let _g = uistate::TEST_LOCK.lock();
    let _d = crate::dialog::TEST_LOCK.lock();
    let key = format!("sk-test-{}-SECRETKEY", rand::random::<u32>());
    uistate::set_for_tests(UiSettings { ai_provider: "gemini".into(), ai_key: key.clone(), ..UiSettings::default() });
    let private = format!("мой личный вопрос {}", rand::random::<u32>());
    crate::dialog::begin(&private, "command");
    crate::dialog::reply(&format!("личный ответ про {}", private), "command");
    crate::dialog::end(true);
    uistate::log_event("command", &format!("Команда «{}» (x): выполняется", private));
    uistate::log_event("error", &format!("ИИ: ошибка с ключом {}", key));
    crate::notify::push("command", "Уведомление", &format!("личное уведомление {}", private), true);

    let before_stats = crate::ai::store().stats();
    let before_ai = crate::ai::status_json()["state"].clone();
    let report = report();
    let json_text = json().to_string();
    // (other tests may add messages meanwhile) nothing disappears and the report itself is not stored anywhere
    let after = crate::ai::store().stats();
    assert!(after["chats"].as_u64() >= before_stats["chats"].as_u64() && after["messages"].as_u64() >= before_stats["messages"].as_u64(), "copying the report never deletes history");
    let active = crate::ai::store().active_or_new();
    assert!(crate::ai::store().load(&active.id).unwrap().messages.iter().all(|m| !m.content.contains("Диагностический отчёт")), "the report is not written into the conversation");
    assert_eq!(crate::ai::status_json()["state"], before_ai, "and never sends a request to the AI");
    for text in [&report, &json_text] {
        assert!(!text.contains(&key), "the key leaked");
        assert!(!text.contains(&private), "a conversation leaked");
        assert!(!text.contains("личное уведомление"), "a notification text leaked");
    }
    assert!(report.contains("Версия Jarvis"), "{}", report);
    assert!(report.contains("Google Gemini"), "the provider name is useful and not secret: {}", report);
    uistate::set_for_tests(UiSettings::default());
}

#[test]
fn the_privacy_list_says_honestly_what_uses_the_internet() {
    let list = |s: &UiSettings| internet_uses(s).as_array().unwrap().clone();
    let find = |l: &Vec<Value>, name: &str| l.iter().find(|x| x["name"].as_str().unwrap().contains(name)).unwrap().clone();
    let off = list(&UiSettings { ai_enabled: false, ai_provider: "gemini".into(), ..UiSettings::default() });
    assert_eq!(find(&off, "ИИ")["active"], false);
    let on = list(&UiSettings { ai_provider: "gemini".into(), ..UiSettings::default() });
    assert_eq!(find(&on, "ИИ")["internet"], true);
    let local = list(&UiSettings { ai_provider: "ollama".into(), ..UiSettings::default() });
    assert_eq!(find(&local, "ИИ")["internet"], false);
    assert_eq!(find(&on, "Vosk")["internet"], false, "speech recognition is local");
    assert_eq!(find(&on, "Пульт")["internet"], false);
}

#[test]
fn telegram_is_working_only_when_telegram_really_answers() {
    assert_eq!(telegram_check(false, true, true, "").status, Status::Unchecked, "off: not tested, not «ошибка»");
    assert_eq!(telegram_check(true, false, false, "").status, Status::Error);
    let down = telegram_check(true, true, true, "нет связи с api.telegram.org — повтор через 10 с");
    assert_eq!(down.status, Status::Error);
    assert!(down.detail.contains("api.telegram.org"));
    assert_eq!(telegram_check(true, true, false, "работает").status, Status::Warn, "on the line but nobody is paired");
    assert_eq!(telegram_check(true, true, true, "подключаюсь…").status, Status::Error, "«подключаюсь» is not «работает»");
    assert_eq!(telegram_check(true, true, true, "работает").status, Status::Ok);
}

#[test]
fn the_tunnel_is_working_only_when_its_address_really_opened() {
    assert_eq!(tunnel_check("off", "", None).status, Status::Unchecked);
    assert_eq!(tunnel_check("starting", "", None).status, Status::Warn);
    assert_eq!(tunnel_check("error", "нет сети", None).status, Status::Error);
    assert_eq!(tunnel_check("on", "", None).status, Status::Unchecked, "an address that was not opened proves nothing");
    assert_eq!(tunnel_check("on", "", Some(Err("ответ 502".into()))).status, Status::Error);
    assert_eq!(tunnel_check("on", "", Some(Ok(()))).status, Status::Ok);
}

#[test]
fn the_notifications_of_other_programs_are_judged_by_the_state_of_the_watcher() {
    assert_eq!(notify_check(false, "off", "").status, Status::Unchecked);
    assert_eq!(notify_check(true, "on", "").status, Status::Ok);
    assert_eq!(notify_check(true, "denied", "").status, Status::Error);
    assert_eq!(notify_check(true, "starting", "").status, Status::Warn);
    assert_eq!(notify_check(true, "error", "PowerShell не запустился").status, Status::Error);
    assert_eq!(notify_check(true, "unsupported", "").status, Status::Unchecked);
}

#[test]
fn the_life_of_the_program_needs_a_writable_data_folder_and_the_right_autostart() {
    assert_eq!(lifecycle_check(Ok(()), false, false, Some(120.0)).status, Status::Ok);
    assert_eq!(lifecycle_check(Err("отказано в доступе".into()), false, false, None).status, Status::Error);
    assert_eq!(lifecycle_check(Ok(()), true, true, None).status, Status::Warn, "autostart of another copy");
    assert_eq!(lifecycle_check(Ok(()), true, false, None).status, Status::Ok);
}

// the diagnostics of every run never carry a secret or a message text
#[test]
fn the_new_checks_hold_no_token_and_no_message_text() {
    let t = telegram_check(true, true, true, "работает");
    let all = format!("{} {} {}", t.detail, t.fix, notify_check(true, "on", "").detail);
    assert!(!all.contains("AAH") && !all.contains("dpapi"));
}

#[test]
fn powershell_is_judged_by_every_way_with_its_own_refusal() {
    let ok = |n: &str| (n.to_string(), Ok(()));
    let bad = |n: &str, e: &str| (n.to_string(), Err(e.to_string()));
    assert_eq!(powershell_check(&[]).status, Status::Unchecked);
    assert_eq!(powershell_check(&[ok("a"), ok("b")]).status, Status::Ok);
    // the way Jarvis uses first refuses, another works: a warning that names the refusal (the exact text of the user's screenshot)
    let w = powershell_check(&[bad("powershell", "Такой запрос не поддерживается. (os error 50)"), ok("полный путь")]);
    assert_eq!(w.status, Status::Warn);
    assert!(w.detail.contains("os error 50") && w.detail.contains("полный путь"), "{}", w.detail);
    // none works: an error with every refusal and a way out
    let e = powershell_check(&[bad("a", "os error 50"), bad("b", "os error 5")]);
    assert_eq!(e.status, Status::Error);
    assert!(e.detail.contains("os error 50") && e.detail.contains("os error 5") && e.fix.contains("Piper"), "{} / {}", e.detail, e.fix);
}

#[test]
fn a_russian_phrase_must_not_be_read_by_an_english_voice() {
    // the voice of another language than Jarvis's: a warning (this was the «nonsense» of the first speech tests)
    let c = piper_language_check("ru", Some(("en.onnx".into(), false))).unwrap();
    assert_eq!(c.status, Status::Warn);
    assert!(c.detail.contains("en.onnx") && c.detail.contains("ru"), "{}", c.detail);
    assert!(piper_language_check("ru", Some(("ru.onnx".into(), true))).is_none());
    assert!(piper_language_check("ru", None).is_none(), "no Piper: the card of Piper says so");
}

fn game_facts() -> crate::gamemode::Facts {
    crate::gamemode::Facts { active: false, in_game: false, anticheat: None, games: 0, macros: 0, recording: false, playing: false, overlay: false, sound: false, scan_running: false, scan: None }
}

#[test]
fn the_game_card_tells_the_anti_cheat_the_unused_mode_and_the_working_one() {
    let c = game_check(&game_facts());
    assert_eq!(c.status, Status::Unchecked);
    assert!(c.fix.contains("Найти мои игры"), "{}", c.fix);
    let c = game_check(&crate::gamemode::Facts { anticheat: Some("easyanticheat.exe".into()), active: true, ..game_facts() });
    assert_eq!(c.status, Status::Warn);
    assert!(c.detail.contains("easyanticheat.exe") && c.detail.contains("не включается"), "{}", c.detail);
    let c = game_check(&crate::gamemode::Facts { active: true, in_game: true, games: 3, macros: 2, scan: Some((5, 3, 2)), ..game_facts() });
    assert_eq!(c.status, Status::Ok);
    assert!(c.detail.contains("игр в списке 3") && c.detail.contains("найдено 5, добавлено 3, не добавлено 2"), "{}", c.detail);
    assert!(c.detail.contains("не подтверждает"), "an honest card: it does not claim that the game works");
}
