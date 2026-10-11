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
}
