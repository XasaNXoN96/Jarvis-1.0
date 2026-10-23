use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use std::time::Duration;
use std::process::{Child, Command};

use seqdiff::ratio;

mod structs;
pub use structs::*;

use crate::{config, i18n, APP_DIR};

#[cfg(feature = "lua")]
use crate::lua::{self, SandboxLevel, CommandContext};

pub fn parse_commands() -> Result<Vec<JCommandsList>, String> {
    let mut commands: Vec<JCommandsList> = Vec::new();

    let commands_path = APP_DIR.join(config::COMMANDS_PATH);
    let cmd_dirs = fs::read_dir(&commands_path)
        .map_err(|e| format!("Error reading commands directory {:?}: {}", commands_path, e))?;

    for entry in cmd_dirs.flatten() {
        let cmd_path = entry.path();
        let toml_file = cmd_path.join("command.toml");
        
        if !toml_file.exists() {
            continue;
        }
        
        let content = match fs::read_to_string(&toml_file) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {}: {}", toml_file.display(), e);
                continue;
            }
        };

        let file: JCommandsList = match toml::from_str(&content) {
            Ok(f) => f,
            Err(e) => {
                warn!("Failed to parse {}: {}", toml_file.display(), e);
                continue;
            }
        };

        commands.push(JCommandsList {
            path: cmd_path,
            commands: file.commands,
        });
    }

    if commands.is_empty() {
        Err("No commands found".into())
    } else {
        info!("Loaded {} command pack(s)", commands.len());
        Ok(commands)
    }
}


pub fn commands_hash(commands: &[JCommandsList]) -> String {
    use sha2::{Sha256, Digest};
    
    let mut hasher = Sha256::new();
    
    let lang = i18n::get_language();
    hasher.update(lang.as_bytes());
    hasher.update(b"|");

    // collect all command ids and phrases for current language, sorted
    let mut all_data: Vec<(&str, _)> = commands.iter()
        .flat_map(|ac| ac.commands.iter().map(|c| (c.id.as_str(), c.get_phrases(&lang))))
        .collect();
    all_data.sort_by_key(|(id, _)| *id);
    
    for (id, phrases) in all_data {
        hasher.update(id.as_bytes());
        for phrase in phrases.iter() {
            hasher.update(phrase.as_bytes());
        }
    }
    
    format!("{:x}", hasher.finalize())
}


// Look up a command by keyword triggers (see JCommand::triggers).
// Returns the command whose matching trigger has the most word stems.
// "открой ворт", "запусти тотал командер": the program command for a phrase that is a verb plus a program name
pub fn fetch_program<'a>(phrase: &str, commands: &'a [JCommandsList]) -> Option<(&'a PathBuf, &'a JCommand)> {
    let low = phrase.to_lowercase();
    let words: Vec<&str> = low.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if not_about_a_program(&words) {
        return None;
    }
    let req = crate::apps::parse_request(phrase)?;
    // "открой проводник", "этот компьютер" keep their own command (places.lua); in English "open explorer" starts it
    if req.app.id == "explorer" && req.action == crate::apps::Action::Open && !phrase.to_lowercase().contains("explorer") {
        return None;
    }
    let id = match req.action {
        crate::apps::Action::Open => "assistant_program_open",
        // "закрой ворд" closes at once (the program may ask about unsaved work itself); only "принудительно" is the
        // dangerous one and asks "да / нет" first
        crate::apps::Action::Close => {
            let low = phrase.to_lowercase();
            if low.contains("принудит") || low.contains("force") { "assistant_program_force_close" } else { "assistant_program_close" }
        }
    };
    commands.iter().flat_map(|l| l.commands.iter().map(move |c| (&l.path, c))).find(|(_, c)| c.id == id)
}

// "пиши привет, как дела?": the text after the writing verb is the text to write, never a command.
// Without this "как дела" (two stems) would beat "пиши" (one stem) and Jarvis would answer instead of typing.
pub fn fetch_dictation<'a>(phrase: &str, commands: &'a [JCommandsList]) -> Option<(&'a PathBuf, &'a JCommand)> {
    let first = crate::apps::tokens(phrase).into_iter().next()?;
    let verb = ["пиши", "пишите", "диктуй", "диктуйте", "печатай", "печатайте"].contains(&first.as_str());
    if !verb {
        return None;
    }
    commands.iter().flat_map(|l| l.commands.iter().map(move |c| (&l.path, c))).find(|(_, c)| c.id == "assistant_office_dictate")
}

// "открой YouTube", "open github": a known website by name (sites.rs). Checked before the keyword triggers and the
// intent model, so a site name is never taken for another command ("youtube" used to become "открой гугл").
pub fn fetch_site<'a>(phrase: &str, commands: &'a [JCommandsList]) -> Option<(&'a PathBuf, &'a JCommand)> {
    crate::sites::direct_open(phrase)?;
    commands.iter().flat_map(|l| l.commands.iter().map(move |c| (&l.path, c))).find(|(_, c)| c.id == "assistant_website")
}

// "открой Photoshop", "запусти командер": a program that is not in the fixed list but is installed (appindex.rs).
// Only when no keyword command claims the phrase ("открой загрузки" stays the folder) and only for a short
// "verb + name" phrase; an ambiguous name also comes here, the program command then asks which one.
pub fn fetch_indexed_program<'a>(phrase: &str, commands: &'a [JCommandsList]) -> Option<(&'a PathBuf, &'a JCommand)> {
    let (action, words) = crate::apps::split_request(phrase)?;
    if words.len() > 4 {
        return None;
    }
    let id = if action == crate::apps::Action::Open { "assistant_program_open" } else { "assistant_program_close" };
    if let Some((_, c)) = fetch_by_triggers(phrase, commands) {
        if c.id != id {
            return None;
        }
    }
    // the first scan of a new computer still runs: the program command waits for it and answers itself
    if !crate::appindex::index_pending() && crate::appindex::find_now(&words) == crate::appindex::Found::Nothing {
        return None;
    }
    // "закрой" only for an exact name: "закрой окно браузера" or "закрой документ" belong to other commands
    if action != crate::apps::Action::Open && !crate::appindex::has_exact(&words) {
        return None;
    }
    // "закрой" an installed program: the program command answers that only the main list can be closed
    commands.iter().flat_map(|l| l.commands.iter().map(move |c| (&l.path, c))).find(|(_, c)| c.id == id)
}

const PARTS: &[&str] = &["звук", "громк", "экран", "монитор", "дисплей", "микрофон", "блютуз", "bluetooth", "вайфай", "wifi", "интернет", "подсветк", "свет", "музык", "камер", "sound", "volume", "screen", "monitor", "display", "mic", "camera", "music"];

// "открой вкладку в хроме", "открой панель задач": about a tab or the taskbar, not about starting or closing the
// program whose name is in the phrase
fn not_about_a_program(words: &[&str]) -> bool {
    let has = |p: &str| words.iter().any(|w| w.starts_with(p));
    has("вкладк") || has("tab") || (has("панел") && has("задач")) || has("taskbar")
}

fn is_program_command(id: &str) -> bool {
    matches!(id, "assistant_program_open" | "assistant_program_close" | "assistant_program_force_close")
}

fn is_power_command(id: &str) -> bool {
    matches!(id, "assistant_power_off" | "assistant_power_restart" | "assistant_power_hibernate" | "assistant_power_logoff")
}

pub fn fetch_by_triggers<'a>(
    phrase: &str,
    commands: &'a [JCommandsList],
) -> Option<(&'a PathBuf, &'a JCommand)> {
    let lang = i18n::get_language();
    let normalized = phrase.to_lowercase().replace('ё', "е");
    let words: Vec<&str> = normalized
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    // best = (stems count, total characters of the stems, command). Ties on the stems count are
    // decided by the longer (more specific) trigger, then by the smaller id so the result is stable.
    let mut best: Option<(usize, usize, &PathBuf, &JCommand)> = None;
    // "выключи звук на компьютере", "выключи экран ноутбука": a part of the computer is named, so the phrase is about
    // that part, never about switching the whole computer off or restarting it
    let names_a_part = words.iter().any(|w| PARTS.iter().any(|p| w.starts_with(p))) || normalized.contains("wi-fi");
    let not_program = not_about_a_program(&words);

    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            if names_a_part && is_power_command(&cmd.id) {
                continue;
            }
            if not_program && is_program_command(&cmd.id) {
                continue;
            }
            for trigger in cmd.get_triggers_with_ru(&lang) {
                let stems: Vec<String> = trigger
                    .to_lowercase()
                    .replace('ё', "е")
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
                if stems.is_empty() {
                    continue;
                }
                if stems.iter().all(|stem| words.iter().any(|w| w.starts_with(stem.as_str()))) {
                    let count = stems.len();
                    let chars: usize = stems.iter().map(|s| s.chars().count()).sum();
                    let better = match &best {
                        None => true,
                        Some((bc, bch, _, bcmd)) => {
                            count > *bc
                                || (count == *bc && chars > *bch)
                                || (count == *bc && chars == *bch && cmd.id < bcmd.id)
                        }
                    };
                    if better {
                        best = Some((count, chars, &cmd_list.path, cmd));
                    }
                }
            }
        }
    }

    best.map(|(_, _, path, cmd)| (path, cmd))
}

// the phrase in the other keyboard layout, when only that one is a phrase of a command («ghbdtn» ↔ «привет», «фдуч» ↔ «alex»).
// Exact words of a command only: nothing is guessed, and a phrase that already matches is left alone.
pub fn fix_layout(phrase: &str, commands: &[JCommandsList]) -> Option<String> {
    let t = phrase.trim();
    if t.chars().count() < 3 || t.chars().count() > 80 {
        return None;
    }
    let other = if crate::textnorm::has_cyrillic(t) { crate::textnorm::ru_to_en(t) } else { crate::textnorm::en_to_ru(t) };
    if other == t.to_lowercase() || (!crate::textnorm::has_cyrillic(&other) && !crate::textnorm::has_latin(&other)) {
        return None;
    }
    if fetch_by_triggers(t, commands).is_some() {
        return None;
    }
    fetch_by_triggers(&other, commands).map(|_| other)
}

// ---- a misheard or mistyped phrase: the best command phrases, and whether to run or to ask ----
pub struct Fuzzy<'a> {
    pub score: f64,
    pub path: &'a PathBuf,
    pub cmd: &'a JCommand,
    pub phrase: String,
}

// at most `n` different commands whose phrase is similar to the input (the best phrase of each), best first
pub fn fuzzy_top<'a>(phrase: &str, commands: &'a [JCommandsList], min_score: f64, n: usize) -> Vec<Fuzzy<'a>> {
    let lang = i18n::get_language();
    let phrase = phrase.trim().to_lowercase();
    if phrase.is_empty() {
        return Vec::new();
    }
    let phrase_chars: Vec<char> = phrase.chars().collect();
    let phrase_words: Vec<&str> = phrase.split_whitespace().collect();
    let mut found: Vec<Fuzzy<'a>> = Vec::new();
    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            let mut best: Option<(f64, String)> = None;
            for cmd_phrase in cmd.get_phrases(&lang).iter() {
                let lower = cmd_phrase.trim().to_lowercase();
                let chars: Vec<char> = lower.chars().collect();
                let words: Vec<&str> = lower.split_whitespace().collect();
                let score = ratio(&phrase_chars, &chars) * 0.6 + word_overlap_score(&phrase_words, &words) * 0.4;
                if score > min_score && best.as_ref().map(|b| score > b.0).unwrap_or(true) {
                    best = Some((score, lower));
                }
            }
            if let Some((score, p)) = best {
                found.push(Fuzzy { score, path: &cmd_list.path, cmd, phrase: p });
            }
        }
    }
    found.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap().then_with(|| a.cmd.id.cmp(&b.cmd.id)));
    found.truncate(n);
    found
}

// a phrase this close is run at once (a typo or a letter lost by the recognizer); a less close one is asked about, never guessed
pub const FUZZY_AUTO: f64 = 92.0;
// two candidates this close to each other are a question «A или B»
pub const FUZZY_TIE: f64 = 6.0;

pub enum FuzzyDecision<'a> {
    Run(&'a PathBuf, &'a JCommand),
    // the phrases to offer: one («Вы имели в виду …?») or two
    Ask(Vec<String>),
    Nothing,
}

pub fn fuzzy_decide<'a>(phrase: &str, commands: &'a [JCommandsList]) -> FuzzyDecision<'a> {
    let top = fuzzy_top(phrase, commands, config::CMD_RATIO_THRESHOLD, 3);
    match top.as_slice() {
        [] => FuzzyDecision::Nothing,
        [a] if a.score >= FUZZY_AUTO => FuzzyDecision::Run(a.path, a.cmd),
        [a, b, ..] if a.score >= FUZZY_AUTO && a.score - b.score >= FUZZY_TIE => FuzzyDecision::Run(a.path, a.cmd),
        [a] => FuzzyDecision::Ask(vec![a.phrase.clone()]),
        [a, b, ..] if a.score - b.score < FUZZY_TIE => FuzzyDecision::Ask(vec![a.phrase.clone(), b.phrase.clone()]),
        [a, ..] => FuzzyDecision::Ask(vec![a.phrase.clone()]),
    }
}

pub fn fetch_command<'a>(
    phrase: &str,
    commands: &'a [JCommandsList],
) -> Option<(&'a PathBuf, &'a JCommand)> {
    fetch_command_min(phrase, commands, config::CMD_RATIO_THRESHOLD)
}

// the same with a higher bar: used when an AI can answer the phrase, so only a near-exact command phrase wins over it
pub fn fetch_command_min<'a>(
    phrase: &str,
    commands: &'a [JCommandsList],
    min_score: f64,
) -> Option<(&'a PathBuf, &'a JCommand)> {
    let lang = i18n::get_language();

    let phrase = phrase.trim().to_lowercase();
    if phrase.is_empty() {
        return None;
    }

    let phrase_chars: Vec<char> = phrase.chars().collect();
    let phrase_words: Vec<&str> = phrase.split_whitespace().collect();

    let mut result: Option<(&PathBuf, &JCommand)> = None;
    let mut best_score = min_score;

    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            let cmd_phrases = cmd.get_phrases(&lang);
            
            for cmd_phrase in cmd_phrases.iter() {
                let cmd_phrase_lower = cmd_phrase.trim().to_lowercase();
                let cmd_phrase_chars: Vec<char> = cmd_phrase_lower.chars().collect();
                
                // character-level similarity
                let char_ratio = ratio(&phrase_chars, &cmd_phrase_chars);
                
                // word-level similarity
                let cmd_words: Vec<&str> = cmd_phrase_lower.split_whitespace().collect();
                let word_score = word_overlap_score(&phrase_words, &cmd_words);
                
                // combined score
                let score = (char_ratio * 0.6) + (word_score * 0.4);
                
                // early exit on perfect match
                if score >= 99.0 {
                    debug!("Perfect match of a phrase of {} chars", phrase.chars().count());
                    return Some((&cmd_list.path, cmd));
                }
                
                if score > best_score {
                    best_score = score;
                    result = Some((&cmd_list.path, cmd));
                }
            }
        }
    }

    if let Some((_, cmd)) = result {
        info!("Fuzzy match: a phrase of {} chars -> cmd '{}' (score: {:.1}%)", phrase.chars().count(), cmd.id, best_score);
    } else {
        debug!("No match for a phrase of {} chars (best: {:.1}%)", phrase.chars().count(), best_score);
    }
    
    result
}


fn word_overlap_score(input_words: &[&str], cmd_words: &[&str]) -> f64 {
    if input_words.is_empty() || cmd_words.is_empty() {
        return 0.0;
    }

    let mut matched = 0.0;
    
    // pre-compute cmd word chars to avoid repeated allocations
    let cmd_word_chars: Vec<Vec<char>> = cmd_words
        .iter()
        .map(|w| w.chars().collect())
        .collect();
    
    for input_word in input_words {
        let input_chars: Vec<char> = input_word.chars().collect();
        
        let best_word_match = cmd_word_chars
            .iter()
            .map(|cw| ratio(&input_chars, cw))
            .fold(0.0_f64, f64::max);
        
        if best_word_match > 70.0 {
            matched += best_word_match / 100.0;
        }
    }

    let max_words = input_words.len().max(cmd_words.len()) as f64;
    (matched / max_words) * 100.0
}




pub fn execute_exe(exe: &str, args: &[String]) -> std::io::Result<Child> {
    let mut c = Command::new(exe);
    c.args(args);
    crate::null_stdio(&mut c);
    c.spawn()
}

// a program with its arguments, started directly. There is no shell here: the text of `cmd` is a program name, not a
// command line (the caller has checked it with policy::authorize)
pub fn execute_cli(cmd: &str, args: &[String]) -> std::io::Result<Child> {
    debug!("Spawning: {} {:?}", cmd, args);
    let mut c = Command::new(cmd);
    c.args(args);
    crate::null_stdio(&mut c);
    c.spawn()
}

// The risk that counts for a command: what it declares when its files are the ones of this build, "low" otherwise (such a
// command is limited to the simple things, see integrity.rs). A risk that is not one of the three words is "low".
pub fn effective_risk(cmd_path: &Path, cmd_config: &JCommand) -> crate::policy::Risk {
    let script = if cmd_config.cmd_type == "lua" { if cmd_config.script.is_empty() { "script.lua" } else { cmd_config.script.as_str() } } else { "" };
    if !crate::integrity::pack_command_is_trusted(cmd_path, script) {
        return crate::policy::Risk::Low;
    }
    crate::policy::Risk::parse(&cmd_config.risk).unwrap_or(crate::policy::Risk::Low)
}

pub fn execute_command(cmd_path: &PathBuf, cmd_config: &JCommand, phrase: Option<&str>, slots: Option<&HashMap<String, SlotValue>>) -> Result<bool, String> {
    // the sphere shows «выполняю» until this returns (also with an error)
    let _busy = crate::activity::enter(crate::activity::Kind::Executing);
    // the last door: whoever calls (voice, window, phone, Telegram, a scenario, a dialog), a paid command does not start without a license
    crate::access::check(&cmd_config.id).map_err(|d| d.message)?;
    // execute command by the type
    match cmd_config.cmd_type.as_str() {

        // a reply sound only. No listening after it (like the Lua commands): the microphone would hear Jarvis'
        // own voice and run random commands
        "voice" => Ok(false),
        
        // LUA command
        #[cfg(feature = "lua")]
        "lua" => {
            execute_lua_command(cmd_path, cmd_config, phrase, slots)
        }

        // AutoHotkey command
        // @TODO: Consider adding ahk source files execution?
        "ahk" => {
            let exe_path_absolute = Path::new(&cmd_config.exe_path);
            let exe_path_local = cmd_path.join(&cmd_config.exe_path);

            let exe_path = if exe_path_absolute.exists() {
                exe_path_absolute
            } else {
                exe_path_local.as_path()
            };

            if !exe_path.exists() {
                return Err(format!(
                    "AHK program not found: {} (compile the matching .ahk file with AutoHotkey into .exe)",
                    exe_path.display()
                ));
            }

            // a program of the command's own folder and only a command that said "high": nothing else starts from here
            let declared = if crate::integrity::pack_command_is_trusted(cmd_path, "") { cmd_config.risk.as_str() } else { "low" };
            let _running = crate::policy::enter_command(&cmd_config.id, declared, cmd_path);
            crate::policy::authorize_own_program(exe_path)?;
            execute_exe(exe_path.to_str().unwrap(), &cmd_config.exe_args)
                .map(|_| false)
                .map_err(|e| format!("AHK process spawn error: {}", e))
        }
        
        // CLI command type: a program from the short list of policy.rs with parameters of a known shape, started
        // directly (never through `cmd /C`)
        "cli" => {
            let declared = if crate::integrity::pack_command_is_trusted(cmd_path, "") { cmd_config.risk.as_str() } else { "low" };
            let _running = crate::policy::enter_command(&cmd_config.id, declared, cmd_path);
            crate::policy::authorize(&crate::policy::Op::Run { program: cmd_config.cli_cmd.clone(), args: cmd_config.cli_args.clone() })?;
            execute_cli(&cmd_config.cli_cmd, &cmd_config.cli_args)
                .map(|_| false)
                .map_err(|e| format!("CLI command error: {}", e))
        }
        
        // TERMINATOR command (T1000)
        "terminate" => {
            // the same full exit as the tray and the window: the speech stops, the microphone is released
            crate::ui::exit_now();
            Ok(false)
        }
        
        // STOP CHANING
        // "хватит", "стоп", "всё": also silences what Jarvis is reading
        "stop_chaining" => {
            // the AI answer that is still arriving stops too: otherwise its next sentences are read after "стоп"
            if let Some(id) = crate::ai::store().active() {
                crate::ai::stop(&id);
            }
            crate::tts::stop();
            Ok(false)
        }

        // other
        _ => {
            error!("Command type unknown: {}", cmd_config.cmd_type);
            Err(format!("Command type unknown: {}", cmd_config.cmd_type).into())
        }
    }
}

// look up a command by its ID
pub fn get_command_by_id<'a>(
    commands: &'a [JCommandsList],
    id: &str,
) -> Option<(&'a PathBuf, &'a JCommand)> {
    for cmd_list in commands {
        for cmd in &cmd_list.commands {
            if cmd.id == id {
                return Some((&cmd_list.path, cmd));
            }
        }
    }
    None
}

pub fn list_paths(commands: &[JCommandsList]) -> Vec<&Path> {
    commands.iter().map(|x| x.path.as_path()).collect()
}

#[cfg(feature = "lua")]
fn execute_lua_command(
    cmd_path: &PathBuf,
    cmd_config: &JCommand,
    phrase: Option<&str>,
    slots: Option<&HashMap<String, SlotValue>>
) -> Result<bool, String> {
    // get script path

    let script_name = if cmd_config.script.is_empty() {
        "script.lua"
    } else {
        &cmd_config.script
    };
    
    let script_path = cmd_path.join(script_name);
    
    if !script_path.exists() {
        return Err(format!("Lua script not found: {}", script_path.display()));
    }
    
    // parse sandbox level. A command whose files are not the ones of this build (changed, new, another build) runs as a
    // plain low command of the Standard level whatever it declares (integrity.rs)
    let trusted = crate::integrity::pack_command_is_trusted(cmd_path, script_name);
    let sandbox = if trusted { SandboxLevel::from_str(&cmd_config.sandbox) } else { SandboxLevel::Standard };
    let declared_risk: &str = if trusted { &cmd_config.risk } else { "low" };

    // create context
    let context = CommandContext {
        phrase: phrase.unwrap_or("").to_string(),
        command_id: cmd_config.id.clone(),
        command_path: cmd_path.clone(),
        language: i18n::get_language(),
        slots: slots.map(|s| s.clone()),
    };
    
    // get timeout
    let timeout = Duration::from_millis(cmd_config.timeout);
    
    info!("Executing Lua command: {} (sandbox: {:?}, timeout: {:?})", 
          cmd_config.id, sandbox, timeout);
    
    // the command runs with the risk it declared: an operation riskier than that is refused (policy.rs)
    let _running = crate::policy::enter_command(&cmd_config.id, declared_risk, cmd_path);

    // execute
    match lua::execute(&script_path, context, sandbox, timeout) {
        Ok(result) => {
            info!("Lua command {} completed (chain: {})", cmd_config.id, result.chain);
            Ok(result.chain)
        }
        Err(e) => {
            error!("Lua command {} failed: {}", cmd_config.id, e);
            // "Runtime error: runtime error: [string \"program.lua\"]:24: text" -> "text"
            let full = e.to_string();
            let short = match full.find("]:") {
                Some(i) => {
                    let rest = &full[i + 2..];
                    match rest.find(": ") {
                        Some(j) if rest[..j].chars().all(|c| c.is_ascii_digit()) => rest[j + 2..].to_string(),
                        _ => full.clone(),
                    }
                }
                None => full.clone(),
            };
            Err(short)
        }
    }
}
#[cfg(test)]
mod layout_tests {
    use super::*;

    fn pack() -> Vec<JCommandsList> {
        let file: JCommandsList = toml::from_str(
            r#"
            [[commands]]
            id = "browser_open"
            type = "lua"
            triggers.ru = ["открой браузер"]
            triggers.en = ["open browser"]
            "#,
        )
        .unwrap();
        vec![JCommandsList { path: PathBuf::from("browser"), commands: file.commands }]
    }

    #[test]
    fn english_commands_work_whatever_the_language_of_the_program_is() {
        let p = pack();
        // (the triggers of the English list count on a Russian program too)
        for lang in ["ru", "en"] {
            let t = p[0].commands[0].get_triggers_with_ru(lang);
            assert!(t.contains(&"open browser".to_string()) && t.contains(&"открой браузер".to_string()), "{}", lang);
        }
        assert!(fetch_by_triggers("please open browser now", &p).is_some());
        assert!(fetch_by_triggers("открой браузер", &p).is_some());
    }

    fn pack2() -> Vec<JCommandsList> {
        let file: JCommandsList = toml::from_str(
            r#"
            [[commands]]
            id = "browser_open"
            type = "lua"
            phrases.ru = ["открой браузер"]
            [[commands]]
            id = "browser_close"
            type = "lua"
            phrases.ru = ["закрой браузер"]
            [[commands]]
            id = "music_play"
            type = "lua"
            phrases.ru = ["включи музыку"]
            "#,
        )
        .unwrap();
        vec![JCommandsList { path: PathBuf::from("p"), commands: file.commands }]
    }

    #[test]
    fn a_near_exact_phrase_runs_a_vaguer_one_asks_and_an_unrelated_one_does_nothing() {
        let p = pack2();
        // one letter off: runs
        assert!(matches!(fuzzy_decide("включи музыкю", &p), FuzzyDecision::Run(_, c) if c.id == "music_play"));
        // two commands differ by one word: «открой» and «закрой» are close to a half-heard word: a question, not a guess
        match fuzzy_decide("ткрой браузер", &p) {
            FuzzyDecision::Ask(v) => assert!(v.contains(&"открой браузер".to_string()), "{:?}", v),
            FuzzyDecision::Run(_, c) => assert_eq!(c.id, "browser_open"),
            FuzzyDecision::Nothing => panic!(),
        }
        assert!(matches!(fuzzy_decide("совсем другое дело", &p), FuzzyDecision::Nothing));
        assert!(matches!(fuzzy_decide("", &p), FuzzyDecision::Nothing));
        // never more than three candidates and a stable order
        assert!(fuzzy_top("браузер", &p, 40.0, 3).len() <= 3);
    }

    #[test]
    fn a_phrase_in_the_wrong_keyboard_layout_is_fixed_only_when_it_becomes_a_command() {
        let p = pack();
        assert_eq!(fix_layout("jnrhjq ,hfepth", &p).as_deref(), Some("открой браузер"));
        assert_eq!(fix_layout("щзут икщцыук", &p).as_deref(), Some("open browser"));
        // already a phrase of a command, or no command in the other layout: left alone
        assert_eq!(fix_layout("открой браузер", &p), None);
        assert_eq!(fix_layout("jnrhjq ytcetcndetn", &p), None);
        assert_eq!(fix_layout("ab", &p), None);
        assert_eq!(fix_layout("12345", &p), None);
    }
}
