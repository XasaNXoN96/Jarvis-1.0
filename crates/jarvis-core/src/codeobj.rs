// VS Code + Python as an object of the work (Object Interaction Engine, stage 3).
//
//   «Открой VS Code» → «Создай Python-файл» → «Назови calculator.py» → «Напиши калькулятор на Python»
//   → «Добавь обработку деления на ноль» → «Стоп» → «Продолжим с объектом»
//
// How it works (and what it does NOT do):
//   * the file is made and changed by Jarvis itself through `safefs` (the same judged, handle-based file operations as
//     every command): no thousands of characters typed into a window, VS Code only shows the file on disk;
//   * the code comes from the connected AI (`ai::complete`, nothing is stored in the dialogs); the generation can be
//     stopped, and an answer that arrives after the stop is dropped (objects.rs decides);
//   * a file that exists is never replaced silently: a new file is made with `create_new`; a change of an existing
//     file is checked against the file as it was read (a file changed meanwhile is not written), a big change waits
//     for «да», the previous version is kept in memory for «верни как было»;
//   * what was written is read back and compared before Jarvis says it is saved;
//   * the code is checked for syntax with `ast.parse` (it parses, it never runs anything); generated code is NEVER run;
//   * a file with something that looks like a key or a password is not sent to the AI.
//
// The state of the work (OBJECT_ACTIVE, AI_TASK, AWAITING_CONFIRMATION, PAUSED, ERROR_RECOVERY) lives in objects.rs.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::objects::{self, Control, Detect, Kind, Probe, State};
use crate::origin::Source;

const MAX_FILE: usize = 256 * 1024; // a file that is read for the AI or written
const MAX_INSTRUCTION: usize = 1000;
const AWAIT_SECS: u64 = 180;
const LARGE_REMOVED: usize = 30; // lines removed: from here a change asks first (when it is also half of the file)

// ---------------------------------------------------------------------------------------------
// replies
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub text: String,   // what the window shows
    pub speech: String, // what the voice says (short, no paths)
}

fn reply(text: impl Into<String>) -> Option<Reply> {
    let text = text.into();
    Some(Reply { speech: text.clone(), text })
}

fn reply2(text: impl Into<String>, speech: impl Into<String>) -> Option<Reply> {
    Some(Reply { text: text.into(), speech: speech.into() })
}

pub type Notifier = Box<dyn Fn(&str, &str) + Send + Sync>;
static NOTIFY: Lazy<Mutex<Option<Notifier>>> = Lazy::new(|| Mutex::new(None));

// the application tells how an answer that comes later (the end of a generation) reaches the window and the voice
pub fn set_notifier(f: Notifier) {
    *NOTIFY.lock() = Some(f);
}

fn notify(r: &Reply) {
    match NOTIFY.lock().as_ref() {
        Some(f) => f(&r.text, &r.speech),
        None => info!("[code] {}", r.text),
    }
}

// ---------------------------------------------------------------------------------------------
// the state of the work with code
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
enum Awaiting {
    Name { special_ext: Option<&'static str> },
    Folder { name: String },
    ProjectName,
    ProjectFolder { name: String },
}

struct Pending {
    path: PathBuf,
    base: Vec<u8>,       // the file as it was read for the AI
    content: String,     // what would be written
    summary: String,
}

struct Code {
    awaiting: Option<(Awaiting, Instant)>,
    project: Option<PathBuf>,
    file: Option<PathBuf>,
    known: Option<[u8; 32]>, // the hash of the file as Jarvis last wrote / read it
    prev: Option<(PathBuf, Vec<u8>)>, // the version before the last change («верни как было»)
    pending: Option<Pending>,
    retry: Option<String>,
}

static CODE: Lazy<Mutex<Code>> = Lazy::new(|| Mutex::new(Code { awaiting: None, project: None, file: None, known: None, prev: None, pending: None, retry: None }));

pub fn forget() {
    let mut c = CODE.lock();
    *c = Code { awaiting: None, project: None, file: None, known: None, prev: None, pending: None, retry: None };
}

fn hash(data: &[u8]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(data);
    h.finalize().into()
}

// ---------------------------------------------------------------------------------------------
// the outside world (replaceable in tests; the real thing everywhere else)
// ---------------------------------------------------------------------------------------------
static PROBE: Lazy<Mutex<Box<dyn Probe + Send>>> = Lazy::new(|| Mutex::new(Box::new(objects::SysProbe)));

type Launcher = Box<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;
static LAUNCH: Lazy<Mutex<Option<Launcher>>> = Lazy::new(|| Mutex::new(None));

#[cfg(test)]
pub(crate) type Generator = Box<dyn Fn(&str, &str, &AtomicBool) -> Result<String, String> + Send + Sync>;
#[cfg(test)]
pub(crate) static GENERATOR: Lazy<Mutex<Option<Generator>>> = Lazy::new(|| Mutex::new(None));

pub(crate) fn generate(system: &str, user: &str, cancel: &AtomicBool) -> Result<String, String> {
    #[cfg(test)]
    if let Some(g) = GENERATOR.lock().as_ref() {
        return g(system, user, cancel);
    }
    crate::ai::complete(system, user, cancel).map_err(|e| match e {
        crate::ai::AiError::Cancelled => "cancelled".to_string(),
        e => e.user_text(),
    })
}

pub(crate) fn ai_ready() -> Result<(), String> {
    #[cfg(test)]
    if GENERATOR.lock().is_some() {
        return Ok(());
    }
    crate::ai::conf_from_settings().map(|_| ()).map_err(|e| e.user_text())
}

// the real VS Code of this computer, found the way every program is (apps.rs); never a path from speech
fn open_in_vscode(path: &Path) -> Result<(), String> {
    if let Some(l) = LAUNCH.lock().as_ref() {
        return l(path);
    }
    let app = crate::apps::APPS.iter().find(|a| a.id == "vscode").ok_or("VS Code нет в списке программ")?;
    let exe = crate::apps::locate(app).ok_or("Не нашёл установленный VS Code")?;
    if !Path::new(&exe).file_name().map_or(false, |n| n.to_string_lossy().eq_ignore_ascii_case("code.exe")) {
        return Err("найденная программа — не VS Code".into());
    }
    // the file is only shown: reading it must be allowed by the rules of the files
    crate::policy::judge(&crate::policy::Op::FsRead(path.to_path_buf()))?;
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg(path);
    crate::null_stdio(&mut cmd);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().map_err(|e| format!("не удалось запустить VS Code: {}", e))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if PROBE.lock().running("code.exe") {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    Err("Windows приняла запуск, но VS Code так и не появился".into())
}

// ---------------------------------------------------------------------------------------------
// speech → names
// ---------------------------------------------------------------------------------------------
fn translit(word: &str) -> String {
    let mut out = String::new();
    for c in word.chars() {
        let s = match c {
            'а' => "a", 'б' => "b", 'в' => "v", 'г' => "g", 'д' => "d", 'е' | 'ё' => "e", 'ж' => "zh", 'з' => "z", 'и' => "i", 'й' => "y", 'к' => "k", 'л' => "l",
            'м' => "m", 'н' => "n", 'о' => "o", 'п' => "p", 'р' => "r", 'с' => "s", 'т' => "t", 'у' => "u", 'ф' => "f", 'х' => "h", 'ц' => "ts", 'ч' => "ch",
            'ш' => "sh", 'щ' => "sch", 'ъ' | 'ь' => "", 'ы' => "y", 'э' => "e", 'ю' => "yu", 'я' => "ya",
            c if c.is_ascii_alphanumeric() => {
                out.push(c);
                continue;
            }
            _ => "",
        };
        out.push_str(s);
    }
    out
}

const NAME_NOISE: &[&str] = &[
    "назови", "назвать", "назовем", "назовите", "называется", "название", "имя", "пусть", "будет", "файл", "файла", "файлу", "скрипт", "как", "пожалуйста", "python", "питон", "пайтон",
    "точка", "с", "именем", "под", "давай", "его", "ее", "это", "то", "в", "на",
];

// "calculator.py", "калькулятор точка пи вай", "назови файл мой скрипт" → "calculator.py", "kalkulyator.py", "moy_skript.py".
// `ext` is used when the speech does not say it.
pub fn file_name_from_speech(text: &str, ext: &str) -> Option<String> {
    let words = crate::apps::tokens(text);
    let mut parts: Vec<String> = Vec::new();
    let mut i = 0;
    let mut said_ext: Option<&str> = None;
    while i < words.len() {
        let w = words[i].as_str();
        let next = words.get(i + 1).map(|s| s.as_str());
        if w == "py" || w == "пай" || w == "пай" {
            said_ext = Some("py");
            i += 1;
            continue;
        }
        if w == "пи" && matches!(next, Some("вай") | Some("уай") | Some("вы")) {
            said_ext = Some("py");
            i += 2;
            continue;
        }
        if w == "txt" || w == "тхт" {
            said_ext = Some("txt");
            i += 1;
            continue;
        }
        if w == "md" || w == "мд" {
            said_ext = Some("md");
            i += 1;
            continue;
        }
        if NAME_NOISE.contains(&w) {
            i += 1;
            continue;
        }
        let t = translit(w);
        if !t.is_empty() {
            parts.push(t);
        }
        i += 1;
    }
    if parts.is_empty() {
        return None;
    }
    let mut stem = parts.join("_");
    stem.truncate(40);
    let stem = stem.trim_matches('_').to_string();
    if stem.is_empty() {
        return None;
    }
    Some(format!("{}.{}", stem, said_ext.unwrap_or(ext)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub folder: &'static str, // the known folder: documents | desktop | downloads
    pub sub: Option<String>,  // a folder inside it ("калькулятор")
}

// "в документах", "на рабочем столе папка калькулятор", "загрузки"
pub fn place_from_speech(text: &str) -> Option<Place> {
    let words = crate::apps::tokens(text);
    let folder = if words.iter().any(|w| w.starts_with("документ") || w == "умолчанию") {
        "documents"
    } else if words.iter().any(|w| w.starts_with("рабоч") || w.starts_with("стол")) {
        "desktop"
    } else if words.iter().any(|w| w.starts_with("загрузк")) {
        "downloads"
    } else {
        return None;
    };
    let mut sub: Vec<String> = Vec::new();
    if let Some(p) = words.iter().position(|w| w.starts_with("папк")) {
        let skip = ["документы", "документах", "рабочем", "рабочий", "столе", "стол", "загрузки", "загрузках", "в", "на", "под", "названием", "именем", "называется", "новую", "новая", "по", "умолчанию", "создай"];
        for w in &words[p + 1..] {
            if !skip.contains(&w.as_str()) {
                sub.push(w.clone());
            }
        }
    }
    let sub = if sub.is_empty() { None } else { Some(clean_dir_name(&sub.join(" "))) }.filter(|s| !s.is_empty());
    Some(Place { folder, sub })
}

fn clean_dir_name(s: &str) -> String {
    s.chars().filter(|c| !matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()).collect::<String>().trim().trim_matches('.').to_string()
}

#[cfg(test)]
static PLACES_ROOT: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

fn place_dir(p: &Place) -> Option<PathBuf> {
    #[cfg(test)]
    let base = match PLACES_ROOT.lock().as_ref() {
        Some(root) => root.join(p.folder),
        None => crate::filesearch::known_folder(p.folder)?,
    };
    #[cfg(not(test))]
    let base = crate::filesearch::known_folder(p.folder)?;
    Some(match &p.sub {
        Some(s) => base.join(s),
        None => base,
    })
}

// ---------------------------------------------------------------------------------------------
// text helpers: code from an answer, a diff, secrets
// ---------------------------------------------------------------------------------------------
// the content of the (first) code block of an answer; a plain answer without blocks is the code itself
pub fn extract_code(answer: &str) -> Option<String> {
    let a = answer.replace("\r\n", "\n");
    if let Some(start) = a.find("```") {
        let rest = &a[start + 3..];
        let body_start = rest.find('\n').map(|n| n + 1)?; // the language tag line
        let body = &rest[body_start..];
        let end = body.find("\n```").unwrap_or_else(|| body.find("```").unwrap_or(body.len()));
        let code = body[..end].trim_end().to_string();
        return if code.trim().is_empty() { None } else { Some(code + "\n") };
    }
    let t = a.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string() + "\n")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Diff {
    pub added: usize,
    pub removed: usize,
}

// lines added / removed (longest common subsequence of lines; a very long file counts as fully changed)
pub fn diff_lines(old: &str, new: &str) -> Diff {
    let a: Vec<&str> = old.lines().collect();
    let b: Vec<&str> = new.lines().collect();
    if a.len() > 3000 || b.len() > 3000 {
        return Diff { added: b.len(), removed: a.len() };
    }
    let mut prev = vec![0usize; b.len() + 1];
    for x in &a {
        let mut cur = vec![0usize; b.len() + 1];
        for (j, y) in b.iter().enumerate() {
            cur[j + 1] = if x == y { prev[j] + 1 } else { cur[j].max(prev[j + 1]) };
        }
        prev = cur;
    }
    let common = prev[b.len()];
    Diff { added: b.len() - common, removed: a.len() - common }
}

// a change that asks before it is written: many lines removed (and more than half of the file), or everything of a file
// that is not tiny
pub fn is_large_change(old: &str, d: Diff) -> bool {
    let lines = old.lines().count();
    (d.removed >= LARGE_REMOVED && d.removed * 2 > lines) || (lines >= 10 && d.removed == lines)
}

// does the text hold something that looks like a key, a token or a password (it is not sent to the AI then)
pub fn contains_secret(text: &str) -> bool {
    for line in text.lines() {
        let low = line.to_lowercase();
        for key in ["password", "passwd", "secret", "api_key", "apikey", "token", "пароль", "private_key"] {
            if let Some(pos) = low.find(key) {
                // the same position in the original text (lower-casing may change the length of a letter)
                let rest: String = line.chars().skip(low[..pos].chars().count() + key.chars().count()).collect();
                // «password = "abc"», «token: hunter22»: an assignment of a literal that looks like a value
                let rest = rest.trim_start_matches(|c: char| c.is_alphanumeric() || c == '_').trim_start();
                if let Some(v) = rest.strip_prefix('=').or_else(|| rest.strip_prefix(':')) {
                    let v = v.trim();
                    let quoted = v.starts_with('"') || v.starts_with('\'');
                    let inner = v.trim_matches(|c| c == '"' || c == '\'');
                    if (quoted && inner.chars().count() >= 4) || (!quoted && inner.chars().count() >= 6 && inner.chars().any(|c| c.is_ascii_digit()) && !inner.contains('(')) {
                        return true;
                    }
                }
            }
        }
    }
    text.split_whitespace().any(looks_secret_word)
}

fn looks_secret_word(w: &str) -> bool {
    let w = w.trim_matches(|c: char| !c.is_alphanumeric() && c != '-' && c != '_' && c != ':');
    let n = w.chars().count();
    (n >= 20 && w.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ':') && w.chars().any(|c| c.is_ascii_digit()))
        || ["sk-", "AIza", "ghp_", "gho_", "xoxb-", "hf_"].iter().any(|p| w.starts_with(p) && n >= 12)
}

// ---------------------------------------------------------------------------------------------
// the syntax check: ast.parse (parses, never runs)
// ---------------------------------------------------------------------------------------------
const SYNTAX_SCRIPT: &str = "import ast, sys\ntry:\n    ast.parse(open(sys.argv[1], encoding='utf-8-sig').read(), sys.argv[1])\nexcept SyntaxError as e:\n    print('%s|%s' % (e.lineno, e.msg))\n    sys.exit(3)\n";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Syntax {
    Ok,
    Error { line: u32, message: String },
    NoPython,
    Failed(String),
}

// python (or the py launcher) on the PATH. The stub of the Microsoft Store in WindowsApps is not Python.
pub fn find_python() -> Option<(PathBuf, Vec<String>)> {
    let path = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(windows) { &["python.exe", "py.exe"] } else { &["python3", "python"] };
    for dir in std::env::split_paths(&path) {
        if dir.to_string_lossy().to_lowercase().contains("windowsapps") {
            continue;
        }
        for n in names {
            let p = dir.join(n);
            if p.is_file() {
                let pre = if *n == "py.exe" { vec!["-3".to_string()] } else { Vec::new() };
                return Some((p, pre));
            }
        }
    }
    None
}

pub fn parse_syntax_output(out: &str) -> Option<(u32, String)> {
    let line = out.lines().next()?;
    let (n, msg) = line.split_once('|')?;
    Some((n.trim().parse().ok()?, msg.trim().chars().take(120).collect()))
}

pub fn syntax_check(path: &Path) -> Syntax {
    let Some((python, pre)) = find_python() else { return Syntax::NoPython };
    // the file is only read
    if let Err(e) = crate::policy::judge(&crate::policy::Op::FsRead(path.to_path_buf())) {
        return Syntax::Failed(e);
    }
    let mut cmd = std::process::Command::new(python);
    cmd.args(&pre).args(["-I", "-B", "-X", "utf8", "-c", SYNTAX_SCRIPT]).arg(path);
    cmd.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Syntax::Failed(e.to_string()),
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Syntax::Failed("проверка не уложилась в 10 секунд".into());
            }
            Err(e) => return Syntax::Failed(e.to_string()),
        }
    }
    let out = child.wait_with_output().ok();
    let (code, text) = match out {
        Some(o) => (o.status.code(), String::from_utf8_lossy(&o.stdout).to_string()),
        None => (None, String::new()),
    };
    match code {
        Some(0) => Syntax::Ok,
        Some(3) => match parse_syntax_output(&text) {
            Some((line, message)) => Syntax::Error { line, message },
            None => Syntax::Failed("не удалось разобрать ответ проверки".into()),
        },
        c => Syntax::Failed(format!("Python вернул код {:?}", c)),
    }
}

fn syntax_line(path: &Path) -> String {
    if path.extension().map_or(true, |e| !e.eq_ignore_ascii_case("py")) {
        return String::new();
    }
    match syntax_check(path) {
        Syntax::Ok => " Синтаксис проверен: ошибок нет.".to_string(),
        Syntax::Error { line, message } => format!(" Ошибка синтаксиса в строке {}: {}.", line, message),
        Syntax::NoPython => " Python не найден на компьютере, синтаксис не проверен.".to_string(),
        Syntax::Failed(why) => format!(" Проверить синтаксис не удалось: {}.", why),
    }
}

// ---------------------------------------------------------------------------------------------
// what the user says
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    CreateFile { name: Option<String>, ext: &'static str, special: Option<&'static str> },
    CreateProject { name: Option<String> },
    Edit(String),    // «напиши …», «добавь …», «исправь …»
    Explain(String), // «объясни ошибку»: an answer to read, nothing is written
    Check,
    Undo,
    Run,
    Retry,
    Save,
}

fn starts(words: &[String], stems: &[&str]) -> bool {
    words.iter().any(|w| stems.iter().any(|s| w.starts_with(s)))
}

fn first_is(words: &[String], stems: &[&str]) -> bool {
    words.first().map_or(false, |w| stems.iter().any(|s| w.starts_with(s)))
}

// `code_ctx`: the work with a code file is open (VS Code is the object); `file`: a file is chosen
pub fn parse(text: &str, code_ctx: bool, file: bool) -> Option<Intent> {
    let words = crate::apps::tokens(text);
    if words.is_empty() {
        return None;
    }
    let py = starts(&words, &["python", "питон", "пайтон"]);
    let create = first_is(&words, &["созда", "сдела", "нов"]) || words.iter().take(2).any(|w| w.starts_with("созда") || w.starts_with("сдела"));
    let has_file = starts(&words, &["файл", "скрипт"]);
    if create && code_ctx {
        if starts(&words, &["requirements", "реквайрментс", "зависимост"]) {
            return Some(Intent::CreateFile { name: Some("requirements.txt".into()), ext: "txt", special: Some("requirements") });
        }
        if starts(&words, &["readme", "ридми", "описани"]) {
            return Some(Intent::CreateFile { name: Some("README.md".into()), ext: "md", special: Some("readme") });
        }
        if starts(&words, &["тест"]) {
            return Some(Intent::CreateFile { name: None, ext: "py", special: Some("tests") });
        }
    }
    if create && has_file && (py || code_ctx) {
        let name = if starts(&words, &["назов", "называ"]) { name_after_marker(text) } else { None };
        return Some(Intent::CreateFile { name, ext: "py", special: None });
    }
    if create && starts(&words, &["проект"]) && (py || code_ctx) {
        return Some(Intent::CreateProject { name: name_after_marker(text).map(|n| n.trim_end_matches(".py").to_string()).filter(|n| !n.is_empty()) });
    }
    if !code_ctx {
        return None;
    }
    if first_is(&words, &["запусти", "выполни", "запустить", "исполни"]) && starts(&words, &["код", "файл", "скрипт", "программ", "питон", "python"]) {
        return Some(Intent::Run);
    }
    if starts(&words, &["проверь", "проверит"]) && starts(&words, &["синтакс", "код", "файл", "ошибк"]) || starts(&words, &["есть"]) && starts(&words, &["ошибк"]) {
        return Some(Intent::Check);
    }
    let joined = words.join(" ");
    if joined.starts_with("верни как было") || joined.starts_with("откати") || joined.starts_with("отмени последнее изменение") || joined.starts_with("верни назад") || joined.starts_with("верни прежн") {
        return Some(Intent::Undo);
    }
    if joined.starts_with("повтори генерацию") || joined.starts_with("повтори запрос") || joined.starts_with("попробуй еще раз") || joined == "повтори" {
        return Some(Intent::Retry);
    }
    if first_is(&words, &["сохрани"]) && file {
        return Some(Intent::Save);
    }
    if first_is(&words, &["объясни", "расскажи"]) && starts(&words, &["ошибк", "код", "файл", "функци"]) {
        return Some(Intent::Explain(text.trim().to_string()));
    }
    if file && first_is(&words, &["напиши", "сгенерируй", "реализуй", "добавь", "исправь", "поправь", "измени", "убери", "замени", "переименуй", "оптимизируй", "перепиши", "обработай", "дополни", "допиши", "удали", "создай", "сделай"]) {
        return Some(Intent::Edit(text.trim().to_string()));
    }
    None
}

// «назови calculator.py», «назови файл мой скрипт»: the part after the marker word
fn name_after_marker(text: &str) -> Option<String> {
    let words = crate::apps::tokens(text);
    let pos = words.iter().position(|w| w.starts_with("назов") || w.starts_with("называ"))?;
    // the original spelling keeps the dots («calculator.py»): the words are cut from the text itself
    let low = text.to_lowercase();
    let marker = low.find(&words[pos]).map(|i| i + words[pos].len())?;
    file_name_from_speech(&text[marker..], "py").or_else(|| file_name_from_speech(&words[pos + 1..].join(" "), "py"))
}

// ---------------------------------------------------------------------------------------------
// the entry point of the application
// ---------------------------------------------------------------------------------------------
fn code_object() -> bool {
    objects::with(|e| e.object().map_or(false, |o| o.app == "vscode"))
}

fn current_file() -> Option<PathBuf> {
    CODE.lock().file.clone()
}

// A phrase from the voice / the window / the console. None = not mine: the application goes on with it.
// `text` is the lower-case phrase the commands see, `original` the words as they were said or typed.
pub fn handle(text: &str, original: &str, source: Source) -> Option<Reply> {
    // the work with files through the AI is driven from this computer; a phrase of a phone / chat is not
    if source.is_remote() {
        return None;
    }
    let state = objects::state();
    let mine = code_object();

    // 1. the control words of a work that is mine
    if mine {
        if let Some(ctl) = objects::classify(text, state) {
            if let Some(r) = control(ctl, state) {
                return Some(r);
            }
        }
    }

    // 2. an answer to a question of the dialog (name, folder)
    let awaiting = {
        let mut c = CODE.lock();
        match &c.awaiting {
            Some((_, t)) if t.elapsed() > Duration::from_secs(AWAIT_SECS) => {
                c.awaiting = None;
                None
            }
            Some((a, _)) => Some(a.clone()),
            None => None,
        }
    };
    if let Some(a) = awaiting {
        let words = crate::apps::tokens(text);
        if matches!(objects::classify(text, State::ObjectActive), Some(Control::Cancel)) || words.first().map_or(false, |w| w.starts_with("отмен")) && words.len() <= 3 {
            CODE.lock().awaiting = None;
            return reply("Хорошо, не создаю.");
        }
        return answer_dialog(a, text, original);
    }

    // 3. an intent
    let file = current_file().is_some();
    let intent = parse(text, mine, file)?;
    run_intent(intent, original)
}

fn control(ctl: Control, state: State) -> Option<Reply> {
    match (state, ctl) {
        (State::AiTask, Control::Stop) | (State::AiTask, Control::Cancel) => {
            let rep = objects::with(|e| e.stop());
            CODE.lock().pending = None;
            reply(objects::stop_reply(&rep, true))
        }
        (State::AiTask, Control::Pause) => {
            let rep = objects::with(|e| e.pause());
            reply(objects::stop_reply(&rep, false))
        }
        (State::AwaitingConfirmation, Control::Yes) => apply_pending(),
        (State::AwaitingConfirmation, Control::No) | (State::AwaitingConfirmation, Control::Cancel) => {
            let _ = objects::with(|e| e.answer(false));
            CODE.lock().pending = None;
            reply("Хорошо, ничего не записал.")
        }
        (State::AwaitingConfirmation, Control::Stop) | (State::AwaitingConfirmation, Control::Pause) => {
            let rep = objects::with(|e| if ctl == Control::Pause { e.pause() } else { e.stop() });
            CODE.lock().pending = None;
            reply(objects::stop_reply(&rep, ctl != Control::Pause))
        }
        (State::Paused, Control::Resume) | (State::Paused, Control::Yes) | (State::ErrorRecovery, Control::Resume) => resume(),
        (State::Paused, Control::No) => {
            objects::with(|e| e.decline_resume());
            reply2("Хорошо, файл остался открытым. Скажите «продолжим с объектом», когда понадобится.", "Хорошо, ничего не закрываю")
        }
        _ => None,
    }
}

// «продолжим с объектом»: the file and VS Code are checked again, then the work goes on
fn resume() -> Option<Reply> {
    let (name, path_s) = objects::with(|e| e.object().map(|o| (o.name.clone(), o.path.clone().unwrap_or_default()))).unwrap_or_default();
    let path = PathBuf::from(&path_s);
    let running = PROBE.lock().running("code.exe");
    let problem = if !running {
        Some("VS Code закрыт".to_string())
    } else if path_s.is_empty() || !path.is_file() {
        Some(format!("файл «{}» не найден: удалён или перемещён", name))
    } else {
        None
    };
    if let Some(why) = problem {
        let st = objects::state();
        if st == State::Paused {
            let _ = objects::with(|e| e.lost(&why));
        } else if st == State::ErrorRecovery {
            objects::with(|e| e.note("jarvis", &why));
        }
        return reply2(format!("{}. Продолжить нельзя. Откройте VS Code и файл, скажите «продолжим с объектом»: я проверю заново.", why), format!("{}. Продолжить нельзя", why));
    }
    let st = objects::state();
    let ok = if st == State::ErrorRecovery { objects::with(|e| e.recovered()) } else { objects::with(|e| e.resume()) };
    if let Err(e) = ok {
        return reply(e.user_text());
    }
    // the file may have been changed by someone else meanwhile: it is read again, nothing is assumed
    let mut note = String::new();
    if let Ok(data) = crate::safefs::read(&path) {
        let h = hash(&data);
        let mut c = CODE.lock();
        if c.known.map_or(false, |k| k != h) {
            note = " Файл изменился с прошлого раза, я прочитал его заново.".to_string();
        }
        c.known = Some(h);
    }
    reply2(format!("Продолжаем работу с «{}».{}", name, note), format!("Продолжаем работу с {}", name))
}

fn answer_dialog(a: Awaiting, text: &str, original: &str) -> Option<Reply> {
    match a {
        Awaiting::Name { special_ext } => {
            let ext = special_ext.unwrap_or("py");
            match file_name_from_speech(original, ext).or_else(|| file_name_from_speech(text, ext)) {
                Some(name) => start_create(name, ext),
                None => reply("Не расслышал имя. Назовите файл, например: calculator.py."),
            }
        }
        Awaiting::Folder { name } => match place_from_speech(text) {
            Some(place) => create_in_place(name, place),
            None => reply("В какой папке создать? Скажите: «документы», «рабочий стол» или «загрузки». Можно добавить: «папка калькулятор»."),
        },
        Awaiting::ProjectName => {
            let n = clean_dir_name(&crate::apps::tokens(text).into_iter().filter(|w| !NAME_NOISE.contains(&w.as_str()) && w != "проект").collect::<Vec<_>>().join(" "));
            if n.is_empty() {
                return reply("Не расслышал название проекта. Назовите его, например: калькулятор.");
            }
            CODE.lock().awaiting = Some((Awaiting::ProjectFolder { name: n }, Instant::now()));
            reply("В какой папке создать проект? Скажите: «документы», «рабочий стол» или «загрузки».")
        }
        Awaiting::ProjectFolder { name } => match place_from_speech(text) {
            Some(mut place) => {
                place.sub = Some(name);
                create_project(place)
            }
            None => reply("В какой папке создать проект? Скажите: «документы», «рабочий стол» или «загрузки»."),
        },
    }
}

fn run_intent(intent: Intent, original: &str) -> Option<Reply> {
    match intent {
        Intent::CreateFile { name, ext, special } => {
            let name = match (name, special) {
                (Some(n), _) => Some(n),
                (None, Some("tests")) => current_file().and_then(|f| f.file_stem().map(|s| format!("test_{}.py", s.to_string_lossy()))),
                _ => None,
            };
            match name {
                Some(n) => start_create(n, ext),
                None => {
                    CODE.lock().awaiting = Some((Awaiting::Name { special_ext: if ext == "py" { None } else { Some(ext) } }, Instant::now()));
                    reply("Как назвать файл? Например: calculator.py.")
                }
            }
        }
        Intent::CreateProject { name } => match name {
            Some(n) => {
                CODE.lock().awaiting = Some((Awaiting::ProjectFolder { name: clean_dir_name(&n) }, Instant::now()));
                reply("В какой папке создать проект? Скажите: «документы», «рабочий стол» или «загрузки».")
            }
            None => {
                CODE.lock().awaiting = Some((Awaiting::ProjectName, Instant::now()));
                reply("Как назвать проект?")
            }
        },
        Intent::Edit(instruction) => start_ai(instruction_of(original, &instruction), true),
        Intent::Explain(q) => start_ai(instruction_of(original, &q), false),
        Intent::Check => check_now(),
        Intent::Undo => undo(),
        Intent::Save => save_now(),
        Intent::Retry => {
            let retry = CODE.lock().retry.clone();
            match retry {
                Some(i) => {
                    if objects::state() == State::ErrorRecovery {
                        let _ = objects::with(|e| e.recovered());
                    }
                    start_ai(i, true)
                }
                None => reply("Нечего повторять."),
            }
        }
        Intent::Run => reply2(
            "Запуск кода я не делаю: сгенерированный код не запускается автоматически. Проверьте его и запустите сами в VS Code.",
            "Код автоматически не запускаю. Проверьте его и запустите сами",
        ),
    }
}

// the words of the user as typed (capitals, punctuation) when the original has them; else the matched phrase
fn instruction_of(original: &str, matched: &str) -> String {
    let o = original.trim();
    let t = if o.is_empty() { matched.trim() } else { o };
    t.chars().take(MAX_INSTRUCTION).collect()
}

// ---------------------------------------------------------------------------------------------
// creating a file
// ---------------------------------------------------------------------------------------------
fn start_create(name: String, ext: &'static str) -> Option<Reply> {
    let project = CODE.lock().project.clone();
    match project {
        Some(dir) => create_file(&dir, &name, ext),
        None => {
            CODE.lock().awaiting = Some((Awaiting::Folder { name }, Instant::now()));
            reply("В какой папке создать файл? Скажите: «документы», «рабочий стол» или «загрузки». Можно добавить: «папка калькулятор».")
        }
    }
}

fn create_in_place(name: String, place: Place) -> Option<Reply> {
    let Some(dir) = place_dir(&place) else { return reply("Не нашёл эту папку на компьютере.") };
    let ext = if name.ends_with(".py") { "py" } else if name.ends_with(".md") { "md" } else { "txt" };
    CODE.lock().awaiting = None;
    let r = create_file(&dir, &name, ext);
    if place.sub.is_some() {
        // a folder made for the file is the project from now on: the next files go there without a question
        if current_file().is_some() {
            CODE.lock().project = Some(dir);
        }
    }
    r
}

fn create_project(place: Place) -> Option<Reply> {
    let Some(dir) = place_dir(&place) else { return reply("Не нашёл эту папку на компьютере.") };
    CODE.lock().awaiting = None;
    if let Err(e) = crate::safefs::mkdir_all(&dir) {
        return reply(format!("Не удалось создать папку проекта: {}", e.to_string()));
    }
    CODE.lock().project = Some(dir.clone());
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    reply2(format!("Проект «{}» готов: {}. Теперь скажите, какой файл создать.", name, dir.display()), format!("Проект {} готов. Какой файл создать?", name))
}

fn start_state(path: &Path, project: &Path, name: &str) -> Result<(), String> {
    objects::with(|e| {
        match e.state() {
            State::AiTask | State::AwaitingConfirmation => return Err(objects::Error::Busy("текущую задачу: скажите «стоп»").user_text()),
            State::Paused | State::ObjectActive | State::Dictation | State::ErrorRecovery => {
                // the previous object (it stays as it is in its program) gives way to the new one
                let _ = e.finish();
            }
            _ => {}
        }
        let _ = e.open_app("vscode");
        e.bind("vscode", Kind::CodeFile, name, Some(&path.to_string_lossy()), Some(&project.to_string_lossy()), Detect::ProcessOnly).map(|_| ()).map_err(|e| e.user_text())
    })
}

fn create_file(dir: &Path, name: &str, _ext: &str) -> Option<Reply> {
    let path = dir.join(name);
    if let Err(why) = start_state_check() {
        return reply(why);
    }
    if let Err(e) = crate::safefs::mkdir_all(dir) {
        return reply(format!("Не удалось подготовить папку: {}", e.to_string()));
    }
    let existed = match crate::safefs::create_new(&path, b"") {
        Ok(()) => false,
        Err(crate::safefs::SafeError::Io(e)) if e.kind() == std::io::ErrorKind::AlreadyExists => true,
        Err(e) => return reply(format!("Не удалось создать файл: {}", e.to_string())),
    };
    if let Err(why) = start_state(&path, dir, name) {
        return reply(why);
    }
    // what is on the disk now, as Jarvis knows it
    let data = crate::safefs::read(&path).unwrap_or_default();
    {
        let mut c = CODE.lock();
        c.file = Some(path.clone());
        c.project = Some(dir.to_path_buf());
        c.known = Some(hash(&data));
        c.prev = None;
        c.pending = None;
        c.retry = None;
        c.awaiting = None;
    }
    objects::with(|e| e.note("jarvis", &format!("файл {}", name)));
    let shown = match open_in_vscode(&path) {
        Ok(()) => " Открыл его в VS Code.".to_string(),
        Err(why) => format!(" В VS Code открыть не удалось: {}.", why),
    };
    if existed {
        reply2(
            format!("Файл «{}» уже есть: {}. Я ничего в нём не менял, работаю с ним.{}", name, path.display(), shown),
            format!("Файл {} уже есть. Я его не менял, работаю с ним", name),
        )
    } else {
        reply2(format!("Файл «{}» создан: {}.{}", name, path.display(), shown), format!("Файл {} создан", name))
    }
}

fn start_state_check() -> Result<(), String> {
    match objects::state() {
        State::AiTask | State::AwaitingConfirmation => Err(objects::Error::Busy("текущую задачу: скажите «стоп»").user_text()),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------------------------
// the AI task
// ---------------------------------------------------------------------------------------------
fn system_prompt(name: &str, write: bool) -> String {
    if write {
        format!(
            "Ты помощник программиста. Пользователь работает с файлом «{}». Ответь ТОЛЬКО полным новым содержимым этого файла в одном блоке кода, без пояснений до и после. \
             Сохраняй существующий код, если его не просят менять. Не добавляй код, который запускает команды системы, удаляет файлы или обращается к сети, если об этом не просили.",
            name
        )
    } else {
        "Ты помощник программиста. Объясни кратко и по-русски, простыми словами, максимум пять предложений. Не пиши длинный код.".to_string()
    }
}

fn start_ai(instruction: String, write: bool) -> Option<Reply> {
    let Some(path) = current_file() else { return reply("Сначала создайте файл или скажите, с каким файлом работать.") };
    if let Err(why) = ai_ready() {
        // nothing was started: the request stays for «повтори»
        CODE.lock().retry = Some(instruction);
        return reply2(format!("{} Когда ИИ заработает, скажите «повтори».", why), "ИИ сейчас недоступен. Скажите повтори, когда он заработает");
    }
    if contains_secret(&instruction) {
        return reply("В запросе похоже есть ключ или пароль. Я не отправляю такое в ИИ.");
    }
    let base = match crate::safefs::read(&path) {
        Ok(b) if b.len() <= MAX_FILE => b,
        Ok(_) => return reply("Файл слишком большой для ИИ (больше 256 КБ)."),
        Err(e) => {
            let _ = objects::with(|e| e.lost("файл недоступен"));
            return reply(format!("Не могу прочитать файл: {}. Он удалён или перемещён?", e.to_string()));
        }
    };
    let current = match String::from_utf8(base.clone()) {
        Ok(s) => s,
        Err(_) => return reply("Файл не в кодировке UTF-8: не трогаю его."),
    };
    if contains_secret(&current) {
        return reply("В файле похоже есть ключ, токен или пароль. Я не отправляю такой файл в ИИ.");
    }
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let (task, cancel) = match objects::with(|e| e.start_ai(&instruction)) {
        Ok(t) => t,
        Err(e) => return reply(e.user_text()),
    };
    objects::with(|e| e.note("user", &instruction));
    {
        let mut c = CODE.lock();
        c.retry = Some(instruction.clone());
        c.pending = None;
    }
    let user = if current.trim().is_empty() {
        format!("Задача: {}", instruction)
    } else {
        format!("Задача: {}\n\nТекущее содержимое файла «{}»:\n```\n{}\n```", instruction, name, current)
    };
    let system = system_prompt(&name, write);
    #[cfg(test)]
    let places = crate::policy::test_places();
    std::thread::spawn(move || {
        #[cfg(test)]
        crate::policy::apply_test_places(places);
        run_task(task, cancel, path, base, current, system, user, write)
    });
    reply2("Пишу код. Это займёт несколько секунд. Скажите «стоп», чтобы отменить.", "Пишу. Скажите стоп, чтобы отменить")
}

#[allow(clippy::too_many_arguments)]
fn run_task(task: u64, cancel: Arc<AtomicBool>, path: PathBuf, base: Vec<u8>, current: String, system: String, user: String, write: bool) {
    let result = generate(&system, &user, &cancel);
    let out = match result {
        Err(why) if why == "cancelled" => {
            // stopped: the engine already moved to PAUSED / OBJECT_ACTIVE; nothing is written, nothing is said twice
            objects::with(|e| e.ai_finished(task, Err("cancelled".into())));
            return;
        }
        Err(why) => {
            // a task that was stopped meanwhile says nothing; a failed one says why and keeps the request for «повтори»
            if objects::with(|e| e.is_current_task(task)) {
                objects::with(|e| e.ai_finished(task, Err(why.clone())));
                notify(&Reply { text: format!("{} Скажите «повтори», когда ИИ заработает.", why), speech: "ИИ не ответил. Скажите повтори, когда он заработает".into() });
            }
            return;
        }
        Ok(a) => a,
    };
    if !write {
        // an explanation: shown and said, never written
        let text: String = out.trim().chars().take(1500).collect();
        if objects::with(|e| e.ai_finished(task, Ok("объяснение".into()))) {
            notify(&Reply { speech: objects::sanitize(&text), text });
        }
        return;
    }
    let bad = |why: &str, text: &str, speech: &str| {
        if objects::with(|e| e.is_current_task(task)) {
            objects::with(|e| e.ai_finished(task, Err(why.to_string())));
            notify(&Reply { text: text.into(), speech: speech.into() });
        }
    };
    let Some(new_code) = extract_code(&out) else {
        return bad("ответ ИИ без кода", "ИИ не вернул код. Файл не тронут. Скажите «повтори».", "ИИ не вернул код. Файл не тронут");
    };
    if new_code.len() > MAX_FILE {
        return bad("слишком длинный ответ", "ИИ вернул слишком длинный файл. Ничего не записал.", "Ответ слишком длинный. Ничего не записал");
    }
    // line ends of the file are kept
    let new_text = if base.windows(2).any(|w| w == b"\r\n") { new_code.replace("\r\n", "\n").replace('\n', "\r\n") } else { new_code.replace("\r\n", "\n") };
    let d = diff_lines(&current, &new_text);
    let summary = format!("добавлено строк: {}, удалено: {}", d.added, d.removed);
    let large = is_large_change(&current, d);
    // the engine takes the result only while the task is the current one (not stopped, not replaced); the write happens
    // under the same lock, so no stop can come between the decision and the write
    let reply = objects::with(|e| {
        if !e.ai_finished(task, Ok(summary.clone())) {
            return None;
        }
        if large {
            let q = format!("Изменение затронет много строк ({}). Записать? Скажите да или нет.", summary);
            if e.ask("write_ai", &q).is_ok() {
                CODE.lock().pending = Some(Pending { path: path.clone(), base: base.clone(), content: new_text.clone(), summary: summary.clone() });
                return Some(Reply { text: q.clone(), speech: "Изменение большое. Записать? Скажите да или нет".into() });
            }
        }
        Some(write_checked(e, &path, &base, &new_text, &summary))
    });
    if let Some(r) = reply {
        notify(&r);
    }
}

// writes the file only when it is still what the AI was given, then reads it back; the engine learns the save only then
fn write_checked(e: &mut objects::Engine, path: &Path, base: &[u8], new_text: &str, summary: &str) -> Reply {
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let now = match crate::safefs::read(path) {
        Ok(n) => n,
        Err(_) => {
            let _ = e.lost("файл недоступен");
            return Reply { text: format!("Файл «{}» пропал или недоступен. Ничего не записал.", name), speech: "Файл пропал. Ничего не записал".into() };
        }
    };
    if hash(&now) != hash(base) {
        return Reply { text: format!("Файл «{}» изменился, пока ИИ писал. Ничего не записал, чтобы не затереть изменения. Повторите запрос.", name), speech: "Файл изменился, пока я писал. Ничего не записал".into() };
    }
    if let Err(err) = crate::safefs::write(path, new_text.as_bytes()) {
        return Reply { text: format!("Не удалось записать файл: {}", err.to_string()), speech: "Не удалось записать файл".into() };
    }
    // the fact of the save is the file read back
    let back = crate::safefs::read(path).unwrap_or_default();
    if back != new_text.as_bytes() {
        return Reply { text: "Файл записан, но проверка чтением не совпала с тем, что я записал. Проверьте файл.".into(), speech: "Запись не подтвердилась. Проверьте файл".into() };
    }
    {
        let mut c = CODE.lock();
        c.prev = Some((path.to_path_buf(), base.to_vec()));
        c.known = Some(hash(&back));
        c.pending = None;
    }
    e.saved(&path.to_string_lossy());
    let lines = new_text.lines().count();
    let syntax = syntax_line(path);
    Reply {
        text: format!("Готово: в «{}» {} строк ({}). Записано и проверено чтением.{} Код не запускался.", name, lines, summary, syntax),
        speech: format!("Готово. В файле {} строк.{}", lines, if syntax.contains("ошибок нет") { " Синтаксис в порядке." } else if syntax.contains("Ошибка синтаксиса") { " Но в синтаксисе есть ошибка." } else { "" }),
    }
}

// «да» to the question of a big change
fn apply_pending() -> Option<Reply> {
    let p = CODE.lock().pending.take();
    let Some(p) = p else {
        let _ = objects::with(|e| e.answer(false));
        return reply("Нечего записывать.");
    };
    objects::with(|e| match e.answer(true) {
        Ok(Some(id)) if id == "write_ai" => Some(write_checked(e, &p.path, &p.base, &p.content, &p.summary)),
        _ => None,
    })
}

// ---------------------------------------------------------------------------------------------
// other actions on the file
// ---------------------------------------------------------------------------------------------
fn check_now() -> Option<Reply> {
    let Some(path) = current_file() else { return reply("Нет файла, с которым я работаю.") };
    if !path.is_file() {
        return reply("Файл не найден: удалён или перемещён.");
    }
    let s = syntax_line(&path);
    if s.is_empty() {
        return reply("Синтаксис проверяю только у файлов Python (.py).");
    }
    let t = s.trim().to_string();
    reply(t)
}

fn undo() -> Option<Reply> {
    let (prev, known) = {
        let c = CODE.lock();
        (c.prev.clone(), c.known)
    };
    let Some((path, data)) = prev else { return reply("Нечего откатывать: прошлой версии у меня нет.") };
    // only when the file is still what Jarvis wrote last (nobody changed it since)
    let now = crate::safefs::read(&path).unwrap_or_default();
    if known.map_or(true, |k| k != hash(&now)) {
        return reply("Файл изменён после моей записи, откатывать не буду, чтобы не затереть чужие изменения.");
    }
    if let Err(e) = crate::safefs::write(&path, &data) {
        return reply(format!("Не удалось вернуть прошлую версию: {}", e.to_string()));
    }
    let back = crate::safefs::read(&path).unwrap_or_default();
    if back != data {
        return reply("Откат записан, но проверка чтением не совпала. Проверьте файл.");
    }
    {
        let mut c = CODE.lock();
        c.known = Some(hash(&back));
        c.prev = None;
    }
    objects::with(|e| e.saved(&path.to_string_lossy()));
    reply2("Вернул прошлую версию файла. Записано и проверено чтением.", "Вернул прошлую версию")
}

fn save_now() -> Option<Reply> {
    let Some(path) = current_file() else { return reply("Нет файла, с которым я работаю.") };
    let data = match crate::safefs::read(&path) {
        Ok(d) => d,
        Err(_) => return reply("Файл не найден: удалён или перемещён."),
    };
    let known = CODE.lock().known;
    if known.map_or(false, |k| k != hash(&data)) {
        CODE.lock().known = Some(hash(&data));
        return reply("Файл на диске изменён не мной (возможно, правками в VS Code). Я ничего не записывал; несохранённые правки в окне VS Code сохраняются в самом редакторе.");
    }
    objects::with(|e| e.saved(&path.to_string_lossy()));
    reply(format!("Файл на диске: {}, {} строк. Мои изменения уже записаны. Несохранённые правки в окне VS Code сохраняйте в самом редакторе.", path.display(), String::from_utf8_lossy(&data).lines().count()))
}

#[cfg(test)]
mod tests;
