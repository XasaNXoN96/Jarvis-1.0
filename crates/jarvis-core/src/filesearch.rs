// File search on this computer: "найди файл report", "найди все PDF-файлы", "найди фотографии за октябрь",
// "найди файл, содержащий текст ICTERA", "найди файл на диске D", "найди файл, который я недавно скачал".
//
// The phrase is turned into a query (parse): words of the name, extensions or a kind of file (документ, фото, …),
// where to look (the user's folders by default, a folder, a drive, or everywhere on request), dates, folders only,
// text inside. The search runs in the background (the answer arrives by itself: window + voice) with a time limit
// and can be cancelled. It never opens, moves or deletes anything by itself; the results are kept so that
// "открой второй", "покажи папку", "ещё" can use them.
//
// The whole computer is searched only when the user asks ("ищи везде", "на диске D"): when nothing is found in the
// usual folders, the answer offers it instead of scanning every drive. The text inside a file is read only for the
// formats that can really be read (plain text and code, Word/Excel/PowerPoint .docx/.xlsx/.pptx, OpenDocument);
// PDF and old .doc are not, and the answer says so.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

const MAX_RESULTS: usize = 200;
const PAGE: usize = 8;
const USER_LIMIT: Duration = Duration::from_secs(40);
const WIDE_LIMIT: Duration = Duration::from_secs(120);
const CONTENT_MAX_BYTES: u64 = 20 * 1024 * 1024;
const CONTENT_MAX_FILES: usize = 4000;

static GENERATION: AtomicU64 = AtomicU64::new(0);
static RESULTS: Lazy<Mutex<Vec<FoundItem>>> = Lazy::new(|| Mutex::new(Vec::new()));
static STATE: Lazy<Mutex<SearchState>> = Lazy::new(|| Mutex::new(SearchState::default()));
static HISTORY: Lazy<Mutex<VecDeque<Value>>> = Lazy::new(|| Mutex::new(VecDeque::new()));

#[derive(Clone, Debug)]
pub struct FoundItem {
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
    pub modified: u64, // unix seconds, 0 = unknown
    score: u8,         // 0 exact, 1 starts with, 2 contains, 3 all words match
}

#[derive(Default)]
struct SearchState {
    busy: bool,
    label: String,
    message: String,
    shown: usize, // how many results the last answer listed ("ещё" continues from here)
    last: Option<Query>,
    voice: bool,  // asked by voice: the answer is spoken and waits for "открой второй"; from the window it is only shown
}

pub fn results() -> Vec<FoundItem> {
    RESULTS.lock().clone()
}

// ---------------------------------------------------------------------------------------------
// the query
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Default)]
pub enum Scope {
    #[default]
    User,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Videos,
    Music,
    Drive(char),
    Everywhere,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Query {
    pub name: String,
    pub exts: Vec<String>,
    pub kind: String, // a readable word for the answer: "документы", "PDF"
    pub folders_only: bool,
    pub scope: Scope,
    pub since: Option<u64>,
    pub until: Option<u64>,
    pub recent: bool,
    pub content: String,
}

const KINDS: &[(&str, &str, &[&str])] = &[
    // (stem, readable, extensions)
    ("документ", "документы", &["doc", "docx", "pdf", "txt", "rtf", "odt", "xls", "xlsx", "ppt", "pptx", "md"]),
    ("презентац", "презентации", &["ppt", "pptx", "pps", "ppsx", "odp", "key"]),
    ("фото", "фотографии", &["jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "bmp", "tif", "tiff", "raw", "cr2", "nef", "dng"]),
    ("фотк", "фотографии", &["jpg", "jpeg", "png", "heic", "heif", "webp", "gif", "bmp"]),
    ("картинк", "картинки", &["jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "heic"]),
    ("изображен", "изображения", &["jpg", "jpeg", "png", "gif", "bmp", "webp", "svg", "heic", "tif", "tiff"]),
    ("снимк", "снимки", &["jpg", "jpeg", "png", "heic", "webp"]),
    ("скриншот", "скриншоты", &["png", "jpg", "jpeg"]),
    ("видео", "видео", &["mp4", "mkv", "avi", "mov", "wmv", "webm", "flv", "m4v"]),
    ("фильм", "видео", &["mp4", "mkv", "avi", "mov", "wmv", "webm", "m4v"]),
    ("ролик", "видео", &["mp4", "mkv", "avi", "mov", "webm"]),
    ("музык", "музыка", &["mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "opus"]),
    ("песн", "музыка", &["mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "opus"]),
    ("аудио", "аудио", &["mp3", "wav", "flac", "ogg", "m4a", "aac", "wma", "opus"]),
    ("таблиц", "таблицы", &["xls", "xlsx", "xlsm", "csv", "ods"]),
    ("архив", "архивы", &["zip", "rar", "7z", "tar", "gz", "bz2", "xz"]),
    ("ворд", "документы Word", &["doc", "docx", "rtf"]),
    ("эксел", "таблицы Excel", &["xls", "xlsx", "xlsm", "csv"]),
    ("photo", "фотографии", &["jpg", "jpeg", "png", "heic", "webp", "gif", "bmp"]),
    ("picture", "картинки", &["jpg", "jpeg", "png", "gif", "bmp", "webp", "svg"]),
    ("image", "изображения", &["jpg", "jpeg", "png", "gif", "bmp", "webp", "svg"]),
    ("document", "документы", &["doc", "docx", "pdf", "txt", "rtf", "odt", "xls", "xlsx", "ppt", "pptx"]),
    ("presentation", "презентации", &["ppt", "pptx", "odp", "key"]),
    ("video", "видео", &["mp4", "mkv", "avi", "mov", "wmv", "webm"]),
    ("music", "музыка", &["mp3", "wav", "flac", "ogg", "m4a"]),
    ("spreadsheet", "таблицы", &["xls", "xlsx", "csv", "ods"]),
];

// extensions that are said as words ("pdf", "пдф", "питон")
const EXT_WORDS: &[(&str, &str)] = &[
    ("пдф", "pdf"), ("пидиэф", "pdf"), ("питон", "py"), ("python", "py"), ("докс", "docx"), ("джейсон", "json"), ("жипег", "jpg"), ("джипег", "jpg"),
    ("эксе", "exe"), ("ехе", "exe"), ("зип", "zip"), ("мп3", "mp3"), ("мп4", "mp4"),
];

const KNOWN_EXTS: &[&str] = &[
    "pdf", "doc", "docx", "xls", "xlsx", "xlsm", "ppt", "pptx", "txt", "rtf", "odt", "ods", "odp", "csv", "md", "py", "js", "ts", "rs", "java", "cpp",
    "cs", "html", "htm", "css", "json", "xml", "yaml", "yml", "ini", "log", "sql", "zip", "rar", "7z", "exe", "msi", "mp3", "mp4", "avi", "mkv",
    "mov", "wav", "flac", "jpg", "jpeg", "png", "gif", "bmp", "webp", "heic", "svg", "psd", "iso", "apk", "bat", "ps1", "epub", "djvu", "fb2",
];

const FILLERS: &[&str] = &[
    "найди", "найти", "найдите", "поищи", "поиск", "искать", "ищи", "покажи", "покажите", "отыщи", "найдешь", "файл", "файлы", "файла", "файлов", "файлик",
    "с", "со", "под", "по", "названием", "название", "названии", "называется", "называемый", "имени", "именем", "имя", "слово", "слова", "словом",
    "который", "которая", "которые", "которого", "которой", "которых", "котором", "где", "есть", "все", "всё", "мои", "мой", "моя", "моих", "мне",
    "пожалуйста", "я", "мы", "и", "в", "во", "на", "из", "за", "о", "об", "про", "этот", "это", "тот", "какой", "нибудь", "нибуть", "изменил", "изменённые",
    "измененные", "измененный", "изменённый", "изменены", "созданные", "созданный", "последние", "последний", "новые", "этом", "компьютере",
    "компьютер", "моем", "моём", "пк", "компе", "год", "года", "году", "расширением", "расширения", "расширение", "формата", "формат", "типа", "тип",
    "find", "search", "for", "show", "me", "please", "file", "files", "named", "called", "with", "the", "a", "an", "all", "my", "in", "on", "of", "from",
    "that", "which", "name", "extension", "type", "recent", "recently", "modified", "changed", "computer", "pc", "and",
];

const MONTHS: &[(&str, u32)] = &[
    ("январ", 1), ("феврал", 2), ("март", 3), ("апрел", 4), ("июн", 6), ("июл", 7), ("август", 8), ("сентябр", 9), ("октябр", 10), ("ноябр", 11), ("декабр", 12),
    ("january", 1), ("february", 2), ("march", 3), ("april", 4), ("june", 6), ("july", 7), ("august", 8), ("september", 9), ("october", 10), ("november", 11), ("december", 12),
];

fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('ё', "е")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

fn drive_letter(w: &str) -> Option<char> {
    let c = w.chars().next()?;
    if w.chars().count() != 1 {
        return None;
    }
    let c = match c {
        'д' => 'd', 'с' => 'c', 'е' => 'e', 'э' => 'e', 'ф' => 'f', 'г' => 'g', 'х' => 'h', 'к' => 'k', 'з' => 'z', 'в' => 'v', 'а' => 'a', 'б' => 'b',
        c if c.is_ascii_alphabetic() => c.to_ascii_lowercase(),
        _ => return None,
    };
    Some(c.to_ascii_uppercase())
}

// local midnight of a date, as unix seconds
fn local_ts(y: i32, m: u32, d: u32) -> Option<u64> {
    use chrono::TimeZone;
    let dt = chrono::Local.with_ymd_and_hms(y, m, d, 0, 0, 0).earliest()?;
    u64::try_from(dt.timestamp()).ok()
}

fn month_range(y: i32, m: u32) -> (Option<u64>, Option<u64>) {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    (local_ts(y, m, 1), local_ts(ny, nm, 1))
}

// "найди фотографии за октябрь" -> Query { exts: [jpg, …], since/until: October }. `now` is the current local time.
pub fn parse(text: &str, now: chrono::DateTime<chrono::Local>) -> Query {
    use chrono::Datelike;
    let mut q = Query::default();
    let low = text.to_lowercase().replace('ё', "е");

    // the text inside the file: everything after the marker, as it was said
    const CONTENT: &[&str] = &[
        "содержащий текст", "содержащий слово", "содержащие текст", "содержащие слово", "содержащий", "содержащие", "содержащая", "содержит текст",
        "содержит слово", "содержит", "с текстом", "в котором есть текст", "в котором есть слово", "внутри которого", "где есть текст", "где написано",
        "в тексте которого", "containing the text", "containing the word", "containing", "with the text", "with text", "that contains",
    ];
    let mut head = low.clone();
    if let Some((pos, marker)) = CONTENT.iter().filter_map(|m| low.find(m).map(|p| (p, *m))).min_by_key(|(p, m)| (*p, usize::MAX - m.len())) {
        // the same position in the original text (lower case keeps the byte length for Russian and Latin letters)
        let orig_tail = text.get(pos + marker.len()..).unwrap_or(&low[pos + marker.len()..]);
        q.content = orig_tail.trim().trim_matches(|c: char| c == '«' || c == '»' || c == '"' || c == '\'' || c == '“' || c == '”' || c == ',' || c == '.').trim().to_string();
        head = low[..pos].to_string();
    }

    // ".py", ".docx" written with the dot
    for w in head.split_whitespace() {
        if let Some(e) = w.trim_matches(|c: char| c == ',' || c == '«' || c == '»' || c == '"').strip_prefix('.') {
            let e: String = e.chars().take_while(|c| c.is_alphanumeric()).collect();
            if !e.is_empty() && e.len() <= 5 && !q.exts.contains(&e) {
                q.exts.push(e);
            }
        }
    }

    let w = tokens(&head);
    let joined = format!(" {} ", w.join(" "));
    let mut used = vec![false; w.len()];
    let mark = |used: &mut Vec<bool>, i: usize| { if i < used.len() { used[i] = true; } };

    // where
    if joined.contains(" везде ") || joined.contains(" всему компьютеру ") || joined.contains(" всех дисках ") || joined.contains(" everywhere ") || joined.contains(" whole computer ") || joined.contains(" всем дискам ") {
        q.scope = Scope::Everywhere;
        for (i, t) in w.iter().enumerate() {
            if ["везде", "всему", "всех", "всем", "дисках", "дискам", "everywhere", "whole"].contains(&t.as_str()) { mark(&mut used, i); }
        }
    }
    for i in 0..w.len() {
        let t = w[i].as_str();
        let prev = if i > 0 { w[i - 1].as_str() } else { "" };
        let next = w.get(i + 1).map(|s| s.as_str()).unwrap_or("");
        let in_prep = prev == "в" || prev == "во" || prev == "на" || prev == "in" || prev == "on" || prev == "из";
        if (t.starts_with("рабоч") && next.starts_with("стол")) || t == "desktop" {
            q.scope = Scope::Desktop; mark(&mut used, i); if next.starts_with("стол") { mark(&mut used, i + 1); }
        } else if ["диске", "диск", "диска", "drive", "disk"].contains(&t) {
            if let Some(l) = drive_letter(next) {
                q.scope = Scope::Drive(l); mark(&mut used, i); mark(&mut used, i + 1);
            }
        } else if t.starts_with("загрузк") || t == "downloads" {
            q.scope = Scope::Downloads; mark(&mut used, i);
        } else if (t.starts_with("скача") || t.starts_with("загрузил") || t == "downloaded") && !t.starts_with("скачать") {
            q.scope = Scope::Downloads; q.recent = true; mark(&mut used, i);
        } else if in_prep && t.starts_with("документ") || t == "documents" && in_prep {
            q.scope = Scope::Documents; mark(&mut used, i);
        } else if in_prep && (t.starts_with("изображени") || t.starts_with("картинк")) || t == "pictures" && in_prep {
            q.scope = Scope::Pictures; mark(&mut used, i);
        } else if in_prep && t == "видео" || t == "videos" && in_prep {
            q.scope = Scope::Videos; mark(&mut used, i);
        } else if in_prep && t.starts_with("музык") {
            q.scope = Scope::Music; mark(&mut used, i);
        }
    }

    // when
    let today = local_ts(now.year(), now.month(), now.day());
    let now_ts = u64::try_from(now.timestamp()).unwrap_or(0);
    for i in 0..w.len() {
        let t = w[i].as_str();
        let prev = if i > 0 { w[i - 1].as_str() } else { "" };
        if t == "сегодня" || t == "today" || t.starts_with("сегодняшн") {
            q.since = today; mark(&mut used, i);
        } else if t == "вчера" || t == "yesterday" || t.starts_with("вчерашн") {
            q.since = today.map(|d| d - 86_400); q.until = today; mark(&mut used, i);
        } else if t.starts_with("недел") || t == "week" {
            q.since = Some(now_ts.saturating_sub(7 * 86_400)); mark(&mut used, i);
            if ["эту", "этой", "последнюю", "прошлую", "this", "last"].contains(&prev) { mark(&mut used, i - 1); }
        } else if t == "месяц" || t == "месяца" || t == "месяце" || t == "month" {
            q.since = Some(now_ts.saturating_sub(30 * 86_400)); mark(&mut used, i);
            if ["этот", "этом", "последний", "прошлый", "this", "last"].contains(&prev) { mark(&mut used, i - 1); }
        } else if t.starts_with("недавн") || t == "recent" || t == "recently" || t == "latest" {
            q.recent = true; q.since = Some(now_ts.saturating_sub(14 * 86_400)); mark(&mut used, i);
        } else if let Some(m) = MONTHS.iter().find(|(s, _)| t.starts_with(s)).map(|(_, m)| *m).or(if ["май", "мая", "мае", "may"].contains(&t) { Some(5) } else { None }) {
            // the year said after the month, else this year (or the last one for a month still ahead)
            let said_year = w.get(i + 1).and_then(|y| y.parse::<i32>().ok()).filter(|y| (1990..=2100).contains(y));
            let y = said_year.unwrap_or(if m > now.month() { now.year() - 1 } else { now.year() });
            let (a, b) = month_range(y, m);
            q.since = a; q.until = b; mark(&mut used, i);
            if said_year.is_some() { mark(&mut used, i + 1); }
        } else if let Ok(y) = t.parse::<i32>() {
            if (1990..=2100).contains(&y) && q.since.is_none() && (w.get(i + 1).map(|n| n.starts_with("год")).unwrap_or(false) || prev == "за" || prev == "в") {
                q.since = local_ts(y, 1, 1); q.until = local_ts(y + 1, 1, 1); mark(&mut used, i);
            }
        }
    }

    // what kind
    for i in 0..w.len() {
        if used[i] { continue; }
        let t = w[i].as_str();
        let prev = if i > 0 { w[i - 1].as_str() } else { "" };
        if t.starts_with("папк") || t == "folder" || t == "folders" || t.starts_with("каталог") {
            mark(&mut used, i);
            if prev != "в" && prev != "из" && prev != "in" { q.folders_only = true; }
            continue;
        }
        if let Some((_, e)) = EXT_WORDS.iter().find(|(s, _)| t == *s) {
            if !q.exts.contains(&e.to_string()) { q.exts.push(e.to_string()); }
            mark(&mut used, i);
            continue;
        }
        let ext_said = ["расширением", "расширения", "расширение", "формата", "формат", "extension", "типа"].contains(&prev);
        if KNOWN_EXTS.contains(&t) && (ext_said || q.exts.is_empty() && (joined.contains(" все ") || joined.contains(" файлы ") || joined.contains(" файл ") || joined.contains(" files ") || joined.contains(" all ") || i + 1 == w.len() || w.get(i + 1).map(|n| n.starts_with("файл")).unwrap_or(false))) {
            if !q.exts.contains(&t.to_string()) { q.exts.push(t.to_string()); }
            mark(&mut used, i);
            continue;
        }
        if let Some((_, readable, exts)) = KINDS.iter().find(|(s, _, _)| t.starts_with(s)) {
            for e in exts.iter() {
                if !q.exts.contains(&e.to_string()) { q.exts.push(e.to_string()); }
            }
            if q.kind.is_empty() { q.kind = readable.to_string(); }
            mark(&mut used, i);
        }
    }
    if q.kind.is_empty() && !q.exts.is_empty() {
        q.kind = q.exts.iter().map(|e| e.to_uppercase()).collect::<Vec<_>>().join(", ");
    }

    // the name: what is left
    let rest: Vec<String> = w.iter().enumerate().filter(|(i, t)| !used[*i] && !FILLERS.contains(&t.as_str())).map(|(_, t)| t.clone()).collect();
    q.name = digits_from_words(&rest).unwrap_or_else(|| rest.join(" "));
    q
}

// "один два три" -> "123" (a file name made of digits, said one by one)
fn digits_from_words(words: &[String]) -> Option<String> {
    const D: &[(&str, char)] = &[("ноль", '0'), ("один", '1'), ("два", '2'), ("три", '3'), ("четыре", '4'), ("пять", '5'), ("шесть", '6'), ("семь", '7'), ("восемь", '8'), ("девять", '9')];
    if words.len() < 2 {
        return None;
    }
    words.iter().map(|w| D.iter().find(|(s, _)| s == w).map(|(_, c)| *c)).collect()
}

// what the query is, in words, for the answer: "PDF в Загрузках, за октябрь"
pub fn describe(q: &Query) -> String {
    let mut parts = Vec::new();
    if !q.name.is_empty() {
        parts.push(format!("«{}»", q.name));
    }
    if q.folders_only {
        parts.push("папки".into());
    } else if !q.kind.is_empty() {
        parts.push(q.kind.clone());
    }
    if !q.content.is_empty() {
        parts.push(format!("с текстом «{}»", q.content));
    }
    parts.push(match &q.scope {
        Scope::User => "в ваших папках".into(),
        Scope::Desktop => "на рабочем столе".into(),
        Scope::Documents => "в Документах".into(),
        Scope::Downloads => "в Загрузках".into(),
        Scope::Pictures => "в Изображениях".into(),
        Scope::Videos => "в Видео".into(),
        Scope::Music => "в Музыке".into(),
        Scope::Drive(l) => format!("на диске {}:", l),
        Scope::Everywhere => "на всём компьютере".into(),
    });
    if q.recent {
        parts.push("недавние".into());
    } else if q.since.is_some() || q.until.is_some() {
        parts.push("за выбранное время".into());
    }
    parts.join(", ")
}

fn is_empty_query(q: &Query) -> bool {
    q.name.is_empty() && q.exts.is_empty() && q.content.is_empty() && !q.folders_only && q.since.is_none() && !q.recent && q.scope == Scope::User
}

// ---------------------------------------------------------------------------------------------
// matching
// ---------------------------------------------------------------------------------------------

fn skip_dir(name_lower: &str) -> bool {
    matches!(
        name_lower,
        "windows" | "program files" | "program files (x86)" | "programdata" | "$recycle.bin" | "system volume information" | "appdata"
            | "node_modules" | ".git" | "$windows.~bt" | "$windows.~ws" | "winsxs" | "windowsapps" | "proc" | "sys" | "dev" | "usr" | "lib"
            | "lib64" | "bin" | "sbin" | "etc" | "var" | "run" | "snap" | "boot" | "target" | "__pycache__" | ".cache"
    )
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() { row[0] = i; }
    for j in 0..=b.len() { d[0][j] = j; }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

fn words_match(name: &str, qtokens: &[String]) -> bool {
    if qtokens.is_empty() {
        return false;
    }
    let ntokens = tokens(name);
    qtokens.iter().all(|q| {
        ntokens.iter().any(|n| {
            n == q
                || (q.chars().count() >= 3 && n.contains(q.as_str()))
                || (q.chars().count() >= 5 && n.chars().count() >= 4 && edit_distance(q, n) <= 1)
        })
    })
}

// 0 exact name, 1 starts with the query, 2 contains the query, 3 all words are there
fn score(name: &str, query: &str, qtokens: &[String]) -> Option<u8> {
    if query.is_empty() {
        return Some(2);
    }
    let name_l = name.to_lowercase().replace('ё', "е");
    let stem_l = Path::new(&name_l).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| name_l.clone());
    if name_l == query || stem_l == query {
        Some(0)
    } else if stem_l.starts_with(query) {
        Some(1)
    } else if name_l.contains(query) {
        Some(2)
    } else if words_match(name, qtokens) {
        Some(3)
    } else {
        None
    }
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

const TEXT_EXTS: &[&str] = &[
    "txt", "md", "csv", "log", "json", "xml", "html", "htm", "py", "js", "ts", "rs", "java", "c", "cpp", "h", "hpp", "cs", "go", "php", "rb", "kt",
    "swift", "ini", "cfg", "conf", "yaml", "yml", "toml", "sql", "bat", "cmd", "ps1", "sh", "srt", "tex", "css", "lua", "rtf",
];
const ZIP_DOCS: &[&str] = &["docx", "xlsx", "xlsm", "pptx", "odt", "ods", "odp"];

pub fn content_supported(ext: &str) -> bool {
    TEXT_EXTS.contains(&ext) || ZIP_DOCS.contains(&ext)
}

// the text of a file, for the formats that can really be read; None when it cannot be read
fn file_text(path: &Path, ext: &str) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > CONTENT_MAX_BYTES {
        return None;
    }
    if TEXT_EXTS.contains(&ext) {
        let bytes = std::fs::read(path).ok()?;
        if bytes.len() >= 2 && bytes[0] == 0xFF && bytes[1] == 0xFE {
            let u: Vec<u16> = bytes[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            return Some(String::from_utf16_lossy(&u));
        }
        return Some(String::from_utf8_lossy(&bytes).to_string());
    }
    if ZIP_DOCS.contains(&ext) {
        let bytes = std::fs::read(path).ok()?;
        let wanted = |n: &str| match ext {
            "docx" => n == "word/document.xml" || n.starts_with("word/header") || n.starts_with("word/footer"),
            "xlsx" | "xlsm" => n == "xl/sharedStrings.xml" || n.starts_with("xl/worksheets/sheet"),
            "pptx" => n.starts_with("ppt/slides/slide") && n.ends_with(".xml"),
            _ => n == "content.xml",
        };
        let mut text = String::new();
        for (name, data) in zip_entries(&bytes, &wanted) {
            let _ = name;
            text.push_str(&strip_xml(&String::from_utf8_lossy(&data)));
            text.push('\n');
        }
        return Some(text);
    }
    None
}

// the entries of a zip archive (Office documents) whose names are wanted: stored or deflated
fn zip_entries(bytes: &[u8], wanted: &dyn Fn(&str) -> bool) -> Vec<(String, Vec<u8>)> {
    let u16le = |o: usize| -> Option<usize> { bytes.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize) };
    let u32le = |o: usize| -> Option<usize> { bytes.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize) };
    let mut out = Vec::new();
    // the end of central directory record, searched from the end
    let Some(eocd) = (0..bytes.len().saturating_sub(21)).rev().take(70_000).find(|&i| bytes[i..].starts_with(&[0x50, 0x4b, 0x05, 0x06])) else { return out };
    let (Some(count), Some(mut p)) = (u16le(eocd + 10), u32le(eocd + 16)) else { return out };
    for _ in 0..count.min(5000) {
        if !bytes.get(p..).map(|b| b.starts_with(&[0x50, 0x4b, 0x01, 0x02])).unwrap_or(false) {
            break;
        }
        let (Some(method), Some(csize), Some(nlen), Some(elen), Some(clen), Some(local)) =
            (u16le(p + 10), u32le(p + 20), u16le(p + 28), u16le(p + 30), u16le(p + 32), u32le(p + 42)) else { break };
        let name = String::from_utf8_lossy(bytes.get(p + 46..p + 46 + nlen).unwrap_or_default()).to_string();
        p += 46 + nlen + elen + clen;
        if !wanted(&name) {
            continue;
        }
        let (Some(lnlen), Some(lelen)) = (u16le(local + 26), u16le(local + 28)) else { continue };
        let start = local + 30 + lnlen + lelen;
        let Some(data) = bytes.get(start..start + csize) else { continue };
        let raw = match method {
            0 => data.to_vec(),
            8 => {
                let mut v = Vec::new();
                if flate2::read::DeflateDecoder::new(data).take(CONTENT_MAX_BYTES * 4).read_to_end(&mut v).is_err() {
                    continue;
                }
                v
            }
            _ => continue,
        };
        out.push((name, raw));
    }
    out
}

fn strip_xml(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len() / 2);
    let mut in_tag = false;
    for c in xml.chars() {
        match c {
            '<' => { in_tag = true; out.push(' '); }
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

fn text_contains(hay: &str, needle_lower: &str) -> bool {
    let h = hay.to_lowercase().replace('ё', "е");
    if h.contains(needle_lower) {
        return true;
    }
    // Word splits words into runs (<w:t>Ex</w:t><w:t>am</w:t>): compare without spaces too
    let squeeze = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    squeeze(&h).contains(&squeeze(needle_lower))
}

// ---------------------------------------------------------------------------------------------
// the search
// ---------------------------------------------------------------------------------------------

struct Search {
    q: Query,
    name_l: String,
    qtokens: Vec<String>,
    content_l: String,
    generation: u64,
    found: Vec<FoundItem>,
    deadline: Option<Instant>,
    denied: usize,
    content_checked: usize,
    content_unsupported: usize,
    timed_out: bool,
}

impl Search {
    fn new(q: Query, generation: u64) -> Search {
        let name_l = q.name.to_lowercase().replace('ё', "е");
        Search {
            qtokens: tokens(&name_l),
            content_l: q.content.to_lowercase().replace('ё', "е"),
            name_l,
            q,
            generation,
            found: Vec::new(),
            deadline: None,
            denied: 0,
            content_checked: 0,
            content_unsupported: 0,
            timed_out: false,
        }
    }

    fn cancelled(&mut self) -> bool {
        if GENERATION.load(Ordering::SeqCst) != self.generation {
            return true;
        }
        if self.deadline.map(|d| Instant::now() > d).unwrap_or(false) {
            self.timed_out = true;
            return true;
        }
        false
    }

    fn consider(&mut self, path: &Path, name: &str, is_dir: bool) {
        if self.q.folders_only && !is_dir {
            return;
        }
        if !self.q.exts.is_empty() && (is_dir || !self.q.exts.contains(&ext_of(path))) {
            return;
        }
        if !self.q.content.is_empty() && is_dir {
            return;
        }
        let Some(s) = score(name, &self.name_l, &self.qtokens) else { return };
        let meta = std::fs::metadata(path).ok();
        let modified = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0);
        if let Some(since) = self.q.since {
            if modified < since {
                return;
            }
        }
        if let Some(until) = self.q.until {
            if modified == 0 || modified >= until {
                return;
            }
        }
        if !self.q.content.is_empty() {
            let ext = ext_of(path);
            if !content_supported(&ext) {
                self.content_unsupported += 1;
                return;
            }
            if self.content_checked >= CONTENT_MAX_FILES {
                return;
            }
            self.content_checked += 1;
            match file_text(path, &ext) {
                Some(t) if text_contains(&t, &self.content_l) => {}
                _ => return,
            }
        }
        let size = if is_dir { 0 } else { meta.map(|m| m.len()).unwrap_or(0) };
        self.found.push(FoundItem { path: path.to_path_buf(), is_dir, size, modified, score: s });
    }

    // breadth-first: shallow matches (the ones you usually want) come first
    fn walk(&mut self, root: &Path) {
        let mut queue: VecDeque<PathBuf> = VecDeque::new();
        queue.push_back(root.to_path_buf());
        while let Some(dir) = queue.pop_front() {
            if self.cancelled() || self.found.len() >= MAX_RESULTS * 5 {
                return;
            }
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::PermissionDenied {
                        self.denied += 1;
                    }
                    continue;
                }
            };
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                let path = entry.path();
                let ft = entry.file_type().ok();
                let is_dir = ft.map(|t| t.is_dir()).unwrap_or(false);
                // links to folders are not followed (no endless loops through junctions)
                let is_link = ft.map(|t| t.is_symlink()).unwrap_or(false);
                self.consider(&path, &name, is_dir);
                if is_dir && !is_link && !skip_dir(&name.to_lowercase()) && !name.starts_with('$') {
                    queue.push_back(path);
                }
            }
        }
    }
}

fn home() -> Option<PathBuf> {
    std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok().map(PathBuf::from)
}

// a known folder of the user, also inside OneDrive (Windows often moves Desktop / Documents there)
fn user_folder(names: &[&str]) -> Vec<PathBuf> {
    let Some(h) = home() else { return Vec::new() };
    let mut out = Vec::new();
    for n in names {
        for p in [h.join(n), h.join("OneDrive").join(n)] {
            if p.is_dir() && !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}

fn all_drives() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    #[cfg(target_os = "windows")]
    for letter in b'C'..=b'Z' {
        let root = PathBuf::from(format!("{}:\\", letter as char));
        if root.exists() {
            roots.push(root);
        }
    }
    roots
}

// the folders of a scope; Err when the place does not exist
pub fn roots(scope: &Scope) -> Result<Vec<PathBuf>, String> {
    let r = match scope {
        Scope::User => {
            let mut v = user_folder(&["Desktop", "Documents", "Downloads", "Pictures", "Videos", "Music"]);
            if let Some(h) = home() {
                let od = h.join("OneDrive");
                if od.is_dir() {
                    v.push(od);
                }
            }
            v
        }
        Scope::Desktop => user_folder(&["Desktop", "Рабочий стол"]),
        Scope::Documents => user_folder(&["Documents", "Документы"]),
        Scope::Downloads => user_folder(&["Downloads", "Загрузки"]),
        Scope::Pictures => user_folder(&["Pictures", "Изображения"]),
        Scope::Videos => user_folder(&["Videos", "Видео"]),
        Scope::Music => user_folder(&["Music", "Музыка"]),
        Scope::Drive(l) => {
            let root = PathBuf::from(format!("{}:\\", l));
            if !cfg!(target_os = "windows") || !root.exists() {
                return Err(format!("Диска {}: нет на этом компьютере", l));
            }
            if std::fs::read_dir(&root).is_err() {
                return Err(format!("Диск {}: недоступен (нет прав или диск не готов)", l));
            }
            vec![root]
        }
        Scope::Everywhere => {
            let mut v: Vec<PathBuf> = home().into_iter().collect();
            v.extend(all_drives());
            v
        }
    };
    if r.is_empty() {
        return Err("Эта папка не найдена на компьютере".into());
    }
    Ok(r)
}

fn rank(mut items: Vec<FoundItem>, by_date: bool) -> Vec<FoundItem> {
    items.sort_by(|a, b| {
        let primary = if by_date { b.modified.cmp(&a.modified) } else { a.score.cmp(&b.score) };
        primary
            .then(a.score.cmp(&b.score))
            .then(a.path.components().count().cmp(&b.path.components().count()))
            .then(a.path.cmp(&b.path))
    });
    items.dedup_by(|a, b| a.path == b.path);
    items.truncate(MAX_RESULTS);
    items
}

// runs the whole search of a query (blocking); used by the background thread and the tests
fn run(q: &Query, generation: u64) -> Result<(Vec<FoundItem>, Search), String> {
    let dirs = roots(&q.scope)?;
    let mut s = Search::new(q.clone(), generation);
    s.deadline = Some(Instant::now() + if matches!(q.scope, Scope::Everywhere | Scope::Drive(_)) { WIDE_LIMIT } else { USER_LIMIT });
    for d in &dirs {
        s.walk(d);
        if s.cancelled() {
            break;
        }
    }
    let by_date = q.recent || q.name.is_empty();
    let found = rank(std::mem::take(&mut s.found), by_date);
    Ok((found, s))
}

fn say(text: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{}", text);
    #[cfg(feature = "jarvis_app")]
    crate::ipc::send(crate::ipc::IpcEvent::Output { text: text.to_string() });
}

fn reply(text: &str, spoken: &str) {
    crate::dialog::reply(text, "command");
    say(text);
    if !spoken.is_empty() {
        crate::tts::speak(spoken);
    }
}

pub fn human_size(n: u64) -> String {
    const U: &[&str] = &["Б", "КБ", "МБ", "ГБ", "ТБ"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < U.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{} {}", n, U[0]) } else { format!("{:.1} {}", v, U[i]).replace('.', ",") }
}

pub fn human_date(ts: u64) -> String {
    use chrono::TimeZone;
    if ts == 0 {
        return "дата неизвестна".into();
    }
    chrono::Local.timestamp_opt(ts as i64, 0).single().map(|d| d.format("%d.%m.%Y %H:%M").to_string()).unwrap_or_default()
}

fn item_line(i: usize, item: &FoundItem) -> String {
    let name = item.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let dir = item.path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    let what = if item.is_dir { "папка".to_string() } else {
        let e = ext_of(&item.path);
        format!("{}{}", if e.is_empty() { "файл".to_string() } else { e.to_uppercase() }, format!(", {}", human_size(item.size)))
    };
    format!("{}. {} — {}, изменён {} ({})", i + 1, name, what, human_date(item.modified), dir)
}

// the next page of the last results ("ещё")
pub fn more() -> String {
    let list = results();
    let mut st = STATE.lock();
    if list.is_empty() {
        return "Сначала найдите файл: «найди файл …»".into();
    }
    if st.shown >= list.len() {
        return format!("Больше результатов нет (всего {})", list.len());
    }
    let from = st.shown;
    let to = (from + PAGE).min(list.len());
    st.shown = to;
    let mut lines = vec![format!("Результаты {}–{} из {}:", from + 1, to, list.len())];
    lines.extend(list[from..to].iter().enumerate().map(|(i, it)| item_line(from + i, it)));
    lines.join("\n")
}

fn announce(q: &Query, items: &[FoundItem], s: &Search, voice: bool) {
    let what = describe(q);
    let mut notes = Vec::new();
    if s.timed_out {
        notes.push("Поиск остановлен по времени: показано то, что успел найти.".to_string());
    }
    if s.denied > 0 {
        notes.push(format!("Папок без доступа (пропущены, права не обходятся): {}.", s.denied));
    }
    if !q.content.is_empty() && s.content_unsupported > 0 {
        notes.push("По тексту проверены только текстовые файлы, .docx, .xlsx, .pptx и OpenDocument; PDF, старые .doc и картинки по содержимому не проверяются.".into());
    }
    let notes_text = if notes.is_empty() { String::new() } else { format!("\n{}", notes.join("\n")) };
    let mut st = STATE.lock();
    if items.is_empty() {
        let hint = match q.scope {
            Scope::User => " Скажите «ищи везде», чтобы искать на всех дисках, или «найди … на диске D».",
            Scope::Everywhere => "",
            _ => " Скажите «ищи везде», чтобы искать на всём компьютере.",
        };
        let msg = format!("Не нашёл: {}.{}{}", what, hint, notes_text);
        st.message = msg.clone();
        st.shown = 0;
        drop(st);
        if voice {
            reply(&msg, if q.scope == Scope::User { "Не нашёл в ваших папках. Скажите: ищи везде" } else { "Ничего не нашёл" });
        }
        return;
    }
    let shown = items.len().min(PAGE);
    st.shown = shown;
    let mut lines = vec![format!("Нашёл {}: {}{}", items.len(), what, if items.len() > shown { format!(" (показаны 1–{})", shown) } else { String::new() })];
    lines.extend(items[..shown].iter().enumerate().map(|(i, it)| item_line(i, it)));
    let msg = format!("{}{}", lines.join("\n"), notes_text);
    st.message = msg.clone();
    drop(st);
    let spoken = if items.len() == 1 {
        "Нашёл один. Открыть или показать папку?".to_string()
    } else {
        format!("Нашёл {}. Скажите номер, открой, покажи папку или ещё.", items.len())
    };
    if !voice {
        return; // the "Файлы" page shows the list with its own buttons
    }
    reply(&msg, &spoken);
    // the answer ("открой", "второй", "покажи папку", "ещё") is taken without the wake word
    crate::session::start("file", 60);
    crate::session::set_data(if items.len() == 1 { "1" } else { "" });
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn remember(kind: &str, text: &str, result: &str) {
    let mut h = HISTORY.lock();
    h.push_front(json!({ "time": now_secs(), "kind": kind, "text": text.chars().take(120).collect::<String>(), "result": result }));
    h.truncate(30);
}

// the history of searches and program starts (this session of Jarvis)
pub fn history() -> Vec<Value> {
    HISTORY.lock().iter().cloned().collect()
}

pub fn note_launch(name: &str, ok: bool) {
    remember("launch", name, if ok { "запущено" } else { "не запустилось" });
}

// Starts a background search for a spoken or typed phrase. A new search cancels the previous one.
// Returns the description of the query, or Err when there is nothing to search for.
pub fn start(text: &str) -> Result<String, String> {
    let q = parse(text, chrono::Local::now());
    start_with(q, true)
}

// a search from the "Файлы" page: shown in the window, not spoken
pub fn start_query(q: Query) -> Result<String, String> {
    start_with(q, false)
}

fn start_with(q: Query, voice: bool) -> Result<String, String> {
    if is_empty_query(&q) {
        return Err("Что искать? Например: «найди файл отчёт» или «найди все PDF»".into());
    }
    if let Err(e) = roots(&q.scope) {
        // the window shows the reason instead of the results of an older search
        let mut st = STATE.lock();
        st.busy = false;
        st.label = describe(&q);
        st.message = e.clone();
        RESULTS.lock().clear();
        return Err(e);
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let what = describe(&q);
    info!("File search started: {}", what);
    {
        let mut st = STATE.lock();
        st.busy = true;
        st.label = what.clone();
        st.message = String::new();
        st.last = Some(q.clone());
        st.voice = voice;
    }
    std::thread::spawn(move || {
        let started = Instant::now();
        let outcome = run(&q, generation);
        if GENERATION.load(Ordering::SeqCst) != generation {
            return; // a newer search replaced this one, or it was cancelled
        }
        match outcome {
            Ok((found, s)) => {
                *RESULTS.lock() = found.clone();
                remember("search", &describe(&q), &format!("найдено {} за {} с", found.len(), started.elapsed().as_secs()));
                STATE.lock().busy = false;
                announce(&q, &found, &s, voice);
            }
            Err(e) => {
                let mut st = STATE.lock();
                st.busy = false;
                st.message = e.clone();
                drop(st);
                remember("search", &describe(&q), &e);
                if voice {
                    reply(&e, &e);
                }
            }
        }
    });
    Ok(what)
}

// "ищи везде": the last query again, on the whole computer
pub fn widen() -> Result<String, String> {
    let last = STATE.lock().last.clone();
    let Some(mut q) = last else { return Err("Сначала скажите, что найти: «найди файл …»".into()) };
    q.scope = Scope::Everywhere;
    start_with(q, true)
}

// "останови поиск"
pub fn cancel() -> bool {
    let mut st = STATE.lock();
    let was = st.busy;
    GENERATION.fetch_add(1, Ordering::SeqCst);
    st.busy = false;
    if was {
        st.message = "Поиск остановлен".into();
    }
    was
}

pub fn is_busy() -> bool {
    STATE.lock().busy
}

// for the "Файлы" page of the window
pub fn state_json() -> Value {
    let st = STATE.lock();
    let list = RESULTS.lock();
    let rows: Vec<Value> = list
        .iter()
        .take(MAX_RESULTS)
        .map(|it| {
            json!({
                "name": it.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
                "dir": it.path.parent().map(|p| p.display().to_string()).unwrap_or_default(),
                "ext": ext_of(&it.path),
                "is_dir": it.is_dir,
                "size": human_size(it.size),
                "modified": human_date(it.modified),
            })
        })
        .collect();
    json!({ "busy": st.busy, "label": st.label, "message": st.message, "results": rows, "history": history() })
}

// the n-th result (1-based): open it, or show it in Explorer selected. Only results of the last search.
pub fn result_path(n: usize) -> Option<PathBuf> {
    results().get(n.wrapping_sub(1)).map(|i| i.path.clone())
}

// open the n-th result (1-based) or show it selected in Explorer; only results of the last search, checked to exist
pub fn open_result(n: usize, reveal: bool) -> Result<String, String> {
    let path = result_path(n).ok_or("такого результата нет")?;
    if !path.exists() {
        return Err(format!("{} больше нет на месте", path.display()));
    }
    let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    if reveal {
        reveal_in_explorer(&path)?;
    } else {
        open_path(&path)?;
    }
    remember("open", &name, if reveal { "показан в папке" } else { "открыт" });
    Ok(name)
}

#[cfg(all(target_os = "windows", not(test)))]
fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // explorer reads `/select,"path"` itself; no shell is involved and a path cannot contain a quote on Windows
    std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", path.display())).spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(not(all(target_os = "windows", not(test))))]
fn reveal_in_explorer(path: &Path) -> Result<(), String> {
    info!("[not windows] would show {} in the file manager", path.display());
    Ok(())
}

#[cfg(all(feature = "lua", target_os = "windows", not(test)))]
fn open_path(path: &Path) -> Result<(), String> {
    crate::lua::shell_open(&path.to_string_lossy())
}

#[cfg(not(all(feature = "lua", target_os = "windows", not(test))))]
fn open_path(path: &Path) -> Result<(), String> {
    info!("[not windows] would open {}", path.display());
    Ok(())
}

// a query from the fields of the "Файлы" page
pub fn query_from_form(name: &str, kind: &str, scope: &str, when: &str, content: &str) -> Query {
    let now = chrono::Local::now();
    let mut q = Query { name: name.trim().to_lowercase(), content: content.trim().to_string(), ..Query::default() };
    if let Some(e) = kind.strip_prefix('.') {
        q.exts = vec![e.to_lowercase()];
        q.kind = e.to_uppercase();
    } else if kind == "folder" {
        q.folders_only = true;
    } else if let Some((_, readable, exts)) = KINDS.iter().find(|(s, _, _)| *s == kind) {
        q.exts = exts.iter().map(|e| e.to_string()).collect();
        q.kind = readable.to_string();
    }
    q.scope = match scope {
        "desktop" => Scope::Desktop,
        "documents" => Scope::Documents,
        "downloads" => Scope::Downloads,
        "pictures" => Scope::Pictures,
        "videos" => Scope::Videos,
        "music" => Scope::Music,
        "everywhere" => Scope::Everywhere,
        s if s.len() == 1 => drive_letter(s).map(Scope::Drive).unwrap_or_default(),
        _ => Scope::User,
    };
    let p = parse(when, now);
    q.since = p.since;
    q.until = p.until;
    q.recent = p.recent;
    q
}

#[cfg(all(test, not(target_os = "windows")))]
pub(crate) fn run_for_tests(q: &Query) -> Result<Vec<FoundItem>, String> {
    run(q, GENERATION.load(Ordering::SeqCst)).map(|(f, _)| f)
}

#[cfg(test)]
pub(crate) fn set_results_for_tests(items: Vec<PathBuf>) {
    *RESULTS.lock() = items.into_iter().map(|p| FoundItem { is_dir: p.is_dir(), path: p, size: 0, modified: 0, score: 0 }).collect();
    STATE.lock().shown = PAGE.min(RESULTS.lock().len());
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

#[cfg(test)]
mod tests;
