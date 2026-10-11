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

fn tell_once(text: &str) {
    warn!("TTS: {}", text);
    if !NOTE_SENT.swap(true, Ordering::SeqCst) {
        tell(text);
    }
}

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
            let _ = std::fs::write(p, b"off");
        }
    }
    if !on {
        stop();
    }
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
}

// ---------------------------------------------------------------------------------------------
// queue: the parts of one long answer are spoken one after another, none interrupts the other
// ---------------------------------------------------------------------------------------------
static QUEUE: Lazy<Mutex<std::collections::VecDeque<(u64, String)>>> = Lazy::new(|| Mutex::new(std::collections::VecDeque::new()));
static EPOCH: AtomicU64 = AtomicU64::new(0);
static WORKER: AtomicBool = AtomicBool::new(false);
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
    info!("TTS (queued): {}", text.chars().take(120).collect::<String>());
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
    let dir = APP_DIR.join("piper");
    let exe = dir.join(if cfg!(windows) { "piper.exe" } else { "piper" });
    if !exe.exists() {
        return None;
    }
    let voices = dir.join("voices");
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
    *slot = Some(child);
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

fn note_play(result: Result<(), String>) {
    if let Err(e) = &result {
        warn!("TTS: playback failed: {}", e);
    }
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    *LAST_PLAY.lock() = Some((t, result));
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
    if crate::uistate::voice_name().starts_with("win:") || piper_voice(&tts_lang()).is_some() {
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
        .arg(format!("{:.2}", 1.0 / crate::uistate::voice_rate()))
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
        let _ = stdin.write_all(text.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    // wait here (not in the shared slot: the previous piece is being played at the same time)
    loop {
        if GENERATION.load(Ordering::SeqCst) != generation {
            let _ = child.kill();
            let _ = std::fs::remove_file(wav);
            return Synth::Cancelled;
        }
        match child.try_wait() {
            Ok(Some(_)) | Err(_) => break,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(40)),
        }
    }
    if wav.exists() {
        let _ = std::fs::remove_file(&err_file);
        note_synth(Ok(()));
        Synth::Done
    } else {
        let why = std::fs::read_to_string(&err_file).unwrap_or_default();
        let why: String = why.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
        let why: String = why.chars().take(300).collect();
        note_synth(Err(if why.is_empty() { "Piper не создал звук".into() } else { why.clone() }));
        Synth::Failed(why)
    }
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
        let mut player = Command::new("powershell");
        player
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command"])
            .arg(format!(
                "(New-Object Media.SoundPlayer '{}').PlaySync()",
                wav.display().to_string().replace('\'', "''")
            ));
        hide_window(&mut player);
        match player.spawn() {
            Ok(child) => {
                if store(child, generation) {
                    match wait_current(generation) {
                        Some(true) => note_play(Ok(())),
                        Some(false) => note_play(Err("проигрыватель Windows не смог воспроизвести звук (нет устройства вывода или файл не открылся)".into())),
                        None => {} // interrupted on purpose
                    }
                }
            }
            Err(e) => note_play(Err(format!("PowerShell для воспроизведения не запустился: {}", e))),
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
    let wav_of = |i: usize| std::env::temp_dir().join(format!("jarvis_tts_{}_{}.wav", generation, i));

    match piper_synth(exe, model, &chunks[0], &wav_of(0), generation) {
        Synth::Done => {}
        Synth::Cancelled => return true,
        Synth::Failed(why) => {
            tell_once(&format!("Голос Piper не создал звук ({}). Пока говорит голос Windows. Скажите «проверь голос» и пришлите вывод.", why));
            return false;
        }
    }

    for i in 0..chunks.len() {
        // prepare the next piece while this one is played
        let next = if i + 1 < chunks.len() {
            let (exe, model, text, wav) = (exe.to_path_buf(), model.to_path_buf(), chunks[i + 1].clone(), wav_of(i + 1));
            Some(std::thread::spawn(move || piper_synth(&exe, &model, &text, &wav, generation)))
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
                    tell_once(&format!("Голос Piper остановился: {}", why));
                    return true;
                }
            }
        }
    }
    true
}

const SAPI_SCRIPT: &str = r#"
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

fn speak_sapi(text: &str, lang: &str, generation: u64) {
    #[cfg(target_os = "windows")]
    {
        let note = std::env::temp_dir().join(format!("jarvis_tts_note_{}.txt", generation));
        let _ = std::fs::remove_file(&note);
        let mut command = Command::new("powershell");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", SAPI_SCRIPT])
            .env("JARVIS_TTS_TEXT", text)
            .env("JARVIS_TTS_LANG", lang)
            .env("JARVIS_TTS_RATE", ((crate::uistate::voice_rate() - 1.0) * 10.0).round().clamp(-10.0, 10.0).to_string())
            .env("JARVIS_TTS_VOLUME", crate::uistate::settings().tts_volume.min(100).to_string())
            .env("JARVIS_TTS_VOICE", crate::uistate::voice_name().strip_prefix("win:").unwrap_or(""))
            .env("JARVIS_TTS_NOTE", &note)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        hide_window(&mut command);
        match command.spawn() {
            Ok(child) => {
                if store(child, generation) {
                    wait_current(generation);
                }
            }
            Err(e) => {
                tell_once(&format!("Не удалось запустить озвучку Windows (PowerShell): {}", e));
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
            let mut command = Command::new("powershell");
            command
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command",
                    "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | Where-Object { $_.Enabled } | ForEach-Object { $_.VoiceInfo.Name + ' | ' + $_.VoiceInfo.Culture.Name + ' | ' + $_.VoiceInfo.Gender }"])
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
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
    let text: String = text.chars().take(MAX_CHARS).collect();
    let text = text.trim().to_string();
    if text.is_empty() || !is_enabled() {
        return;
    }
    // the same phrase again while it is still being said (two parts of the program reporting one event):
    // it is said once, not started over from the beginning
    let fresh = crate::guard::first(&format!("tts:{}", crate::guard::norm(&text)), crate::guard::NOTIFY_WINDOW);
    if !fresh && is_speaking() {
        return;
    }
    info!("TTS: {}", text.chars().take(120).collect::<String>());

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
        speak_blocking(&text, &lang, generation);
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
    match piper_voice(&lang) {
        Some((exe, model)) => {
            if !speak_piper(&exe, &model, &text, generation) {
                tell_once("Голос Jarvis (Piper) не смог произнести фразу. Windows-голос не подставляется. Скажите «проверь голос» или откройте Настройки → Голос.");
            }
        }
        None => tell_once("Голос Jarvis (Piper) не установлен на этом компьютере, поэтому Jarvis молчит: чужой голос Windows не используется. Откройте Настройки → Голос и нажмите «Установить голос Jarvis»."),
    }
}

pub fn piper_available() -> bool {
    piper_voice(&tts_lang()).is_some()
}

// voices to choose from in the settings: ("", automatic), ("piper", ...), ("win:<name>", ...)
pub fn list_voices() -> Vec<(String, String)> {
    let mut list = vec![(String::new(), "Автоматически: голос Jarvis (Piper)".to_string())];
    list.push(("piper".to_string(), if piper_available() { "Jarvis (Piper, нейросеть)".to_string() } else { "Jarvis (Piper) — не установлен, нажмите «Установить голос»".to_string() }));
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("powershell");
        command
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command",
                "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | Where-Object { $_.Enabled } | ForEach-Object { $_.VoiceInfo.Name + '|' + $_.VoiceInfo.Culture.Name + '|' + $_.VoiceInfo.Gender }"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
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

    // the queue and the "busy" counter are global: one test at a time
    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn queued_phrases_are_taken_one_by_one_and_the_counter_returns_to_zero() {
        let _g = LOCK.lock();
        stop();
        ENABLED.store(true, Ordering::SeqCst);
        // nothing here can really be spoken (no voices in the test), the point is the bookkeeping
        speak_queued("Первая фраза.");
        speak_queued("Вторая фраза.");
        speak_queued("   ");
        assert!(BUSY.load(Ordering::SeqCst) >= 1 || QUEUE.lock().is_empty(), "queued phrases count as speaking");
        let t = std::time::Instant::now();
        while is_speaking() && t.elapsed() < std::time::Duration::from_secs(20) {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(!is_speaking(), "the voice is free again after the queue is done");
        assert!(QUEUE.lock().is_empty());
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
