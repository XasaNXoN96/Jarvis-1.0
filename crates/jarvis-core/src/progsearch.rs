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


// A search by PURPOSE («программа для видео», «архиватор», «video editor»): a short fixed table of what people ask for, each with the English
// words winget understands. Only these words reach winget (never the text of the user as an option); the user's own words are searched too.
const PURPOSES: &[(&[&str], &[&str])] = &[
    (&["видео", "монтаж", "video editor", "video edit"], &["video editor", "shotcut", "kdenlive"]),
    (&["плеер", "смотреть фильм", "медиаплеер", "media player", "music player", "музык"], &["media player", "vlc", "aimp"]),
    (&["архив", "архиватор", "zip", "rar", "archiver"], &["7zip", "winrar", "peazip"]),
    (&["браузер", "browser", "интернет"], &["browser", "firefox", "chrome"]),
    (&["pdf", "пдф", "читать книги"], &["pdf", "sumatra", "adobe reader"]),
    (&["торрент", "torrent"], &["torrent", "qbittorrent"]),
    (&["антивирус", "вирус", "antivirus"], &["antivirus", "malwarebytes"]),
    (&["заметки", "блокнот", "текстовый редактор", "notes", "text editor"], &["notepad++", "obsidian", "text editor"]),
    (&["мессенджер", "общение", "чат", "messenger", "telegram", "whatsapp"], &["telegram", "whatsapp", "discord"]),
    (&["почта", "email", "mail"], &["thunderbird", "email"]),
    (&["запись экрана", "стрим", "screen record", "stream"], &["obs studio", "screen recorder"]),
    (&["фото", "картинк", "изображен", "photo", "image editor", "рисова"], &["gimp", "paint.net", "krita"]),
    (&["программирован", "код", "ide", "programming", "coding"], &["visual studio code", "pycharm", "ide"]),
    (&["игр", "game launcher", "steam"], &["steam", "epic games", "game launcher"]),
    (&["пароль", "password"], &["password manager", "keepass", "bitwarden"]),
    (&["облако", "cloud", "диск", "синхрониз"], &["dropbox", "google drive", "onedrive"]),
    (&["vpn", "впн"], &["vpn", "wireguard"]),
    (&["скриншот", "screenshot"], &["sharex", "screenshot"]),
    (&["переводчик", "translator"], &["translator", "deepl"]),
    (&["аудио", "звук", "запись голоса", "audio editor"], &["audacity", "audio editor"]),
    (&["3d", "моделирован"], &["blender", "3d"]),
];

// the queries to run for a text: always the text itself first, then (when it names a purpose) at most two fixed English words
pub fn expand_query(q: &str) -> Vec<String> {
    let q = q.trim();
    let low = q.to_lowercase().replace('ё', "е");
    let mut out = vec![q.to_string()];
    if q.chars().count() < 3 {
        return out;
    }
    for (keys, terms) in PURPOSES {
        if keys.iter().any(|k| low.contains(k)) {
            for t in terms.iter().take(2) {
                if !out.iter().any(|o| o.eq_ignore_ascii_case(t)) {
                    out.push(t.to_string());
                }
            }
            break;
        }
    }
    out
}

// results of several searches in one list: no id twice, the user's own words first, at most `max`
pub fn merge(lists: Vec<Vec<Package>>, max: usize) -> Vec<Package> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for l in lists {
        for p in l {
            if seen.insert(p.id.to_lowercase()) {
                out.push(p);
            }
        }
    }
    out.truncate(max);
    out
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
/// Atomic claim of the single install slot: two clicks can no longer start two installs.
pub fn begin(id: &str) -> bool {
    let mut g = STATUS.lock();
    if matches!(g.as_ref().map(|s| s.state.as_str()), Some("downloading") | Some("installing") | Some("searching")) {
        return false;
    }
    *g = Some(Status { state: "downloading".into(), id: id.into(), percent: None, line: "Запуск winget…".into() });
    true
}

/// The end state of an install: «cancelled» only if the cancel was really delivered, otherwise by the exit code.
pub fn finish_state(cancelled: bool, success: bool) -> &'static str {
    if cancelled {
        "cancelled"
    } else if success {
        "done"
    } else {
        "failed"
    }
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
        let _busy = crate::activity::enter(crate::activity::Kind::Searching);
        if !valid_query(q) {
            return Err("Запрос должен быть коротким и без спецсимволов.".into());
        }
        set("searching", "", None, "");
        let mut lists = Vec::new();
        let mut first_error: Option<String> = None;
        for (i, term) in expand_query(q).iter().enumerate() {
            let out = winget().args(["search", "--source", "winget", "--accept-source-agreements", "--disable-interactivity", "--", term]).output();
            let out = match out {
                Ok(o) => o,
                Err(e) => {
                    set("idle", "", None, "");
                    return Err(format!("winget не запустился ({}). Нужен «Установщик приложений» из Microsoft Store.", e));
                }
            };
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            let list = parse_table(&text);
            if list.is_empty() && !out.status.success() && !text.contains("No package") && i == 0 {
                first_error = Some(format!("winget ответил ошибкой {}", out.status.code().unwrap_or(-1)));
            }
            lists.push(list);
        }
        set("idle", "", None, "");
        let list = merge(lists, 15);
        if list.is_empty() {
            if let Some(e) = first_error {
                return Err(e);
            }
        }
        Ok(list)
    }

    // blocks until the end; the status is read by the window
    pub fn install(p: &Package) -> Result<(), String> {
        CANCEL.store(false, Ordering::SeqCst);
        let fail = |why: String| -> String {
            set("failed", &p.id, None, &why);
            log(Entry { id: p.id.clone(), name: p.name.clone(), at: store::now(), result: "failed".into(), detail: why.chars().take(120).collect() });
            why
        };
        // (no --silent: the installer shows its own window; no hidden install)
        let mut child = winget().args(["install", "--id", &p.id, "--exact", "--source", "winget", "--accept-source-agreements", "--accept-package-agreements", "--interactive"]).spawn().map_err(|e| fail(format!("winget не запустился: {}", e)))?;
        CHILD_PID.store(child.id(), Ordering::SeqCst);
        let mut stdout = match child.stdout.take() {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                CHILD_PID.store(0, Ordering::SeqCst);
                return Err(fail("нет вывода winget".into()));
            }
        };
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
        let status = child.wait().map_err(|e| fail(e.to_string()))?;
        CHILD_PID.store(0, Ordering::SeqCst);
        if finish_state(CANCEL.load(Ordering::SeqCst), status.success()) == "cancelled" {
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
        let pid = CHILD_PID.load(Ordering::SeqCst);
        if pid == 0 {
            return false;
        }
        let mut c = Command::new("taskkill");
        crate::null_stdio(&mut c);
        let ok = c.args(["/PID", &pid.to_string(), "/T", "/F"]).status().map(|s| s.success()).unwrap_or(false);
        // «cancelled» is written only when the process was really stopped
        if ok {
            CANCEL.store(true, Ordering::SeqCst);
        }
        ok
    }
}

#[cfg(target_os = "windows")]
pub use win::{cancel, install, search};

#[cfg(not(target_os = "windows"))]
pub fn search(_q: &str) -> Result<Vec<Package>, String> {
    let _busy = crate::activity::enter(crate::activity::Kind::Searching);
    Err("Каталог программ (winget) есть только в Windows.".into())
}
#[cfg(not(target_os = "windows"))]
pub fn install(p: &Package) -> Result<(), String> {
    set("failed", &p.id, None, "Установка программ (winget) есть только в Windows.");
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
    fn a_purpose_is_turned_into_a_few_fixed_search_words() {
        assert_eq!(expand_query("vlc"), vec!["vlc"], "a name is searched as it is");
        let v = expand_query("программа для монтажа видео");
        assert_eq!(v[0], "программа для монтажа видео");
        assert!(v.len() == 3 && v.contains(&"video editor".to_string()) && v.contains(&"shotcut".to_string()), "{:?}", v);
        assert!(expand_query("Архиватор").contains(&"7zip".to_string()));
        assert!(expand_query("ёж").len() == 1, "a very short text is never expanded");
        // whatever the text, the extra words come only from the table and pass the same check as a query
        for q in ["видео", "торрент", "vpn", "пароль", "3d", "почта"] {
            for t in expand_query(q) {
                assert!(valid_query(&t), "{}", t);
            }
        }
        assert_eq!(expand_query("видео --source evil")[1..].iter().all(|t| !t.starts_with('-')), true);
    }

    #[test]
    fn several_searches_make_one_list_without_repeats() {
        let p = |id: &str| Package { name: id.into(), id: id.into(), version: String::new(), source: "winget".into() };
        let m = merge(vec![vec![p("A.One"), p("B.Two")], vec![p("a.one"), p("C.Three")]], 3);
        assert_eq!(m.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["A.One", "B.Two", "C.Three"]);
        assert_eq!(merge(vec![vec![p("A.One"), p("B.Two")]], 1).len(), 1);
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
    fn the_install_slot_is_claimed_once() {
        set("idle", "", None, "");
        assert!(begin("A.B"));
        assert!(!begin("C.D"), "a second click must not start a second install");
        assert_eq!(status().id, "A.B");
        set("failed", "A.B", None, "x");
        assert!(begin("C.D"), "after the end the slot is free again");
        set("idle", "", None, "");
    }

    #[test]
    fn the_end_state_does_not_claim_a_cancel_that_did_not_happen() {
        assert_eq!(finish_state(true, false), "cancelled");
        assert_eq!(finish_state(false, true), "done");
        assert_eq!(finish_state(false, false), "failed");
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
