// System Lua API: exec, open, clipboard, notify

use mlua::{Lua, Table};
use std::process::Command;

use crate::lua::sandbox::SandboxLevel;


// Opens a URL / file / program the way a double click does (ShellExecute). Returns the real result:
// rundll32 "succeeds" even when nothing opens, ShellExecute reports an error code.
#[cfg(target_os = "windows")]
pub fn shell_open(target: &str) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::shellapi::ShellExecuteW;
    use winapi::um::winuser::SW_SHOWNORMAL;

    let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() };
    let verb = wide("open");
    let file = wide(target);
    let code = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute returned {}", code))
    }
}

#[cfg(target_os = "windows")]
fn open_target(target: &str) -> bool {
    match shell_open(target) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("[Lua] open '{}' failed: {}; trying cmd start", target, e);
            // second chance: cmd /C start "" target
            use std::os::windows::process::CommandExt;
            Command::new("cmd")
                .args(["/C", "start", "", target])
                .creation_flags(0x0800_0000)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn open_target(target: &str) -> bool {
    let result = if cfg!(target_os = "macos") {
        Command::new("open").arg(target).spawn()
    } else {
        Command::new("xdg-open").arg(target).spawn()
    };
    result.is_ok()
}

// Show a desktop notification (toast on Windows). Never fails, errors are only logged.
pub fn show_notification(title: &str, message: &str) {
    log::info!("[Lua] NOTIFY: {} - {}", title, message);

    // platform-specific notification
    #[cfg(target_os = "windows")]
    {
        use winrt_notification::{Toast, Duration as ToastDuration};

        if let Err(e) = Toast::new(Toast::POWERSHELL_APP_ID)
            .title(title)
            .text1(message)
            .duration(ToastDuration::Short)
            .show()
        {
            log::warn!("[Lua] Failed to show toast notification: {}", e);
            // fallback to msg.exe
            let _ = Command::new("msg")
                .args(["*", "/time:10", &format!("{}: {}", title, message)])
                .spawn();
        }
    }

    #[cfg(target_os = "linux")]
    {
        let _ = Command::new("notify-send")
            .args([title, message])
            .spawn();
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            r#"display notification "{}" with title "{}""#,
            message.replace("\"", "\\\""),
            title.replace("\"", "\\\"")
        );
        let _ = Command::new("osascript")
            .args(["-e", &script])
            .spawn();
    }

}

pub fn register(lua: &Lua, jarvis: &Table, sandbox: SandboxLevel) -> mlua::Result<()> {
    let system = lua.create_table()?;
    
    // jarvis.system.open(url_or_path) - always available; returns true only when Windows accepted it
    let open_fn = lua.create_function(|_, target: String| {
        let ok = open_target(&target);
        if !ok {
            log::warn!("[Lua] Failed to open {}", target);
        }
        Ok(ok)
    })?;
    system.set("open", open_fn)?;
    
    // jarvis.system.run(program, args?, wait?) - only in full sandbox
    // Starts a program directly (no shell), window hidden on Windows.
    // wait = true (default): returns { success, code, stdout, stderr }
    // wait = false: starts it and returns { success }
    if sandbox.allows_exec() {
        let run_fn = lua.create_function(|lua, (program, args, wait): (String, Option<Table>, Option<bool>)| {
            let mut command = Command::new(&program);
            if let Some(args_table) = args {
                for arg in args_table.sequence_values::<String>().flatten() {
                    command.arg(arg);
                }
            }

            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }

            let result = lua.create_table()?;
            if wait.unwrap_or(true) {
                match command.output() {
                    Ok(output) => {
                        result.set("success", output.status.success())?;
                        result.set("code", output.status.code().unwrap_or(-1))?;
                        result.set("stdout", String::from_utf8_lossy(&output.stdout).to_string())?;
                        result.set("stderr", String::from_utf8_lossy(&output.stderr).to_string())?;
                    }
                    Err(e) => {
                        log::warn!("[Lua] Failed to run {}: {}", program, e);
                        result.set("success", false)?;
                        result.set("code", -1)?;
                        result.set("stdout", "")?;
                        result.set("stderr", e.to_string())?;
                    }
                }
            } else {
                match command.spawn() {
                    Ok(_) => result.set("success", true)?,
                    Err(e) => {
                        log::warn!("[Lua] Failed to start {}: {}", program, e);
                        result.set("success", false)?;
                        result.set("stderr", e.to_string())?;
                    }
                }
            }
            Ok(result)
        })?;
        system.set("run", run_fn)?;

        let exec_fn = lua.create_function(|lua, (cmd, args): (String, Option<Table>)| {
            let mut command = if cfg!(target_os = "windows") {
                let mut c = Command::new("cmd");
                c.args(["/C", &cmd]);
                c
            } else {
                let mut c = Command::new("sh");
                c.args(["-c", &cmd]);
                c
            };
            
            // add extra args if provided
            if let Some(args_table) = args {
                for pair in args_table.sequence_values::<String>() {
                    if let Ok(arg) = pair {
                        command.arg(arg);
                    }
                }
            }
            
            let output = command.output()
                .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            
            let result = lua.create_table()?;
            result.set("success", output.status.success())?;
            result.set("code", output.status.code().unwrap_or(-1))?;
            result.set("stdout", String::from_utf8_lossy(&output.stdout).to_string())?;
            result.set("stderr", String::from_utf8_lossy(&output.stderr).to_string())?;
            
            Ok(result)
        })?;
        system.set("exec", exec_fn)?;
    }
    
    // jarvis.system.notify(title, message) - always available
    let notify_fn = lua.create_function(|_, (title, message): (String, String)| {
        show_notification(&title, &message);
        Ok(true)
    })?;
    system.set("notify", notify_fn)?;

    let popup_fn = lua.create_function(|_, (title, message): (String, String)| {
        show_popup(&title, &message);
        Ok(true)
    })?;
    system.set("popup", popup_fn)?;

    // jarvis.system.volume(action, value?) -> level 0..100 | nil, error
    let volume_fn = lua.create_function(|_, (action, value): (String, Option<i32>)| {
        match volume_control(&action, value.unwrap_or(0)) {
            Ok(v) => Ok((Some(v), None::<String>)),
            Err(e) => {
                log::warn!("[Lua] volume {} failed: {}", action, e);
                Ok((None, Some(e)))
            }
        }
    })?;
    system.set("volume", volume_fn)?;

    // jarvis.system.copy_selection() -> true when Ctrl+C was sent to another program
    system.set("copy_selection", lua.create_function(|_, ()| Ok(send_copy()))?)?;

    // jarvis.system.clipboard - subtable
    let clipboard = lua.create_table()?;
    
    // jarvis.system.clipboard.get() - always available
    let clipboard_get_fn = lua.create_function(|_, ()| {
        #[cfg(target_os = "windows")]
        {
            let output = Command::new("powershell")
                .args(["-Command", "Get-Clipboard"])
                .output()
                .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        }
        
        #[cfg(target_os = "linux")]
        {
            let output = Command::new("xclip")
                .args(["-selection", "clipboard", "-o"])
                .output()
                .or_else(|_| {
                    Command::new("xsel")
                        .args(["--clipboard", "--output"])
                        .output()
                })
                .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        
        #[cfg(target_os = "macos")]
        {
            let output = Command::new("pbpaste")
                .output()
                .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            
            Ok(String::from_utf8_lossy(&output.stdout).to_string())
        }
        
        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            Err(mlua::Error::runtime("Clipboard not supported on this platform"))
        }
    })?;
    clipboard.set("get", clipboard_get_fn)?;
    
    // jarvis.system.clipboard.set(text) - only in full sandbox
    if sandbox.allows_clipboard_write() {
        let clipboard_set_fn = lua.create_function(|_, text: String| {
            #[cfg(target_os = "windows")]
            {
                let script = format!("Set-Clipboard -Value '{}'", text.replace("'", "''"));
                Command::new("powershell")
                    .args(["-Command", &script])
                    .output()
                    .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            }
            
            #[cfg(target_os = "linux")]
            {
                use std::io::Write;
                let mut child = Command::new("xclip")
                    .args(["-selection", "clipboard"])
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .or_else(|_| {
                        Command::new("xsel")
                            .args(["--clipboard", "--input"])
                            .stdin(std::process::Stdio::piped())
                            .spawn()
                    })
                    .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(text.as_bytes())
                        .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                }
            }
            
            #[cfg(target_os = "macos")]
            {
                use std::io::Write;
                let mut child = Command::new("pbcopy")
                    .stdin(std::process::Stdio::piped())
                    .spawn()
                    .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(text.as_bytes())
                        .map_err(|e| mlua::Error::runtime(e.to_string()))?;
                }
            }
            
            Ok(true)
        })?;
        clipboard.set("set", clipboard_set_fn)?;
    }
    
    system.set("clipboard", clipboard)?;
    
    // jarvis.system.env(name) - get environment variable (always available)
    let env_fn = lua.create_function(|_, name: String| {
        Ok(std::env::var(&name).ok())
    })?;
    system.set("env", env_fn)?;
    
    // jarvis.system.platform - read-only string
    let platform = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unknown"
    };
    system.set("platform", platform)?;
    
    jarvis.set("system", system)?;
    
    Ok(())
}
// ---------------------------------------------------------------------------------------------
// Native helpers (no PowerShell): master volume, popup window, copy the selection
// ---------------------------------------------------------------------------------------------

// Master volume of the default playback device (Core Audio). Returns the new level 0..100.
#[cfg(target_os = "windows")]
pub fn volume_control(action: &str, value: i32) -> Result<i32, String> {
    use std::ptr::null_mut;
    use winapi::shared::guiddef::GUID;
    use winapi::um::combaseapi::CLSCTX_ALL;
    use winapi::um::combaseapi::{CoCreateInstance, CoInitializeEx, CoUninitialize};
    use winapi::um::endpointvolume::IAudioEndpointVolume;
    use winapi::um::mmdeviceapi::{eConsole, eRender, CLSID_MMDeviceEnumerator, IMMDevice, IMMDeviceEnumerator};
    use winapi::Interface;

    unsafe {
        // may return S_FALSE / RPC_E_CHANGED_MODE when COM is already set up on this thread: that is fine
        let init = CoInitializeEx(null_mut(), 0x2); // COINIT_APARTMENTTHREADED
        let init_ok = init >= 0;

        let result = (|| -> Result<i32, String> {
            let mut enumerator: *mut IMMDeviceEnumerator = null_mut();
            let hr = CoCreateInstance(
                &CLSID_MMDeviceEnumerator,
                null_mut(),
                CLSCTX_ALL,
                &IMMDeviceEnumerator::uuidof() as *const GUID,
                &mut enumerator as *mut _ as *mut _,
            );
            if hr < 0 || enumerator.is_null() {
                return Err(format!("no audio enumerator (0x{:08x})", hr as u32));
            }
            let mut device: *mut IMMDevice = null_mut();
            let hr = (*enumerator).GetDefaultAudioEndpoint(eRender, eConsole, &mut device);
            (*enumerator).Release();
            if hr < 0 || device.is_null() {
                return Err(format!("no playback device (0x{:08x})", hr as u32));
            }
            let mut endpoint: *mut IAudioEndpointVolume = null_mut();
            let hr = (*device).Activate(
                &IAudioEndpointVolume::uuidof() as *const GUID,
                CLSCTX_ALL,
                null_mut(),
                &mut endpoint as *mut _ as *mut _,
            );
            (*device).Release();
            if hr < 0 || endpoint.is_null() {
                return Err(format!("no volume control (0x{:08x})", hr as u32));
            }

            let null_ctx: *const GUID = std::ptr::null();
            let mut level: f32 = 0.0;
            (*endpoint).GetMasterVolumeLevelScalar(&mut level);
            let current = (level * 100.0).round() as i32;
            let clamp = |v: i32| -> f32 { v.max(0).min(100) as f32 / 100.0 };

            let mut hr = 0;
            match action {
                "set" => {
                    (*endpoint).SetMute(0, null_ctx);
                    hr = (*endpoint).SetMasterVolumeLevelScalar(clamp(value), null_ctx);
                }
                "up" => {
                    (*endpoint).SetMute(0, null_ctx);
                    hr = (*endpoint).SetMasterVolumeLevelScalar(clamp(current + value), null_ctx);
                }
                "down" => hr = (*endpoint).SetMasterVolumeLevelScalar(clamp(current - value), null_ctx),
                "mute" => hr = (*endpoint).SetMute(1, null_ctx),
                "unmute" => hr = (*endpoint).SetMute(0, null_ctx),
                _ => {}
            }
            if hr < 0 {
                (*endpoint).Release();
                return Err(format!("volume change refused (0x{:08x})", hr as u32));
            }
            (*endpoint).GetMasterVolumeLevelScalar(&mut level);
            (*endpoint).Release();
            Ok((level * 100.0).round() as i32)
        })();

        if init_ok {
            CoUninitialize();
        }
        result
    }
}

#[cfg(not(target_os = "windows"))]
pub fn volume_control(action: &str, value: i32) -> Result<i32, String> {
    log::info!("[not windows] volume {} {}", action, value);
    Ok(value)
}

// A window that stays on top until it is closed (a toast can be hidden by Focus assist / notification settings).
// Does not block the caller.
pub fn show_popup(title: &str, message: &str) {
    log::info!("[Lua] POPUP: {} - {}", title, message);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::winuser::{MessageBoxW, MB_ICONINFORMATION, MB_OK, MB_SETFOREGROUND, MB_TOPMOST};
        let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() };
        let (t, m) = (wide(title), wide(message));
        std::thread::spawn(move || unsafe {
            MessageBoxW(std::ptr::null_mut(), m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND);
        });
    }
}

// Ctrl+C in whatever window has the focus. Returns false when Jarvis' own window is in front
// (it cannot have a selection from another program then).
#[cfg(target_os = "windows")]
pub fn send_copy() -> bool {
    use winapi::um::winuser::{keybd_event, GetForegroundWindow, GetWindowTextW, KEYEVENTF_KEYUP};
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), 256);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]).to_lowercase();
        if title.contains("jarvis") {
            return false;
        }
        keybd_event(0x11, 0, 0, 0); // Ctrl
        keybd_event(0x43, 0, 0, 0); // C
        keybd_event(0x43, 0, KEYEVENTF_KEYUP, 0);
        keybd_event(0x11, 0, KEYEVENTF_KEYUP, 0);
        true
    }
}

#[cfg(not(target_os = "windows"))]
pub fn send_copy() -> bool { false }
