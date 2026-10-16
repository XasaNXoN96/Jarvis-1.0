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

// A pin of one voice file: the commit of the voices repository the file is taken from (a URL with a commit, not the branch
// `main`, which changes), the SHA-256 and the size of exactly that file. Nothing is pinned yet: the values must come from the
// procedure in docs/SECURITY-DOWNLOADS.md (the «voice-pins» workflow downloads each file at a commit, checks the bytes against
// the LFS id the repository states for that commit, and prints the lines below for a person to review and commit). A value
// that was not obtained that way is not entered: it is not a proof of anything.
//   VoicePin { file: "ru.onnx", revision: "<40 hex>", sha256: "<64 hex>", size: <bytes> },
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoicePin {
    pub file: &'static str,
    pub revision: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

pub(crate) const VOICE_PINS: &[VoicePin] = &[];

#[cfg(test)]
thread_local! {
    static TEST_PINS: std::cell::RefCell<Vec<VoicePin>> = std::cell::RefCell::new(Vec::new());
}
#[cfg(test)]
pub(crate) fn set_test_pins(v: Vec<VoicePin>) {
    TEST_PINS.with(|t| *t.borrow_mut() = v);
}

pub fn voice_pin(file: &str) -> Option<VoicePin> {
    #[cfg(test)]
    {
        let t = TEST_PINS.with(|t| t.borrow().clone());
        if !t.is_empty() {
            return t.into_iter().find(|p| p.file == file);
        }
    }
    VOICE_PINS.iter().find(|p| p.file == file).cloned()
}

// is a table entry well formed (a typo in a pin must fail a test, not weaken a download)
pub fn pin_is_well_formed(p: &VoicePin) -> bool {
    let hex = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    hex(p.sha256, 64) && hex(p.revision, 40) && p.size > 1000 && matches!(p.file, "ru.onnx" | "ru.onnx.json" | "en.onnx" | "en.onnx.json")
}

// the address of one voice file: at the pinned commit when there is a pin, else the branch (and the file is only checked for shape)
pub(crate) fn voice_url(folder: &str, name: &str, suffix: &str, local_file: &str) -> String {
    let rev = voice_pin(local_file).map(|p| p.revision.to_string()).unwrap_or_else(|| "main".to_string());
    format!("https://huggingface.co/rhasspy/piper-voices/resolve/{}/{}/{}{}", rev, folder, name, suffix)
}

// are all the files of the voice pinned (shown in the status: «not pinned» is a visible state, not a hidden one)
pub fn voice_pinned(lang: &str) -> bool {
    [format!("{}.onnx", lang), format!("{}.onnx.json", lang)].iter().all(|f| voice_pin(f).is_some())
}

// the shape of a downloaded voice file; with a pin, the exact hash
pub(crate) fn verify_voice_file(path: &Path, file: &str) -> Result<(), String> {
    verify_voice_with(path, file, voice_pin(file).as_ref())
}

pub(crate) fn verify_voice_with(path: &Path, file: &str, pin: Option<&VoicePin>) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|_| "файл голоса не найден после загрузки".to_string())?;
    if let Some(pin) = pin {
        // the size first (cheap), then the hash: exactly the pinned file or nothing
        if meta.len() != pin.size {
            return Err(format!("размер файла голоса {} не совпал с закреплённым", file));
        }
        let got = crate::integrity::sha256_file(path).map_err(|e| e.to_string())?;
        if got != pin.sha256 {
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
    json!({ "running": s.running, "done": s.done, "step": s.step, "percent": s.percent, "error": s.error, "installed": crate::tts::piper_available(),
        // the voice files are checked by a pinned hash only when a pin exists; false is shown as it is, not hidden
        "voice_pinned": voice_pinned(&match crate::i18n::get_language().as_str() { "ua" => "uk".to_string(), o => o.to_string() }) })
}

fn set(step: &str, percent: u8) {
    let mut s = STATUS.lock();
    s.step = step.to_string();
    s.percent = percent;
}

// More Russian voices of Piper (the same files server, chosen by the user in Settings → Voice): (id, folder, file name, label).
// They lie next to the main voice as «ru-<id>.onnx»; only these ids are ever taken (the id becomes a part of a file name).
pub const EXTRA_RU: [(&str, &str, &str, &str); 3] = [
    ("denis", "ru/ru_RU/denis/medium", "ru_RU-denis-medium", "Денис (мужской, Piper)"),
    ("dmitri", "ru/ru_RU/dmitri/medium", "ru_RU-dmitri-medium", "Дмитрий (мужской, Piper)"),
    ("ruslan", "ru/ru_RU/ruslan/medium", "ru_RU-ruslan-medium", "Руслан (мужской, Piper)"),
];
pub const EXTRA_PREFIX: &str = "piper:";

// "piper:denis" -> "denis" (only the known voices)
pub fn extra_id(choice: &str) -> Option<&'static str> {
    let name = choice.strip_prefix(EXTRA_PREFIX)?;
    EXTRA_RU.iter().find(|(id, ..)| *id == name).map(|(id, ..)| *id)
}

// the files of an extra voice are on the disk
pub fn extra_installed(id: &str) -> bool {
    let v = APP_DIR.join("piper").join("voices");
    extra_id(&format!("{}{}", EXTRA_PREFIX, id)).is_some() && v.join(format!("ru-{}.onnx", id)).exists() && v.join(format!("ru-{}.onnx.json", id)).exists()
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
    let mut file = crate::create_fresh(&part).map_err(|_| "нет прав записи в папку Jarvis (поставьте Jarvis в папку пользователя или используйте большой установщик)".to_string())?;
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

fn run_install(extra: Option<&'static str>) -> Result<(), String> {
    let lang = match crate::i18n::get_language().as_str() {
        "ua" => "uk".to_string(),
        other => other.to_string(),
    };
    // an extra voice is a Russian one: its own folder, name and local file name
    let (folder, name, local) = match extra.and_then(|id| EXTRA_RU.iter().find(|(i, ..)| *i == id)) {
        Some((id, folder, name, _)) => (*folder, *name, format!("ru-{}", id)),
        None => {
            let (f, n) = voice_for(&lang).ok_or_else(|| "для вашего языка голоса Piper пока нет".to_string())?;
            (f, n, lang.clone())
        }
    };
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
            let mut unpack = crate::psrun::quiet(crate::psrun::Script::InstallPiper, &[("JARVIS_ZIP", zip.display().to_string()), ("JARVIS_DEST", APP_DIR.display().to_string())])
                .map_err(|e| {
                    let _ = std::fs::remove_file(&zip);
                    format!("распаковка отклонена защитой: {}", e)
                })?;
            // all three standard handles NULL (status() would inherit the output handles: «os error 50» without a console)
            crate::null_stdio(&mut unpack);
            let status = unpack.status().map_err(|e| format!("PowerShell не запустился для распаковки: {}", e))?;
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
    let model = voices.join(format!("{}.onnx", local));
    if !model.exists() {
        let file = format!("{}.onnx", local);
        download(&voice_url(folder, name, ".onnx", &file), &model, "Голос", 30, 96, &|p| verify_voice_file(p, &file))?;
        let jfile = format!("{}.onnx.json", local);
        download(&voice_url(folder, name, ".onnx.json", &jfile), &voices.join(&jfile), "Настройки голоса", 96, 99, &|p| verify_voice_file(p, &jfile))?;
        if voice_pin(&file).is_none() {
            warn!("Voice {}: no pinned SHA-256 for this file yet (shape checked only)", file);
        }
    }
    Ok(())
}

pub fn start(voice: Option<&str>) -> Result<(), String> {
    let extra = match voice {
        Some(v) if !v.is_empty() => Some(extra_id(&format!("{}{}", EXTRA_PREFIX, v)).ok_or_else(|| "такого голоса нет".to_string())?),
        _ => None,
    };
    {
        let mut s = STATUS.lock();
        if s.running {
            return Err("установка уже идёт".to_string());
        }
        *s = Status { running: true, step: "Начинаю…".to_string(), ..Default::default() };
    }
    std::thread::spawn(move || {
        let result = run_install(extra);
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
    fn only_known_extra_voices_are_taken_and_their_files_are_named_by_the_table() {
        assert_eq!(extra_id("piper:denis"), Some("denis"));
        assert_eq!(extra_id("piper:ruslan"), Some("ruslan"));
        assert_eq!(extra_id("piper"), None);
        assert_eq!(extra_id("piper:../../x"), None);
        assert_eq!(extra_id("piper:irina"), None);
        assert_eq!(extra_id(""), None);
        assert!(start(Some("../evil")).is_err(), "an unknown voice does not start a download");
        for (id, folder, name, _) in EXTRA_RU {
            assert!(folder.ends_with("medium") && folder.contains(id) && name == format!("ru_RU-{}-medium", id));
            assert!(voice_url(folder, name, ".onnx", &format!("ru-{}.onnx", id)).starts_with("https://huggingface.co/rhasspy/piper-voices/"));
        }
    }

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
        let hash: &'static str = Box::leak(good.into_boxed_str());
        let pin = VoicePin { file: "ru.onnx", revision: "0123456789abcdef0123456789abcdef01234567", sha256: hash, size: 1_500_000 };
        assert!(pin_is_well_formed(&pin));
        assert!(verify_voice_with(&model, "ru.onnx", Some(&pin)).is_ok());
        std::fs::write(&model, vec![9u8; 1_500_000]).unwrap();
        assert!(verify_voice_with(&model, "ru.onnx", Some(&pin)).unwrap_err().contains("SHA-256"), "a changed model of the same size passed its pin");
        std::fs::write(&model, vec![8u8; 1_499_999]).unwrap();
        assert!(verify_voice_with(&model, "ru.onnx", Some(&pin)).unwrap_err().contains("размер"), "a file of another size passed its pin");
        assert!(verify_voice_with(&d.path().join("none"), "x", None).is_err());
    }

    // the state of the pins is a fact the report states: nothing is pinned for the voices yet
    #[test]
    fn the_voice_files_are_not_pinned_yet_and_the_zip_is() {
        assert!(voice_pin("ru.onnx").is_none() && voice_pin("en.onnx").is_none());
        assert_eq!(PIPER_ZIP_SHA256.len(), 64);
    }

    #[test]
    fn a_pinned_file_is_taken_at_its_commit_and_an_unpinned_one_from_the_branch() {
        set_test_pins(Vec::new());
        assert!(voice_url("ru/ru_RU/irina/medium", "ru_RU-irina-medium", ".onnx", "ru.onnx").contains("/resolve/main/"));
        assert!(!voice_pinned("ru"));
        let rev = "0123456789abcdef0123456789abcdef01234567";
        let sha = "ab".repeat(32);
        let sha: &'static str = Box::leak(sha.into_boxed_str());
        set_test_pins(vec![
            VoicePin { file: "ru.onnx", revision: rev, sha256: sha, size: 60_000_000 },
            VoicePin { file: "ru.onnx.json", revision: rev, sha256: sha, size: 5_000 },
        ]);
        let u = voice_url("ru/ru_RU/irina/medium", "ru_RU-irina-medium", ".onnx", "ru.onnx");
        assert!(u.contains(&format!("/resolve/{}/", rev)) && !u.contains("/main/"), "{}", u);
        assert!(voice_pinned("ru") && !voice_pinned("en"), "only the files with a pin count as pinned");
        set_test_pins(Vec::new());
    }

    // a table entry that is malformed (a short hash, a branch instead of a commit, an unknown file) must fail here
    #[test]
    fn every_entry_of_the_pin_table_is_well_formed() {
        for p in VOICE_PINS {
            assert!(pin_is_well_formed(p), "malformed pin: {:?}", p);
        }
        let bad = [
            VoicePin { file: "ru.onnx", revision: "main", sha256: "a".repeat(64).leak(), size: 5000 },
            VoicePin { file: "ru.onnx", revision: "a".repeat(40).leak(), sha256: "abc", size: 5000 },
            VoicePin { file: "evil.exe", revision: "a".repeat(40).leak(), sha256: "a".repeat(64).leak(), size: 5000 },
            VoicePin { file: "ru.onnx", revision: "A".repeat(40).leak(), sha256: "a".repeat(64).leak(), size: 5000 },
        ];
        for p in &bad {
            assert!(!pin_is_well_formed(p), "{:?} was accepted", p);
        }
    }
}

