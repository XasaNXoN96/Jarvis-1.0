// Who may talk to the local servers of the Jarvis window (the page server 9714 and the event socket 9712)?
//
// Both listen on 127.0.0.1 only, but "only this computer" is not "only the person": a web page in a browser (DNS
// rebinding, a forgotten tab), another Windows user who is signed in, a program that has nothing to do with Jarvis can all
// connect. A Host check and an Origin check stop a page of a web site; they do not stop a program, which sends whatever
// headers it likes. So every request that does anything (reading the state, a phrase, an action, the link of the phone remote)
// must carry a SECRET:
//  - 256 random bits from the system generator, kept in the data folder of the user (`ui_token`, Windows gives that folder to
//    the user alone) and used again until the file is lost, so that the second start of Jarvis and the scripts of the user
//    find it;
//  - the window of Jarvis gets it in the address it opens (after the #, which is never sent over the network), the page keeps
//    it in the memory of its tab and adds it to every request;
//  - compared in constant time; a wrong or missing secret is refused with 403, never answered with data.
// Only the page itself and its icon open without it (the page holds no data and no secret).
//
// If the secret cannot be made (the system generator fails), nothing is allowed: the protection fails closed.
//
// What this does not stop: a program that runs as the SAME Windows user can read the file (and the memory of Jarvis) as well.

use std::path::PathBuf;

use once_cell::sync::Lazy;
use parking_lot::Mutex;

static TOKEN: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[cfg(test)]
static BROKEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn fresh() -> Option<String> {
    #[cfg(test)]
    if BROKEN.load(std::sync::atomic::Ordering::SeqCst) {
        return None;
    }
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.try_fill_bytes(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{:02x}", b)).collect())
}

pub fn valid_shape(t: &str) -> bool {
    t.len() == 64 && t.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn token_file() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(p) = TEST_FILE.lock().clone() {
        return Some(p);
    }
    crate::APP_CONFIG_DIR.get().map(|d| d.join("ui_token"))
}

#[cfg(test)]
pub(crate) static TEST_FILE: Lazy<Mutex<Option<PathBuf>>> = Lazy::new(|| Mutex::new(None));

// made once per run: the token of the file when it is a good one, else a new one (written back)
pub fn init() -> bool {
    let mut slot = TOKEN.lock();
    if slot.is_some() {
        return true;
    }
    let file = token_file();
    if let Some(p) = &file {
        if let Ok(old) = std::fs::read_to_string(p) {
            let old = old.trim().to_string();
            if valid_shape(&old) {
                *slot = Some(old);
                return true;
            }
        }
    }
    let Some(new) = fresh() else {
        warn!("UI: the secret of the window could not be made: the window server refuses every request");
        return false;
    };
    if let Some(p) = &file {
        // (an unwritable folder is not fatal: the secret lives in memory, only scripts outside of Jarvis cannot read it)
        match crate::write_atomic(p, &new) {
            Ok(()) => {
                let _ = crate::acl::private_file(p);
            }
            Err(e) => warn!("UI: the secret of the window could not be saved: {}", e),
        }
    }
    *slot = Some(new);
    true
}

pub fn token() -> Option<String> {
    TOKEN.lock().clone()
}

// the address that opens the window: the secret after the # (the page removes it from the address bar at once)
pub fn with_token(url: &str) -> String {
    match token() {
        Some(t) => format!("{}#t={}", url, t),
        None => url.to_string(),
    }
}

// is this the secret? (no secret made: nothing is)
pub fn check(supplied: Option<&str>) -> bool {
    let Some(expected) = token() else { return false };
    let Some(s) = supplied else { return false };
    let (a, b) = (s.as_bytes(), expected.as_bytes());
    a.len() == b.len() && !b.is_empty() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

// the value of `jt=` in a query string (images and the socket cannot send a header)
pub fn from_query(query: &str) -> Option<String> {
    query.split('&').find_map(|kv| kv.strip_prefix("jt=")).map(|v| v.to_string())
}

// May this connection to the event socket (ws://127.0.0.1:9712/?jt=…) be accepted? The secret decides; an Origin that is not
// the window's is refused even with the secret (a web page never has it, so this is a second lock). No Origin = a program.
pub fn socket_allowed(query: Option<&str>, origin: Option<&str>) -> bool {
    const OWN: &[&str] = &["http://127.0.0.1:9714", "http://localhost:9714", "tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];
    let supplied = query.and_then(from_query);
    if !check(supplied.as_deref()) {
        return false;
    }
    origin.map(|o| OWN.contains(&o)).unwrap_or(true)
}

// the token of the file, for the scripts and the second start of Jarvis (read-only, never created here)
pub fn read_saved() -> Option<String> {
    let t = std::fs::read_to_string(token_file()?).ok()?;
    let t = t.trim().to_string();
    valid_shape(&t).then_some(t)
}

#[cfg(test)]
pub(crate) fn reset_for_tests(file: Option<PathBuf>, broken: bool) {
    *TOKEN.lock() = None;
    *TEST_FILE.lock() = file;
    BROKEN.store(broken, std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_secret_is_long_random_kept_in_a_file_and_reused() {
        let _g = TEST_LOCK.lock();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("ui_token");
        reset_for_tests(Some(file.clone()), false);
        assert!(init());
        let t = token().unwrap();
        assert!(valid_shape(&t));
        assert_eq!(std::fs::read_to_string(&file).unwrap().trim(), t);
        // the next start of Jarvis (a new process: nothing in memory) takes the same one
        *TOKEN.lock() = None;
        assert!(init());
        assert_eq!(token().unwrap(), t);
        // a damaged file is not trusted: a new secret replaces it
        *TOKEN.lock() = None;
        std::fs::write(&file, "short").unwrap();
        assert!(init());
        assert_ne!(token().unwrap(), "short");
        assert!(valid_shape(&token().unwrap()));
        assert_eq!(read_saved(), token());
        // two runs on two computers never share a secret
        reset_for_tests(None, false);
        assert!(init());
        let a = token().unwrap();
        reset_for_tests(None, false);
        assert!(init());
        assert_ne!(a, token().unwrap());
        reset_for_tests(None, false);
    }

    #[test]
    fn a_wrong_missing_or_almost_right_secret_is_not_the_secret() {
        let _g = TEST_LOCK.lock();
        reset_for_tests(None, false);
        init();
        let t = token().unwrap();
        assert!(check(Some(&t)));
        assert!(!check(None));
        assert!(!check(Some("")));
        assert!(!check(Some(&t[..63])));
        assert!(!check(Some(&format!("{}0", t))));
        let mut flipped = t.clone().into_bytes();
        flipped[63] = if flipped[63] == b'0' { b'1' } else { b'0' };
        assert!(!check(Some(&String::from_utf8(flipped).unwrap())));
        assert!(!check(Some(&t.to_uppercase())) || t.to_uppercase() == t);
        reset_for_tests(None, false);
    }

    // the protection that cannot be made lets nothing in
    #[test]
    fn when_the_secret_cannot_be_made_nothing_is_allowed() {
        let _g = TEST_LOCK.lock();
        reset_for_tests(None, true);
        assert!(!init());
        assert_eq!(token(), None);
        assert!(!check(Some("")));
        assert!(!check(Some(&"0".repeat(64))));
        assert!(!check(None));
        assert_eq!(with_token("http://127.0.0.1:9714/"), "http://127.0.0.1:9714/");
        reset_for_tests(None, false);
    }

    #[test]
    fn the_address_of_the_window_carries_the_secret_after_the_hash_and_the_query_form_is_read() {
        let _g = TEST_LOCK.lock();
        reset_for_tests(None, false);
        init();
        let t = token().unwrap();
        assert_eq!(with_token("http://127.0.0.1:9714/"), format!("http://127.0.0.1:9714/#t={}", t));
        assert_eq!(from_query(&format!("x=1&jt={}&y=2", t)), Some(t.clone()));
        assert_eq!(from_query("t=123"), None, "the cache-busting t= of the images is not the secret");
        assert_eq!(from_query(""), None);
        reset_for_tests(None, false);
    }
    // the event socket carries what Jarvis heard and said and takes phrases and «stop»: only for the holder of the secret
    #[test]
    fn the_event_socket_needs_the_secret_whatever_the_origin() {
        let _g = TEST_LOCK.lock();
        reset_for_tests(None, false);
        init();
        let t = token().unwrap();
        // a native program sends no Origin at all: it used to be let in
        assert!(!socket_allowed(None, None));
        assert!(!socket_allowed(Some(""), None));
        assert!(!socket_allowed(Some("jt="), None));
        assert!(!socket_allowed(Some(&format!("jt={}", "0".repeat(64))), None));
        assert!(!socket_allowed(Some("t=123"), None));
        assert!(!socket_allowed(None, Some("http://127.0.0.1:9714")), "the Origin of the window is not the secret");
        // the right secret opens it; a foreign web page is refused even with the secret in its hands
        assert!(socket_allowed(Some(&format!("jt={}", t)), None));
        assert!(socket_allowed(Some(&format!("jt={}", t)), Some("http://127.0.0.1:9714")));
        assert!(!socket_allowed(Some(&format!("jt={}", t)), Some("https://evil.example")));
        // no secret could be made: nobody is let in
        reset_for_tests(None, true);
        assert!(!socket_allowed(None, None));
        assert!(!socket_allowed(Some("jt="), Some("http://127.0.0.1:9714")));
        reset_for_tests(None, false);
    }
}
