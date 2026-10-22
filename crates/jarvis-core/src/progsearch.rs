//! Search and install of programs from the OFFICIAL catalogue of Windows (winget, source «winget»). winget itself downloads the
//! installer from the address in the manifest and refuses it when its SHA-256 does not match the manifest (that is the hash check;
//! Jarvis does not repeat it and does not claim more). The installer is started WITHOUT the silent flag, so its own window is shown and
//! nothing installs behind the user's back; an install also needs the permission «Скачивать и ставить программы» and a click on
//! «Установить». Progress is read from the real output of winget; «Отменить» stops the process. Every install is written to the history.

use crate::store;
use serde::{Deserialize, Serialize};

const HISTORY: &str = "install_history.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Package {
    pub name: String,
    pub id: String,
    pub version: String,
    pub source: String,
}

// the id goes to the command line: letters, digits, dots, dashes only (no spaces, quotes, «&», «|», «-»-options)
pub fn valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    (3..=100).contains(&b.len()) && b[0].is_ascii_alphanumeric() && id.chars().all(|c| c.is_ascii_alphanumeric() || ".+_-".contains(c)) && id.contains('.')
}

pub fn valid_query(q: &str) -> bool {
    let q = q.trim();
    !q.is_empty() && q.chars().count() <= 60 && !q.starts_with('-') && !q.chars().any(|c| c.is_control() || "&|<>^\"%".contains(c))
}

// the table printed by «winget search»: a header, a line of dashes, then the rows. The columns are cut where the header words start
// (the header is localised, so no word is expected, only the shape)
pub fn parse_table(out: &str) -> Vec<Package> {
    let lines: Vec<&str> = out.lines().map(|l| l.trim_end_matches('\r')).collect();
    let Some(d) = lines.iter().position(|l| l.len() >= 10 && l.chars().all(|c| c == '-')) else { return Vec::new() };
    if d == 0 {
        return Vec::new();
    }
    // the header may be preceded by the spinner text on the same line (\r updates): take what follows the last \r
    let header = lines[d - 1].rsplit('\r').next().unwrap_or("");
    let chars: Vec<char> = header.chars().collect();
    let mut starts = Vec::new();
    let mut prev_space = true;
    let mut gap = 2;
    for (i, c) in chars.iter().enumerate() {
        if *c == ' ' {
            gap += 1;
        } else {
            if gap >= 2 || prev_space && starts.is_empty() {
                starts.push(i);
            }
            gap = 0;
        }
        prev_space = *c == ' ';
    }
    if starts.len() < 3 {
        return Vec::new();
    }
    let cell = |row: &[char], i: usize| -> String {
        let from = starts[i].min(row.len());
        let to = starts.get(i + 1).copied().unwrap_or(row.len()).min(row.len());
        row[from..to.max(from)].iter().collect::<String>().trim().to_string()
    };
    let mut res = Vec::new();
    for l in &lines[d + 1..] {
        let row: Vec<char> = l.chars().collect();
        if row.is_empty() || l.trim().is_empty() {
            continue;
        }
        let (name, id, version) = (cell(&row, 0), cell(&row, 1), cell(&row, 2));
        if name.is_empty() || !valid_id(&id) {
            continue;
        }
        res.push(Package { name, id, version, source: "winget".into() });
    }
    res
}

// «45%» / «12.3 MB / 50.0 MB» in a line of winget → percent
pub fn parse_progress(line: &str) -> Option<u32> {
    if let Some(i) = line.find('%') {
        let digits: String = line[..i].trim_end().chars().rev().take_while(|c| c.is_ascii_digit()).collect::<Vec<_>>().into_iter().rev().collect();
        if let Ok(n) = digits.parse::<u32>() {
            return Some(n.min(100));
        }
    }
    // «12.3 MB / 50.0 MB»
    let parts: Vec<&str> = line.split_whitespace().collect();
    for w in parts.windows(5) {
        if w[2] == "/" {
            let unit = |u: &str| match u.to_ascii_uppercase().as_str() { "KB" => Some(1.0), "MB" => Some(1024.0), "GB" => Some(1024.0 * 1024.0), _ => None };
            if let (Ok(a), Ok(b), Some(ua), Some(ub)) = (w[0].replace(',', ".").parse::<f64>(), w[3].replace(',', ".").parse::<f64>(), unit(w[1]), unit(w[4])) {
                if b > 0.0 {
                    return Some(((a * ua) / (b * ub) * 100.0).round().clamp(0.0, 100.0) as u32);
                }
            }
        }
    }
    None
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub at: u64,
    // "ok" | "failed" | "cancelled"
    pub result: String,
    pub detail: String,
}

pub fn history() -> Vec<Entry> {
    store::read::<Vec<Entry>>(HISTORY).0
}
fn log(e: Entry) {
    let mut h = history();
    h.push(e);
    let n = h.len();
    if n > 200 {
        h.drain(..n - 200);
    }
    let _ = store::write(HISTORY, &h);
}

#[derive(Serialize, Clone, Debug, Default, PartialEq)]
pub struct Status {
    // "idle" | "searching" | "downloading" | "installing" | "done" | "failed" | "cancelled"
    pub state: String,
    pub id: String,
    pub percent: Option<u32>,
    pub line: String,
}

static STATUS: parking_lot::Mutex<Option<Status>> = parking_lot::const_mutex(None);
static CHILD_PID: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn status() -> Status {
    STATUS.lock().clone().unwrap_or(Status { state: "idle".into(), ..Default::default() })
}
fn set(state: &str, id: &str, percent: Option<u32>, line: &str) {
    *STATUS.lock() = Some(Status { state: state.into(), id: id.into(), percent, line: line.chars().take(200).collect() });
}

#[cfg(target_os = "windows")]
mod win {
    use super::*;
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::atomic::Ordering;

    fn winget() -> Command {
        let mut c = Command::new("winget");
        crate::null_stdio(&mut c);
        c.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }

    pub fn search(q: &str) -> Result<Vec<Package>, String> {
        if !valid_query(q) {
            return Err("Запрос должен быть коротким и без спецсимволов.".into());
        }
        set("searching", "", None, "");
        let out = winget().args(["search", "--source", "winget", "--accept-source-agreements", "--disable-interactivity", "--", q.trim()]).output().map_err(|e| format!("winget не запустился ({}). Нужен «Установщик приложений» из Microsoft Store.", e));
        set("idle", "", None, "");
        let out = out?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let list = parse_table(&text);
        if list.is_empty() && !out.status.success() && !text.contains("No package") {
            return Err(format!("winget ответил ошибкой {}", out.status.code().unwrap_or(-1)));
        }
        Ok(list.into_iter().take(15).collect())
    }

    // blocks until the end; the status is read by the window
    pub fn install(p: &Package) -> Result<(), String> {
        CANCEL.store(false, Ordering::SeqCst);
        set("downloading", &p.id, None, "Запуск winget…");
        // (no --silent: the installer shows its own window; no hidden install)
        let mut child = winget().args(["install", "--id", &p.id, "--exact", "--source", "winget", "--accept-source-agreements", "--accept-package-agreements", "--interactive"]).spawn().map_err(|e| format!("winget не запустился: {}", e))?;
        CHILD_PID.store(child.id(), Ordering::SeqCst);
        let mut stdout = child.stdout.take().ok_or("нет вывода winget")?;
        let mut buf = [0u8; 2048];
        let mut pending = String::new();
        loop {
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    pending.push_str(&String::from_utf8_lossy(&buf[..n]));
                    // the progress is redrawn with \r
                    let parts: Vec<String> = pending.split(|c| c == '\r' || c == '\n').map(|s| s.to_string()).collect();
                    for l in &parts[..parts.len().saturating_sub(1)] {
                        let l = l.trim();
                        if l.is_empty() {
                            continue;
                        }
                        let state = if l.to_lowercase().contains("install") || l.contains("Установ") { "installing" } else { "downloading" };
                        set(state, &p.id, parse_progress(l), l);
                    }
                    pending = parts.last().cloned().unwrap_or_default();
                }
            }
        }
        let status = child.wait().map_err(|e| e.to_string())?;
        CHILD_PID.store(0, Ordering::SeqCst);
        if CANCEL.load(Ordering::SeqCst) {
            set("cancelled", &p.id, None, "Отменено");
            log(Entry { id: p.id.clone(), name: p.name.clone(), at: store::now(), result: "cancelled".into(), detail: String::new() });
            return Err("Установка отменена.".into());
        }
        if status.success() {
            set("done", &p.id, Some(100), "Готово");
            log(Entry { id: p.id.clone(), name: p.name.clone(), at: store::now(), result: "ok".into(), detail: String::new() });
            Ok(())
        } else {
            let code = status.code().unwrap_or(-1);
            set("failed", &p.id, None, &format!("winget: код {}", code));
            log(Entry { id: p.id.clone(), name: p.name.clone(), at: store::now(), result: "failed".into(), detail: format!("код {}", code) });
            Err(format!("winget завершился с ошибкой (код {}). Если установщик показал окно и вы его закрыли — это нормально.", code))
        }
    }

    pub fn cancel() -> bool {
        CANCEL.store(true, Ordering::SeqCst);
        let pid = CHILD_PID.load(Ordering::SeqCst);
        if pid == 0 {
            return false;
        }
        let mut c = Command::new("taskkill");
        crate::null_stdio(&mut c);
        c.args(["/PID", &pid.to_string(), "/T", "/F"]).status().map(|s| s.success()).unwrap_or(false)
    }
}

#[cfg(target_os = "windows")]
pub use win::{cancel, install, search};

#[cfg(not(target_os = "windows"))]
pub fn search(_q: &str) -> Result<Vec<Package>, String> {
    Err("Каталог программ (winget) есть только в Windows.".into())
}
#[cfg(not(target_os = "windows"))]
pub fn install(_p: &Package) -> Result<(), String> {
    Err("Установка программ (winget) есть только в Windows.".into())
}
#[cfg(not(target_os = "windows"))]
pub fn cancel() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const OUT: &str = "\r   - \r   \\ \rName                         Id                           Version      Match        Source\n-------------------------------------------------------------------------------------------------\nVisual Studio Code           Microsoft.VisualStudioCode   1.94.2       Tag: vscode  winget\nNotepad++                    Notepad++.Notepad++          8.7.1                     winget\nBad Row                      bad id                       1.0                       winget\n";

    #[test]
    fn parses_the_table_of_winget() {
        let r = parse_table(OUT);
        assert_eq!(r.len(), 2, "{:?}", r);
        assert_eq!(r[0], Package { name: "Visual Studio Code".into(), id: "Microsoft.VisualStudioCode".into(), version: "1.94.2".into(), source: "winget".into() });
        assert_eq!(r[1].id, "Notepad++.Notepad++");
        assert!(parse_table("No package found matching input criteria.").is_empty());
        assert!(parse_table("").is_empty());
    }

    #[test]
    fn ids_and_queries_cannot_carry_options_or_shell() {
        assert!(valid_id("Microsoft.VisualStudioCode"));
        assert!(!valid_id("--source evil"));
        assert!(!valid_id("a&b.c"));
        assert!(!valid_id("noDots"));
        assert!(!valid_id("x.y z"));
        assert!(valid_query("vlc"));
        assert!(!valid_query("--help"));
        assert!(!valid_query("a & calc"));
        assert!(!valid_query(""));
    }

    #[test]
    fn progress_is_read_from_the_real_lines() {
        assert_eq!(parse_progress("  ██████▒▒▒▒  45%"), Some(45));
        assert_eq!(parse_progress("12.5 MB / 50.0 MB"), Some(25));
        assert_eq!(parse_progress("Downloading https://x"), None);
        assert_eq!(parse_progress("100 %"), Some(100));
    }

    #[test]
    fn history_is_kept() {
        store::with_temp(|| {
            log(Entry { id: "A.B".into(), name: "AB".into(), at: 1, result: "ok".into(), detail: String::new() });
            assert_eq!(history().len(), 1);
        });
    }

    #[test]
    fn without_windows_nothing_pretends_to_install() {
        #[cfg(not(target_os = "windows"))]
        {
            assert!(search("vlc").is_err());
            assert!(install(&Package { name: "x".into(), id: "A.B".into(), version: String::new(), source: String::new() }).is_err());
        }
    }
}
