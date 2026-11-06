use std::fs;
use std::path::{Path, PathBuf};
use rand::prelude::*;
use once_cell::sync::OnceCell;
use parking_lot::RwLock;
// use chrono::Timelike;

use crate::{DB, SOUND_DIR, audio, config, time};

mod structs;
pub use structs::*;

static VOICES: OnceCell<Vec<structs::VoiceConfig>> = OnceCell::new();
static CURRENT_VOICE_ID: OnceCell<RwLock<String>> = OnceCell::new();

pub fn init(default_voice: &str, language: &str) -> Result<(), String> {
    let voices = scan_voices()?;
    
    if voices.is_empty() {
        return Err("No voices found".into());
    }
    
    info!("Loaded {} voice(s): {:?}", 
        voices.len(), 
        voices.iter().map(|v| &v.voice.id).collect::<Vec<_>>()
    );

    // resolve which voice to use
    let voice_id = if !default_voice.is_empty() && voices.iter().any(|v| v.voice.id == default_voice) {
        default_voice.to_string()
    } else {
        // auto-detect: pick the first voice that supports the active language
        let auto = voices.iter()
            .find(|v| v.voice.languages.contains(&language.to_string()))
            .or_else(|| voices.first());

        match auto {
            Some(v) => {
                if default_voice.is_empty() {
                    info!("No voice configured, auto-selected '{}' for language '{}'", v.voice.id, language);
                } else {
                    warn!("Voice '{}' not found, auto-selected '{}'", default_voice, v.voice.id);
                }
                v.voice.id.clone()
            }
            None => return Err("No compatible voice found".into()),
        }
    };

    CURRENT_VOICE_ID.get_or_init(|| RwLock::new(voice_id));
    VOICES.set(voices).map_err(|_| "Voices already initialized")?;
    
    Ok(())
}

pub fn scan_voices() -> Result<Vec<structs::VoiceConfig>, String> {
    let voices_dir = SOUND_DIR.join(&config::VOICES_PATH);
    
    if !voices_dir.exists() {
        return Err(format!("Voices directory not found: {:?}", voices_dir));
    }
    
    let mut voices = Vec::new();
    
    let entries = fs::read_dir(&voices_dir)
        .map_err(|e| format!("Failed to read voices directory: {}", e))?;
    
    for entry in entries.flatten() {
        let voice_path = entry.path();
        if !voice_path.is_dir() {
            continue;
        }
        
        let toml_path = voice_path.join("voice.toml");
        if !toml_path.exists() {
            warn!("Voice folder {:?} missing voice.toml, skipping", voice_path);
            continue;
        }
        
        match load_voice_config(&toml_path, &voice_path) {
            Ok(config) => voices.push(config),
            Err(e) => warn!("Failed to load voice {:?}: {}", voice_path, e),
        }
    }
    
    Ok(voices)
}

fn load_voice_config(toml_path: &Path, voice_path: &Path) -> Result<structs::VoiceConfig, String> {
    let content = fs::read_to_string(toml_path)
        .map_err(|e| format!("Failed to read voice.toml: {}", e))?;
    
    let mut config: structs::VoiceConfig = toml::from_str(&content)
        .map_err(|e| format!("Failed to parse voice.toml: {}", e))?;
    
    config.path = voice_path.to_path_buf();
    
    Ok(config)
}



pub fn list_voices() -> &'static [structs::VoiceConfig] {
    VOICES.get().map(|v| v.as_slice()).unwrap_or(&[])
}

pub fn get_voice(voice_id: &str) -> Option<&'static structs::VoiceConfig> {
    VOICES.get()?.iter().find(|v| v.voice.id == voice_id)
}

pub fn get_current_voice() -> Option<&'static structs::VoiceConfig> {
    let current_id = CURRENT_VOICE_ID.get()?.read().clone();
    get_voice(&current_id)
}

pub fn set_current_voice(voice_id: &str) {
    if let Some(lock) = CURRENT_VOICE_ID.get() {
        *lock.write() = voice_id.to_string();
    }
}

fn get_current_language() -> String {
    DB.get()
        .map(|db| db.read().language.clone())
        .unwrap_or_else(|| "ru".to_string())
}



fn find_sound_file(voice_path: &Path, lang: &str, sound_name: &str) -> Option<PathBuf> {
    const EXTENSIONS: &[&str] = &["mp3", "wav", "ogg"];
    let lang_path = voice_path.join(lang);
    
    // try language subfolder first (/en, /ua, /ru, etc)
    for ext in EXTENSIONS {
        let file_path = lang_path.join(format!("{}.{}", sound_name, ext));
        if file_path.exists() {
            return Some(file_path);
        }
    }
    
    // fallback to root voice folder
    for ext in EXTENSIONS {
        let file_path = voice_path.join(format!("{}.{}", sound_name, ext));
        if file_path.exists() {
            return Some(file_path);
        }
    }
    
    None
}

fn play_random_from_list(voice_path: &Path, lang: &str, sounds: &[String]) {
    if sounds.is_empty() {
        return;
    }
    
    // a pack may lack some of the names of a list (the jokes of a command are recorded for some packs only): only the ones that exist are drawn
    let existing: Vec<&String> = sounds.iter().filter(|n| find_sound_file(voice_path, lang, n).is_some()).collect();
    let sound_name = match existing.choose(&mut rand::thread_rng()) {
        Some(n) => *n,
        None => sounds.choose(&mut rand::thread_rng()).unwrap(),
    };
    
    match find_sound_file(voice_path, lang, sound_name) {
        Some(path) => {
            debug!("Playing: {:?}", path);
            audio::play_sound(&path);
        }
        None => {
            warn!("Sound not found: {} (lang: {})", sound_name, lang);
        }
    }
}

pub fn play(reaction: structs::Reaction) {
    let voice = match get_current_voice() {
        Some(v) => v,
        None => {
            warn!("No current voice set");
            return;
        }
    };
    
    let lang = get_current_language();
    
    let reactions = match voice.reactions.get(&lang) {
        Some(r) => r,
        None => {
            warn!("No reactions for language: {}", lang);
            return;
        }
    };

    let sounds = match reaction {
        structs::Reaction::Greet => {
            // try time-specific first
            let time_specific = match time::TimeOfDay::now() {
                time::TimeOfDay::Morning => &reactions.greet_morning,
                time::TimeOfDay::Day => &reactions.greet_day,
                time::TimeOfDay::Evening => &reactions.greet_evening,
                time::TimeOfDay::Night => &reactions.greet_night,
            };

            if time_specific.is_empty() {
                &reactions.greet
            } else {
                time_specific
            }
        }
        structs::Reaction::Reply => &reactions.reply,
        structs::Reaction::Ok => &reactions.ok,
        structs::Reaction::NotFound => &reactions.not_found,
        structs::Reaction::Thanks => &reactions.thanks,
        structs::Reaction::Error => &reactions.error,
        structs::Reaction::Goodbye => &reactions.goodbye,
    };
    
    play_random_from_list(&voice.path, &lang, sounds);
}

/// The recorded phrase of the current pack for this command (see cmdclips). True = a clip was played; false = the pack has none for it
/// (the caller then plays the general reaction).
pub fn play_command_clip(command_id: &str, said: &str) -> bool {
    let clips = match crate::cmdclips::clips_for(command_id, said) {
        Some(c) => c,
        None => return false,
    };
    let voice = match get_current_voice() {
        Some(v) => v,
        None => return false,
    };
    let lang = get_current_language();
    let found: Vec<PathBuf> = clips.iter().filter_map(|n| find_sound_file(&voice.path, &lang, n)).collect();
    match found.choose(&mut rand::thread_rng()) {
        Some(path) => {
            debug!("Playing the clip of the command {}: {:?}", command_id, path);
            audio::play_sound(path);
            true
        }
        None => false,
    }
}

/// One clip of a pack for the list on the page: its name, what the pack plays it on (a reaction), and the commands that use it.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct ClipInfo {
    pub name: String,
    pub reaction: String,
    pub commands: Vec<String>,
}

fn reaction_label(r: &structs::VoiceReactions, name: &str) -> String {
    let has = |l: &Vec<String>| l.iter().any(|n| n == name);
    let table: [(&Vec<String>, &str); 10] = [
        (&r.greet, "запуск программы"), (&r.greet_morning, "запуск утром"), (&r.greet_day, "запуск днём"), (&r.greet_evening, "запуск вечером"), (&r.greet_night, "запуск ночью"),
        (&r.reply, "слово-активатор («Слушаю»)"), (&r.ok, "общее «выполнено»"), (&r.thanks, "«спасибо»"), (&r.goodbye, "прощание"), (&r.not_found, "«не понял»"),
    ];
    let mut out: Vec<&str> = table.iter().filter(|(l, _)| has(l)).map(|(_, t)| *t).collect();
    if has(&r.error) {
        out.push("ошибка");
    }
    out.join(", ")
}

/// The clips of a pack in the language of the program (the names are the files of the folder; nothing else is ever played from the page).
pub fn clips_of(voice_id: &str) -> Vec<ClipInfo> {
    let voice = match get_voice(voice_id) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let lang = get_current_language();
    let dir = voice.path.join(&lang);
    let react = voice.reactions.get(&lang);
    let mut names: Vec<String> = fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let p = e.path();
                    let ext = p.extension()?.to_string_lossy().to_lowercase();
                    (["mp3", "wav", "ogg"].contains(&ext.as_str())).then(|| p.file_stem().map(|s| s.to_string_lossy().to_string())).flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .map(|name| ClipInfo {
            reaction: react.map(|r| reaction_label(r, &name)).unwrap_or_default(),
            commands: crate::cmdclips::users_of(&name),
            name,
        })
        .collect()
}

/// Plays one clip of a pack for the list on the page. Only a name from the list of that pack is played (no path comes from the page).
pub fn play_clip_of(voice_id: &str, name: &str) -> bool {
    if !clips_of(voice_id).iter().any(|c| c.name == name) {
        return false;
    }
    let voice = match get_voice(voice_id) {
        Some(v) => v,
        None => return false,
    };
    let path = match find_sound_file(&voice.path, &get_current_language(), name) {
        Some(p) => p,
        None => return false,
    };
    // the sound is played to its end: not in the thread of the request
    std::thread::spawn(move || audio::play_sound(&path));
    true
}

/// A clip of the current pack by its name («timer_done»). True = it was played (the call returns when the sound ends).
pub fn play_clip(name: &str) -> bool {
    let voice = match get_current_voice() {
        Some(v) => v,
        None => return false,
    };
    match find_sound_file(&voice.path, &get_current_language(), name) {
        Some(path) => {
            debug!("Playing the clip {}: {:?}", name, path);
            audio::play_sound(&path);
            true
        }
        None => false,
    }
}

pub fn play_random_from(sounds: &[String]) {
    let voice = match get_current_voice() {
        Some(v) => v,
        None => {
            warn!("No current voice set");
            return;
        }
    };
    
    play_random_from_list(&voice.path, &get_current_language(), sounds);
}

// Play a preview sound for a specific voice
pub fn play_preview(voice_id: &str) {
    let voice = match get_voice(voice_id) {
        Some(v) => v,
        None => {
            warn!("Voice not found for preview: {}", voice_id);
            return;
        }
    };
    
    let lang = get_current_language();
    
    let reactions = match voice.reactions.get(&lang) {
        Some(r) => r,
        None => {
            warn!("No reactions for language {} in voice {}", lang, voice_id);
            return;
        }
    };
    
    // pick from reply, ok, or greet sounds for preview
    let sounds: Vec<&String> = reactions.reply.iter()
        .chain(reactions.ok.iter())
        .chain(reactions.greet.iter())
        .collect();
    
    if sounds.is_empty() {
        warn!("No preview sounds for voice: {}", voice_id);
        return;
    }
    
    let sound_name = sounds.choose(&mut rand::thread_rng()).unwrap();
    
    if let Some(path) = find_sound_file(&voice.path, &lang, sound_name) {
        debug!("Playing preview: {:?}", path);
        audio::play_sound(&path);
    }
}


// shortcuts
pub fn play_greet() { play(structs::Reaction::Greet); } // app startup
pub fn play_reply() { play(structs::Reaction::Reply); } // wake word detected
pub fn play_ok() { play(structs::Reaction::Ok); } // command executed
pub fn play_not_found() { play(structs::Reaction::NotFound); }
pub fn play_thanks() { play(structs::Reaction::Thanks); }
pub fn play_error() { play(structs::Reaction::Error); }
pub fn play_goodbye() { play(structs::Reaction::Goodbye); }

#[cfg(test)]
mod clip_list_tests {
    use super::*;

    #[test]
    fn a_clip_says_what_it_is_played_on() {
        let r = structs::VoiceReactions { reply: vec!["reply1".into(), "reply2".into()], ok: vec!["ok1".into()], goodbye: vec!["bye".into()], ..Default::default() };
        assert_eq!(reaction_label(&r, "reply2"), "слово-активатор («Слушаю»)");
        assert_eq!(reaction_label(&r, "ok1"), "общее «выполнено»");
        assert_eq!(reaction_label(&r, "bye"), "прощание");
        // a clip that no reaction names (a clip of a command, a joke) has no reaction
        assert_eq!(reaction_label(&r, "cmd_open_youtube"), "");
    }

    #[test]
    fn an_unknown_pack_or_clip_plays_nothing() {
        assert!(clips_of("no-such-pack").is_empty());
        assert!(!play_clip_of("no-such-pack", "ok1"));
        // a path is never a clip
        assert!(!play_clip_of("jarvis-og", "../../secret"));
    }
}
