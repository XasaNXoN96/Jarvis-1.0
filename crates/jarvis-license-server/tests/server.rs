//! The behaviour of the license server, through the same entry point the HTTP loop uses (`Api::handle`) with a fixed clock,
//! and through real HTTP on a local port. These tests are the checklist of the licensing: online-only activation, one key =
//! one installation, atomic binding, signed tokens, calendar subscriptions, renewal, revocation, audit.
use ed25519_dalek::SigningKey;
use jarvis_license::{generate_key, new_installation, normalize_key, parse_public_key, sha256_hex, sign_proof, verify_token, verify_token_any, TokenError};
use jarvis_license_server::api::{Api, Request};
use jarvis_license_server::store::Store;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

const T0: i64 = 1_800_000_000; // 2027-01-15 08:00:00 UTC
const DAY: i64 = 86_400;
const PEPPER: &[u8] = b"test-pepper-test-pepper-test-pepper-0123456789";

struct Env {
    api: Api,
    pubkey_b64: String,
    admin: String,
    _dir: tempfile::TempDir,
    db: std::path::PathBuf,
    signing_seed: [u8; 32],
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("ls.sqlite");
    let signing = SigningKey::generate(&mut rand::rngs::OsRng);
    let store = Store::open(&db, PEPPER.to_vec(), signing.clone(), 72, 365).unwrap();
    let admin = store.admin_create("owner", T0).unwrap();
    let pubkey_b64 = {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
        B64.encode(signing.verifying_key().to_bytes())
    };
    Env { api: Api::new(store, "info".into()), pubkey_b64, admin, _dir: dir, db, signing_seed: signing.to_bytes() }
}

impl Env {
    fn another_api(&self) -> Api {
        // a second server process on the same database (its own connection)
        let store = Store::open(&self.db, PEPPER.to_vec(), SigningKey::from_bytes(&self.signing_seed), 72, 365).unwrap();
        Api::new(store, "info".into())
    }

    fn pubkey(&self) -> ed25519_dalek::VerifyingKey {
        parse_public_key(&self.pubkey_b64).unwrap()
    }
}

fn call(api: &Api, method: &str, path: &str, body: Value, auth: Option<&str>, ip: &str, now: i64) -> (u16, Value) {
    let mut headers = HashMap::new();
    if let Some(a) = auth {
        headers.insert("authorization".to_string(), format!("Bearer {}", a));
    }
    let r = api.handle(&Request { method: method.into(), path: path.into(), headers, body: body.to_string(), ip: ip.into() }, now);
    (r.status, serde_json::from_str(&r.body).unwrap_or(Value::Null))
}

fn admin_post(e: &Env, path: &str, body: Value, now: i64) -> (u16, Value) {
    call(&e.api, "POST", path, body, Some(&e.admin), "10.0.0.1", now)
}

fn admin_get(e: &Env, path: &str, now: i64) -> (u16, Value) {
    call(&e.api, "GET", path, Value::Null, Some(&e.admin), "10.0.0.1", now)
}

// issues keys through the admin API; returns (license id, key)
fn issue(e: &Env, tariff: &str, purpose: &str, now: i64) -> (String, String) {
    let (st, v) = admin_post(e, "/admin/licenses", json!({ "count": 1, "tariff_id": tariff, "purpose": purpose, "note": "test" }), now);
    assert_eq!(st, 200, "{}", v);
    let l = &v["licenses"][0];
    (l["id"].as_str().unwrap().to_string(), l["key"].as_str().unwrap().to_string())
}

struct Inst {
    seed: [u8; 32],
    id: String,
}

fn inst() -> Inst {
    let (seed, id) = new_installation();
    Inst { seed, id }
}

impl Inst {
    fn body(&self, purpose: &str, ts: i64, subject: &str, extra: Value) -> Value {
        let mut v = json!({ "installation": self.id, "ts": ts, "sig": sign_proof(&self.seed, purpose, ts, subject) });
        for (k, x) in extra.as_object().unwrap() {
            v[k] = x.clone();
        }
        v
    }

    fn activate(&self, api: &Api, key: &str, ip: &str, now: i64) -> (u16, Value) {
        let canon = normalize_key(key).unwrap_or_else(|| key.to_string());
        call(api, "POST", "/licenses/activate", self.body("activate", now, &sha256_hex(canon.as_bytes()), json!({ "key": key })), None, ip, now)
    }

    fn refresh(&self, api: &Api, lid: &str, now: i64) -> (u16, Value) {
        call(api, "POST", "/licenses/refresh", self.body("refresh", now, lid, json!({ "lid": lid })), None, "9.9.9.9", now)
    }

    fn renew(&self, api: &Api, lid: &str, key: &str, now: i64) -> (u16, Value) {
        let canon = normalize_key(key).unwrap_or_else(|| key.to_string());
        call(api, "POST", "/licenses/renew", self.body("renew", now, &format!("{}|{}", lid, sha256_hex(canon.as_bytes())), json!({ "lid": lid, "key": key })), None, "9.9.9.9", now)
    }
}

fn ts_of(s: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp()
}

// ---------------------------------------------------------------------------------------------
// keys and activation
// ---------------------------------------------------------------------------------------------
#[test]
fn a_valid_key_activates_online_and_the_token_is_signed_for_this_installation_only() {
    let e = env();
    let (lid, key) = issue(&e, "premium-monthly", "NEW", T0);
    let i = inst();
    let (st, v) = i.activate(&e.api, &key, "1.1.1.1", T0 + 10);
    assert_eq!(st, 200, "{}", v);
    let claims = verify_token(v["token"].as_str().unwrap(), &e.pubkey(), &i.id).expect("signed by the server, for this installation");
    assert_eq!((claims.lid.as_str(), claims.tier.as_str()), (lid.as_str(), "PREMIUM"));
    assert_eq!(claims.sub_end, Some(jarvis_license::add_months(T0 + 10, 1)), "a calendar month from the activation, by the SERVER's clock");
    assert_eq!(claims.refresh_by, T0 + 10 + 72 * 3600, "the offline grace period is 72 hours (a server setting)");
    // the token is no good for another installation
    assert_eq!(verify_token(v["token"].as_str().unwrap(), &e.pubkey(), &inst().id), Err(TokenError::WrongInstallation));
    // and the answer holds nothing but what the program needs
    assert!(!v.to_string().contains(&key.replace('-', "")) && !v.to_string().contains(&key));
}

#[test]
fn unknown_wrong_revoked_suspended_and_late_keys_are_refused() {
    let e = env();
    let i = inst();
    for bad in ["", "JRV-AAAA", "not a key", &generate_key()] {
        let (st, v) = i.activate(&e.api, bad, "2.2.2.2", T0);
        assert_eq!(st, 404, "{} {}", bad, v);
        assert_eq!(v["code"], "UNKNOWN_KEY");
    }
    let (id1, k1) = issue(&e, "premium-monthly", "NEW", T0);
    admin_post(&e, &format!("/admin/licenses/{}/revoke", id1), json!({ "reason": "chargeback" }), T0);
    let (st, v) = inst().activate(&e.api, &k1, "3.3.3.3", T0 + 1);
    assert_eq!((st, v["code"].as_str().unwrap()), (403, "LICENSE_REVOKED"));
    let (id2, k2) = issue(&e, "premium-monthly", "NEW", T0);
    admin_post(&e, &format!("/admin/licenses/{}/suspend", id2), json!({ "reason": "check" }), T0);
    assert_eq!(inst().activate(&e.api, &k2, "3.3.3.4", T0 + 1).1["code"], "LICENSE_SUSPENDED");
    // the window for the FIRST activation is over (a key that nobody activated in time)
    let (st, v) = admin_post(&e, "/admin/licenses", json!({ "count": 1, "tariff_id": "premium-monthly", "activate_days": 30 }), T0);
    assert_eq!(st, 200);
    let late = v["licenses"][0]["key"].as_str().unwrap().to_string();
    let (st, v) = inst().activate(&e.api, &late, "3.3.3.5", T0 + 31 * DAY);
    assert_eq!((st, v["code"].as_str().unwrap()), (410, "KEY_EXPIRED"));
    let (_, list) = admin_get(&e, "/admin/licenses?status=EXPIRED", T0 + 31 * DAY);
    assert_eq!(list["licenses"].as_array().unwrap().len(), 1, "an unactivated key past its window is listed as EXPIRED");
    // while it is still inside the window it works
    let (st, v) = admin_post(&e, "/admin/licenses", json!({ "count": 1, "tariff_id": "premium-monthly", "activate_days": 30 }), T0);
    assert_eq!(st, 200);
    let ok = v["licenses"][0]["key"].as_str().unwrap().to_string();
    assert_eq!(inst().activate(&e.api, &ok, "3.3.3.6", T0 + 29 * DAY).0, 200);
}

#[test]
fn one_key_one_installation_the_second_device_is_refused_and_a_repeat_from_the_first_is_harmless() {
    let e = env();
    let (_, key) = issue(&e, "premium-yearly", "NEW", T0);
    let (a, b) = (inst(), inst());
    let (st1, v1) = a.activate(&e.api, &key, "4.4.4.4", T0);
    assert_eq!(st1, 200);
    let (st2, v2) = b.activate(&e.api, &key, "4.4.4.5", T0 + 5);
    assert_eq!((st2, v2["code"].as_str().unwrap()), (409, "BOUND_TO_OTHER_DEVICE"));
    assert!(v2.get("token").is_none(), "no token for the second device");
    // the answer to A was lost: A asks again (a new proof, the same key): the same subscription, not a new period
    let (st3, v3) = a.activate(&e.api, &key, "4.4.4.4", T0 + 3600);
    assert_eq!(st3, 200);
    assert_eq!(v1["sub_end"], v3["sub_end"], "nothing is counted twice");
    assert_eq!(v1["lid"], v3["lid"]);
}

#[test]
fn many_devices_at_once_only_one_wins_even_across_server_processes() {
    let e = env();
    let (_, key) = issue(&e, "premium-monthly", "NEW", T0);
    let apis: Vec<Arc<Api>> = (0..6).map(|_| Arc::new(e.another_api())).collect();
    let key = Arc::new(key);
    let mut handles = Vec::new();
    for n in 0..12 {
        let (api, key) = (apis[n % 6].clone(), key.clone());
        handles.push(std::thread::spawn(move || inst().activate(&api, &key, &format!("5.5.5.{}", n), T0 + 1)));
    }
    let results: Vec<(u16, Value)> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    let ok = results.iter().filter(|r| r.0 == 200).count();
    let refused = results.iter().filter(|r| r.0 == 409 && r.1["code"] == "BOUND_TO_OTHER_DEVICE").count();
    assert_eq!((ok, refused), (1, 11), "{:?}", results.iter().map(|r| (r.0, r.1["code"].clone())).collect::<Vec<_>>());
    let (_, list) = admin_get(&e, "/admin/licenses", T0 + 2);
    assert_eq!(list["licenses"][0]["bound"], true);
}

#[test]
fn a_forged_stale_or_replayed_request_is_refused() {
    let e = env();
    let (_, key) = issue(&e, "premium-monthly", "NEW", T0);
    let i = inst();
    let canon = normalize_key(&key).unwrap();
    let subject = sha256_hex(canon.as_bytes());
    // signed by another installation's key while claiming this one
    let thief = inst();
    let body = json!({ "installation": i.id, "ts": T0, "sig": sign_proof(&thief.seed, "activate", T0, &subject), "key": key });
    assert_eq!(call(&e.api, "POST", "/licenses/activate", body, None, "6.6.6.6", T0).1["code"], "BAD_PROOF");
    // a proof for another purpose
    let body = json!({ "installation": i.id, "ts": T0, "sig": sign_proof(&i.seed, "refresh", T0, &subject), "key": key });
    assert_eq!(call(&e.api, "POST", "/licenses/activate", body, None, "6.6.6.6", T0).1["code"], "BAD_PROOF");
    // the clock of the computer far from the clock of the server
    let body = json!({ "installation": i.id, "ts": T0 - 3600, "sig": sign_proof(&i.seed, "activate", T0 - 3600, &subject), "key": key });
    assert_eq!(call(&e.api, "POST", "/licenses/activate", body, None, "6.6.6.6", T0).1["code"], "STALE_PROOF");
    // the very same signed request twice: the second is a replay
    let body = json!({ "installation": i.id, "ts": T0, "sig": sign_proof(&i.seed, "activate", T0, &subject), "key": key });
    assert_eq!(call(&e.api, "POST", "/licenses/activate", body.clone(), None, "6.6.6.6", T0).0, 200);
    assert_eq!(call(&e.api, "POST", "/licenses/activate", body, None, "6.6.6.6", T0 + 1).1["code"], "REPLAY");
    // a missing installation / garbage
    assert_eq!(call(&e.api, "POST", "/licenses/activate", json!({ "key": key }), None, "6.6.6.6", T0).0, 400);
    assert_eq!(call(&e.api, "POST", "/licenses/activate", json!({ "installation": "xx", "ts": T0, "sig": "yy", "key": key }), None, "6.6.6.7", T0).0, 400);
}

#[test]
fn guessing_keys_is_stopped_per_address_and_for_the_whole_server() {
    let e = env();
    let i = inst();
    let mut last = 0;
    for n in 0..12 {
        last = i.activate(&e.api, &generate_key(), "7.7.7.7", T0 + n).0;
    }
    assert_eq!(last, 429, "after ten wrong keys the address waits");
    // another address is not affected...
    assert_eq!(i.activate(&e.api, &generate_key(), "7.7.7.8", T0 + 20).0, 404);
    // ...and after the window the first address may try again
    assert_eq!(i.activate(&e.api, &generate_key(), "7.7.7.7", T0 + 20 * 60).0, 404);
    // the whole server: many addresses together
    let mut blocked = false;
    for n in 0..400 {
        if i.activate(&e.api, &generate_key(), &format!("8.8.{}.{}", n / 200, n % 200), T0 + 100).0 == 429 {
            blocked = true;
            break;
        }
    }
    assert!(blocked, "a guess spread over many addresses is stopped too");
}

#[test]
fn a_database_that_cannot_write_issues_nothing() {
    let e = env();
    let (_, key) = issue(&e, "premium-monthly", "NEW", T0);
    e.api.store.sabotage_for_tests();
    let (st, v) = inst().activate(&e.api, &key, "9.1.1.1", T0 + 1);
    assert_eq!(st, 500, "{}", v);
    assert!(v.get("token").is_none());
    assert!(!v.to_string().to_lowercase().contains("sqlite"), "the text of a database error does not go to the customer");
}

// ---------------------------------------------------------------------------------------------
// subscriptions, tariffs
// ---------------------------------------------------------------------------------------------
#[test]
fn the_standard_tariffs_have_the_agreed_prices_and_the_periods_are_calendar_ones() {
    let e = env();
    let (st, v) = call(&e.api, "GET", "/tariffs", Value::Null, None, "1.2.3.4", T0);
    assert_eq!(st, 200);
    let prices: HashMap<String, i64> = v["tariffs"].as_array().unwrap().iter().map(|t| (t["id"].as_str().unwrap().to_string(), t["price_cents"].as_i64().unwrap())).collect();
    assert_eq!(prices["premium-monthly"], 500);
    assert_eq!(prices["premium-6-months"], 2500);
    assert_eq!(prices["premium-yearly"], 5000);
    assert_eq!(prices["premium-lifetime"], 0, "the price of Lifetime is set by the administrator");
    let start = ts_of("2027-01-31T10:00:00Z");
    for (tariff, end) in [("premium-monthly", "2027-02-28T10:00:00Z"), ("premium-6-months", "2027-07-31T10:00:00Z"), ("premium-yearly", "2028-01-31T10:00:00Z")] {
        let (_, key) = issue(&e, tariff, "NEW", start);
        let (st, v) = inst().activate(&e.api, &key, "1.2.3.5", start);
        assert_eq!(st, 200);
        assert_eq!(v["sub_end"].as_i64(), Some(ts_of(end)), "{}", tariff);
    }
    // lifetime: no end of its own
    let (_, key) = issue(&e, "premium-lifetime", "NEW", start);
    let (_, v) = inst().activate(&e.api, &key, "1.2.3.5", start);
    assert!(v["sub_end"].is_null() && v["lifetime"] == true);
    // custom periods
    for (kind, amount, ok) in [("DAYS", 7, true), ("DAYS", 14, true), ("DAYS", 365, true), ("DAYS", 6, false), ("DAYS", 5000, false), ("MONTHS", 3, true), ("MONTHS", 0, false), ("MONTHS", 121, false), ("LIFETIME", 0, true), ("WEEKS", 2, false)] {
        let (st, _) = admin_post(&e, "/admin/licenses", json!({ "count": 1, "custom": { "kind": kind, "amount": amount } }), start);
        assert_eq!(st == 200, ok, "{} {}", kind, amount);
    }
    let (_, key) = {
        let (_, v) = admin_post(&e, "/admin/licenses", json!({ "count": 1, "custom": { "kind": "DAYS", "amount": 14 } }), start);
        ((), v["licenses"][0]["key"].as_str().unwrap().to_string())
    };
    assert_eq!(inst().activate(&e.api, &key, "1.2.3.5", start + 100).1["sub_end"].as_i64(), Some(start + 100 + 14 * DAY));
}

#[test]
fn the_administrator_changes_prices_and_switches_tariffs_and_it_is_audited() {
    let e = env();
    let (st, _) = call(&e.api, "PUT", "/admin/tariffs", json!({ "id": "premium-lifetime", "name": "Premium Lifetime", "kind": "LIFETIME", "amount": 0, "price_cents": 12900, "currency": "USD", "enabled": true, "sort": 4 }), Some(&e.admin), "10.0.0.1", T0);
    assert_eq!(st, 200);
    let (_, v) = call(&e.api, "GET", "/tariffs", Value::Null, None, "1.2.3.4", T0);
    assert!(v["tariffs"].as_array().unwrap().iter().any(|t| t["id"] == "premium-lifetime" && t["price_cents"] == 12900));
    // a disabled tariff is not offered and issues no keys
    call(&e.api, "PUT", "/admin/tariffs", json!({ "id": "premium-6-months", "name": "Premium 6 Months", "kind": "MONTHS", "amount": 6, "price_cents": 2500, "currency": "USD", "enabled": false, "sort": 2 }), Some(&e.admin), "10.0.0.1", T0);
    let (_, v) = call(&e.api, "GET", "/tariffs", Value::Null, None, "1.2.3.4", T0);
    assert!(!v["tariffs"].as_array().unwrap().iter().any(|t| t["id"] == "premium-6-months"));
    assert_eq!(admin_post(&e, "/admin/licenses", json!({ "count": 1, "tariff_id": "premium-6-months" }), T0).0, 409);
    // bad tariffs are refused
    for bad in [json!({ "id": "x y", "kind": "MONTHS", "amount": 1, "price_cents": 1, "currency": "USD" }), json!({ "id": "a", "kind": "MONTHS", "amount": 1, "price_cents": -5, "currency": "USD" }), json!({ "id": "a", "kind": "DAYS", "amount": 3, "price_cents": 5, "currency": "USD" }), json!({ "id": "a", "kind": "MONTHS", "amount": 1, "price_cents": 5, "currency": "usd" })] {
        assert_eq!(call(&e.api, "PUT", "/admin/tariffs", bad, Some(&e.admin), "10.0.0.1", T0).0, 400);
    }
    // change the tariff of a license, with the history
    let (lid, key) = issue(&e, "premium-monthly", "NEW", T0);
    inst().activate(&e.api, &key, "1.2.3.5", T0);
    let (st, v) = admin_post(&e, &format!("/admin/licenses/{}/change-tariff", lid), json!({ "tariff_id": "premium-yearly", "recompute": true }), T0 + DAY);
    assert_eq!(st, 200, "{}", v);
    assert_eq!(v["license"]["plan"], "Premium Yearly");
    assert_eq!(v["license"]["sub_end"].as_i64(), Some(jarvis_license::add_months(T0, 12)));
    let (_, a) = admin_get(&e, &format!("/admin/audit?license={}", lid), T0 + DAY);
    let change = a["audit"].as_array().unwrap().iter().find(|r| r["action"] == "license.change_tariff").expect("audited");
    assert_eq!(change["actor"], "admin:owner");
    assert_eq!(change["before"]["plan"], "Premium Monthly");
    assert_eq!(change["after"]["plan"], "Premium Yearly");
}

// ---------------------------------------------------------------------------------------------
// refresh, revocation, the end of a subscription, moving to another device
// ---------------------------------------------------------------------------------------------
#[test]
fn refresh_follows_the_server_revoked_and_expired_licenses_get_no_token() {
    let e = env();
    let (lid, key) = issue(&e, "premium-monthly", "NEW", T0);
    let i = inst();
    let (_, act) = i.activate(&e.api, &key, "1.1.1.1", T0);
    // a normal refresh
    let (st, v) = i.refresh(&e.api, &lid, T0 + 10 * DAY);
    assert_eq!(st, 200);
    assert!(verify_token(v["token"].as_str().unwrap(), &e.pubkey(), &i.id).is_ok());
    // the subscription ended: no token, the end is told
    let end = act["sub_end"].as_i64().unwrap();
    let (st, v) = i.refresh(&e.api, &lid, end + 5);
    assert_eq!((st, v["code"].as_str().unwrap()), (402, "SUBSCRIPTION_EXPIRED"));
    assert!(v.get("token").is_none());
    let (_, one) = admin_get(&e, &format!("/admin/licenses/{}", lid), end + 5);
    assert_eq!(one["license"]["effective_status"], "EXPIRED");
    // another installation cannot refresh it
    assert_eq!(inst().refresh(&e.api, &lid, T0 + DAY).1["code"], "NOT_BOUND");
    // revoked: no token (also for a lifetime license)
    let (lid2, key2) = issue(&e, "premium-lifetime", "NEW", T0);
    let j = inst();
    j.activate(&e.api, &key2, "1.1.1.2", T0);
    assert_eq!(j.refresh(&e.api, &lid2, T0 + 100 * DAY).0, 200, "lifetime does not expire");
    admin_post(&e, &format!("/admin/licenses/{}/revoke", lid2), json!({ "reason": "fraud" }), T0 + 101 * DAY);
    let (st, v) = j.refresh(&e.api, &lid2, T0 + 102 * DAY);
    assert_eq!((st, v["code"].as_str().unwrap()), (403, "LICENSE_REVOKED"));
    // the validate route says the state without giving a token
    let (lid3, key3) = issue(&e, "premium-monthly", "NEW", T0);
    let k = inst();
    k.activate(&e.api, &key3, "1.1.1.3", T0);
    let (st, v) = call(&e.api, "POST", "/licenses/validate", k.body("validate", T0 + 5, &lid3, json!({ "lid": lid3 })), None, "9.9.9.9", T0 + 5);
    assert_eq!(st, 200);
    assert!(v.get("token").is_none());
}

#[test]
fn a_reset_binding_moves_the_license_to_a_new_device_without_restarting_the_subscription() {
    let e = env();
    let (lid, key) = issue(&e, "premium-6-months", "NEW", T0);
    let (old, new) = (inst(), inst());
    let (_, act) = old.activate(&e.api, &key, "1.1.1.1", T0);
    // the new device cannot take it while the old one holds it
    assert_eq!(new.activate(&e.api, &key, "1.1.1.9", T0 + DAY).0, 409);
    let (st, _) = admin_post(&e, &format!("/admin/licenses/{}/reset-device", lid), json!({ "reason": "new computer" }), T0 + 2 * DAY);
    assert_eq!(st, 200);
    // the old one gets no more confirmations
    assert_eq!(old.refresh(&e.api, &lid, T0 + 2 * DAY + 60).1["code"], "NOT_BOUND");
    // the new one takes the key: the dates are the old ones
    let (st, v) = new.activate(&e.api, &key, "1.1.1.9", T0 + 3 * DAY);
    assert_eq!(st, 200, "{}", v);
    assert_eq!(v["sub_end"], act["sub_end"], "the subscription is not restarted by a transfer");
    // and the old one cannot take it back
    assert_eq!(old.activate(&e.api, &key, "1.1.1.1", T0 + 4 * DAY).1["code"], "BOUND_TO_OTHER_DEVICE");
}

// ---------------------------------------------------------------------------------------------
// renewal
// ---------------------------------------------------------------------------------------------
#[test]
fn a_renewal_key_adds_its_period_to_the_end_of_a_running_subscription_once() {
    let e = env();
    let (lid, key) = issue(&e, "premium-monthly", "NEW", T0);
    let i = inst();
    let (_, act) = i.activate(&e.api, &key, "1.1.1.1", T0);
    let end0 = act["sub_end"].as_i64().unwrap();
    let (_, rkey) = issue(&e, "premium-6-months", "RENEWAL", T0);
    // a renewal key cannot be used as an activation key, and a new key cannot renew
    assert_eq!(inst().activate(&e.api, &rkey, "1.1.1.2", T0).1["code"], "USE_RENEW");
    let (_, newkey) = issue(&e, "premium-monthly", "NEW", T0);
    assert_eq!(i.renew(&e.api, &lid, &newkey, T0 + 10).1["code"], "NOT_A_RENEWAL_KEY");
    // renew: still running, the paid days are kept
    let (st, v) = i.renew(&e.api, &lid, &rkey, T0 + 10 * DAY);
    assert_eq!(st, 200, "{}", v);
    assert_eq!(v["sub_end"].as_i64(), Some(jarvis_license::add_months(end0, 6)));
    // the same installation sends the same key again (a lost answer): nothing is added twice
    let (st, v2) = i.renew(&e.api, &lid, &rkey, T0 + 10 * DAY + 60);
    assert_eq!((st, v2["result"].as_str().unwrap()), (200, "ALREADY_APPLIED"));
    assert_eq!(v2["sub_end"], v["sub_end"]);
    // another installation cannot spend it, and it cannot be spent on another license
    let other = inst();
    let (olid, okey) = issue(&e, "premium-monthly", "NEW", T0);
    other.activate(&e.api, &okey, "1.1.1.3", T0);
    assert_eq!(other.renew(&e.api, &olid, &rkey, T0 + 20 * DAY).1["code"], "RENEWAL_USED");
    // an ended subscription starts the new period NOW, not from the old end
    let (lid2, key2) = issue(&e, "premium-monthly", "NEW", T0);
    let j = inst();
    let (_, a2) = j.activate(&e.api, &key2, "1.1.1.4", T0);
    let ended = a2["sub_end"].as_i64().unwrap() + 40 * DAY;
    let (_, rkey2) = issue(&e, "premium-yearly", "RENEWAL", ended);
    let (st, v) = j.renew(&e.api, &lid2, &rkey2, ended);
    assert_eq!(st, 200);
    assert_eq!(v["sub_end"].as_i64(), Some(jarvis_license::add_months(ended, 12)));
    // after the renewal Premium works again (a refresh gives a token)
    assert_eq!(j.refresh(&e.api, &lid2, ended + 60).0, 200);
}

#[test]
fn renewal_is_refused_for_lifetime_revoked_or_late_cases() {
    let e = env();
    let (lid, key) = issue(&e, "premium-lifetime", "NEW", T0);
    let i = inst();
    i.activate(&e.api, &key, "1.1.1.1", T0);
    let (_, rkey) = issue(&e, "premium-monthly", "RENEWAL", T0);
    assert_eq!(i.renew(&e.api, &lid, &rkey, T0 + 5).1["code"], "ALREADY_LIFETIME");
    let (lid2, key2) = issue(&e, "premium-monthly", "NEW", T0);
    let j = inst();
    j.activate(&e.api, &key2, "1.1.1.2", T0);
    admin_post(&e, &format!("/admin/licenses/{}/revoke", lid2), json!({ "reason": "x" }), T0 + 1);
    assert_eq!(j.renew(&e.api, &lid2, &rkey, T0 + 5).1["code"], "LICENSE_REVOKED");
    // a renewal key past its own window
    let (st, v) = admin_post(&e, "/admin/licenses", json!({ "count": 1, "tariff_id": "premium-monthly", "purpose": "RENEWAL", "activate_days": 10 }), T0);
    assert_eq!(st, 200);
    let late = v["licenses"][0]["key"].as_str().unwrap().to_string();
    let (lid3, key3) = issue(&e, "premium-monthly", "NEW", T0);
    let k = inst();
    k.activate(&e.api, &key3, "1.1.1.3", T0);
    assert_eq!(k.renew(&e.api, &lid3, &late, T0 + 11 * DAY).1["code"], "KEY_EXPIRED");
    // a lifetime renewal key makes a license lifetime
    let (_, forever) = issue(&e, "premium-lifetime", "RENEWAL", T0);
    let (st, v) = k.renew(&e.api, &lid3, &forever, T0 + 2 * DAY);
    assert_eq!(st, 200);
    assert!(v["sub_end"].is_null());
}

#[test]
fn manual_extension_deadline_and_start_date_work_and_are_audited() {
    let e = env();
    let (lid, key) = issue(&e, "premium-monthly", "NEW", T0);
    // not activated: nothing to extend; the window for the first activation can be moved
    assert_eq!(admin_post(&e, &format!("/admin/licenses/{}/extend", lid), json!({ "kind": "DAYS", "amount": 7 }), T0).0, 409);
    assert_eq!(admin_post(&e, &format!("/admin/licenses/{}/deadline", lid), json!({ "activate_by": T0 + 400 * DAY }), T0).0, 200);
    let i = inst();
    let (_, act) = i.activate(&e.api, &key, "1.1.1.1", T0 + 300 * DAY);
    let end0 = act["sub_end"].as_i64().unwrap();
    // the deadline of an activated key cannot be moved
    assert_eq!(admin_post(&e, &format!("/admin/licenses/{}/deadline", lid), json!({ "activate_by": T0 }), T0).0, 409);
    // extend by 7 days (the shortest by hand) and refuse less
    let (st, v) = admin_post(&e, &format!("/admin/licenses/{}/extend", lid), json!({ "kind": "DAYS", "amount": 7, "reason": "gift" }), T0 + 301 * DAY);
    assert_eq!(st, 200);
    assert_eq!(v["license"]["sub_end"].as_i64(), Some(end0 + 7 * DAY));
    assert_eq!(admin_post(&e, &format!("/admin/licenses/{}/extend", lid), json!({ "kind": "DAYS", "amount": 3 }), T0 + 301 * DAY).0, 400);
    // a fixed start date in the future: the subscription runs from that date
    let start = T0 + 50 * DAY;
    let (st, v) = admin_post(&e, "/admin/licenses", json!({ "count": 1, "tariff_id": "premium-monthly", "start_not_before": start }), T0);
    assert_eq!(st, 200);
    let key2 = v["licenses"][0]["key"].as_str().unwrap().to_string();
    let (_, a2) = inst().activate(&e.api, &key2, "1.1.1.2", T0 + DAY);
    assert_eq!(a2["sub_end"].as_i64(), Some(jarvis_license::add_months(start, 1)));
    // every one of these is in the log with who and when
    let (_, a) = admin_get(&e, &format!("/admin/audit?license={}", lid), T0 + 302 * DAY);
    let actions: Vec<&str> = a["audit"].as_array().unwrap().iter().map(|r| r["action"].as_str().unwrap()).collect();
    for want in ["license.create", "license.set_deadline", "license.activate", "license.extend"] {
        assert!(actions.contains(&want), "{} is not audited: {:?}", want, actions);
    }
}

// ---------------------------------------------------------------------------------------------
// the administrator door, secrets
// ---------------------------------------------------------------------------------------------
#[test]
fn the_admin_routes_need_an_administrator_and_a_header_or_a_guess_gives_nothing() {
    let e = env();
    for (m, p) in [("GET", "/admin/licenses"), ("POST", "/admin/licenses"), ("GET", "/admin/audit"), ("PUT", "/admin/tariffs"), ("POST", "/admin/licenses/L-1/revoke")] {
        assert_eq!(call(&e.api, m, p, json!({}), None, "1.1.1.1", T0).0, 401, "{} {}", m, p);
    }
    // a header that only says it is an admin
    let mut headers = HashMap::new();
    headers.insert("x-admin".to_string(), "true".to_string());
    headers.insert("is_admin".to_string(), "true".to_string());
    let r = e.api.handle(&Request { method: "GET".into(), path: "/admin/licenses".into(), headers, body: String::new(), ip: "1.1.1.2".into() }, T0);
    assert_eq!(r.status, 401);
    // the hash of a token is not a token; a token of the database is not "jla_" + guess
    assert_eq!(call(&e.api, "GET", "/admin/licenses", Value::Null, Some("jla_0000000000000000000000000000000000000000000000000000000000000000"), "1.1.1.3", T0).0, 401);
    // wrong tries are limited per address
    let mut last = 0;
    for _ in 0..8 {
        last = call(&e.api, "GET", "/admin/licenses", Value::Null, Some("wrong-wrong-wrong-wrong-wrong"), "1.1.1.4", T0).0;
    }
    assert_eq!(last, 429);
    assert_eq!(admin_get(&e, "/admin/licenses", T0).0, 200, "the real token of the same server still works from its own address");
    // a disabled administrator... a second administrator has his own token
    let second = e.api.store.admin_create("second", T0).unwrap();
    assert_eq!(call(&e.api, "GET", "/admin/licenses", Value::Null, Some(&second), "1.1.1.5", T0).0, 200);
    assert_ne!(second, e.admin);
}

#[test]
fn full_keys_and_hashes_are_never_in_lists_logs_or_the_database_text() {
    let e = env();
    let (st, v) = admin_post(&e, "/admin/licenses", json!({ "count": 3, "tariff_id": "premium-monthly", "note": "customer A" }), T0);
    assert_eq!(st, 200);
    let keys: Vec<String> = v["licenses"].as_array().unwrap().iter().map(|l| l["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(keys.len(), 3);
    let mut unique = keys.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), 3);
    for k in &keys {
        let (_, list) = admin_get(&e, "/admin/licenses", T0);
        let (_, audit) = admin_get(&e, "/admin/audit", T0);
        for text in [list.to_string(), audit.to_string()] {
            assert!(!text.contains(k) && !text.contains(&k.replace('-', "")), "a full key is in a list or in the audit");
            assert!(!text.contains(&jarvis_license::key_hash(PEPPER, k)), "a hash is shown");
        }
        // in the database file only the hash
        let raw = std::fs::read(&e.db).unwrap_or_default();
        let wal = std::fs::read(e.db.with_extension("sqlite-wal")).unwrap_or_default();
        let hay = String::from_utf8_lossy(&[raw, wal].concat()).to_string();
        assert!(!hay.contains(k), "the database file holds an open key");
    }
    // a big batch is refused, an empty one too
    assert_eq!(admin_post(&e, "/admin/licenses", json!({ "count": 201, "tariff_id": "premium-monthly" }), T0).0, 400);
    assert_eq!(admin_post(&e, "/admin/licenses", json!({ "count": 0, "tariff_id": "premium-monthly" }), T0).0, 400);
    // the health answer holds no secret
    let (st, h) = call(&e.api, "GET", "/health", Value::Null, None, "1.1.1.1", T0);
    assert_eq!(st, 200);
    assert_eq!(h["ok"], true);
    assert!(!h.to_string().contains("pepper") && !h.to_string().to_lowercase().contains("key"));
}

#[test]
fn list_filters_search_and_the_remaining_days_work() {
    let e = env();
    for note in ["alpha", "beta", "gamma"] {
        admin_post(&e, "/admin/licenses", json!({ "count": 1, "tariff_id": "premium-monthly", "note": note }), T0);
    }
    let (_, v) = admin_get(&e, "/admin/licenses?q=beta", T0);
    assert_eq!(v["licenses"].as_array().unwrap().len(), 1);
    let (_, v) = admin_get(&e, "/admin/licenses?status=CREATED", T0);
    assert_eq!(v["licenses"].as_array().unwrap().len(), 3);
    let (_, v) = admin_get(&e, "/admin/licenses?status=ACTIVE", T0);
    assert_eq!(v["licenses"].as_array().unwrap().len(), 0);
    let (lid, key) = issue(&e, "premium-monthly", "NEW", T0);
    inst().activate(&e.api, &key, "1.1.1.1", T0);
    let (_, one) = admin_get(&e, &format!("/admin/licenses/{}", lid), T0 + 10 * DAY);
    let left = one["license"]["days_left"].as_i64().unwrap();
    assert!((20..=21).contains(&left), "{}", left);
    assert_eq!(admin_get(&e, "/admin/licenses/L-nope", T0).0, 404);
}

// ---------------------------------------------------------------------------------------------
// backup, migrations, real HTTP
// ---------------------------------------------------------------------------------------------
#[test]
fn the_backup_is_a_whole_database_and_a_second_start_does_not_reset_the_tariffs() {
    let e = env();
    let (lid, key) = issue(&e, "premium-monthly", "NEW", T0);
    inst().activate(&e.api, &key, "1.1.1.1", T0);
    call(&e.api, "PUT", "/admin/tariffs", json!({ "id": "premium-monthly", "name": "Premium Monthly", "kind": "MONTHS", "amount": 1, "price_cents": 700, "currency": "USD", "enabled": true, "sort": 1 }), Some(&e.admin), "10.0.0.1", T0);
    let target = e._dir.path().join("backup.sqlite");
    e.api.store.backup_to(&target).unwrap();
    assert!(e.api.store.backup_to(&target).is_err(), "an existing copy is never overwritten");
    // the copy opens as a database with the same data
    let restored = Store::open(&target, PEPPER.to_vec(), SigningKey::from_bytes(&e.signing_seed), 72, 365).unwrap();
    assert_eq!(restored.license(&lid).unwrap().plan_name, "Premium Monthly");
    assert_eq!(restored.schema_version(), 1);
    // the standard tariffs are written only into an empty table: the price changed by the owner stays
    let again = e.another_api();
    let prices: Vec<i64> = again.store.tariffs(false).unwrap().iter().filter(|t| t.id == "premium-monthly").map(|t| t.price_cents).collect();
    assert_eq!(prices, vec![700]);
}

fn http(addr: &str, method: &str, path: &str, body: &str, auth: Option<&str>) -> (u16, Value) {
    use std::io::{Read, Write};
    let host = addr.trim_start_matches("http://");
    let mut s = std::net::TcpStream::connect(host).unwrap();
    s.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
    let auth = auth.map(|a| format!("Authorization: Bearer {}\r\n", a)).unwrap_or_default();
    write!(s, "{} {} HTTP/1.1\r\nHost: {}\r\n{}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", method, path, host, auth, body.len(), body).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let status: u16 = out.split_whitespace().nth(1).unwrap().parse().unwrap();
    let json_body = out.split("\r\n\r\n").nth(1).unwrap_or("");
    (status, serde_json::from_str(json_body).unwrap_or(Value::Null))
}

#[test]
fn the_real_http_server_activates_and_refuses_the_same_way() {
    let e = env();
    let (lid, key) = issue(&e, "premium-monthly", "NEW", jarvis_license_server::http::now());
    let admin = e.admin.clone();
    let pubkey = e.pubkey();
    let addr = jarvis_license_server::http::spawn_test_server(Arc::new(e.api));
    // the clock of this machine is the clock of the server here: real time
    let now = jarvis_license_server::http::now();
    let i = inst();
    let canon = normalize_key(&key).unwrap();
    let body = json!({ "installation": i.id, "ts": now, "sig": sign_proof(&i.seed, "activate", now, &sha256_hex(canon.as_bytes())), "key": key });
    let (st, v) = http(&addr, "POST", "/licenses/activate", &body.to_string(), None);
    assert_eq!(st, 200, "{}", v);
    assert!(verify_token(v["token"].as_str().unwrap(), &pubkey, &i.id).is_ok());
    // a second device over real HTTP
    let j = inst();
    let body = json!({ "installation": j.id, "ts": now, "sig": sign_proof(&j.seed, "activate", now, &sha256_hex(canon.as_bytes())), "key": key });
    assert_eq!(http(&addr, "POST", "/licenses/activate", &body.to_string(), None).1["code"], "BOUND_TO_OTHER_DEVICE");
    // admin over HTTP needs the token
    assert_eq!(http(&addr, "GET", "/admin/licenses", "", None).0, 401);
    let (st, v) = http(&addr, "GET", "/admin/licenses", "", Some(&admin));
    assert_eq!(st, 200);
    assert_eq!(v["licenses"][0]["id"], lid);
    assert_eq!(http(&addr, "GET", "/health", "", None).0, 200);
    // a body that is too big
    assert_eq!(http(&addr, "POST", "/licenses/activate", &"x".repeat(20_000), None).0, 413);
    // garbage
    assert_eq!(http(&addr, "POST", "/licenses/activate", "not json", None).0, 400);
    assert_eq!(http(&addr, "GET", "/nothing", "", None).0, 404);
    let _ = verify_token_any;
}
