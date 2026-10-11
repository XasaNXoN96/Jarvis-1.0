use super::*;

fn id_of(text: &str) -> Option<&'static str> {
    parse_request(text).map(|r| r.app.id)
}

#[test]
fn word_in_all_its_spoken_forms() {
    for t in ["открой word", "открой ворд", "открой ворт", "Открой Ворд.", "запусти вордс", "открой майкрософт ворд", "открой microsoft word", "открой программу ворд", "открой ворд пожалуйста"] {
        assert_eq!(id_of(t), Some("word"), "{}", t);
    }
}

#[test]
fn excel_in_all_its_spoken_forms() {
    for t in ["открой excel", "открой эксель", "открой эксел", "открой ексел", "открой ексель", "запусти эксэль", "открой майкрософт эксель"] {
        assert_eq!(id_of(t), Some("excel"), "{}", t);
    }
}

#[test]
fn total_commander_in_all_its_spoken_forms() {
    for t in ["открой total commander", "открой Total Commander", "открой тотал командер", "открой тотал коммандер", "открой тотал командир", "открой Тотал Командер", "запусти totalcmd", "открой тоталкоммандер"] {
        assert_eq!(id_of(t), Some("totalcmd"), "{}", t);
    }
}

#[test]
fn common_programs_notepad_browsers_explorer_and_office() {
    let cases = [
        ("открой блокнот", "notepad"), ("открой notepad", "notepad"), ("запусти блокнот", "notepad"),
        ("открой проводник", "explorer"), ("открой explorer", "explorer"),
        ("открой хром", "chrome"), ("открой google chrome", "chrome"), ("открой гугл хром", "chrome"),
        ("открой эдж", "edge"), ("открой microsoft edge", "edge"),
        ("открой фаерфокс", "firefox"), ("открой firefox", "firefox"), ("открой мозилла", "firefox"),
        ("открой яндекс браузер", "yandex"), ("открой опера", "opera"),
        ("открой пауэрпоинт", "powerpoint"), ("открой powerpoint", "powerpoint"), ("открой повер поинт", "powerpoint"),
        ("открой outlook", "outlook"), ("открой аутлук", "outlook"),
        ("открой калькулятор", "calc"), ("открой диспетчер задач", "taskmgr"), ("открой пейнт", "paint"),
    ];
    for (t, id) in cases {
        assert_eq!(id_of(t), Some(id), "{}", t);
    }
}

#[test]
fn closing_uses_the_same_names() {
    let r = parse_request("закрой ворт").unwrap();
    assert_eq!((r.action, r.app.id), (Action::Close, "word"));
    let r = parse_request("закрой тотал командер").unwrap();
    assert_eq!((r.action, r.app.id), (Action::Close, "totalcmd"));
}

#[test]
fn loose_matches_never_start_another_program() {
    // not program names, or too far from one: no program may be chosen
    for t in ["открой ворон", "открой вордка", "открой хлеб", "открой экзамен", "открой эксперт", "открой хрон", "открой стол", "открой тоталитаризм",
              "открой коммандер", "открой командир", "открой калькуляцию", "открой блок", "открой проводка"] {
        assert_eq!(id_of(t), None, "{} must not start anything", t);
    }
}

#[test]
fn only_a_verb_plus_a_name_counts() {
    // these keep going to the ordinary commands (sites, files, sentences)
    for t in ["открой сайт ворд", "открой файл word", "открой документ ворд на рабочем столе", "что такое ворд", "объясни как работает excel", "найди ворд в интернете", "ворд"] {
        assert_eq!(id_of(t), None, "{}", t);
    }
}

#[test]
fn every_variant_resolves_to_its_own_program() {
    for app in APPS {
        for alias in app.aliases {
            let m = resolve_name(&tokens(alias));
            assert_eq!(m.map(|m| m.app.id), Some(app.id), "alias '{}' of {}", alias, app.id);
        }
    }
}

#[test]
fn different_programs_do_not_sound_alike() {
    // no variant of one program may be within the fuzzy limit of a variant of another one
    for a in APPS {
        for b in APPS {
            if a.id == b.id {
                continue;
            }
            for x in a.aliases {
                for y in b.aliases {
                    let kx: Vec<char> = phrase_key(&tokens(x)).chars().collect();
                    let ky: Vec<char> = phrase_key(&tokens(y)).chars().collect();
                    let limit = allowed_edits(kx.len().min(ky.len()));
                    assert!(edit_distance(&kx, &ky) > limit, "'{}' ({}) is too close to '{}' ({})", x, a.id, y, b.id);
                }
            }
        }
    }
}

#[test]
fn phonetic_keys() {
    assert_eq!(word_key("word"), word_key("ворт"));
    assert_eq!(word_key("ворд"), word_key("ворт"));
    assert_eq!(word_key("excel"), word_key("эксель"));
    assert_eq!(word_key("эксел"), word_key("ексел"));
    assert_eq!(phrase_key(&tokens("total commander")), phrase_key(&tokens("тотал командер")));
}

#[test]
fn find_in_phrase_inside_a_sentence() {
    assert_eq!(find_in_phrase("пожалуйста открой ворт").map(|m| m.app.id), Some("word"));
    assert_eq!(find_in_phrase("закрой тотал командер").map(|m| m.app.id), Some("totalcmd"));
    assert_eq!(find_in_phrase("что угодно").map(|m| m.app.id), None);
}

#[test]
fn env_expansion_and_install_lookup_do_not_invent_paths() {
    std::env::set_var("JARVIS_TEST_DIR", "/tmp/jt");
    assert_eq!(expand_env("%JARVIS_TEST_DIR%\\x.exe"), "/tmp/jt\\x.exe");
    assert_eq!(expand_env("100% sure"), "100% sure");
    // nothing is installed in the build sandbox: no path may be returned
    let tc = APPS.iter().find(|a| a.id == "totalcmd").unwrap();
    assert_eq!(locate(tc), None);
}

#[test]
fn new_programs_vs_code_snipping_tool_and_the_kind_word_before_a_name() {
    let cases = [
        ("открой vs code", "vscode"), ("Открой VS Code", "vscode"), ("запусти visual studio code", "vscode"), ("открой вижуал студио код", "vscode"),
        ("запусти Visual Studio Code", "vscode"), ("открой visual studio", "visualstudio"),
        ("открой ножницы", "snipping"), ("открой фотошоп", "photoshop"), ("запусти Photoshop", "photoshop"),
        ("запусти браузер firefox", "firefox"), ("Запусти браузер Firefox", "firefox"), ("открой яндекс браузер", "yandex"),
        ("открой телеграм", "telegram"), ("запусти дискорд", "discord"), ("открой стим", "steam"), ("открой Total Commander", "totalcmd"),
        ("открой командную строку", "cmd"), ("открой powershell", "powershell"), ("открой панель управления", "control"),
        ("открой параметры windows", "settings"), ("открой блокнот", "notepad"), ("открой калькулятор", "calc"), ("открой paint", "paint"),
    ];
    for (t, id) in cases {
        assert_eq!(id_of(t), Some(id), "{}", t);
    }
    // "командер" alone is not a name of the fixed list: the index of installed programs decides (and asks when several fit)
    assert_eq!(id_of("открой commander"), None);
    assert_eq!(split_request("запусти браузер Firefox").unwrap().1, ["firefox"]);
    assert_eq!(split_request("открой программу Total Commander").unwrap().1, ["total", "commander"]);
}
