// "Автозагрузка с Windows": the same registry value the installer writes (HKCU\...\Run, "Jarvis" =
// "<path>\jarvis-app.exe" --background), so the switch of the window and the installer option are one setting.

pub const VALUE: &str = "Jarvis";
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

// the command line for the Run key
pub fn command_line(exe: &str) -> String {
    format!("\"{}\" --background", exe)
}

#[cfg(target_os = "windows")]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
pub fn read() -> Option<String> {
    use winapi::um::winreg::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let (key, val) = (wide(RUN_KEY), wide(VALUE));
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size) };
    if r != 0 {
        return None;
    }
    let n = (size as usize / 2).saturating_sub(1);
    Some(String::from_utf16_lossy(&buf[..n.min(buf.len())]))
}

#[cfg(target_os = "windows")]
pub fn set(on: bool) -> Result<(), String> {
    use winapi::um::winnt::REG_SZ;
    use winapi::um::winreg::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER};
    let (key, val) = (wide(RUN_KEY), wide(VALUE));
    let r = if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let data = wide(&command_line(&exe.display().to_string()));
        unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), REG_SZ, data.as_ptr() as *const _, (data.len() * 2) as u32) }
    } else {
        let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr()) };
        if r == 2 { 0 } else { r } // ERROR_FILE_NOT_FOUND: already off
    };
    if r == 0 { Ok(()) } else { Err(format!("Windows не дала изменить автозагрузку (код {})", r)) }
}

#[cfg(not(target_os = "windows"))]
pub fn read() -> Option<String> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn set(_on: bool) -> Result<(), String> {
    Err("Автозагрузка настраивается только в Windows".into())
}

// for the settings page: on (and pointing to this program) / on but to another copy / off
pub fn state() -> serde_json::Value {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    match read() {
        None => serde_json::json!({ "on": false, "supported": cfg!(target_os = "windows") }),
        Some(v) => serde_json::json!({ "on": true, "supported": true, "other_copy": !v.to_lowercase().contains(&exe.to_lowercase()) }),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_command_line_is_the_one_the_installer_writes() {
        assert_eq!(super::command_line("C:\\Users\\A\\AppData\\Local\\Programs\\Jarvis\\jarvis-app.exe"), "\"C:\\Users\\A\\AppData\\Local\\Programs\\Jarvis\\jarvis-app.exe\" --background");
        assert_eq!(super::VALUE, "Jarvis");
    }
}
