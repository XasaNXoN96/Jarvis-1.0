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
    if let Some(mut child) = CURRENT.lock().take() {
        let _ = child.kill();
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

fn wait_current(generation: u64) {
    // poll so that stop() (which kills the child) ends the wait quickly
    loop {
        if GENERATION.load(Ordering::SeqCst) != generation {
            return;
        }
        let mut slot = CURRENT.lock();
        match slot.as_mut() {
            Some(child) => match child.try_wait() {
                Ok(Some(_)) | Err(_) => {
                    *slot = None;
                    return;
                }
                Ok(None) => {}
            },
            None => return,
        }
        drop(slot);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn speak_piper(exe: &Path, model: &Path, text: &str, generation: u64) -> bool {
    let wav = std::env::temp_dir().join(format!("jarvis_tts_{}.wav", generation));
    let mut command = Command::new(exe);
    command
        .arg("--model")
        .arg(model)
        .arg("--output_file")
        .arg(&wav)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_window(&mut command);

    let mut child = match command.spawn() {
        Ok(c) => c,
        Err(e) => {
            warn!("TTS: cannot start Piper: {}", e);
            return false;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
        let _ = stdin.write_all(b"\n");
    }
    if !store(child, generation) {
        return true;
    }
    wait_current(generation);
    if GENERATION.load(Ordering::SeqCst) != generation || !wav.exists() {
        let _ = std::fs::remove_file(&wav);
        return GENERATION.load(Ordering::SeqCst) != generation;
    }

    // play the wav
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
        if let Ok(child) = player.spawn() {
            if store(child, generation) {
                wait_current(generation);
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        info!("TTS (Piper) wrote {}, playback is Windows-only", wav.display());
    }
    let _ = std::fs::remove_file(&wav);
    true
}

const SAPI_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Speech
$s = New-Object System.Speech.Synthesis.SpeechSynthesizer
$lang = $env:JARVIS_TTS_LANG
$voices = @($s.GetInstalledVoices() | Where-Object { $_.Enabled })
$pick = $voices | Where-Object { $_.VoiceInfo.Culture.TwoLetterISOLanguageName -eq $lang -and $_.VoiceInfo.Gender -eq 'Female' } | Select-Object -First 1
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
            Some((exe, model)) => tell(&format!("Piper: найден ({}), голос {}", exe.display(), model.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())),
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
    info!("TTS: {}", text.chars().take(120).collect::<String>());

    #[cfg(feature = "jarvis_app")]
    crate::ipc::send(crate::ipc::IpcEvent::Speaking);

    // interrupt whatever is being said
    stop();
    let generation = GENERATION.load(Ordering::SeqCst);
    let lang = tts_lang();

    std::thread::spawn(move || {
        if let Some((exe, model)) = piper_voice(&lang) {
            if speak_piper(&exe, &model, &text, generation) {
                return;
            }
            warn!("TTS: Piper produced no sound, falling back to the Windows voice");
        }
        speak_sapi(&text, &lang, generation);
    });
}
