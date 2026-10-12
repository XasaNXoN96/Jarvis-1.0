// System Lua API: exec, open, clipboard, notify

use mlua::{Lua, Table};
use std::process::Command;

use crate::lua::sandbox::SandboxLevel;


// Opens a URL / file / program the way a double click does (ShellExecute). Returns the real result:
// rundll32 "succeeds" even when nothing opens, ShellExecute reports an error code.
// sites open in the browser chosen in the settings ("" = the Windows default); when that browser cannot be started,
// the default one opens the site (and the log says why)
#[cfg(target_os = "windows")]
pub fn shell_open(target: &str) -> Result<(), String> {
    let lower = target.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        if let Some(exe) = crate::uistate::browser_exe(&crate::uistate::settings().browser) {
            // one argument for the browser: spaces and quotes of the address are encoded, the address is quoted
            let arg = format!("\"{}\"", target.replace('"', "%22").replace(' ', "%20"));
            match shell_execute(exe, Some(&arg)) {
                Ok(()) => return Ok(()),
                Err(e) => log::warn!("Browser {} did not start ({}), the default browser is used", exe, e),
            }
        }
    }
    shell_execute(target, None)
}

#[cfg(target_os = "windows")]
fn shell_execute(target: &str, params: Option<&str>) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::shellapi::{ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SHELLEXECUTEINFOW};
    use winapi::um::winuser::SW_SHOWNORMAL;

    let wide = |s: &str| -> Vec<u16> { std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect() };
    let verb = wide("open");
    let file = wide(target);
    let args = params.map(wide);
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        // NO_UI: no Windows error dialog (it would block Jarvis until someone clicks OK); the error comes back to us
        info.fMask = SEE_MASK_FLAG_NO_UI;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        if let Some(a) = &args {
            info.lpParameters = a.as_ptr();
        }
        info.nShow = SW_SHOWNORMAL;
        if ShellExecuteExW(&mut info) != 0 {
            Ok(())
        } else {
            Err(format!("ShellExecuteEx failed: {}", std::io::Error::last_os_error()))
        }
    }
}

// cmd would read & | < > ^ % ! ( ) " in a file name as its own syntax ("x&calc" would start calc): such a target
// is never given to the "cmd /C start" fallback
pub fn cmd_fallback_allowed(target: &str) -> bool {
    !target.chars().any(|c| matches!(c, '&' | '|' | '<' | '>' | '^' | '%' | '!' | '(' | ')' | '"' | '\n' | '\r'))
}

#[cfg(target_os = "windows")]
fn open_target(target: &str) -> bool {
    match shell_open(target) {
        Ok(()) => true,
        Err(e) => {
            // No second try with "cmd /C start": start uses the same ShellExecute, only with the Windows error
            // dialog ("Windows cannot find …", "find an app for this link") that blocks Jarvis until someone
            // clicks OK and makes a failed launch look like a success. The command says what failed instead.
            log::warn!("[Lua] open '{}' failed: {}", target, e);
            false
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

// A notification from a command (jarvis.system.notify): the notification window of Jarvis, with its settings
// ("do not disturb", sound, voice). It is never spoken here: the command speaks for itself.
// the processes the tests pretend are running: (names now, names that appear after this many checks)
pub static TEST_PROCESSES: once_cell::sync::Lazy<parking_lot::Mutex<Option<(Vec<String>, Vec<String>, u32)>>> = once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(None));

// a process with this file name ("Code.exe") is running (the program really started, not only "ShellExecute said ok")
pub fn process_running(exe: &str) -> bool {
    if let Some((now, later, after)) = TEST_PROCESSES.lock().as_mut() {
        if *after == 0 {
            now.extend(later.drain(..));
        } else {
            *after -= 1;
        }
        return now.iter().any(|n| n.eq_ignore_ascii_case(exe));
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
        use winapi::um::tlhelp32::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE {
            // cannot be known: never reported as "did not start"
            return true;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut found = false;
        if Process32FirstW(snap, &mut e) != 0 {
            loop {
                let len = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                if String::from_utf16_lossy(&e.szExeFile[..len]).eq_ignore_ascii_case(exe) {
                    found = true;
                    break;
                }
                if Process32NextW(snap, &mut e) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snap);
        found
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = exe;
        true
    }
}

#[cfg(test)]
pub static TEST_CLIPBOARD: once_cell::sync::Lazy<parking_lot::Mutex<Option<String>>> = once_cell::sync::Lazy::new(|| parking_lot::Mutex::new(None));

pub fn show_notification(title: &str, message: &str) {
    // the text of a notification is private: only its size goes to the log
    log::info!("[Lua] NOTIFY: {} ({} chars)", title, message.chars().count());
    crate::notify::push("command", title, message, true);
    crate::dialog::mark_notified();
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
                // (a program started and not waited for shares none of our standard handles)
                crate::null_stdio(&mut command);
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

    // jarvis.system.type_text(text, window_class?) -> true | nil, error
    system.set("type_text", lua.create_function(|_, (text, class): (String, Option<String>)| {
        match type_text(&text, class.as_deref()) {
            Ok(()) => Ok((true, None::<String>)),
            Err(e) => Ok((false, Some(e))),
        }
    })?)?;

    // jarvis.system.window_exists(class) -> true when a top-level window of this class is open (Windows)
    system.set("window_exists", lua.create_function(|_, class: String| {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::ffi::OsStrExt;
            let wide: Vec<u16> = std::ffi::OsStr::new(&class).encode_wide().chain(std::iter::once(0)).collect();
            return Ok(unsafe { !winapi::um::winuser::FindWindowW(wide.as_ptr(), std::ptr::null()).is_null() });
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = class;
            Ok(true)
        }
    })?)?;

    // jarvis.system.process_running("TOTALCMD64.EXE") -> true when a process with this file name runs now;
    // jarvis.system.can_check_processes() -> whether that can be known here (Windows; the tests set a list)
    system.set("process_running", lua.create_function(|_, exe: String| Ok(process_running(&exe)))?)?;
    system.set("can_check_processes", lua.create_function(|_, ()| Ok(cfg!(target_os = "windows") || TEST_PROCESSES.lock().is_some()))?)?;

    // jarvis.system.exists(path) -> true when the file / folder / drive exists
    system.set("exists", lua.create_function(|_, path: String| Ok(std::path::Path::new(&path).exists()))?)?;

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

    // jarvis.system.keys("ctrl+t") / jarvis.system.mouse("click" | "double" | "right" | "up" | ..., px)
    // / jarvis.system.window("maximize" | "minimize" | "restore") -> true | false, reason
    // (in the user's window: when the Jarvis window is in front, the window under it gets the focus first)
    fn outcome(o: crate::input::Outcome) -> (bool, Option<String>) {
        match o {
            crate::input::Outcome::Done => (true, None),
            crate::input::Outcome::NoTarget => (false, Some("нет окна, в котором можно выполнить действие: откройте нужную программу".into())),
            crate::input::Outcome::Unknown(k) => (false, Some(format!("неизвестное действие или клавиша: {}", k))),
        }
    }
    system.set("keys", lua.create_function(|_, combo: String| Ok(outcome(crate::input::keys(&combo))))?)?;
    system.set("mouse", lua.create_function(|_, (action, amount): (String, Option<i32>)| Ok(outcome(crate::input::mouse(&action, amount.unwrap_or(0)))))?)?;
    system.set("window", lua.create_function(|_, action: String| Ok(outcome(crate::input::window(&action))))?)?;

    // jarvis.system.clipboard - subtable
    let clipboard = lua.create_table()?;
    
    // jarvis.system.clipboard.get() - always available
    let clipboard_get_fn = lua.create_function(|_, ()| {
        #[cfg(test)]
        if let Some(text) = TEST_CLIPBOARD.lock().clone() {
            return Ok(text);
        }
        #[cfg(target_os = "windows")]
        {
            // native read of the Unicode text: PowerShell output arrives in the OEM code page (garbled Russian)
            Ok(clipboard_text().unwrap_or_default())
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
                set_clipboard_text(&text).map_err(|e| mlua::Error::runtime(e))?;
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

    // jarvis.system.folder("documents" | "desktop" | "downloads" | "pictures" | "videos" | "music") -> path | nil:
    // where Windows really keeps it (it can be moved to another drive or into OneDrive)
    system.set("folder", lua.create_function(|_, name: String| {
        Ok(crate::filesearch::known_folder(&name).map(|p| p.to_string_lossy().to_string()))
    })?)?;
    
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
    if !crate::guard::first(&format!("popup:{}:{}", title, message), crate::guard::NOTIFY_WINDOW) {
        return;
    }
    log::info!("[Lua] POPUP: {} ({} chars)", title, message.chars().count());
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

// ---------------------------------------------------------------------------------------------
// Clipboard text through the Win32 API (UTF-16, no code page problems)
// ---------------------------------------------------------------------------------------------
#[cfg(target_os = "windows")]
pub fn clipboard_text() -> Option<String> {
    use winapi::um::winbase::{GlobalLock, GlobalUnlock};
    use winapi::um::winuser::{CloseClipboard, GetClipboardData, OpenClipboard};
    unsafe {
        // another program may hold the clipboard for a moment
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        if !opened {
            return None;
        }
        let mut result = None;
        let handle = GetClipboardData(13); // CF_UNICODETEXT
        if !handle.is_null() {
            let ptr = GlobalLock(handle) as *const u16;
            if !ptr.is_null() {
                let mut len = 0usize;
                while *ptr.add(len) != 0 && len < 2_000_000 {
                    len += 1;
                }
                result = Some(String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len)));
                GlobalUnlock(handle);
            }
        }
        CloseClipboard();
        result
    }
}

#[cfg(target_os = "windows")]
pub fn set_clipboard_text(text: &str) -> Result<(), String> {
    use winapi::um::winbase::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
    use winapi::um::winuser::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let mut opened = false;
        for _ in 0..10 {
            if OpenClipboard(std::ptr::null_mut()) != 0 {
                opened = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        if !opened {
            return Err("clipboard is busy".to_string());
        }
        EmptyClipboard();
        let mem = GlobalAlloc(GMEM_MOVEABLE, wide.len() * 2);
        if mem.is_null() {
            CloseClipboard();
            return Err("out of memory".to_string());
        }
        let ptr = GlobalLock(mem) as *mut u16;
        std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
        GlobalUnlock(mem);
        let ok = !SetClipboardData(13, mem).is_null();
        CloseClipboard();
        if ok { Ok(()) } else { Err("SetClipboardData failed".to_string()) }
    }
}

// Types text into a window with the keyboard (Unicode, any language). `class`: window class to bring to the front
// first (for example "Notepad"); None = the window that has the focus now (not the Jarvis window).
#[cfg(target_os = "windows")]
pub fn type_text(text: &str, class: Option<&str>) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use winapi::um::processthreadsapi::GetCurrentThreadId;
    use winapi::um::winuser::{
        AttachThreadInput, FindWindowW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId, IsIconic, SendInput,
        SetForegroundWindow, ShowWindow, INPUT, INPUT_KEYBOARD, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, SW_RESTORE, VK_RETURN,
    };

    unsafe {
        let mut hwnd = GetForegroundWindow();
        if let Some(class) = class {
            let wide: Vec<u16> = std::ffi::OsStr::new(class).encode_wide().chain(std::iter::once(0)).collect();
            let found = FindWindowW(wide.as_ptr(), std::ptr::null());
            if found.is_null() {
                return Err(format!("window {} not found", class));
            }
            hwnd = found;
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            // Windows only lets the foreground program take the focus: borrow its input queue for a moment
            let fg = GetForegroundWindow();
            let (cur, fg_thread) = (GetCurrentThreadId(), GetWindowThreadProcessId(fg, std::ptr::null_mut()));
            AttachThreadInput(cur, fg_thread, 1);
            SetForegroundWindow(hwnd);
            AttachThreadInput(cur, fg_thread, 0);
            std::thread::sleep(std::time::Duration::from_millis(150));
        } else {
            let mut buf = [0u16; 256];
            let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), 256);
            if String::from_utf16_lossy(&buf[..n.max(0) as usize]).to_lowercase().contains("jarvis") {
                return Err("the Jarvis window has the focus".to_string());
            }
        }

        let key = |vk: u16, scan: u16, flags: u32| -> INPUT {
            let mut input: INPUT = std::mem::zeroed();
            input.type_ = INPUT_KEYBOARD;
            let ki = input.u.ki_mut();
            ki.wVk = vk;
            ki.wScan = scan;
            ki.dwFlags = flags;
            input
        };
        for unit in text.encode_utf16() {
            let mut inputs = if unit == 10 {
                vec![key(VK_RETURN as u16, 0, 0), key(VK_RETURN as u16, 0, KEYEVENTF_KEYUP)]
            } else {
                vec![key(0, unit, KEYEVENTF_UNICODE), key(0, unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]
            };
            SendInput(inputs.len() as u32, inputs.as_mut_ptr(), std::mem::size_of::<INPUT>() as i32);
            std::thread::sleep(std::time::Duration::from_millis(4));
        }
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn type_text(text: &str, class: Option<&str>) -> Result<(), String> {
    log::info!("[not windows, skipped] type into {:?}: {}", class, text);
    // automated tests read what was "typed" from this file
    if let Ok(path) = std::env::var("JARVIS_TEST_TYPED") {
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(f, "{}\t{}", class.unwrap_or(""), text.replace('\n', "\\n"));
        }
    }
    Ok(())
}
