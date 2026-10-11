// One Jarvis at a time.
//
// Two copies (autostart with Windows + a click on the shortcut) both listened to the microphone, both ran every
// spoken command, both wrote the same conversation files (each question and answer appeared twice), both fired the
// saved timers and both watched notifications. A named mutex of Windows lets only the first copy run; a second
// start opens the window of the running Jarvis and quits.

#[cfg(target_os = "windows")]
pub fn acquire(name: &str) -> bool {
    use winapi::shared::winerror::ERROR_ALREADY_EXISTS;
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::synchapi::CreateMutexW;
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let handle = CreateMutexW(std::ptr::null_mut(), 0, wide.as_ptr());
        if handle.is_null() {
            // the mutex could not be made at all: better one Jarvis too many than none
            warn!("Single instance: CreateMutexW failed ({}), continuing", GetLastError());
            return true;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            winapi::um::handleapi::CloseHandle(handle);
            return false;
        }
        // the handle stays open for the whole life of the process (Windows frees it when the process ends)
        true
    }
}

#[cfg(not(target_os = "windows"))]
pub fn acquire(name: &str) -> bool {
    // elsewhere (development only): a local port held for the life of the process, freed when it ends
    use std::sync::Mutex;
    static HELD: Mutex<Vec<(String, Option<std::net::TcpListener>)>> = Mutex::new(Vec::new());
    let mut held = HELD.lock().unwrap();
    if held.iter().any(|(n, _)| n == name) {
        return false;
    }
    let port = if name == NAME { Some(9715) } else { None };
    let listener = match port {
        Some(p) => match std::net::TcpListener::bind(("127.0.0.1", p)) {
            Ok(l) => Some(l),
            Err(_) => return false,
        },
        None => None,
    };
    held.push((name.to_string(), listener));
    true
}

pub const NAME: &str = "Local\\JarvisVoiceAssistant-com.priler.jarvis";

// `wait`: a restart (tray menu, "Включить голос") starts the new copy before the old one has quit: it waits for it
pub fn acquire_or_wait(name: &str, wait: std::time::Duration) -> bool {
    let until = std::time::Instant::now() + wait;
    loop {
        if acquire(name) {
            return true;
        }
        if std::time::Instant::now() >= until {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_copy_is_refused_while_the_first_runs() {
        let name = format!("Local\\JarvisTest-{}-{}", std::process::id(), rand::random::<u32>());
        assert!(acquire(&name), "the first copy runs");
        assert!(!acquire(&name), "a second copy is refused");
        assert!(!acquire_or_wait(&name, std::time::Duration::from_millis(600)), "and waiting does not help while the first one runs");
    }
}
