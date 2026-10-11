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
    assert_eq!(piper_check(true, false, PiperFacts::Ok(30_000), None).status, Status::Ok);
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

    let before_rev = crate::ai::rev();
    let before_stats = crate::ai::store().stats();
    let before_ai = crate::ai::status_json()["state"].clone();
    let report = report();
    let json_text = json().to_string();
    assert_eq!(crate::ai::rev(), before_rev, "copying the report never changes the history");
    assert_eq!(crate::ai::store().stats(), before_stats);
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
