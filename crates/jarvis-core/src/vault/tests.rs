use super::*;
use std::fs;

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("jarvis-vault-{}-{}", name, std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn the_title_is_the_heading_or_the_file_name() {
    assert_eq!(title_of("idea", "# План поездки\nтекст"), "План поездки");
    assert_eq!(title_of("idea", "---\n# После шапки\n"), "После шапки");
    assert_eq!(title_of("idea", "просто текст без заголовка"), "idea");
    assert_eq!(title_of("idea", ""), "idea");
    assert_eq!(title_of("x", &format!("# {}", "я".repeat(200))).chars().count(), 80);
}

#[test]
fn recent_notes_are_found_hidden_and_sensitive_ones_are_not() {
    let d = tmp("find");
    fs::write(d.join("a.md"), "# Идея проекта\nдлинный текст").unwrap();
    fs::write(d.join("b.txt"), "без заголовка").unwrap();
    fs::write(d.join("c.png"), "x").unwrap();
    fs::write(d.join(".env"), "KEY=1").unwrap();
    fs::create_dir_all(d.join(".obsidian")).unwrap();
    fs::write(d.join(".obsidian").join("workspace.md"), "# скрыто").unwrap();
    fs::create_dir_all(d.join("sub")).unwrap();
    fs::write(d.join("sub").join("deep.md"), "# Глубокая заметка").unwrap();
    let mut t = recent_titles(d.to_str().unwrap(), 3, 10);
    t.sort();
    assert_eq!(t, vec!["b", "Глубокая заметка", "Идея проекта"]);
    assert_eq!(recent_titles(d.to_str().unwrap(), 3, 2).len(), 2);
    let _ = fs::remove_dir_all(&d);
}

#[test]
fn a_missing_or_empty_folder_gives_nothing() {
    assert!(recent_titles("", 3, 5).is_empty());
    assert!(recent_titles("/no/such/folder-xyz", 3, 5).is_empty());
}

#[test]
fn old_notes_are_not_listed() {
    let d = tmp("old");
    fs::write(d.join("a.md"), "# Старое").unwrap();
    // 0 days: nothing is newer than «now»
    assert!(recent_titles(d.to_str().unwrap(), 0, 5).is_empty());
    let _ = fs::remove_dir_all(&d);
}
