use once_cell::sync::OnceCell;
use vosk::{DecodingState, Recognizer};
use std::sync::Arc;
use parking_lot::Mutex;

use crate::{vosk_models, i18n, config, models};
use crate::models::vosk::VoskModel;
use crate::DB;

// the model Arc keeps the vosk::Model alive for the recognizers
static VOSK_MODEL: OnceCell<Arc<VoskModel>> = OnceCell::new();
static WAKE_RECOGNIZER: OnceCell<Mutex<Recognizer>> = OnceCell::new();
static SPEECH_RECOGNIZER: OnceCell<Mutex<Recognizer>> = OnceCell::new();

pub fn init_vosk() -> Result<(), String> {
    if VOSK_MODEL.get().is_some() {
        return Ok(());
    }

    // (Vosk opens its files by ANSI paths: a model in a folder with non-Latin letters is reached through an ASCII alias)
    let model_path = crate::ascii_path::ascii_dir(get_configured_model_path()?);
    let model_id = format!("vosk:{}", model_path.display());

    // load through registry (shared if anything else needs the same model)
    let vosk = models::vosk::load(
        models::registry(),
        &model_id,
        model_path.to_str().unwrap(),
    )?;

    // language-specific wake grammar
    let lang = i18n::get_language();
    let wake_grammar = config::wake_grammar_now(&lang);
    info!("Wake grammar for '{}': {:?}", lang, wake_grammar);

    let mut wake_recognizer = Recognizer::new_with_grammar(&vosk.model, 16000.0, &wake_grammar)
        .ok_or("Failed to create wake word recognizer")?;

    wake_recognizer.set_max_alternatives(1);

    let mut speech_recognizer = Recognizer::new(&vosk.model, 16000.0)
        .ok_or("Failed to create speech recognizer")?;

    speech_recognizer.set_max_alternatives(config::VOSK_SPEECH_RECOGNIZER_MAX_ALTERNATIVES);
    speech_recognizer.set_words(config::VOSK_SPEECH_RECOGNIZER_WORDS);
    speech_recognizer.set_partial_words(config::VOSK_SPEECH_PARTIAL_WORDS);

    VOSK_MODEL.set(vosk).map_err(|_| "Model already set")?;
    WAKE_RECOGNIZER.set(Mutex::new(wake_recognizer)).map_err(|_| "Wake recognizer already set")?;
    SPEECH_RECOGNIZER.set(Mutex::new(speech_recognizer)).map_err(|_| "Speech recognizer already set")?;

    Ok(())
}


pub fn recognize_wake_word(data: &[i16]) -> Option<(String, f32)> {
    let mut recognizer = WAKE_RECOGNIZER.get()?.lock();
    
    match recognizer.accept_waveform(data) {
        Ok(DecodingState::Running) => {
            None
        }
        Ok(DecodingState::Finalized) => {
            let result = recognizer.result();
            
            if let Some(alternatives) = result.multiple() {
                if let Some(best) = alternatives.alternatives.first() {
                    if !best.text.is_empty() {
                        return Some((best.text.to_string(), best.confidence));
                    }
                }
            }
            
            None
        }
        _ => None,
    }
}


pub fn recognize_speech(data: &[i16]) -> Option<String> {
    let mut recognizer = SPEECH_RECOGNIZER.get()?.lock();
    
    match recognizer.accept_waveform(data) {
        Ok(DecodingState::Finalized) => {
            recognizer.result()
                .multiple()
                .and_then(|m| m.alternatives.first().map(|a| a.text.to_string()))
        }
        _ => None,
    }
}


// a recording that is not the microphone (a voice message of Telegram): a recognizer of its own on the same model, so
// the live recognition of the microphone is never disturbed. 16 kHz mono.
pub fn transcribe(data: &[i16]) -> Result<String, String> {
    // the sphere shows «распознаю» while the phrase is being recognized
    let _busy = crate::activity::enter(crate::activity::Kind::Recognizing);
    init_vosk()?;
    let model = VOSK_MODEL.get().ok_or("модель распознавания не загружена")?;
    let mut r = Recognizer::new(&model.model, 16000.0).ok_or("не удалось запустить распознавание")?;
    for chunk in data.chunks(8000) {
        r.accept_waveform(chunk).map_err(|_| "распознавание не приняло звук".to_string())?;
    }
    let text = match r.final_result() {
        vosk::CompleteResult::Single(s) => s.text.to_string(),
        vosk::CompleteResult::Multiple(m) => m.alternatives.first().map(|a| a.text.to_string()).unwrap_or_default(),
    };
    Ok(text)
}

pub fn reset_speech_recognizer() {
    if let Some(recognizer) = SPEECH_RECOGNIZER.get() {
        recognizer.lock().reset();
    }
}

// the user chose another name for the assistant: the wake-word recognizer is built again with it
pub fn rebuild_wake_grammar() {
    let (Some(model), Some(slot)) = (VOSK_MODEL.get(), WAKE_RECOGNIZER.get()) else { return };
    let grammar = config::wake_grammar_now(&i18n::get_language());
    match Recognizer::new_with_grammar(&model.model, 16000.0, &grammar) {
        Some(mut r) => {
            r.set_max_alternatives(1);
            *slot.lock() = r;
            info!("Wake grammar changed: {:?}", grammar);
        }
        None => warn!("Wake grammar {:?} was not accepted: the old one stays", grammar),
    }
}

// does the speech model know this word? Some(false): by voice the name is not heard (by text it still works);
// None: it could not be checked (no model list)
pub fn wake_word_known(word: &str) -> Option<bool> {
    let dir = get_configured_model_path().ok()?;
    let word = word.to_lowercase();
    for rel in ["graph/words.txt", "words.txt"] {
        if let Ok(text) = std::fs::read_to_string(dir.join(rel)) {
            return Some(text.lines().any(|l| l.split_whitespace().next() == Some(word.as_str())));
        }
    }
    None
}

pub fn reset_wake_recognizer() {
    if let Some(recognizer) = WAKE_RECOGNIZER.get() {
        recognizer.lock().reset();
    }
}

fn get_configured_model_path() -> Result<std::path::PathBuf, String> {
    // try to get from settings
    if let Some(db) = DB.get() {
        let settings = db.read();
        if !settings.vosk_model.is_empty() {
            if let Some(path) = vosk_models::get_model_path(&settings.vosk_model) {
                return Ok(path);
            }
            warn!("Configured Vosk model '{}' not found, falling back to auto-detect", settings.vosk_model);
        }
    }
    
    // auto-detect: prefer model matching current language
    let available = vosk_models::scan_vosk_models();
    let language = i18n::get_language();

    let lang_code = match language.as_str() {
        "ru" => "ru",
        "en" => "us",
        "ua" => "uk",
        other => other,
    };

    if let Some(matched) = available.iter().find(|m| m.language == lang_code) {
        info!("Auto-detected Vosk model for '{}': {}", language, matched.name);
        return Ok(matched.path.clone());
    }

    if let Some(first) = available.first() {
        info!("Auto-detected Vosk model (no language match): {}", first.name);
        return Ok(first.path.clone());
    }
    
    // fallback to legacy path
    let legacy_path = crate::APP_DIR.join(config::VOSK_MODEL_PATH);
    if legacy_path.exists() {
        return Ok(legacy_path);
    }
    
    Err("No Vosk models found".into())
}
