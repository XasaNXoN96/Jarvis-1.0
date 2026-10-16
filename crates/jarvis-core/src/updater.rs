//! Updates of the program: «check», «download», «install» — each one is a button, nothing happens by itself except the check.
//!
//! The build knows two things (set when the program is built): the address of the update list (`JARVIS_UPDATE_URL`, https) and
//! the PUBLIC key of the owner (`JARVIS_UPDATE_PUBKEY`). The list (`update.json`) is made by `jarvis-license-server update-sign`
//! on the owner's computer: it names the version, the address of the installer, its SHA-256 and size, and carries the owner's
//! signature over all of that. The program installs a file only when the signature fits, the address is https, the size and
//! the SHA-256 of the downloaded file fit, and the version is newer. Without the two build values the feature says so and does nothing.

use jarvis_license::{parse_public_key, verify_update, version_newer};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::path::PathBuf;

pub const CURRENT: &str = env!("CARGO_PKG_VERSION");
// the installer is never larger than this (a wrong list cannot fill the disk)
const MAX_INSTALLER: u64 = 1_500_000_000;
const MAX_LIST: usize = 64 * 1024;

pub fn build_url() -> Option<&'static str> {
    option_env!("JARVIS_UPDATE_URL").filter(|s| !s.is_empty())
}
fn build_pubkey() -> Option<&'static str> {
    option_env!("JARVIS_UPDATE_PUBKEY").filter(|s| !s.is_empty())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
    pub notes: String,
}

#[derive(Default, Clone)]
struct Status {
    checked: bool,
    checking: bool,
    available: Option<Update>,
    error: String,
    downloading: bool,
    percent: u8,
    // the verified installer on the disk
    ready: Option<PathBuf>,
}

static STATUS: Mutex<Status> = Mutex::new(Status { checked: false, checking: false, available: None, error: String::new(), downloading: false, percent: 0, ready: None });

fn is_hex(s: &str, n: usize) -> bool {
    s.len() == n && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// the list is believed only with the right signature over version, address, hash and size; None when nothing newer than `current`
pub fn read_list(text: &str, pubkey_b64: &str, current: &str) -> Result<Option<Update>, String> {
    let key = parse_public_key(pubkey_b64).ok_or("в сборке неверный публичный ключ обновлений")?;
    if text.len() > MAX_LIST {
        return Err("список обновлений слишком большой".into());
    }
    let v: Value = serde_json::from_str(text).map_err(|_| "список обновлений не читается".to_string())?;
    let field = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let (version, url, sha256, sig) = (field("version"), field("url"), field("sha256"), field("sig"));
    let size = v.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
    if !verify_update(&key, &version, &url, &sha256, size, &sig) {
        return Err("подпись списка обновлений не подходит: обновление отклонено".into());
    }
    if !url.starts_with("https://") || !is_hex(&sha256, 64) || size < 1_000_000 || size > MAX_INSTALLER {
        return Err("список обновлений составлен неверно".into());
    }
    if !version_newer(current, &version) {
        return Ok(None);
    }
    Ok(Some(Update { version, url, sha256, size, notes: field("notes").chars().take(600).collect() }))
}

// where the verified installer lies: the folder of the program, a name made of the version (only digits and dots)
fn installer_path(version: &str) -> Option<PathBuf> {
    if version.is_empty() || !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    Some(crate::APP_DIR.join("update").join(format!("Jarvis-Setup-{}.exe", version)))
}

pub fn status_json() -> Value {
    let s = STATUS.lock().clone();
    json!({
        "configured": build_url().is_some() && build_pubkey().is_some(),
        "current": CURRENT,
        "checked": s.checked, "checking": s.checking,
        "available": s.available.as_ref().map(|u| json!({ "version": u.version, "size": u.size, "notes": u.notes })),
        "error": s.error, "downloading": s.downloading, "percent": s.percent, "ready": s.ready.is_some(),
    })
}

#[cfg(feature = "lua")]
fn fetch_list(url: &str) -> Result<String, String> {
    if !url.starts_with("https://") {
        return Err("адрес обновлений должен начинаться с https://".into());
    }
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let resp = client.get(url).send().map_err(|_| "нет связи с сервером обновлений".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("сервер обновлений ответил {}", resp.status()));
    }
    let bytes = resp.bytes().map_err(|_| "список обновлений не загрузился".to_string())?;
    String::from_utf8(bytes.to_vec()).map_err(|_| "список обновлений не текст".to_string())
}

#[cfg(not(feature = "lua"))]
fn fetch_list(_url: &str) -> Result<String, String> {
    Err("в этой сборке нет сетевого модуля".into())
}

// one check in the background; the answer is read from `status_json`
pub fn check() -> Result<(), String> {
    let (Some(url), Some(key)) = (build_url(), build_pubkey()) else {
        return Err("В этой сборке не настроены обновления (нет адреса или ключа владельца).".into());
    };
    {
        let mut s = STATUS.lock();
        if s.checking || s.downloading {
            return Err("уже идёт".into());
        }
        s.checking = true;
        s.error.clear();
    }
    std::thread::spawn(move || {
        let result = fetch_list(url).and_then(|t| read_list(&t, key, CURRENT));
        let mut s = STATUS.lock();
        s.checking = false;
        s.checked = true;
        match result {
            Ok(found) => {
                if s.available.as_ref().map(|u| &u.version) != found.as_ref().map(|u| &u.version) {
                    s.ready = None;
                }
                s.available = found;
            }
            Err(e) => s.error = e,
        }
    });
    Ok(())
}

#[cfg(feature = "lua")]
fn download_to(update: &Update, dest: &std::path::Path) -> Result<(), String> {
    use std::io::{Read, Write};
    let dir = dest.parent().ok_or("нет папки для установщика")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("не удалось создать папку обновления: {}", e))?;
    let part = dest.with_extension("part");
    let _ = std::fs::remove_file(&part);
    let client = reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(20)).timeout(std::time::Duration::from_secs(60 * 40)).build().map_err(|e| e.to_string())?;
    let mut resp = client.get(&update.url).send().map_err(|_| "нет связи с сервером обновлений".to_string())?;
    if !resp.status().is_success() {
        return Err(format!("сервер ответил {}", resp.status()));
    }
    let mut file = std::fs::File::create(&part).map_err(|e| e.to_string())?;
    let (mut got, mut buf) = (0u64, [0u8; 64 * 1024]);
    loop {
        let n = resp.read(&mut buf).map_err(|_| "загрузка прервалась".to_string())?;
        if n == 0 {
            break;
        }
        got += n as u64;
        if got > update.size {
            let _ = std::fs::remove_file(&part);
            return Err("файл больше, чем сказано в списке: он удалён".into());
        }
        file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        STATUS.lock().percent = ((got * 100) / update.size.max(1)).min(99) as u8;
    }
    drop(file);
    if let Err(e) = verify_file(&part, update) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    std::fs::rename(&part, dest).map_err(|e| e.to_string())
}

#[cfg(not(feature = "lua"))]
fn download_to(_u: &Update, _d: &std::path::Path) -> Result<(), String> {
    Err("в этой сборке нет сетевого модуля".into())
}

// the file is exactly the one the owner signed: the size and the SHA-256
pub fn verify_file(path: &std::path::Path, update: &Update) -> Result<(), String> {
    let meta = std::fs::metadata(path).map_err(|_| "установщик не найден".to_string())?;
    if meta.len() != update.size {
        return Err("размер установщика не совпал со списком: файл удалён".into());
    }
    let got = crate::integrity::sha256_file(path).map_err(|e| e.to_string())?;
    if got != update.sha256 {
        return Err("SHA-256 установщика не совпал со списком: файл удалён и не будет запущен".into());
    }
    Ok(())
}

pub fn download() -> Result<(), String> {
    let update = {
        let mut s = STATUS.lock();
        if s.checking || s.downloading {
            return Err("уже идёт".into());
        }
        let u = s.available.clone().ok_or("новой версии нет: сначала проверьте обновления")?;
        s.downloading = true;
        s.percent = 0;
        s.error.clear();
        u
    };
    let Some(dest) = installer_path(&update.version) else {
        STATUS.lock().downloading = false;
        return Err("неверная версия в списке".into());
    };
    std::thread::spawn(move || {
        let result = download_to(&update, &dest);
        let mut s = STATUS.lock();
        s.downloading = false;
        match result {
            Ok(()) => {
                s.percent = 100;
                s.ready = Some(dest);
            }
            Err(e) => s.error = e,
        }
    });
    Ok(())
}

// starts the verified installer (checked once more right now); the caller closes the program after that
pub fn install() -> Result<(), String> {
    let (update, path) = {
        let s = STATUS.lock();
        (s.available.clone().ok_or("новой версии нет")?, s.ready.clone().ok_or("установщик ещё не загружен")?)
    };
    if Some(path.clone()) != installer_path(&update.version) {
        return Err("путь установщика не тот".into());
    }
    verify_file(&path, &update)?;
    launch(&path)
}

#[cfg(target_os = "windows")]
fn launch(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::shellapi::ShellExecuteW;
    let wide = |s: &std::ffi::OsStr| -> Vec<u16> { s.encode_wide().chain(std::iter::once(0)).collect() };
    let file = wide(path.as_os_str());
    let verb = wide(std::ffi::OsStr::new("open"));
    // the installer asks for the rights itself; it closes the running Jarvis and starts the new one afterwards
    let args = wide(std::ffi::OsStr::new("/SILENT /CLOSEAPPLICATIONS /RESTARTAPPLICATIONS"));
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), args.as_ptr(), std::ptr::null(), 1) } as isize;
    if r <= 32 {
        return Err("Windows не запустила установщик (отказ в правах?)".into());
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn launch(_path: &std::path::Path) -> Result<(), String> {
    Err("установка обновлений работает только в Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
    use ed25519_dalek::SigningKey;

    fn pair() -> (SigningKey, String) {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let p = B64.encode(sk.verifying_key().to_bytes());
        (sk, p)
    }
    fn sign(sk: &SigningKey, version: &str, url: &str, sha: &str, size: u64) -> String {
        use ed25519_dalek::Signer;
        B64.encode(sk.sign(jarvis_license::update_message(version, url, sha, size).as_bytes()).to_bytes())
    }
    fn list(sk: &SigningKey, version: &str, url: &str, sha: &str, size: u64) -> String {
        json!({ "version": version, "url": url, "sha256": sha, "size": size, "notes": "что нового", "sig": sign(sk, version, url, sha, size) }).to_string()
    }
    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn a_signed_newer_list_is_taken_and_an_old_or_equal_one_is_not_an_update() {
        let (sk, key) = pair();
        let t = list(&sk, "0.2.0", "https://example.com/Jarvis-Setup.exe", SHA, 50_000_000);
        let u = read_list(&t, &key, "0.1.0").unwrap().unwrap();
        assert_eq!((u.version.as_str(), u.size, u.notes.as_str()), ("0.2.0", 50_000_000, "что нового"));
        assert_eq!(read_list(&t, &key, "0.2.0").unwrap(), None);
        assert_eq!(read_list(&t, &key, "0.3.0").unwrap(), None);
    }

    #[test]
    fn a_list_with_a_wrong_signature_or_a_changed_field_is_refused() {
        let (sk, key) = pair();
        let good = list(&sk, "0.2.0", "https://example.com/Jarvis-Setup.exe", SHA, 50_000_000);
        // another owner
        let other = SigningKey::from_bytes(&[9u8; 32]);
        assert!(read_list(&list(&other, "0.2.0", "https://example.com/Jarvis-Setup.exe", SHA, 50_000_000), &key, "0.1.0").is_err());
        // the address or the hash changed after signing
        assert!(read_list(&good.replace("example.com", "evil.example"), &key, "0.1.0").is_err());
        assert!(read_list(&good.replace(&SHA[..8], "ffffffff"), &key, "0.1.0").is_err());
        assert!(read_list(&good.replace("50000000", "60000000"), &key, "0.1.0").is_err());
        // not json, empty, a wrong key in the build
        assert!(read_list("не json", &key, "0.1.0").is_err());
        assert!(read_list("{}", &key, "0.1.0").is_err());
        assert!(read_list(&good, "не ключ", "0.1.0").is_err());
    }

    #[test]
    fn a_signed_but_unsafe_list_is_refused_too() {
        let (sk, key) = pair();
        // plain http, a hash of a wrong shape, a tiny or huge size: signed by the owner, still not taken
        assert!(read_list(&list(&sk, "0.2.0", "http://example.com/a.exe", SHA, 50_000_000), &key, "0.1.0").is_err());
        assert!(read_list(&list(&sk, "0.2.0", "https://example.com/a.exe", "zz", 50_000_000), &key, "0.1.0").is_err());
        assert!(read_list(&list(&sk, "0.2.0", "https://example.com/a.exe", SHA, 10), &key, "0.1.0").is_err());
        assert!(read_list(&list(&sk, "0.2.0", "https://example.com/a.exe", SHA, MAX_INSTALLER + 1), &key, "0.1.0").is_err());
    }

    #[test]
    fn the_installer_file_must_have_the_signed_size_and_hash() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.exe");
        std::fs::write(&f, b"installer bytes").unwrap();
        let sha = crate::integrity::sha256_file(&f).unwrap();
        let ok = Update { version: "0.2.0".into(), url: "https://example.com/a.exe".into(), sha256: sha.clone(), size: 15, notes: String::new() };
        assert!(verify_file(&f, &ok).is_ok());
        assert!(verify_file(&f, &Update { size: 16, ..ok.clone() }).is_err());
        assert!(verify_file(&f, &Update { sha256: SHA.into(), ..ok.clone() }).is_err());
        assert!(verify_file(&d.path().join("missing.exe"), &ok).is_err());
    }

    #[test]
    fn the_installer_name_is_made_only_of_a_version() {
        assert!(installer_path("0.2.0").unwrap().ends_with("update/Jarvis-Setup-0.2.0.exe") || installer_path("0.2.0").unwrap().ends_with("update\\Jarvis-Setup-0.2.0.exe"));
        assert!(installer_path("../../x").is_none());
        assert!(installer_path("1.0 /x").is_none());
        assert!(installer_path("").is_none());
    }

    #[test]
    fn without_the_build_values_the_feature_says_so_and_does_nothing() {
        if build_url().is_none() || build_pubkey().is_none() {
            assert!(check().is_err());
            assert_eq!(status_json()["configured"], false);
        }
        assert!(download().is_err(), "nothing to download before a check found something");
        assert!(install().is_err());
    }
}
