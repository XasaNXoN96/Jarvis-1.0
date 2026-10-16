// Text-to-speech.
//
// Engines, in order of preference:
//   1. Piper (neural, offline): <app dir>/piper/piper.exe + voices in <app dir>/piper/voices/*.onnx
//      voice file is chosen by language: ru.onnx, en.onnx, uz.onnx ... (any .onnx as a fallback)
//   2. Windows built-in voices (System.Speech), a female voice of the current language when installed
//
// speak() never blocks. A new speak() interrupts the previous one, stop() silences it.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::{i18n, APP_CONFIG_DIR, APP_DIR};

static CURRENT: Lazy<Mutex<Option<Child>>> = Lazy::new(|| Mutex::new(None));
static GENERATION: AtomicU64 = AtomicU64::new(0);
static ENABLED: AtomicBool = AtomicBool::new(true);
// number of speak() jobs that have not finished yet (the microphone must not listen to Jarvis' own voice)
static BUSY: AtomicU64 = AtomicU64::new(0);

pub fn is_speaking() -> bool {
    BUSY.load(Ordering::SeqCst) > 0
}

const MAX_CHARS: usize = 30_000;

// tell the user once per run about a voice problem (console + Jarvis window + log)
static NOTE_SENT: AtomicBool = AtomicBool::new(false);

fn tell(text: &str) {
    use std::io::Write;
    let _ = writeln!(std::io::stdout(), "{}", text);
    #[cfg(feature = "jarvis_app")]
    crate::ipc::send(crate::ipc::IpcEvent::Output { text: text.to_string() });
}

// A phrase could not be said. The reason stays visible (the window shows it until a phrase is heard again) and is
// told again after a while: a voice that fails on every answer is never silent about it after the first time.
static PROBLEM: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
const PROBLEM_REPEAT: std::time::Duration = std::time::Duration::from_secs(600);

fn tell_once(text: &str) {
    warn!("TTS: {}", text);
    *PROBLEM.lock() = Some(text.to_string());
    // (the kind of the problem is the text before its details: a Piper error log with times in it would otherwise
    // make every failed phrase look like a new problem)
    let kind: String = text.split(['(', ':']).next().unwrap_or(text).chars().take(60).collect();
    if crate::guard::first(&format!("tts-problem:{}", crate::guard::norm(&kind)), PROBLEM_REPEAT) {
        NOTE_SENT.store(true, Ordering::SeqCst);
        tell(text);
    }
}

// why the last answer was not heard (None: the last phrase was played, or nothing failed yet)
pub fn speech_problem() -> Option<String> {
    PROBLEM.lock().clone()
}

// a player is running right now: only then the window says "SPEAKING" (making a sound file is not speaking)
pub fn is_playing() -> bool {
    CURRENT.lock().is_some() || NATIVE_PLAYING.load(Ordering::SeqCst)
}

// a sound file is being played by Windows itself (PlaySound), not by a child process
static NATIVE_PLAYING: AtomicBool = AtomicBool::new(false);

// Plays a WAV file with the Windows sound API in this thread (no PowerShell, no child process) and returns when it is
// over or purged by stop(). true: played; false: Windows could not (no output device, a file that does not open).
#[cfg(target_os = "windows")]
fn play_file_native(wav: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::playsoundapi::{PlaySoundW, SND_FILENAME, SND_NODEFAULT, SND_SYNC};
    let wide: Vec<u16> = wav.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    unsafe { PlaySoundW(wide.as_ptr(), std::ptr::null_mut(), SND_FILENAME | SND_SYNC | SND_NODEFAULT) != 0 }
}

// the sound being played is cut off at once (stop, a newer phrase, exit)
#[cfg(target_os = "windows")]
fn purge_native() {
    use winapi::um::playsoundapi::{PlaySoundW, SND_PURGE};
    unsafe {
        PlaySoundW(std::ptr::null(), std::ptr::null_mut(), SND_PURGE);
    }
}

#[cfg(not(target_os = "windows"))]
fn purge_native() {}

fn off_flag_path() -> Option<PathBuf> {
    APP_CONFIG_DIR.get().map(|d| d.join("tts_off"))
}

// call once at start: restores the "speech off" choice
pub fn init() {
    if let Some(p) = off_flag_path() {
        if p.exists() {
            ENABLED.store(false, Ordering::SeqCst);
            info!("TTS: speech is switched off (saved setting).");
        }
    }
    if ENABLED.load(Ordering::SeqCst) && crate::silero::speaker_of(&crate::uistate::voice_name()).is_some() {
        crate::silero::prewarm();
    }
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::SeqCst)
}

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::SeqCst);
    if let Some(p) = off_flag_path() {
        if on {
            let _ = std::fs::remove_file(p);
        } else {
            let _ = crate::write_atomic(&p, b"off");
        }
    }
    if !on {
        stop();
    }
}

// Jarvis is exiting: what is said stops and nothing new starts (the saved on/off choice is not touched)
pub fn shutdown() {
    ENABLED.store(false, Ordering::SeqCst);
    stop();
    // the running Piper is closed here: an installer that replaces or removes piper.exe finds it free
    warm_stop();
    crate::silero::stop_server();
}

pub fn stop() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    // what was waiting in the queue is dropped too
    EPOCH.fetch_add(1, Ordering::SeqCst);
    let dropped = QUEUE.lock().drain(..).count() as u64;
    if dropped > 0 {
        BUSY.fetch_sub(dropped, Ordering::SeqCst);
    }
    if let Some(mut child) = CURRENT.lock().take() {
        let _ = child.kill();
    }
    if NATIVE_PLAYING.load(Ordering::SeqCst) {
        purge_native();
    }
}

// ---------------------------------------------------------------------------------------------
// queue: the parts of one long answer are spoken one after another, none interrupts the other
// ---------------------------------------------------------------------------------------------
static QUEUE: Lazy<Mutex<std::collections::VecDeque<(u64, String)>>> = Lazy::new(|| Mutex::new(std::collections::VecDeque::new()));
static EPOCH: AtomicU64 = AtomicU64::new(0);
static WORKER: AtomicBool = AtomicBool::new(false);
// one phrase at a time: a plain speak() and the queue never play together
static SPEAKING: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));
// unique names of the sound files (two phrases never write the same file)
static WAV_SEQ: AtomicU64 = AtomicU64::new(0);
// when the voice began to work on the latest phrase (for the timing report)
static SPEECH_STARTED: Lazy<Mutex<Option<std::time::Instant>>> = Lazy::new(|| Mutex::new(None));

pub fn last_speech_started() -> Option<std::time::Instant> {
    *SPEECH_STARTED.lock()
}

// Queues a phrase after the ones that are being said. stop() (or a plain speak()) silences the whole queue.
pub fn speak_queued(text: &str) {
    let text: String = text.chars().take(MAX_CHARS).collect();
    let text = text.trim().to_string();
    if text.is_empty() || !is_enabled() {
        return;
    }
    // what is said can be private (a message, a note): only its size goes to the log
    info!("TTS (queued): {} chars", text.chars().count());
    if test_capture(&text) {
        return;
    }
    BUSY.fetch_add(1, Ordering::SeqCst);
    QUEUE.lock().push_back((EPOCH.load(Ordering::SeqCst), text));
    if !WORKER.swap(true, Ordering::SeqCst) {
        std::thread::spawn(queue_worker);
    }
}

fn queue_worker() {
    loop {
        let item = QUEUE.lock().pop_front();
        match item {
            None => {
                WORKER.store(false, Ordering::SeqCst);
                // a phrase may have arrived between the check and the flag
                if QUEUE.lock().is_empty() || WORKER.swap(true, Ordering::SeqCst) {
                    break;
                }
            }
            Some((epoch, text)) => {
                let _one = SPEAKING.lock();
                if epoch == EPOCH.load(Ordering::SeqCst) && is_enabled() {
                    #[cfg(feature = "jarvis_app")]
                    crate::ipc::send(crate::ipc::IpcEvent::Speaking);
                    let generation = GENERATION.load(Ordering::SeqCst);
                    speak_blocking(&text, &tts_lang(), generation);
                }
                BUSY.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }
}

fn tts_lang() -> String {
    match i18n::get_language().as_str() {
        "ua" => "uk".to_string(),
        other => other.to_string(),
    }
}

fn hide_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = command;
    }
}

// find the Piper voice for a language
fn piper_voice(lang: &str) -> Option<(PathBuf, PathBuf)> {
    // Piper (a C++ program) crashes (0xC0000409) when it is installed in a folder with non-Latin letters (C:\Users\Иван\…,
    // «C:\Jarvis Тест»): it builds the paths of its own files in the ANSI code page. It is started through a plain-ASCII
    // alias of its folder (a junction, no administrator rights needed).
    let dir = crate::ascii_path::ascii_dir(APP_DIR.join("piper"));
    let exe = dir.join(if cfg!(windows) { "piper.exe" } else { "piper" });
    if !exe.exists() {
        return None;
    }
    let voices = dir.join("voices");
    // one of the extra Russian voices, when the user chose it and its files are here (else the main voice speaks)
    if lang == "ru" {
        if let Some(id) = crate::voice_install::extra_id(&crate::uistate::voice_name()) {
            let own = voices.join(format!("ru-{}.onnx", id));
            if own.exists() {
                return Some((exe, own));
            }
        }
    }
    let exact = voices.join(format!("{}.onnx", lang));
    if exact.exists() {
        return Some((exe, exact));
    }
    let any = std::fs::read_dir(&voices)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .find(|p| p.extension().map(|x| x == "onnx").unwrap_or(false))?;
    Some((exe, any))
}

fn store(child: Child, generation: u64) -> bool {
    let mut slot = CURRENT.lock();
    if GENERATION.load(Ordering::SeqCst) != generation {
        let mut child = child;
        let _ = child.kill();
        return false;
    }
    // never two players: a previous one that is still there is stopped, not forgotten
    if let Some(mut old) = slot.replace(child) {
        let _ = old.kill();
    }
    true
}

// Some(true): the player finished normally, Some(false): it failed, None: interrupted (stop / a newer phrase)
fn wait_current(generation: u64) -> Option<bool> {
    // poll so that stop() (which kills the child) ends the wait quickly
    loop {
        if GENERATION.load(Ordering::SeqCst) != generation {
            return None;
        }
        let mut slot = CURRENT.lock();
        match slot.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(status)) => {
                    *slot = None;
                    return Some(status.success());
                }
                Err(_) => {
                    *slot = None;
                    return Some(false);
                }
                Ok(None) => {}
            },
            None => return None,
        }
        drop(slot);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

// sentences grouped into pieces of about 120-300 characters: the first piece starts playing quickly,
// the next one is prepared while the current one is played
fn split_chunks(text: &str) -> Vec<String> {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chunks: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in flat.chars() {
        cur.push(ch);
        let len = cur.chars().count();
        let sentence_end = matches!(ch, '.' | '!' | '?' | '…' | ';' | ':');
        if (sentence_end && len >= 120) || len >= 300 && ch == ' ' || len >= 400 {
            chunks.push(cur.trim().to_string());
            cur.clear();
        }
    }
    if !cur.trim().is_empty() {
        chunks.push(cur.trim().to_string());
    }
    chunks
}

enum Synth {
    Done,
    Cancelled,
    Failed(String),
}

// the last real synthesis of Piper (unix time, Ok or the error): the diagnostics show what really happened
static LAST_SYNTH: Lazy<Mutex<Option<(u64, Result<(), String>)>>> = Lazy::new(|| Mutex::new(None));

pub fn last_synth() -> Option<(u64, Result<(), String>)> {
    LAST_SYNTH.lock().clone()
}

fn note_synth(result: Result<(), String>) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    *LAST_SYNTH.lock() = Some((t, result));
}

// the last real playback (unix time, Ok or the error): making a sound file is not the same as hearing it
static LAST_PLAY: Lazy<Mutex<Option<(u64, Result<(), String>)>>> = Lazy::new(|| Mutex::new(None));

// which voice engine played (or failed): a failure of one engine is never written down as a failure of the other
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Engine {
    Piper,
    Windows,
}

impl Engine {
    fn label(self) -> &'static str {
        match self {
            Engine::Piper => "голос Jarvis (Piper)",
            Engine::Windows => "голос Windows",
        }
    }
}

static LAST_PLAY_PIPER: Lazy<Mutex<Option<(u64, Result<(), String>)>>> = Lazy::new(|| Mutex::new(None));
static LAST_PLAY_WINDOWS: Lazy<Mutex<Option<(u64, Result<(), String>)>>> = Lazy::new(|| Mutex::new(None));

fn note_play(result: Result<(), String>) {
    note_play_for(Engine::Piper, result)
}

fn note_play_for(engine: Engine, result: Result<(), String>) {
    match &result {
        Err(e) => {
            warn!("TTS ({}): playback failed: {}", engine.label(), e);
            *PROBLEM.lock() = Some(format!("Ответ не озвучен ({}): {}", engine.label(), e));
        }
        Ok(()) => *PROBLEM.lock() = None,
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let slot = match engine {
        Engine::Piper => &LAST_PLAY_PIPER,
        Engine::Windows => &LAST_PLAY_WINDOWS,
    };
    *slot.lock() = Some((t, result.clone()));
    *LAST_PLAY.lock() = Some((t, result));
}

// the last playback of one engine
pub fn last_play_for(engine: Engine) -> Option<(u64, Result<(), String>)> {
    match engine {
        Engine::Piper => LAST_PLAY_PIPER.lock().clone(),
        Engine::Windows => LAST_PLAY_WINDOWS.lock().clone(),
    }
}

pub fn last_play() -> Option<(u64, Result<(), String>)> {
    LAST_PLAY.lock().clone()
}

pub fn last_play_json() -> serde_json::Value {
    match last_play() {
        None => serde_json::json!(null),
        Some((t, Ok(()))) => serde_json::json!({ "at": t, "ok": true }),
        Some((t, Err(e))) => serde_json::json!({ "at": t, "ok": false, "error": e }),
    }
}

#[cfg(test)]
fn test_capture(text: &str) -> bool {
    TEST_SPOKEN.lock().push(text.to_string());
    true
}
#[cfg(not(test))]
fn test_capture(_text: &str) -> bool {
    false
}

#[cfg(test)]
mod speech_problem_tests {
    use super::*;

    #[test]
    fn a_failed_phrase_stays_visible_until_a_phrase_is_heard_again() {
        let _g = crate::uistate::TEST_LOCK.lock();
        tell_once("Голос Jarvis (Piper) не создал звук (тест), поэтому ответ не озвучен.");
        assert!(speech_problem().unwrap().contains("не озвучен"));
        // a second failure of another kind replaces the reason, it is not lost after the first one
        note_play(Err("проигрыватель не запустился".into()));
        assert!(speech_problem().unwrap().contains("проигрыватель не запустился"));
        note_play(Ok(()));
        assert_eq!(speech_problem(), None, "a phrase that was heard clears the reason");
        assert!(!is_playing(), "nothing plays in the tests: the window never says SPEAKING");
    }
}

// what reached the voice in the tests (speak and speak_queued); the tests that switch speech on/off hold uistate::TEST_LOCK
#[cfg(test)]
pub static TEST_SPOKEN: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));

// the tests decide whether a voice "is installed" (the test machine has no Piper)
#[cfg(test)]
pub static TEST_CAN_SPEAK: Lazy<Mutex<Option<Result<(), String>>>> = Lazy::new(|| Mutex::new(None));

// can a phrase be said right now? (the reason when not: the user is told instead of a silent "success")
pub fn can_speak() -> Result<(), String> {
    #[cfg(test)]
    if let Some(r) = TEST_CAN_SPEAK.lock().clone() {
        return r;
    }
    if !is_enabled() {
        return Err("озвучка выключена".into());
    }
    if crate::uistate::voice_name().starts_with("win:") || crate::silero::speaker_of(&crate::uistate::voice_name()).is_some() || piper_voice(&tts_lang()).is_some() {
        Ok(())
    } else {
        Err("голос Джарвиса (Piper) не установлен".into())
    }
}

pub enum SynthCheck {
    NotInstalled,
    Ok(u64),          // bytes of sound made
    Failed(String),
    Interrupted,      // Jarvis started to speak meanwhile: not checked
}

// diagnostics: Piper makes a short phrase into a temporary file, nothing is played. Proves that the voice can be made.
pub fn synth_check() -> SynthCheck {
    let Some((exe, model)) = piper_voice(&tts_lang()) else { return SynthCheck::NotInstalled };
    let wav = std::env::temp_dir().join(format!("jarvis_diag_{}.wav", std::process::id()));
    let generation = GENERATION.load(Ordering::SeqCst);
    let r = match piper_synth(&exe, &model, if tts_lang() == "en" { "Check." } else { "Проверка." }, &wav, generation) {
        Synth::Done => {
            let len = std::fs::metadata(&wav).map(|m| m.len()).unwrap_or(0);
            if len > 1000 { SynthCheck::Ok(len) } else { SynthCheck::Failed(format!("звуковой файл почти пустой ({} байт)", len)) }
        }
        Synth::Cancelled => SynthCheck::Interrupted,
        Synth::Failed(why) => SynthCheck::Failed(if why.is_empty() { "Piper не создал звук".into() } else { why }),
    };
    let _ = std::fs::remove_file(&wav);
    r
}

// The text goes to Piper as one JSON line {"text": "..."} with every non-ASCII letter written as \uXXXX: it does not depend
// on the code page of the console or of the pipe (a Windows build of Piper can read the bytes of a Russian phrase wrongly)
static JSON_INPUT: Lazy<Mutex<Option<bool>>> = Lazy::new(|| Mutex::new(None));

fn json_input() -> bool {
    JSON_INPUT.lock().unwrap_or(JSON_INPUT_DEFAULT)
}

const JSON_INPUT_DEFAULT: bool = false;

pub fn piper_json_line(text: &str) -> String {
    piper_json_line_to(text, None)
}

// the same, and with "output_file": the file of this phrase (a Piper that stays running writes each phrase where it is told)
pub fn piper_json_line_to(text: &str, output_file: Option<&str>) -> String {
    let mut out = String::from("{\"text\":\"");
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' | '\r' | '\t' => out.push(' '),
            c if (c as u32) < 0x20 => out.push(' '),
            c if c.is_ascii() => out.push(c),
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{:04x}", unit));
                }
            }
        }
    }
    out.push('"');
    if let Some(file) = output_file {
        out.push_str(",\"output_file\":\"");
        for ch in file.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                c if (c as u32) < 0x20 => out.push(' '),
                c if c.is_ascii() => out.push(c),
                c => {
                    let mut buf = [0u16; 2];
                    for unit in c.encode_utf16(&mut buf) {
                        out.push_str(&format!("\\u{:04x}", unit));
                    }
                }
            }
        }
        out.push('"');
    }
    out.push('}');
    out
}

static PIPER_DEBUG: AtomicBool = AtomicBool::new(false);
static LAST_PIPER_STDERR: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

// for the self-test only: Piper's own account of the last phrase (with --debug: the text it got and its phonemes)
pub fn set_piper_debug(on: bool) {
    PIPER_DEBUG.store(on, Ordering::SeqCst);
}

pub fn last_piper_stderr() -> String {
    LAST_PIPER_STDERR.lock().clone()
}

// for the self-test only: the same phrase with the text given as JSON or as plain bytes
pub fn synth_to_wav_input(text: &str, json: bool) -> Result<Vec<u8>, String> {
    *JSON_INPUT.lock() = Some(json);
    let r = synth_to_wav(text);
    *JSON_INPUT.lock() = None;
    r
}

// how long Piper stretches the phonemes: the speed of the voice from the settings (1.0 = as the voice was made)
static LENGTH_SCALE_OVERRIDE: Lazy<Mutex<Option<f32>>> = Lazy::new(|| Mutex::new(None));

pub fn length_scale() -> f32 {
    LENGTH_SCALE_OVERRIDE.lock().unwrap_or_else(|| 1.0 / crate::uistate::voice_rate())
}

// for the self-test only: a phrase at an exact length scale
pub fn synth_to_wav_scaled(text: &str, scale: f32) -> Result<Vec<u8>, String> {
    *LENGTH_SCALE_OVERRIDE.lock() = Some(scale);
    let r = synth_to_wav(text);
    *LENGTH_SCALE_OVERRIDE.lock() = None;
    r
}

// the speech as WAV bytes, nothing is played (the voice answer of the Telegram bot). Only Piper can do it.
pub fn synth_to_wav(text: &str) -> Result<Vec<u8>, String> {
    let Some((exe, model)) = piper_voice(&tts_lang()) else {
        return Err(if crate::uistate::voice_name().starts_with("win:") { "голос Windows нельзя записать в файл, нужен Piper".into() } else { "голос Джарвиса (Piper) не установлен".into() });
    };
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let wav = std::env::temp_dir().join(format!("jarvis_tg_{}_{}.wav", std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst)));
    let generation = GENERATION.load(Ordering::SeqCst);
    let r = match piper_synth_any(&exe, &model, text, &wav, generation) {
        Synth::Done => std::fs::read(&wav).map_err(|_| "звуковой файл не прочитался".to_string()),
        Synth::Cancelled => Err("озвучка была прервана".into()),
        Synth::Failed(why) => Err(if why.is_empty() { "Piper не создал звук".into() } else { why }),
    };
    let _ = std::fs::remove_file(&wav);
    r
}

fn piper_synth(exe: &Path, model: &Path, text: &str, wav: &Path, generation: u64) -> Synth {
    let err_file = wav.with_extension("err");
    let _ = std::fs::remove_file(wav);
    let mut command = Command::new(exe);
    command
        .arg("--model")
        .arg(ansi_safe(model))
        .arg("--output_file")
        .arg(ansi_safe_new(wav))
        .arg("--length_scale")
        .arg(format!("{:.2}", length_scale()))
        .args(if json_input() { vec!["--json-input"] } else { Vec::new() })
        .args(if PIPER_DEBUG.load(Ordering::SeqCst) { vec!["--debug"] } else { Vec::new() })
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(&err_file).map(Stdio::from).unwrap_or_else(|_| Stdio::null()));
    // piper looks for espeak-ng-data and its dlls next to piper.exe; the folder is also given explicitly,
    // because piper builds the default from its own path, which breaks in a folder with non-Latin letters
    if let Some(dir) = exe.parent() {
        command.current_dir(dir);
        let data = dir.join("espeak-ng-data");
        if data.is_dir() && !is_ascii(&data) {
            command.arg("--espeak_data").arg(ansi_safe(&data));
        }
    }
    hide_window(&mut command);

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            note_synth(Err(format!("не запустился: {}", e)));
            return Synth::Failed(format!("не запустился: {}", e));
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        if json_input() {
            let _ = stdin.write_all(piper_json_line(text).as_bytes());
        } else {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = stdin.write_all(b"\n");
    }
    // wait here (not in the shared slot: the previous piece is being played at the same time)
    let mut exit: Option<std::process::ExitStatus> = None;
    loop {
        if GENERATION.load(Ordering::SeqCst) != generation {
            let _ = child.kill();
            let _ = std::fs::remove_file(wav);
            return Synth::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(st)) => {
                exit = Some(st);
                break;
            }
            Err(_) => break,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(40)),
        }
    }
    // what Piper said about itself stays in memory only (the self-test shows it; it is never written to the log)
    *LAST_PIPER_STDERR.lock() = std::fs::read_to_string(&err_file).unwrap_or_default().chars().take(6000).collect();
    if wav.exists() {
        let _ = std::fs::remove_file(&err_file);
        note_synth(Ok(()));
        Synth::Done
    } else {
        let why = std::fs::read_to_string(&err_file).unwrap_or_default();
        let why: String = why.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
        let why: String = why.chars().take(300).collect();
        // no message of its own: at least the exit code (a missing dll or a crash has no text, only a code)
        let code = exit.and_then(|st| st.code()).map(|c| format!("Piper завершился с кодом {} (0x{:X}) и не создал звук", c, c as u32));
        let why = if why.is_empty() { code.unwrap_or_else(|| "Piper не создал звук".into()) } else { why };
        note_synth(Err(why.clone()));
        Synth::Failed(why)
    }
}

// ---------------------------------------------------------------------------------------------
// Piper that stays running: the model is loaded once, not for every phrase (starting Piper costs about half a second)
// ---------------------------------------------------------------------------------------------
// Piper is started once with --json-input and --output_dir; every phrase is one JSON line with its own "output_file",
// and Piper writes the sound there and prints the path. If anything is wrong (it does not start, does not answer, dies)
// the phrase is made the usual way, with a Piper of its own: this never makes the voice worse than before.
struct WarmSlot {
    wav: PathBuf,
    tx: std::sync::mpsc::Sender<Result<(), String>>,
    abandoned: std::sync::Arc<AtomicBool>,
}

struct WarmServer {
    key: String,
    child: Child,
    stdin: std::process::ChildStdin,
    pending: std::sync::Arc<Mutex<std::collections::VecDeque<WarmSlot>>>,
    alive: std::sync::Arc<AtomicBool>,
}

static WARM: Lazy<Mutex<Option<WarmServer>>> = Lazy::new(|| Mutex::new(None));
static WARM_SPAWNS: AtomicU64 = AtomicU64::new(0);
static WARM_SERVED: AtomicU64 = AtomicU64::new(0);
static WARM_FALLBACKS: AtomicU64 = AtomicU64::new(0);
static WARM_FIRST_MS: AtomicU64 = AtomicU64::new(0);
static WARM_LAST_MS: AtomicU64 = AtomicU64::new(0);
static WARM_WHY: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));
// how long one phrase may take before the running Piper is given up (tests make it short)
static WARM_TIMEOUT_MS: AtomicU64 = AtomicU64::new(60_000);

// only for the ordinary speech: the self-tests of the CI (debug, plain bytes, a fixed speed) use a Piper of their own
fn warm_enabled() -> bool {
    crate::uistate::settings().tts_warm && !PIPER_DEBUG.load(Ordering::SeqCst) && JSON_INPUT.lock().is_none() && LENGTH_SCALE_OVERRIDE.lock().is_none()
}

fn warm_key(exe: &Path, model: &Path) -> String {
    format!("{}|{}|{:.2}", exe.display(), model.display(), length_scale())
}

fn warm_dir() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("jarvis_piper_warm_{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("нет временной папки: {}", e))?;
    Ok(dir)
}

fn warm_kill(server: &mut WarmServer) {
    let _ = server.child.kill();
    let _ = server.child.wait();
    server.alive.store(false, Ordering::SeqCst);
}

// the file is complete when its size stops changing (Piper prints the path just before it closes the file)
fn wait_stable(path: &Path) -> bool {
    let mut last = 0u64;
    for _ in 0..60 {
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if len > 44 && len == last {
            return true;
        }
        last = len;
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    false
}

fn warm_reader(stdout: std::process::ChildStdout, pending: std::sync::Arc<Mutex<std::collections::VecDeque<WarmSlot>>>, alive: std::sync::Arc<AtomicBool>) {
    use std::io::BufRead;
    for line in std::io::BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        // Piper answers in the order of the lines
        let Some(slot) = pending.lock().pop_front() else { continue };
        if slot.abandoned.load(Ordering::SeqCst) {
            // the phrase was stopped while it was made: its sound is not needed
            wait_stable(&slot.wav);
            let _ = std::fs::remove_file(&slot.wav);
            continue;
        }
        let r = if wait_stable(&slot.wav) { Ok(()) } else { Err("Piper не создал звуковой файл".to_string()) };
        let _ = slot.tx.send(r);
    }
    alive.store(false, Ordering::SeqCst);
    for slot in pending.lock().drain(..) {
        let _ = slot.tx.send(Err("Piper завершился".to_string()));
    }
}

fn warm_spawn(exe: &Path, model: &Path, key: String) -> Result<WarmServer, String> {
    let dir = warm_dir()?;
    let mut command = Command::new(exe);
    command
        .arg("--model")
        .arg(ansi_safe(model))
        .arg("--output_dir")
        .arg(ansi_safe(&dir))
        .arg("--length_scale")
        .arg(format!("{:.2}", length_scale()))
        .arg("--json-input")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(dir.join("piper.err")).map(Stdio::from).unwrap_or_else(|_| Stdio::null()));
    if let Some(parent) = exe.parent() {
        command.current_dir(parent);
        let data = parent.join("espeak-ng-data");
        if data.is_dir() && !is_ascii(&data) {
            command.arg("--espeak_data").arg(ansi_safe(&data));
        }
    }
    hide_window(&mut command);
    let mut child = command.spawn().map_err(|e| format!("не запустился: {}", e))?;
    // it ends with Jarvis however Jarvis ends (a Windows job object); without it a forced end would leave it for a moment
    crate::tunnel::bind_to_job(&child);
    let stdin = child.stdin.take().ok_or("нет входа Piper")?;
    let stdout = child.stdout.take().ok_or("нет выхода Piper")?;
    let pending = std::sync::Arc::new(Mutex::new(std::collections::VecDeque::new()));
    let alive = std::sync::Arc::new(AtomicBool::new(true));
    let (p2, a2) = (pending.clone(), alive.clone());
    std::thread::spawn(move || warm_reader(stdout, p2, a2));
    WARM_SPAWNS.fetch_add(1, Ordering::SeqCst);
    info!("TTS: Piper stays running (model {})", model.display());
    Ok(WarmServer { key, child, stdin, pending, alive })
}

// the phrase through the running Piper; None = not possible now, make it the usual way
fn piper_synth_warm(exe: &Path, model: &Path, text: &str, wav: &Path, generation: u64) -> Option<Synth> {
    if !warm_enabled() {
        return None;
    }
    let out = ansi_safe_new(wav);
    if !is_ascii(&out) {
        return None;
    }
    let key = warm_key(exe, model);
    let t0 = std::time::Instant::now();
    let (tx, rx) = std::sync::mpsc::channel();
    let abandoned = std::sync::Arc::new(AtomicBool::new(false));
    let fresh;
    {
        let mut guard = WARM.lock();
        let usable = guard.as_ref().map(|s| s.key == key && s.alive.load(Ordering::SeqCst)).unwrap_or(false);
        fresh = !usable;
        if !usable {
            // another voice or speed, or it is gone
            if let Some(mut old) = guard.take() {
                warm_kill(&mut old);
            }
            match warm_spawn(exe, model, key) {
                Ok(s) => *guard = Some(s),
                Err(why) => {
                    WARM_FALLBACKS.fetch_add(1, Ordering::SeqCst);
                    *WARM_WHY.lock() = why;
                    return None;
                }
            }
        }
        let server = guard.as_mut()?;
        let _ = std::fs::remove_file(wav);
        server.pending.lock().push_back(WarmSlot { wav: wav.to_path_buf(), tx, abandoned: abandoned.clone() });
        let line = piper_json_line_to(text, Some(&out.to_string_lossy()));
        let written = server.stdin.write_all(line.as_bytes()).and_then(|_| server.stdin.write_all(b"\n")).and_then(|_| server.stdin.flush());
        if written.is_err() {
            if let Some(mut dead) = guard.take() {
                warm_kill(&mut dead);
            }
            WARM_FALLBACKS.fetch_add(1, Ordering::SeqCst);
            *WARM_WHY.lock() = "Piper не принимает фразы".into();
            return None;
        }
    }
    let limit = std::time::Duration::from_millis(WARM_TIMEOUT_MS.load(Ordering::SeqCst));
    loop {
        if GENERATION.load(Ordering::SeqCst) != generation {
            abandoned.store(true, Ordering::SeqCst);
            return Some(Synth::Cancelled);
        }
        match rx.recv_timeout(std::time::Duration::from_millis(40)) {
            Ok(Ok(())) => {
                let ms = t0.elapsed().as_millis() as u64;
                WARM_LAST_MS.store(ms, Ordering::SeqCst);
                if fresh || WARM_SERVED.load(Ordering::SeqCst) == 0 {
                    WARM_FIRST_MS.store(ms, Ordering::SeqCst);
                }
                WARM_SERVED.fetch_add(1, Ordering::SeqCst);
                note_synth(Ok(()));
                return Some(Synth::Done);
            }
            Ok(Err(why)) => {
                warm_failed(&why);
                return None;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if t0.elapsed() > limit {
                    abandoned.store(true, Ordering::SeqCst);
                    warm_failed("Piper не ответил вовремя");
                    return None;
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                warm_failed("Piper завершился");
                return None;
            }
        }
    }
}

// the running Piper did not work: it is closed, the phrase is made the usual way, the next phrase starts a new one
fn warm_failed(why: &str) {
    if let Some(mut old) = WARM.lock().take() {
        warm_kill(&mut old);
    }
    WARM_FALLBACKS.fetch_add(1, Ordering::SeqCst);
    *WARM_WHY.lock() = why.to_string();
    warn!("TTS: the running Piper failed ({}), the phrase is made the usual way", why);
}

// Piper stays running or not (the setting "Держать голос в памяти"); closing it frees its memory
pub fn warm_stop() {
    if let Some(mut old) = WARM.lock().take() {
        warm_kill(&mut old);
    }
}

// started in the background when Jarvis starts: the first answer does not wait for the model to load
pub fn warm_up() {
    if !is_enabled() || !warm_enabled() {
        return;
    }
    std::thread::spawn(|| {
        let Some((exe, model)) = piper_voice(&tts_lang()) else { return };
        let key = warm_key(&exe, &model);
        let mut guard = WARM.lock();
        if guard.as_ref().map(|s| s.key == key && s.alive.load(Ordering::SeqCst)).unwrap_or(false) {
            return;
        }
        if let Some(mut old) = guard.take() {
            warm_kill(&mut old);
        }
        match warm_spawn(&exe, &model, key) {
            Ok(s) => *guard = Some(s),
            Err(why) => *WARM_WHY.lock() = why,
        }
    });
}

// for the diagnostics and the CI: how the voice runs
pub fn warm_status() -> serde_json::Value {
    let running = WARM.lock().as_ref().map(|s| s.alive.load(Ordering::SeqCst)).unwrap_or(false);
    let enabled = crate::uistate::settings().tts_warm;
    serde_json::json!({
        "enabled": enabled,
        "running": running,
        "mode": if !enabled { "one-shot" } else if running || WARM_SERVED.load(Ordering::SeqCst) > 0 { "persistent" } else { "not started" },
        "spawns": WARM_SPAWNS.load(Ordering::SeqCst),
        "served": WARM_SERVED.load(Ordering::SeqCst),
        "fallbacks": WARM_FALLBACKS.load(Ordering::SeqCst),
        "first_ms": WARM_FIRST_MS.load(Ordering::SeqCst),
        "last_ms": WARM_LAST_MS.load(Ordering::SeqCst),
        "last_error": WARM_WHY.lock().clone(),
    })
}

// the running Piper if it can, else a Piper of its own for this one phrase
fn piper_synth_any(exe: &Path, model: &Path, text: &str, wav: &Path, generation: u64) -> Synth {
    if let Some(done) = piper_synth_warm(exe, model, text, wav, generation) {
        return done;
    }
    piper_synth(exe, model, text, wav, generation)
}

fn is_ascii(p: &Path) -> bool {
    p.to_string_lossy().is_ascii()
}

// Piper (a C++ program) receives its arguments in the ANSI code page: a path with Russian letters
// (C:\Users\Иван\...) arrives broken and Piper can open neither the voice nor its output file.
// The short 8.3 form of such a path has only Latin letters.
#[cfg(target_os = "windows")]
fn ansi_safe(p: &Path) -> PathBuf {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    if is_ascii(p) {
        return p.to_path_buf();
    }
    let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut buf = vec![0u16; 1024];
    let n = unsafe { winapi::um::fileapi::GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return p.to_path_buf();
    }
    PathBuf::from(std::ffi::OsString::from_wide(&buf[..n]))
}

#[cfg(not(target_os = "windows"))]
fn ansi_safe(p: &Path) -> PathBuf {
    p.to_path_buf()
}

// the same for a file that does not exist yet: the folder in its short form + the (Latin) file name
fn ansi_safe_new(p: &Path) -> PathBuf {
    match (p.parent(), p.file_name()) {
        (Some(dir), Some(name)) if !is_ascii(p) => ansi_safe(dir).join(name),
        _ => p.to_path_buf(),
    }
}

// "Громкость озвучки": the samples of a 16-bit PCM wav are scaled (the volume of Windows is not touched). Pure, tested.
pub fn scale_wav(data: &mut [u8], volume: u8) {
    if volume >= 100 || data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return;
    }
    // find the "data" chunk (Piper writes the plain 44-byte header, but other chunks are allowed)
    let mut pos = 12;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let len = u32::from_le_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
        if id == b"data" {
            let start = pos + 8;
            let end = (start + len).min(data.len());
            let k = volume as i32;
            for chunk in data[start..end].chunks_exact_mut(2) {
                let v = i16::from_le_bytes([chunk[0], chunk[1]]) as i32 * k / 100;
                chunk.copy_from_slice(&(v as i16).to_le_bytes());
            }
            return;
        }
        pos += 8 + len + (len & 1);
    }
}

fn apply_volume(wav: &Path) {
    let vol = crate::uistate::settings().tts_volume;
    if vol >= 100 {
        return;
    }
    if let Ok(mut data) = std::fs::read(wav) {
        scale_wav(&mut data, vol);
        let _ = std::fs::write(wav, data);
    }
}

fn play_wav(wav: &Path, generation: u64) {
    apply_volume(wav);
    // a sound file Piper made, but without any sound in it, is a failure, not a success
    let size = std::fs::metadata(wav).map(|m| m.len()).unwrap_or(0);
    if size <= 44 {
        note_play(Err(format!("Piper создал пустой звуковой файл ({} байт)", size)));
        return;
    }
    #[cfg(target_os = "windows")]
    {
        // interrupted before it started (stop / a newer phrase): nothing is played
        if GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        NATIVE_PLAYING.store(true, Ordering::SeqCst);
        let ok = play_file_native(wav);
        NATIVE_PLAYING.store(false, Ordering::SeqCst);
        // stopped on purpose while it played: neither a success nor a failure
        if GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        if ok {
            note_play(Ok(()));
        } else {
            note_play(Err("Windows не смогла воспроизвести звук (нет устройства вывода звука или файл не открылся)".into()));
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = generation;
        info!("TTS (Piper) wrote {}, playback is Windows-only", wav.display());
    }
}

// returns false only when Piper gave no sound at all (then the Windows voice takes over)
fn speak_piper(exe: &Path, model: &Path, text: &str, generation: u64) -> bool {
    let chunks = split_chunks(text);
    if chunks.is_empty() {
        return true;
    }
    let seq = WAV_SEQ.fetch_add(1, Ordering::SeqCst);
    let wav_of = |i: usize| std::env::temp_dir().join(format!("jarvis_tts_{}_{}_{}_{}.wav", std::process::id(), generation, seq, i));

    match piper_synth_any(exe, model, &chunks[0], &wav_of(0), generation) {
        Synth::Done => {}
        Synth::Cancelled => return true,
        Synth::Failed(why) => {
            tell_once(&format!("Голос Jarvis (Piper) не создал звук ({}), поэтому ответ не озвучен. Скажите «проверь голос» или откройте Настройки → Голос.", why));
            return false;
        }
    }

    for i in 0..chunks.len() {
        // prepare the next piece while this one is played
        let next = if i + 1 < chunks.len() {
            let (exe, model, text, wav) = (exe.to_path_buf(), model.to_path_buf(), chunks[i + 1].clone(), wav_of(i + 1));
            Some(std::thread::spawn(move || piper_synth_any(&exe, &model, &text, &wav, generation)))
        } else {
            None
        };
        play_wav(&wav_of(i), generation);
        let _ = std::fs::remove_file(wav_of(i));
        if GENERATION.load(Ordering::SeqCst) != generation {
            if let Some(h) = next {
                let _ = h.join();
                let _ = std::fs::remove_file(wav_of(i + 1));
            }
            return true;
        }
        if let Some(h) = next {
            match h.join() {
                Ok(Synth::Done) => {}
                Ok(Synth::Cancelled) | Err(_) => return true,
                Ok(Synth::Failed(why)) => {
                    tell_once(&format!("Голос Jarvis (Piper) остановился посреди ответа: {}", why));
                    return true;
                }
            }
        }
    }
    true
}

// the cloud voice (Yandex SpeechKit): piece by piece, the next piece is made while this one is played; false when nothing came
// at all (then Piper takes over)
fn speak_yandex(voice: &str, text: &str, generation: u64) -> bool {
    let chunks = split_chunks(text);
    if chunks.is_empty() {
        return true;
    }
    let key = crate::uistate::cloud_tts_key();
    let seq = WAV_SEQ.fetch_add(1, Ordering::SeqCst);
    let wav_of = |i: usize| std::env::temp_dir().join(format!("jarvis_yandex_{}_{}_{}_{}.wav", std::process::id(), generation, seq, i));
    let cancelled = move || GENERATION.load(Ordering::SeqCst) != generation;
    let speed = crate::uistate::settings().voice_rate;
    if let Err(why) = crate::yandex::synth(&key, &chunks[0], voice, speed, &wav_of(0)) {
        note_play(Err(format!("облачный голос не создал звук: {}", why)));
        tell_once(&format!("Облачный голос Яндекса не сработал ({}), отвечает Jarvis (Piper). Проверьте Настройки → Голос.", why));
        return false;
    }
    for i in 0..chunks.len() {
        let next = if i + 1 < chunks.len() {
            let (text, wav, key, voice) = (chunks[i + 1].clone(), wav_of(i + 1), key.clone(), voice.to_string());
            Some(std::thread::spawn(move || crate::yandex::synth(&key, &text, &voice, speed, &wav)))
        } else {
            None
        };
        if !cancelled() {
            play_wav(&wav_of(i), generation);
        }
        let _ = std::fs::remove_file(wav_of(i));
        if cancelled() {
            if let Some(h) = next {
                let _ = h.join();
                let _ = std::fs::remove_file(wav_of(i + 1));
            }
            return true;
        }
        if let Some(h) = next {
            match h.join() {
                Ok(Ok(())) => {}
                Ok(Err(why)) => {
                    tell_once(&format!("Облачный голос остановился посреди ответа: {}", why));
                    return true;
                }
                Err(_) => return true,
            }
        }
    }
    true
}

// false only when no sound came at all (then Piper takes over)
fn speak_silero(speaker: &str, text: &str, generation: u64) -> bool {
    let chunks = split_chunks(text);
    if chunks.is_empty() {
        return true;
    }
    let seq = WAV_SEQ.fetch_add(1, Ordering::SeqCst);
    let wav_of = |i: usize| std::env::temp_dir().join(format!("jarvis_silero_{}_{}_{}_{}.wav", std::process::id(), generation, seq, i));
    // speak.py speeds up by resampling, which raises the pitch (a thin voice): the natural speed is kept
    let speed = 1.0_f32;
    let cancelled = move || GENERATION.load(Ordering::SeqCst) != generation;
    let speaker = speaker.to_string();

    match crate::silero::synth(&chunks[0], &speaker, speed, &wav_of(0), &cancelled) {
        crate::silero::Outcome::Done => {}
        crate::silero::Outcome::Cancelled => return true,
        crate::silero::Outcome::Failed(why) => {
            note_play(Err(format!("голос Silero не создал звук: {}", why)));
            tell_once(&format!("Голос Silero не сработал ({}), отвечает Jarvis (Piper). Проверьте Настройки → Голос.", why));
            return false;
        }
    }
    for i in 0..chunks.len() {
        // the next piece is made while this one is played
        let next = if i + 1 < chunks.len() {
            let (text, wav, speaker) = (chunks[i + 1].clone(), wav_of(i + 1), speaker.clone());
            Some(std::thread::spawn(move || crate::silero::synth(&text, &speaker, speed, &wav, &move || GENERATION.load(Ordering::SeqCst) != generation)))
        } else {
            None
        };
        play_wav(&wav_of(i), generation);
        let _ = std::fs::remove_file(wav_of(i));
        if cancelled() {
            if let Some(h) = next {
                let _ = h.join();
                let _ = std::fs::remove_file(wav_of(i + 1));
            }
            return true;
        }
        if let Some(h) = next {
            match h.join() {
                Ok(crate::silero::Outcome::Done) => {}
                Ok(crate::silero::Outcome::Cancelled) | Err(_) => return true,
                Ok(crate::silero::Outcome::Failed(why)) => {
                    tell_once(&format!("Голос Silero остановился посреди ответа: {}", why));
                    return true;
                }
            }
        }
    }
    true
}

pub(crate) const SAPI_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$s.Rate = [int]$env:JARVIS_TTS_RATE
if ($env:JARVIS_TTS_VOLUME) { $s.Volume = [int]$env:JARVIS_TTS_VOLUME }
$lang = $env:JARVIS_TTS_LANG
$voices = @($s.GetInstalledVoices() | Where-Object { $_.Enabled })
$pick = $null
if ($env:JARVIS_TTS_VOICE) { $pick = $voices | Where-Object { $_.VoiceInfo.Name -eq $env:JARVIS_TTS_VOICE } | Select-Object -First 1 }
if (-not $pick) { $pick = $voices | Where-Object { $_.VoiceInfo.Culture.TwoLetterISOLanguageName -eq $lang -and $_.VoiceInfo.Gender -eq 'Female' } | Select-Object -First 1 }
if (-not $pick) { $pick = $voices | Where-Object { $_.VoiceInfo.Culture.TwoLetterISOLanguageName -eq $lang } | Select-Object -First 1 }
if (-not $pick) {
    # no voice for this language: Windows would stay silent on foreign text, tell Jarvis
    Set-Content -Path $env:JARVIS_TTS_NOTE -Value "NOVOICE"
    $pick = $voices | Where-Object { $_.VoiceInfo.Gender -eq 'Female' } | Select-Object -First 1
}
if ($pick) { $s.SelectVoice($pick.VoiceInfo.Name) }
$s.Speak($env:JARVIS_TTS_TEXT)
"#;

// ---- starting PowerShell for the Windows voice ----
// Windows refuses CreateProcess in some environments (a user's report: «os error 50») and the cause can be in HOW it is
// asked: the name looked up in PATH, the creation flags, the environment block, the standard handles, the command line.
// The ways below are tried one after another; the one that worked is remembered; when none works the message names every
// refusal (nothing is hidden and nothing is invented).
struct PsWay {
    name: &'static str,
    absolute: bool,
    inline_env: bool, // the values go into the script itself instead of the environment of the process
    no_flags: bool,   // without CREATE_NO_WINDOW (the window is hidden by PowerShell itself)
    piped: bool,      // standard handles are pipes, not NUL
}

const PS_WAYS: [PsWay; 6] = [
    PsWay { name: "powershell", absolute: false, inline_env: false, no_flags: false, piped: false },
    PsWay { name: "powershell, значения в команде", absolute: false, inline_env: true, no_flags: false, piped: false },
    PsWay { name: "полный путь к powershell.exe", absolute: true, inline_env: false, no_flags: false, piped: false },
    PsWay { name: "полный путь, значения в команде", absolute: true, inline_env: true, no_flags: false, piped: false },
    PsWay { name: "полный путь, без флагов окна", absolute: true, inline_env: true, no_flags: true, piped: false },
    PsWay { name: "полный путь, потоки-каналы", absolute: true, inline_env: true, no_flags: false, piped: true },
];

static PS_WAY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

fn build_ps(way: &PsWay, script: crate::psrun::Script, envs: &[(&str, String)]) -> Result<Command, String> {
    let mut command = crate::psrun::command(script, envs, crate::psrun::Way { absolute: way.absolute, inline_env: way.inline_env, no_flags: way.no_flags })?;
    if way.piped {
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        crate::null_stdio(&mut command);
    }
    if !way.no_flags {
        hide_window(&mut command);
    }
    Ok(command)
}

// Diagnostics: every way of starting PowerShell is tried with a harmless command (exit 0), and the answer of each is
// reported (the way, ok or the exact refusal). The first way that works becomes the way the voice uses.
pub fn probe_powershell_ways() -> Vec<(String, Result<(), String>)> {
    #[cfg(target_os = "windows")]
    {
        let mut out = Vec::new();
        let mut first_ok: Option<usize> = None;
        for (i, way) in PS_WAYS.iter().enumerate() {
            let r = match build_ps(way, crate::psrun::Script::Probe, &[]).and_then(|mut c| c.spawn().map_err(|e| e.to_string())) {
                Err(e) => Err(e),
                Ok(mut child) => {
                    let t0 = std::time::Instant::now();
                    loop {
                        match child.try_wait() {
                            Ok(Some(st)) if st.success() => break Ok(()),
                            Ok(Some(st)) => break Err(format!("PowerShell завершился с кодом {}", st.code().unwrap_or(-1))),
                            Ok(None) if t0.elapsed() > std::time::Duration::from_secs(15) => {
                                let _ = child.kill();
                                break Err("PowerShell не ответил за 15 секунд".to_string());
                            }
                            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                            Err(e) => break Err(e.to_string()),
                        }
                    }
                }
            };
            if r.is_ok() && first_ok.is_none() {
                first_ok = Some(i);
            }
            out.push((PS_WAYS[i].name.to_string(), r));
        }
        if let Some(i) = first_ok {
            PS_WAY.store(i, std::sync::atomic::Ordering::SeqCst);
        }
        out
    }
    #[cfg(not(target_os = "windows"))]
    {
        Vec::new()
    }
}

// which voice file Piper takes for the language, and whether it is a voice OF that language
// (without one of its own Piper falls back to any voice it has: a Russian phrase read by an English voice is nonsense)
pub fn piper_voice_info() -> Option<(String, bool)> {
    let lang = tts_lang();
    let (_, model) = piper_voice(&lang)?;
    let file = model.file_name()?.to_string_lossy().to_string();
    Some((file.clone(), file == format!("{}.onnx", lang)))
}

fn spawn_powershell(script: crate::psrun::Script, envs: &[(&str, String)]) -> Result<Child, String> {
    use std::sync::atomic::Ordering::SeqCst;
    let first = PS_WAY.load(SeqCst).min(PS_WAYS.len() - 1);
    let mut refusals: Vec<String> = Vec::new();
    for k in 0..PS_WAYS.len() {
        let i = (first + k) % PS_WAYS.len();
        match build_ps(&PS_WAYS[i], script, envs).and_then(|mut c| c.spawn().map_err(|e| e.to_string())) {
            Ok(child) => {
                if i != first || !refusals.is_empty() {
                    info!("TTS: PowerShell started the way «{}» after {} refusal(s)", PS_WAYS[i].name, refusals.len());
                }
                PS_WAY.store(i, SeqCst);
                return Ok(child);
            }
            Err(e) => refusals.push(format!("{}: {}", PS_WAYS[i].name, e)),
        }
    }
    Err(refusals.join("; "))
}

// PowerShell could not be started for the Windows voice: the reason of THIS engine (never of Piper), the way out that
// really exists, and the build (so that a report of it names the program)
fn sapi_start_failed(e: &dyn std::fmt::Display) {
    let way_out = if piper_available() {
        "Выберите голос Jarvis (Piper): Настройки → Голос → «Автоматически»."
    } else {
        "Установите голос Jarvis (Piper): Настройки → Голос → «Установить голос Джарвиса»."
    };
    let msg = format!("Не удалось запустить озвучку Windows (PowerShell): {} [сборка {}]. {}", e, crate::config::build_id(), way_out);
    note_play_for(Engine::Windows, Err(format!("PowerShell не запустился: {} [сборка {}]", e, crate::config::build_id())));
    tell_once(&msg);
}

fn speak_sapi(text: &str, lang: &str, generation: u64) {
    #[cfg(target_os = "windows")]
    {
        let note = std::env::temp_dir().join(format!("jarvis_tts_note_{}.txt", generation));
        let _ = std::fs::remove_file(&note);
        let envs: Vec<(&str, String)> = vec![
            ("JARVIS_TTS_TEXT", text.to_string()),
            ("JARVIS_TTS_LANG", lang.to_string()),
            ("JARVIS_TTS_RATE", ((crate::uistate::voice_rate() - 1.0) * 10.0).round().clamp(-10.0, 10.0).to_string()),
            ("JARVIS_TTS_VOLUME", crate::uistate::settings().tts_volume.min(100).to_string()),
            ("JARVIS_TTS_VOICE", crate::uistate::voice_name().strip_prefix("win:").unwrap_or("").to_string()),
            ("JARVIS_TTS_NOTE", note.display().to_string()),
        ];
        match spawn_powershell(crate::psrun::Script::Speak, &envs) {
            Ok(child) => {
                let result = if store(child, generation) { wait_current(generation) } else { None };
                if result == Some(true) && !note.exists() {
                    note_play_for(Engine::Windows, Ok(()));
                }
                if result == Some(false) {
                    // PowerShell / System.Speech failed (no output device, a broken voice): said, not hidden
                    note_play_for(Engine::Windows, Err("голос Windows не смог произнести фразу".into()));
                    tell_once("Голос Windows не смог произнести фразу (нет устройства вывода звука или голос повреждён). Проверьте звук или выберите голос Jarvis (Piper) в Настройки → Голос.");
                }
            }
            Err(why) => {
                sapi_start_failed(&why);
                return;
            }
        }
        if note.exists() {
            let _ = std::fs::remove_file(&note);
            if i18n::get_language() != "en" {
                tell_once("В Windows нет голоса для вашего языка, поэтому озвучка молчит. Установите голос: Параметры → Время и язык → Речь → Добавить голоса, или поставьте полную версию Jarvis с голосом Piper. Проверить: скажите «проверь голос».");
            } else {
                tell_once("Windows has no voice for your language, so speech stays silent. Install one: Settings > Time & language > Speech > Add voices, or use the full Jarvis installer with the Piper voice. Check: say \"test voice\".");
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (text, lang, generation, SAPI_SCRIPT);
        info!("TTS (not Windows, skipped)");
    }
}

// "проверь голос": what can speak on this computer, and a spoken test
pub fn diagnose() {
    std::thread::spawn(|| {
        let lang = tts_lang();
        tell(&format!("Проверка голоса. Язык Jarvis: {}", lang));
        match piper_voice(&lang) {
            Some((exe, model)) => {
                tell(&format!("Piper: найден ({}), голос {}", exe.display(), model.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
                // real test: make a short wav
                let wav = std::env::temp_dir().join("jarvis_tts_selftest.wav");
                let _ = std::fs::remove_file(&wav);
                let mut c = Command::new(&exe);
                c.arg("--model").arg(&model).arg("--output_file").arg(&wav)
                    .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped());
                if let Some(dir) = exe.parent() { c.current_dir(dir); }
                hide_window(&mut c);
                match c.spawn() {
                    Ok(mut child) => {
                        if let Some(mut si) = child.stdin.take() { let _ = si.write_all("Проверка голоса\n".as_bytes()); }
                        match child.wait_with_output() {
                            Ok(out) => {
                                let size = std::fs::metadata(&wav).map(|m| m.len()).unwrap_or(0);
                                if size > 1000 {
                                    tell(&format!("Piper: работает, тестовый звук {} КБ", size / 1024));
                                } else {
                                    let err = String::from_utf8_lossy(&out.stderr).to_string();
                                    let err: String = err.lines().rev().take(4).collect::<Vec<_>>().join(" | ");
                                    tell(&format!("Piper: ЗВУК НЕ СОЗДАН (код {:?}). {}", out.status.code(), err.chars().take(400).collect::<String>()));
                                }
                            }
                            Err(e) => tell(&format!("Piper: ошибка ожидания: {}", e)),
                        }
                        let _ = std::fs::remove_file(&wav);
                    }
                    Err(e) => tell(&format!("Piper: не запускается: {}", e)),
                }
            }
            None => tell("Piper: не установлен (в лёгкой версии его нет)"),
        }
        #[cfg(target_os = "windows")]
        {
            let command = crate::psrun::command(crate::psrun::Script::VoicesReport, &[], crate::psrun::Way::default());
            let mut command = match command {
                Ok(c) => c,
                Err(e) => {
                    tell(&format!("Не удалось опросить голоса Windows: {}", e));
                    return;
                }
            };
            command.stdout(Stdio::piped()).stderr(Stdio::null());
            hide_window(&mut command);
            match command.output() {
                Ok(out) => {
                    let list = String::from_utf8_lossy(&out.stdout).to_string();
                    let lines: Vec<&str> = list.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
                    if lines.is_empty() {
                        tell("Голоса Windows: не найдено ни одного");
                    }
                    for l in &lines {
                        tell(&format!("Голос Windows: {}", l));
                    }
                    let has_lang = lines.iter().any(|l| l.split('|').nth(1).map(|c| c.trim().to_lowercase().starts_with(&lang)).unwrap_or(false));
                    if !has_lang && piper_voice(&lang).is_none() {
                        tell(&format!("Нет ни одного голоса для языка «{}». Установите голос Windows (Параметры → Время и язык → Речь) или полную версию Jarvis с Piper.", lang));
                    }
                }
                Err(e) => tell(&format!("Не удалось опросить голоса Windows: {}", e)),
            }
        }
        NOTE_SENT.store(false, Ordering::SeqCst);
        speak("Проверка голоса. Раз, два, три.");
    });
}

pub fn speak(text: &str) {
    speak_inner(text, true);
}

// the user pressed "Озвучить" / "Озвучить задачи": said from the beginning even when the same text was just said
// (the check against two parts of the program reporting one event is for events, not for a button)
pub fn speak_requested(text: &str) {
    speak_inner(text, false);
}

fn speak_inner(text: &str, dedupe: bool) {
    let text: String = text.chars().take(MAX_CHARS).collect();
    let text = text.trim().to_string();
    if text.is_empty() || !is_enabled() {
        return;
    }
    // the same phrase again while it is still being said (two parts of the program reporting one event):
    // it is said once, not started over from the beginning
    let fresh = crate::guard::first(&format!("tts:{}", crate::guard::norm(&text)), crate::guard::NOTIFY_WINDOW);
    if dedupe && !fresh && is_speaking() {
        return;
    }
    info!("TTS: {} chars", text.chars().count());
    // the tests see what reached the voice (the test machine has no sound to play it)
    if test_capture(&text) {
        return;
    }

    #[cfg(feature = "jarvis_app")]
    crate::ipc::send(crate::ipc::IpcEvent::Speaking);

    // interrupt whatever is being said
    stop();
    let generation = GENERATION.load(Ordering::SeqCst);
    let lang = tts_lang();

    BUSY.fetch_add(1, Ordering::SeqCst);
    std::thread::spawn(move || {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                BUSY.fetch_sub(1, Ordering::SeqCst);
            }
        }
        let _done = Done;
        // the interrupted phrase ends at once (stop() killed its player); then this one plays alone
        let _one = SPEAKING.lock();
        if GENERATION.load(Ordering::SeqCst) == generation {
            speak_blocking(&text, &lang, generation);
        }
    });
}

// says the phrase and returns when it is finished (or interrupted)
fn speak_blocking(text: &str, lang: &str, generation: u64) {
    *SPEECH_STARTED.lock() = Some(std::time::Instant::now());
    let text = text.to_string();
    let lang = lang.to_string();
    // voice choice from the settings: "" = automatic, "piper", "win:<name>".
    // Jarvis speaks with his OWN voice (Piper). The Windows voice is used only when the user picked one by name.
    let choice = crate::uistate::voice_name();
    if choice.starts_with("win:") {
        speak_sapi(&text, &lang, generation);
        return;
    }
    if let Some(voice) = crate::yandex::voice_of(&choice) {
        // the cloud voice of the user's own key; when it cannot speak, Jarvis's own Piper takes over (never the Windows voice)
        if speak_yandex(voice, &text, generation) {
            return;
        }
    }
    if let Some(speaker) = crate::silero::speaker_of(&choice) {
        // the voice of the other program; when it cannot speak, Jarvis's own Piper takes over (never the Windows voice)
        if speak_silero(speaker, &text, generation) {
            return;
        }
    }
    match piper_voice(&lang) {
        Some((exe, model)) => {
            // (speak_piper reports the reason of a failure itself)
            let _ = speak_piper(&exe, &model, &text, generation);
        }
        None => {
            note_play(Err("голос Jarvis (Piper) не установлен".into()));
            tell_once("Голос Jarvis (Piper) не установлен на этом компьютере, поэтому ответы не озвучиваются (голос Windows сам не подставляется). Откройте Настройки → Голос: «Установить голос Jarvis» или выберите голос Windows.");
        }
    }
}

pub fn piper_available() -> bool {
    piper_voice(&tts_lang()).is_some()
}

// voices to choose from in the settings: ("", automatic), ("piper", ...), ("win:<name>", ...)
pub fn list_voices() -> Vec<(String, String)> {
    let mut list = vec![(String::new(), "Автоматически: голос Jarvis (Piper)".to_string())];
    list.push(("piper".to_string(), if piper_available() { "Jarvis (Piper, нейросеть)".to_string() } else { "Jarvis (Piper) — не установлен, нажмите «Установить голос»".to_string() }));
    // more Russian voices of Piper: chosen here, loaded on the first choice (the page starts the loading)
    if tts_lang() == "ru" {
        for (id, _, _, label) in crate::voice_install::EXTRA_RU {
            let text = if crate::voice_install::extra_installed(id) { label.to_string() } else { format!("{} — будет загружен при выборе", label) };
            list.push((format!("{}{}", crate::voice_install::EXTRA_PREFIX, id), text));
        }
    }
    // the cloud voices of Yandex: only when the user saved his own key (the text of an answer goes to Yandex)
    if !crate::uistate::cloud_tts_key().is_empty() && tts_lang() == "ru" {
        for (id, label) in crate::yandex::VOICES {
            list.push((format!("{}{}", crate::yandex::PREFIX, id), label.to_string()));
        }
    }
    // the Silero voices of the other program, only when it is installed here
    if crate::silero::available() {
        for (id, label) in crate::silero::SPEAKERS {
            list.push((format!("{}{}", crate::silero::PREFIX, id), label.to_string()));
        }
    }
    #[cfg(target_os = "windows")]
    {
        let command = crate::psrun::command(crate::psrun::Script::VoicesPicker, &[], crate::psrun::Way::default());
        let Ok(mut command) = command else { return list };
        command.stdout(Stdio::piped()).stderr(Stdio::null());
        hide_window(&mut command);
        if let Ok(out) = command.output() {
            for line in String::from_utf8_lossy(&out.stdout).lines() {
                let parts: Vec<&str> = line.trim().split('|').collect();
                if parts.len() == 3 && !parts[0].is_empty() {
                    list.push((format!("win:{}", parts[0]), format!("Windows: {} ({}, {})", parts[0], parts[1], parts[2])));
                }
            }
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    // the queue, the "busy" counter and the on/off switch are global: one test at a time, with every other test
    // that switches the speech (uistate::TEST_LOCK)
    use crate::uistate::TEST_LOCK as LOCK;

    #[test]
    fn queued_phrases_are_taken_one_by_one_and_the_counter_returns_to_zero() {
        let _g = LOCK.lock();
        stop();
        ENABLED.store(true, Ordering::SeqCst);
        // in the tests the voice records what reached it instead of playing it
        let before = TEST_SPOKEN.lock().len();
        speak_queued("Первая фраза.");
        speak_queued("Вторая фраза.");
        speak_queued("   ");
        let got: Vec<String> = TEST_SPOKEN.lock()[before..].to_vec();
        assert!(got.iter().any(|s| s == "Первая фраза.") && got.iter().any(|s| s == "Вторая фраза."), "{:?}", got);
        assert!(!got.iter().any(|s| s.trim().is_empty()), "an empty phrase is not queued");
        assert!(!is_speaking(), "nothing stays counted as speaking");
        assert!(QUEUE.lock().is_empty());
    }

    #[test]
    fn the_speak_button_says_the_text_again_while_an_event_repeat_is_said_once() {
        let _g = LOCK.lock();
        stop();
        ENABLED.store(true, Ordering::SeqCst);
        let text = format!("Ответ для озвучки {}", rand::random::<u32>());
        // a phrase is being said
        BUSY.fetch_add(1, Ordering::SeqCst);
        let before = TEST_SPOKEN.lock().len();
        speak(&text);
        speak(&text);
        assert_eq!(TEST_SPOKEN.lock()[before..].iter().filter(|s| **s == text).count(), 1, "the same event twice: said once");
        speak_requested(&text);
        assert_eq!(TEST_SPOKEN.lock()[before..].iter().filter(|s| **s == text).count(), 2, "«Озвучить» pressed again: said again, not silenced");
        BUSY.fetch_sub(1, Ordering::SeqCst);
    }

    #[test]
    fn stop_drops_everything_that_is_waiting() {
        let _g = LOCK.lock();
        stop();
        ENABLED.store(true, Ordering::SeqCst);
        // put phrases into the queue without a worker to take them
        for i in 0..3 {
            BUSY.fetch_add(1, Ordering::SeqCst);
            QUEUE.lock().push_back((EPOCH.load(Ordering::SeqCst), format!("фраза {}", i)));
        }
        stop();
        assert!(QUEUE.lock().is_empty(), "stop() empties the queue");
        assert_eq!(BUSY.load(Ordering::SeqCst), 0, "and the counter does not stay behind");
    }

    #[test]
    fn exiting_silences_the_voice_without_saving_speech_off() {
        let _g = LOCK.lock();
        ENABLED.store(true, Ordering::SeqCst);
        shutdown();
        assert!(!is_enabled(), "nothing new is said while Jarvis exits");
        // the saved choice (the tts_off file) is not written: the next start speaks as before
        assert!(off_flag_path().map(|p| !p.exists()).unwrap_or(true));
        ENABLED.store(true, Ordering::SeqCst);
    }

    #[test]
    fn nothing_is_queued_when_the_voice_is_off() {
        let _g = LOCK.lock();
        stop();
        ENABLED.store(false, Ordering::SeqCst);
        speak_queued("Это не должно прозвучать.");
        assert!(QUEUE.lock().is_empty());
        assert!(!is_speaking());
        ENABLED.store(true, Ordering::SeqCst);
    }

    use super::split_chunks;

    #[test]
    fn long_text_is_cut_into_sentence_pieces() {
        let text = "Это первое предложение, оно довольно длинное, чтобы набрать нужное количество символов для первой части текста. Второе предложение тоже достаточно длинное и содержит много слов, чтобы получилась вторая часть. Третье короткое.";
        let chunks = split_chunks(text);
        assert!(chunks.len() >= 2, "{:?}", chunks);
        assert!(chunks.iter().all(|c| c.chars().count() <= 400));
        assert_eq!(chunks.join(" "), text);
        assert_eq!(split_chunks("Привет."), vec!["Привет.".to_string()]);
        assert!(split_chunks("   ").is_empty());
    }
}

#[cfg(test)]
mod volume_tests {
    use super::scale_wav;

    fn wav(samples: &[i16]) -> Vec<u8> {
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut v = b"RIFF".to_vec();
        v.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        v.extend_from_slice(b"WAVEfmt ");
        v.extend_from_slice(&16u32.to_le_bytes());
        v.extend_from_slice(&[1, 0, 1, 0, 0x22, 0x56, 0, 0, 0x44, 0xAC, 0, 0, 2, 0, 16, 0]);
        v.extend_from_slice(b"data");
        v.extend_from_slice(&(data.len() as u32).to_le_bytes());
        v.extend_from_slice(&data);
        v
    }

    #[test]
    fn the_voice_volume_scales_the_samples_and_keeps_the_header() {
        let mut w = wav(&[1000, -1000, 32767, -32768]);
        let header = w[..44].to_vec();
        scale_wav(&mut w, 50);
        assert_eq!(&w[..44], header.as_slice(), "the header is untouched");
        let s: Vec<i16> = w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(s, vec![500, -500, 16383, -16384]);
        let mut full = wav(&[1234]);
        let before = full.clone();
        scale_wav(&mut full, 100);
        assert_eq!(full, before, "100 % changes nothing");
        let mut junk = b"not a wav at all".to_vec();
        scale_wav(&mut junk, 10);
        assert_eq!(junk, b"not a wav at all".to_vec());
    }
}

#[cfg(test)]
mod json_input_tests {
    use super::*;

    #[test]
    fn the_phrase_goes_to_piper_as_ascii_json() {
        let line = piper_json_line("открой ютуб \"сейчас\" \\ да");
        assert!(line.is_ascii(), "{}", line);
        assert!(line.starts_with("{\"text\":\"") && line.ends_with("\"}"));
        // the line is valid JSON and gives the phrase back
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["text"], "открой ютуб \"сейчас\" \\ да");
        // a letter outside the basic plane is written as a surrogate pair, and a line break cannot split the JSON line
        let v: serde_json::Value = serde_json::from_str(&piper_json_line("привет 😀\nмир")).unwrap();
        assert_eq!(v["text"], "привет 😀 мир");
    }
}

#[cfg(test)]
mod engine_tests {
    use super::*;

    // the message of the user's screenshot: «Не удалось запустить озвучку Windows (PowerShell): … (os error 50)»
    #[test]
    fn a_powershell_start_failure_is_the_windows_voice_failure_and_never_piper() {
        let _g = crate::uistate::TEST_LOCK.lock();
        note_play_for(Engine::Piper, Ok(()));
        let piper_before = last_play_for(Engine::Piper);
        let e = std::io::Error::from_raw_os_error(50);
        sapi_start_failed(&e);
        let problem = speech_problem().unwrap();
        assert!(problem.contains("Не удалось запустить озвучку Windows (PowerShell)"), "{}", problem);
        assert!(problem.contains("os error 50"), "the real reason is shown, not hidden: {}", problem);
        assert!(problem.contains("[сборка "), "the build is named: {}", problem);
        assert!(problem.contains("Piper"), "a way out is offered: {}", problem);
        // the failure belongs to the Windows voice only
        assert!(matches!(last_play_for(Engine::Windows), Some((_, Err(_)))));
        assert_eq!(last_play_for(Engine::Piper), piper_before, "Piper's own record is untouched");
        let win = crate::diag::windows_voice_check(true, true, last_play_for(Engine::Windows));
        assert_eq!(win.status, crate::diag::Status::Error);
        assert!(win.detail.contains("os error 50"), "{}", win.detail);
        let piper = crate::diag::piper_check_play(true, true, crate::diag::PiperFacts::Ok(30_000), None, last_play_for(Engine::Piper));
        assert_ne!(piper.status, crate::diag::Status::Error, "the Piper card does not blame Piper: {}", piper.detail);
        assert!(!piper.detail.contains("os error"), "{}", piper.detail);
        note_play_for(Engine::Windows, Ok(()));
    }

    #[test]
    fn a_failure_of_piper_is_never_written_down_for_the_windows_voice() {
        let _g = crate::uistate::TEST_LOCK.lock();
        note_play_for(Engine::Windows, Ok(()));
        let win_before = last_play_for(Engine::Windows);
        note_play_for(Engine::Piper, Err("Windows не смогла воспроизвести звук".into()));
        assert_eq!(last_play_for(Engine::Windows), win_before);
        assert!(speech_problem().unwrap().contains("голос Jarvis (Piper)"));
        let win = crate::diag::windows_voice_check(true, true, last_play_for(Engine::Windows));
        assert_eq!(win.status, crate::diag::Status::Ok, "{}", win.detail);
        note_play_for(Engine::Piper, Ok(()));
    }

    #[test]
    fn the_values_go_into_the_script_as_base64_and_the_ways_cover_every_difference() {
        let envs = vec![("JARVIS_TTS_TEXT", "он сказал 'привет' \"мир\" $x `n".to_string()), ("JARVIS_TTS_LANG", "ru".to_string())];
        let s = crate::psrun::inline_env_script("$s.Speak($env:JARVIS_TTS_TEXT)", &envs);
        assert!(s.starts_with("$env:JARVIS_TTS_TEXT = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('"));
        assert!(!s.contains("привет") && !s.contains('"'), "the text itself never stands in the command: {}", s);
        assert!(s.ends_with("$s.Speak($env:JARVIS_TTS_TEXT)"));
        assert!(s.contains(&crate::ai::base64("ru".as_bytes())));
        // the first way is the one that always was; the others differ in the name, the environment, the flags, the handles
        assert!(!PS_WAYS[0].absolute && !PS_WAYS[0].inline_env && !PS_WAYS[0].no_flags && !PS_WAYS[0].piped);
        assert!(PS_WAYS.iter().any(|w| w.absolute) && PS_WAYS.iter().any(|w| w.inline_env) && PS_WAYS.iter().any(|w| w.no_flags) && PS_WAYS.iter().any(|w| w.piped));
        let mut names: Vec<&str> = PS_WAYS.iter().map(|w| w.name).collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), PS_WAYS.len());
    }

    // every program that is started for the voice has its three standard handles set (an inherited invalid one was
    // the cause of «os error 50» before): a spawn() with none of them set is a regression
    #[test]
    fn every_spawn_of_the_voice_sets_its_standard_handles() {
        // (the file has CRLF line ends in a Windows checkout)
        let src = include_str!("tts.rs").replace("\r\n", "\n");
        let body = &src[..src.find("#[cfg(test)]\nmod speech_problem_tests").unwrap()];
        let lines: Vec<&str> = body.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            if l.contains("Command::new(") && !l.trim_start().starts_with("//") {
                let window = lines[i..(i + 25).min(lines.len())].join("\n");
                let sets_all = window.contains("null_stdio") || (window.contains(".stdin(") && window.contains(".stdout(") && window.contains(".stderr("));
                // (output() gives the child no stdin of the parent by itself)
                let captured = window.contains(".output()");
                assert!(sets_all || captured, "line {}: a process of the voice is started without its standard handles: {}", i + 1, l.trim());
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// the Piper that stays running, tested with a stand-in program that speaks the same protocol (one JSON line in, a sound
// file written, its path printed); the real Piper is checked by the CI on Windows
// ---------------------------------------------------------------------------------------------
#[cfg(all(test, unix))]
mod warm_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    static LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

    // `delay`: seconds one phrase takes; `mode`: normal | die (exits at once) | silent (reads and never answers)
    fn fake(dir: &Path, name: &str, delay: &str, mode: &str) -> PathBuf {
        let exe = dir.join(name);
        let body = match mode {
            "die" => "#!/bin/sh\nexit 1\n".to_string(),
            "silent" => "#!/bin/sh\nwhile IFS= read -r line; do :; done\n".to_string(),
            _ => format!(
                "#!/bin/sh\necho $$ >> \"$0.spawns\"\nwhile IFS= read -r line; do\n  f=$(printf '%s' \"$line\" | sed -n 's/.*\"output_file\":\"\\([^\"]*\\)\".*/\\1/p')\n  sleep {}\n  printf 'RIFF....WAVEfmt ' > \"$f\"; head -c 200 /dev/zero >> \"$f\"; printf '%s' \"$line\" >> \"$f\"\n  echo \"$f\"\ndone\n",
                delay
            ),
        };
        std::fs::write(&exe, body).unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        exe
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("jarvis-warm-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    // the tests that change the common settings of the program take the common lock of the tests first, then their own
    fn setup() -> (parking_lot::MutexGuard<'static, ()>, parking_lot::MutexGuard<'static, ()>) {
        let common = crate::uistate::TEST_LOCK.lock();
        let g = LOCK.lock();
        warm_stop();
        crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
        WARM_TIMEOUT_MS.store(60_000, Ordering::SeqCst);
        (common, g)
    }

    fn spawned(exe: &Path) -> usize {
        std::fs::read_to_string(format!("{}.spawns", exe.display())).map(|s| s.lines().count()).unwrap_or(0)
    }

    #[test]
    fn two_phrases_use_one_running_piper_and_each_gets_its_own_file() {
        let _g = setup();
        let d = tmp("two");
        let exe = fake(&d, "piper", "0", "normal");
        let model = d.join("ru.onnx");
        let generation = GENERATION.load(Ordering::SeqCst);
        let (a, b) = (d.join("a.wav"), d.join("b.wav"));
        assert!(matches!(piper_synth_warm(&exe, &model, "первая фраза", &a, generation), Some(Synth::Done)));
        assert!(matches!(piper_synth_warm(&exe, &model, "вторая фраза", &b, generation), Some(Synth::Done)));
        assert_eq!(spawned(&exe), 1, "one Piper made both phrases");
        let (ta, tb) = (std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
        assert!(ta.len() > 44 && tb.len() > 44);
        assert!(String::from_utf8_lossy(&ta).contains("\\u043f\\u0435\\u0440\\u0432\\u0430\\u044f"), "the first file holds the first phrase (written as \\u escapes)");
        assert!(String::from_utf8_lossy(&tb).contains("\\u0432\\u0442\\u043e\\u0440\\u0430\\u044f"), "the second file holds the second phrase");
        assert_eq!(warm_status()["served"].as_u64().unwrap() >= 2, true);
        assert_eq!(warm_status()["mode"], "persistent");
        warm_stop();
    }

    #[test]
    fn a_stopped_phrase_does_not_give_its_sound_to_the_next_one() {
        let _g = setup();
        let d = tmp("cancel");
        let exe = fake(&d, "piper", "0.4", "normal");
        let model = d.join("ru.onnx");
        let g1 = GENERATION.load(Ordering::SeqCst);
        let (a, b) = (d.join("a.wav"), d.join("b.wav"));
        let stopper = std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_millis(120));
            GENERATION.fetch_add(1, Ordering::SeqCst);
        });
        assert!(matches!(piper_synth_warm(&exe, &model, "прерванная", &a, g1), Some(Synth::Cancelled)));
        stopper.join().unwrap();
        let g2 = GENERATION.load(Ordering::SeqCst);
        assert!(matches!(piper_synth_warm(&exe, &model, "следующая", &b, g2), Some(Synth::Done)));
        let text = String::from_utf8_lossy(&std::fs::read(&b).unwrap()).to_string();
        assert!(text.contains("\\u0441\\u043b\\u0435\\u0434\\u0443\\u044e\\u0449\\u0430\\u044f"), "the second file is its own phrase, not the stopped one");
        assert!(!text.contains("\\u043f\\u0440\\u0435\\u0440\\u0432"), "{}", text);
        assert_eq!(spawned(&exe), 1, "the same Piper went on");
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!a.exists(), "the sound of the stopped phrase is removed");
        warm_stop();
    }

    #[test]
    fn a_changed_voice_or_a_closed_piper_starts_a_new_one() {
        let _g = setup();
        let d = tmp("new");
        let exe = fake(&d, "piper", "0", "normal");
        let generation = GENERATION.load(Ordering::SeqCst);
        let w = d.join("w.wav");
        assert!(matches!(piper_synth_warm(&exe, &d.join("ru.onnx"), "раз", &w, generation), Some(Synth::Done)));
        assert!(matches!(piper_synth_warm(&exe, &d.join("en.onnx"), "two", &w, generation), Some(Synth::Done)));
        assert_eq!(spawned(&exe), 2, "another voice, another Piper");
        warm_stop();
        assert_eq!(warm_status()["running"], false);
        assert!(matches!(piper_synth_warm(&exe, &d.join("en.onnx"), "three", &w, generation), Some(Synth::Done)));
        assert_eq!(spawned(&exe), 3, "closed: the next phrase starts a new one");
        warm_stop();
    }

    #[test]
    fn a_piper_that_dies_or_stays_silent_never_blocks_the_phrase() {
        let _g = setup();
        let d = tmp("bad");
        let generation = GENERATION.load(Ordering::SeqCst);
        let w = d.join("w.wav");
        let dead = fake(&d, "dead", "0", "die");
        let t = std::time::Instant::now();
        assert!(piper_synth_warm(&dead, &d.join("ru.onnx"), "раз", &w, generation).is_none(), "the usual way takes over");
        assert!(t.elapsed() < std::time::Duration::from_secs(5));
        assert!(warm_status()["fallbacks"].as_u64().unwrap() >= 1);
        assert!(!warm_status()["last_error"].as_str().unwrap().is_empty());
        let silent = fake(&d, "silent", "0", "silent");
        WARM_TIMEOUT_MS.store(400, Ordering::SeqCst);
        assert!(piper_synth_warm(&silent, &d.join("ru.onnx"), "раз", &w, generation).is_none(), "a silent Piper is given up");
        assert_eq!(warm_status()["running"], false, "and closed");
        WARM_TIMEOUT_MS.store(60_000, Ordering::SeqCst);
    }

    #[test]
    fn the_setting_and_the_self_test_modes_keep_the_usual_way() {
        let _g = setup();
        let d = tmp("off");
        let exe = fake(&d, "piper", "0", "normal");
        let model = d.join("ru.onnx");
        let generation = GENERATION.load(Ordering::SeqCst);
        let w = d.join("w.wav");
        crate::uistate::set_for_tests(crate::uistate::UiSettings { tts_warm: false, ..Default::default() });
        assert!(piper_synth_warm(&exe, &model, "раз", &w, generation).is_none(), "switched off in the settings");
        assert_eq!(spawned(&exe), 0, "nothing was started");
        crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
        set_piper_debug(true);
        assert!(piper_synth_warm(&exe, &model, "раз", &w, generation).is_none(), "the debug self-test uses its own Piper");
        set_piper_debug(false);
        *JSON_INPUT.lock() = Some(false);
        assert!(piper_synth_warm(&exe, &model, "раз", &w, generation).is_none(), "the plain-bytes self-test uses its own Piper");
        *JSON_INPUT.lock() = None;
        // a path with letters outside ASCII cannot be told to Piper safely: the usual way
        assert!(piper_synth_warm(&exe, &model, "раз", &d.join("файл.wav"), generation).is_none());
    }

    #[test]
    fn the_json_line_has_the_file_and_only_ascii() {
        let line = piper_json_line_to("Привет \"мир\"", Some("C:\\Temp\\a b.wav"));
        assert!(line.is_ascii(), "{}", line);
        assert!(line.contains("\"output_file\":\"C:\\\\Temp\\\\a b.wav\""), "{}", line);
        assert_eq!(piper_json_line("a"), "{\"text\":\"a\"}", "without a file the line is as it was");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(v["text"], "Привет \"мир\"");
        assert_eq!(v["output_file"], "C:\\Temp\\a b.wav");
    }
}
