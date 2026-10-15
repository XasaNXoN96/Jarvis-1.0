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
    assert!(icons.iter().any(|l| (l.starts_with("Name: \"{group}\\Jarvis\"") || l.starts_with("Name: \"{group}\\{#EditionName}\"")) && !l.contains("--text")));
}

// Every file with a secret that the program writes into its data folder is removed by the uninstaller whatever the user
// answers about the rest of the data. The names are the ones the program really uses (its own sources are read for them).
#[test]
fn the_uninstaller_removes_every_secret_file_the_program_writes() {
    let (_, text) = script();
    let sources = [include_str!("ui_auth.rs"), include_str!("remote.rs"), include_str!("telegram.rs")];
    for file in ["ui_token", "remote_token", "telegram.json"] {
        assert!(sources.iter().any(|s| s.contains(&format!("\"{}\"", file))), "{} is not a file of the program any more", file);
        assert!(text.contains(&format!("DeleteFile(dir + '\\{}')", file)), "the uninstaller keeps {}", file);
    }
}

// The program folder gets its own rights after the files are copied: no inheritance from a wide parent, SYSTEM and
// Administrators and the owner only, other users read and run at most. (The real rights of an installed folder are read back
// by the CI with Get-Acl; this keeps the script from drifting.)
#[test]
fn the_installer_closes_the_program_folder_to_other_users() {
    let (_, text) = script();
    let body = text.split("procedure HardenAppFolder;").nth(1).expect("HardenAppFolder").split("procedure CurStepChanged").next().unwrap();
    assert!(body.contains("/inheritance:r"), "the rights of the parent must be cut");
    for sid in ["*S-1-5-18:(OI)(CI)F", "*S-1-5-32-544:(OI)(CI)F", "(OI)(CI)M"] {
        assert!(body.contains(sid), "{} is missing", sid);
    }
    assert!(body.contains("*S-1-5-32-545:(OI)(CI)RX"), "other users may read and run outside the profile");
    // nothing that lets Users, Authenticated Users or Everyone change files
    for bad in ["S-1-1-0", "S-1-5-11", "S-1-5-32-545:(OI)(CI)M", "S-1-5-32-545:(OI)(CI)F", "S-1-5-32-545:(OI)(CI)W"] {
        assert!(!body.contains(bad), "{} would widen the rights", bad);
    }
    // the grants come first: the inheritance is cut only after they succeeded
    assert!(body.find("/grant:r").unwrap() < body.find("/inheritance:r").unwrap(), "the order of the two icacls calls");
    let step = text.split("procedure CurStepChanged").nth(1).expect("CurStepChanged");
    assert!(step.contains("ssPostInstall") && step.contains("HardenAppFolder;"), "the rights are set after the files are copied");
}

// An unknown Inno constant is a RUNTIME error (the installer still ends with code 0 in a silent run, and the rights were never
// set: the CI caught exactly that). Every constant used in the script must be one Inno Setup knows.
#[test]
fn every_inno_constant_in_the_script_is_a_known_one() {
    let (_, text) = script();
    let known = ["app", "sys", "win", "tmp", "localappdata", "userappdata", "userdocs", "userdesktop", "userstartup", "autopf", "group", "cm", "src", "pf", "commonappdata", "computername", "username", "uninstallexe", "srcexe", "sd", "cf", "fonts", "autodesktop", "userprograms", "userstartmenu", "autoprograms", "usercf", "userpf", "autocf", "autoappdata", "autodocs", "commondocs", "dao", "language", "log", "wizardhwnd", "dotnet20", "dotnet40", "dotnet20", "syswow64", "sys32", "sysnative", "pf32", "pf64", "cf32", "cf64", "userfavorites", "usersavedgames", "usertemplates", "commontemplates", "commonstartmenu", "commonprograms", "commonstartup", "commondesktop", "commonfavorites", "commonpf", "commoncf", "commonpf32", "commonpf64", "commoncf32", "commoncf64", "usercf", "autostartmenu", "autostartup", "autotemplates", "autofavorites", "drive", "param", "ini", "reg", "code", "special", "uninstallexe", "userprograms"];
    for chunk in text.split('{').skip(1) {
        let Some(end) = chunk.find('}') else { continue };
        let name = &chunk[..end];
        // (only simple words: «{app}», «{sys}»; texts of the messages and the Pascal code have braces of other kinds)
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) || name.starts_with("cm") {
            continue;
        }
        assert!(known.contains(&name), "«{{{}}}» is not an Inno constant (a runtime error in the installer)", name);
    }
}


// The window of Jarvis (Tao: OleInitialize, WebView2) needs a single-threaded COM thread of its own. The main thread is
// multithreaded COM after the microphone library, and Tao panicked there with RPC_E_CHANGED_MODE (exit code 101, on a
// computer with a microphone). The program is built only on Windows, so what is checked here is the source: the
// window loop must not return to the main thread, and Tao must be told it is not the main thread. (The behaviour itself
// is checked by the CI on Windows: "WINDOW WITH MULTITHREADED COM ON THE MAIN THREAD".)
#[cfg(test)]
mod window_thread {
    fn app_source(file: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../jarvis-app/src").join(file);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {}", path.display(), e))
    }

    #[test]
    fn the_window_loop_runs_on_its_own_thread_and_a_panic_is_not_hidden() {
        let main = app_source("main.rs");
        let call = main.find("tray::init_blocking(settings)").expect("the window loop is started in main.rs");
        let before = &main[..call];
        let block_start = before.rfind("#[cfg(target_os = \"windows\")]").expect("a Windows block");
        let block = &main[block_start..];
        assert!(block.contains("std::thread::Builder::new()") && block.contains(".spawn("), "the window loop must run on a thread of its own");
        assert!(block.contains("resume_unwind"), "a panic of the window thread must reach the process, not be swallowed");
        // exactly one call for Windows (inside the thread) and one for the other systems
        assert_eq!(main.matches("tray::init_blocking(settings)").count(), 2);
    }

    #[test]
    fn tao_is_told_the_event_loop_is_not_on_the_main_thread() {
        let window = app_source("window.rs");
        assert!(window.contains("with_any_thread(true)"), "without it Tao refuses an event loop outside the main thread");
        assert!(window.contains("EventLoopBuilderExtWindows"));
    }

    #[test]
    fn the_com_mode_of_the_threads_is_written_to_the_log() {
        let main = app_source("main.rs");
        for stage in ["COM of the main thread at start", "COM of the main thread after the microphone", "COM of the window thread"] {
            assert!(main.contains(stage), "{} is missing from the log lines", stage);
        }
    }

    // the test seam of the CI must stay off unless asked for
    #[test]
    fn the_com_test_seam_needs_its_variable() {
        let main = app_source("main.rs");
        let seam = main.find("fn test_com_mta_main()").expect("the seam");
        assert!(main[seam..seam + 400].contains("JARVIS_TEST_MTA_MAIN"));
    }
}
