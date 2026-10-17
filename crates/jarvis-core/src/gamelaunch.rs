//! Starting a game of the library from the Games page («Запустить») and checking a program that the user named.
//!
//! Only a game the user chose by pressing the button is started: never at the opening of the page, never after a scan, never after adding.
//! A game is started in one of two ways, and nothing is put into a command line of a shell:
//!   * a Steam game that the scan found is started through Steam itself (`steam://rungameid/<number>`; the number is checked to be digits);
//!   * any other game is started from its program file: the path is checked right before the start (it exists, it is an `.exe`, it is the
//!     program of this game), and Windows starts it (ShellExecute, which also asks for the rights when the game needs them).
//! A game whose program was only guessed from the files of a folder (not named by the user and not by a manifest of a store) is not
//! started: the user is asked to choose the file. There is no way around an anti-cheat here and none is looked for: this module starts a
//! program like a double click does, nothing more.

use crate::gamelist::{self, Entry};
use crate::gamemode::refuse_reason;
use std::path::{Path, PathBuf};

pub const NOT_FOUND: &str = "Файл игры не найден. Выберите новый путь к игре";

// a program the user named (typed or chosen): checked, with the name to show
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Valid {
    pub path: String,
    pub exe: String,
    pub name: String,
}

// how an entry can be started
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    // a Steam game: the number of the application
    Steam(String),
    Exe { path: PathBuf, dir: PathBuf, args: Vec<String> },
}

// «steam» (through Steam), «path» (its program file), «unverified» (the program was only guessed), «nopath» (only the name is known)
pub fn kind_of(e: &Entry) -> &'static str {
    if e.verified && !e.path.is_empty() {
        "path"
    } else if steam_app(e).is_some() {
        "steam"
    } else if e.path.is_empty() {
        "nopath"
    } else {
        "unverified"
    }
}

fn steam_app(e: &Entry) -> Option<String> {
    let id = e.store_id.strip_prefix("steam:")?;
    (!id.is_empty() && id.len() <= 12 && id.chars().all(|c| c.is_ascii_digit())).then(|| id.to_string())
}

// the folders that say nothing about the game: the name is then taken from the folder above
const GENERIC_FILES: &[&str] = &["game", "start", "launcher", "run", "play", "main", "app", "client", "bin"];
const GENERIC_DIRS: &[&str] = &["bin", "bin32", "bin64", "binaries", "win32", "win64", "x64", "x86", "game", "release", "windows", "windowsnoeditor", "retail"];

// a name for the library from the path of the program: the name of the file, or of the folder when the file says nothing
pub fn guess_name(path: &str) -> String {
    let norm = path.trim().trim_matches('"').replace('/', "\\");
    let parts: Vec<&str> = norm.split('\\').filter(|p| !p.is_empty()).collect();
    let Some(file) = parts.last() else { return String::new() };
    let stem = file.rsplit_once('.').map(|(s, _)| s).unwrap_or(file);
    let pretty = |s: &str| s.replace('_', " ").trim().to_string();
    if !GENERIC_FILES.contains(&stem.to_lowercase().as_str()) {
        return pretty(stem);
    }
    // up the folders, over the usual names of the folders of programs (but never up to a drive letter)
    for d in parts[..parts.len() - 1].iter().rev() {
        if d.ends_with(':') {
            break;
        }
        if !GENERIC_DIRS.contains(&d.to_lowercase().as_str()) {
            return pretty(d);
        }
    }
    pretty(stem)
}

// a path the user named -> the program, or the reason (in words for the page)
pub fn check_path(raw: &str) -> Result<Valid, String> {
    let path = raw.trim().trim_matches('"').trim().to_string();
    if path.is_empty() {
        return Err("Укажите путь к программе игры.".into());
    }
    if path.len() > 520 || path.chars().any(|c| c.is_control()) {
        return Err("Путь выглядит неправильно.".into());
    }
    let p = Path::new(&path);
    if !p.is_absolute() {
        return Err("Нужен полный путь к файлу, например D:\\Games\\Игра\\game.exe (кнопка «Обзор…» подставит его сама).".into());
    }
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("В пути не должно быть «..».".into());
    }
    let exe = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    if !exe.ends_with(".exe") {
        return Err("Нужен файл программы с расширением .exe. Документы, картинки и архивы игрой не считаются.".into());
    }
    if let Some(why) = refuse_reason(&exe) {
        return Err(why);
    }
    if !p.is_file() {
        return Err(NOT_FOUND.into());
    }
    let name = guess_name(&path);
    let name = if name.is_empty() { exe_stem(p) } else { name };
    Ok(Valid { path, exe, name })
}

fn exe_stem(p: &Path) -> String {
    p.file_stem().and_then(|s| s.to_str()).unwrap_or("").to_string()
}

// what to start for an entry, checked now (the file may have been moved since the scan)
pub fn plan(e: &Entry) -> Result<Target, String> {
    match kind_of(e) {
        "nopath" => Err("Для этой игры не указан файл запуска. Выберите файл игры".into()),
        "unverified" => Err("Не удалось надёжно определить файл запуска этой игры. Выберите файл игры".into()),
        "steam" => Ok(Target::Steam(steam_app(e).unwrap_or_default())),
        _ => {
            let v = check_path(&e.path).map_err(|why| if why == NOT_FOUND { why } else { format!("{} Выберите новый путь к игре", why.trim_end_matches('.')) })?;
            // the file must still be the program of this game
            if v.exe != e.exe {
                return Err("Файл по этому пути — другая программа, не эта игра. Выберите новый путь к игре".into());
            }
            let path = PathBuf::from(&v.path);
            let dir = path.parent().map(|d| d.to_path_buf()).unwrap_or_default();
            Ok(Target::Exe { path, dir, args: e.args.clone() })
        }
    }
}

// one argument of a command line as Windows reads it (CommandLineToArgvW): spaces and quotes are protected, nothing else is added
pub fn quote_arg(a: &str) -> String {
    if !a.is_empty() && !a.contains([' ', '\t', '"']) {
        return a.to_string();
    }
    let mut out = String::from("\"");
    let mut slashes = 0;
    for c in a.chars() {
        match c {
            '\\' => slashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(slashes * 2 + 1));
                out.push('"');
                slashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(slashes));
                out.push(c);
                slashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(slashes * 2));
    out.push('"');
    out
}

#[cfg(target_os = "windows")]
fn wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

// the real start; returns when the process was created (not when the game ends)
#[cfg(target_os = "windows")]
fn os_start(t: &Target) -> Result<(), String> {
    use winapi::um::shellapi::ShellExecuteW;
    use winapi::um::winuser::SW_SHOWNORMAL;
    let (file, params, dir) = match t {
        Target::Steam(app) => (format!("steam://rungameid/{}", app), None, None),
        Target::Exe { path, dir, args } => (path.to_string_lossy().to_string(), if args.is_empty() { None } else { Some(args.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ")) }, Some(dir.to_string_lossy().to_string())),
    };
    let (wfile, wverb) = (wide(&file), wide("open"));
    let wparams = params.as_deref().map(wide);
    let wdir = dir.as_deref().map(wide);
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wverb.as_ptr(),
            wfile.as_ptr(),
            wparams.as_ref().map(|p| p.as_ptr()).unwrap_or(std::ptr::null()),
            wdir.as_ref().map(|d| d.as_ptr()).unwrap_or(std::ptr::null()),
            SW_SHOWNORMAL,
        ) as isize
    };
    if code > 32 {
        return Ok(());
    }
    Err(match (code, t) {
        (2 | 3, _) => NOT_FOUND.to_string(),
        (5, _) => "Windows отказала в запуске (нет доступа или запрос прав отклонён).".to_string(),
        (31, Target::Steam(_)) => "Steam не найден на этом компьютере: запустите игру из Steam или выберите файл игры.".to_string(),
        (c, _) => format!("Windows не смогла запустить игру (код {}).", c),
    })
}

// not Windows (the tests): the program is started directly, without a shell, and reaped in the background
#[cfg(not(target_os = "windows"))]
fn os_start(t: &Target) -> Result<(), String> {
    match t {
        Target::Steam(_) => Err("Запуск через Steam работает только в Windows.".into()),
        Target::Exe { path, dir, args } => {
            let mut child = std::process::Command::new(path)
                .current_dir(dir)
                .args(args)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("Не удалось запустить игру: {}.", e))?;
            std::thread::spawn(move || {
                let _ = child.wait();
            });
            Ok(())
        }
    }
}

// the button «Запустить»: the entry by its id, the checks, the start; the answer for the page
pub fn launch(id: &str) -> Result<String, String> {
    launch_with(id, os_start)
}

// (the way of starting is given, so that the checks can be tested without starting anything)
pub fn launch_with(id: &str, start: impl FnOnce(&Target) -> Result<(), String>) -> Result<String, String> {
    let Some(e) = gamelist::get(id) else { return Err("Игры нет в библиотеке (обновите страницу).".into()) };
    let target = plan(&e)?;
    start(&target)?;
    gamelist::touch_launch(id);
    crate::gamemode::note_event(&format!("игра запущена кнопкой: {}", e.exe));
    Ok(format!("Запускаю «{}»…", e.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamemode::Genre;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jarvis-launch-{}-{}", tag, std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn entry(path: &Path, verified: bool) -> Entry {
        let exe = path.file_name().unwrap().to_string_lossy().to_lowercase();
        Entry { path: path.to_string_lossy().to_string(), verified, ..Entry::basic(&exe, "Тест", Genre::Action, "manual") }
    }

    #[test]
    fn names_are_taken_from_the_file_or_from_the_folder_when_the_file_says_nothing() {
        assert_eq!(guess_name("D:\\Games\\Fallout 3\\Fallout3.exe"), "Fallout3");
        assert_eq!(guess_name("D:\\Игры\\Мор Утопия\\game.exe"), "Мор Утопия");
        assert_eq!(guess_name("D:\\Games\\Cool_Racer\\bin\\x64\\launcher.exe"), "Cool Racer");
        assert_eq!(guess_name("C:\\game.exe"), "game", "never up to the drive");
        assert_eq!(guess_name("\"E:/Steam/steamapps/common/Hades/Hades.exe\""), "Hades");
        assert_eq!(guess_name(""), "");
    }

    #[test]
    fn only_an_existing_exe_of_a_game_with_a_full_path_is_accepted() {
        let d = temp_dir("check");
        let ok = d.join("игра с пробелами").join("Моя Игра.exe");
        std::fs::create_dir_all(ok.parent().unwrap()).unwrap();
        std::fs::write(&ok, b"MZ").unwrap();
        let v = check_path(&format!("\"{}\"", ok.display())).unwrap();
        assert_eq!((v.exe.as_str(), v.name.as_str()), ("моя игра.exe", "Моя Игра"));
        assert!(!v.path.contains('"'));
        // not accepted: a missing file, documents and pictures, a folder, a relative path, a way up, empty text, a browser, an online game
        assert_eq!(check_path(&d.join("нет.exe").to_string_lossy()).unwrap_err(), NOT_FOUND);
        for (name, body) in [("doc.docx", "x"), ("pic.png", "x"), ("a.zip", "x"), ("run.bat", "x"), ("readme.txt", "x")] {
            let f = d.join(name);
            std::fs::write(&f, body).unwrap();
            assert!(check_path(&f.to_string_lossy()).unwrap_err().contains(".exe"), "{}", name);
        }
        assert!(check_path(&d.to_string_lossy()).is_err());
        assert!(check_path("game.exe").unwrap_err().contains("полный путь"));
        assert!(check_path(&format!("{}/../x.exe", d.display())).unwrap_err().contains(".."));
        assert!(check_path("  ").is_err());
        for bad in ["chrome.exe", "valorant.exe", "cmd.exe"] {
            let f = d.join(bad);
            std::fs::write(&f, b"MZ").unwrap();
            assert!(check_path(&f.to_string_lossy()).is_err(), "{}", bad);
        }
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_game_is_planned_only_when_the_way_to_start_it_is_sure() {
        let d = temp_dir("plan");
        let exe = d.join("game.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        // a verified path
        assert!(matches!(plan(&entry(&exe, true)).unwrap(), Target::Exe { .. }));
        assert_eq!(kind_of(&entry(&exe, true)), "path");
        // a guessed path (a scan of a folder): not started
        assert_eq!(kind_of(&entry(&exe, false)), "unverified");
        assert!(plan(&entry(&exe, false)).unwrap_err().contains("надёжно"));
        // only a name
        let bare = Entry::basic("game.exe", "G", Genre::Action, "voice");
        assert_eq!(kind_of(&bare), "nopath");
        assert!(plan(&bare).unwrap_err().contains("не указан файл"));
        // a Steam game is started through Steam, by a number; a store id that is not a number is not trusted
        let steam = |id: &str| Entry { store_id: id.into(), path: exe.to_string_lossy().to_string(), ..Entry::basic("game.exe", "S", Genre::Action, "scan") };
        assert_eq!(plan(&steam("steam:22370")).unwrap(), Target::Steam("22370".into()));
        for bad in ["steam:22370 & calc", "steam:", "steam:../x", "steam:1234567890123", "epic:22370"] {
            assert_ne!(kind_of(&steam(bad)), "steam", "{}", bad);
            assert!(plan(&steam(bad)).is_err(), "{}", bad);
        }
        // the file is gone / is another program / became a folder
        let gone = entry(&d.join("moved.exe"), true);
        assert_eq!(plan(&gone).unwrap_err(), NOT_FOUND);
        let other = Entry { exe: "other.exe".into(), ..entry(&exe, true) };
        assert!(plan(&other).unwrap_err().contains("другая программа"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_arguments_of_a_launch_are_quoted_one_by_one() {
        assert_eq!(quote_arg("-windowed"), "-windowed");
        assert_eq!(quote_arg("a b"), "\"a b\"");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_arg("C:\\Games\\x y\\"), "\"C:\\Games\\x y\\\\\"");
        assert_eq!(quote_arg("a&calc"), "a&calc", "no shell reads it: the text stays one argument");
    }

    #[test]
    fn the_launch_asks_the_start_only_after_the_checks_and_counts_it() {
        gamelist::test_reset();
        let d = temp_dir("launch");
        let exe = d.join("My Game.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        let (_, id) = gamelist::add_entry(Entry { path: exe.to_string_lossy().to_string(), verified: true, ..Entry::basic("my game.exe", "My Game", Genre::Action, "manual") }, false).unwrap();
        // the start is not asked when a check fails
        assert!(launch_with("нет такого", |_| panic!("must not start")).unwrap_err().contains("нет в библиотеке"));
        let mut asked = None;
        assert_eq!(launch_with(&id, |t| { asked = Some(t.clone()); Ok(()) }).unwrap(), "Запускаю «My Game»…");
        assert!(matches!(asked, Some(Target::Exe { ref path, .. }) if *path == exe));
        let g = gamelist::get(&id).unwrap();
        assert_eq!(g.launches, 1);
        assert!(g.last_launched > 0);
        // a failure of the start is told as it is and is not counted
        assert_eq!(launch_with(&id, |_| Err("Windows отказала".into())).unwrap_err(), "Windows отказала");
        assert_eq!(gamelist::get(&id).unwrap().launches, 1);
        // the file is moved: the message of the task, nothing is started
        std::fs::remove_file(&exe).unwrap();
        assert_eq!(launch_with(&id, |_| panic!("must not start")).unwrap_err(), NOT_FOUND);
        std::fs::remove_dir_all(&d).ok();
        gamelist::test_reset();
    }

    // a real start of a real program file (a small script) with a path that has spaces and Cyrillic and an argument: the process runs
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_real_program_is_started_without_a_shell_and_the_game_files_stay() {
        use std::os::unix::fs::PermissionsExt;
        gamelist::test_reset();
        let d = temp_dir("real");
        let dir = d.join("Моя папка с пробелами");
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("игра.exe");
        let marker = d.join("отметка.txt");
        std::fs::write(&exe, format!("#!/bin/sh\necho \"$1\" > '{}'\n", marker.display())).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        let (_, id) = gamelist::add_entry(Entry { path: exe.to_string_lossy().to_string(), verified: true, args: vec!["один; два && три".into()], ..Entry::basic("игра.exe", "Игра", Genre::Action, "manual") }, false).unwrap();
        assert_eq!(launch(&id).unwrap(), "Запускаю «Игра»…");
        // the process was really created: it wrote the argument as one piece of text (a shell would have split it)
        for _ in 0..100 {
            if marker.exists() && std::fs::read_to_string(&marker).map(|t| !t.is_empty()).unwrap_or(false) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "один; два && три");
        assert!(exe.exists(), "starting never touches the file");
        std::fs::remove_dir_all(&d).ok();
        gamelist::test_reset();
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_program_that_cannot_run_is_an_error_on_the_page() {
        gamelist::test_reset();
        let d = temp_dir("norun");
        let exe = d.join("broken.exe");
        std::fs::write(&exe, b"not a program").unwrap(); // no rights to run it
        let (_, id) = gamelist::add_entry(Entry { path: exe.to_string_lossy().to_string(), verified: true, ..Entry::basic("broken.exe", "Broken", Genre::Action, "manual") }, false).unwrap();
        assert!(launch(&id).unwrap_err().starts_with("Не удалось запустить игру"));
        assert_eq!(gamelist::get(&id).unwrap().launches, 0);
        std::fs::remove_dir_all(&d).ok();
        gamelist::test_reset();
    }
}
