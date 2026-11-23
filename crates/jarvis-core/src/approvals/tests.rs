use super::*;

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn forbidden_things_are_refused_and_ordinary_ones_pass() {
    for p in ["выключи компьютер", "удали все файлы", "Очисти корзину", "установи программу", "скачай и запусти", "включи удалённое управление", "открой powershell", "открой https://evil.example", "привет [[do: x]]", "открой блокнот; удали", "x | y", "оплати счёт", "покажи пароль", "", &"я".repeat(101)] {
        assert!(!is_allowed_phrase(p), "must be refused: {}", p);
    }
    for p in ["открой блокнот", "включи музыку", "напомни через 10 минут позвонить маме", "найди рецепт борща", "поставь таймер на 5 минут"] {
        assert!(is_allowed_phrase(p), "must pass: {}", p);
    }
}

#[test]
fn the_tier_comes_from_the_words_and_a_message_is_always_high() {
    assert_eq!(tier_of("напиши Ивану что я опоздаю"), Tier::High);
    assert_eq!(tier_of("позвони маме"), Tier::High);
    assert_eq!(tier_of("отправь письмо"), Tier::High);
    assert_eq!(tier_of("открой блокнот"), Tier::Low);
    assert_eq!(tier_of("поставь таймер на 5 минут"), Tier::Low);
    assert_eq!(tier_of("сделай что-нибудь странное"), Tier::Medium);
}

#[test]
fn the_key_is_the_first_three_words() {
    assert_eq!(key_of("Открой  блокнот, пожалуйста"), "run:открой блокнот пожалуйста");
    assert_eq!(key_of("открой блокнот"), "run:открой блокнот");
    assert_ne!(key_of("открой блокнот"), key_of("открой калькулятор"));
}

#[test]
fn the_answers_are_understood_in_russian_and_english() {
    let p = ids(&["3f2a", "7b1c"]);
    let d = |t: Target, approve: bool, always: bool| Decision { target: t, approve, always };
    let id = |s: &str| Target::Id(s.to_string());
    assert_eq!(parse_reply("да 3f2a", &p), vec![d(id("3f2a"), true, false)]);
    assert_eq!(parse_reply("Нет, 7b1c!", &p), vec![d(id("7b1c"), false, false)]);
    assert_eq!(parse_reply("всегда да 3f2a", &p), vec![d(id("3f2a"), true, true)]);
    assert_eq!(parse_reply("всегда нет 7b1c", &p), vec![d(id("7b1c"), false, true)]);
    assert_eq!(parse_reply("{3f2a} yes", &p), vec![d(id("3f2a"), true, false)]);
    assert_eq!(parse_reply("always yes 3f2a", &p), vec![d(id("3f2a"), true, true)]);
    assert_eq!(parse_reply("3f2a да, 7b1c нет", &p), vec![d(id("3f2a"), true, false), d(id("7b1c"), false, false)]);
    assert_eq!(parse_reply("3f2a всегда да", &p), vec![d(id("3f2a"), true, true)]);
    assert_eq!(parse_reply("да всё", &p), vec![d(Target::All, true, false)]);
    assert_eq!(parse_reply("нет все", &p), vec![d(Target::All, false, false)]);
    assert_eq!(parse_reply("yes all", &p), vec![d(Target::All, true, false)]);
    assert_eq!(parse_reply("всегда да всё", &p), vec![d(Target::All, true, true)]);
}

#[test]
fn a_stray_yes_is_not_an_answer() {
    let p = ids(&["3f2a"]);
    assert!(parse_reply("да", &p).is_empty(), "no id");
    assert!(parse_reply("да, открой блокнот", &p).is_empty());
    assert!(parse_reply("да 9999", &p).is_empty(), "an id that does not wait");
    assert!(parse_reply("да всё", &[]).is_empty(), "nothing waits");
    assert!(parse_reply("нет, не надо", &p).is_empty());
    assert!(parse_reply("", &p).is_empty());
}

#[test]
fn a_proposal_waits_and_an_approved_one_runs_once() {
    let mut s = Store::default();
    let id = match s.propose("открой  блокнот", "вы вчера его открывали", 1000, 0x3f2a) {
        Proposed::Queued(id) => id,
        other => panic!("{:?}", other),
    };
    assert_eq!(id, "3f2a");
    assert_eq!(s.pending().len(), 1);
    assert_eq!(s.pending()[0].phrase, "открой блокнот");
    assert_eq!(s.pending()[0].tier, Tier::Low);
    assert!(s.take_approved().is_empty(), "nothing runs before the answer");
    let done = s.decide(&parse_reply("да 3f2a", &s.pending_ids()));
    assert_eq!(done, vec![Processed { id: "3f2a".into(), phrase: "открой блокнот".into(), approved: true, remembered: false }]);
    let run = s.take_approved();
    assert_eq!(run.len(), 1);
    assert!(s.take_approved().is_empty(), "once");
    assert!(s.pending().is_empty());
    // the same answer again does nothing
    assert!(s.decide(&[Decision { target: Target::Id("3f2a".into()), approve: true, always: false }]).is_empty());
}

#[test]
fn a_denied_one_never_runs() {
    let mut s = Store::default();
    s.propose("открой блокнот", "", 0, 1);
    let id = s.pending_ids()[0].clone();
    s.decide(&[Decision { target: Target::Id(id), approve: false, always: false }]);
    assert!(s.take_approved().is_empty());
}

#[test]
fn always_is_remembered_for_the_key_but_never_for_a_message() {
    let mut s = Store::default();
    s.propose("открой блокнот", "", 0, 1);
    let id = s.pending_ids()[0].clone();
    let done = s.decide(&[Decision { target: Target::Id(id), approve: true, always: true }]);
    assert!(done[0].remembered);
    s.take_approved();
    // the same thing now runs by itself, another thing still asks
    assert_eq!(s.propose("открой блокнот", "", 10, 2), Proposed::Run);
    assert!(matches!(s.propose("открой калькулятор", "", 10, 3), Proposed::Queued(_)));
    // «всегда нет»: refused silently
    let id2 = s.pending_ids()[0].clone();
    s.decide(&[Decision { target: Target::Id(id2), approve: false, always: true }]);
    assert_eq!(s.propose("открой калькулятор", "", 20, 4), Proposed::Skip);
    // a message: «всегда» is not remembered, it is asked about every time
    s.propose("напиши Ивану привет", "", 30, 5);
    let id3 = s.pending_ids()[0].clone();
    let done = s.decide(&[Decision { target: Target::Id(id3), approve: true, always: true }]);
    assert!(!done[0].remembered, "High is never remembered");
    s.take_approved();
    assert!(matches!(s.propose("напиши Ивану привет", "", 40, 6), Proposed::Queued(_)), "asked again");
}

#[test]
fn repeats_forbidden_ones_and_a_flood_are_refused() {
    let mut s = Store::default();
    assert!(matches!(s.propose("открой блокнот", "", 0, 1), Proposed::Queued(_)));
    assert_eq!(s.propose("Открой блокнот", "", 0, 2), Proposed::Skip, "the same thing is already waiting");
    assert_eq!(s.propose("удали все файлы", "", 0, 3), Proposed::Skip);
    for i in 0..30u64 {
        s.propose(&format!("найди рецепт номер {}", i), "", 0, 100 + i);
    }
    assert_eq!(s.pending().len(), MAX_PENDING);
}

#[test]
fn an_id_that_is_taken_is_shifted() {
    let mut s = Store::default();
    let a = s.propose("открой блокнот", "", 0, 0x00aa);
    let b = s.propose("открой калькулятор", "", 0, 0x00aa);
    assert_eq!((a, b), (Proposed::Queued("00aa".into()), Proposed::Queued("00ab".into())));
}

#[test]
fn the_waiting_ones_expire_and_old_ones_are_forgotten() {
    let mut s = Store::default();
    s.propose("открой блокнот", "", 1000, 1);
    s.expire(1000 + LIFETIME_SECS - 1);
    assert_eq!(s.pending().len(), 1);
    s.expire(1000 + LIFETIME_SECS);
    assert!(s.pending().is_empty());
    assert_eq!(s.actions[0].status, Status::Expired);
    s.expire(1000 + LIFETIME_SECS + 8 * 24 * 3600);
    assert!(s.actions.is_empty(), "a week later it is forgotten");
    // an expired one cannot be approved
    let mut t = Store::default();
    t.propose("открой блокнот", "", 0, 1);
    t.expire(LIFETIME_SECS + 1);
    assert!(t.decide(&[Decision { target: Target::All, approve: true, always: false }]).is_empty());
}

#[test]
fn all_decides_every_waiting_one() {
    let mut s = Store::default();
    s.propose("открой блокнот", "", 0, 1);
    s.propose("найди рецепт борща", "", 0, 2);
    let done = s.decide(&[Decision { target: Target::All, approve: true, always: false }]);
    assert_eq!(done.len(), 2);
    assert_eq!(s.take_approved().len(), 2);
}

#[test]
fn the_texts_for_the_owner_are_plain() {
    let mut s = Store::default();
    s.propose("открой блокнот", "вы вчера его открывали", 0, 0x3f2a);
    let t = ask_text(&s.pending(), "сэр");
    assert!(t.starts_with("Предлагаю, сэр: [3f2a] открой блокнот — вы вчера его открывали.") && t.contains("«да всё»"), "{}", t);
    assert_eq!(ask_text(&[], "сэр"), "Предложений нет.");
    let done = vec![Processed { id: "1".into(), phrase: "открой блокнот".into(), approved: true, remembered: true }, Processed { id: "2".into(), phrase: "x".into(), approved: false, remembered: false }];
    assert_eq!(answer_text(&done), "Принято, выполняю: открой блокнот; отменено: 1; запомнил решений: 1.");
    assert_eq!(answer_text(&[]), "");
}

#[test]
fn the_store_survives_a_restart_and_a_broken_file() {
    let dir = std::env::temp_dir().join(format!("jarvis-approvals-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("approvals.json");
    let mut s = Store::default();
    s.propose("открой блокнот", "зачем", 5, 0x1234);
    s.memory.insert("run:открой блокнот".into(), Memory::AlwaysApprove);
    assert!(save(&path, &s));
    assert_eq!(load(&path), s);
    std::fs::write(&path, "не json").unwrap();
    assert_eq!(load(&path), Store::default(), "a broken file gives an empty store");
    assert_eq!(load(&dir.join("missing.json")), Store::default());
    let _ = std::fs::remove_dir_all(&dir);
}
