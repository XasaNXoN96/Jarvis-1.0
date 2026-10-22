// Silero voices («Джарвис» / «Луна» ...) of another program that is installed on the same computer.
// Nothing of that program is copied or shipped: its own `tts\speak.py --serve`, its own Python and its own model are
// started from where the user installed it, and only the text and a speaker name are sent to it.
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::json;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub const PREFIX: &str = "silero:";
const TAG: &str = "@@jarvis ";

// (silero speaker, name shown in the settings)
pub const SPEAKERS: [(&str, &str); 3] = [
    ("kseniya", "Луна (женский, Silero)"),
    ("xenia", "Ксения (женский, Silero)"),
    ("baya", "Бая (женский, Silero)"),
];

#[derive(Clone, Debug, PartialEq)]
pub struct Install {
    pub python: PathBuf,
    pub script: PathBuf,
}

// "silero:aidar" -> "aidar" (only the known speakers)
pub fn speaker_of(choice: &str) -> Option<&'static str> {
    let name = choice.strip_prefix(PREFIX)?;
    SPEAKERS.iter().find(|(id, _)| *id == name).map(|(id, _)| *id)
}

fn has_model(tts_dir: &Path) -> bool {
    ["v5_5_ru.pt", "v5_ru.pt"].iter().any(|n| std::fs::metadata(tts_dir.join("models").join(n)).map(|m| m.len() > 1_000_000).unwrap_or(false))
}

// python.exe somewhere near the tts folder (the runtime is bundled with the program, its exact place is not assumed)
fn find_python(start: &Path, depth: u32) -> Option<PathBuf> {
    for name in ["python.exe", "python"] {
        let p = start.join(name);
        if p.is_file() {
            return Some(p);
        }
    }
    if depth == 0 {
        return None;
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(start).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    // a folder that says "python" first: it is the likely one
    dirs.sort_by_key(|p| !p.file_name().map(|n| n.to_string_lossy().to_lowercase().contains("python")).unwrap_or(false));
    dirs.iter().find_map(|d| {
        let n = d.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
        if n == "models" || n == "voices" || n == "__pycache__" || n == "site-packages" {
            return None;
        }
        find_python(d, depth - 1)
    })
}

// a tts folder (with speak.py and a model) -> how to start it
pub fn install_in(tts_dir: &Path) -> Option<Install> {
    let script = tts_dir.join("speak.py");
    if !script.is_file() || !has_model(tts_dir) {
        return None;
    }
    // the runtime may lie in the tts folder or beside it (resources\python ...)
    let python = find_python(tts_dir, 3).or_else(|| tts_dir.parent().and_then(|p| find_python(p, 2)))?;
    Some(Install { python, script })
}

fn candidate_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Ok(d) = std::env::var("JARVIS_SILERO_DIR") {
        if !d.trim().is_empty() {
            v.push(PathBuf::from(d.trim()));
        }
    }
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Ok(p) = std::env::var(var) {
            v.push(Path::new(&p).join("JARVIS").join("resources").join("tts"));
        }
    }
    if let Ok(p) = std::env::var("LOCALAPPDATA") {
        v.push(Path::new(&p).join("Programs").join("JARVIS").join("resources").join("tts"));
    }
    v
}

static FOUND: Lazy<Mutex<Option<(Instant, Option<Install>)>>> = Lazy::new(|| Mutex::new(None));

// cached for a minute: the settings page asks often, a folder search is not free
pub fn find_install() -> Option<Install> {
    let mut g = FOUND.lock();
    if let Some((t, r)) = g.as_ref() {
        if t.elapsed() < Duration::from_secs(60) {
            return r.clone();
        }
    }
    let r = candidate_dirs().iter().find_map(|d| install_in(d));
    *g = Some((Instant::now(), r.clone()));
    r
}

pub fn available() -> bool {
    find_install().is_some()
}

pub fn searched_places() -> String {
    candidate_dirs().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("; ")
}

// ---- the running worker ----

struct Server {
    key: String,
    child: Child,
    stdin: ChildStdin,
    rx: Receiver<String>,
}

static SERVER: Lazy<Mutex<Option<Server>>> = Lazy::new(|| Mutex::new(None));
static JOB: AtomicU64 = AtomicU64::new(1);

pub fn job_line(id: u64, text: &str, speaker: &str, speed: f32, out: &Path) -> String {
    json!({ "id": id.to_string(), "text": text, "voice": speaker, "speed": speed, "sample_rate": 48000, "out": out.to_string_lossy() }).to_string()
}

// "@@jarvis OK 12" -> Some(("OK", "12")); "@@jarvis ERR 12 ValueError: x" -> ("ERR", "12 ValueError: x")
pub fn parse_reply(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim_end().strip_prefix(TAG)?;
    Some(rest.split_once(' ').unwrap_or((rest, "")))
}

fn spawn(inst: &Install) -> Result<Server, String> {
    let mut cmd = Command::new(&inst.python);
    cmd.arg("-u").arg(&inst.script).arg("--serve").arg("--idle-exit").arg("900");
    if let Some(dir) = inst.script.parent() {
        cmd.current_dir(dir);
    }
    cmd.env("PYTHONIOENCODING", "utf-8").env("PYTHONUTF8", "1");
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("не удалось запустить {}: {}", inst.python.display(), e))?;
    let stdin = child.stdin.take().ok_or("нет входного потока")?;
    let stdout = child.stdout.take().ok_or("нет выходного потока")?;
    let (tx, rx) = channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    Ok(Server { key: format!("{}|{}", inst.python.display(), inst.script.display()), child, stdin, rx })
}

// the worker is started when it is not running (or is another install); the model loads in the background
fn ensure_running(guard: &mut Option<Server>, inst: &Install) -> Result<(), String> {
    let key = format!("{}|{}", inst.python.display(), inst.script.display());
    let alive = match guard.as_mut() {
        Some(s) if s.key == key => s.child.try_wait().map(|r| r.is_none()).unwrap_or(false),
        _ => false,
    };
    if !alive {
        if let Some(mut old) = guard.take() {
            let _ = old.child.kill();
        }
        *guard = Some(spawn(inst)?);
    }
    Ok(())
}

// loads the model before the first answer (a Silero voice is chosen): the first phrase does not wait for it
pub fn prewarm() {
    std::thread::spawn(|| {
        let Some(inst) = find_install() else { return };
        let mut guard = SERVER.lock();
        let _ = ensure_running(&mut guard, &inst);
    });
}

pub fn stop_server() {
    if let Some(mut s) = SERVER.lock().take() {
        let _ = s.child.kill();
        let _ = s.child.wait();
    }
}

pub enum Outcome {
    Done,
    Cancelled,
    Failed(String),
}

// one phrase -> one WAV file. `cancelled` is asked while waiting (a newer phrase / stop).
pub fn synth(text: &str, speaker: &str, speed: f32, out: &Path, cancelled: &dyn Fn() -> bool) -> Outcome {
    let Some(inst) = find_install() else {
        return Outcome::Failed(format!("озвучка Silero не найдена (искал: {})", searched_places()));
    };
    let mut guard = SERVER.lock();
    if let Err(e) = ensure_running(&mut guard, &inst) {
        return Outcome::Failed(e);
    }
    let Some(srv) = guard.as_mut() else { return Outcome::Failed("Silero не запущен".into()) };
    let id = JOB.fetch_add(1, Ordering::SeqCst);
    let line = job_line(id, text, speaker, speed, out);
    if writeln!(srv.stdin, "{}", line).and_then(|_| srv.stdin.flush()).is_err() {
        let _ = srv.child.kill();
        *guard = None;
        return Outcome::Failed("Silero закрылся, попробуйте ещё раз".into());
    }
    // the model loads once (tens of seconds on a slow computer); later phrases are quick
    let started = Instant::now();
    let limit = Duration::from_secs(180);
    let want = id.to_string();
    loop {
        if cancelled() {
            return Outcome::Cancelled;
        }
        if started.elapsed() > limit {
            let _ = srv.child.kill();
            *guard = None;
            return Outcome::Failed("Silero не ответил за 3 минуты".into());
        }
        match srv.rx.recv_timeout(Duration::from_millis(100)) {
            Ok(l) => match parse_reply(&l) {
                Some(("OK", rest)) if rest.trim() == want => return Outcome::Done,
                Some(("ERR", rest)) if rest.split_whitespace().next() == Some(want.as_str()) => {
                    let why = rest.trim_start_matches(want.as_str()).trim().to_string();
                    return Outcome::Failed(format!("Silero: {}", why));
                }
                _ => {} // READY and replies of earlier (cancelled) phrases
            },
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                *guard = None;
                return Outcome::Failed("Silero неожиданно закрылся".into());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_speakers_are_taken() {
        assert_eq!(speaker_of("silero:aidar"), None, "male voices are not offered");
        assert_eq!(speaker_of("silero:kseniya"), Some("kseniya"));
        assert_eq!(speaker_of("silero:../../x"), None);
        assert_eq!(speaker_of("piper"), None);
        assert_eq!(speaker_of(""), None);
    }

    #[test]
    fn replies_of_the_worker_are_read() {
        assert_eq!(parse_reply("@@jarvis OK 12\r\n"), Some(("OK", "12")));
        assert_eq!(parse_reply("@@jarvis ERR 3 ValueError: x"), Some(("ERR", "3 ValueError: x")));
        assert_eq!(parse_reply("@@jarvis READY 4.2"), Some(("READY", "4.2")));
        assert_eq!(parse_reply("warning: something"), None);
    }

    #[test]
    fn job_line_is_one_line_of_json_with_the_text_intact() {
        let l = job_line(7, "Привет\n\"мир\"", "xenia", 1.2, Path::new("C:\\Users\\Иван\\a.wav"));
        assert!(!l.contains('\n'));
        let v: serde_json::Value = serde_json::from_str(&l).unwrap();
        assert_eq!(v["id"], "7");
        assert_eq!(v["text"], "Привет\n\"мир\"");
        assert_eq!(v["voice"], "xenia");
        assert_eq!(v["out"], "C:\\Users\\Иван\\a.wav");
    }

    #[test]
    fn an_install_is_found_by_its_files_and_not_by_a_name() {
        let root = std::env::temp_dir().join(format!("jarvis_silero_test_{}", std::process::id()));
        let tts = root.join("tts");
        std::fs::create_dir_all(tts.join("models")).unwrap();
        assert!(install_in(&tts).is_none(), "an empty folder is not an install");
        std::fs::write(tts.join("speak.py"), "").unwrap();
        assert!(install_in(&tts).is_none(), "no model, no python");
        std::fs::write(tts.join("models").join("v5_5_ru.pt"), vec![0u8; 1_100_000]).unwrap();
        assert!(install_in(&tts).is_none(), "no python");
        std::fs::create_dir_all(root.join("python311")).unwrap();
        std::fs::write(root.join("python311").join("python.exe"), "").unwrap();
        let got = install_in(&tts).expect("python beside the tts folder is found");
        assert_eq!(got.python, root.join("python311").join("python.exe"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_real_layout_tts_runtime_python_is_found() {
        let root = std::env::temp_dir().join(format!("jarvis_silero_real_{}", std::process::id()));
        let tts = root.join("tts");
        std::fs::create_dir_all(tts.join("models")).unwrap();
        std::fs::create_dir_all(tts.join("voices")).unwrap();
        std::fs::create_dir_all(tts.join("runtime").join("python")).unwrap();
        std::fs::write(tts.join("speak.py"), "").unwrap();
        std::fs::write(tts.join("models").join("v5_5_ru.pt"), vec![0u8; 1_100_000]).unwrap();
        std::fs::write(tts.join("runtime").join("python").join("python.exe"), "").unwrap();
        let got = install_in(&tts).expect("the layout of the installed program");
        assert_eq!(got.python, tts.join("runtime").join("python").join("python.exe"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
