//! Licensing code shared by the license server and the program (Jarvis Admin / Jarvis User).
//!
//! What lives here and what does NOT:
//!  * the format of a license key and how its hash is made (the server stores only hashes);
//!  * the signed license token: the program VERIFIES it with the public key; only the server (feature `server`) SIGNS;
//!  * the proof of an installation (an Ed25519 key pair made on the computer): a request is signed with it, so a copied
//!    installation id alone is not enough;
//!  * the calendar rules of a subscription (calendar months, lifetime);
//!  * the answer of Premium: `entitlement` decides from a verified token, the time and the clock guard.
//! No private key, no admin secret and no password is ever in this crate.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const TOKEN_VERSION: u32 = 1;
pub const AUDIENCE: &str = "jarvis-user";

// ---------------------------------------------------------------------------------------------
// plans
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Period {
    Months(u32),
    Days(u32),
    Lifetime,
}

impl Period {
    // the end of a subscription that starts at `start` (unix seconds): calendar months, days, or never (None)
    pub fn end_from(self, start: i64) -> Option<i64> {
        match self {
            Period::Lifetime => None,
            Period::Days(d) => Some(start + d as i64 * 86_400),
            Period::Months(m) => Some(add_months(start, m)),
        }
    }
}

// calendar months: 31 Jan + 1 month = 28/29 Feb (the last day of the month when the day does not exist)
pub fn add_months(ts: i64, months: u32) -> i64 {
    use chrono::{DateTime, Months, Utc};
    let Some(dt) = DateTime::<Utc>::from_timestamp(ts, 0) else { return ts };
    dt.checked_add_months(Months::new(months)).map(|d| d.timestamp()).unwrap_or(ts)
}

// ---------------------------------------------------------------------------------------------
// the key
// ---------------------------------------------------------------------------------------------
const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"; // no 0 O 1 I
pub const KEY_GROUPS: usize = 7;
pub const KEY_GROUP_LEN: usize = 4; // 28 symbols x 5 bits = 140 bits of entropy

// a new key from the random generator of the operating system: JRV-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX-XXXX
pub fn generate_key() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; KEY_GROUPS * KEY_GROUP_LEN];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let mut out = String::from("JRV");
    for (i, b) in bytes.iter().enumerate() {
        if i % KEY_GROUP_LEN == 0 {
            out.push('-');
        }
        out.push(ALPHABET[(*b & 31) as usize] as char); // 256 is a multiple of 32: no bias
    }
    out
}

// what the user typed -> the canonical key, or None when it cannot be a key (spaces, case and dashes do not matter;
// the letters O and I are read as the digits 0 and 1 are not in the alphabet: they are refused, not guessed)
pub fn normalize_key(raw: &str) -> Option<String> {
    let compact: String = raw.chars().filter(|c| !c.is_whitespace() && *c != '-').collect::<String>().to_uppercase();
    let body = compact.strip_prefix("JRV")?;
    if body.len() != KEY_GROUPS * KEY_GROUP_LEN || !body.bytes().all(|b| ALPHABET.contains(&b)) {
        return None;
    }
    let mut out = String::from("JRV");
    for (i, ch) in body.chars().enumerate() {
        if i % KEY_GROUP_LEN == 0 {
            out.push('-');
        }
        out.push(ch);
    }
    Some(out)
}

// the hash that the server stores: HMAC-SHA256 with a server pepper that lives outside the database
pub fn key_hash(pepper: &[u8], canonical_key: &str) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(pepper).expect("hmac accepts any key length");
    mac.update(canonical_key.as_bytes());
    hex(&mac.finalize().into_bytes())
}

// HMAC-SHA256 (RFC 2104) as lower-case hex: the keyed check of the state file of the program
pub fn hmac_sha256_hex(key: &[u8], msg: &[u8]) -> String {
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(key).expect("hmac accepts any key length");
    mac.update(msg);
    hex(&mac.finalize().into_bytes())
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex(&Sha256::digest(data))
}

// a key as it may be shown in a log: never more than the first group
pub fn key_for_log(canonical_key: &str) -> String {
    format!("{}-…", canonical_key.split('-').take(2).collect::<Vec<_>>().join("-"))
}

// ---------------------------------------------------------------------------------------------
// the token
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Claims {
    pub v: u32,
    pub aud: String,
    // the id of the license (not the key)
    pub lid: String,
    // the tariff: "PREMIUM"
    pub tier: String,
    // the plan name shown to the user ("MONTHLY", "LIFETIME" ...)
    pub plan: String,
    // the installation (base64url of its public key) this token is for
    pub inst: String,
    // issued at (server clock)
    pub iat: i64,
    // the end of the subscription; None = lifetime
    pub sub_end: Option<i64>,
    // the program must have asked the server again before this moment (the offline grace period), server clock
    pub refresh_by: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TokenError {
    Malformed,
    BadSignature,
    WrongAudience,
    WrongVersion,
    WrongInstallation,
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            TokenError::Malformed => "the token is malformed",
            TokenError::BadSignature => "the signature of the token is wrong",
            TokenError::WrongAudience => "the token is for another product",
            TokenError::WrongVersion => "the token has an unknown version",
            TokenError::WrongInstallation => "the token is for another installation",
        })
    }
}

#[cfg(feature = "server")]
pub fn sign_token(signing: &SigningKey, claims: &Claims) -> String {
    let payload = B64.encode(serde_json::to_vec(claims).expect("claims serialize"));
    let sig = signing.sign(payload.as_bytes());
    format!("{}.{}", payload, B64.encode(sig.to_bytes()))
}

pub fn parse_public_key(b64: &str) -> Option<VerifyingKey> {
    let bytes: [u8; 32] = B64.decode(b64.trim()).ok()?.try_into().ok()?;
    VerifyingKey::from_bytes(&bytes).ok()
}

// the signature is checked FIRST, only then the content is read; the installation must be this one
pub fn verify_token(token: &str, server_key: &VerifyingKey, my_installation: &str) -> Result<Claims, TokenError> {
    let claims = verify_token_any(token, server_key)?;
    if claims.inst != my_installation {
        return Err(TokenError::WrongInstallation);
    }
    Ok(claims)
}

// the same without the installation check (the server reads the tokens it signed itself)
pub fn verify_token_any(token: &str, server_key: &VerifyingKey) -> Result<Claims, TokenError> {
    let (payload, sig) = token.split_once('.').ok_or(TokenError::Malformed)?;
    let sig_bytes: [u8; 64] = B64.decode(sig).map_err(|_| TokenError::Malformed)?.try_into().map_err(|_| TokenError::Malformed)?;
    server_key.verify(payload.as_bytes(), &Signature::from_bytes(&sig_bytes)).map_err(|_| TokenError::BadSignature)?;
    let claims: Claims = serde_json::from_slice(&B64.decode(payload).map_err(|_| TokenError::Malformed)?).map_err(|_| TokenError::Malformed)?;
    if claims.v != TOKEN_VERSION {
        return Err(TokenError::WrongVersion);
    }
    if claims.aud != AUDIENCE {
        return Err(TokenError::WrongAudience);
    }
    Ok(claims)
}

// ---------------------------------------------------------------------------------------------
// the installation and its proof
// ---------------------------------------------------------------------------------------------
// a new key pair of an installation: (secret seed, public key as the id of the installation)
pub fn new_installation() -> ([u8; 32], String) {
    let signing = SigningKey::generate(&mut rand::rngs::OsRng);
    (signing.to_bytes(), B64.encode(signing.verifying_key().to_bytes()))
}

pub fn installation_id_of(seed: &[u8; 32]) -> String {
    B64.encode(SigningKey::from_bytes(seed).verifying_key().to_bytes())
}

// what the program signs: the purpose, the time, and the subject (a key hash or a license id): a signature made for one
// purpose is no good for another, and an old one is no good later
pub fn proof_message(purpose: &str, ts: i64, subject: &str) -> Vec<u8> {
    format!("jarvis-license-proof/v1\n{}\n{}\n{}", purpose, ts, subject).into_bytes()
}

pub fn sign_proof(seed: &[u8; 32], purpose: &str, ts: i64, subject: &str) -> String {
    B64.encode(SigningKey::from_bytes(seed).sign(&proof_message(purpose, ts, subject)).to_bytes())
}

pub fn verify_proof(installation_id: &str, purpose: &str, ts: i64, subject: &str, sig_b64: &str) -> bool {
    let Some(key) = parse_public_key(installation_id) else { return false };
    let Ok(sig) = B64.decode(sig_b64).map_err(|_| ()).and_then(|b| <[u8; 64]>::try_from(b).map_err(|_| ())) else { return false };
    key.verify(&proof_message(purpose, ts, subject), &Signature::from_bytes(&sig)).is_ok()
}

// ---------------------------------------------------------------------------------------------
// the right to Premium
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub enum Entitlement {
    // Premium works; `days_left` is None for lifetime
    Premium { plan: String, sub_end: Option<i64> },
    // the subscription is over (the server's date has passed)
    Expired { sub_end: i64 },
    // the program has not been in touch with the server for too long (or the clock was turned back): ask again
    NeedsRefresh,
}

// `now` is the clock of the computer; `clock_floor` is the latest moment the program has ever seen (the server's time or its own
// clock): a clock that went BACK below it is not believed and cannot extend anything
pub fn entitlement(claims: &Claims, now: i64, clock_floor: i64) -> Entitlement {
    const CLOCK_TOLERANCE: i64 = 5 * 60;
    if now + CLOCK_TOLERANCE < clock_floor || now + CLOCK_TOLERANCE < claims.iat {
        return Entitlement::NeedsRefresh;
    }
    if let Some(end) = claims.sub_end {
        if now >= end {
            return Entitlement::Expired { sub_end: end };
        }
    }
    if now >= claims.refresh_by {
        return Entitlement::NeedsRefresh;
    }
    Entitlement::Premium { plan: claims.plan.clone(), sub_end: claims.sub_end }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server_pair() -> (SigningKey, String) {
        let s = SigningKey::generate(&mut rand::rngs::OsRng);
        let p = B64.encode(s.verifying_key().to_bytes());
        (s, p)
    }

    fn claims(inst: &str) -> Claims {
        Claims { v: 1, aud: AUDIENCE.into(), lid: "L1".into(), tier: "PREMIUM".into(), plan: "MONTHLY".into(), inst: inst.into(), iat: 1_000_000, sub_end: Some(1_000_000 + 30 * 86_400), refresh_by: 1_000_000 + 72 * 3600 }
    }

    #[test]
    fn a_key_has_140_bits_the_right_shape_and_is_never_repeated() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..2000 {
            let k = generate_key();
            assert_eq!(k.len(), 3 + KEY_GROUPS * (KEY_GROUP_LEN + 1), "{}", k);
            assert_eq!(normalize_key(&k).as_deref(), Some(k.as_str()), "a generated key reads back");
            assert!(seen.insert(k));
        }
    }

    #[test]
    fn typed_keys_are_read_loosely_but_a_wrong_one_is_refused() {
        let k = generate_key();
        let sloppy = format!("  {}  ", k.to_lowercase().replace('-', " "));
        assert_eq!(normalize_key(&sloppy).as_deref(), Some(k.as_str()));
        assert_eq!(normalize_key("JRV-AAAA"), None);
        assert_eq!(normalize_key(""), None);
        assert_eq!(normalize_key(&k.replace("JRV", "XYZ")), None);
        // the letters that are not in the alphabet (0, O, 1, I) make a key invalid
        let mut bad = k.clone();
        bad.replace_range(4..5, "0");
        assert_eq!(normalize_key(&bad), None);
    }

    #[test]
    fn the_hash_depends_on_the_pepper_and_hides_the_key() {
        let k = generate_key();
        let a = key_hash(b"pepper-one", &k);
        assert_eq!(a, key_hash(b"pepper-one", &k));
        assert_ne!(a, key_hash(b"pepper-two", &k));
        assert_eq!(a.len(), 64);
        assert!(!a.contains(&k.replace('-', "")));
        assert!(!key_for_log(&k).contains(&k[8..]), "a log line never holds the whole key");
    }

    #[cfg(feature = "server")]
    #[test]
    fn a_signed_token_is_accepted_and_every_change_of_it_is_not() {
        let (sk, pk) = server_pair();
        let pubkey = parse_public_key(&pk).unwrap();
        let (_, inst) = new_installation();
        let token = sign_token(&sk, &claims(&inst));
        assert_eq!(verify_token(&token, &pubkey, &inst).unwrap().plan, "MONTHLY");
        // the token of another installation
        let (_, other) = new_installation();
        assert_eq!(verify_token(&token, &pubkey, &other), Err(TokenError::WrongInstallation));
        // the tier raised / the end moved by editing the payload: the signature no longer fits
        let (payload, sig) = token.split_once('.').unwrap();
        let mut c: Claims = serde_json::from_slice(&B64.decode(payload).unwrap()).unwrap();
        c.sub_end = None;
        let forged = format!("{}.{}", B64.encode(serde_json::to_vec(&c).unwrap()), sig);
        assert_eq!(verify_token(&forged, &pubkey, &inst), Err(TokenError::BadSignature));
        // a token signed by someone else
        let (evil, _) = server_pair();
        assert_eq!(verify_token(&sign_token(&evil, &claims(&inst)), &pubkey, &inst), Err(TokenError::BadSignature));
        // garbage
        for bad in ["", "a", "a.b", "...", "e30.e30"] {
            assert!(verify_token(bad, &pubkey, &inst).is_err(), "{}", bad);
        }
        // another product / version
        let mut c2 = claims(&inst);
        c2.aud = "something-else".into();
        assert_eq!(verify_token(&sign_token(&sk, &c2), &pubkey, &inst), Err(TokenError::WrongAudience));
        let mut c3 = claims(&inst);
        c3.v = 9;
        assert_eq!(verify_token(&sign_token(&sk, &c3), &pubkey, &inst), Err(TokenError::WrongVersion));
    }

    #[test]
    fn a_proof_is_for_one_purpose_one_time_one_subject_and_one_installation() {
        let (seed, inst) = new_installation();
        assert_eq!(installation_id_of(&seed), inst);
        let sig = sign_proof(&seed, "activate", 1000, "hash1");
        assert!(verify_proof(&inst, "activate", 1000, "hash1", &sig));
        assert!(!verify_proof(&inst, "refresh", 1000, "hash1", &sig), "another purpose");
        assert!(!verify_proof(&inst, "activate", 1001, "hash1", &sig), "another time");
        assert!(!verify_proof(&inst, "activate", 1000, "hash2", &sig), "another subject");
        let (_, other) = new_installation();
        assert!(!verify_proof(&other, "activate", 1000, "hash1", &sig), "another installation");
        assert!(!verify_proof("not a key", "activate", 1000, "hash1", &sig));
        assert!(!verify_proof(&inst, "activate", 1000, "hash1", "###"));
    }

    #[test]
    fn calendar_months_not_30_days() {
        let jan31 = chrono::DateTime::parse_from_rfc3339("2026-01-31T10:00:00Z").unwrap().timestamp();
        let feb28 = chrono::DateTime::parse_from_rfc3339("2026-02-28T10:00:00Z").unwrap().timestamp();
        assert_eq!(Period::Months(1).end_from(jan31), Some(feb28), "31 January + a month = the last day of February");
        let start = chrono::DateTime::parse_from_rfc3339("2026-10-15T21:00:00Z").unwrap().timestamp();
        let six = chrono::DateTime::parse_from_rfc3339("2027-04-15T21:00:00Z").unwrap().timestamp();
        let twelve = chrono::DateTime::parse_from_rfc3339("2027-10-15T21:00:00Z").unwrap().timestamp();
        assert_eq!(Period::Months(6).end_from(start), Some(six));
        assert_eq!(Period::Months(12).end_from(start), Some(twelve));
        assert_eq!(Period::Days(7).end_from(start), Some(start + 7 * 86_400));
        assert_eq!(Period::Lifetime.end_from(start), None);
    }

    #[test]
    fn premium_ends_with_the_subscription_with_the_grace_period_and_with_a_clock_turned_back() {
        let c = claims("i");
        let t0 = c.iat;
        assert!(matches!(entitlement(&c, t0 + 10, t0), Entitlement::Premium { .. }));
        // the grace period is over: ask the server again
        assert_eq!(entitlement(&c, c.refresh_by, t0), Entitlement::NeedsRefresh);
        // the subscription is over (when both are due, the end of the subscription is what the user is told)
        let end = c.sub_end.unwrap();
        let mut long = c.clone();
        long.refresh_by = end + 10 * 86_400;
        assert_eq!(entitlement(&long, end, t0), Entitlement::Expired { sub_end: end });
        // the clock turned back below what was seen before: not believed
        assert_eq!(entitlement(&c, t0 - 3600, t0 + 10_000), Entitlement::NeedsRefresh);
        assert_eq!(entitlement(&c, t0 - 3600, 0), Entitlement::NeedsRefresh, "before the token was even issued");
        // lifetime: no end of its own, but the refresh is still needed
        let mut life = c.clone();
        life.sub_end = None;
        life.plan = "LIFETIME".into();
        assert!(matches!(entitlement(&life, t0 + 100 * 86_400 - 1 - (100 * 86_400 - 70 * 3600), t0), Entitlement::Premium { sub_end: None, .. }));
        assert_eq!(entitlement(&life, life.refresh_by + 1, t0), Entitlement::NeedsRefresh);
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vector() {
        // RFC 4231, test case 2
        assert_eq!(hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"), "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }
}
