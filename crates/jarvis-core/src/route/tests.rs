use super::*;

#[test]
fn knowledge_questions_go_to_the_ai() {
    for t in [
        "Что такое программа?", "что такое искусственный интеллект", "Объясни, как работает Excel", "объясни что такое нейросеть",
        "Напиши программу на Python", "помоги составить учебный план", "Расскажи про Ташкент", "почему небо голубое", "как работает word",
        "чем отличается ворд от экселя", "Кто такой Эйнштейн", "составь учебный план по информатике", "напиши стих про осень",
    ] {
        assert_eq!(decide(t, true), Route::Ai, "{}", t);
    }
}

#[test]
fn commands_are_never_sent_to_the_ai() {
    for t in [
        "открой ворд", "открой word", "запусти тотал командер", "закрой excel", "поставь таймер на 30 секунд", "громкость 50 процентов",
        "сколько времени", "какая погода", "выключи компьютер", "включи музыку", "найди файл отчёт", "ячейка а1 пиши 5", "пиши привет как дела",
        "что ты умеешь", "кто ты", "расскажи о себе", "как дела", "кто такой джарвис", "найди в википедии Ташкент", "найди в интернете погоду",
        "сделай скриншот", "заканчивай", "открой блокнот",
    ] {
        assert_eq!(decide(t, true), Route::Command, "{}", t);
    }
}

#[test]
fn without_a_connected_ai_everything_stays_with_the_commands() {
    assert_eq!(decide("Что такое программа?", false), Route::Command);
    assert_eq!(decide("объясни как работает excel", false), Route::Command);
}

#[test]
fn a_program_name_inside_a_question_does_not_start_the_program() {
    // the router and the program parser agree: a question is not an "open" request
    for t in ["Объясни, как работает Excel", "что такое ворд", "как работает total commander"] {
        assert!(is_knowledge_question(t), "{}", t);
        assert!(apps::parse_request(t).is_none(), "{}", t);
    }
}

#[test]
fn open_questions_after_the_commands_failed() {
    assert!(is_open_question("а где живут пингвины?"));
    assert!(is_open_question("сколько стоит билет в музей"));
    assert!(!is_open_question("привет"));
    assert!(!is_open_question("ворд"));
}

#[test]
fn search_query_drops_the_question_words() {
    assert_eq!(search_query("Что такое программа?"), "программа");
    assert_eq!(search_query("расскажи про Ташкент"), "ташкент");
    assert_eq!(search_query("кто такой Эйнштейн"), "эйнштейн");
}

#[test]
fn the_text_is_shown_as_typed_without_the_wake_word() {
    assert_eq!(original_without_wake("Сколько будет 2 + 2?", "сколько будет 2 + 2?"), "Сколько будет 2 + 2?");
    assert_eq!(original_without_wake("Джарвис, Что такое Python?", "что такое python?"), "Что такое Python?");
    assert_eq!(original_without_wake("Открой Word", "открой word"), "Открой Word");
    // the matcher removed the question mark: the user's one is kept
    assert_eq!(original_without_wake("Что такое Python?", "что такое python"), "Что такое Python?");
    // nothing to find: the matched text is used
    assert_eq!(original_without_wake("", "привет"), "привет");
}

#[test]
fn a_question_about_a_program_is_a_question_not_a_launch() {
    for t in ["Почему не запускается Word?", "почему не открывается эксель", "Почему тормозит total commander"] {
        assert!(is_knowledge_question(t), "{}", t);
        assert_eq!(decide(t, true), Route::Ai, "{}", t);
        assert!(apps::parse_request(t).is_none(), "{}", t);
    }
}

#[test]
fn local_commands_never_go_to_the_ai_or_a_web_search() {
    for text in ["сколько времени", "открой блокнот", "поставь таймер на 5 минут", "какое сегодня число", "громкость 50 процентов"] {
        assert_eq!(decide(text, true), Route::Command, "{} stays a local command even with the AI connected", text);
        assert_eq!(decide(text, false), Route::Command);
    }
    assert_eq!(decide("что такое python", true), Route::Ai);
    assert_eq!(decide("что такое python", false), Route::Command, "without the AI (switched off) nothing is sent");
}

#[test]
fn the_talk_mode_ends_only_on_its_own_words() {
    for t in ["закончи чат", "выйди из чата", "закончи разговор", "хватит", "стоп", "Стоп!", "end chat"] {
        assert!(ends_chat(t), "{}", t);
    }
    for t in ["расскажи про чат-боты", "что такое разговорный стиль", "как дела", "продолжим"] {
        assert!(!ends_chat(t), "{} is a question, not the end", t);
    }
}
