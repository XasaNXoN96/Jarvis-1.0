//! «Мои программы»: the library of programs in the style of the library of games. Besides the programs found on the computer, the user can
//! add their own shortcut (a name and a path of a program), hide a card they do not want to see, and sort by name or by recent launch.
//! «Убрать» hides only the CARD: the program itself is never uninstalled or deleted. Launches are counted here only to sort by «недавние».

use crate::store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

const FILE: &str = "program_library.json";
pub const MAX_SHORTCUTS: usize = 200;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Shortcut {
    pub id: String,
    pub name: String,
    pub path: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Launch {
    pub count: u32,
    pub last: u64,
}

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct Lib {
    #[serde(default)]
    pub shortcuts: Vec<Shortcut>,
    // names of the cards that are hidden (lower case)
    #[serde(default)]
    pub hidden: Vec<String>,
    #[serde(default)]
    pub launches: BTreeMap<String, Launch>,
}

pub fn load() -> Lib {
    store::read::<Lib>(FILE).0
}
fn save(l: &Lib) -> Result<(), String> {
    store::write(FILE, l)
}

fn key(name: &str) -> String {
    name.trim().to_lowercase()
}

// a program file the user may add: an existing .exe / .lnk / .bat / .cmd / .url, not a folder, not a script of unknown kind
pub fn check_path(p: &str) -> Result<String, String> {
    let path = Path::new(p.trim());
    if !path.is_absolute() {
        return Err("Нужен полный путь к программе.".into());
    }
    if !path.is_file() {
        return Err("Такого файла нет.".into());
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    if !["exe", "lnk", "bat", "cmd", "url"].contains(&ext.as_str()) {
        return Err("Добавить можно программу (.exe), ярлык (.lnk) или пакетный файл (.bat, .cmd).".into());
    }
    Ok(path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())
}

pub fn add_shortcut(name: &str, path: &str) -> Result<Shortcut, String> {
    let default_name = check_path(path)?;
    let name = if name.trim().is_empty() { default_name } else { name.trim().chars().take(60).collect() };
    let mut l = load();
    if l.shortcuts.len() >= MAX_SHORTCUTS {
        return Err("Слишком много ярлыков.".into());
    }
    if l.shortcuts.iter().any(|s| s.path.eq_ignore_ascii_case(path.trim())) {
        return Err("Этот ярлык уже добавлен.".into());
    }
    let s = Shortcut { id: store::new_id(), name, path: path.trim().to_string() };
    // an added card is shown even if the same name was hidden before
    l.hidden.retain(|h| *h != key(&s.name));
    l.shortcuts.push(s.clone());
    save(&l)?;
    Ok(s)
}

// removes a shortcut of the user (the file stays where it is)
pub fn remove_shortcut(id: &str) -> bool {
    let mut l = load();
    let n = l.shortcuts.len();
    l.shortcuts.retain(|s| s.id != id);
    l.shortcuts.len() != n && save(&l).is_ok()
}

// hides the card of a found program (it is only not shown; «Показать скрытые» brings them back)
pub fn hide(name: &str) -> bool {
    let mut l = load();
    let k = key(name);
    if k.is_empty() || l.hidden.contains(&k) {
        return false;
    }
    l.hidden.push(k);
    save(&l).is_ok()
}

pub fn unhide_all() -> bool {
    let mut l = load();
    l.hidden.clear();
    save(&l).is_ok()
}

pub fn note_launch(name: &str, now: u64) {
    let mut l = load();
    let e = l.launches.entry(key(name)).or_default();
    e.count += 1;
    e.last = now;
    let _ = save(&l);
}

// (the name, its last launch) → sorting helpers for the page
pub fn json() -> serde_json::Value {
    let l = load();
    serde_json::json!({ "shortcuts": l.shortcuts, "hidden": l.hidden, "launches": l.launches })
}

#[cfg(all(feature = "lua", target_os = "windows"))]
fn open(target: &str) -> Result<(), String> {
    crate::lua::shell_open(target)
}
#[cfg(not(all(feature = "lua", target_os = "windows")))]
fn open(_target: &str) -> Result<(), String> {
    Err("запуск программ доступен только в Windows".into())
}

pub fn launch_shortcut(id: &str) -> Result<String, String> {
    let l = load();
    let s = l.shortcuts.iter().find(|s| s.id == id).ok_or("Такого ярлыка нет.")?;
    if !Path::new(&s.path).is_file() {
        return Err(format!("Файл не найден: {}. Возможно, программу перенесли или удалили.", s.path));
    }
    open(&s.path)?;
    note_launch(&s.name, store::now());
    Ok(format!("Запускаю {}", s.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(d: &Path, n: &str) -> String {
        let p = d.join(n);
        std::fs::write(&p, b"MZ").unwrap();
        p.to_string_lossy().to_string()
    }

    #[test]
    fn only_programs_can_be_added_and_not_twice() {
        store::with_temp(|| {
            let d = tempfile::tempdir().unwrap();
            let p = exe(d.path(), "Tool.exe");
            let s = add_shortcut("", &p).unwrap();
            assert_eq!(s.name, "Tool");
            assert!(add_shortcut("x", &p).unwrap_err().contains("уже"));
            assert!(add_shortcut("x", &exe(d.path(), "readme.txt")).is_err());
            assert!(add_shortcut("x", "tool.exe").is_err());
            assert!(add_shortcut("x", &d.path().to_string_lossy()).is_err());
            assert_eq!(load().shortcuts.len(), 1);
        });
    }

    #[test]
    fn removing_a_card_never_touches_the_program() {
        store::with_temp(|| {
            let d = tempfile::tempdir().unwrap();
            let p = exe(d.path(), "A.exe");
            let s = add_shortcut("Моя А", &p).unwrap();
            assert!(remove_shortcut(&s.id));
            assert!(Path::new(&p).exists(), "the file stays");
            assert!(!remove_shortcut(&s.id));
            assert!(hide("Microsoft Word"));
            assert!(!hide("microsoft word"), "once");
            assert!(load().hidden.contains(&"microsoft word".to_string()));
            assert!(unhide_all() && load().hidden.is_empty());
        });
    }

    #[test]
    fn an_added_card_is_shown_even_if_its_name_was_hidden() {
        store::with_temp(|| {
            let d = tempfile::tempdir().unwrap();
            hide("Tool");
            add_shortcut("Tool", &exe(d.path(), "t.exe")).unwrap();
            assert!(load().hidden.is_empty());
        });
    }

    #[test]
    fn launches_are_counted_for_sorting() {
        store::with_temp(|| {
            note_launch("Chrome", 100);
            note_launch("chrome", 200);
            let l = load();
            assert_eq!(l.launches["chrome"], Launch { count: 2, last: 200 });
            assert!(launch_shortcut("nope").is_err());
        });
    }

    #[test]
    fn a_vanished_file_is_reported_not_launched() {
        store::with_temp(|| {
            let d = tempfile::tempdir().unwrap();
            let p = exe(d.path(), "gone.exe");
            let s = add_shortcut("Gone", &p).unwrap();
            std::fs::remove_file(&p).unwrap();
            assert!(launch_shortcut(&s.id).unwrap_err().contains("не найден"));
        });
    }
}
