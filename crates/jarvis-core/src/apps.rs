// Names of programs as people say them: "ворд", "ворт" (what the recognizer hears), "Word", "Тотал командер".
// Every program has a list of spoken variants; both the variants and the heard text are reduced to a phonetic key
// (Latin letters are read as Russian ones, voiced/unvoiced endings and double letters are merged), so
// "word" = "ворд" = "ворт". A fuzzy match is used only after the exact one and only for a whole name,
// with a limit on the number of different letters and a margin over the second best program,
// so a loose match can never start another program.

use std::path::Path;

pub struct AppDef {
    pub id: &'static str,
    pub name: &'static str,     // how Jarvis calls it
    pub aliases: &'static [&'static str],
    pub start: &'static str,    // what to launch (an exe name from App Paths, or a protocol)
    pub process: Option<&'static str>, // for "закрой"
    pub paths: &'static [&'static str], // typical install locations (%VAR% is expanded)
    pub office: Option<&'static str>,  // "word" | "excel" | "notepad": dictation target
}

pub const APPS: &[AppDef] = &[
    AppDef { id: "notepad", name: "Блокнот", aliases: &["блокнот", "блокнотик", "notepad", "ноутпад", "нотпад", "текстовый редактор"], start: "notepad.exe", process: Some("notepad.exe"), paths: &[], office: Some("notepad") },
    AppDef { id: "calc", name: "Калькулятор", aliases: &["калькулятор", "calculator", "calc", "калькулятор виндовс"], start: "calc.exe", process: Some("CalculatorApp.exe"), paths: &[], office: None },
    AppDef { id: "paint", name: "Paint", aliases: &["пейнт", "пеинт", "paint", "mspaint", "рисовалка", "пэйнт"], start: "mspaint.exe", process: Some("mspaint.exe"), paths: &[], office: None },
    AppDef { id: "explorer", name: "Проводник", aliases: &["проводник", "explorer", "эксплорер", "файловый менеджер", "мой компьютер", "этот компьютер"], start: "explorer.exe", process: None, paths: &[], office: None },
    AppDef { id: "taskmgr", name: "Диспетчер задач", aliases: &["диспетчер задач", "диспетчер", "task manager", "таск менеджер", "taskmgr"], start: "taskmgr.exe", process: Some("taskmgr.exe"), paths: &[], office: None },
    AppDef { id: "cmd", name: "Командная строка", aliases: &["командная строка", "командную строку", "командную", "cmd", "сиэмди", "терминал", "terminal", "консоль"], start: "cmd.exe", process: None, paths: &[], office: None },
    AppDef { id: "powershell", name: "PowerShell", aliases: &["powershell", "пауэршелл", "паувершелл", "повершелл", "павершел", "пауэр шелл", "power shell"], start: "powershell.exe", process: None, paths: &[], office: None },
    AppDef { id: "settings", name: "Настройки", aliases: &["настройки", "параметры", "settings", "параметры виндовс", "настройки виндовс"], start: "ms-settings:", process: None, paths: &[], office: None },
    AppDef { id: "control", name: "Панель управления", aliases: &["панель управления", "панель", "control panel", "контрол панель"], start: "control.exe", process: None, paths: &[], office: None },
    AppDef { id: "word", name: "Word", aliases: &["ворд", "ворт", "word", "ворд", "вордс", "microsoft word", "майкрософт ворд", "ms word", "майкрософт ворт", "эмэс ворд", "вурд", "ворде"], start: "winword.exe", process: Some("WINWORD.EXE"), paths: &["%ProgramFiles%\\Microsoft Office\\root\\Office16\\WINWORD.EXE", "%ProgramFiles(x86)%\\Microsoft Office\\root\\Office16\\WINWORD.EXE", "%ProgramFiles%\\Microsoft Office\\Office16\\WINWORD.EXE", "%ProgramFiles%\\Microsoft Office\\Office15\\WINWORD.EXE"], office: Some("word") },
    AppDef { id: "excel", name: "Excel", aliases: &["эксель", "эксел", "ексел", "excel", "ексель", "эксэль", "microsoft excel", "майкрософт эксель", "ms excel", "экзель", "ексэль", "эксил"], start: "excel.exe", process: Some("EXCEL.EXE"), paths: &["%ProgramFiles%\\Microsoft Office\\root\\Office16\\EXCEL.EXE", "%ProgramFiles(x86)%\\Microsoft Office\\root\\Office16\\EXCEL.EXE", "%ProgramFiles%\\Microsoft Office\\Office16\\EXCEL.EXE", "%ProgramFiles%\\Microsoft Office\\Office15\\EXCEL.EXE"], office: Some("excel") },
    AppDef { id: "powerpoint", name: "PowerPoint", aliases: &["пауэрпоинт", "powerpoint", "паверпоинт", "повер поинт", "пауэр поинт", "павер поинт", "power point", "microsoft powerpoint", "презентации", "пауэрпойнт"], start: "powerpnt.exe", process: Some("POWERPNT.EXE"), paths: &["%ProgramFiles%\\Microsoft Office\\root\\Office16\\POWERPNT.EXE", "%ProgramFiles(x86)%\\Microsoft Office\\root\\Office16\\POWERPNT.EXE"], office: None },
    AppDef { id: "outlook", name: "Outlook", aliases: &["outlook", "аутлук", "аутлок", "outlook почта", "майкрософт аутлук"], start: "outlook.exe", process: Some("OUTLOOK.EXE"), paths: &["%ProgramFiles%\\Microsoft Office\\root\\Office16\\OUTLOOK.EXE", "%ProgramFiles(x86)%\\Microsoft Office\\root\\Office16\\OUTLOOK.EXE"], office: None },
    AppDef { id: "onenote", name: "OneNote", aliases: &["onenote", "ванноут", "ван нот", "one note", "уаннот"], start: "onenote.exe", process: Some("ONENOTE.EXE"), paths: &["%ProgramFiles%\\Microsoft Office\\root\\Office16\\ONENOTE.EXE"], office: None },
    AppDef { id: "chrome", name: "Chrome", aliases: &["хром", "chrome", "гугл хром", "google chrome", "хроме", "хрома"], start: "chrome.exe", process: Some("chrome.exe"), paths: &["%ProgramFiles%\\Google\\Chrome\\Application\\chrome.exe", "%ProgramFiles(x86)%\\Google\\Chrome\\Application\\chrome.exe", "%LOCALAPPDATA%\\Google\\Chrome\\Application\\chrome.exe"], office: None },
    AppDef { id: "edge", name: "Edge", aliases: &["эдж", "edge", "microsoft edge", "майкрософт эдж", "эйдж", "едж"], start: "msedge.exe", process: Some("msedge.exe"), paths: &["%ProgramFiles(x86)%\\Microsoft\\Edge\\Application\\msedge.exe", "%ProgramFiles%\\Microsoft\\Edge\\Application\\msedge.exe"], office: None },
    AppDef { id: "firefox", name: "Firefox", aliases: &["фаерфокс", "файрфокс", "firefox", "мозилла", "mozilla", "фаер фокс", "файер фокс", "мозила фаерфокс"], start: "firefox.exe", process: Some("firefox.exe"), paths: &["%ProgramFiles%\\Mozilla Firefox\\firefox.exe", "%ProgramFiles(x86)%\\Mozilla Firefox\\firefox.exe"], office: None },
    AppDef { id: "yandex", name: "Яндекс Браузер", aliases: &["яндекс браузер", "яндекс браузера", "yandex browser", "яндекс бразуер", "яндексбраузер"], start: "browser.exe", process: Some("browser.exe"), paths: &["%LOCALAPPDATA%\\Yandex\\YandexBrowser\\Application\\browser.exe", "%ProgramFiles%\\Yandex\\YandexBrowser\\Application\\browser.exe"], office: None },
    AppDef { id: "opera", name: "Opera", aliases: &["опера", "opera", "опера браузер", "оперу"], start: "opera.exe", process: Some("opera.exe"), paths: &["%LOCALAPPDATA%\\Programs\\Opera\\opera.exe", "%ProgramFiles%\\Opera\\opera.exe", "%LOCALAPPDATA%\\Programs\\Opera GX\\opera.exe"], office: None },
    AppDef { id: "totalcmd", name: "Total Commander", aliases: &["тотал командер", "total commander", "тотал коммандер", "тотал командир", "тоталь командер", "тотал комендер", "тотал командор", "тотал", "тотал кмдр", "тоталкоммандер", "тотал команда", "total", "totalcmd", "тоталкмд", "тотал смд"], start: "totalcmd64.exe", process: Some("TOTALCMD64.EXE"), paths: &["%ProgramFiles%\\totalcmd\\TOTALCMD64.EXE", "%ProgramFiles%\\Total Commander\\TOTALCMD64.EXE", "%ProgramFiles(x86)%\\totalcmd\\TOTALCMD.EXE", "%ProgramFiles(x86)%\\Total Commander\\TOTALCMD.EXE", "C:\\totalcmd\\TOTALCMD64.EXE", "C:\\TOTALCMD\\TOTALCMD64.EXE", "C:\\totalcmd\\TOTALCMD.EXE", "%LOCALAPPDATA%\\Programs\\totalcmd\\TOTALCMD64.EXE", "D:\\totalcmd\\TOTALCMD64.EXE"], office: None },
    AppDef { id: "discord", name: "Discord", aliases: &["дискорд", "discord", "дискорт"], start: "discord://", process: Some("Discord.exe"), paths: &[], office: None },
    AppDef { id: "telegram", name: "Telegram", aliases: &["телеграм", "telegram", "телеграмм", "телега"], start: "tg://", process: Some("Telegram.exe"), paths: &[], office: None },
    AppDef { id: "spotify", name: "Spotify", aliases: &["спотифай", "spotify", "спотифи", "спотик"], start: "spotify:", process: Some("Spotify.exe"), paths: &[], office: None },
    AppDef { id: "steam", name: "Steam", aliases: &["стим", "steam", "стимм"], start: "steam://open/main", process: Some("steam.exe"), paths: &[], office: None },
    AppDef { id: "obs", name: "OBS", aliases: &["обс", "obs", "обэес", "о би эс"], start: "obs64.exe", process: Some("obs64.exe"), paths: &[], office: None },
    AppDef { id: "vlc", name: "VLC", aliases: &["влс", "vlc", "вэлэсэ", "вл си", "ви эл си"], start: "vlc.exe", process: Some("vlc.exe"), paths: &["%ProgramFiles%\\VideoLAN\\VLC\\vlc.exe", "%ProgramFiles(x86)%\\VideoLAN\\VLC\\vlc.exe"], office: None },
];

// ---------------------------------------------------------------------------------------------
// phonetic key
// ---------------------------------------------------------------------------------------------
fn latin_to_ru(w: &str) -> String {
    // longer groups first
    const GROUPS: &[(&str, &str)] = &[
        ("sch", "ш"), ("sh", "ш"), ("ch", "ч"), ("th", "т"), ("ph", "ф"), ("ck", "к"), ("ee", "и"), ("oo", "у"), ("qu", "кв"), ("exc", "экс"), ("ex", "экс"), ("ow", "ау"), ("ou", "ау"),
        ("ie", "и"), ("ea", "и"), ("ai", "эй"), ("ay", "эй"),
    ];
    let chars: Vec<char> = w.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    'outer: while i < chars.len() {
        let rest: String = chars[i..].iter().take(3).collect();
        for (g, r) in GROUPS {
            if rest.starts_with(g) {
                out.push_str(r);
                i += g.chars().count();
                continue 'outer;
            }
        }
        let c = chars[i];
        let r = match c {
            'a' => "а", 'b' => "б", 'c' => "к", 'd' => "д", 'e' => "е", 'f' => "ф", 'g' => "г", 'h' => "х", 'i' => "и", 'j' => "дж",
            'k' => "к", 'l' => "л", 'm' => "м", 'n' => "н", 'o' => "о", 'p' => "п", 'q' => "к", 'r' => "р", 's' => "с", 't' => "т",
            'u' => "у", 'v' => "в", 'w' => "в", 'x' => "кс", 'y' => "и", 'z' => "з",
            _ => "",
        };
        if r.is_empty() {
            out.push(c);
        } else {
            out.push_str(r);
        }
        i += 1;
    }
    out
}

fn devoice(c: char) -> char {
    match c {
        'д' => 'т', 'б' => 'п', 'г' => 'к', 'з' => 'с', 'в' => 'ф', 'ж' => 'ш',
        other => other,
    }
}

// one word -> phonetic key
fn word_key(w: &str) -> String {
    let w = w.to_lowercase().replace('ё', "е");
    let w: String = if w.chars().any(|c| c.is_ascii_alphabetic()) { latin_to_ru(&w) } else { w };
    let mut chars: Vec<char> = Vec::new();
    for c in w.chars() {
        let c = match c {
            'э' => 'е', 'ы' => 'и', 'й' => 'и', 'ё' => 'е',
            'ь' | 'ъ' => continue,
            c if c.is_alphanumeric() => c,
            _ => continue,
        };
        if chars.last() != Some(&c) {
            chars.push(c); // double letters merge
        }
    }
    if let Some(last) = chars.last_mut() {
        *last = devoice(*last); // ворд / ворт, словарь / словарт
    }
    chars.into_iter().collect()
}

pub fn tokens(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('ё', "е")
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_string())
        .collect()
}

fn phrase_key(words: &[String]) -> String {
    words.iter().map(|w| word_key(w)).collect::<Vec<_>>().join("")
}

fn edit_distance(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev[j - 1].saturating_sub(cost) + 1); // swapped neighbours
            }
            cur.push(v);
        }
        prev = cur;
    }
    prev[b.len()]
}

// how many wrong letters are allowed for a name of this key length (short names must match exactly)
fn allowed_edits(len: usize) -> usize {
    match len {
        0..=4 => 0,
        5..=7 => 1,
        _ => 2,
    }
}

// ---------------------------------------------------------------------------------------------
// resolving a name
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Match {
    pub app: &'static AppDef,
    pub distance: usize, // 0 = exact (after normalization)
}

impl std::fmt::Debug for AppDef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppDef({})", self.id)
    }
}
impl PartialEq for AppDef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for AppDef {}

// the program a whole phrase like "тотал командер" names, or None. Exact keys win; a fuzzy match must be unique:
// when two different programs are about equally close, nothing is returned.
pub fn resolve_name(name_words: &[String]) -> Option<Match> {
    if name_words.is_empty() || name_words.len() > 4 {
        return None;
    }
    let key: Vec<char> = phrase_key(name_words).chars().collect();
    if key.is_empty() {
        return None;
    }
    let mut best: Vec<(usize, &'static AppDef)> = Vec::new(); // (distance, app) best per app
    for app in APPS {
        let mut app_best: Option<usize> = None;
        for alias in app.aliases {
            let akey: Vec<char> = phrase_key(&tokens(alias)).chars().collect();
            let d = edit_distance(&key, &akey);
            let limit = allowed_edits(akey.len().min(key.len()));
            if d <= limit {
                app_best = Some(app_best.map_or(d, |b| b.min(d)));
            }
        }
        if let Some(d) = app_best {
            best.push((d, app));
        }
    }
    best.sort_by_key(|(d, a)| (*d, a.id));
    match best.as_slice() {
        [] => None,
        [(d, a)] => Some(Match { app: a, distance: *d }),
        [(d1, a1), (d2, _), ..] => {
            if *d1 == 0 && *d2 > 0 {
                Some(Match { app: a1, distance: 0 })
            } else if *d1 < *d2 {
                Some(Match { app: a1, distance: *d1 })
            } else {
                None // ambiguous: better to ask again than to start the wrong program
            }
        }
    }
}

// a program mentioned anywhere in a phrase (windows of 1..3 words, the longest first); used by the Lua script
pub fn find_in_phrase(text: &str) -> Option<Match> {
    let words = tokens(text);
    let mut found: Option<(usize, usize, Match)> = None; // (window size, -distance order, match)
    for size in (1..=3).rev() {
        if words.len() < size {
            continue;
        }
        for start in 0..=(words.len() - size) {
            if let Some(m) = resolve_name(&words[start..start + size]) {
                let better = match &found {
                    None => true,
                    Some((fs, _, fm)) => m.distance < fm.distance || (m.distance == fm.distance && size > *fs),
                };
                if better {
                    found = Some((size, 0, m));
                }
            }
        }
    }
    found.map(|(_, _, m)| m)
}

// ---------------------------------------------------------------------------------------------
// "открой ..." / "закрой ..." with a program name
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Open,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request {
    pub action: Action,
    pub app: &'static AppDef,
    pub exact: bool,
}

const OPEN_STEMS: &[&str] = &["откр", "запуст", "запуск", "включ", "стартуй", "старт", "вызов", "покаж"];
const CLOSE_STEMS: &[&str] = &["закр", "зактр"];
const FILLERS: &[&str] = &["пожалуйста", "мне", "программу", "программа", "приложение", "приложения", "ка", "быстро", "джарвис", "плиз", "мою", "нам", "пожалуста", "давай", "окно", "в"];

// "открой тотал командер" -> Open Total Commander. Only a short phrase that is nothing but a verb and a program
// name counts: "открой сайт ворд", "открой файл word.docx" or a whole sentence are left to the normal commands.
pub fn parse_request(text: &str) -> Option<Request> {
    let words = tokens(text);
    let first = words.first()?;
    let action = if OPEN_STEMS.iter().any(|s| first.starts_with(s)) {
        Action::Open
    } else if CLOSE_STEMS.iter().any(|s| first.starts_with(s)) {
        Action::Close
    } else {
        return None;
    };
    let rest: Vec<String> = words[1..].iter().filter(|w| !FILLERS.contains(&w.as_str())).cloned().collect();
    let m = resolve_name(&rest)?;
    Some(Request { action, app: m.app, exact: m.distance == 0 })
}

// ---------------------------------------------------------------------------------------------
// where a program is installed
// ---------------------------------------------------------------------------------------------
pub fn expand_env(p: &str) -> String {
    let mut out = String::new();
    let mut rest = p;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) => {
                let var = &after[..j];
                out.push_str(&std::env::var(var).unwrap_or_else(|_| format!("%{}%", var)));
                rest = &after[j + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(target_os = "windows")]
fn registry_install_dir(app: &AppDef) -> Option<String> {
    use std::os::windows::process::CommandExt;
    if app.id != "totalcmd" {
        return None;
    }
    for key in ["HKCU\\Software\\Ghisler\\Total Commander", "HKLM\\Software\\Ghisler\\Total Commander", "HKLM\\Software\\WOW6432Node\\Ghisler\\Total Commander"] {
        let out = std::process::Command::new("reg").args(["query", key, "/v", "InstallDir"]).creation_flags(0x0800_0000).output().ok()?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        if let Some(line) = text.lines().find(|l| l.contains("InstallDir")) {
            if let Some(idx) = line.find("REG_SZ") {
                return Some(line[idx + 6..].trim().to_string());
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn registry_install_dir(_app: &AppDef) -> Option<String> {
    None
}

// the full path of the installed program, if it is found in a usual place (or the registry)
pub fn locate(app: &AppDef) -> Option<String> {
    for p in app.paths {
        let path = expand_env(p);
        if !path.contains('%') && Path::new(&path).exists() {
            return Some(path);
        }
    }
    if let Some(dir) = registry_install_dir(app) {
        for exe in ["TOTALCMD64.EXE", "TOTALCMD.EXE"] {
            let p = Path::new(&dir).join(exe);
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests;
