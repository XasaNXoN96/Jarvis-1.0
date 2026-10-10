// File search on this computer: "найди файл 123".
//
// Runs in the background (the answer arrives by itself: window + voice), so Jarvis stays usable.
// Stage 1: the user's own folders (Desktop, Documents, Downloads, Pictures, Videos, Music).
// If nothing good is found there, stage 2 searches the whole user profile and the other drives.
// Results are ranked: exact name, name starts with the query, name contains the query.
// The last results are kept so that "открой первый результат" can open them.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::i18n;

const MAX_RESULTS: usize = 40;
const STAGE2_LIMIT: Duration = Duration::from_secs(75);

static GENERATION: AtomicU64 = AtomicU64::new(0);
static RESULTS: Lazy<Mutex<Vec<FoundItem>>> = Lazy::new(|| Mutex::new(Vec::new()));

#[derive(Clone, Debug)]
pub struct FoundItem {
    pub path: PathBuf,
    pub is_dir: bool,
    score: u8, // 0 exact, 1 starts with, 2 contains
}

pub fn results() -> Vec<FoundItem> {
    RESULTS.lock().clone()
}

// folders we never look into: system stuff, caches, build output
fn skip_dir(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "windows"
            | "program files"
            | "program files (x86)"
            | "programdata"
            | "$recycle.bin"
            | "system volume information"
            | "appdata"
            | "node_modules"
            | ".git"
            | "$windows.~bt"
            | "$windows.~ws"
            | "winsxs"
            | "windowsapps"
            | "proc"
            | "sys"
            | "dev"
            | "usr"
            | "lib"
            | "lib64"
            | "bin"
            | "sbin"
            | "etc"
            | "var"
            | "run"
            | "snap"
            | "boot"
    )
}

fn score(name: &str, query: &str) -> Option<u8> {
    let name_l = name.to_lowercase();
    let stem_l = Path::new(&name_l)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| name_l.clone());
    if name_l == query || stem_l == query {
        Some(0)
    } else if stem_l.starts_with(query) {
        Some(1)
    } else if name_l.contains(query) {
        Some(2)
    } else {
        None
    }
}

struct Search<'a> {
    query: String,
    generation: u64,
    found: Vec<FoundItem>,
    deadline: Option<Instant>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl<'a> Search<'a> {
    fn cancelled(&self) -> bool {
        GENERATION.load(Ordering::SeqCst) != self.generation
            || self.deadline.map(|d| Instant::now() > d).unwrap_or(false)
    }

    // breadth-first walk: shallow matches (the ones you usually want) come first
    fn walk(&mut self, root: &Path) {
        let mut queue: VecDeque<PathBuf> = VecDeque::new();
        queue.push_back(root.to_path_buf());
        while let Some(dir) = queue.pop_front() {
            if self.cancelled() || self.found.len() >= MAX_RESULTS * 5 {
                return;
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if let Some(s) = score(&name, &self.query) {
                    self.found.push(FoundItem { path: path.clone(), is_dir, score: s });
                }
                if is_dir && !skip_dir(&name.to_lowercase()) {
                    queue.push_back(path);
                }
            }
        }
    }

    fn good_matches(&self) -> usize {
        self.found.iter().filter(|f| f.score <= 1).count()
    }
}

fn user_dirs() -> Vec<PathBuf> {
    let home = match std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        Ok(h) => PathBuf::from(h),
        Err(_) => return Vec::new(),
    };
    ["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music", "OneDrive"]
        .iter()
        .map(|d| home.join(d))
        .filter(|p| p.exists())
        .collect()
}

fn other_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        roots.push(PathBuf::from(home));
    }
    #[cfg(target_os = "windows")]
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if root.exists() {
            roots.push(root);
        }
    }
    roots
}

fn rank_and_dedup(mut items: Vec<FoundItem>) -> Vec<FoundItem> {
    items.sort_by(|a, b| {
        a.score
            .cmp(&b.score)
            .then(a.path.components().count().cmp(&b.path.components().count()))
            .then(a.path.cmp(&b.path))
    });
    items.dedup_by(|a, b| a.path == b.path);
    items.truncate(MAX_RESULTS);
    items
}

fn say(text: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{}", text);
    #[cfg(feature = "jarvis_app")]
    crate::ipc::send(crate::ipc::IpcEvent::Output { text: text.to_string() });
}

fn announce(query: &str, items: &[FoundItem]) {
    let ru = i18n::get_language() != "en";
    if items.is_empty() {
        let msg = if ru {
            format!("По запросу «{}» ничего не нашёл", query)
        } else {
            format!("Nothing found for \"{}\"", query)
        };
        say(&msg);
        crate::tts::speak(&msg);
        return;
    }
    say(&if ru {
        format!("Нашёл по запросу «{}»: {}", query, items.len())
    } else {
        format!("Found for \"{}\": {}", query, items.len())
    });
    for (i, item) in items.iter().take(8).enumerate() {
        let name = item.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        let dir = item.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
        say(&format!("  {}. {}{}   ({})", i + 1, name, if item.is_dir { " [папка]" } else { "" }, dir));
    }
    let first = &items[0];
    let first_name = first.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let first_dir = first.path.parent().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let spoken = if ru {
        format!(
            "Нашёл {}. Первый: {}, в папке {}. Скажите «открой первый результат».",
            items.len(),
            first_name,
            first_dir
        )
    } else {
        format!("Found {}. The first one: {}, in the folder {}. Say \"open the first result\".", items.len(), first_name, first_dir)
    };
    crate::tts::speak(&spoken);
}

// Starts a background search. A new search cancels the previous one.
pub fn start(query: &str) {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return;
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    info!("File search started: '{}'", query);

    std::thread::spawn(move || {
        let mut search = Search {
            query: query.clone(),
            generation,
            found: Vec::new(),
            deadline: None,
            _marker: std::marker::PhantomData,
        };

        // stage 1: the user's own folders
        for dir in user_dirs() {
            search.walk(&dir);
        }

        // stage 2: nothing good yet -> the whole profile and other drives, with a time limit
        if search.good_matches() == 0 && !search.cancelled() {
            say(&if i18n::get_language() != "en" {
                "В ваших папках не нашёл, ищу по всему компьютеру…".to_string()
            } else {
                "Not in your folders, searching the whole computer…".to_string()
            });
            search.deadline = Some(Instant::now() + STAGE2_LIMIT);
            for root in other_roots() {
                search.walk(&root);
                if search.cancelled() {
                    break;
                }
            }
        }

        if GENERATION.load(Ordering::SeqCst) != generation {
            return; // a newer search replaced this one
        }
        let ranked = rank_and_dedup(search.found);
        *RESULTS.lock() = ranked.clone();
        announce(&query, &ranked);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoring() {
        assert_eq!(score("123.txt", "123"), Some(0));
        assert_eq!(score("123", "123"), Some(0));
        assert_eq!(score("123 отчёт.docx", "123"), Some(1));
        assert_eq!(score("мой файл 123.pdf", "123"), Some(2));
        assert_eq!(score("abc.txt", "123"), None);
        assert_eq!(score("ОТЧЁТ.DOCX", "отчёт"), Some(0));
    }

    #[test]
    fn finds_files_in_a_tree() {
        let base = std::env::temp_dir().join(format!("jarvis_fs_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("a/b")).unwrap();
        std::fs::write(base.join("a/b/123.txt"), b"x").unwrap();
        std::fs::write(base.join("a/other 123 file.txt"), b"x").unwrap();
        std::fs::write(base.join("a/nope.txt"), b"x").unwrap();
        let mut s = Search {
            query: "123".into(),
            generation: GENERATION.load(Ordering::SeqCst),
            found: Vec::new(),
            deadline: None,
            _marker: std::marker::PhantomData,
        };
        s.walk(&base);
        let ranked = rank_and_dedup(s.found);
        assert_eq!(ranked.len(), 2);
        assert!(ranked[0].path.ends_with("123.txt")); // exact match first
        let _ = std::fs::remove_dir_all(&base);
    }
}
