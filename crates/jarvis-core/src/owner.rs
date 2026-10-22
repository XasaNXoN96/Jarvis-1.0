//! The password of the owner. With a password set, the sensitive actions of Jarvis (sending messages, deleting memory and
//! contacts, changing permissions…) answer «Ты не мой создатель» until the owner unlocks the program in the window. The password is
//! never stored: only a salted PBKDF2-HMAC-SHA256 hash (210 000 rounds). Wrong tries are slowed down and saved, so closing the
//! program does not reset the waiting time. A recovery code (shown once when the password is set) replaces a forgotten password.
//! Nothing here can be bypassed by voice: there is no spoken password.

use crate::store;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const FILE: &str = "owner.json";
// (the tests use a small number: the hash itself is checked against the known vectors)
#[cfg(not(test))]
pub const ROUNDS: u32 = 210_000;
#[cfg(test)]
pub const ROUNDS: u32 = 500;
pub const MIN_LEN: usize = 6;
pub const UNLOCK_SECS: u64 = 900;
const FREE_TRIES: u32 = 5;
pub const DEFAULT_MESSAGE: &str = "Ты не мой создатель.";

#[derive(Serialize, Deserialize, Default, Clone)]
struct State {
    #[serde(default)]
    salt: String,
    #[serde(default)]
    hash: String,
    #[serde(default)]
    rounds: u32,
    #[serde(default)]
    recovery_salt: String,
    #[serde(default)]
    recovery_hash: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    fails: u32,
    // no tries before this time (unix seconds)
    #[serde(default)]
    blocked_until: u64,
}

static UNLOCKED_UNTIL: parking_lot::Mutex<u64> = parking_lot::const_mutex(0);

fn hmac(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let (mut ipad, mut opad) = ([0x36u8; 64], [0x5cu8; 64]);
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut h = Sha256::new();
    h.update(ipad);
    h.update(msg);
    let inner = h.finalize();
    let mut h = Sha256::new();
    h.update(opad);
    h.update(inner);
    h.finalize().into()
}

// PBKDF2-HMAC-SHA256, one 32-byte block
pub fn pbkdf2(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut s = salt.to_vec();
    s.extend_from_slice(&1u32.to_be_bytes());
    let mut u = hmac(password, &s);
    let mut out = u;
    for _ in 1..rounds {
        u = hmac(password, &u);
        for i in 0..32 {
            out[i] ^= u[i];
        }
    }
    out
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{:02x}", x)).collect()
}
fn unhex(s: &str) -> Vec<u8> {
    (0..s.len() / 2).filter_map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()).collect()
}
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}
fn random_hex(n: usize) -> String {
    (0..n).map(|_| format!("{:02x}", rand::random::<u8>())).collect()
}

fn load() -> State {
    store::read::<State>(FILE).0
}
fn save(s: &State) -> Result<(), String> {
    store::write(FILE, s)
}

pub fn is_set() -> bool {
    !load().hash.is_empty()
}

pub fn message() -> String {
    let m = load().message;
    if m.trim().is_empty() { DEFAULT_MESSAGE.to_string() } else { m }
}

pub fn set_message(m: &str) -> Result<(), String> {
    let m = m.trim();
    if m.chars().count() > 120 {
        return Err("Фраза слишком длинная.".into());
    }
    let mut s = load();
    s.message = m.to_string();
    save(&s)
}

// the wait after the wrong tries: 5 free, then 30 s, 60 s, 120 s … up to 15 minutes
pub fn wait_after(fails: u32) -> u64 {
    if fails < FREE_TRIES {
        return 0;
    }
    (30u64 << (fails - FREE_TRIES).min(5)).min(900)
}

fn check(s: &State, pw: &str) -> bool {
    !s.hash.is_empty() && same(&pbkdf2(pw.as_bytes(), &unhex(&s.salt), s.rounds.max(1)), &unhex(&s.hash))
}

// the recovery code of the new password (shown once)
fn hash_into(s: &mut State, pw: &str) -> String {
    s.salt = random_hex(16);
    s.rounds = ROUNDS;
    s.hash = hex(&pbkdf2(pw.as_bytes(), &unhex(&s.salt), ROUNDS));
    let code = random_hex(10).to_uppercase();
    s.recovery_salt = random_hex(16);
    s.recovery_hash = hex(&pbkdf2(code.as_bytes(), &unhex(&s.recovery_salt), ROUNDS));
    s.fails = 0;
    s.blocked_until = 0;
    code
}

// sets or changes the password. `old` is needed when one is set. Returns the recovery code.
pub fn set_password(new: &str, old: &str) -> Result<String, String> {
    if new.chars().count() < MIN_LEN {
        return Err(format!("Пароль короче {} символов.", MIN_LEN));
    }
    let mut s = load();
    if !s.hash.is_empty() {
        verify_inner(&mut s, old)?;
    }
    let code = hash_into(&mut s, new);
    save(&s)?;
    *UNLOCKED_UNTIL.lock() = store::now() + UNLOCK_SECS;
    Ok(code)
}

fn verify_inner(s: &mut State, pw: &str) -> Result<(), String> {
    let now = store::now();
    if s.blocked_until > now {
        return Err(format!("Слишком много попыток. Подождите {} с.", s.blocked_until - now));
    }
    if check(s, pw) {
        s.fails = 0;
        s.blocked_until = 0;
        let _ = save(s);
        return Ok(());
    }
    s.fails += 1;
    s.blocked_until = now + wait_after(s.fails);
    let _ = save(s);
    Err(if s.blocked_until > now { format!("Неверный пароль. Следующая попытка через {} с.", s.blocked_until - now) } else { "Неверный пароль.".to_string() })
}

pub fn unlock(pw: &str) -> Result<(), String> {
    let mut s = load();
    if s.hash.is_empty() {
        return Ok(());
    }
    verify_inner(&mut s, pw)?;
    *UNLOCKED_UNTIL.lock() = store::now() + UNLOCK_SECS;
    Ok(())
}

pub fn lock() {
    *UNLOCKED_UNTIL.lock() = 0;
}

pub fn unlocked() -> bool {
    !is_set() || *UNLOCKED_UNTIL.lock() > store::now()
}

// the check at the beginning of every sensitive handler
pub fn guard() -> Result<(), String> {
    if unlocked() { Ok(()) } else { Err(message()) }
}

pub fn remove_password(pw: &str) -> Result<(), String> {
    let mut s = load();
    verify_inner(&mut s, pw)?;
    s.hash.clear();
    s.salt.clear();
    s.recovery_hash.clear();
    save(&s)
}

// a forgotten password: the recovery code and a new password (a new recovery code is returned)
pub fn recover(code: &str, new: &str) -> Result<String, String> {
    if new.chars().count() < MIN_LEN {
        return Err(format!("Пароль короче {} символов.", MIN_LEN));
    }
    let mut s = load();
    let now = store::now();
    if s.blocked_until > now {
        return Err(format!("Слишком много попыток. Подождите {} с.", s.blocked_until - now));
    }
    let ok = !s.recovery_hash.is_empty() && same(&pbkdf2(code.trim().to_uppercase().as_bytes(), &unhex(&s.recovery_salt), s.rounds.max(1)), &unhex(&s.recovery_hash));
    if !ok {
        s.fails += 1;
        s.blocked_until = now + wait_after(s.fails);
        let _ = save(&s);
        return Err("Код восстановления не подошёл.".into());
    }
    let new_code = hash_into(&mut s, new);
    save(&s)?;
    *UNLOCKED_UNTIL.lock() = now + UNLOCK_SECS;
    Ok(new_code)
}

#[cfg(test)]
pub fn reset_for_tests() {
    *UNLOCKED_UNTIL.lock() = 0;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pbkdf2_matches_the_known_vectors() {
        assert_eq!(hex(&pbkdf2(b"password", b"salt", 1)), "120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b");
        assert_eq!(hex(&pbkdf2(b"password", b"salt", 2)), "ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43");
        assert_eq!(hex(&pbkdf2(b"password", b"salt", 4096)), "c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a");
    }

    #[test]
    fn nothing_is_locked_without_a_password() {
        store::with_temp(|| {
            reset_for_tests();
            assert!(!is_set());
            assert!(guard().is_ok());
        });
    }

    #[test]
    fn the_password_locks_and_unlocks_and_is_not_stored() {
        store::with_temp(|| {
            reset_for_tests();
            assert!(set_password("123", "").is_err());
            let code = set_password("секрет-пароль", "").unwrap();
            assert_eq!(code.len(), 20);
            let file = std::fs::read_to_string(store::file(FILE)).unwrap();
            assert!(!file.contains("секрет-пароль") && !file.contains(&code));
            lock();
            assert_eq!(guard().unwrap_err(), DEFAULT_MESSAGE);
            assert!(unlock("не тот").is_err());
            assert!(guard().is_err());
            unlock("секрет-пароль").unwrap();
            assert!(guard().is_ok());
            set_message("Вы не владелец").unwrap();
            lock();
            assert_eq!(guard().unwrap_err(), "Вы не владелец");
        });
    }

    #[test]
    fn wrong_tries_wait_and_the_wait_survives_a_restart() {
        store::with_temp(|| {
            reset_for_tests();
            set_password("правильный", "").unwrap();
            lock();
            for _ in 0..5 {
                assert!(unlock("нет").is_err());
            }
            // the sixth try is refused without even looking at the password
            let e = unlock("правильный").unwrap_err();
            assert!(e.contains("Подождите") || e.contains("через"), "{}", e);
            // (a restart: the in-memory part is gone, the file still holds the wait)
            reset_for_tests();
            assert!(unlock("правильный").is_err());
            assert_eq!(wait_after(4), 0);
            assert_eq!(wait_after(5), 30);
            assert_eq!(wait_after(7), 120);
            assert_eq!(wait_after(100), 900);
        });
    }

    #[test]
    fn recovery_and_change() {
        store::with_temp(|| {
            reset_for_tests();
            let code = set_password("первый-пароль", "").unwrap();
            assert!(set_password("второй-пароль", "не тот").is_err());
            let code2 = set_password("второй-пароль", "первый-пароль").unwrap();
            assert_ne!(code, code2);
            lock();
            assert!(recover(&code, "третий-пароль").is_err(), "the old recovery code does not work");
            let code3 = recover(&code2.to_lowercase(), "третий-пароль").unwrap();
            assert!(unlocked());
            lock();
            unlock("третий-пароль").unwrap();
            assert!(remove_password("не тот").is_err());
            remove_password("третий-пароль").unwrap();
            assert!(!is_set());
            assert!(guard().is_ok());
            let _ = code3;
        });
    }
}
