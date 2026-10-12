// The programs installed on this computer, found once and kept in a small index (apps_index.json), so that
// "открой Photoshop" or "запусти Total Commander" work for programs that are not in the fixed list of apps.rs.
//
// Where the names come from (no endless scan of the disk on every command):
//   - the Start menu of the current user and of all users (shortcuts, the folders inside),
//   - the desktop of the user and the public desktop,
//   - the programs registered by Windows: Get-StartApps (includes Store apps) and the "App Paths" registry key.
// The index is read from the file at start, rebuilt in the background when it is older than a day and on request
// ("обнови список программ", the button on the "Программы" page).
//
// A name that fits several programs ("командер": Total Commander and Double Commander) never starts a random one:
// the caller gets the list and asks which one. Only entries of the index are ever started, never a path typed by
// the user, and a path is checked to exist before it is started.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::apps::{allowed_edits, edit_distance, tokens, word_key};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct IndexedApp {
    pub name: String,
    // what is started: a shortcut / program file, or "shell:AppsFolder\<AppID>" of a registered app
    pub launch: String,
    // start_menu | desktop | registered | app_paths
    pub source: String,
    // the program file name in lower case when it is known ("winword.exe"), for the programs of apps.rs
    #[serde(default)]
    pub exe: String,
}

#[derive(Default, Serialize, Deserialize)]
struct Cache {
    scanned_at: u64,
    apps: Vec<IndexedApp>,
}

static INDEX: Lazy<RwLock<Cache>> = Lazy::new(|| RwLock::new(Cache::default()));
static LOADED: AtomicBool = AtomicBool::new(false);
static SCANNING: AtomicBool = AtomicBool::new(false);
static LAST_ERROR: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
// the programs offered by the last ambiguous request ("какой открыть: 1. …, 2. …")
static PENDING: Lazy<Mutex<Vec<IndexedApp>>> = Lazy::new(|| Mutex::new(Vec::new()));

const MAX_AGE_SECS: u64 = 24 * 3600;
const SCAN_TIMEOUT: Duration = Duration::from_secs(40);

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn cache_file() -> Option<PathBuf> {
    crate::APP_CONFIG_DIR.get().map(|d| d.join("apps_index.json"))
}

pub fn apps() -> Vec<IndexedApp> {
    ensure_loaded();
    INDEX.read().apps.clone()
}

pub fn scanned_at() -> u64 {
    INDEX.read().scanned_at
}

pub fn is_scanning() -> bool {
    SCANNING.load(Ordering::SeqCst)
}

pub fn last_error() -> String {
    LAST_ERROR.lock().clone()
}

// read the saved index once; an old or missing index is rebuilt in the background
pub fn ensure_loaded() {
    if LOADED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(f) = cache_file() {
        if let Ok(text) = std::fs::read_to_string(&f) {
            match serde_json::from_str::<Cache>(&text) {
                Ok(c) => *INDEX.write() = c,
                Err(e) => warn!("Programs index {} is damaged ({}), it is rebuilt", f.display(), e),
            }
        }
    }
    let stale = { let c = INDEX.read(); c.apps.is_empty() || now_secs().saturating_sub(c.scanned_at) > MAX_AGE_SECS };
    if stale && cfg!(target_os = "windows") && !cfg!(test) {
        rescan_background();
    }
}

// rebuild the index without blocking the caller; false when a scan is already running
pub fn rescan_background() -> bool {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return false;
    }
    std::thread::spawn(|| {
        let n = rescan_inner();
        info!("Programs index: {} programs", n);
    });
    true
}

// rebuild the index now (the scan itself has a time limit)
pub fn rescan_now() -> usize {
    if SCANNING.swap(true, Ordering::SeqCst) {
        // another scan runs: wait for it instead of scanning twice
        for _ in 0..(SCAN_TIMEOUT.as_millis() / 100 + 20) {
            if !is_scanning() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        return INDEX.read().apps.len();
    }
    rescan_inner()
}

fn rescan_inner() -> usize {
    let apps = scan();
    let n = apps.len();
    {
        let mut c = INDEX.write();
        // a scan that found nothing (PowerShell blocked, folders unreadable) never wipes a good index
        if n > 0 || c.apps.is_empty() {
            c.apps = apps;
            c.scanned_at = now_secs();
        }
    }
    if let Some(f) = cache_file() {
        let text = serde_json::to_string(&*INDEX.read()).unwrap_or_default();
        if let Err(e) = std::fs::write(&f, text) {
            warn!("Programs index not saved: {}", e);
        }
    }
    SCANNING.store(false, Ordering::SeqCst);
    INDEX.read().apps.len()
}

#[cfg(test)]
pub(crate) fn set_for_tests(apps: Vec<IndexedApp>) {
    LOADED.store(true, Ordering::SeqCst);
    *INDEX.write() = Cache { scanned_at: now_secs(), apps };
    PENDING.lock().clear();
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

// ---------------------------------------------------------------------------------------------
// scanning
// ---------------------------------------------------------------------------------------------

// shortcuts that are not programs: uninstallers, manuals, web links of the vendor
pub fn is_junk(name: &str) -> bool {
    let n = name.to_lowercase();
    const JUNK: &[&str] = &[
        "uninstall", "удалить", "удаление", "деинсталл", "readme", "read me", "documentation", "документация", "license", "лиценз",
        "website", "веб-сайт", "веб сайт", "release notes", "changelog", "what's new", "manual", "руководство", "справка",
        "help", "support", "поддержка", "check for updates", "repair", "восстановить",
    ];
    JUNK.iter().any(|j| n.contains(j))
}

fn shortcut_ext(p: &Path) -> bool {
    matches!(p.extension().map(|e| e.to_string_lossy().to_lowercase()).as_deref(), Some("lnk") | Some("url") | Some("appref-ms") | Some("exe"))
}

// the shortcuts of these folders (the Start menu is a tree; the desktop is searched one level deep)
pub(crate) fn scan_dirs(start_dirs: &[PathBuf], desktop_dirs: &[PathBuf]) -> Vec<IndexedApp> {
    let mut out = Vec::new();
    let walk = |root: &Path, depth_limit: usize, source: &str, out: &mut Vec<IndexedApp>| {
        let mut stack = vec![(root.to_path_buf(), 0usize)];
        let mut seen = 0usize;
        while let Some((dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for e in entries.flatten() {
                seen += 1;
                if seen > 20_000 {
                    return; // a strange folder: never hang on it
                }
                let p = e.path();
                let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                if is_dir {
                    if depth + 1 < depth_limit {
                        stack.push((p, depth + 1));
                    }
                    continue;
                }
                if !shortcut_ext(&p) {
                    continue;
                }
                let name = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
                if name.trim().is_empty() || is_junk(&name) {
                    continue;
                }
                let exe = if p.extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false) {
                    p.file_name().map(|f| f.to_string_lossy().to_lowercase()).unwrap_or_default()
                } else {
                    String::new()
                };
                out.push(IndexedApp { name, launch: p.to_string_lossy().to_string(), source: source.to_string(), exe });
            }
        }
    };
    for d in start_dirs {
        walk(d, 5, "start_menu", &mut out);
    }
    for d in desktop_dirs {
        walk(d, 1, "desktop", &mut out);
    }
    out
}

#[derive(Deserialize)]
struct Registered {
    #[serde(default, rename = "start")]
    start: OneOrMany<StartApp>,
    #[serde(default, rename = "paths")]
    paths: OneOrMany<AppPath>,
}

// ConvertTo-Json writes a list of one element as the element itself
#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    Many(Vec<T>),
    // Windows PowerShell 5.1 sometimes writes an array as { "value": [...], "Count": n }
    Wrapped { value: Vec<T> },
    One(T),
}
impl<T> Default for OneOrMany<T> {
    fn default() -> Self {
        OneOrMany::Many(Vec::new())
    }
}
impl<T> OneOrMany<T> {
    fn into_vec(self) -> Vec<T> {
        match self {
            OneOrMany::Many(v) | OneOrMany::Wrapped { value: v } => v,
            OneOrMany::One(x) => vec![x],
        }
    }
}

// no defaults: an object of another shape is an error to report, not an empty program
#[derive(Deserialize)]
struct StartApp {
    // a null name or id is one entry to skip, not a reason to lose the whole list
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "AppID")]
    app_id: Option<String>,
}

#[derive(Deserialize)]
struct AppPath {
    exe: String,
    path: String,
}

// the answer of the PowerShell part of the scan (Get-StartApps + App Paths), as JSON
pub(crate) fn parse_registered(json: &str) -> Vec<IndexedApp> {
    try_parse_registered(json).unwrap_or_default()
}

pub(crate) fn try_parse_registered(json: &str) -> Result<Vec<IndexedApp>, String> {
    let text = json.trim().trim_start_matches('\u{feff}');
    // a null list (nothing registered) is written as null by ConvertTo-Json
    let text = text.replace("\"start\":null", "\"start\":[]").replace("\"paths\":null", "\"paths\":[]");
    let r = serde_json::from_str::<Registered>(&text).map_err(|e| format!("непонятный ответ PowerShell ({})", e))?;
    let mut out = Vec::new();
    let starts = r.start.into_vec();
    if !starts.is_empty() && starts.iter().all(|s| s.name.is_none() && s.app_id.is_none()) {
        return Err("непонятный ответ PowerShell (нет имён приложений)".into());
    }
    for s in starts {
        let name = s.name.unwrap_or_default().trim().to_string();
        let id = s.app_id.unwrap_or_default().trim().to_string();
        // web links and empty ids are not programs; an id with a space or a quote is not a safe AppsFolder name
        if name.is_empty() || id.is_empty() || is_junk(&name) || id.contains('"') || id.to_lowercase().starts_with("http") {
            continue;
        }
        let exe = Path::new(&id.replace('\\', "/")).file_name().map(|f| f.to_string_lossy().to_lowercase()).filter(|f| f.ends_with(".exe")).unwrap_or_default();
        out.push(IndexedApp { name, launch: format!("shell:AppsFolder\\{}", id), source: "registered".into(), exe });
    }
    for p in r.paths.into_vec() {
        let path = p.path.trim().trim_matches('"').to_string();
        let exe = p.exe.trim().to_lowercase();
        if path.is_empty() || !exe.ends_with(".exe") {
            continue;
        }
        let name = exe.trim_end_matches(".exe").to_string();
        out.push(IndexedApp { name, launch: path, source: "app_paths".into(), exe });
    }
    Ok(out)
}

// one list without repeats: the same program from several places is kept once (Start menu first)
pub(crate) fn merge(lists: Vec<Vec<IndexedApp>>) -> Vec<IndexedApp> {
    let mut out: Vec<IndexedApp> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for list in lists {
        for a in list {
            let key = name_key(&a.name);
            if key.is_empty() || !seen.insert(key) {
                // keep the program file name learned from another source
                if !a.exe.is_empty() {
                    if let Some(o) = out.iter_mut().find(|o| name_key(&o.name) == name_key(&a.name) && o.exe.is_empty()) {
                        o.exe = a.exe.clone();
                    }
                }
                continue;
            }
            out.push(a);
        }
    }
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn env_dir(var: &str, rest: &str) -> Option<PathBuf> {
    std::env::var(var).ok().map(|v| PathBuf::from(v).join(rest))
}

fn scan() -> Vec<IndexedApp> {
    let start_dirs: Vec<PathBuf> = [
        env_dir("APPDATA", "Microsoft\\Windows\\Start Menu\\Programs"),
        env_dir("ProgramData", "Microsoft\\Windows\\Start Menu\\Programs"),
    ]
    .into_iter()
    .flatten()
    .filter(|p| p.exists())
    .collect();
    let desktop_dirs: Vec<PathBuf> = [env_dir("USERPROFILE", "Desktop"), env_dir("PUBLIC", "Desktop"), env_dir("OneDrive", "Desktop")]
        .into_iter()
        .flatten()
        .filter(|p| p.exists())
        .collect();
    let shortcuts = scan_dirs(&start_dirs, &desktop_dirs);
    let registered = match registered_apps().and_then(|json| try_parse_registered(&json)) {
        Ok(list) => {
            LAST_ERROR.lock().clear();
            list
        }
        Err(e) => {
            warn!("Programs index: registered apps not read: {}", e);
            *LAST_ERROR.lock() = format!("список зарегистрированных приложений Windows не прочитан: {}", e);
            Vec::new()
        }
    };
    let (start, desk): (Vec<_>, Vec<_>) = shortcuts.into_iter().partition(|a| a.source == "start_menu");
    let (reg, paths): (Vec<_>, Vec<_>) = registered.into_iter().partition(|a| a.source == "registered");
    merge(vec![start, desk, reg, paths])
}

#[cfg(target_os = "windows")]
fn registered_apps() -> Result<String, String> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    const SCRIPT: &str = r#"[Console]::OutputEncoding = [Text.Encoding]::UTF8
$r = [ordered]@{ start = @(); paths = @() }
try { $r.start = @(Get-StartApps | Select-Object Name, AppID) } catch {}
foreach ($k in 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths') {
  Get-ChildItem $k -ErrorAction SilentlyContinue | ForEach-Object {
    $d = $_.GetValue('')
    if ($d) { $r.paths += [pscustomobject]@{ exe = $_.PSChildName; path = [Environment]::ExpandEnvironmentVariables(([string]$d).Trim('"')) } }
  }
}
$r | ConvertTo-Json -Depth 3 -Compress"#;
    let mut child = std::process::Command::new("powershell")
        // the script as UTF-16 base64: no quoting or line-break questions on the command line
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded_command(SCRIPT)])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000)
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut stdout = child.stdout.take().ok_or("no output")?;
    let reader = std::thread::spawn(move || {
        let mut s = Vec::new();
        let _ = stdout.read_to_end(&mut s);
        s
    });
    let started = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() > SCAN_TIMEOUT => {
                let _ = child.kill();
                return Err("PowerShell не ответил за 40 секунд".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let bytes = reader.join().unwrap_or_default();
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub(crate) fn encoded_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    crate::ai::base64(&bytes)
}

#[cfg(not(target_os = "windows"))]
fn registered_apps() -> Result<String, String> {
    Err("только в Windows".into())
}

// ---------------------------------------------------------------------------------------------
// finding a program by its spoken name
// ---------------------------------------------------------------------------------------------

// the words of a program name without the noise of versions and editions: "Telegram Desktop (x64) 4.2" -> telegram desktop
fn name_words(name: &str) -> Vec<String> {
    let mut clean = String::new();
    let mut depth = 0;
    for c in name.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth as i32 - 1).max(0) as usize,
            _ if depth == 0 => clean.push(c),
            _ => {}
        }
    }
    const NOISE: &[&str] = &["x64", "x86", "64", "32", "bit", "64bit", "32bit", "portable"];
    tokens(&clean)
        .into_iter()
        // years and versions ("2022", "365") are not said; a short number is part of the name ("7-Zip")
        .filter(|w| !NOISE.contains(&w.as_str()) && !(w.len() >= 3 && w.chars().all(|c| c.is_ascii_digit())))
        .collect()
}

// the phonetic key of a word; the silent final "e" of English words is dropped ("code" = "код", "adobe" = "адоб")
fn akey(w: &str) -> String {
    let chars: Vec<char> = w.chars().collect();
    let n = chars.len();
    if n >= 4 && chars.iter().all(|c| c.is_ascii_alphabetic()) && chars[n - 1] == 'e' && !"aeiouy".contains(chars[n - 2]) {
        return word_key(&w[..w.len() - 1]);
    }
    word_key(w)
}

fn name_key(name: &str) -> String {
    name_words(name).iter().map(|w| akey(w)).collect::<Vec<_>>().join(" ")
}

fn key_close(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let (ac, bc): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    edit_distance(&ac, &bc) <= allowed_edits(ac.len().min(bc.len()))
}

// 0: the same name; 1: the name begins with the spoken words ("телеграм" -> Telegram Desktop);
// 2: every spoken word is a word of the name ("командер" -> Total Commander). None: not this program.
fn score(query_keys: &[String], app: &IndexedApp) -> Option<u8> {
    let nk: Vec<String> = name_words(&app.name).iter().map(|w| akey(w)).collect();
    if nk.is_empty() || query_keys.is_empty() {
        return None;
    }
    if query_keys.join("") == nk.join("") {
        return Some(0);
    }
    if query_keys.len() == nk.len() && query_keys.iter().zip(&nk).all(|(q, n)| key_close(q, n)) {
        return Some(0);
    }
    if query_keys.len() < nk.len() && query_keys.iter().zip(&nk).all(|(q, n)| key_close(q, n)) {
        return Some(1);
    }
    // a word of one or two letters ("vs") must be a whole name or its beginning: it would fit too many names
    if query_keys.iter().all(|q| q.chars().count() >= 3 && nk.iter().any(|n| key_close(q, n))) {
        return Some(2);
    }
    None
}

#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    One(IndexedApp),
    Many(Vec<IndexedApp>),
    Nothing,
}

// the program these words name in this list
pub fn find_in(list: &[IndexedApp], words: &[String]) -> Found {
    if words.is_empty() || words.len() > 5 {
        return Found::Nothing;
    }
    let qk: Vec<String> = words.iter().map(|w| akey(w)).filter(|k| !k.is_empty()).collect();
    let mut scored: Vec<(u8, &IndexedApp)> = list.iter().filter_map(|a| score(&qk, a).map(|s| (s, a))).collect();
    if scored.is_empty() {
        return Found::Nothing;
    }
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.name.len().cmp(&b.1.name.len())));
    let best = scored[0].0;
    let top: Vec<IndexedApp> = scored.iter().filter(|(s, _)| *s == best).map(|(_, a)| (*a).clone()).collect();
    match top.len() {
        1 => Found::One(top[0].clone()),
        _ => Found::Many(top.into_iter().take(8).collect()),
    }
}

// for the routing of a phrase: never waits (the first scan may still run)
pub fn find_now(words: &[String]) -> Found {
    find_in(&apps(), words)
}

// a program of the index has exactly this name (for "закрой": a loose match must not take "закрой окно браузера")
pub fn has_exact(words: &[String]) -> bool {
    let qk: Vec<String> = words.iter().map(|w| akey(w)).filter(|k| !k.is_empty()).collect();
    apps().iter().any(|a| score(&qk, a) == Some(0))
}

pub fn index_pending() -> bool {
    ensure_loaded();
    INDEX.read().apps.is_empty() && is_scanning()
}

pub fn find(words: &[String]) -> Found {
    ensure_loaded();
    // the very first scan of this computer is still running: wait for it a little instead of saying "not found"
    for _ in 0..100 {
        if !INDEX.read().apps.is_empty() || !is_scanning() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    find_in(&apps(), words)
}

// the installed copy of a program of apps.rs ("winword.exe", "Total Commander"): its program file or its shortcut
pub fn locate_known(app: &crate::apps::AppDef) -> Option<String> {
    let list = apps();
    let start = app.start.to_lowercase();
    if start.ends_with(".exe") {
        if let Some(a) = list.iter().find(|a| a.exe == start && launchable(a)) {
            return Some(a.launch.clone());
        }
    }
    match find_in(&list, &tokens(app.name)) {
        Found::One(a) if launchable(&a) => Some(a.launch),
        _ => None,
    }
}

// the entry can still be started: a registered app, or a file that exists
pub fn launchable(a: &IndexedApp) -> bool {
    a.launch.starts_with("shell:AppsFolder\\") || Path::new(&a.launch).exists()
}

// the list of an ambiguous request, to be answered with a number or a name
pub fn set_pending(list: Vec<IndexedApp>) {
    *PENDING.lock() = list;
}

pub fn pending() -> Vec<IndexedApp> {
    PENDING.lock().clone()
}

// "второй", "2", "номер три", "тотал" -> the chosen entry of the pending list. Only a short answer counts: in
// "поставь таймер на 2 минуты" the number is not a choice, and "двадцать" is not "два".
pub fn choose(answer: &str) -> Option<IndexedApp> {
    let list = pending();
    if list.is_empty() {
        return None;
    }
    const FILLER: &[&str] = &["номер", "вариант", "давай", "открой", "запусти", "программу", "пожалуйста", "number", "the", "one", "open"];
    let words: Vec<String> = tokens(answer).into_iter().filter(|w| !FILLER.contains(&w.as_str())).collect();
    if words.is_empty() || words.len() > 3 {
        return None;
    }
    // a name first ("7 zip", "тотал")
    if let Found::One(a) = find_in(&list, &words) {
        return Some(a);
    }
    if words.len() != 1 {
        return None;
    }
    let w = words[0].as_str();
    const ORD: &[(&str, usize)] = &[
        ("один", 1), ("одна", 1), ("первый", 1), ("первую", 1), ("первая", 1), ("первое", 1), ("два", 2), ("две", 2), ("второй", 2), ("вторую", 2), ("вторая", 2),
        ("три", 3), ("третий", 3), ("третью", 3), ("третья", 3), ("четыре", 4), ("четвертый", 4), ("четвертую", 4), ("пять", 5), ("пятый", 5), ("пятую", 5),
        ("шесть", 6), ("шестой", 6), ("шестую", 6), ("семь", 7), ("седьмой", 7), ("седьмую", 7), ("восемь", 8), ("восьмой", 8), ("восьмую", 8),
        ("first", 1), ("second", 2), ("third", 3), ("fourth", 4), ("fifth", 5),
    ];
    let w = w.replace('ё', "е");
    let n = w.parse::<usize>().ok().or_else(|| ORD.iter().find(|(s, _)| *s == w).map(|(_, n)| *n))?;
    list.get(n.wrapping_sub(1)).cloned()
}

// start an entry of the index (never anything else): checked to exist first
// the program file of an entry when it can be known ("code.exe"): its own field, or the started file itself
pub fn exe_name(a: &IndexedApp) -> Option<String> {
    if !a.exe.is_empty() {
        return Some(a.exe.clone());
    }
    // (an App Paths entry is named after its registry key, "pbrush.exe" starting mspaint.exe: not a process name)
    if a.source == "app_paths" {
        return None;
    }
    let file = a.launch.rsplit(['\\', '/']).next().unwrap_or("");
    file.to_lowercase().ends_with(".exe").then(|| file.to_string())
}

pub fn launch(a: &IndexedApp) -> Result<(), String> {
    let in_index = apps().iter().any(|x| x.launch == a.launch) || pending().iter().any(|x| x.launch == a.launch);
    if !in_index {
        return Err("этой программы нет в списке найденных".into());
    }
    if !launchable(a) {
        return Err(format!("{}: файл запуска не найден, обновите список программ", a.name));
    }
    open(&a.launch)
}

#[cfg(all(feature = "lua", target_os = "windows", not(test)))]
fn open(target: &str) -> Result<(), String> {
    crate::lua::shell_open(target)
}

#[cfg(not(all(feature = "lua", target_os = "windows", not(test))))]
fn open(target: &str) -> Result<(), String> {
    info!("[not windows] would start {}", target);
    Ok(())
}

pub fn source_title(source: &str) -> &'static str {
    match source {
        "start_menu" => "меню «Пуск»",
        "desktop" => "рабочий стол",
        "registered" => "приложения Windows",
        "app_paths" => "реестр (App Paths)",
        _ => "другое",
    }
}

#[cfg(test)]
mod tests;
