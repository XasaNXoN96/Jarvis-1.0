// Access to the phone remote from ANY network, without a VPN and without opening a port on the router: a free
// "quick tunnel" of Cloudflare (cloudflared). The program on this PC makes an OUTGOING connection to Cloudflare, which
// gives a public https address (https://<words>.trycloudflare.com); what the phone sends there reaches the remote of
// Jarvis on 127.0.0.1. No account and no payment are needed.
//
// It is only switched on by the user (a switch with an explanation in the window), never at start, and it is off
// again when the remote or Jarvis ends:
// - the phone remote still needs its secret for everything but the empty page (remote.rs); the secret has 128 bits,
//   and wrong tries from one address are throttled (remote.rs, `throttle`);
// - the traffic goes through the servers of Cloudflare (it can see it, like any proxy): the window says so;
// - the address is new every time the tunnel is switched on: the old one stops working;
// - cloudflared is downloaded once (about 55 MB) from the official releases of Cloudflare on GitHub: a FIXED version
//   whose SHA-256 is written here and checked at every start (that exact file, signed by Cloudflare, was checked
//   when the version was chosen); a file with another hash is deleted and never run;
// - the child process lives in a Windows job object: it ends with Jarvis however Jarvis ends.

use std::io::{BufRead, BufReader, Read};
#[cfg(feature = "lua")]
use std::io::Write;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use log::{info, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

pub const VERSION: &str = "2026.10.0";
pub const SHA256: &str = "86aee4017b26625cee8484c113558f48effa4cd47f7aa05fcf425604e5d2b23c";
pub const EXE_SIZE: u64 = 55_365_048;

#[derive(Clone, Debug, PartialEq)]
pub enum State {
    Off,
    // percent of the download
    Downloading(u8),
    Starting,
    On(String),
    Error(String),
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::Off));
static CHILD: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));
// a new generation ends the thread of the old run (switched off, switched on again)
static GEN: AtomicU64 = AtomicU64::new(0);
// the program to run (a fake one in the tests)
static EXE_OVERRIDE: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

fn set(s: State) {
    *STATE.lock() = s;
}

pub fn state() -> State {
    STATE.lock().clone()
}

fn alive(gen: u64) -> bool {
    GEN.load(Ordering::SeqCst) == gen
}

fn exe_path() -> Option<PathBuf> {
    if let Some(p) = EXE_OVERRIDE.lock().clone() {
        return Some(p);
    }
    crate::APP_CONFIG_DIR.get().map(|d| d.join("cloudflared").join("cloudflared.exe"))
}

// "https://quiet-river-1234.trycloudflare.com" found in a line of the log of cloudflared (and nothing else: a
// look-alike address in some other line is not taken)
pub fn find_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == ':' || c == '/')).unwrap_or(rest.len());
    let url = rest[..end].trim_end_matches('/');
    let host = url.strip_prefix("https://")?;
    let label = host.strip_suffix(".trycloudflare.com")?;
    if label.is_empty() || label.contains('.') || label == "api" || label.starts_with('-') || label.ends_with('-') {
        return None;
    }
    Some(url.to_string())
}

pub fn sha256_hex(path: &std::path::Path) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}

fn download_url() -> String {
    format!("https://github.com/cloudflare/cloudflared/releases/download/{}/cloudflared-windows-amd64.exe", VERSION)
}

// downloads the pinned release into `part` (the percent goes to the state); the caller checks the hash
#[cfg(feature = "lua")]
fn download(part: &std::path::Path, gen: u64) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder().connect_timeout(Duration::from_secs(15)).timeout(Duration::from_secs(600)).build().map_err(|e| e.to_string())?;
    let mut resp = client.get(download_url()).send().map_err(|_| "нет связи с github.com (загрузка cloudflared)".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("github.com ответил ошибкой {} при загрузке cloudflared", resp.status().as_u16()));
    }
    let total = resp.content_length().unwrap_or(EXE_SIZE).max(1);
    let mut f = crate::create_fresh(part).map_err(|e| format!("не удалось записать файл: {}", e))?;
    let mut buf = vec![0u8; 1 << 16];
    let mut got = 0u64;
    loop {
        if !alive(gen) {
            drop(f);
            let _ = std::fs::remove_file(part);
            return Err("отменено".into());
        }
        let n = resp.read(&mut buf).map_err(|_| "загрузка прервалась".to_string())?;
        if n == 0 {
            return Ok(());
        }
        f.write_all(&buf[..n]).map_err(|e| format!("не удалось записать файл: {}", e))?;
        got += n as u64;
        set(State::Downloading(((got * 100) / total).min(100) as u8));
        if got > EXE_SIZE * 2 {
            drop(f);
            let _ = std::fs::remove_file(part);
            return Err("загруженный файл слишком большой — отменено".into());
        }
    }
}

#[cfg(not(feature = "lua"))]
fn download(_part: &std::path::Path, _gen: u64) -> Result<(), String> {
    Err("в этой сборке нет сетевого модуля".into())
}

// the program is on the disk and genuine, or it is downloaded now. `progress` gets the percent.
fn ensure_exe(gen: u64) -> Result<PathBuf, String> {
    let exe = exe_path().ok_or("нет папки настроек")?;
    if EXE_OVERRIDE.lock().is_some() {
        return Ok(exe);
    }
    if !cfg!(target_os = "windows") {
        return Err("туннель доступен только в Windows".into());
    }
    // already downloaded: the hash is checked every time (a file that was changed afterwards is not run)
    if exe.exists() {
        if sha256_hex(&exe).map(|h| h == SHA256).unwrap_or(false) {
            return Ok(exe);
        }
        let _ = std::fs::remove_file(&exe);
    }
    let dir = exe.parent().ok_or("нет папки")?.to_path_buf();
    std::fs::create_dir_all(&dir).map_err(|e| format!("не удалось создать папку: {}", e))?;
    let part = dir.join("cloudflared-download.exe");
    download(&part, gen)?;
    let hash = sha256_hex(&part).map_err(|e| e.to_string())?;
    if hash != SHA256 {
        let _ = std::fs::remove_file(&part);
        return Err("контрольная сумма cloudflared не совпала — файл удалён".into());
    }
    std::fs::rename(&part, &exe).map_err(|e| format!("не удалось сохранить cloudflared: {}", e))?;
    info!("Tunnel: cloudflared {} downloaded and verified", VERSION);
    Ok(exe)
}

// the child process ends with Jarvis, also when Jarvis is ended by force (TerminateProcess)
#[cfg(target_os = "windows")]
pub(crate) fn bind_to_job(child: &Child) {
    use std::os::windows::io::AsRawHandle;
    use winapi::um::jobapi2::{AssignProcessToJobObject, CreateJobObjectW, SetInformationJobObject};
    use winapi::um::winnt::{JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE};
    static JOB: Lazy<usize> = Lazy::new(|| unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if job.is_null() {
            return 0;
        }
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &mut info as *mut _ as *mut _, std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
        job as usize
    });
    let job = *JOB;
    if job != 0 {
        unsafe {
            AssignProcessToJobObject(job as *mut _, child.as_raw_handle() as *mut _);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn bind_to_job(_child: &Child) {}

fn kill_child() {
    if let Some(mut c) = CHILD.lock().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

// switched on by the user: the remote must be running (it is started when it is not)
pub fn enable() -> Result<(), String> {
    if matches!(state(), State::Downloading(_) | State::Starting | State::On(_)) {
        return Ok(());
    }
    crate::remote::start().map_err(|e| format!("пульт не запустился: {}", e))?;
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    set(State::Starting);
    std::thread::spawn(move || run(gen));
    Ok(())
}

pub fn disable() {
    GEN.fetch_add(1, Ordering::SeqCst);
    kill_child();
    set(State::Off);
}

fn run(gen: u64) {
    let exe = match ensure_exe(gen) {
        Ok(p) => p,
        Err(e) => {
            if alive(gen) {
                warn!("Tunnel: {}", e);
                set(State::Error(e));
            }
            return;
        }
    };
    if !alive(gen) {
        return;
    }
    set(State::Starting);
    let origin = format!("http://127.0.0.1:{}", crate::remote::PORT);
    let mut cmd = Command::new(&exe);
    cmd.args(["tunnel", "--no-autoupdate", "--url", &origin]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            set(State::Error(format!("cloudflared не запустился: {}", e)));
            return;
        }
    };
    bind_to_job(&child);
    let stderr = child.stderr.take();
    *CHILD.lock() = Some(child);
    if !alive(gen) {
        kill_child();
        return;
    }
    let Some(stderr) = stderr else {
        set(State::Error("нет вывода cloudflared".into()));
        kill_child();
        return;
    };
    let started = Instant::now();
    // the watcher of the start: no address in two minutes is an error
    std::thread::spawn(move || {
        while alive(gen) && started.elapsed() < Duration::from_secs(120) {
            if matches!(state(), State::On(_) | State::Error(_)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
        if alive(gen) && !matches!(state(), State::On(_)) {
            warn!("Tunnel: no address in 120 s");
            set(State::Error("Cloudflare не выдал адрес за 2 минуты (нет интернета или он закрыт)".into()));
            kill_child();
        }
    });
    let mut last_error = String::new();
    for line in BufReader::new(stderr).lines().map_while(Result::ok) {
        if !alive(gen) {
            break;
        }
        if let Some(url) = find_url(&line) {
            if !matches!(state(), State::On(_)) {
                info!("Tunnel: the public address is ready");
                set(State::On(url));
            }
        } else if line.contains("ERR") || line.contains("error") {
            // (the log of cloudflared holds addresses and error texts, never what the phone sends)
            last_error = line.chars().take(160).collect();
        }
    }
    // the program ended
    if alive(gen) {
        kill_child();
        let why = if last_error.is_empty() { "cloudflared остановился".to_string() } else { format!("cloudflared остановился: {}", last_error) };
        warn!("Tunnel: {}", why);
        set(State::Error(why));
    }
}

// the link for the phone: the public address with the secret of the remote
pub fn public_link() -> Option<String> {
    match state() {
        State::On(url) => Some(format!("{}/?t={}", url, crate::remote::link_token())),
        _ => None,
    }
}

pub fn json() -> Value {
    let (name, percent, url, error) = match state() {
        State::Off => ("off", 0, String::new(), String::new()),
        State::Downloading(p) => ("downloading", p as u32, String::new(), String::new()),
        State::Starting => ("starting", 0, String::new(), String::new()),
        State::On(u) => ("on", 100, u, String::new()),
        State::Error(e) => ("error", 0, String::new(), e),
    };
    json!({
        "state": name,
        "percent": percent,
        "url": if url.is_empty() { Value::Null } else { json!(public_link().unwrap_or_default()) },
        "error": error,
        "available": cfg!(target_os = "windows") || EXE_OVERRIDE.lock().is_some(),
        "version": VERSION,
    })
}

#[cfg(test)]
pub fn set_exe_for_tests(p: Option<PathBuf>) {
    *EXE_OVERRIDE.lock() = p;
}

#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_trycloudflare_address_is_taken_from_the_log() {
        let l = "2026-10-12T10:00:00Z INF |  https://quiet-river-1234.trycloudflare.com                                      |";
        assert_eq!(find_url(l).unwrap(), "https://quiet-river-1234.trycloudflare.com");
        assert_eq!(find_url("INF Requesting new quick Tunnel on trycloudflare.com..."), None);
        assert_eq!(find_url("https://api.trycloudflare.com/tunnel"), None, "the API address of the service is not the tunnel");
        assert_eq!(find_url("https://evil.com/?x=a.trycloudflare.com"), None);
        assert_eq!(find_url("https://a.b.trycloudflare.com"), None, "one label only");
        assert_eq!(find_url("https://trycloudflare.com"), None);
        assert_eq!(find_url("https://x.trycloudflare.com.evil.ru"), None, "a longer host is not the tunnel");
    }

    #[test]
    fn the_hash_of_a_file_is_computed_and_a_wrong_file_is_not_the_pinned_one() {
        let p = std::env::temp_dir().join(format!("jarvis_sha_{}.bin", std::process::id()));
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(sha256_hex(&p).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_ne!(sha256_hex(&p).unwrap(), SHA256);
        let _ = std::fs::remove_file(&p);
        assert_eq!(SHA256.len(), 64);
    }

    // a fake cloudflared (a script that prints the address like the real one and keeps running): the whole chain —
    // start, address, link with the secret, switching off ends the process
    #[cfg(unix)]
    #[test]
    fn the_tunnel_is_started_gives_the_link_and_is_ended_with_its_process() {
        use std::os::unix::fs::PermissionsExt;
        let _g = TEST_LOCK.lock();
        let dir = std::env::temp_dir().join(format!("jarvis_tunnel_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("fake-cloudflared.sh");
        let pidfile = dir.join("pid");
        std::fs::write(&exe, format!("#!/bin/sh\necho $$ > {}\necho 'INF Requesting new quick Tunnel' >&2\necho 'INF |  https://fake-words-42.trycloudflare.com  |' >&2\nsleep 300\n", pidfile.display())).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        set_exe_for_tests(Some(exe));
        disable();
        // (the remote is started by enable(); in the tests its port may be taken: the tunnel start does not depend on it)
        let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
        set(State::Starting);
        std::thread::spawn(move || run(gen));
        let t0 = Instant::now();
        while !matches!(state(), State::On(_)) && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(state(), State::On("https://fake-words-42.trycloudflare.com".into()));
        let link = public_link().unwrap();
        assert!(link.starts_with("https://fake-words-42.trycloudflare.com/?t=") && link.len() > 60, "{}", link);
        let j = json();
        assert_eq!(j["state"], "on");
        assert_eq!(j["url"], link);
        let pid: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        assert!(std::path::Path::new(&format!("/proc/{}", pid)).exists(), "the process runs");
        disable();
        assert_eq!(state(), State::Off);
        assert!(public_link().is_none() && json()["url"].is_null(), "no link after switching off");
        std::thread::sleep(Duration::from_millis(300));
        assert!(!std::path::Path::new(&format!("/proc/{}", pid)).exists(), "the process ended with the switch");
        // the program that ends by itself is an error, not a silent 'on'
        std::fs::write(dir.join("dies.sh"), "#!/bin/sh\necho 'ERR connection refused' >&2\nexit 1\n").unwrap();
        std::fs::set_permissions(dir.join("dies.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        set_exe_for_tests(Some(dir.join("dies.sh")));
        let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
        set(State::Starting);
        std::thread::spawn(move || run(gen));
        let t0 = Instant::now();
        while !matches!(state(), State::Error(_)) && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(matches!(state(), State::Error(e) if e.contains("connection refused")), "{:?}", state());
        disable();
        set_exe_for_tests(None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_program_that_is_not_there_is_an_honest_error() {
        let _g = TEST_LOCK.lock();
        set_exe_for_tests(Some(std::path::PathBuf::from("/definitely/not/here/cloudflared")));
        let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
        set(State::Starting);
        run(gen);
        assert!(matches!(state(), State::Error(e) if e.contains("не запустился")));
        disable();
        set_exe_for_tests(None);
    }
}
