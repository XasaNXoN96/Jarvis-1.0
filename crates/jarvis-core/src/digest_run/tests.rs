use super::*;

#[test]
fn reminders_of_the_next_24_hours_are_listed_in_order() {
    let now = Local.with_ymd_and_hms(2026, 11, 17, 7, 30, 0).unwrap().timestamp();
    let at = |h: u32, d: u32| Local.with_ymd_and_hms(2026, 11, d, h, 15, 0).unwrap().timestamp();
    let json = serde_json::json!([
        { "id": 1, "due": at(18, 17), "title": "Таймер", "message": "тренировка" },
        { "id": 2, "due": at(9, 17), "title": "Напоминание", "message": "позвонить маме" },
        { "id": 3, "due": at(5, 17), "title": "Прошло", "message": "уже было" },
        { "id": 4, "due": at(8, 19), "title": "Далеко", "message": "через два дня" },
        { "id": 5, "due": at(6, 18), "title": "Утро", "message": "" },
        { "id": 6, "due": at(7, 18), "title": "Утро", "message": "разбудить\nменя   в семь" },
    ])
    .to_string();
    let r = reminders_from_json(&json, now);
    assert_eq!(r, vec!["в 09:15 — позвонить маме", "в 18:15 — тренировка", "завтра в 06:15 — Утро", "завтра в 07:15 — разбудить меня в семь"]);
    assert!(reminders_from_json("{broken", now).is_empty());
    assert!(reminders_from_json("{}", now).is_empty());
}

#[test]
fn only_the_last_notes_are_taken_without_stamps() {
    let text = "[2026-11-01 10:00] первая\n[2026-11-02 11:00] вторая\n\n[2026-11-03 12:00] третья\nбез метки\n   \n[2026-11-04 12:00]   четвёртая  ";
    assert_eq!(notes_from_text(text), vec!["третья", "без метки", "четвёртая"]);
    assert!(notes_from_text("").is_empty());
    let long = format!("[2026-11-04 12:00] {}", "я".repeat(500));
    assert_eq!(notes_from_text(&long)[0].chars().count(), MAX_ITEM_CHARS);
}

#[test]
fn without_a_connected_ai_the_briefing_is_composed_locally() {
    let data = Data { weekday: "вторник".into(), day: 17, month: 11, hour: 7, notes: vec!["купить молоко".into()], ..Default::default() };
    let (text, by_ai) = compose(&data);
    assert!(!by_ai);
    assert!(text.starts_with("Доброе утро, сэр. Сегодня вторник, 17 ноября.") && text.contains("купить молоко"), "{}", text);
}

// the "Команды" tab: the pack file parses, its script exists, and every listed phrase is really recognised by the program
#[test]
fn the_commands_tab_entries_match_what_the_program_recognises() {
    use crate::commands::JCommandsList;
    use crate::digest::{parse_command, Cmd};
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/digest");
    let list: JCommandsList = toml::from_str(&std::fs::read_to_string(dir.join("command.toml")).expect("pack file")).expect("valid toml");
    assert_eq!(list.commands.len(), 3);
    for c in &list.commands {
        assert!(dir.join(&c.script).exists(), "{}", c.script);
        assert!(c.triggers.is_empty(), "no keyword triggers: the program handles the phrases first");
        for phrase in c.phrases.get("ru").expect("ru phrases") {
            let cmd = parse_command(phrase).unwrap_or_else(|| panic!("not recognised: {}", phrase));
            let ok = match c.id.as_str() {
                "digest_now" => cmd == Cmd::Now,
                "digest_on" => matches!(cmd, Cmd::Enable(_)),
                "digest_off" => matches!(cmd, Cmd::Disable | Cmd::Status),
                other => panic!("unexpected id {}", other),
            };
            assert!(ok, "{} -> {:?}", phrase, cmd);
        }
    }
    // the canonical phrases of the script are recognised (so the script cannot loop)
    assert_eq!(parse_command("утренняя сводка"), Some(Cmd::Now));
    assert_eq!(parse_command("включи утреннюю сводку"), Some(Cmd::Enable(None)));
    assert_eq!(parse_command("выключи утреннюю сводку"), Some(Cmd::Disable));
    assert_eq!(crate::uistate::group_key("digest", "digest_now"), "info");
}
