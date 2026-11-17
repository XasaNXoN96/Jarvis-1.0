use super::*;

#[test]
fn marks_are_removed_and_collected() {
    let (text, actions) = extract("Сию минуту, сэр. [[do: открой блокнот]] Заодно запомню. [[remember: пользователь живёт в Ташкенте]]");
    assert_eq!(text, "Сию минуту, сэр. Заодно запомню.");
    assert_eq!(actions, vec![Action::Do("открой блокнот".into()), Action::Remember("пользователь живёт в Ташкенте".into())]);
}

#[test]
fn an_answer_without_marks_is_unchanged() {
    let (text, actions) = extract("Всё в порядке, сэр.\nВторая строка.");
    assert_eq!(text, "Всё в порядке, сэр.\nВторая строка.");
    assert!(actions.is_empty());
}

#[test]
fn unknown_and_empty_marks_are_hidden_but_do_nothing() {
    let (text, actions) = extract("Хорошо. [[fly: в космос]] [[do:   ]] Готово.");
    assert_eq!(text, "Хорошо. Готово.");
    assert!(actions.is_empty());
}

#[test]
fn an_unclosed_mark_drops_the_rest() {
    let (text, actions) = extract("Сделано. [[do: открой бло");
    assert_eq!(text, "Сделано.");
    assert!(actions.is_empty());
}

#[test]
fn forbidden_phrases_never_run() {
    for p in ["выключи компьютер через минуту", "Перезагрузи компьютер", "очисти корзину", "удали файл отчёт", "включи удалённое управление", "shutdown now", ""] {
        assert!(!is_allowed_phrase(p), "{}", p);
    }
    for p in ["открой блокнот", "поставь таймер на 5 минут", "громкость 40 процентов", "сделай скриншот"] {
        assert!(is_allowed_phrase(p), "{}", p);
    }
    assert!(!is_allowed_phrase(&"а".repeat(200)));
}

#[test]
fn runnable_limits_and_filters() {
    let acts = vec![
        Action::Do("открой блокнот".into()),
        Action::Do("открой блокнот".into()),
        Action::Do("выключи компьютер".into()),
        Action::Do("громче".into()),
        Action::Remember("любит чай".into()),
        Action::Do("тише".into()),
    ];
    let r = runnable(&acts);
    assert_eq!(r.len(), MAX_ACTIONS);
    assert_eq!(r[0], Action::Do("открой блокнот".into()));
    assert_eq!(r[1], Action::Do("громче".into()));
    assert!(!r.contains(&Action::Do("выключи компьютер".into())));
}

fn stream(parts: &[&str]) -> String {
    let mut f = MarkerFilter::new();
    let mut out = String::new();
    for p in parts {
        out.push_str(&f.push(p));
    }
    out.push_str(&f.finish());
    out
}

#[test]
fn the_filter_hides_marks_split_anywhere() {
    assert_eq!(stream(&["Сию минуту, сэр. [[do: открой блокнот]]"]), "Сию минуту, сэр. ");
    assert_eq!(stream(&["Сию ", "минуту. [", "[do: от", "кой блокнот]", "] Готово."]), "Сию минуту.  Готово.");
    assert_eq!(stream(&["a [", "b"]), "a [b");
    assert_eq!(stream(&["a [[", "do: x", "]]", "b"]), "a b");
    assert_eq!(stream(&["Текст [[do: незакрытая"]), "Текст ");
}

#[test]
fn the_filter_gives_the_same_text_as_extract() {
    let answer = "Хорошо, сэр. [[do: поставь таймер на 5 минут]] Через пять минут скажу. [[remember: любит крепкий чай]]";
    let (clean, _) = extract(answer);
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let streamed = stream(&[&answer[..10], &answer[10..40], &answer[40..]]);
    assert_eq!(norm(&streamed), norm(&clean));
}

#[test]
fn a_huge_unclosed_mark_does_not_swallow_the_answer_forever() {
    let mut f = MarkerFilter::new();
    f.push("[[");
    let long = "x".repeat(MAX_MARK_CHARS + 50);
    assert_eq!(f.push(&long), "");
    assert_eq!(f.push("Дальше обычный текст."), "Дальше обычный текст.");
}

fn temp_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("jarvis-persona-test-{}-{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir.join("memory.json")
}

#[test]
fn memory_stores_dedupes_and_forgets() {
    let p = temp_file("mem");
    assert!(load_facts(&p).is_empty());
    assert!(add_fact(&p, "Пользователя зовут Хасан"));
    assert!(!add_fact(&p, "пользователя зовут хасан"));
    assert!(add_fact(&p, "Живёт в Ташкенте\nи любит чай"));
    assert_eq!(load_facts(&p), vec!["Пользователя зовут Хасан".to_string(), "Живёт в Ташкенте и любит чай".to_string()]);
    assert_eq!(forget_fact(&p, "ташкент"), 1);
    assert_eq!(forget_fact(&p, "ташкент"), 0);
    assert_eq!(forget_fact(&p, ""), 0);
    assert_eq!(load_facts(&p), vec!["Пользователя зовут Хасан".to_string()]);
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

#[test]
fn memory_is_bounded() {
    let p = temp_file("bound");
    for i in 0..(MAX_FACTS + 10) {
        assert!(add_fact(&p, &format!("факт номер {}", i)));
    }
    let facts = load_facts(&p);
    assert_eq!(facts.len(), MAX_FACTS);
    assert_eq!(facts[0], "факт номер 10");
    assert!(add_fact(&p, &"я".repeat(500)));
    assert!(load_facts(&p).last().unwrap().chars().count() <= MAX_FACT_CHARS);
    // a damaged file is an empty memory, not a crash
    std::fs::write(&p, "{not json").unwrap();
    assert!(load_facts(&p).is_empty());
    let _ = std::fs::remove_dir_all(p.parent().unwrap());
}

#[test]
fn the_prompt_has_the_character_the_time_and_the_memory() {
    let ru = build_prompt("ru", "вторник, 17 ноября 2026, 22:05", "Ташкент", &["Пользователя зовут Хасан".to_string()]);
    assert!(ru.contains("ДЖАРВИС") && ru.contains("сэр") && ru.contains("[[do:") && ru.contains("[[remember:"));
    assert!(ru.contains("вторник, 17 ноября 2026, 22:05") && ru.contains("Ташкент") && ru.contains("- Пользователя зовут Хасан"));
    let empty = build_prompt("ru", "сейчас", "", &[]);
    assert!(empty.contains("не указан") && empty.contains("пока ничего"));
    let en = build_prompt("en", "Tuesday, 22:05", "Tashkent", &[]);
    assert!(en.contains("JARVIS") && en.contains("sir") && en.contains("nothing yet") && en.contains("Tashkent"));
    assert_eq!(weekday_ru(1), "вторник");
    assert_eq!(weekday_en(8), "Tuesday");
}

#[test]
fn code_indentation_survives() {
    let code = "Вот код:\n```python\ndef f():\n    return 1\n```\n";
    let (text, actions) = extract(code);
    assert_eq!(text, code);
    assert!(actions.is_empty());
    let (text, _) = extract("```\nif x:\n    y()\n```\n[[remember: пишет на python]]");
    assert!(text.contains("    y()"));
}
