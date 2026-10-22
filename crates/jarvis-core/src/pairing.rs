//! Pairing of phones and tablets with a one-time code. The window shows a 6-digit code (valid 5 minutes, five wrong tries burn it); the
//! phone opens the remote page, types the code and receives its OWN secret. Each paired device has a name, a last-seen time and can be revoked
//! alone, without a new link for all the others. Only the hash (SHA-256) of a device secret is stored; the secret itself lives on the phone.
//! The old shared link of the remote keeps working next to this (its «Новая ссылка» revokes that link, not the paired devices).

use crate::store;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const FILE: &str = "paired_devices.json";
pub const CODE_TTL: u64 = 300;
pub const MAX_TRIES: u32 = 5;
pub const MAX_DEVICES: usize = 20;
// last-seen is written at most this often
const SEEN_EVERY: u64 = 60;

#[derive(Serialize, Deserialize, Clone, Debug)]
struct Dev {
    id: String,
    name: String,
    // sha256 of the secret, hex
    hash: String,
    created: u64,
    #[serde(default)]
    last_seen: u64,
    #[serde(default)]
    revoked: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Public {
    pub id: String,
    pub name: String,
    pub created: u64,
    pub last_seen: u64,
    pub revoked: bool,
}

struct Code {
    value: String,
    expires: u64,
    tries: u32,
}
static CODE: Mutex<Option<Code>> = parking_lot::const_mutex(None);

fn hash(secret: &str) -> String {
    Sha256::digest(secret.as_bytes()).iter().map(|b| format!("{:02x}", b)).collect()
}

fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

fn load() -> Vec<Dev> {
    store::read::<Vec<Dev>>(FILE).0
}
fn save(d: &[Dev]) -> Result<(), String> {
    store::write(FILE, &d)
}

fn random_hex(n: usize) -> String {
    use rand::RngCore;
    let mut b = vec![0u8; n];
    rand::rngs::OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

// a new code replaces the old one
pub fn start_code(now: u64) -> String {
    use rand::Rng;
    let v: u32 = rand::rngs::OsRng.gen_range(0..1_000_000);
    let value = format!("{:06}", v);
    *CODE.lock() = Some(Code { value: value.clone(), expires: now + CODE_TTL, tries: 0 });
    value
}

// (the code, seconds left) while a code is valid
pub fn active_code(now: u64) -> Option<(String, u64)> {
    CODE.lock().as_ref().filter(|c| c.expires > now && c.tries < MAX_TRIES).map(|c| (c.value.clone(), c.expires - now))
}

pub fn cancel_code() {
    *CODE.lock() = None;
}

// the phone typed the code: its own secret comes back (once; it is not stored here in clear)
pub fn redeem(code: &str, name: &str, now: u64) -> Result<String, String> {
    let mut g = CODE.lock();
    let Some(c) = g.as_mut() else { return Err("Код не запрошен: нажмите на компьютере «Привязать телефон по коду».".into()) };
    if c.expires <= now {
        *g = None;
        return Err("Код устарел: запросите новый на компьютере.".into());
    }
    if c.tries >= MAX_TRIES {
        *g = None;
        return Err("Слишком много неверных попыток: запросите новый код.".into());
    }
    if !same(code.trim(), &c.value) {
        c.tries += 1;
        if c.tries >= MAX_TRIES {
            *g = None;
        }
        return Err("Неверный код.".into());
    }
    // the code works once
    *g = None;
    drop(g);
    let mut devs = load();
    if devs.iter().filter(|d| !d.revoked).count() >= MAX_DEVICES {
        return Err("Слишком много привязанных устройств: отзовите ненужные.".into());
    }
    let secret = random_hex(32);
    let name: String = name.trim().chars().filter(|c| !c.is_control()).take(40).collect();
    devs.push(Dev { id: random_hex(4), name: if name.is_empty() { "Телефон".into() } else { name }, hash: hash(&secret), created: now, last_seen: now, revoked: false });
    save(&devs)?;
    Ok(secret)
}

// the device that owns this secret (a revoked one is nobody); its last-seen time is refreshed now and then
pub fn check(secret: &str, now: u64) -> Option<String> {
    if secret.len() < 32 || secret.len() > 128 {
        return None;
    }
    let h = hash(secret);
    let mut devs = load();
    let i = devs.iter().position(|d| !d.revoked && same(&d.hash, &h))?;
    let id = devs[i].id.clone();
    if now.saturating_sub(devs[i].last_seen) >= SEEN_EVERY {
        devs[i].last_seen = now;
        let _ = save(&devs);
    }
    Some(id)
}

pub fn list() -> Vec<Public> {
    load().into_iter().map(|d| Public { id: d.id, name: d.name, created: d.created, last_seen: d.last_seen, revoked: d.revoked }).collect()
}

// the device stays in the list as «отозвано» (so that the history is clear); its secret stops working at once
pub fn revoke(id: &str) -> bool {
    let mut devs = load();
    match devs.iter_mut().find(|d| d.id == id && !d.revoked) {
        Some(d) => {
            d.revoked = true;
            save(&devs).is_ok()
        }
        None => false,
    }
}

pub fn forget_revoked() -> bool {
    let mut devs = load();
    devs.retain(|d| !d.revoked);
    save(&devs).is_ok()
}

#[cfg(test)]
pub fn reset_for_tests() {
    cancel_code();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_code_pairs_one_device_once_and_the_secret_is_not_stored_in_clear() {
        store::with_temp(|| {
            reset_for_tests();
            let code = start_code(1000);
            assert_eq!(code.len(), 6);
            let secret = redeem(&code, "Pixel 8", 1010).unwrap();
            assert_eq!(secret.len(), 64);
            assert!(redeem(&code, "Other", 1011).is_err(), "the code works once");
            let file = std::fs::read_to_string(store::file(FILE)).unwrap();
            assert!(!file.contains(&secret) && file.contains("Pixel 8"));
            assert!(check(&secret, 1020).is_some());
            assert!(check("0123456789abcdef0123456789abcdef0123", 1020).is_none());
            assert!(check("short", 1020).is_none());
        });
    }

    #[test]
    fn a_wrong_code_burns_after_five_tries_and_an_old_one_expires() {
        store::with_temp(|| {
            reset_for_tests();
            let code = start_code(0);
            let wrong = if code == "000000" { "111111" } else { "000000" };
            for _ in 0..MAX_TRIES {
                assert!(redeem(wrong, "x", 1).is_err());
            }
            assert!(redeem(&code, "x", 2).unwrap_err().contains("не запрошен"), "the code is burnt");
            let code = start_code(0);
            assert!(redeem(&code, "x", CODE_TTL + 1).unwrap_err().contains("устарел"));
            assert!(list().is_empty());
        });
    }

    #[test]
    fn each_device_is_revoked_alone_and_a_restart_keeps_the_list() {
        store::with_temp(|| {
            reset_for_tests();
            let a = redeem(&start_code(0), "Телефон мамы", 1).unwrap();
            let b = redeem(&start_code(0), "Планшет", 2).unwrap();
            assert_eq!(list().len(), 2);
            let id_a = check(&a, 3).unwrap();
            assert!(revoke(&id_a));
            assert!(check(&a, 4).is_none(), "revoked at once");
            assert!(check(&b, 4).is_some(), "the other device is not touched");
            assert!(!revoke(&id_a), "once");
            reset_for_tests();
            let l = list();
            assert_eq!(l.iter().filter(|d| d.revoked).count(), 1);
            assert!(forget_revoked() && list().len() == 1);
        });
    }

    #[test]
    fn last_seen_is_refreshed_but_not_written_on_every_request() {
        store::with_temp(|| {
            reset_for_tests();
            let s = redeem(&start_code(0), "P", 100).unwrap();
            check(&s, 110);
            assert_eq!(list()[0].last_seen, 100);
            check(&s, 200);
            assert_eq!(list()[0].last_seen, 200);
        });
    }

    #[test]
    fn the_name_is_cleaned_and_the_number_of_devices_is_limited() {
        store::with_temp(|| {
            reset_for_tests();
            redeem(&start_code(0), "  <b>Ph\u{7}one</b>  ", 1).unwrap();
            assert_eq!(list()[0].name, "<b>Phone</b>");
            redeem(&start_code(0), "", 1).unwrap();
            assert_eq!(list()[1].name, "Телефон");
            for _ in 0..MAX_DEVICES - 2 {
                redeem(&start_code(0), "d", 1).unwrap();
            }
            assert!(redeem(&start_code(0), "one more", 1).unwrap_err().contains("Слишком много"));
        });
    }
}
