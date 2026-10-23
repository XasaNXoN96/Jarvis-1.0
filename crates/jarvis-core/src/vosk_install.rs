// Installs a speech recognition model (Vosk) next to the program. For a computer where only the small installer was used: without a model
// the microphone cannot understand anything. Started from Settings → Components. The archive comes from the official Vosk models server and
// is accepted only if its size and SHA-256 are exactly the pinned ones (obtained by the «vosk-model-hash» workflow, see docs); it is
// unpacked by this program (not by a shell) with every path checked, into a staging folder, and moves to its place only when it looks like
// a model. Nothing else is ever written outside the models folder.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

use crate::{config, APP_DIR};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelPin {
    pub id: &'static str,    // "ru"
    pub name: &'static str,  // the folder (and the file) name
    pub label: &'static str,
    pub size: u64,           // bytes of the archive
    pub sha256: &'static str,
}

pub const MODELS: [ModelPin; 3] = [
    ModelPin { id: "ru", name: "vosk-model-small-ru-0.22", label: "Распознавание речи: русский", size: 46_236_750, sha256: "961d5ff98a17f4aa6de69864d0aa71fa5bac682301d2b5d17a3f24c5c99a46d4" },
    ModelPin { id: "en", name: "vosk-model-small-en-us-0.15", label: "Распознавание речи: английский", size: 41_205_931, sha256: "30f26242c4eb449f948e42cb302dd7a686cb29a3423a8367f99ff41780942498" },
    ModelPin { id: "uk", name: "vosk-model-small-uk-v3-nano", label: "Распознавание речи: украинский", size: 77_622_640, sha256: "d6b50be0a2d20f891720f3a83c98290bb2d396fd666142dbf716e8ad302f8233" },
];

// limits of one archive: a model has some hundreds of files; a bigger count or size is not a model
const MAX_ENTRIES: usize = 3000;
const MAX_UNPACKED: u64 = 700 * 1024 * 1024;

pub fn pin(id: &str) -> Option<&'static ModelPin> {
    MODELS.iter().find(|m| m.id == id)
}

fn url(p: &ModelPin) -> String {
    format!("https://alphacephei.com/vosk/models/{}.zip", p.name)
}

pub fn models_dir() -> PathBuf {
    APP_DIR.join(config::VOSK_MODELS_PATH)
}

// a folder that is a usable model (the files the recognizer cannot start without)
fn looks_like_model(dir: &Path) -> bool {
    dir.join("am").join("final.mdl").is_file() && dir.join("conf").join("model.conf").is_file()
}

pub fn installed(p: &ModelPin) -> bool {
    looks_like_model(&models_dir().join(p.name))
}

pub fn size_mb(p: &ModelPin) -> u64 {
    fn walk(d: &Path) -> u64 {
        std::fs::read_dir(d).map(|r| r.flatten().map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => walk(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        }).sum()).unwrap_or(0)
    }
    walk(&models_dir().join(p.name)) / 1_048_576
}

// the archive on the disk is exactly the pinned one
pub(crate) fn verify_archive(path: &Path, p: &ModelPin) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|_| "архив модели не найден после загрузки".to_string())?;
    if meta.len() != p.size {
        return Err("размер загруженного архива модели не совпал с закреплённым: он удалён и не используется".into());
    }
    let got = crate::integrity::sha256_file(path).map_err(|e| e.to_string())?;
    if got != p.sha256 {
        return Err("SHA-256 загруженного архива модели не совпал с закреплённым: он удалён и не используется".into());
    }
    Ok(())
}

// Unpacks the archive into `staging`; every entry must lie inside the folder `name/` (no `..`, no drive, no backslash, no link), the number and the
// size of the entries are limited. On any problem the staging folder is removed and nothing is left.
pub(crate) fn extract(zip_path: &Path, staging: &Path, name: &str) -> Result<(), String> {
    let result = extract_inner(zip_path, staging, name);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(staging);
    }
    result
}

fn extract_inner(zip_path: &Path, staging: &Path, name: &str) -> Result<(), String> {
    let file = std::fs::File::open(zip_path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "архив модели повреждён".to_string())?;
    if zip.len() > MAX_ENTRIES {
        return Err("в архиве слишком много файлов для модели".into());
    }
    let _ = std::fs::remove_dir_all(staging);
    std::fs::create_dir_all(staging).map_err(|_| "нет прав записи в папку Jarvis (поставьте Jarvis в папку пользователя или используйте большой установщик)".to_string())?;
    let mut total = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(|_| "архив модели повреждён".to_string())?;
        let raw = entry.name().to_string();
        if raw.contains('\\') || raw.contains(':') || raw.starts_with('/') {
            return Err("в архиве недопустимое имя файла".into());
        }
        let rel = entry.enclosed_name().ok_or_else(|| "в архиве путь выходит за папку модели".to_string())?;
        let mut parts = rel.components();
        match parts.next() {
            Some(std::path::Component::Normal(first)) if first == name => {}
            _ => return Err("в архиве не та папка модели".into()),
        }
        if let Some(mode) = entry.unix_mode() {
            // a link (or a device) is not a file of a model
            if mode & 0o170000 != 0 && mode & 0o170000 != 0o100000 && mode & 0o170000 != 0o040000 {
                return Err("в архиве ссылка или особый файл".into());
            }
        }
        let target = staging.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| e.to_string())?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED {
            return Err("распакованная модель слишком велика".into());
        }
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut out = crate::create_fresh(&target).map_err(|e| e.to_string())?;
        // the declared size is not trusted: at most the limit is read
        let copied = std::io::copy(&mut (&mut entry).take(MAX_UNPACKED + 1), &mut out).map_err(|_| "не удалось записать файл модели".to_string())?;
        out.flush().ok();
        if copied > MAX_UNPACKED {
            return Err("файл в архиве больше заявленного".into());
        }
    }
    if !looks_like_model(&staging.join(name)) {
        return Err("в архиве нет файлов модели (am/final.mdl, conf/model.conf)".into());
    }
    Ok(())
}

#[derive(Default, Clone)]
struct Status {
    running: bool,
    done: bool,
    id: String,
    step: String,
    percent: u8,
    error: String,
}

static STATUS: Lazy<Mutex<Status>> = Lazy::new(|| Mutex::new(Status::default()));

fn set(step: &str, percent: u8) {
    let mut s = STATUS.lock();
    s.step = step.to_string();
    s.percent = percent;
}

pub fn status_json() -> Value {
    let s = STATUS.lock().clone();
    json!({ "running": s.running, "done": s.done, "id": s.id, "step": s.step, "percent": s.percent, "error": s.error })
}

pub fn is_busy() -> bool {
    STATUS.lock().running
}

fn run_install(p: &'static ModelPin) -> Result<(), String> {
    let dir = models_dir();
    if installed(p) {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|_| "нет прав записи в папку Jarvis (поставьте Jarvis в папку пользователя или используйте большой установщик)".to_string())?;
    let zip = dir.join(format!(".jarvis_vosk_{}.zip", p.id));
    let _ = std::fs::remove_file(&zip);
    crate::voice_install::download_with(&url(p), &zip, "Модель речи", 0, 85, &|f| verify_archive(f, p), &set)?;
    set("Распаковываю…", 88);
    let staging = dir.join(format!(".installing-{}", p.id));
    let r = extract(&zip, &staging, p.name);
    let _ = std::fs::remove_file(&zip);
    r?;
    set("Ставлю на место…", 97);
    let final_dir = dir.join(p.name);
    // a half-made folder of an earlier try is not a model: it is replaced
    if final_dir.exists() {
        std::fs::remove_dir_all(&final_dir).map_err(|e| format!("не удалось заменить неполную папку модели: {}", e))?;
    }
    let moved = std::fs::rename(staging.join(p.name), &final_dir).map_err(|e| e.to_string());
    let _ = std::fs::remove_dir_all(&staging);
    moved
}

pub fn start(id: &str) -> Result<(), String> {
    let p = pin(id).ok_or_else(|| "такой модели речи нет".to_string())?;
    {
        let mut s = STATUS.lock();
        if s.running {
            return Err("установка модели уже идёт".to_string());
        }
        *s = Status { running: true, id: id.to_string(), step: "Начинаю…".to_string(), ..Default::default() };
    }
    std::thread::spawn(move || {
        let result = run_install(p);
        let mut s = STATUS.lock();
        s.running = false;
        match result {
            Ok(()) => {
                s.done = true;
                s.percent = 100;
                s.step = "Готово. Перезапустите Джарвиса, чтобы он начал слушать.".to_string();
            }
            Err(e) => {
                warn!("Vosk model install failed: {}", e);
                s.error = e;
            }
        }
    });
    Ok(())
}

// removes exactly the folder of the named model (never a link, never a path that came from outside)
pub fn remove(id: &str) -> Result<bool, String> {
    let p = pin(id).ok_or_else(|| "такой модели речи нет".to_string())?;
    if is_busy() {
        return Err("Сейчас идёт установка модели.".into());
    }
    let path = models_dir().join(p.name);
    match std::fs::symlink_metadata(&path) {
        Err(_) => Ok(false),
        Ok(m) if m.file_type().is_symlink() => Err("папка модели — ссылка, я её не трогаю".into()),
        Ok(_) => std::fs::remove_dir_all(&path).map(|_| true).map_err(|e| format!("не удалось удалить модель (она сейчас используется? закройте Джарвиса и повторите): {}", e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    fn make_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (n, data) in entries {
            if n.ends_with('/') {
                w.add_directory(*n, o).unwrap();
            } else {
                w.start_file(*n, o).unwrap();
                w.write_all(data).unwrap();
            }
        }
        w.finish().unwrap();
    }

    const GOOD: [(&str, &[u8]); 3] = [("m1/am/final.mdl", b"x"), ("m1/conf/model.conf", b"y"), ("m1/graph/HCLG.fst", b"z")];

    #[test]
    fn the_table_is_well_formed_and_only_known_models_are_taken() {
        for m in MODELS.iter() {
            assert!(m.sha256.len() == 64 && m.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "{}", m.id);
            assert!(m.name.starts_with("vosk-model-") && !m.name.contains('/') && !m.name.contains(".."), "{}", m.name);
            assert!(m.size > 1_000_000);
            assert_eq!(url(m), format!("https://alphacephei.com/vosk/models/{}.zip", m.name));
        }
        assert!(pin("../x").is_none() && pin("").is_none() && pin("fr").is_none());
        assert!(start("../evil").is_err() && remove("../evil").is_err(), "an unknown model starts nothing and removes nothing");
    }

    #[test]
    fn a_good_archive_is_unpacked_and_looks_like_a_model() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("a.zip");
        make_zip(&z, &GOOD);
        let st = d.path().join("stage");
        extract(&z, &st, "m1").unwrap();
        assert!(looks_like_model(&st.join("m1")));
        assert_eq!(std::fs::read(st.join("m1/graph/HCLG.fst")).unwrap(), b"z");
    }

    #[test]
    fn a_bad_archive_leaves_nothing_behind() {
        let d = tempfile::tempdir().unwrap();
        let st = d.path().join("stage");
        let cases: Vec<(&str, Vec<(&str, &[u8])>)> = vec![
            ("path out of the folder", vec![("m1/am/final.mdl", b"x"), ("m1/../../evil.txt", b"x")]),
            ("absolute path", vec![("/etc/evil", b"x")]),
            ("backslash", vec![("m1\\..\\evil", b"x")]),
            ("drive", vec![("C:/evil", b"x")]),
            ("another folder", vec![("other/am/final.mdl", b"x"), ("other/conf/model.conf", b"y")]),
            ("no model files", vec![("m1/readme.txt", b"x")]),
            ("only half of a model", vec![("m1/am/final.mdl", b"x")]),
        ];
        for (what, entries) in cases {
            let z = d.path().join("b.zip");
            make_zip(&z, &entries);
            assert!(extract(&z, &st, "m1").is_err(), "{} must be refused", what);
            assert!(!st.exists(), "{}: the staging folder is removed", what);
        }
        assert!(!d.path().join("evil.txt").exists() && !d.path().parent().unwrap().join("evil.txt").exists());
        let junk = d.path().join("c.zip");
        std::fs::write(&junk, b"this is not a zip").unwrap();
        assert!(extract(&junk, &st, "m1").is_err());
    }

    #[test]
    fn a_link_in_the_archive_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let z = d.path().join("l.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&z).unwrap());
            let o = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
            w.start_file("m1/am/final.mdl", o).unwrap();
            w.write_all(b"x").unwrap();
            w.start_file("m1/conf/model.conf", o).unwrap();
            w.write_all(b"y").unwrap();
            w.add_symlink("m1/link", "/etc/passwd", o).unwrap();
            w.finish().unwrap();
        }
        assert!(extract(&z, &d.path().join("s"), "m1").unwrap_err().contains("ссылка"));
    }

    #[test]
    fn only_the_exact_pinned_archive_is_accepted() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.zip");
        std::fs::write(&f, b"hello model").unwrap();
        let h = crate::integrity::sha256_file(&f).unwrap();
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let ok = ModelPin { id: "ru", name: "vosk-model-t", label: "t", size: 11, sha256: leak(h.clone()) };
        assert!(verify_archive(&f, &ok).is_ok());
        let wrong_size = ModelPin { size: 12, ..ok.clone() };
        assert!(verify_archive(&f, &wrong_size).unwrap_err().contains("размер"));
        let wrong_hash = ModelPin { sha256: leak("0".repeat(64)), ..ok.clone() };
        assert!(verify_archive(&f, &wrong_hash).unwrap_err().contains("SHA-256"));
        assert!(verify_archive(&d.path().join("none.zip"), &ok).is_err());
    }
}
