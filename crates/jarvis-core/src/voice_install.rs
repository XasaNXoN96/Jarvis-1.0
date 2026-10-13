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

#[cfg(test)]
thread_local! {
    static TEST_PIN: std::cell::RefCell<Option<String>> = std::cell::RefCell::new(None);
}
#[cfg(test)]
pub(crate) fn set_test_pin(h: Option<String>) {
    TEST_PIN.with(|t| *t.borrow_mut() = h);
}
// the pinned hash (a test names its own for a small archive it makes)
pub(crate) fn pinned_zip_sha256() -> String {
    #[cfg(test)]
    if let Some(h) = TEST_PIN.with(|t| t.borrow().clone()) {
        return h;
    }
    PIPER_ZIP_SHA256.to_string()
}

// The voice files (the model and its settings) come from the voices repository `main` branch, which changes over time, and
// no hash of them has been pinned yet (the value must come from a trusted source, see docs/SECURITY-DOWNLOADS.md). Until
// then a file is only checked for shape; with a hash in this table it is checked exactly, and a mismatch deletes it.
pub(crate) const VOICE_PINS: &[(&str, &str)] = &[];

pub fn voice_pin(file: &str) -> Option<&'static str> {
    VOICE_PINS.iter().find(|(f, _)| *f == file).map(|(_, h)| *h)
}

// the shape of a downloaded voice file; with a pin, the exact hash
pub(crate) fn verify_voice_file(path: &Path, file: &str) -> Result<(), String> {
    verify_voice_with(path, file, voice_pin(file))
}

pub(crate) fn verify_voice_with(path: &Path, file: &str, pin: Option<&str>) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|_| "файл голоса не найден после загрузки".to_string())?;
    if let Some(want) = pin {
        let got = crate::integrity::sha256_file(path).map_err(|e| e.to_string())?;
        if got != want {
            return Err(format!("SHA-256 файла голоса {} не совпал с закреплённым", file));
        }
        return Ok(());
    }
    let bytes = if file.ends_with(".json") {
        let b = std::fs::read(path).map_err(|e| e.to_string())?;
        let v: Value = serde_json::from_slice(&b).map_err(|_| "настройки голоса — не JSON".to_string())?;
        if v.pointer("/audio/sample_rate").and_then(|x| x.as_u64()).is_none() || meta.len() > 2_000_000 {
            return Err("настройки голоса не такие, как у голосов Piper".to_string());
        }
        return Ok(());
    } else {
        // a model: a protobuf file of a believable size (not a page of an error, not an endless stream)
        meta.len()
    };
    if !(1_000_000..=400_000_000).contains(&bytes) {
        return Err(format!("файл модели голоса странного размера ({} байт)", bytes));
    }
    let mut head = [0u8; 1];
    use std::io::Read;
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut head)).map_err(|e| e.to_string())?;
    if head[0] == b'<' {
        return Err("вместо модели пришла веб-страница".to_string());
    }
    Ok(())
}
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
fn download(url: &str, dest: &Path, label: &str, from: u8, to: u8, verify: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), String> {
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
        let n = resp.read(&mut buf).map_err(|_| {
            let _ = std::fs::remove_file(&part);
            "связь прервалась во время загрузки".to_string()
        })?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            e.to_string()
        })?;
        done += n as u64;
        if total > 0 {
            let p = from as u64 + (to - from) as u64 * done / total;
            set(&format!("{}: {} из {} МБ", label, done / 1_000_000, total / 1_000_000), p.min(to as u64) as u8);
        }
    }
    drop(file);
    // checked BEFORE it gets its real name: a file that fails is deleted and never used
    if let Err(e) = verify(&part) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

#[cfg(not(feature = "lua"))]
fn download(_url: &str, _dest: &Path, _label: &str, _from: u8, _to: u8, _verify: &dyn Fn(&Path) -> Result<(), String>) -> Result<(), String> {
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
        let _ = std::fs::remove_file(&zip);
        download(PIPER_ZIP, &zip, "Программа голоса", 0, 25, &|p| {
            let h = crate::integrity::sha256_file(p).map_err(|e| e.to_string())?;
            if h == PIPER_ZIP_SHA256 { Ok(()) } else { Err("загруженный архив голоса не совпал с закреплённым SHA-256: он удалён и не используется".to_string()) }
        })?;
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
        let file = format!("{}.onnx", lang);
        download(&format!("{}.onnx", base), &model, "Голос", 30, 96, &|p| verify_voice_file(p, &file))?;
        let jfile = format!("{}.onnx.json", lang);
        download(&format!("{}.onnx.json", base), &voices.join(&jfile), "Настройки голоса", 96, 99, &|p| verify_voice_file(p, &jfile))?;
        if voice_pin(&file).is_none() {
            warn!("Voice {}: no pinned SHA-256 for this file yet (shape checked only)", file);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_voice_file_is_checked_by_its_pin_or_at_least_by_its_shape() {
        let d = tempfile::tempdir().unwrap();
        let json = d.path().join("ru.onnx.json");
        std::fs::write(&json, r#"{"audio":{"sample_rate":22050},"num_speakers":1}"#).unwrap();
        assert!(verify_voice_with(&json, "ru.onnx.json", None).is_ok());
        std::fs::write(&json, "<html>error</html>").unwrap();
        assert!(verify_voice_with(&json, "ru.onnx.json", None).is_err());
        std::fs::write(&json, r#"{"hello":1}"#).unwrap();
        assert!(verify_voice_with(&json, "ru.onnx.json", None).is_err(), "a JSON that is not a voice config");
        // a model: a web page instead of it, a tiny file, a good-looking one
        let model = d.path().join("ru.onnx");
        std::fs::write(&model, vec![b'<'; 2_000_000]).unwrap();
        assert!(verify_voice_with(&model, "ru.onnx", None).is_err());
        std::fs::write(&model, vec![8u8; 1000]).unwrap();
        assert!(verify_voice_with(&model, "ru.onnx", None).is_err());
        std::fs::write(&model, vec![8u8; 1_500_000]).unwrap();
        assert!(verify_voice_with(&model, "ru.onnx", None).is_ok());
        // with a pin only the exact file passes (a shape that would pass is not enough)
        let good = crate::integrity::sha256_file(&model).unwrap();
        assert!(verify_voice_with(&model, "ru.onnx", Some(&good)).is_ok());
        std::fs::write(&model, vec![9u8; 1_500_000]).unwrap();
        assert!(verify_voice_with(&model, "ru.onnx", Some(&good)).is_err(), "a changed model passed its pin");
        assert!(verify_voice_with(&d.path().join("none"), "x", None).is_err());
    }

    // the state of the pins is a fact the report states: nothing is pinned for the voices yet
    #[test]
    fn the_voice_files_are_not_pinned_yet_and_the_zip_is() {
        assert!(voice_pin("ru.onnx").is_none() && voice_pin("en.onnx").is_none());
        assert_eq!(PIPER_ZIP_SHA256.len(), 64);
    }
}
