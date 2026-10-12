// The installer script (installer/jarvis.iss) is compiled only on Windows (CI). These tests read it and check
// what can be checked without Inno Setup: both languages, the language question, a translation for every text
// of Jarvis, and the last page that starts the normal Jarvis first.

fn script() -> (Vec<u8>, String) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../installer/jarvis.iss");
    let bytes = std::fs::read(&path).expect("installer/jarvis.iss");
    let text = String::from_utf8(bytes[3.min(bytes.len())..].to_vec()).expect("UTF-8");
    (bytes, text)
}

fn section<'a>(text: &'a str, name: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            inside = l.eq_ignore_ascii_case(&format!("[{}]", name));
            continue;
        }
        if inside && !l.is_empty() && !l.starts_with(';') && !l.starts_with('#') {
            out.push(l);
        }
    }
    out
}

#[test]
fn the_script_is_utf8_with_bom_so_russian_texts_are_not_broken() {
    let (bytes, _) = script();
    assert!(bytes.starts_with(&[0xEF, 0xBB, 0xBF]), "without a BOM Inno Setup may read the Russian texts as ANSI");
}

#[test]
fn both_installers_ask_the_language_first_and_know_russian_and_english() {
    let (_, t) = script();
    let setup = section(&t, "Setup").join("\n");
    assert!(setup.contains("ShowLanguageDialog=yes"), "the language is always asked");
    let langs = section(&t, "Languages").join("\n");
    assert!(langs.contains("Name: \"russian\"") && langs.contains("Languages\\Russian.isl"));
    assert!(langs.contains("Name: \"english\"") && langs.contains("Default.isl"));
    let msgs = section(&t, "Messages").join("\n");
    for lang in ["russian", "english"] {
        assert!(msgs.contains(&format!("{}.SelectLanguageTitle=Выберите язык установки / Select Setup Language", lang)));
    }
    // one script for both installers: the Lite one differs only by its name (and its note on the last page)
    assert!(t.contains("OutputBaseFilename={#OutName}"));
    assert!(section(&t, "Files").iter().all(|l| !l.contains("OutName")), "the Lite installer installs the same program files");
}

#[test]
fn every_text_of_jarvis_has_a_russian_and_an_english_version() {
    let (_, t) = script();
    let mut ru = std::collections::BTreeSet::new();
    let mut en = std::collections::BTreeSet::new();
    for l in section(&t, "CustomMessages") {
        if let Some((k, v)) = l.split_once('=') {
            assert!(!v.trim().is_empty(), "{} is empty", k);
            if let Some(name) = k.strip_prefix("russian.") { ru.insert(name.to_string()); }
            if let Some(name) = k.strip_prefix("english.") { en.insert(name.to_string()); }
        }
    }
    assert_eq!(ru, en, "a text without its translation would show the other language");
    // every text used by the script exists
    for used in t.split("{cm:").skip(1).map(|s| s.split('}').next().unwrap()).chain(t.split("CustomMessage('").skip(1).map(|s| s.split('\'').next().unwrap())) {
        assert!(ru.contains(used), "{} is used but not defined", used);
    }
    // the tasks, shortcuts and choices of the last page are translated, none is written in one language only
    for l in section(&t, "Tasks").into_iter().chain(section(&t, "Run")) {
        assert!(l.contains("{cm:"), "untranslated: {}", l);
    }
}

#[test]
fn the_last_page_starts_the_normal_jarvis_first_and_the_text_mode_only_on_request() {
    let (_, t) = script();
    let run = section(&t, "Run");
    assert_eq!(run.len(), 2, "{:?}", run);
    let normal = run[0];
    assert!(normal.contains("jarvis-app.exe") && !normal.contains("--text"), "the first choice is the normal window with voice");
    assert!(normal.contains("postinstall") && !normal.contains("unchecked"), "and it is checked");
    let text = run[1];
    assert!(text.contains("--text") && text.contains("unchecked"), "the text mode is a second, unchecked choice");
    // the two choices exclude each other (code of the last page), and the longer text does not cover them
    assert!(t.contains("RunList.OnClickCheck := @RunListClickCheck") && t.contains("WizardForm.RunList.Checked[1 - i] := False"));
    assert!(t.contains("AdjustLabelHeight(WizardForm.FinishedLabel)"));
    // the normal shortcut starts the window, not the text mode
    let icons = section(&t, "Icons");
    assert!(icons.iter().any(|l| l.starts_with("Name: \"{group}\\Jarvis\"") && !l.contains("--text")));
}
