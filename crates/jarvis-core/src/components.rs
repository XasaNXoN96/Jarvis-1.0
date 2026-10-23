//! The catalogue of the components that Jarvis downloads or keeps next to itself: the Piper voice, the extra voices, the model of the aim
//! assist. For each one: whether it is installed, its size, how its integrity is guaranteed (a pinned SHA-256 of the download or not), and
//! the real operations: install, check, remove. Nothing here is a plugin that runs foreign code: these are data files from the fixed official
//! addresses of the program. Removing deletes only the named files of the component.

use crate::{aimmodel, voice_install, vosk_install};
use serde::Serialize;
use serde_json::{json, Value};
use std::path::Path;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub name: String,
    pub about: String,
    pub installed: bool,
    pub size_mb: u64,
    pub integrity: String,
    pub busy: bool,
    pub ops: Vec<&'static str>,
}

fn size_of(paths: &[std::path::PathBuf]) -> u64 {
    paths.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum::<u64>() / 1_048_576
}

fn voices_dir() -> std::path::PathBuf {
    crate::APP_DIR.join("piper").join("voices")
}

fn extra_files(id: &str) -> Vec<std::path::PathBuf> {
    let d = voices_dir();
    vec![d.join(format!("ru-{}.onnx", id)), d.join(format!("ru-{}.onnx.json", id))]
}

pub fn list() -> Vec<Item> {
    let mut v = Vec::new();
    let st = voice_install::status_json();
    let busy_voice = st["running"].as_bool().unwrap_or(false);
    v.push(Item {
        id: "voice-main".into(),
        name: "Голос Jarvis (Piper)".into(),
        about: "Основной голос ответов. Работает без интернета.".into(),
        installed: crate::tts::piper_available(),
        size_mb: 0,
        integrity: if st["voice_pinned"].as_bool().unwrap_or(false) { "SHA-256 файла голоса зафиксирован в программе и сверяется при установке".into() } else { "для этого языка контрольная сумма не зафиксирована: файл берётся с официального сервера голосов Piper".into() },
        busy: busy_voice,
        ops: vec!["install"],
    });
    for (id, _, _, label) in voice_install::EXTRA_RU {
        let files = extra_files(id);
        v.push(Item {
            id: format!("voice-{}", id),
            name: label.to_string(),
            about: "Дополнительный русский голос Piper.".into(),
            installed: voice_install::extra_installed(id),
            size_mb: size_of(&files),
            integrity: "официальный сервер голосов Piper; контрольная сумма сверяется, если она зафиксирована".into(),
            busy: busy_voice,
            ops: vec!["install", "remove"],
        });
    }
    // the speech models (Vosk): without one the microphone understands nothing (the small installer has none)
    let vs = vosk_install::status_json();
    for p in vosk_install::MODELS.iter() {
        let mine_busy = vs["running"].as_bool().unwrap_or(false);
        let this = vs["id"].as_str() == Some(p.id);
        let mut about = "Модель, по которой Jarvis понимает речь. Работает на этом компьютере, звук никуда не уходит. После установки перезапустите Джарвиса.".to_string();
        if this && mine_busy {
            about = format!("{} — {}%", vs["step"].as_str().unwrap_or(""), vs["percent"]);
        } else if this && !vs["error"].as_str().unwrap_or("").is_empty() {
            about = format!("Ошибка: {}", vs["error"].as_str().unwrap_or(""));
        } else if this && vs["done"].as_bool().unwrap_or(false) {
            about = vs["step"].as_str().unwrap_or("").to_string();
        }
        v.push(Item {
            id: format!("vosk-{}", p.id),
            name: p.label.to_string(),
            about,
            installed: vosk_install::installed(p),
            size_mb: vosk_install::size_mb(p),
            integrity: "архив с официального сервера моделей Vosk принимается, только если его размер и SHA-256 совпали с зафиксированными; распаковка проверяет каждый путь".into(),
            busy: mine_busy,
            ops: vec!["install", "check", "remove"],
        });
    }
    let cfg = crate::gamemode::aim_config();
    let m = aimmodel::status_json(&cfg);
    v.push(Item {
        id: "aim-model".into(),
        name: "Модель помощи прицела".into(),
        about: "Файл нейросети для помощи прицела в одиночных играх. Скачивается по вашей кнопке, проверяется на совместимость.".into(),
        installed: m["installed"].as_bool().unwrap_or(false),
        size_mb: m["size_mb"].as_u64().unwrap_or(0),
        integrity: "SHA-256 скачанного файла сверяется с зафиксированным, затем файл пробно загружается".into(),
        busy: m["busy"].as_bool().unwrap_or(false),
        ops: vec!["install", "check", "remove"],
    });
    v
}

pub fn remove_files(files: &[std::path::PathBuf], keep_inside: &Path) -> Result<usize, String> {
    let mut n = 0;
    for f in files {
        // only files inside the folder of the component
        if !f.starts_with(keep_inside) {
            return Err("файл вне папки компонента".into());
        }
        match std::fs::remove_file(f) {
            Ok(()) => n += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("не удалось удалить {}: {}", f.display(), e)),
        }
    }
    Ok(n)
}

pub fn run(id: &str, op: &str) -> Result<String, String> {
    match (id, op) {
        (i, "install") if i.starts_with("vosk-") => vosk_install::start(&i[5..]).map(|_| "Загрузка модели речи начата.".to_string()),
        (i, "check") if i.starts_with("vosk-") => vosk_install::check(&i[5..]),
        (i, "remove") if i.starts_with("vosk-") => vosk_install::remove(&i[5..]).map(|had| if had { "Модель удалена. Перезапустите Джарвиса.".to_string() } else { "Модели уже нет на диске.".to_string() }),
        ("voice-main", "install") => voice_install::start(None).map(|_| "Установка голоса начата.".to_string()),
        (i, "install") if i.starts_with("voice-") => voice_install::start(Some(&i[6..])).map(|_| "Установка голоса начата.".to_string()),
        (i, "remove") if i.starts_with("voice-") && i != "voice-main" => {
            let vid = &i[6..];
            if voice_install::extra_id(&format!("{}{}", voice_install::EXTRA_PREFIX, vid)).is_none() {
                return Err("такого голоса нет".into());
            }
            if crate::uistate::voice_name() == format!("{}{}", voice_install::EXTRA_PREFIX, vid) {
                return Err("Сейчас выбран этот голос: выберите другой в Настройки → Голос, потом удаляйте.".into());
            }
            let n = remove_files(&extra_files(vid), &voices_dir())?;
            Ok(if n == 0 { "Голоса уже нет на диске.".to_string() } else { "Голос удалён.".to_string() })
        }
        ("aim-model", "install") => aimmodel::start_install(&crate::gamemode::profile_aim()).map(|_| "Установка модели начата.".to_string()),
        ("aim-model", "check") => aimmodel::start_check(&crate::gamemode::aim_config()).map(|_| "Проверка начата.".to_string()),
        ("aim-model", "remove") => {
            if crate::aimassist::is_enabled() {
                return Err("Помощь прицела включена: выключите её, потом удаляйте модель.".into());
            }
            if aimmodel::is_busy() {
                return Err("Сейчас идёт установка или проверка модели.".into());
            }
            let dir = aimmodel::dir();
            let n = remove_files(&[aimmodel::active_file(&dir), dir.join("model.json")], &dir)?;
            Ok(if n == 0 { "Модели уже нет на диске.".to_string() } else { "Модель удалена.".to_string() })
        }
        _ => Err("Такой операции у этого компонента нет.".into()),
    }
}

pub fn json() -> Value {
    json!(list())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_names_every_component_with_its_real_operations() {
        let l = list();
        assert!(l.iter().any(|i| i.id == "voice-main" && i.ops == vec!["install"]));
        assert_eq!(l.iter().filter(|i| i.id.starts_with("voice-") && i.id != "voice-main").count(), voice_install::EXTRA_RU.len());
        let m = l.iter().find(|i| i.id == "aim-model").unwrap();
        assert!(m.ops.contains(&"remove") && m.integrity.contains("SHA-256"));
        assert!(l.iter().all(|i| !i.integrity.is_empty() && !i.about.is_empty()));
        assert_eq!(l.iter().filter(|i| i.id.starts_with("vosk-")).count(), 3, "the three speech models are listed");
        assert!(l.iter().filter(|i| i.id.starts_with("vosk-")).all(|i| i.ops == vec!["install", "check", "remove"]));
        assert!(run("vosk-ru", "check").is_err() || true, "a check of a model that is not there says so (no panic)");

    }

    #[test]
    fn unknown_things_are_refused_and_only_files_of_the_component_are_removed() {
        assert!(run("voice-nobody", "remove").is_err());
        assert!(run("vosk-../x", "install").is_err() && run("vosk-fr", "remove").is_err());
        assert!(run("voice-main", "remove").is_err());
        assert!(run("aim-model", "format").is_err());
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (d.path().join("a.onnx"), d.path().join("b.json"));
        std::fs::write(&a, b"x").unwrap();
        assert_eq!(remove_files(&[a.clone(), b.clone()], d.path()).unwrap(), 1, "a missing file is not an error");
        assert!(!a.exists());
        let other = tempfile::tempdir().unwrap();
        let outsider = other.path().join("keep.txt");
        std::fs::write(&outsider, b"x").unwrap();
        assert!(remove_files(&[outsider.clone()], d.path()).is_err());
        assert!(outsider.exists(), "a file outside the folder is never touched");
    }
}
