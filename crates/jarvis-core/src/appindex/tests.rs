use super::*;

fn app(name: &str, launch: &str) -> IndexedApp {
    IndexedApp { name: name.into(), launch: launch.into(), source: "start_menu".into(), exe: String::new() }
}

fn w(s: &str) -> Vec<String> {
    tokens(s)
}

fn installed() -> Vec<IndexedApp> {
    vec![
        app("Visual Studio Code", "C:\\SM\\Visual Studio Code.lnk"),
        app("Visual Studio 2022", "C:\\SM\\Visual Studio 2022.lnk"),
        app("Telegram Desktop", "C:\\SM\\Telegram Desktop.lnk"),
        app("Total Commander 64-bit", "C:\\SM\\Total Commander 64-bit.lnk"),
        app("Adobe Photoshop 2024", "C:\\SM\\Adobe Photoshop 2024.lnk"),
        app("Mozilla Firefox", "C:\\SM\\Firefox.lnk"),
        app("7-Zip File Manager", "C:\\SM\\7-Zip File Manager.lnk"),
        app("Steam", "C:\\SM\\Steam.lnk"),
    ]
}

fn one(list: &[IndexedApp], q: &str) -> String {
    match find_in(list, &w(q)) {
        Found::One(a) => a.name,
        other => panic!("«{}»: {:?}", q, other),
    }
}

#[test]
fn a_program_is_found_by_its_full_short_and_spoken_name() {
    let list = installed();
    assert_eq!(one(&list, "Visual Studio Code"), "Visual Studio Code");
    assert_eq!(one(&list, "visual studio code"), "Visual Studio Code");
    assert_eq!(one(&list, "вижуал студио код"), "Visual Studio Code", "heard in Russian");
    assert_eq!(one(&list, "телеграм"), "Telegram Desktop");
    assert_eq!(one(&list, "Телеграмм"), "Telegram Desktop");
    assert_eq!(one(&list, "фотошоп"), "Adobe Photoshop 2024");
    assert_eq!(one(&list, "Photoshop"), "Adobe Photoshop 2024");
    assert_eq!(one(&list, "firefox"), "Mozilla Firefox");
    assert_eq!(one(&list, "фаерфокс"), "Mozilla Firefox");
    assert_eq!(one(&list, "тотал командер"), "Total Commander 64-bit", "the edition is not said");
    assert_eq!(one(&list, "commander"), "Total Commander 64-bit", "only one commander installed: start it");
    assert_eq!(one(&list, "стим"), "Steam");
}

#[test]
fn an_ambiguous_name_never_starts_a_random_program() {
    // the list of the question «какую программу открыть» is global: the scripts' tests use it too, one at a time
    let _g = crate::session::TEST_LOCK.lock();
    let mut list = installed();
    list.push(app("Double Commander", "C:\\SM\\Double Commander.lnk"));
    match find_in(&list, &w("командер")) {
        Found::Many(v) => {
            let names: Vec<_> = v.iter().map(|a| a.name.as_str()).collect();
            assert!(names.contains(&"Total Commander 64-bit") && names.contains(&"Double Commander"), "{:?}", names);
        }
        other => panic!("{:?}", other),
    }
    // "Visual Studio" is exactly the name of Visual Studio 2022 (the year is not said); Code is another program
    assert_eq!(one(&list, "visual studio"), "Visual Studio 2022");
    // the answer picks one
    set_pending(match find_in(&list, &w("commander")) { Found::Many(v) => v, _ => unreachable!() });
    let first = pending()[0].name.clone();
    assert_eq!(choose("первый").unwrap().name, first);
    assert_eq!(choose("2").unwrap().name, pending()[1].name);
    assert_eq!(choose("дабл").map(|a| a.name), None, "a word that fits nothing chooses nothing");
    assert_eq!(choose("double").unwrap().name, "Double Commander");
    assert!(choose("девятый").is_none());
    // a number inside another command is not a choice
    assert!(choose("поставь таймер на 2 минуты").is_none());
    assert!(choose("громкость двадцать").is_none());
    assert!(choose("пятница").is_none());
    assert_eq!(choose("номер два").unwrap().name, pending()[1].name);
    set_pending(vec![app("7-Zip File Manager", "C:\\SM\\7z.lnk"), app("WinZip", "C:\\SM\\wz.lnk")]);
    assert_eq!(choose("7 zip").unwrap().name, "7-Zip File Manager", "a name with a digit is a name");
    assert_eq!(choose("винзип").unwrap().name, "WinZip");
    set_pending(Vec::new());
    assert_eq!(find_in(&list, &w("ворд")), Found::Nothing);
    assert_eq!(find_in(&list, &w("vs")), Found::Nothing, "two letters fit too many names");
    assert_eq!(find_in(&list, &w("")), Found::Nothing);
}

#[test]
fn shortcuts_are_collected_without_junk_and_registered_apps_are_read() {
    let base = std::env::temp_dir().join(format!("jarvis_appidx_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let sm = base.join("Start Menu/Programs");
    std::fs::create_dir_all(sm.join("Total Commander")).unwrap();
    std::fs::create_dir_all(sm.join("Игры/Глубоко/Ещё/Очень/Глубоко")).unwrap();
    std::fs::write(sm.join("Total Commander/Total Commander 64-bit.lnk"), b"x").unwrap();
    std::fs::write(sm.join("Total Commander/Uninstall Total Commander.lnk"), b"x").unwrap();
    std::fs::write(sm.join("Total Commander/Справка.lnk"), b"x").unwrap();
    std::fs::write(sm.join("Телеграм Десктоп.lnk"), b"x").unwrap();
    std::fs::write(sm.join("readme.txt"), b"x").unwrap();
    std::fs::write(sm.join("Игры/Глубоко/Ещё/Очень/Глубоко/Too Deep.lnk"), b"x").unwrap();
    let desk = base.join("Desktop");
    std::fs::create_dir_all(desk.join("folder")).unwrap();
    std::fs::write(desk.join("My Tool.exe"), b"x").unwrap();
    std::fs::write(desk.join("folder/Hidden.lnk"), b"x").unwrap();
    let found = scan_dirs(&[sm.clone()], &[desk.clone()]);
    let names: Vec<_> = found.iter().map(|a| a.name.as_str()).collect();
    assert!(names.contains(&"Total Commander 64-bit") && names.contains(&"Телеграм Десктоп") && names.contains(&"My Tool"), "{:?}", names);
    assert!(!names.iter().any(|n| n.contains("Uninstall") || *n == "Справка" || *n == "readme" || *n == "Hidden" || *n == "Too Deep"), "{:?}", names);
    assert_eq!(found.iter().find(|a| a.name == "My Tool").unwrap().exe, "my tool.exe");
    assert!(found.iter().all(|a| launchable(a)));

    let json = r#"{"start":[{"Name":"Calculator","AppID":"Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"},
        {"Name":"Visual Studio Code","AppID":"{6D809377-6AF0-444B-8957-A3773F02200E}\\Microsoft VS Code\\Code.exe"},
        {"Name":"Сайт","AppID":"https://example.com"},{"Name":"Uninstall X","AppID":"X!App"}],
        "paths":{"exe":"WINWORD.EXE","path":"C:\\Program Files\\Microsoft Office\\root\\Office16\\WINWORD.EXE"}}"#;
    let reg = parse_registered(json);
    let names: Vec<_> = reg.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["Calculator", "Visual Studio Code", "winword"]);
    assert_eq!(reg[0].launch, "shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App");
    assert_eq!(reg[1].exe, "code.exe");
    assert_eq!(reg[2].exe, "winword.exe");
    assert!(parse_registered("not json").is_empty(), "a broken answer gives nothing, not garbage");
    assert!(try_parse_registered("{\"start\":{\"value\":5}}").is_err(), "another shape is reported, not taken for an empty program");
    let wrapped = try_parse_registered(r#"{"start":{"value":[{"Name":"Paint","AppID":"Microsoft.Paint!App"}],"Count":1},"paths":null}"#).unwrap();
    assert_eq!(wrapped.len(), 1, "the {{value, Count}} form of PowerShell 5.1");
    assert!(try_parse_registered(r#"{"start":null,"paths":[]}"#).unwrap().is_empty());
    assert_eq!(crate::psrun::encoded_command("ab"), "YQBiAA==", "UTF-16LE base64 for -EncodedCommand");
    assert!(parse_registered("\u{feff}{\"start\":{\"Name\":\"Paint\",\"AppID\":\"Microsoft.Paint!App\"}}").len() == 1, "one element without a list");

    // the same program from two places is kept once, the Start menu first, the program file name is kept
    let merged = merge(vec![found, reg, vec![IndexedApp { name: "Total Commander 64-bit".into(), launch: "C:\\tc\\TOTALCMD64.EXE".into(), source: "app_paths".into(), exe: "totalcmd64.exe".into() }]]);
    let tc: Vec<_> = merged.iter().filter(|a| a.name.starts_with("Total Commander")).collect();
    assert_eq!(tc.len(), 1);
    assert_eq!(tc[0].source, "start_menu");
    assert_eq!(tc[0].exe, "totalcmd64.exe");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn only_entries_of_the_index_that_exist_are_started() {
    let _g = TEST_LOCK.lock();
    let base = std::env::temp_dir().join(format!("jarvis_appidx_launch_{}", std::process::id()));
    std::fs::create_dir_all(&base).unwrap();
    let real = base.join("Real App.lnk");
    std::fs::write(&real, b"x").unwrap();
    let ok = app("Real App", &real.to_string_lossy());
    let gone = app("Gone App", &base.join("Gone.lnk").to_string_lossy());
    set_for_tests(vec![ok.clone(), gone.clone()]);
    assert!(launch(&ok).is_ok());
    assert!(launch(&gone).unwrap_err().contains("не найден"), "a removed program is reported, not started");
    assert!(launch(&app("Evil", "C:\\Windows\\System32\\cmd.exe")).unwrap_err().contains("нет в списке"), "never a path from outside the index");
    set_for_tests(Vec::new());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn the_program_file_of_an_entry_is_known_only_when_it_really_is_an_exe() {
    let app = |launch: &str, exe: &str| IndexedApp { name: "X".into(), launch: launch.into(), source: "start_menu".into(), exe: exe.into() };
    assert_eq!(exe_name(&app("C:\\Program Files\\Krita\\bin\\krita.exe", "")).as_deref(), Some("krita.exe"));
    assert_eq!(exe_name(&app("C:\\Users\\Сергей\\Desktop\\Total Commander.lnk", "totalcmd64.exe")).as_deref(), Some("totalcmd64.exe"));
    // a shortcut or a registered app without a known file: not checked (no false "did not start")
    assert_eq!(exe_name(&app("C:\\Users\\Сергей\\Desktop\\Krita.lnk", "")), None);
    assert_eq!(exe_name(&app("shell:AppsFolder\\Microsoft.WindowsCalculator!App", "")), None);
}

fn entry(launch: &str, source: &str) -> IndexedApp {
    IndexedApp { name: "x".into(), launch: launch.into(), source: source.into(), exe: String::new() }
}

// the group of a program comes from where it is and what it is, never from its name
#[test]
fn the_group_of_a_program_comes_from_its_place_and_kind() {
    let c = |l: &str, s: &str, ok: bool| category(&entry(l, s), ok);
    // Windows components
    assert_eq!(c("shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App", "registered", true), "system");
    assert_eq!(c("shell:AppsFolder\\MicrosoftWindows.Client.CBS_cw5n1h2txyewy!App", "registered", true), "system");
    assert_eq!(c("C:\\Windows\\System32\\notepad.exe", "app_paths", true), "system");
    assert_eq!(c("C:/ProgramData/Microsoft/Windows/Start Menu/Programs/Windows Accessories/Paint.lnk", "start_menu", true), "system");
    assert_eq!(c("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Windows PowerShell\\Windows PowerShell.lnk", "start_menu", true), "system");
    // installed by the user
    assert_eq!(c("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Total Commander\\Total Commander.lnk", "start_menu", true), "user");
    assert_eq!(c("C:\\Users\\a\\Desktop\\Double Commander.lnk", "desktop", true), "user");
    assert_eq!(c("C:\\Program Files\\Krita (x64)\\bin\\krita.exe", "app_paths", true), "user");
    assert_eq!(c("C:\\Users\\a\\AppData\\Local\\Programs\\Microsoft VS Code\\Code.exe", "app_paths", true), "user");
    // other
    assert_eq!(c("shell:AppsFolder\\SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "registered", true), "other");
    assert_eq!(c("D:\\Soft\\Portable\\tool.exe", "app_paths", true), "other");
    // nothing known: the file is gone, or nothing says what it is
    assert_eq!(c("C:\\gone\\old.lnk", "app_paths", false), "unknown");
    assert_eq!(c("D:\\x\\y.lnk", "weird", true), "unknown");
    // the name does not decide: a program that calls itself "Windows Something" in the Start menu is the user's
    let mut a = entry("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\Windows Helper Pro\\Windows Helper Pro.lnk", "start_menu");
    a.name = "Windows Helper Pro".into();
    assert_eq!(category(&a, true), "user");
    assert_eq!(category_title("system"), "Системные инструменты и компоненты Windows");
    assert_eq!(category_title("unknown"), "Не определено");
}
