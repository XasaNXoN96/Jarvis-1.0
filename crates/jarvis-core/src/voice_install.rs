// Installs the Jarvis voice (Piper) next to the program: the Piper program (about 22 MB) and a voice (about 60 MB).
// For a computer where only the small installer was used and Jarvis would otherwise speak with the Windows voice.
// Started from Settings → Voice. Downloads from the official Piper release and the Piper voices repository.

use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::APP_DIR;

const PIPER_ZIP: &str = "https://github.com/rhasspy/piper/releases/download/2023.11.14-2/piper_windows_amd64.zip";
// the archive of that release; nothing else is unpacked (checked by psrun right before the unpacking)
pub(crate) const PIPER_ZIP_SHA256: &str = "f3c58906402b24f3a96d92145f58acba6d86c9b5db896d207f78dc80811efcea";
// it lies in the folder of the program (not in the shared temporary folder), where only the owner of the program writes
pub(crate) const ZIP_NAME: &str = "jarvis_piper.zip";

#[derive(Default, Clone)]
struct Status {
    running: bool,
    done: bool,
    step: String,
    percent: u8,
    error: String,
}

static STATUS: Lazy<Mutex<Status>> = Lazy::new(|| Mutex::new(Status::default()));

pub fn status_json() -> Value {
    let s = STATUS.lock().clone();
    json!({ "running": s.running, "done": s.done, "step": s.step, "percent": s.percent, "error": s.error, "installed": crate::tts::piper_available() })
}

fn set(step: &str, percent: u8) {
    let mut s = STATUS.lock();
    s.step = step.to_string();
    s.percent = percent;
}

// the voice of the language: (folder on the voices server, file name)
fn voice_for(lang: &str) -> Option<(&'static str, &'static str)> {
    match lang {
        "ru" => Some(("ru/ru_RU/irina/medium", "ru_RU-irina-medium")),
        "en" => Some(("en/en_US/amy/medium", "en_US-amy-medium")),
        _ => None,
    }
}

#[cfg(feature = "lua")]
fn download(url: &str, dest: &Path, label: &str, from: u8, to: u8) -> Result<(), String> {
    use std::io::{Read, Write};
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .timeout(std::time::Duration::from_secs(60 * 40))
        .build()
        .map_err(|e| e.to_string())?;
    let mut resp = client.get(url).send().map_err(|_| "нет соединения с интернетом или сервер недоступен".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("сервер ответил {}", resp.status()));
    }
    let total = resp.content_length().unwrap_or(0);
    let part = dest.with_extension("part");
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|_| "нет прав записи в папку Jarvis (поставьте Jarvis в папку пользователя или используйте большой установщик)".to_string())?;
    }
    let mut file = std::fs::File::create(&part).map_err(|_| "нет прав записи в папку Jarvis (поставьте Jarvis в папку пользователя или используйте большой установщик)".to_string())?;
    let mut buf = [0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let n = resp.read(&mut buf).map_err(|_| "связь прервалась во время загрузки".to_string())?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if total > 0 {
            let p = from as u64 + (to - from) as u64 * done / total;
            set(&format!("{}: {} из {} МБ", label, done / 1_000_000, total / 1_000_000), p.min(to as u64) as u8);
        }
    }
    drop(file);
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

#[cfg(not(feature = "lua"))]
fn download(_url: &str, _dest: &Path, _label: &str, _from: u8, _to: u8) -> Result<(), String> {
    Err("в этой сборке нет сетевого модуля".to_string())
}

fn run_install() -> Result<(), String> {
    let lang = match crate::i18n::get_language().as_str() {
        "ua" => "uk".to_string(),
        other => other.to_string(),
    };
    let (folder, name) = voice_for(&lang).ok_or_else(|| "для вашего языка голоса Piper пока нет".to_string())?;
    let piper_dir: PathBuf = APP_DIR.join("piper");
    let exe = piper_dir.join(if cfg!(windows) { "piper.exe" } else { "piper" });

    if !exe.exists() {
        let zip = APP_DIR.join(ZIP_NAME);
        download(PIPER_ZIP, &zip, "Программа голоса", 0, 25)?;
        set("Распаковываю…", 26);
        #[cfg(target_os = "windows")]
        {
            let _ = std::fs::remove_file(&zip.with_extension("part"));
            let status = crate::psrun::quiet(crate::psrun::Script::InstallPiper, &[("JARVIS_ZIP", zip.display().to_string()), ("JARVIS_DEST", APP_DIR.display().to_string())])
                .map_err(|e| {
                    let _ = std::fs::remove_file(&zip);
                    format!("распаковка отклонена защитой: {}", e)
                })?
                .status()
                .map_err(|e| e.to_string())?;
            if !status.success() {
                return Err("не получилось распаковать (нет прав записи в папку Jarvis?)".to_string());
            }
        }
        let _ = std::fs::remove_file(&zip);
        if !exe.exists() {
            return Err("после распаковки не найден piper.exe".to_string());
        }
    }

    let voices = piper_dir.join("voices");
    let model = voices.join(format!("{}.onnx", lang));
    if !model.exists() {
        let base = format!("https://huggingface.co/rhasspy/piper-voices/resolve/main/{}/{}", folder, name);
        download(&format!("{}.onnx", base), &model, "Голос", 30, 96)?;
        download(&format!("{}.onnx.json", base), &voices.join(format!("{}.onnx.json", lang)), "Настройки голоса", 96, 99)?;
    }
    Ok(())
}

pub fn start() -> Result<(), String> {
    {
        let mut s = STATUS.lock();
        if s.running {
            return Err("установка уже идёт".to_string());
        }
        *s = Status { running: true, step: "Начинаю…".to_string(), ..Default::default() };
    }
    std::thread::spawn(|| {
        let result = run_install();
        let mut s = STATUS.lock();
        s.running = false;
        match result {
            Ok(()) => {
                s.done = true;
                s.percent = 100;
                s.step = "Готово".to_string();
            }
            Err(e) => {
                warn!("Voice install failed: {}", e);
                s.error = e;
            }
        }
    });
    Ok(())
}
