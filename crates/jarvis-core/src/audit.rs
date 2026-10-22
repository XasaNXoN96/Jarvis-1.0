//! The journal of critical actions: WHO asked (the door: voice, window, phone, Telegram…), WHAT (the id of a command or of an action,
//! never the words of the user), at what risk, what was decided (allowed / refused / asked), whether it was confirmed, and WHY it was
//! refused. It never stores the text of a phrase, a path, an argument, a password, a key or a token: the `reason` is a short fixed
//! phrase chosen by the code that decides, and everything is cut and cleaned again here. The last 500 entries are kept on this computer.
use crate::origin;
use crate::store;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const FILE: &str = "audit_log.json";
const KEEP: usize = 500;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub at: u64,
    // the door: voice | window | phone | telegram | console | ipc | script | internal | system
    pub door: String,
    // command | policy | owner | pairing | remote | gateway | license
    pub kind: String,
    // the id of a command / an action / an event: a name from the code, not text of the user
    pub subject: String,
    // low | medium | high | "" when it has none
    pub risk: String,
    // allowed | refused | asked
    pub decision: String,
    pub confirmed: bool,
    pub reason: String,
}

static LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub fn door_name() -> String {
    origin::current().map(|s| s.as_str().to_string()).unwrap_or_else(|| "system".to_string())
}

// only letters, digits and a few signs of a name; no spaces, no paths, no quotes
fn name(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':')).take(60).collect()
}

// a reason is one short phrase: cut, without line breaks, and with anything that looks like a secret removed
fn reason_text(s: &str) -> String {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).take(140).collect();
    crate::diag::sanitize(one.trim())
}

pub fn record(kind: &str, subject: &str, risk: &str, decision: &str, confirmed: bool, reason: &str) {
    record_door(&door_name(), kind, subject, risk, decision, confirmed, reason)
}

pub fn record_door(door: &str, kind: &str, subject: &str, risk: &str, decision: &str, confirmed: bool, reason: &str) {
    // (the tests that do not have a folder of their own write nothing: no stray files in the folder of the project)
    #[cfg(test)]
    if store::TEST_DIR.lock().is_none() {
        return;
    }
    let e = Entry { at: store::now(), door: name(door), kind: name(kind), subject: name(subject), risk: name(risk), decision: name(decision), confirmed, reason: reason_text(reason) };
    let _g = LOCK.lock();
    let mut l = store::read::<Vec<Entry>>(FILE).0;
    l.push(e);
    let n = l.len();
    if n > KEEP {
        l.drain(..n - KEEP);
    }
    let _ = store::write(FILE, &l);
}

// the newest first
pub fn list(limit: usize) -> Vec<Entry> {
    let _g = LOCK.lock();
    store::read::<Vec<Entry>>(FILE).0.into_iter().rev().take(limit.min(200)).collect()
}

pub fn json(limit: usize) -> Value {
    json!({ "entries": list(limit) })
}

pub fn clear() -> usize {
    let _g = LOCK.lock();
    let n = store::read::<Vec<Entry>>(FILE).0.len();
    let _ = store::write(FILE, &Vec::<Entry>::new());
    n
}

// at most one entry of this kind per `secs` (a flood of wrong requests is one line a minute, not 500 lines)
pub fn record_rate_limited(key: &str, secs: u64, kind: &str, subject: &str, decision: &str, reason: &str) {
    if crate::guard::first(&format!("audit:{}", key), std::time::Duration::from_secs(secs)) {
        record(kind, subject, "", decision, false, reason);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_has_the_door_the_command_and_the_decision_but_never_the_words() {
        store::with_temp(|| {
            origin::run_as(origin::Source::Phone, || record("command", "assistant_power_off", "high", "asked", false, "ждёт подтверждения"));
            record("owner", "unlock", "", "refused", false, "неверный пароль");
            let l = list(10);
            assert_eq!(l.len(), 2);
            assert_eq!((l[1].door.as_str(), l[1].subject.as_str(), l[1].risk.as_str(), l[1].decision.as_str()), ("phone", "assistant_power_off", "high", "asked"));
            assert_eq!(l[0].door, "system", "outside of a command there is no door");
        });
    }

    #[test]
    fn nothing_that_looks_like_a_secret_or_a_path_gets_in() {
        store::with_temp(|| {
            record("command", "C:\\Users\\Ivan\\secret key.txt", "high", "refused", false, "пароль hunter2 и токен sk-abcdefghijklmnop12345\nвторая строка");
            let e = &list(1)[0];
            assert!(!e.subject.contains('\\') && !e.subject.contains(' ') && !e.subject.contains('/'), "{:?}", e);
            assert!(!e.reason.contains("sk-abcdefghijklmnop12345") && !e.reason.contains('\n'), "{:?}", e);
            let raw = std::fs::read_to_string(store::file(FILE)).unwrap();
            assert!(!raw.contains("sk-abcdefghijklmnop12345"));
        });
    }

    // no line of the program that writes to the log may hand over a password, a key, a token or a recovery code (a static check of every source file)
    #[test]
    fn no_log_line_of_the_program_prints_a_secret() {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().map(|x| x == "rs").unwrap_or(false) {
                    out.push(p);
                }
            }
        }
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let mut files = Vec::new();
        for d in ["src", "../jarvis-app/src", "../jarvis-license/src", "../jarvis-license-server/src"] {
            walk(&root.join(d), &mut files);
        }
        assert!(files.len() > 100, "the sources were not found: {}", files.len());
        let words = ["password", "passwd", "secret", "token", "api_key", "ai_key", "cloud_tts_key", "recovery", "pfx", "signing", "seed", "pepper"];
        let mut bad = Vec::new();
        for f in files {
            let name = f.to_string_lossy().to_string();
            if name.ends_with("tests.rs") || name.contains("/tests/") || name.contains("_tests.rs") || name.ends_with("_app.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&f).unwrap_or_default();
            // the test modules at the end of a file are not the program
            let program = text.split("\n#[cfg(test)]").next().unwrap_or("");
            for (n, line) in program.lines().enumerate() {
                let l = line.trim();
                if l.starts_with("//") || l.contains("// safe-log") {
                    continue;
                }
                let is_log = ["info!(", "warn!(", "error!(", "debug!(", "trace!(", "println!(", "eprintln!("].iter().any(|m| l.contains(m));
                if !is_log {
                    continue;
                }
                // the arguments: what follows the format string
                let args = match l.find("\",") {
                    Some(i) => &l[i + 2..],
                    None => continue,
                };
                let args = args.to_lowercase();
                for w in words {
                    // a whole word or a field of something: `token`, `s.token`, `&token`; not a word inside a longer word like `tokens_total`
                    for (i, _) in args.match_indices(w) {
                        let before = args[..i].chars().last().map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false);
                        let after = args[i + w.len()..].chars().next().map(|c| c.is_alphanumeric() || c == '_').unwrap_or(false);
                        if !before && !after && !args.contains(".len()") && !args.contains("chars().count()") {
                            bad.push(format!("{}:{}: {}", name.rsplit("crates/").next().unwrap_or(&name), n + 1, l));
                        }
                    }
                }
            }
        }
        assert!(bad.is_empty(), "log lines that hand over a secret:\n{}", bad.join("\n"));
    }

    #[test]
    fn the_journal_is_bounded_and_can_be_cleared() {
        store::with_temp(|| {
            for i in 0..(KEEP + 25) {
                record("command", &format!("c{}", i), "low", "allowed", false, "");
            }
            assert_eq!(store::read::<Vec<Entry>>(FILE).0.len(), KEEP);
            assert_eq!(list(1000).len(), 200, "a page is at most 200");
            assert_eq!(clear(), KEEP);
            assert!(list(10).is_empty());
        });
    }
}
