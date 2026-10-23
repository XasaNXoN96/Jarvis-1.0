use std::{collections::HashMap, path::PathBuf, sync::Arc};
use serde::{Serialize, Deserialize};
use parking_lot::RwLock;

#[derive(Serialize, Deserialize, Debug)]
pub struct JCommandsList {
    #[serde(skip)]
    pub path: PathBuf,

    pub commands: Vec<JCommand>,
}



#[derive(Serialize, Deserialize, Debug)]
pub struct JCommand {
    pub id: String,

    // Available command types are: "lua", "ahk", "cli", "voice", "terminate", "stop_chaining"
    #[serde(rename = "type")]
    pub cmd_type: String,
    
    #[serde(default)]
    pub description: String,
    
    // for "ahk" type
    #[serde(default)]
    pub exe_path: String,
    #[serde(default)]
    pub exe_args: Vec<String>,
    
    // for "cli" type
    #[serde(default)]
    pub cli_cmd: String,
    #[serde(default)]
    pub cli_args: Vec<String>,
    
    // #[serde(default)]
    // pub sounds: Vec<String>,

    // for "lua" type
    #[serde(default)]
    pub script: String,

    // Lua sandbox level: "minimal", "standard", "full"
    // basically this is an access level
    #[serde(default)]
    pub sandbox: String,

    // Risk of the command: "low" | "medium" | "high". It is the most the command may do (see policy.rs): a command
    // that does not say is "low". Every command of the packs of Jarvis names its risk (a test checks it).
    #[serde(default)]
    pub risk: String,

    // Script timeout in milliseconds (default 10000 = 10s)
    #[serde(default)]
    pub timeout: u64,

    // Multi-language sounds
    #[serde(default)]
    pub sounds: HashMap<String, Vec<String>>,

    // Multi-language phrases
    #[serde(default)]
    pub phrases: HashMap<String, Vec<String>>,

    // Multi-language keyword triggers for commands with parameters
    // ("поставь таймер на 5 минут"). Each trigger is a space-separated list of word
    // stems; the command matches when every stem starts some word of the phrase.
    // Checked BEFORE intent recognition, the trigger with the most stems wins.
    #[serde(default)]
    pub triggers: HashMap<String, Vec<String>>,

    // Slot definitions: slot_name -> how to extract it
    #[serde(default)]
    pub slots: HashMap<String, SlotDefinition>,

    // CACHE
    #[serde(skip, default)]
    sounds_cache: RwLock<HashMap<String, Arc<Vec<String>>>>,
    
    #[serde(skip, default)]
    phrases_cache: RwLock<HashMap<String, Arc<Vec<String>>>>,
}

// custom Clone 
impl Clone for JCommand {
    fn clone(&self) -> Self {
        Self {
            id: self.id.clone(),

            cmd_type: self.cmd_type.clone(),
            description: self.description.clone(),

            exe_path: self.exe_path.clone(),
            exe_args: self.exe_args.clone(),

            cli_cmd: self.cli_cmd.clone(),
            cli_args: self.cli_args.clone(),

            script: self.script.clone(),
            sandbox: self.sandbox.clone(),
            risk: self.risk.clone(),
            timeout: self.timeout.clone(),

            sounds: self.sounds.clone(),
            phrases: self.phrases.clone(),
            triggers: self.triggers.clone(),

            slots: self.slots.clone(),

            // empty caches for cloned instance
            sounds_cache: RwLock::new(HashMap::new()),
            phrases_cache: RwLock::new(HashMap::new()),
        }
    }
}

impl JCommand {
    // get phrases for current language
    pub fn get_phrases(&self, lang: &str) -> Arc<Vec<String>> {
        if let Some(cached) = self.phrases_cache.read().get(lang) {
            return Arc::clone(cached);
        }
        
        let result = Arc::new(self.resolve_localized(&self.phrases, lang));
        self.phrases_cache.write().insert(lang.to_string(), Arc::clone(&result));
        
        result
    }

    // get keyword triggers for current language
    pub fn get_triggers(&self, lang: &str) -> Vec<String> {
        self.resolve_localized(&self.triggers, lang)
    }

    // the triggers of the language itself plus the Russian ones: a Russian phrase must work on an English Windows too
    // (the system language only picks the default, the user speaks Russian to Jarvis)
    pub fn get_triggers_with_ru(&self, lang: &str) -> Vec<String> {
        let mut all = self.get_triggers(lang);
        if lang != "ru" {
            if let Some(ru) = self.triggers.get("ru") {
                all.extend(ru.iter().cloned());
            }
        }
        // English commands work whatever the language of the program is (their words are Latin, they cannot meet a Russian phrase)
        if lang != "en" {
            if let Some(en) = self.triggers.get("en") {
                all.extend(en.iter().cloned());
            }
        }
        // Uzbek commands (Latin letters) work whatever the language of the program is: for typed text, the phone and Telegram
        // (the recognizer of speech has no Uzbek model, so they are not heard from the microphone)
        if lang != "uz" {
            if let Some(uz) = self.triggers.get("uz") {
                all.extend(uz.iter().cloned());
            }
        }
        all
    }

    // get all phrases (for backwards compat)
    pub fn get_all_phrases(&self) -> Vec<String> {
        self.phrases.values().flatten().cloned().collect()
    }

    // get sounds for current language
    pub fn get_sounds(&self, lang: &str) -> Arc<Vec<String>> {
        if let Some(cached) = self.sounds_cache.read().get(lang) {
            return Arc::clone(cached);
        }
        
        let result = Arc::new(self.resolve_localized(&self.sounds, lang));
        self.sounds_cache.write().insert(lang.to_string(), Arc::clone(&result));
        
        result
    }

    // get all sounds (for backwards compat)
    pub fn get_all_sounds(&self) -> Vec<String> {
        self.sounds.values().flatten().cloned().collect()
    }


    // shared fallback
    fn resolve_localized(&self, map: &HashMap<String, Vec<String>>, lang: &str) -> Vec<String> {
        // exact match
        if let Some(values) = map.get(lang) {
            return values.clone();
        }

        // fallback to "en"
        if lang != "en" {
            if let Some(values) = map.get("en") {
                return values.clone();
            }
        }

        // fallback to first available
        map.values().next().cloned().unwrap_or_default()
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SlotDefinition {
    // Entity label for GLiNER (e.g. "city name", "song title", "number")
    // This is a free-form description - GLiNER matches it semantically
    #[serde(default)]
    pub entity: String,

    // Optional: fallback context words for template-based extraction
    // e.g. ["in", "for", "at"] for a city slot
    #[serde(default)]
    pub context: Vec<String>,
}

// Extracted slot value passed to commands
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SlotValue {
    Text(String),
    Number(f64),
}