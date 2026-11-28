//! A folder of notes (Obsidian, plain .md/.txt files) as a source for the morning briefing: the titles of the notes changed in the
//! last days. Read only; only the file name and the first heading line are read, never the whole note.
//!
//! Idea: the Obsidian connector of OpenJarvis (https://github.com/open-jarvis/OpenJarvis, Apache-2.0); own code and own limits:
//! hidden folders (.obsidian, .git), sensitive files and folders (shield::is_sensitive_path) are skipped, a link out of the folder is
//! not followed, the depth and the number of looked-at files are capped.

use std::path::Path;
use std::time::{Duration, SystemTime};

const MAX_DEPTH: usize = 4;
const MAX_FILES: usize = 3000;

/// The title of a note: the first «# Heading» line when there is one, else the file name without its extension.
pub fn title_of(stem: &str, head: &str) -> String {
    let h = head.lines().map(|l| l.trim()).find(|l| !l.is_empty() && *l != "---");
    let t = match h {
        Some(l) if l.starts_with('#') => l.trim_start_matches('#').trim().to_string(),
        _ => stem.to_string(),
    };
    t.chars().filter(|c| !c.is_control()).take(80).collect::<String>().trim().to_string()
}

fn is_note(p: &Path) -> bool {
    matches!(p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref(), Some("md") | Some("txt"))
}

fn walk(dir: &Path, depth: usize, seen: &mut usize, out: &mut Vec<(SystemTime, std::path::PathBuf)>) {
    if depth > MAX_DEPTH {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if *seen >= MAX_FILES {
            return;
        }
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        let path = e.path();
        // (symlink_metadata: a link is neither followed nor read)
        let Ok(md) = std::fs::symlink_metadata(&path) else { continue };
        if crate::shield::is_sensitive_path(&path) {
            continue;
        }
        if md.is_dir() {
            walk(&path, depth + 1, seen, out);
        } else if md.is_file() && is_note(&path) {
            *seen += 1;
            if let Ok(t) = md.modified() {
                out.push((t, path));
            }
        }
    }
}

/// Titles of the notes changed within `days` days, newest first, at most `max`.
pub fn recent_titles(dir: &str, days: u64, max: usize) -> Vec<String> {
    let root = Path::new(dir.trim());
    if dir.trim().is_empty() || !root.is_dir() {
        return Vec::new();
    }
    let since = SystemTime::now().checked_sub(Duration::from_secs(days * 86400)).unwrap_or(SystemTime::UNIX_EPOCH);
    let mut files = Vec::new();
    walk(root, 0, &mut 0, &mut files);
    files.retain(|(t, _)| *t >= since);
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .take(max)
        .map(|(_, p)| {
            let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let mut head = String::new();
            if let Ok(f) = std::fs::File::open(&p) {
                use std::io::Read;
                let mut buf = vec![0u8; 400];
                let n = f.take(400).read(&mut buf).unwrap_or(0);
                head = String::from_utf8_lossy(&buf[..n]).to_string();
            }
            title_of(&stem, &head)
        })
        .filter(|t| !t.is_empty())
        .collect()
}

#[cfg(test)]
mod tests;
