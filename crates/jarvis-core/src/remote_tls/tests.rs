use super::*;
use crate::store;

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

fn spki_of_cert(der: &[u8]) -> Vec<u8> {
    // the SubjectPublicKeyInfo of the certificate is the public key of the key pair that signed it (self-signed): compare through rcgen's parser
    let (_, cert) = x509_parser_lite::parse(der);
    cert
}

mod x509_parser_lite {
    // a tiny reader: finds the 91-byte P-256 SPKI (30 59 30 13 06 07 2A 86 48 CE 3D 02 01 06 08 2A 86 48 CE 3D 03 01 07 03 42 00 04 ...) inside the DER
    pub fn parse(der: &[u8]) -> ((), Vec<u8>) {
        let head = [0x30u8, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x02, 0x01, 0x06, 0x08, 0x2A, 0x86, 0x48, 0xCE, 0x3D, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00];
        let i = der.windows(head.len()).position(|w| w == head).expect("an EC P-256 public key in the certificate");
        ((), der[i..i + 91].to_vec())
    }
}

#[test]
fn the_key_is_made_once_and_is_the_same_after_a_restart() {
    store::with_temp(|| {
        let first = load_or_create_key().unwrap();
        let pin1 = spki_hash(&first);
        // «restart»: nothing is kept in memory, the key comes from the file
        let second = load_or_create_key().unwrap();
        assert_eq!(spki_hash(&second), pin1, "the pin does not change between runs");
        assert_eq!(first.serialize_der(), second.serialize_der());
        assert!(store::file(KEY_FILE).exists());
        // the pin has the agreed forms
        assert!(pin_text(&pin1).starts_with("sha256/") && pin_text(&pin1).len() == 7 + 43);
        let g = fingerprint_groups(&pin1);
        assert_eq!(g.len(), 6 * 4 + 5, "{}", g);
        assert!(g.split('-').all(|p| p.len() == 4 && p.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())));
    });
}

#[test]
fn a_new_ip_gives_a_new_certificate_with_the_same_key_and_the_same_pin() {
    store::with_temp(|| {
        let now = 1_800_000_000;
        let (a, _) = material(&names(&["192.168.1.5", "localhost"]), now).unwrap();
        let (a2, _) = material(&names(&["192.168.1.5", "localhost"]), now + 100).unwrap();
        assert_eq!(a.cert_der, a2.cert_der, "the same addresses: the saved certificate is served");
        let (b, _) = material(&names(&["10.0.0.7", "localhost"]), now + 200).unwrap();
        assert_ne!(a.cert_der, b.cert_der, "another address: another certificate");
        
        assert!(a.cert_der.windows(4).any(|w| w == [192, 168, 1, 5]) && !a.cert_der.windows(4).any(|w| w == [10, 0, 0, 7]), "the old certificate names the old address");
        assert!(b.cert_der.windows(4).any(|w| w == [10, 0, 0, 7]) && !b.cert_der.windows(4).any(|w| w == [192, 168, 1, 5]), "the new certificate names the new address");
        assert_eq!(a.spki_sha256, b.spki_sha256, "…but the pin of the key is the same");
        assert_eq!(spki_of_cert(&a.cert_der), spki_of_cert(&b.cert_der), "the public key inside both certificates is the same");
        // the pin is exactly the hash of that key
        let h: [u8; 32] = Sha256::digest(spki_of_cert(&b.cert_der)).into();
        assert_eq!(h, b.spki_sha256);
    });
}

#[test]
fn a_certificate_near_its_end_is_issued_again_with_the_same_key() {
    store::with_temp(|| {
        let now = 1_800_000_000;
        let n = names(&["192.168.1.5"]);
        let (a, _) = material(&n, now).unwrap();
        assert!(a.not_after > now + 700 * 24 * 3600, "a term of about two years");
        // 20 days before the end: less than the month, so a new certificate (same key)
        let late = a.not_after - 20 * 24 * 3600;
        let (b, _) = material(&n, late).unwrap();
        assert_ne!(a.cert_der, b.cert_der);
        assert!(b.not_after > late + 700 * 24 * 3600);
        assert_eq!(a.spki_sha256, b.spki_sha256);
        // an EXPIRED certificate is never served again: after the end a new one is made
        let (c, _) = material(&n, b.not_after + 10).unwrap();
        assert!(c.not_after > b.not_after + 10 + 700 * 24 * 3600);
        assert_eq!(a.spki_sha256, c.spki_sha256);
    });
}

#[test]
fn a_clock_a_day_behind_does_not_meet_a_certificate_that_is_not_yet_valid() {
    store::with_temp(|| {
        let now = 1_800_000_000;
        let key = load_or_create_key().unwrap();
        let (_, nb, na) = issue(&names(&["192.168.1.5"]), &key, now).unwrap();
        assert!(nb <= now - 23 * 3600 && nb >= now - 25 * 3600);
        assert!(na > now);
    });
}

#[test]
fn a_damaged_or_foreign_key_file_is_an_error_and_never_a_new_key() {
    store::with_temp(|| {
        let key = load_or_create_key().unwrap();
        let pin = spki_hash(&key);
        let p = store::file(KEY_FILE);
        let good = std::fs::read_to_string(&p).unwrap();
        for bad in ["", "garbage", "dpapi:zz", "plain:zz", "plain:00", &good[..good.len() - 10]] {
            std::fs::write(&p, bad).unwrap();
            let e = load_or_create_key().err().unwrap_or_else(|| panic!("«{}» must be an error", bad));
            assert!(e.contains("Новый ключ не создан"), "{}", e);
            assert_eq!(std::fs::read_to_string(&p).unwrap(), bad, "the damaged file is left as it is, not replaced");
            assert!(material(&names(&["1.2.3.4"]), 1_800_000_000).is_err(), "no certificate without the key");
        }
        // the file comes back: the same pin again
        std::fs::write(&p, &good).unwrap();
        assert_eq!(spki_hash(&load_or_create_key().unwrap()), pin);
        // a key file that is a DIRECTORY or cannot be read is an error as well
        std::fs::remove_file(&p).unwrap();
        std::fs::create_dir(&p).unwrap();
        assert!(load_or_create_key().is_err());
    });
}

#[test]
fn a_key_that_cannot_be_written_is_an_error() {
    store::with_temp(|| {
        // the folder of the program is a FILE: nothing can be created in it
        let dir = store::file("blocked");
        std::fs::write(&dir, b"x").unwrap();
        let target = dir.join("remote_key.sealed");
        assert!(write_private(&target, b"data").is_err());
        assert!(!target.exists());
    });
}

#[cfg(unix)]
#[test]
fn the_key_file_is_private_and_holds_no_readable_key_on_windows_only() {
    use std::os::unix::fs::PermissionsExt;
    store::with_temp(|| {
        load_or_create_key().unwrap();
        let mode = std::fs::metadata(store::file(KEY_FILE)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "only the owner reads the key file");
        // (on Windows the same file holds only a DPAPI blob: see secret::tests; here the development marker is used)
        assert!(std::fs::read_to_string(store::file(KEY_FILE)).unwrap().starts_with("plain:"));
    });
}

#[test]
fn the_old_plain_key_files_are_removed_and_the_old_key_is_never_adopted() {
    store::with_temp(|| {
        std::fs::write(store::file("remote_key.der"), b"old plain key").unwrap();
        std::fs::write(store::file("remote_cert.ip"), b"1.2.3.4").unwrap();
        std::fs::write(store::file(CERT_FILE), b"old certificate").unwrap();
        let (m, warn) = material(&names(&["192.168.1.5"]), 1_800_000_000).unwrap();
        assert!(warn.is_none(), "{:?}", warn);
        assert!(!store::file("remote_key.der").exists() && !store::file("remote_cert.ip").exists());
        assert_ne!(m.cert_der, b"old certificate".to_vec(), "the old certificate (another key) is replaced");
    });
}

#[test]
fn a_key_is_changed_only_on_purpose_and_the_old_one_is_kept_aside() {
    store::with_temp(|| {
        let before = spki_hash(&load_or_create_key().unwrap());
        let after = rotate_key().unwrap();
        assert_ne!(before, after, "an explicit change makes another key");
        assert_eq!(spki_hash(&load_or_create_key().unwrap()), after);
        let kept = std::fs::read_dir(store::file("x").parent().unwrap()).unwrap().filter_map(|e| e.ok()).filter(|e| e.file_name().to_string_lossy().contains(".replaced-")).count();
        assert_eq!(kept, 1, "the old key file is kept, not deleted");
    });
}

#[test]
fn the_served_tls_config_is_built_from_the_stored_identity_and_a_failure_stops_the_remote() {
    store::with_temp(|| {
        let (c1, na1) = super::super::remote::tls_config_for_tests(&names(&["192.168.1.5", "localhost"])).unwrap();
        let (_c2, na2) = super::super::remote::tls_config_for_tests(&names(&["10.1.1.1", "localhost"])).unwrap();
        assert!(na1 > 0 && na2 > 0);
        drop(c1);
        // a broken key file: no configuration, an explicit error
        std::fs::write(store::file(KEY_FILE), "garbage").unwrap();
        let e = super::super::remote::tls_config_for_tests(&names(&["192.168.1.5"])).err().unwrap();
        assert!(e.contains("Новый ключ не создан"), "{}", e);
    });
}

#[test]
fn base64url_matches_the_standard_vectors() {
    // RFC 4648 test vectors (no padding, url alphabet)
    for (raw, enc) in [("", ""), ("f", "Zg"), ("fo", "Zm8"), ("foo", "Zm9v"), ("foob", "Zm9vYg"), ("fooba", "Zm9vYmE"), ("foobar", "Zm9vYmFy")] {
        assert_eq!(b64url_for_tests(raw.as_bytes()), enc);
    }
    assert_eq!(b64url_for_tests(&[0xfb, 0xff, 0xfe]), "-__-", "the url alphabet uses - and _");
}
