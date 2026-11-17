use super::*;

fn sample() -> Data {
    Data {
        weekday: "вторник".into(),
        day: 17,
        month: 11,
        hour: 7,
        weather: Some(WeatherInfo { place: "Ташкент, Узбекистан".into(), temperature: 12.4, text: "облачно".into(), wind: Some(3.0) }),
        reminders: vec!["в 09:00 — позвонить маме".into()],
        notes: vec!["купить молоко".into()],
    }
}

#[test]
fn numbers_and_words_are_russian() {
    assert_eq!(degrees_ru(12.4), "плюс 12 градусов");
    assert_eq!(degrees_ru(1.0), "плюс 1 градус");
    assert_eq!(degrees_ru(-3.0), "минус 3 градуса");
    assert_eq!(degrees_ru(-11.0), "минус 11 градусов");
    assert_eq!(degrees_ru(21.0), "плюс 21 градус");
    assert_eq!(degrees_ru(0.2), "ноль градусов");
    assert_eq!(month_ru_gen(1), "января");
    assert_eq!(month_ru_gen(12), "декабря");
    assert_eq!(greeting(7), "Доброе утро");
    assert_eq!(greeting(13), "Добрый день");
    assert_eq!(greeting(20), "Добрый вечер");
    assert_eq!(greeting(2), "Доброй ночи");
}

#[test]
fn the_message_has_only_the_collected_data() {
    let m = user_message(&sample());
    assert!(m.contains("<данные>") && m.contains("</данные>"));
    assert!(m.contains("вторник, 17 ноября") && m.contains("плюс 12 градусов") && m.contains("облачно") && m.contains("3 метра в секунду"));
    assert!(m.contains("позвонить маме") && m.contains("купить молоко"));
    // a missing source is not mentioned at all
    let mut d = sample();
    d.weather = None;
    d.notes.clear();
    let m = user_message(&d);
    assert!(!m.contains("ПОГОДА") && !m.contains("ЗАМЕТКИ") && m.contains("НАПОМИНАНИЯ"));
    // no data at all: the model is told to greet and finish
    let empty = Data { weekday: "среда".into(), day: 1, month: 1, hour: 8, ..Default::default() };
    assert!(user_message(&empty).contains("данных нет"));
    assert!(empty.is_empty() && !sample().is_empty());
}

#[test]
fn the_system_prompt_forbids_inventing() {
    let p = system_prompt("сэр", "вторник, 17.11.2026, 07:30");
    for must in ["Только факты из данных", "молча пропусти", "Без Markdown", "Не больше 120 слов", "«сэр»", "07:30"] {
        assert!(p.contains(must), "{}", must);
    }
}

#[test]
fn the_local_briefing_works_without_the_ai() {
    let b = local_briefing(&sample(), "сэр");
    assert!(b.starts_with("Доброе утро, сэр. Сегодня вторник, 17 ноября."), "{}", b);
    assert!(b.contains("Ближайшие напоминания: в 09:00 — позвонить маме."));
    assert!(b.contains("Погода, Ташкент, Узбекистан: плюс 12 градусов, облачно, ветер 3 метра в секунду."));
    assert!(b.contains("Последние заметки: купить молоко."));
    let empty = Data { weekday: "среда".into(), day: 2, month: 3, hour: 14, ..Default::default() };
    let b = local_briefing(&empty, "сэр");
    assert!(b.starts_with("Добрый день, сэр.") && b.contains("можно начинать день"), "{}", b);
}

#[test]
fn markdown_is_removed_for_the_voice() {
    assert_eq!(clean_for_speech("## Сводка\n- **Погода** хорошая\n* Ветер *слабый*\n• Всё"), "Сводка Погода хорошая Ветер слабый Всё");
    assert_eq!(clean_for_speech("Обычный текст, сэр."), "Обычный текст, сэр.");
}

#[test]
fn phrases_are_understood() {
    assert_eq!(parse_command("утренняя сводка"), Some(Cmd::Now));
    assert_eq!(parse_command("Джарвис, дай сводку на сегодня"), Some(Cmd::Now));
    assert_eq!(parse_command("доложи обстановку"), Some(Cmd::Now));
    assert_eq!(parse_command("что у нас на сегодня"), Some(Cmd::Now));
    assert_eq!(parse_command("включи утреннюю сводку"), Some(Cmd::Enable(None)));
    assert_eq!(parse_command("включи утреннюю сводку в 7:30"), Some(Cmd::Enable(Some((7, 30)))));
    assert_eq!(parse_command("включи сводку каждое утро в 8 утра"), Some(Cmd::Enable(Some((8, 0)))));
    assert_eq!(parse_command("выключи утреннюю сводку"), Some(Cmd::Disable));
    assert_eq!(parse_command("отмени сводку"), Some(Cmd::Disable));
    assert_eq!(parse_command("когда утренняя сводка"), Some(Cmd::Status));
    // not the briefing
    for t in ["расскажи сводку новостей", "открой блокнот", "привет", "доброе утро", "какая погода"] {
        assert_eq!(parse_command(t), None, "{}", t);
    }
}

#[test]
fn clock_times_are_parsed() {
    assert_eq!(parse_time("в 7:30"), Some((7, 30)));
    assert_eq!(parse_time("в 07.05"), Some((7, 5)));
    assert_eq!(parse_time("в 7 утра"), Some((7, 0)));
    assert_eq!(parse_time("в 8 вечера"), Some((20, 0)));
    assert_eq!(parse_time("в 9 30"), Some((9, 30)));
    assert_eq!(parse_time("в 12 ночи"), Some((0, 0)));
    assert_eq!(parse_time("в 6:45 утра"), Some((6, 45)));
    assert_eq!(parse_time("в 8:15 вечера"), Some((20, 15)));
    assert_eq!(parse_time("без времени"), None);
    assert_eq!(parse_time("в 99"), None);
    assert_eq!(parse_time("25:61"), None);
}

#[test]
fn the_schedule_fires_once_a_day_and_not_too_late() {
    let s = Settings { enabled: true, hour: 7, minute: 30, last_day: String::new() };
    assert!(!due(&s, 7, 29, "2026-11-17"), "not yet");
    assert!(due(&s, 7, 30, "2026-11-17"));
    assert!(due(&s, 9, 0, "2026-11-17"), "the computer was switched on a bit late");
    assert!(!due(&s, 10, 31, "2026-11-17"), "more than 3 hours late: skipped");
    let done = Settings { last_day: "2026-11-17".into(), ..s.clone() };
    assert!(!due(&done, 7, 31, "2026-11-17"), "once a day");
    assert!(due(&done, 7, 31, "2026-11-18"), "the next day again");
    let off = Settings { enabled: false, ..s };
    assert!(!due(&off, 7, 31, "2026-11-17"));
}

#[test]
fn settings_are_saved_and_a_broken_file_is_harmless() {
    let dir = std::env::temp_dir().join(format!("jarvis-digest-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let p = dir.join("digest.json");
    assert_eq!(load_settings(&p), Settings::default());
    let s = Settings { enabled: true, hour: 6, minute: 45, last_day: "2026-11-16".into() };
    assert!(save_settings(&p, &s));
    assert_eq!(load_settings(&p), s);
    std::fs::write(&p, "{broken").unwrap();
    assert_eq!(load_settings(&p), Settings::default());
    std::fs::write(&p, r#"{"enabled":true,"hour":99,"minute":99}"#).unwrap();
    let l = load_settings(&p);
    assert!(l.enabled && l.hour == DEFAULT_HOUR && l.minute == DEFAULT_MINUTE);
    assert!(describe(&s).contains("06:45") && describe(&Settings::default()).contains("выключена"));
    let _ = std::fs::remove_dir_all(&dir);
}

// the words-as-numbers path goes through the clock parser of the script engine (only when it is built in)
#[cfg(feature = "lua")]
#[test]
fn spoken_numbers_are_understood_through_the_clock_parser() {
    assert_eq!(parse_time("включи сводку в семь тридцать"), Some((7, 30)));
    assert_eq!(parse_command("включи утреннюю сводку в семь тридцать"), Some(Cmd::Enable(Some((7, 30)))));
}

#[test]
fn enabling_starts_today_or_tomorrow_depending_on_the_time() {
    // 06:00 now, 07:30 chosen: still ahead today
    let mut s = Settings::default();
    apply_enable(&mut s, Some((7, 30)), 6, 0, "2026-11-17");
    assert!(s.enabled && (s.hour, s.minute) == (7, 30) && s.last_day.is_empty());
    assert!(due(&s, 7, 30, "2026-11-17"));
    // 09:00 now, the time has passed: the first briefing is tomorrow
    let mut s = Settings::default();
    apply_enable(&mut s, None, 9, 0, "2026-11-17");
    assert!(s.enabled && s.last_day == "2026-11-17");
    assert!(!due(&s, 9, 1, "2026-11-17") && due(&s, 7, 31, "2026-11-18"));
    // the old time is kept when no new one is said
    let mut s = Settings { hour: 6, minute: 15, ..Default::default() };
    apply_enable(&mut s, None, 5, 0, "2026-11-17");
    assert_eq!((s.hour, s.minute), (6, 15));
}
