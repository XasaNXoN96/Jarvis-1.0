//! The model of the aim assist: where it comes from, how it is installed and checked (the page «Игры» → «Помощь прицела»).
//!
//! Two ways in, and both end in the same checks:
//!   * `start_install` — the official model (the manifest `OFFICIAL`: YOLOX-s of OpenCV Zoo, Apache-2.0, a file pinned by the commit of
//!     the repository). It is downloaded over HTTPS from the one address of the manifest into a temporary file, with a progress and a
//!     way to cancel, and is accepted only when its SIZE and SHA-256 are exactly those of the manifest.
//!   * `install_manual` — a file the user chose. A file is NEVER trusted because of its extension: it must load in ONNX Runtime, have
//!     an input of a picture and an output of a detector of people (aimdetect.rs), and give numbers (not NaN) on a trial run.
//! Then the file replaces the old model atomically (a rename in the same folder). The old model is moved aside to `player.onnx.bak-<time>`
//! and put back when anything after that fails; it is never deleted by a failed attempt. Before the swap nothing of the old model is touched.
//! The file that is run is never an executable: only the data of a network, read by ONNX Runtime.

use crate::aimassist::AimConfig;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct Manifest {
    pub id: &'static str,
    pub title: &'static str,
    pub url: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub license: &'static str,
    pub source_page: &'static str,
    pub note: &'static str,
}

// The official model. Where the numbers come from: the repository opencv/opencv_zoo at the commit below, the file
// models/object_detection_yolox/object_detection_yolox_2022nov.onnx is a Git LFS file, and its LFS pointer in that commit says
// `oid sha256:c5c2d13e…` and `size 35858002`. The address names the commit, so the file behind it cannot change; if it did, the hash would not match.
pub const OFFICIAL: Manifest = Manifest {
    id: "yolox-s-opencv-zoo-2022nov",
    title: "YOLOX-s (OpenCV Zoo)",
    url: "https://github.com/opencv/opencv_zoo/raw/47534e27c9851bb1128ccc0102f1145e27f23f98/models/object_detection_yolox/object_detection_yolox_2022nov.onnx",
    sha256: "c5c2d13e59ae883e6af3b45daea64af4833a4951c92d116ec270d9ddbe998063",
    size: 35_858_002,
    license: "Apache-2.0",
    source_page: "https://github.com/opencv/opencv_zoo/tree/47534e27c9851bb1128ccc0102f1145e27f23f98/models/object_detection_yolox",
    note: "Находит ЦЕЛОГО человека (класс person) среди 80 классов COCO; «голова» и «корпус» — точки внутри рамки человека, а не отдельные части тела.",
};

pub fn download_size_mb() -> u64 {
    OFFICIAL.size / 1_048_576 + 1
}

// the folder of the model; the tests use a folder of their own
pub fn dir() -> PathBuf {
    #[cfg(test)]
    {
        return std::env::temp_dir().join(format!("jarvis-aimmodel-{}", std::process::id()));
    }
    #[cfg(not(test))]
    {
        crate::APP_DIR.join("aim")
    }
}

pub fn active_file(dir: &Path) -> PathBuf {
    dir.join("player.onnx")
}

fn meta_file(dir: &Path) -> PathBuf {
    dir.join("model.json")
}

// ---------------------------------------------------------------------------------------------
// what a check of a model says (kept in model.json, shown on the page)
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub layout: String,
    pub input: (usize, usize),
    pub classes: usize,
    pub trial_ms: u64,
    pub device: String,
}

// the check of a file: it loads, fits and answers like a detector of people. Without ONNX Runtime in the build nothing can be checked.
pub type Probe<'a> = &'a dyn Fn(&Path) -> Result<Summary, String>;

pub fn real_probe(cfg: &AimConfig) -> impl Fn(&Path) -> Result<Summary, String> + '_ {
    move |path: &Path| {
        #[cfg(feature = "ort")]
        {
            let i = crate::aimdetect::ortrun::probe(path, cfg)?;
            Ok(Summary { layout: i.spec.layout.name().to_string(), input: (i.spec.input_w, i.spec.input_h), classes: i.spec.classes, trial_ms: i.trial_ms, device: if i.device_note.is_empty() { i.device.title().to_string() } else { format!("{} ({})", i.device.title(), i.device_note) } })
        }
        #[cfg(not(feature = "ort"))]
        {
            let _ = (path, cfg);
            Err("в этой сборке нет ONNX Runtime: модель проверить нельзя".to_string())
        }
    }
}

// ---------------------------------------------------------------------------------------------
// the source of the bytes: HTTPS in the program, a piece of memory in the tests
// ---------------------------------------------------------------------------------------------
pub trait Source {
    // the length told by the server (if it told) and the stream of the bytes
    fn open(&self) -> Result<(Option<u64>, Box<dyn Read>), String>;
}

#[cfg(feature = "lua")]
pub struct Https(pub &'static str);

#[cfg(feature = "lua")]
impl Source for Https {
    fn open(&self) -> Result<(Option<u64>, Box<dyn Read>), String> {
        if !self.0.starts_with("https://") {
            return Err("адрес модели не HTTPS: отказ".into());
        }
        // redirects (GitHub hands big files over to its storage) are followed only to https and only inside GitHub
        let policy = reqwest::redirect::Policy::custom(|a| {
            let ok = a.url().scheme() == "https" && a.url().host_str().map(|h| h == "github.com" || h.ends_with(".githubusercontent.com") || h == "githubusercontent.com").unwrap_or(false);
            if a.previous().len() > 5 {
                a.error("слишком много перенаправлений")
            } else if ok {
                a.follow()
            } else {
                a.error("перенаправление за пределы GitHub: отказ")
            }
        });
        let client = reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(20)).timeout(std::time::Duration::from_secs(60 * 30)).redirect(policy).build().map_err(|e| e.to_string())?;
        let resp = client.get(self.0).send().map_err(|e| if e.is_connect() || e.is_timeout() { "нет связи с сервером загрузки".to_string() } else { format!("загрузка не началась: {}", e) })?;
        if !resp.status().is_success() {
            return Err(format!("сервер ответил {}", resp.status()));
        }
        let len = resp.content_length();
        Ok((len, Box::new(resp)))
    }
}

// ---------------------------------------------------------------------------------------------
// the state shown on the page
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub enum Phase {
    Idle,
    Downloading { got: u64, total: u64 },
    Verifying,
    Checking,
    Installing,
}

struct State {
    phase: Phase,
    // the last failed installation (kept until the next attempt)
    install_error: String,
    // the last result of the check of the file that is installed: None = not checked in this run of the program
    check: Option<Result<Summary, String>>,
    // a short note about the last good installation
    last_ok: String,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State { phase: Phase::Idle, install_error: String::new(), check: None, last_ok: String::new() }));
static CANCEL: AtomicBool = AtomicBool::new(false);
static BUSY: AtomicBool = AtomicBool::new(false);
static CHECKING: AtomicBool = AtomicBool::new(false);

pub fn is_busy() -> bool {
    BUSY.load(Ordering::SeqCst)
}

fn set_phase(p: Phase) {
    STATE.lock().phase = p;
}

pub fn cancel() -> bool {
    if is_busy() {
        CANCEL.store(true, Ordering::SeqCst);
        true
    } else {
        false
    }
}

// ---------------------------------------------------------------------------------------------
// the installation
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub summary: Summary,
    pub sha256: String,
    pub size: u64,
    pub backup: Option<PathBuf>,
}

fn sha256_of(path: &Path) -> Result<String, String> {
    crate::integrity::sha256_file(path).map_err(|e| format!("файл не прочитался: {}", e))
}

// moves the checked file `incoming` in place of the active model. The old one is moved aside first and is put back if the rest fails.
fn swap_in(dir: &Path, incoming: &Path, meta: &Value) -> Result<Option<PathBuf>, String> {
    let target = active_file(dir);
    let mut backup = None;
    if target.exists() {
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let mut b = dir.join(format!("player.onnx.bak-{}", secs));
        let mut n = 1;
        while b.exists() {
            b = dir.join(format!("player.onnx.bak-{}-{}", secs, n));
            n += 1;
        }
        std::fs::rename(&target, &b).map_err(|e| format!("не удалось сохранить прежнюю модель: {}; новая не установлена", e))?;
        backup = Some(b);
    }
    let restore = |backup: &Option<PathBuf>| {
        if let Some(b) = backup {
            let _ = std::fs::rename(b, &target);
        }
    };
    if let Err(e) = std::fs::rename(incoming, &target) {
        restore(&backup);
        return Err(format!("не удалось поставить модель на место: {}; прежняя возвращена", e));
    }
    if let Err(e) = crate::write_atomic(&meta_file(dir), serde_json::to_string_pretty(meta).unwrap_or_default()) {
        // the model itself is in place; only its description is missing: the old pair is put back so that the folder is not half old, half new
        let _ = std::fs::remove_file(&target);
        restore(&backup);
        return Err(format!("не удалось записать описание модели: {}; прежняя возвращена", e));
    }
    Ok(backup)
}

fn meta_json(kind: &str, m: Option<&Manifest>, name: &str, sha: &str, size: u64, s: &Summary) -> Value {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    json!({ "kind": kind, "name": name, "source": m.map(|m| m.source_page).unwrap_or(""), "license": m.map(|m| m.license).unwrap_or("неизвестна (файл выбран пользователем)"),
        "manifest_id": m.map(|m| m.id).unwrap_or(""), "sha256": sha, "size": size, "installed_at": secs,
        "layout": s.layout, "input": format!("{}x{}", s.input.0, s.input.1), "classes": s.classes, "trial_ms": s.trial_ms, "device": s.device })
}

// The download. `progress` is called with (got, total). On any failure the temporary file is deleted and the old model stays as it was.
pub fn install_from(src: &dyn Source, m: &Manifest, dir: &Path, probe: Probe, cancel: &AtomicBool, progress: &dyn Fn(u64, u64)) -> Result<Installed, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("не удалось создать папку модели: {}", e))?;
    let part = dir.join("incoming.onnx");
    let _ = std::fs::remove_file(&part);
    let result = (|| -> Result<Installed, String> {
        let (len, mut stream) = src.open()?;
        if let Some(l) = len {
            if l != m.size {
                return Err(format!("сервер сообщил размер {} байт, а ожидается {}: файл не тот", l, m.size));
            }
        }
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&part).map_err(|e| format!("не удалось создать временный файл: {}", e))?;
            let (mut got, mut buf) = (0u64, vec![0u8; 64 * 1024]);
            loop {
                if cancel.load(Ordering::SeqCst) {
                    return Err("cancelled".into());
                }
                let n = stream.read(&mut buf).map_err(|_| "загрузка прервалась (обрыв связи)".to_string())?;
                if n == 0 {
                    break;
                }
                got += n as u64;
                if got > m.size {
                    return Err("файл больше ожидаемого: загрузка остановлена".into());
                }
                f.write_all(&buf[..n]).map_err(|e| format!("не удалось записать файл: {}", e))?;
                progress(got, m.size);
            }
            f.flush().ok();
            if got != m.size {
                return Err(format!("получено {} байт из {}: загрузка оборвалась", got, m.size));
            }
        }
        set_phase(Phase::Verifying);
        let sha = sha256_of(&part)?;
        if !sha.eq_ignore_ascii_case(m.sha256) {
            return Err("контрольная сумма SHA-256 не совпала с ожидаемой: файл подменён или повреждён, он удалён".into());
        }
        set_phase(Phase::Checking);
        let summary = probe(&part).map_err(|e| format!("модель не прошла проверку: {}", e))?;
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        set_phase(Phase::Installing);
        let meta = meta_json("official", Some(m), m.title, &sha, m.size, &summary);
        let backup = swap_in(dir, &part, &meta)?;
        Ok(Installed { summary, sha256: sha, size: m.size, backup })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

// A file of the user: copied to a temporary file of the model folder (so that the swap is a rename on one disk), checked, put in place.
pub fn install_manual(src: &Path, dir: &Path, probe: Probe) -> Result<Installed, String> {
    let name = src.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    if !name.to_lowercase().ends_with(".onnx") {
        return Err("Нужен файл модели с расширением .onnx.".into());
    }
    let meta = std::fs::symlink_metadata(src).map_err(|_| "Файл модели не найден.".to_string())?;
    if !meta.is_file() {
        return Err("Это не файл (или ссылка): выберите сам файл модели.".into());
    }
    if meta.len() < crate::aimdetect::MIN_MODEL_BYTES || meta.len() > 1_500_000_000 {
        return Err("Файл не похож на модель: он слишком маленький или слишком большой.".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("Не удалось создать папку модели: {}.", e))?;
    // the chosen file is the installed one already
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(src), std::fs::canonicalize(active_file(dir))) {
        if a == b {
            return Err("Эта модель уже установлена.".into());
        }
    }
    let part = dir.join("incoming.onnx");
    let _ = std::fs::remove_file(&part);
    let result = (|| -> Result<Installed, String> {
        std::fs::copy(src, &part).map_err(|e| format!("Не удалось скопировать модель: {}.", e))?;
        set_phase(Phase::Checking);
        let sha = sha256_of(&part)?;
        let summary = probe(&part).map_err(|e| format!("Модель не прошла проверку: {}", e))?;
        set_phase(Phase::Installing);
        let meta = meta_json("manual", None, &name, &sha, meta.len(), &summary);
        let backup = swap_in(dir, &part, &meta)?;
        Ok(Installed { summary, sha256: sha, size: meta["size"].as_u64().unwrap_or(0), backup })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result
}

fn finish(res: Result<Installed, String>) {
    let mut s = STATE.lock();
    s.phase = Phase::Idle;
    match res {
        Ok(i) => {
            s.install_error.clear();
            s.last_ok = format!("Модель установлена и проверена ({}, вход {}×{}, {} классов). Включите помощь прицела.", i.summary.layout, i.summary.input.0, i.summary.input.1, i.summary.classes);
            s.check = Some(Ok(i.summary));
        }
        Err(e) if e == "cancelled" => {
            s.install_error = "Установка отменена. Прежняя модель (если была) осталась как была.".into();
        }
        Err(e) => {
            s.install_error = format!("{}. Прежняя модель (если была) осталась как была.", e);
        }
    }
}

// the button «Установить модель автоматически»: runs in a thread of its own, the page watches `status_json`
pub fn start_install(cfg: &AimConfig) -> Result<(), String> {
    #[cfg(feature = "lua")]
    {
        if BUSY.swap(true, Ordering::SeqCst) {
            return Err("Установка уже идёт.".into());
        }
        CANCEL.store(false, Ordering::SeqCst);
        {
            let mut s = STATE.lock();
            s.install_error.clear();
            s.last_ok.clear();
            s.phase = Phase::Downloading { got: 0, total: OFFICIAL.size };
        }
        // a model cannot be replaced while it works
        crate::aimassist::suspend();
        let cfg = cfg.clone();
        std::thread::spawn(move || {
            let probe = real_probe(&cfg);
            let res = install_from(&Https(OFFICIAL.url), &OFFICIAL, &dir(), &probe, &CANCEL, &|got, total| set_phase(Phase::Downloading { got, total }));
            finish(res);
            BUSY.store(false, Ordering::SeqCst);
        });
        Ok(())
    }
    #[cfg(not(feature = "lua"))]
    {
        let _ = cfg;
        Err("В этой сборке нет сетевого модуля: выберите файл модели вручную.".into())
    }
}

// the button «Выбрать файл модели…» (the file is chosen in the window of the system by the caller); the check runs in a thread, too
pub fn start_manual(src: PathBuf, cfg: &AimConfig) -> Result<(), String> {
    if BUSY.swap(true, Ordering::SeqCst) {
        return Err("Установка уже идёт.".into());
    }
    {
        let mut s = STATE.lock();
        s.install_error.clear();
        s.last_ok.clear();
        s.phase = Phase::Checking;
    }
    crate::aimassist::suspend();
    let cfg = cfg.clone();
    std::thread::spawn(move || {
        let probe = real_probe(&cfg);
        finish(install_manual(&src, &dir(), &probe));
        BUSY.store(false, Ordering::SeqCst);
    });
    Ok(())
}

// «Проверить модель»: the installed file is checked again (nothing is changed on the disk). Returns at once; the result is on the page.
pub fn start_check(cfg: &AimConfig) -> Result<(), String> {
    if is_busy() {
        return Err("Идёт установка: подождите.".into());
    }
    if CHECKING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let cfg = cfg.clone();
    std::thread::spawn(move || {
        let path = crate::aimassist::model_path(&cfg);
        let res = if path.is_file() { real_probe(&cfg)(&path) } else { Err("файла модели нет".to_string()) };
        STATE.lock().check = Some(res);
        CHECKING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

// the first look at the page checks the installed file once in a run of the program (not on every refresh)
pub fn ensure_checked(cfg: &AimConfig) {
    let need = STATE.lock().check.is_none() && !is_busy() && crate::aimassist::model_path(cfg).is_file();
    if need {
        let _ = start_check(cfg);
    }
}

// ---------------------------------------------------------------------------------------------
// for the page and the diagnostics
// ---------------------------------------------------------------------------------------------
pub fn read_meta(dir: &Path) -> Option<Value> {
    std::fs::read_to_string(meta_file(dir)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(|v| v.is_object())
}

// the state of the card: not_installed / downloading / verifying / checking / installing / install_error / compat_error / unchecked / ready / on
pub fn card_state(cfg: &AimConfig) -> &'static str {
    let s = STATE.lock();
    match s.phase {
        Phase::Downloading { .. } => return "downloading",
        Phase::Verifying => return "verifying",
        Phase::Checking => return "checking",
        Phase::Installing => return "installing",
        Phase::Idle => {}
    }
    let installed = crate::aimassist::model_path(cfg).is_file();
    if !installed {
        return if s.install_error.is_empty() { "not_installed" } else { "install_error" };
    }
    if !s.install_error.is_empty() {
        return "install_error";
    }
    match &s.check {
        Some(Err(_)) => "compat_error",
        None => "unchecked",
        Some(Ok(_)) => {
            if crate::aimassist::is_enabled() {
                "on"
            } else {
                "ready"
            }
        }
    }
}

pub fn status_json(cfg: &AimConfig) -> Value {
    let state = card_state(cfg);
    let s = STATE.lock();
    let (got, total) = match s.phase {
        Phase::Downloading { got, total } => (got, total),
        _ => (0, 0),
    };
    let path = crate::aimassist::model_path(cfg);
    let meta = if cfg.model.is_none() { read_meta(&dir()) } else { None };
    let (check_ok, check_err, summary) = match &s.check {
        Some(Ok(sm)) => (true, String::new(), json!({ "layout": sm.layout, "input": format!("{}×{}", sm.input.0, sm.input.1), "classes": sm.classes, "trial_ms": sm.trial_ms, "device": sm.device })),
        Some(Err(e)) => (false, e.clone(), Value::Null),
        None => (false, String::new(), Value::Null),
    };
    json!({
        "state": state, "busy": is_busy(), "percent": if total > 0 { (got * 100 / total).min(100) } else { 0 }, "got_mb": got / 1_048_576, "total_mb": total / 1_048_576,
        "installed": path.is_file(), "size_mb": std::fs::metadata(&path).map(|m| m.len() / 1_048_576).unwrap_or(0),
        "checked": s.check.is_some(), "check_ok": check_ok, "check_error": check_err, "summary": summary,
        "install_error": s.install_error, "last_ok": s.last_ok,
        "meta": meta.unwrap_or(Value::Null),
        "official": { "title": OFFICIAL.title, "download_mb": download_size_mb(), "license": OFFICIAL.license, "source": OFFICIAL.source_page, "note": OFFICIAL.note },
        "network": cfg!(feature = "lua"), "runtime": cfg!(feature = "ort"),
        "directml_build": cfg!(feature = "directml"),
    })
}

fn file_name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

// The report that is copied from the card. Only what the user can see anyway: no keys, no screenshots, no voice, no folders of the user.
pub fn report_text(cfg: &AimConfig) -> String {
    let st = status_json(cfg);
    let aim = crate::aimassist::status_json(cfg);
    let eff = crate::aimassist::effective(cfg);
    let mut r = String::from("Помощь прицела — отчёт\n");
    let yn = |b: bool| if b { "да" } else { "нет" };
    let path = crate::aimassist::model_path(cfg);
    r.push_str(&format!("Состояние карточки: {}\n", st["state"].as_str().unwrap_or("")));
    r.push_str(&format!("Модель установлена: {}", yn(st["installed"].as_bool().unwrap_or(false))));
    if path.is_file() {
        r.push_str(&format!(" ({}, {} МБ)", file_name_of(&path), st["size_mb"]));
    }
    r.push('\n');
    if let Some(m) = st["meta"].as_object() {
        r.push_str(&format!("Название: {}\nОткуда: {} ({})\nЛицензия: {}\nSHA-256: {}\n", m.get("name").and_then(|v| v.as_str()).unwrap_or(""), if m.get("kind").and_then(|v| v.as_str()) == Some("official") { "официальная загрузка" } else { "файл пользователя" }, m.get("source").and_then(|v| v.as_str()).unwrap_or("—"), m.get("license").and_then(|v| v.as_str()).unwrap_or("—"), m.get("sha256").and_then(|v| v.as_str()).unwrap_or("—")));
        r.push_str("Проверка целостности: при установке SHA-256 и размер ");
        r.push_str(if m.get("kind").and_then(|v| v.as_str()) == Some("official") { "сверены с ожидаемыми — совпали\n" } else { "посчитаны (для чужого файла эталона нет)\n" });
    }
    if st["checked"].as_bool().unwrap_or(false) {
        if st["check_ok"].as_bool().unwrap_or(false) {
            let s = &st["summary"];
            r.push_str(&format!("Проверка совместимости: успешно — формат {}, вход {}, классов {}, пробный запуск {} мс\n", s["layout"].as_str().unwrap_or(""), s["input"].as_str().unwrap_or(""), s["classes"], s["trial_ms"]));
        } else {
            r.push_str(&format!("Проверка совместимости: ОШИБКА — {}\n", st["check_error"].as_str().unwrap_or("")));
        }
    } else {
        r.push_str("Проверка совместимости: ещё не выполнялась\n");
    }
    if let Some(e) = st["install_error"].as_str().filter(|e| !e.is_empty()) {
        r.push_str(&format!("Последняя ошибка установки: {}\n", e));
    }
    r.push_str(&format!("Среда выполнения: ONNX Runtime {} (Python не нужен)\n", if st["runtime"].as_bool().unwrap_or(false) { "встроен в программу" } else { "НЕ встроен в эту сборку" }));
    r.push_str(&format!("Устройство: выбрано «{}»; сейчас: {}\n", eff.device.title(), aim["device_used"].as_str().filter(|d| !d.is_empty()).unwrap_or("модель ещё не запускалась")));
    r.push_str(&format!("DirectML в сборке: {}\n", yn(cfg!(feature = "directml"))));
    r.push_str(&format!("Настройки: кадр {}×{}, до {} кадров/с, порог {}%, размер человека {}…{}% кадра (мин. {} пикс.), перекрестие {}/{} % от середины\n", eff.side, eff.side, eff.fps, eff.conf_pct, 0, eff.max_size_pct, eff.min_size_px, eff.cx_pct, eff.cy_pct));
    r.push_str(&format!("Помощь прицела: {} (состояние: {}, поток: {})\n", if aim["enabled"].as_bool().unwrap_or(false) { "включена" } else { "выключена" }, aim["state"].as_str().unwrap_or(""), yn(aim["thread"].as_bool().unwrap_or(false))));
    r.push_str(&format!("Захват экрана: {}\n", aim["capture"].as_str().unwrap_or("")));
    r.push_str(&format!("Игра: {}; перекрестие: {}; поиск перекрестия: {}\n", aim["last_game"].as_str().filter(|g| !g.is_empty()).unwrap_or("ещё не видели"), aim["cross"].as_str().unwrap_or(""), if eff.auto_cross { "включён" } else { "выключен" }));
    r.push_str(&format!("Время кадра: {} мс; найдено людей: {}; кадров обработано: {}\n", aim["last_frame_ms"], aim["objects"], aim["frames"]));
    r.push_str(&crate::aimassist::snapshot_details());
    r.push('\n');
    if let Some(p) = aim["problem"].as_str().filter(|p| !p.is_empty()) {
        r.push_str(&format!("Последняя ошибка работы: {}\n", p));
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aimassist::Device;
    use std::io::Cursor;

    // a source in memory; `fail_after` cuts the stream with an error, `tell_len` is the length that the "server" tells
    struct Fake {
        data: Vec<u8>,
        tell_len: Option<u64>,
        fail_after: Option<usize>,
        open_error: Option<String>,
    }
    struct Cut {
        inner: Cursor<Vec<u8>>,
        left: usize,
    }
    impl Read for Cut {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset"));
            }
            let take = b.len().min(self.left);
            let n = self.inner.read(&mut b[..take])?;
            self.left -= n;
            Ok(n)
        }
    }
    impl Source for Fake {
        fn open(&self) -> Result<(Option<u64>, Box<dyn Read>), String> {
            if let Some(e) = &self.open_error {
                return Err(e.clone());
            }
            let len = self.tell_len.or(Some(self.data.len() as u64));
            Ok(match self.fail_after {
                Some(n) => (len, Box::new(Cut { inner: Cursor::new(self.data.clone()), left: n })),
                None => (len, Box::new(Cursor::new(self.data.clone()))),
            })
        }
    }

    fn manifest_for(data: &[u8]) -> Manifest {
        use sha2::{Digest, Sha256};
        let sha: String = Sha256::digest(data).iter().map(|b| format!("{:02x}", b)).collect();
        Manifest { id: "t", title: "T", url: "https://example.invalid/m.onnx", sha256: Box::leak(sha.into_boxed_str()), size: data.len() as u64, license: "test", source_page: "test", note: "" }
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jarvis-aimmodel-test-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn good_probe(_: &Path) -> Result<Summary, String> {
        Ok(Summary { layout: "YOLOX".into(), input: (640, 640), classes: 80, trial_ms: 5, device: "CPU".into() })
    }
    fn bad_probe(_: &Path) -> Result<Summary, String> {
        Err("выход модели не похож на детектор".into())
    }

    fn data(n: usize, seed: u8) -> Vec<u8> {
        (0..n).map(|i| (i as u8).wrapping_mul(7).wrapping_add(seed)).collect()
    }

    fn run(src: &Fake, m: &Manifest, dir: &Path, probe: Probe) -> Result<Installed, String> {
        install_from(src, m, dir, probe, &AtomicBool::new(false), &|_, _| {})
    }

    #[test]
    fn the_official_manifest_is_pinned_and_looks_right() {
        assert!(OFFICIAL.url.starts_with("https://github.com/opencv/opencv_zoo/raw/"));
        assert!(OFFICIAL.url.contains("47534e27c9851bb1128ccc0102f1145e27f23f98"), "the address names a commit, not a branch");
        assert_eq!(OFFICIAL.sha256.len(), 64);
        assert!(OFFICIAL.sha256.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(OFFICIAL.size, 35_858_002);
        assert_eq!(download_size_mb(), 35);
    }

    #[test]
    fn a_good_download_is_checked_and_put_in_place() {
        let d = tmp("good");
        let bytes = data(300_000, 1);
        let m = manifest_for(&bytes);
        let seen = Mutex::new(0u64);
        let got = install_from(&Fake { data: bytes.clone(), tell_len: None, fail_after: None, open_error: None }, &m, &d, &good_probe, &AtomicBool::new(false), &|g, t| {
            assert_eq!(t, bytes.len() as u64);
            *seen.lock() = g;
        })
        .unwrap();
        assert_eq!(*seen.lock(), bytes.len() as u64, "the progress reached the end");
        assert_eq!(std::fs::read(active_file(&d)).unwrap(), bytes);
        assert!(!d.join("incoming.onnx").exists(), "no temporary file is left");
        assert!(got.backup.is_none());
        let meta = read_meta(&d).unwrap();
        assert_eq!(meta["kind"], "official");
        assert_eq!(meta["sha256"], m.sha256);
        assert_eq!(meta["license"], "test");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_wrong_checksum_is_refused_and_nothing_changes() {
        let d = tmp("sum");
        let bytes = data(50_000, 1);
        let mut other = bytes.clone();
        other[100] ^= 1; // the same size, one bit different
        let m = manifest_for(&bytes);
        let e = run(&Fake { data: other, tell_len: None, fail_after: None, open_error: None }, &m, &d, &good_probe).unwrap_err();
        assert!(e.contains("SHA-256"), "{}", e);
        assert!(!active_file(&d).exists());
        assert!(!d.join("incoming.onnx").exists(), "the bad file is deleted");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_download_that_breaks_or_has_the_wrong_size_leaves_the_old_model_alone() {
        let d = tmp("keep");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(active_file(&d), b"OLD MODEL").unwrap();
        let bytes = data(80_000, 2);
        let m = manifest_for(&bytes);
        // the connection is cut in the middle
        let e = run(&Fake { data: bytes.clone(), tell_len: None, fail_after: Some(30_000), open_error: None }, &m, &d, &good_probe).unwrap_err();
        assert!(e.contains("прервалась"), "{}", e);
        // the server says another size
        let e = run(&Fake { data: bytes.clone(), tell_len: Some(1234), fail_after: None, open_error: None }, &m, &d, &good_probe).unwrap_err();
        assert!(e.contains("размер"), "{}", e);
        // the server gives less than it should, then more
        let e = run(&Fake { data: bytes[..bytes.len() - 10].to_vec(), tell_len: Some(bytes.len() as u64), fail_after: None, open_error: None }, &m, &d, &good_probe).unwrap_err();
        assert!(e.contains("оборвалась"), "{}", e);
        let mut bigger = bytes.clone();
        bigger.extend_from_slice(&[0; 99]);
        assert!(run(&Fake { data: bigger, tell_len: Some(bytes.len() as u64), fail_after: None, open_error: None }, &m, &d, &good_probe).is_err());
        // no network at all
        let e = run(&Fake { data: vec![], tell_len: None, fail_after: None, open_error: Some("нет связи с сервером загрузки".into()) }, &m, &d, &good_probe).unwrap_err();
        assert!(e.contains("нет связи"));
        // after all of it the old model is as it was and nothing is left beside
        assert_eq!(std::fs::read(active_file(&d)).unwrap(), b"OLD MODEL");
        assert!(!d.join("incoming.onnx").exists());
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_file_that_does_not_fit_as_a_detector_is_refused_after_the_checksum_and_the_old_model_stays() {
        let d = tmp("compat");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(active_file(&d), b"OLD MODEL").unwrap();
        let bytes = data(40_000, 3);
        let m = manifest_for(&bytes);
        let e = run(&Fake { data: bytes, tell_len: None, fail_after: None, open_error: None }, &m, &d, &bad_probe).unwrap_err();
        assert!(e.contains("не прошла проверку"), "{}", e);
        assert_eq!(std::fs::read(active_file(&d)).unwrap(), b"OLD MODEL");
        assert!(!d.join("incoming.onnx").exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn cancelling_stops_the_download_and_cleans_up() {
        let d = tmp("cancel");
        let bytes = data(400_000, 4);
        let m = manifest_for(&bytes);
        let cancel = AtomicBool::new(false);
        let n = std::sync::atomic::AtomicU32::new(0);
        let e = install_from(&Fake { data: bytes, tell_len: None, fail_after: None, open_error: None }, &m, &d, &good_probe, &cancel, &|_, _| {
            if n.fetch_add(1, Ordering::SeqCst) == 2 {
                cancel.store(true, Ordering::SeqCst);
            }
        })
        .unwrap_err();
        assert_eq!(e, "cancelled");
        assert!(!d.join("incoming.onnx").exists());
        assert!(!active_file(&d).exists());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reinstalling_keeps_the_previous_model_beside() {
        let d = tmp("again");
        for (i, seed) in [5u8, 6].iter().enumerate() {
            let bytes = data(60_000, *seed);
            let m = manifest_for(&bytes);
            let got = run(&Fake { data: bytes.clone(), tell_len: None, fail_after: None, open_error: None }, &m, &d, &good_probe).unwrap();
            assert_eq!(std::fs::read(active_file(&d)).unwrap(), bytes);
            assert_eq!(got.backup.is_some(), i == 1);
        }
        let baks: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().starts_with("player.onnx.bak-")).collect();
        assert_eq!(baks.len(), 1);
        assert_eq!(std::fs::read(baks[0].path()).unwrap(), data(60_000, 5));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_file_of_the_user_is_checked_not_trusted_by_its_name() {
        let d = tmp("manual");
        let src = tmp("manual-src");
        std::fs::create_dir_all(&src).unwrap();
        let ok = src.join("мой детектор.onnx");
        std::fs::write(&ok, data(20_000, 7)).unwrap();
        // a good file
        let got = install_manual(&ok, &d, &good_probe).unwrap();
        assert_eq!(std::fs::read(active_file(&d)).unwrap(), data(20_000, 7));
        assert_eq!(read_meta(&d).unwrap()["kind"], "manual");
        assert_eq!(got.size, 20_000);
        // a second, that does not pass the check: the first stays
        let other = src.join("плохая.onnx");
        std::fs::write(&other, data(30_000, 8)).unwrap();
        assert!(install_manual(&other, &d, &bad_probe).unwrap_err().contains("не прошла проверку"));
        assert_eq!(std::fs::read(active_file(&d)).unwrap(), data(20_000, 7));
        assert!(!d.join("incoming.onnx").exists());
        // a wrong name, a missing file, a tiny file, the installed file itself, a folder
        let txt = src.join("a.txt");
        std::fs::write(&txt, data(20_000, 1)).unwrap();
        assert!(install_manual(&txt, &d, &good_probe).is_err());
        assert!(install_manual(&src.join("нет.onnx"), &d, &good_probe).is_err());
        let tiny = src.join("tiny.onnx");
        std::fs::write(&tiny, b"x").unwrap();
        assert!(install_manual(&tiny, &d, &good_probe).is_err());
        assert!(install_manual(&active_file(&d), &d, &good_probe).unwrap_err().contains("уже установлена"));
        let folder = src.join("dir.onnx");
        std::fs::create_dir_all(&folder).unwrap();
        assert!(install_manual(&folder, &d, &good_probe).is_err());
        std::fs::remove_dir_all(&d).ok();
        std::fs::remove_dir_all(&src).ok();
    }

    // the real check with the real ONNX Runtime on the fake models that are in the repository
    #[cfg(feature = "ort")]
    #[test]
    fn the_real_check_takes_detectors_and_refuses_a_classifier_and_rubbish() {
        let cfg = AimConfig { device: Device::Cpu, ..AimConfig::default() };
        let probe = real_probe(&cfg);
        let td = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata");
        let d = tmp("real");
        let got = install_manual(&td.join("aim_yolox_dummy.onnx"), &d, &probe).unwrap();
        assert!(got.summary.layout.contains("YOLOX"), "{:?}", got.summary);
        assert_eq!(got.summary.input, (640, 640));
        // another detector replaces it, the YOLOX one is kept
        let got = install_manual(&td.join("aim_dummy.onnx"), &d, &probe).unwrap();
        assert!(got.summary.layout.contains("YOLOv8"), "{:?}", got.summary);
        assert!(got.backup.is_some());
        // a classifier is refused, the detector in place stays
        let e = install_manual(&td.join("aim_classifier_dummy.onnx"), &d, &probe).unwrap_err();
        assert!(e.contains("не прошла проверку"), "{}", e);
        // rubbish under the name .onnx
        let junk = d.join("junk.onnx");
        std::fs::write(&junk, vec![3u8; 4000]).unwrap();
        assert!(install_manual(&junk, &d, &probe).is_err());
        let before = std::fs::read(active_file(&d)).unwrap();
        assert_eq!(before, std::fs::read(td.join("aim_dummy.onnx")).unwrap());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_card_state_and_the_report_tell_the_truth_and_hold_no_secrets() {
        let cfg = AimConfig { model: Some(tmp("card").join("none.onnx").display().to_string()), ..AimConfig::default() };
        {
            let mut s = STATE.lock();
            s.phase = Phase::Idle;
            s.install_error.clear();
            s.check = None;
        }
        assert_eq!(card_state(&cfg), "not_installed");
        STATE.lock().phase = Phase::Downloading { got: 5 * 1_048_576, total: 10 * 1_048_576 };
        assert_eq!(card_state(&cfg), "downloading");
        assert_eq!(status_json(&cfg)["percent"], 50);
        STATE.lock().phase = Phase::Idle;
        STATE.lock().install_error = "x".into();
        assert_eq!(card_state(&cfg), "install_error");
        STATE.lock().install_error.clear();
        let r = report_text(&cfg);
        for want in ["Модель установлена: нет", "Среда выполнения", "DirectML в сборке", "Захват экрана", "Время кадра"] {
            assert!(r.contains(want), "{}\n{}", want, r);
        }
        let low = r.to_lowercase();
        for secret in ["api_key", "secret", "token", "password", "c:\\users"] {
            assert!(!low.contains(secret), "{}", secret);
        }
    }
}
