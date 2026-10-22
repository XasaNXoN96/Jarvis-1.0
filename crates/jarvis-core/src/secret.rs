// Protects the API key on disk. On Windows the key is encrypted with DPAPI (bound to the current Windows user:
// another user or another computer cannot decrypt it). Elsewhere (development only) it stays plain.
// The key is never logged and never sent to the Jarvis window.

const PREFIX: &str = "dpapi:";

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn from_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok()).collect()
}

#[cfg(target_os = "windows")]
fn protect_bytes(data: &[u8]) -> Option<Vec<u8>> {
    use winapi::um::dpapi::CryptProtectData;
    use winapi::um::winbase::LocalFree;
    use winapi::um::wincrypt::DATA_BLOB;
    unsafe {
        let mut input = DATA_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output: DATA_BLOB = std::mem::zeroed();
        if CryptProtectData(&mut input, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0, &mut output) == 0 {
            return None;
        }
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Some(out)
    }
}

#[cfg(target_os = "windows")]
fn unprotect_bytes(data: &[u8]) -> Option<Vec<u8>> {
    use winapi::um::dpapi::CryptUnprotectData;
    use winapi::um::winbase::LocalFree;
    use winapi::um::wincrypt::DATA_BLOB;
    unsafe {
        let mut input = DATA_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut output: DATA_BLOB = std::mem::zeroed();
        if CryptUnprotectData(&mut input, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), 0, &mut output) == 0 {
            return None;
        }
        let out = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as *mut _);
        Some(out)
    }
}

#[cfg(not(target_os = "windows"))]
fn protect_bytes(_data: &[u8]) -> Option<Vec<u8>> {
    None
}

#[cfg(not(target_os = "windows"))]
fn unprotect_bytes(_data: &[u8]) -> Option<Vec<u8>> {
    None
}

// ---- strict sealing of key material (the TLS key of the remote): never a silent fallback ----
// On Windows the bytes are sealed with DPAPI or the call FAILS (no plain key on a Windows disk). Elsewhere (development and the tests on Linux)
// the bytes are stored with the marker "plain:" and the file is made private by the caller. A sealed text is only opened by the same Windows
// user on the same computer; another user, another computer or a damaged file is an ERROR (the caller never answers it by making a new key).
const PLAIN: &str = "plain:";

pub fn seal_bytes_strict(data: &[u8]) -> Result<String, String> {
    if data.is_empty() {
        return Err("нечего защищать: ключ пуст".into());
    }
    match protect_bytes(data) {
        Some(blob) => Ok(format!("{}{}", PREFIX, to_hex(&blob))),
        None if cfg!(target_os = "windows") => Err("Windows (DPAPI) не смог защитить ключ".into()),
        None => Ok(format!("{}{}", PLAIN, to_hex(data))),
    }
}

pub fn open_bytes_strict(stored: &str) -> Result<Vec<u8>, String> {
    let stored = stored.trim();
    if let Some(hex) = stored.strip_prefix(PREFIX) {
        let blob = from_hex(hex).ok_or("файл ключа повреждён")?;
        return unprotect_bytes(&blob).ok_or_else(|| "Windows не смог открыть ключ (другой пользователь или другой компьютер, либо файл повреждён)".to_string());
    }
    if let Some(hex) = stored.strip_prefix(PLAIN) {
        if cfg!(target_os = "windows") {
            return Err("на Windows ключ должен быть защищён DPAPI, а в файле открытый ключ".into());
        }
        return from_hex(hex).ok_or_else(|| "файл ключа повреждён".to_string());
    }
    Err("неизвестный формат файла ключа".into())
}

// text for the settings file
pub fn seal(secret: &str) -> String {
    if secret.is_empty() {
        return String::new();
    }
    match protect_bytes(secret.as_bytes()) {
        Some(blob) => format!("{}{}", PREFIX, to_hex(&blob)),
        None => secret.to_string(),
    }
}

// the secret from the settings file text ("dpapi:..." is decrypted, anything else is an old plain value)
pub fn open(stored: &str) -> String {
    match stored.strip_prefix(PREFIX) {
        Some(hex) => from_hex(hex).and_then(|b| unprotect_bytes(&b)).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default(),
        None => stored.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip_and_plain_values() {
        assert_eq!(from_hex(&to_hex(&[0, 1, 254, 255])), Some(vec![0, 1, 254, 255]));
        assert_eq!(open("plain-key"), "plain-key");
        assert_eq!(open("dpapi:zz"), "");
        assert_eq!(seal(""), "");
    }

    #[test]
    fn key_material_is_sealed_strictly_and_a_bad_file_is_an_error_not_an_empty_key() {
        assert!(seal_bytes_strict(&[]).is_err());
        let sealed = seal_bytes_strict(&[1, 2, 3, 250]).unwrap();
        // (on Linux the development marker, on Windows a DPAPI blob; both open to the same bytes)
        assert_eq!(open_bytes_strict(&sealed).unwrap(), vec![1, 2, 3, 250]);
        assert_eq!(open_bytes_strict(&format!("  {}\n", sealed)).unwrap(), vec![1, 2, 3, 250], "whitespace around the text is harmless");
        for bad in ["", "garbage", "dpapi:", "dpapi:zz", "dpapi:0102", "plain:zz", "plain:1"] {
            assert!(open_bytes_strict(bad).is_err(), "«{}» must be an error", bad);
        }
    }
}
