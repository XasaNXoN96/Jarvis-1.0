//! The TLS identity of the remote (phone <-> PC): ONE long-lived key of the PC and a certificate that is issued again with the SAME key.
//!
//! The phone pins the public key of the PC (SPKI, SHA-256), not the certificate and not the address, so the pin survives a new IP address,
//! a renewal of the certificate and a restart. A new key appears ONLY when the owner asks for it (`rotate_key`) or when the folder of the
//! program was erased; a key file that exists but cannot be read, opened or parsed is an ERROR and never a reason to make a new key:
//! a silent new key would make every phone refuse the PC (or, worse, teach the owner to accept whatever key comes).
//!
//! Files (in the folder of the program): `remote_key.sealed` (the PKCS#8 key sealed with DPAPI; written privately and atomically),
//! `remote_cert.der` (the certificate, public) and `remote_cert.meta` (the addresses and the term of the certificate).

use rcgen::{CertificateParams, DnType, KeyPair};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const KEY_FILE: &str = "remote_key.sealed";
pub const CERT_FILE: &str = "remote_cert.der";
pub const META_FILE: &str = "remote_cert.meta";
// the files of the old scheme (a new key and a new certificate for every set of addresses, the key stored plain): removed after the new key is safe
const LEGACY_FILES: [&str; 2] = ["remote_key.der", "remote_cert.ip"];

pub const CERT_YEARS: i64 = 2;
// the certificate is issued again (same key) when less than this is left of its term
pub const RENEW_BEFORE_SECS: u64 = 30 * 24 * 3600;
// a start a day in the past: a phone or a PC with a clock a little behind does not meet «not yet valid»
const BACKDATE_SECS: u64 = 24 * 3600;

pub struct Material {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub spki_sha256: [u8; 32],
    pub not_after: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Meta {
    names: Vec<String>,
    not_before: u64,
    not_after: u64,
}

fn path(name: &str) -> Result<PathBuf, String> {
    #[cfg(not(test))]
    if crate::APP_CONFIG_DIR.get().is_none() {
        // (never a key in the current folder: without the folder of the program there is no safe place)
        return Err("нет папки данных программы: ключ пульта негде безопасно хранить".into());
    }
    Ok(crate::store::file(name))
}

// base64url without padding (RFC 4648 §5); small enough to write here instead of a dependency that only the Windows target has
fn b64url(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 3);
    for c in bytes.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..=c.len() {
            out.push(A[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

/// `sha256/<base64url>` of the public key: the form that is shown, put into the QR link and pinned by the phone.
pub fn pin_text(spki_sha256: &[u8; 32]) -> String {
    format!("sha256/{}", b64url(spki_sha256))
}

#[cfg(test)]
pub(crate) fn b64url_for_tests(bytes: &[u8]) -> String {
    b64url(bytes)
}

/// The same hash for the eyes: 24 hex characters in groups of four («A1B2-C3D4-…»), compared by the owner with the phone.
pub fn fingerprint_groups(spki_sha256: &[u8; 32]) -> String {
    let hex: String = spki_sha256.iter().map(|b| format!("{:02X}", b)).collect();
    hex[..24].as_bytes().chunks(4).map(|c| String::from_utf8_lossy(c).to_string()).collect::<Vec<_>>().join("-")
}

pub fn spki_hash(key: &KeyPair) -> [u8; 32] {
    Sha256::digest(key.public_key_der()).into()
}

// a private file: written next to the target under a name of its own and renamed over it; on Unix with mode 0600 from the start
fn write_private(target: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut tmp = target.as_os_str().to_owned();
    tmp.push(format!(".{}-{:08x}.tmp", std::process::id(), rand::random::<u32>()));
    let tmp = PathBuf::from(tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let res = opts.open(&tmp).and_then(|mut f| f.write_all(data).and_then(|_| f.sync_all())).and_then(|_| std::fs::rename(&tmp, target));
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("не удалось записать {}: {}", target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), e));
    }
    Ok(())
}

fn read_key(p: &Path) -> Result<KeyPair, String> {
    let text = std::fs::read_to_string(p).map_err(|e| format!("файл ключа пульта не читается: {}", e))?;
    let der = crate::secret::open_bytes_strict(&text)?;
    KeyPair::try_from(der.as_slice()).map_err(|_| "файл ключа пульта повреждён (это не ключ)".to_string())
}

/// The key of the PC: read from the file, or made once when there is no file at all. An existing file that cannot be used is an error.
pub fn load_or_create_key() -> Result<KeyPair, String> {
    let p = path(KEY_FILE)?;
    if p.exists() {
        return read_key(&p).map_err(|e| format!("{}. Новый ключ не создан: он заставил бы все телефоны отказаться от этого компьютера. Восстановите файл или смените ключ вручную (Пульт → «Сменить ключ ПК»).", e));
    }
    let key = KeyPair::generate().map_err(|e| format!("не удалось создать ключ: {}", e))?;
    let sealed = crate::secret::seal_bytes_strict(&key.serialize_der())?;
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("не удалось создать папку данных: {}", e))?;
    }
    write_private(&p, sealed.as_bytes())?;
    // (what was written must open again: a key that cannot be read back is not a key)
    let back = read_key(&p).map_err(|e| format!("ключ записан, но не читается обратно: {}", e))?;
    if back.public_key_der() != key.public_key_der() {
        return Err("ключ записан, но при чтении обратно отличается".into());
    }
    Ok(back)
}

/// Explicit change of the key (the owner's decision): the old file is kept next to it as `.replaced-<time>`, a new key is made.
/// Every phone must then be paired again; the caller shows that warning and asks for the password of the owner.
pub fn rotate_key() -> Result<[u8; 32], String> {
    let p = path(KEY_FILE)?;
    if p.exists() {
        let mut old = p.as_os_str().to_owned();
        old.push(format!(".replaced-{}", crate::store::now()));
        std::fs::rename(&p, PathBuf::from(old)).map_err(|e| format!("не удалось отложить старый ключ: {}", e))?;
    }
    let _ = std::fs::remove_file(path(CERT_FILE)?);
    let _ = std::fs::remove_file(path(META_FILE)?);
    Ok(spki_hash(&load_or_create_key()?))
}

// the certificate for the given names, signed with the key of the PC: (DER, not_before, not_after) in unix seconds
pub fn issue(names: &[String], key: &KeyPair, now: u64) -> Result<(Vec<u8>, u64, u64), String> {
    use chrono::Datelike;
    let mut params = CertificateParams::new(names.to_vec()).map_err(|e| format!("адреса для сертификата: {}", e))?;
    params.distinguished_name.push(DnType::CommonName, "Jarvis Remote");
    let (nb, na) = (now.saturating_sub(BACKDATE_SECS), now + (CERT_YEARS as u64) * 365 * 24 * 3600);
    let ymd = |t: u64| -> Result<(i32, u8, u8), String> {
        let d = chrono::DateTime::from_timestamp(t as i64, 0).ok_or("время вне диапазона")?;
        Ok((d.year(), d.month() as u8, d.day() as u8))
    };
    let (a, b) = (ymd(nb)?, ymd(na)?);
    params.not_before = rcgen::date_time_ymd(a.0, a.1, a.2);
    params.not_after = rcgen::date_time_ymd(b.0, b.1, b.2);
    let cert = params.self_signed(key).map_err(|e| format!("не удалось выпустить сертификат: {}", e))?;
    Ok((cert.der().to_vec(), nb, na))
}

/// The key and a valid certificate for `names`: the saved certificate when it names the same addresses and has more than a month left,
/// otherwise a new certificate with the SAME key. A certificate that cannot be saved is reported (the caller logs it) but is still served.
pub fn material(names: &[String], now: u64) -> Result<(Material, Option<String>), String> {
    let key = load_or_create_key()?;
    let (cert_p, meta_p) = (path(CERT_FILE)?, path(META_FILE)?);
    let mut sorted: Vec<String> = names.to_vec();
    sorted.sort();
    let saved: Option<(Vec<u8>, Meta)> = std::fs::read(&cert_p).ok().zip(std::fs::read_to_string(&meta_p).ok().and_then(|t| serde_json::from_str::<Meta>(&t).ok()));
    let mut warning = None;
    let (cert_der, not_after) = match saved {
        Some((der, m)) if m.names == sorted && m.not_after > now + RENEW_BEFORE_SECS && m.not_before <= now => (der, m.not_after),
        _ => {
            let (der, nb, na) = issue(names, &key, now)?;
            let m = Meta { names: sorted, not_before: nb, not_after: na };
            let saved = std::fs::write(&cert_p, &der).map_err(|e| e.to_string()).and_then(|_| serde_json::to_string(&m).map_err(|e| e.to_string()).and_then(|t| std::fs::write(&meta_p, t).map_err(|e| e.to_string())));
            if let Err(e) = saved {
                warning = Some(format!("сертификат пульта не сохранён ({}): он действует до перезапуска", e));
            }
            (der, na)
        }
    };
    // the files of the old scheme (plain key!) are removed once the new key is safe
    for f in LEGACY_FILES {
        if let Ok(p) = path(f) {
            if p.exists() {
                if let Err(e) = std::fs::remove_file(&p) {
                    warning = Some(format!("старый открытый ключ пульта не удалён: {}", e));
                }
            }
        }
    }
    Ok((Material { cert_der, key_der: key.serialize_der(), spki_sha256: spki_hash(&key), not_after }, warning))
}

/// The pin and the eye form of the current key, without issuing anything (for the window and the QR link).
pub fn current_pin() -> Result<([u8; 32], String, String), String> {
    let key = load_or_create_key()?;
    let h = spki_hash(&key);
    Ok((h, pin_text(&h), fingerprint_groups(&h)))
}

#[cfg(test)]
mod tests;
