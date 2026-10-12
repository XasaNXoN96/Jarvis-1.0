// The tasks of the user: one task per line in Documents\Jarvis-tasks.txt. It is the same file the voice commands
// use ("добавь задачу …", "озвучь задачи", "выполнил задачу 2", resources/commands/assistant/tasks.lua), so the
// panel "Задачи" of the window and the voice always see one list, and the user can still edit the file by hand.
// A file that exists but cannot be read is never written over.

use std::path::PathBuf;

use parking_lot::Mutex;

pub const MAX: usize = 30;
pub const MAX_LEN: usize = 200;
const NAME: &str = "Jarvis-tasks.txt";

static LOCK: Mutex<()> = parking_lot::const_mutex(());
#[cfg(test)]
pub static TEST_PATH: Mutex<Option<PathBuf>> = parking_lot::const_mutex(None);
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

// the same place as docs_path() of the Lua prelude: Documents where Windows really keeps it; a file written before
// Documents was moved (to OneDrive or another drive) keeps being used
pub fn path() -> PathBuf {
    #[cfg(test)]
    if let Some(p) = TEST_PATH.lock().clone() {
        return p;
    }
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_else(|_| ".".into());
    let old = PathBuf::from(home).join("Documents").join(NAME);
    match crate::filesearch::known_folder("documents") {
        Some(dir) => {
            let p = dir.join(NAME);
            if !p.exists() && old.exists() {
                old
            } else {
                p
            }
        }
        None => old,
    }
}

// the lines of the text: trimmed, without empty lines and the BOM a text editor may add
pub fn parse(text: &str) -> Vec<String> {
    text.trim_start_matches('\u{feff}').lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect()
}

fn read() -> Result<Vec<String>, String> {
    let p = path();
    match std::fs::read(&p) {
        Ok(bytes) => Ok(parse(&String::from_utf8_lossy(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(_) => Err(format!("Не удалось прочитать файл задач (он открыт в другой программе или нет доступа): {}", p.display())),
    }
}

fn write(list: &[String]) -> Result<(), String> {
    let p = path();
    if let Some(dir) = p.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text: String = list.iter().map(|t| format!("{}\n", t)).collect();
    crate::write_atomic(&p, text).map_err(|_| format!("Не удалось сохранить файл задач: {}", p.display()))
}

// one line of text: no line breaks inside a task, no runs of spaces
pub fn clean(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn list() -> Result<Vec<String>, String> {
    let _g = LOCK.lock();
    read()
}

fn check(text: &str) -> Result<String, String> {
    let t = clean(text);
    if t.is_empty() {
        return Err("Напишите текст задачи".into());
    }
    if t.chars().count() > MAX_LEN {
        return Err(format!("Задача длиннее {} символов", MAX_LEN));
    }
    Ok(t)
}

// the number of tasks after the new one
pub fn add(text: &str) -> Result<usize, String> {
    let t = check(text)?;
    let _g = LOCK.lock();
    let mut list = read()?;
    if list.len() >= MAX {
        return Err(format!("Список полон: {} задач. Удалите выполненные.", MAX));
    }
    list.push(t);
    write(&list)?;
    Ok(list.len())
}

// `index` from 0, as the panel shows the list; `old` is the text the panel saw, so a list changed meanwhile
// (by voice or in the file) is not edited at the wrong line
pub fn edit(index: usize, old: &str, text: &str) -> Result<(), String> {
    let t = check(text)?;
    let _g = LOCK.lock();
    let mut list = read()?;
    match list.get_mut(index) {
        Some(cur) if *cur == old => {
            *cur = t;
            write(&list)
        }
        _ => Err("Список задач изменился. Обновите и попробуйте снова.".into()),
    }
}

pub fn remove(index: usize, old: &str) -> Result<String, String> {
    let _g = LOCK.lock();
    let mut list = read()?;
    match list.get(index) {
        Some(cur) if cur == old => {
            let t = list.remove(index);
            write(&list)?;
            Ok(t)
        }
        _ => Err("Список задач изменился. Обновите и попробуйте снова.".into()),
    }
}

pub fn clear() -> Result<usize, String> {
    let _g = LOCK.lock();
    let n = read()?.len();
    write(&[])?;
    Ok(n)
}

// what "Озвучить задачи" says: the count and the first seven (the whole list is on the screen)
pub fn spoken(list: &[String]) -> String {
    if list.is_empty() {
        return "Задач нет".into();
    }
    let items: Vec<String> = list.iter().take(7).enumerate().map(|(i, t)| format!("{}. {}", i + 1, t)).collect();
    let more = if list.len() > 7 { format!(". И ещё {}", list.len() - 7) } else { String::new() };
    format!("Задач: {}. {}{}", list.len(), items.join(". "), more)
}

pub fn json() -> serde_json::Value {
    match list() {
        Ok(l) => serde_json::json!({ "ok": true, "tasks": l, "max": MAX, "path": path().to_string_lossy() }),
        Err(e) => serde_json::json!({ "ok": false, "tasks": [], "max": MAX, "message": e, "path": path().to_string_lossy() }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_file<F: FnOnce(&PathBuf)>(f: F) {
        let _g = TEST_LOCK.lock();
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("jarvis_tasks_{}_{}", std::process::id(), n));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join(NAME);
        *TEST_PATH.lock() = Some(p.clone());
        f(&p);
        *TEST_PATH.lock() = None;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_panel_and_the_voice_share_one_file_with_a_task_per_line() {
        with_file(|p| {
            assert_eq!(list().unwrap(), Vec::<String>::new(), "no file: no tasks");
            assert_eq!(add("  пойти   погулять ").unwrap(), 1);
            // a line the voice command (tasks.lua) wrote is seen by the panel
            let mut text = std::fs::read_to_string(p).unwrap();
            text.push_str("купить хлеб\r\n\r\n");
            std::fs::write(p, text).unwrap();
            assert_eq!(list().unwrap(), vec!["пойти погулять".to_string(), "купить хлеб".to_string()]);
            assert_eq!(std::fs::read_to_string(p).unwrap().lines().next(), Some("пойти погулять"));
        });
    }

    #[test]
    fn edit_and_remove_touch_only_the_line_the_panel_saw() {
        with_file(|_| {
            add("первая").unwrap();
            add("вторая").unwrap();
            edit(1, "вторая", "вторая, исправленная").unwrap();
            assert!(edit(0, "не та", "x").is_err(), "the list changed meanwhile: nothing is edited");
            assert!(remove(5, "вторая").is_err());
            assert_eq!(remove(0, "первая").unwrap(), "первая");
            assert_eq!(list().unwrap(), vec!["вторая, исправленная".to_string()]);
            assert_eq!(clear().unwrap(), 1);
            assert!(list().unwrap().is_empty());
        });
    }

    #[test]
    fn empty_too_long_and_the_thirty_first_task_are_refused() {
        with_file(|_| {
            assert!(add("   ").is_err());
            assert!(add(&"я".repeat(MAX_LEN + 1)).is_err());
            for i in 0..MAX {
                add(&format!("задача {}", i)).unwrap();
            }
            let e = add("лишняя").unwrap_err();
            assert!(e.contains("30"), "{}", e);
            assert_eq!(list().unwrap().len(), MAX);
        });
    }

    #[test]
    fn the_bom_of_a_text_editor_is_not_part_of_the_first_task() {
        assert_eq!(parse("\u{feff}одна\n  \nдве"), vec!["одна".to_string(), "две".to_string()]);
    }

    #[test]
    fn the_spoken_list_names_the_count_and_the_first_seven() {
        assert_eq!(spoken(&[]), "Задач нет");
        let l: Vec<String> = (1..=9).map(|i| format!("t{}", i)).collect();
        let s = spoken(&l);
        assert!(s.starts_with("Задач: 9. 1. t1. 2. t2"), "{}", s);
        assert!(s.ends_with("И ещё 2"), "{}", s);
        assert!(!s.contains("t8"));
    }
}
