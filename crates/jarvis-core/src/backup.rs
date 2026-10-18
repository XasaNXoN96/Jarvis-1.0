//! A backup of the settings in one file (and back): the interface settings, the aim assist numbers (with the numbers of every game), the
//! list of games, the macros and the game profile. Secrets are NEVER in the file: every field whose name says key, token, secret, password or
//! hash is left out, and on loading such fields of the present settings stay as they are. Only the files below are read or written, and
//! only if they hold a JSON object.

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

// (the file, where it lives: true = the folder of the settings of the program, false = the data folder)
pub const FILES: [(&str, bool); 5] = [("ui_settings.json", true), ("aim_settings.json", false), ("game_list.json", false), ("game_macros.json", false), ("game_profile.json", false)];
const MAX_BYTES: usize = 2_000_000;

fn secret_name(k: &str) -> bool {
    // the words of the name: «ai_key», «remote_pin», «apiToken» (a name like «hotkey» is not a secret)
    let spaced: String = k.chars().enumerate().map(|(i, c)| if c.is_uppercase() && i > 0 { format!("_{}", c.to_lowercase()) } else { c.to_lowercase().to_string() }).collect();
    spaced.split(|c: char| !c.is_alphanumeric()).any(|w| ["key", "keys", "token", "secret", "pass", "password", "hash", "pin", "pwd"].contains(&w))
}

// a copy of the object without the secret fields (top level and one level down)
pub fn strip_secrets(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().filter(|(k, _)| !secret_name(k)).map(|(k, x)| (k.clone(), strip_secrets(x))).collect()),
        other => other.clone(),
    }
}

pub fn export_from(dir_of: &dyn Fn(bool) -> Option<PathBuf>) -> Value {
    let mut files = Map::new();
    for (name, cfg) in FILES {
        let Some(dir) = dir_of(cfg) else { continue };
        let Some(v) = std::fs::read_to_string(dir.join(name)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(|v| v.is_object()) else { continue };
        files.insert(name.to_string(), strip_secrets(&v));
    }
    json!({ "jarvis_backup": 1, "files": files })
}

// puts the files of a backup in place; the secret fields of the present file stay; the number of files written
pub fn import_into(backup: &Value, dir_of: &dyn Fn(bool) -> Option<PathBuf>) -> Result<usize, String> {
    if backup.get("jarvis_backup").and_then(|v| v.as_i64()) != Some(1) {
        return Err("это не файл резервной копии Jarvis".into());
    }
    let files = backup.get("files").and_then(|f| f.as_object()).ok_or("в файле нет настроек")?;
    // everything is checked before the first write
    let mut plan: Vec<(PathBuf, Value)> = Vec::new();
    for (name, v) in files {
        let Some((_, cfg)) = FILES.iter().find(|(n, _)| n == name) else { return Err(format!("в файле лишний пункт «{}»", name)) };
        if !v.is_object() || v.to_string().len() > MAX_BYTES {
            return Err(format!("пункт «{}» повреждён или слишком большой", name));
        }
        let dir = dir_of(*cfg).ok_or("папка настроек недоступна")?;
        let path = dir.join(name);
        // the present secrets are kept: they are not in the backup
        let mut merged = strip_secrets(v);
        if let Some(old) = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).and_then(|o| o.as_object().cloned()) {
            if let Some(m) = merged.as_object_mut() {
                for (k, x) in old {
                    if secret_name(&k) {
                        m.insert(k, x);
                    }
                }
            }
        }
        plan.push((path, merged));
    }
    for (path, v) in &plan {
        crate::write_atomic(path, serde_json::to_string_pretty(v).unwrap_or_default()).map_err(|e| format!("не удалось записать {}: {}", path.display(), e))?;
    }
    Ok(plan.len())
}

pub fn export() -> Value {
    export_from(&real_dir)
}

pub fn import(backup: &Value) -> Result<usize, String> {
    import_into(backup, &real_dir)
}

fn real_dir(cfg: bool) -> Option<PathBuf> {
    if cfg {
        crate::APP_CONFIG_DIR.get().cloned()
    } else {
        Some(crate::APP_DIR.clone())
    }
}

#[allow(dead_code)]
fn _unused(_: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_backup_has_no_secrets_and_a_restore_keeps_the_present_ones() {
        let dir = std::env::temp_dir().join(format!("jarvis-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let d = dir.clone();
        let dir_of = move |_: bool| Some(d.clone());
        std::fs::write(dir.join("ui_settings.json"), r#"{"hotkey":"Ctrl+Alt+J","ai_key":"SECRET","remote":{"pin":"1234","port":8080}}"#).unwrap();
        std::fs::write(dir.join("aim_settings.json"), r#"{"side":256,"games":{"x.exe":{"cx":1}}}"#).unwrap();
        let b = export_from(&dir_of);
        let text = b.to_string();
        assert!(!text.contains("SECRET") && !text.contains("1234"), "{}", text);
        assert!(text.contains("Ctrl+Alt+J") && text.contains("x.exe") && text.contains("8080"));
        // another computer: other secrets, other numbers
        std::fs::write(dir.join("ui_settings.json"), r#"{"hotkey":"","ai_key":"MINE"}"#).unwrap();
        std::fs::write(dir.join("aim_settings.json"), r#"{"side":512}"#).unwrap();
        assert_eq!(import_into(&b, &dir_of), Ok(2));
        let ui: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("ui_settings.json")).unwrap()).unwrap();
        assert_eq!((ui["hotkey"].as_str(), ui["ai_key"].as_str()), (Some("Ctrl+Alt+J"), Some("MINE")));
        let aim: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("aim_settings.json")).unwrap()).unwrap();
        assert_eq!(aim["side"], 256);
        // rubbish is refused before anything is written
        assert!(import_into(&json!({"x": 1}), &dir_of).is_err());
        assert!(import_into(&json!({"jarvis_backup": 1, "files": {"evil.json": {}}}), &dir_of).is_err());
        assert!(import_into(&json!({"jarvis_backup": 1, "files": {"../ui_settings.json": {}}}), &dir_of).is_err());
        assert!(import_into(&json!({"jarvis_backup": 1, "files": {"ui_settings.json": 5}}), &dir_of).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
