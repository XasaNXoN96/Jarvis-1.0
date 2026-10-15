//! The licensing client: the identity of this installation, activation, renewal, the offline grace period, the protection of the clock.
//!
//! The program holds NO secret of the server: it only knows the PUBLIC key (built in, `JARVIS_LICENSE_PUBKEY`) and the address
//! (`JARVIS_LICENSE_URL`). A token is believed only when the server's signature is right and the token is made for THIS installation.
//! The secret of the installation (a seed of a key pair) is sealed with the Windows data protection (secret.rs), so the copy of the
//! folder on another account or computer cannot use it. State is kept in `license.json`, protected by an HMAC made with a key derived
//! from the seed: a hand-made edit (the clock floor, the token) is noticed and the file is dropped, i.e. the program falls back to Demo.
use crate::access::{Denied, LicenseView, Why};
use jarvis_license::{entitlement, installation_id_of, new_installation, normalize_key, parse_public_key, sha256_hex, sign_proof, verify_token, Claims, Entitlement};
use parking_lot::Mutex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const STATE_FILE: &str = "license.json";
// how often the background thread talks to the server (the grace period of the server is 72 hours)
const REFRESH_EVERY: i64 = 6 * 3600;

pub fn build_url() -> Option<&'static str> {
    option_env!("JARVIS_LICENSE_URL").filter(|s| !s.is_empty())
}
fn build_pubkey() -> Option<&'static str> {
    option_env!("JARVIS_LICENSE_PUBKEY").filter(|s| !s.is_empty())
}

// the answer of the server: (http status, json body)
pub type Reply = Result<(u16, Value), String>;
type Transport = Box<dyn Fn(&str, &str, Option<&Value>) -> Reply + Send + Sync>;

struct Setup {
    pubkey: String,
    url: String,
    transport: Option<Transport>,
}

static SETUP: Mutex<Option<Setup>> = Mutex::new(None);

#[derive(Clone, Default)]
struct State {
    seed_sealed: String,
    token: String,
    // the clock floor: the latest moment ever seen (the computer's clock or the server's)
    floor: i64,
    last_refresh: i64,
    last_error: String,
    // the plan name and end as the server last said (for the screen only; the rights come from the token)
    prompt: Option<(u64, String, String)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

#[cfg(test)]
static TEST_CLOCK_SHIFT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn now() -> i64 {
    #[cfg(test)]
    {
        return chrono::Utc::now().timestamp() + TEST_CLOCK_SHIFT.load(std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg(not(test))]
    chrono::Utc::now().timestamp()
}

fn path() -> Option<std::path::PathBuf> {
    crate::APP_CONFIG_DIR.get().map(|d| d.join(STATE_FILE))
}

fn mac_of(seed: &[u8; 32], body: &str) -> String {
    let mut key = Sha256::new();
    key.update(b"jarvis-license-state-key");
    key.update(seed);
    let mut h = Sha256::new();
    h.update(key.finalize());
    h.update(body.as_bytes());
    jarvis_license::hex(&h.finalize())
}

fn seed_of(st: &State) -> Option<[u8; 32]> {
    let hex = crate::secret::open(&st.seed_sealed);
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(out)
}

fn load() -> State {
    let Some(p) = path() else { return State::default() };
    let Ok(text) = std::fs::read_to_string(&p) else { return State::default() };
    let Ok(v) = serde_json::from_str::<Value>(&text) else { return State::default() };
    let g = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let mut st = State { seed_sealed: g("seed"), token: g("token"), floor: v.get("floor").and_then(|x| x.as_i64()).unwrap_or(0), last_refresh: v.get("last_refresh").and_then(|x| x.as_i64()).unwrap_or(0), ..Default::default() };
    // a file that was edited by hand (or made on another account: the seed cannot be opened) is not believed
    let ok = seed_of(&st).map(|seed| mac_of(&seed, &body_of(&st)) == g("mac")).unwrap_or(false);
    if !ok {
        return State { seed_sealed: if seed_of(&st).is_some() { st.seed_sealed.clone() } else { String::new() }, last_error: "state".into(), ..Default::default() };
    }
    st.last_error.clear();
    st
}

fn body_of(st: &State) -> String {
    format!("{}|{}|{}|{}", st.seed_sealed, st.token, st.floor, st.last_refresh)
}

fn save(st: &State) {
    let Some(p) = path() else { return };
    let Some(seed) = seed_of(st) else { return };
    let v = json!({ "seed": st.seed_sealed, "token": st.token, "floor": st.floor, "last_refresh": st.last_refresh, "mac": mac_of(&seed, &body_of(st)) });
    let _ = crate::write_atomic(&p, v.to_string());
}

fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut g = STATE.lock();
    let st = g.get_or_insert_with(load);
    f(st)
}

// the seed of this installation; made at the first need
fn ensure_seed(st: &mut State) -> [u8; 32] {
    if let Some(s) = seed_of(st) {
        return s;
    }
    let (seed, _) = new_installation();
    st.seed_sealed = crate::secret::seal(&seed.iter().map(|b| format!("{:02x}", b)).collect::<String>());
    // a new identity means the old token (made for another one) is of no use
    st.token.clear();
    save(st);
    seed
}

pub fn installation_id() -> String {
    with_state(|st| installation_id_of(&ensure_seed(st)))
}

fn pubkey() -> Option<ed25519_dalek::VerifyingKey> {
    let g = SETUP.lock();
    match &*g {
        Some(s) => parse_public_key(&s.pubkey),
        None => build_pubkey().and_then(parse_public_key),
    }
}

fn claims_of(st: &State) -> Option<Claims> {
    if st.token.is_empty() {
        return None;
    }
    let seed = seed_of(st)?;
    verify_token(&st.token, &pubkey()?, &installation_id_of(&seed)).ok()
}

// the clock floor only goes up; the persisted file keeps it, so turning the clock back (even after a restart) does not help
fn raise_floor(st: &mut State, at: i64) {
    if at > st.floor {
        let big = at - st.floor > 600;
        st.floor = at;
        if big {
            save(st);
        }
    }
}

pub fn view() -> LicenseView {
    let t = now();
    with_state(|st| {
        let Some(c) = claims_of(st) else { return LicenseView::None };
        let e = entitlement(&c, t, st.floor);
        if matches!(e, Entitlement::Premium { .. }) {
            raise_floor(st, t);
        }
        match e {
            Entitlement::Premium { .. } => LicenseView::Premium,
            Entitlement::Expired { .. } => LicenseView::Expired,
            Entitlement::NeedsRefresh => LicenseView::NeedsRefresh,
        }
    })
}

// the last refused command: the window asks for it to show the offer (the command itself is gone, it is never queued)
pub fn note_denied(id: &str, d: &Denied) {
    let why = match d.why {
        Why::NeedsPremium => "premium",
        Why::Expired => "expired",
        Why::NeedsRefresh => "refresh",
        Why::AdminOnly => "admin",
        Why::PolicyBroken => "policy",
    };
    with_state(|st| {
        let seq = st.prompt.as_ref().map(|p| p.0).unwrap_or(0) + 1;
        st.prompt = Some((seq, why.to_string(), id.to_string()));
    });
}

// the last refused command as {seq, why, id}: the window shows the offer once per `seq`. The refused command itself is gone:
// it is never queued and never runs after an activation.
pub fn prompt_json() -> Value {
    with_state(|st| match &st.prompt {
        Some((seq, why, id)) => json!({ "seq": seq, "why": why, "id": id }),
        None => json!({ "seq": 0 }),
    })
}

pub fn status() -> Value {
    let t = now();
    let view = view();
    with_state(|st| {
        let c = claims_of(st);
        let name = match &view {
            LicenseView::Premium => "premium",
            LicenseView::Expired => "expired",
            LicenseView::NeedsRefresh => "needs_refresh",
            LicenseView::None => "demo",
        };
        let mut v = json!({ "state": name, "edition": crate::access::edition_name(), "configured": cfg_ok(), "server": cfg_url(), "last_refresh": st.last_refresh, "installation": installation_short(st) });
        if let Some(c) = c {
            v["plan"] = json!(c.plan);
            v["lifetime"] = json!(c.sub_end.is_none());
            v["sub_end"] = json!(c.sub_end);
            v["days_left"] = json!(c.sub_end.map(|e| ((e - t).max(0) + 86_399) / 86_400));
            v["refresh_by"] = json!(c.refresh_by);
        }
        v
    })
}

fn installation_short(st: &State) -> String {
    seed_of(st).map(|s| installation_id_of(&s).chars().take(8).collect()).unwrap_or_default()
}

fn cfg_ok() -> bool {
    pubkey().is_some() && cfg_url().is_some()
}
fn cfg_url() -> Option<String> {
    let g = SETUP.lock();
    match &*g {
        Some(s) => Some(s.url.clone()),
        None => build_url().map(|s| s.to_string()),
    }
}

// ---- talking to the server ----
fn post(path: &str, body: &Value) -> Reply {
    {
        let g = SETUP.lock();
        if let Some(Setup { transport: Some(tr), .. }) = &*g {
            return tr("POST", path, Some(body));
        }
    }
    http("POST", path, Some(body))
}

fn get(path: &str) -> Reply {
    {
        let g = SETUP.lock();
        if let Some(Setup { transport: Some(tr), .. }) = &*g {
            return tr("GET", path, None);
        }
    }
    http("GET", path, None)
}

#[cfg(feature = "lua")]
fn http(method: &str, path: &str, body: Option<&Value>) -> Reply {
    let base = cfg_url().ok_or("Адрес сервера лицензий не задан в этой сборке.")?;
    if !base.starts_with("https://") && !base.starts_with("http://127.0.0.1") && !base.starts_with("http://localhost") {
        return Err("Адрес сервера лицензий должен начинаться с https://".into());
    }
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(|e| e.to_string())?;
    let url = format!("{}{}", base.trim_end_matches('/'), path);
    let req = if method == "GET" { client.get(&url) } else { client.post(&url).json(body.unwrap_or(&Value::Null)) };
    let resp = req.send().map_err(|_| "Нет связи с сервером лицензий. Проверьте интернет.".to_string())?;
    let status = resp.status().as_u16();
    let v: Value = resp.json().unwrap_or(Value::Null);
    Ok((status, v))
}

#[cfg(not(feature = "lua"))]
fn http(_m: &str, _p: &str, _b: Option<&Value>) -> Reply {
    Err("Сетевая часть не включена в эту сборку.".into())
}

fn explain(code: &str, fallback: &str) -> String {
    match code {
        "UNKNOWN_KEY" => "Ключ не найден. Проверьте, что он введён без ошибок.",
        "KEY_EXPIRED" => "Срок активации этого ключа закончился. Обратитесь к тому, кто выдал ключ.",
        "LICENSE_REVOKED" => "Эта лицензия отозвана.",
        "LICENSE_SUSPENDED" => "Эта лицензия временно заблокирована.",
        "BOUND_TO_OTHER_DEVICE" => "Этот ключ уже активирован на другом компьютере. Один ключ работает только на одной установке.",
        "USE_RENEW" => "Это ключ продления. Введите его в окне продления подписки.",
        "NOT_A_RENEWAL_KEY" => "Это не ключ продления. Новая подписка активируется в окне активации.",
        "RENEWAL_USED" => "Этот ключ продления уже использован.",
        "ALREADY_LIFETIME" => "У вас уже бессрочная лицензия, продлевать нечего.",
        "RATE_LIMITED" => "Слишком много неудачных попыток. Подождите несколько минут.",
        "STALE_PROOF" => "Дата и время на компьютере сильно отличаются от настоящих. Исправьте их и повторите.",
        "SUBSCRIPTION_EXPIRED" => "Подписка закончилась.",
        _ => fallback,
    }
    .to_string()
}

fn failure_text(status: u16, v: &Value) -> String {
    let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("");
    let msg = v.get("message").and_then(|c| c.as_str()).unwrap_or("Сервер лицензий ответил ошибкой.");
    explain(code, &format!("{} ({})", msg, status))
}

fn adopt(token: &str, server_time: Option<i64>) -> Result<(), String> {
    with_state(|st| {
        let seed = ensure_seed(st);
        let key = pubkey().ok_or("В этой сборке нет открытого ключа сервера лицензий.")?;
        let claims = verify_token(token, &key, &installation_id_of(&seed)).map_err(|_| "Ответ сервера не прошёл проверку подписи.".to_string())?;
        st.token = token.to_string();
        st.last_refresh = now();
        let at = server_time.unwrap_or(claims.iat).max(claims.iat);
        st.floor = st.floor.max(at).max(now());
        save(st);
        Ok(())
    })
}

// Activation: ONLINE only. The key is checked by the server; a key that is bound to another installation is refused there.
pub fn activate(key_raw: &str) -> Result<Value, String> {
    let key = normalize_key(key_raw).ok_or("Ключ введён неверно. Он состоит из групп букв и цифр, например XXXX-XXXX-…")?;
    let (seed, inst) = with_state(|st| {
        let s = ensure_seed(st);
        (s, installation_id_of(&s))
    });
    let ts = now();
    let body = json!({ "key": key, "installation": inst, "ts": ts, "sig": sign_proof(&seed, "activate", ts, &sha256_hex(key.as_bytes())) });
    let (status, v) = post("/licenses/activate", &body)?;
    if status != 200 || v.get("ok") != Some(&json!(true)) {
        return Err(failure_text(status, &v));
    }
    adopt(v.get("token").and_then(|t| t.as_str()).unwrap_or(""), v.get("server_time").and_then(|t| t.as_i64()))?;
    Ok(status_with_result(&v))
}

// Renewal: a dedicated renewal key adds its period to the license of this installation.
pub fn renew(key_raw: &str) -> Result<Value, String> {
    let key = normalize_key(key_raw).ok_or("Ключ введён неверно.")?;
    let (seed, inst, lid) = with_state(|st| {
        let s = ensure_seed(st);
        let lid = claims_of(st).map(|c| c.lid);
        (s, installation_id_of(&s), lid)
    });
    let lid = lid.or_else(|| last_lid()).ok_or("Сначала нужно активировать Jarvis Premium: у этой установки нет лицензии.")?;
    let ts = now();
    let body = json!({ "lid": lid, "key": key, "installation": inst, "ts": ts, "sig": sign_proof(&seed, "renew", ts, &format!("{}|{}", lid, sha256_hex(key.as_bytes()))) });
    let (status, v) = post("/licenses/renew", &body)?;
    if status != 200 || v.get("ok") != Some(&json!(true)) {
        return Err(failure_text(status, &v));
    }
    adopt(v.get("token").and_then(|t| t.as_str()).unwrap_or(""), v.get("server_time").and_then(|t| t.as_i64()))?;
    Ok(status_with_result(&v))
}

// the id of the license is read from the (possibly expired) token: an expired token still carries it and is still signed
fn last_lid() -> Option<String> {
    with_state(|st| claims_of(st).map(|c| c.lid))
}

fn status_with_result(v: &Value) -> Value {
    let mut s = status();
    s["result"] = v.get("result").cloned().unwrap_or(Value::Null);
    s
}

// A refresh: the server confirms the license again and sends a new token. Network trouble changes nothing (the grace period runs);
// a revoked / suspended / foreign license is dropped at once: the program becomes Demo.
pub fn refresh() -> Result<(), String> {
    let (seed, inst, lid) = with_state(|st| match claims_of(st) {
        Some(c) => (seed_of(st), installation_id_of(&seed_of(st).unwrap_or([0; 32])), Some(c.lid)),
        None => (None, String::new(), None),
    });
    let (Some(seed), Some(lid)) = (seed, lid) else { return Ok(()) };
    let ts = now();
    let body = json!({ "lid": lid, "installation": inst, "ts": ts, "sig": sign_proof(&seed, "refresh", ts, &lid) });
    let (status, v) = match post("/licenses/refresh", &body) {
        Ok(r) => r,
        Err(e) => {
            with_state(|st| st.last_error = e.clone());
            return Err(e);
        }
    };
    if status == 200 && v.get("ok") == Some(&json!(true)) {
        return adopt(v.get("token").and_then(|t| t.as_str()).unwrap_or(""), v.get("server_time").and_then(|t| t.as_i64()));
    }
    let code = v.get("code").and_then(|c| c.as_str()).unwrap_or("");
    if matches!(code, "LICENSE_REVOKED" | "LICENSE_SUSPENDED" | "NOT_BOUND" | "UNKNOWN_LICENSE") {
        with_state(|st| {
            st.token.clear();
            save(st);
        });
    }
    // an expired subscription: the token stays, and its own `sub_end` already says "expired" (the server is the source of truth for the date)
    if code == "SUBSCRIPTION_EXPIRED" {
        with_state(|st| {
            st.last_refresh = now();
            save(st);
        });
    }
    Err(failure_text(status, &v))
}

// the tariffs and the buying instructions for the renewal window (as the server states them)
pub fn tariffs() -> Result<Value, String> {
    let (status, v) = get("/tariffs")?;
    if status != 200 {
        return Err(failure_text(status, &v));
    }
    Ok(v)
}

// the background: one refresh at the start and then every few hours
pub fn start_background() {
    if crate::access::EDITION != crate::access::Edition::User || !cfg_ok() {
        return;
    }
    std::thread::spawn(|| loop {
        let due = with_state(|st| st.token.is_empty() == false && now() - st.last_refresh >= REFRESH_EVERY || (st.last_refresh == 0 && !st.token.is_empty()));
        if due {
            if let Err(e) = refresh() {
                log::warn!("license refresh: {}", e);
            }
        }
        std::thread::sleep(std::time::Duration::from_secs(900));
    });
}

// ---- the same for the tests: a key, an address and a transport of their own ----
#[doc(hidden)]
pub fn test_setup(pubkey: &str, transport: impl Fn(&str, &str, Option<&Value>) -> Reply + Send + Sync + 'static) {
    *SETUP.lock() = Some(Setup { pubkey: pubkey.to_string(), url: "https://license.test".into(), transport: Some(Box::new(transport)) });
}

#[doc(hidden)]
pub fn test_lock() -> parking_lot::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock()
}

#[doc(hidden)]
pub fn test_reset() {
    *STATE.lock() = Some(State::default());
    *SETUP.lock() = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
    use ed25519_dalek::SigningKey;
    use jarvis_license_server::api::{Api, Request};
    use jarvis_license_server::store::Store;
    use std::collections::HashMap;
    use std::sync::Arc;

    // the licensing state is one global thing: these tests run one after another

    struct Server {
        api: Arc<Api>,
        admin: String,
        pubkey: String,
    }

    fn server() -> Server {
        let signing = SigningKey::generate(&mut rand::rngs::OsRng);
        let store = Store::open_memory(b"pepper-pepper-pepper-pepper-pepper-0123".to_vec(), signing.clone()).unwrap();
        let admin = store.admin_create("owner", now()).unwrap();
        let api = Arc::new(Api::new(store, "info".into()));
        Server { api, admin, pubkey: B64.encode(signing.verifying_key().to_bytes()) }
    }

    impl Server {
        fn call(&self, method: &str, path: &str, body: Option<&Value>, admin: bool) -> Reply {
            let mut headers = HashMap::new();
            if admin {
                headers.insert("authorization".to_string(), format!("Bearer {}", self.admin));
            }
            let t = now();
            let r = self.api.handle(&Request { method: method.into(), path: path.into(), headers, body: body.map(|b| b.to_string()).unwrap_or_default(), ip: "10.0.0.7".into() }, t);
            Ok((r.status, serde_json::from_str(&r.body).unwrap_or(Value::Null)))
        }

        fn key(&self, tariff: &str, purpose: &str) -> String {
            let (st, v) = match self.call("POST", "/admin/licenses", Some(&json!({ "count": 1, "tariff_id": tariff, "purpose": purpose, "note": "client test" })), true).unwrap() {
                r => r,
            };
            assert_eq!(st, 200, "{}", v);
            v["licenses"][0]["key"].as_str().unwrap().to_string()
        }

        fn wire(self: &Arc<Self>) {
            let me = self.clone();
            test_setup(&self.pubkey, move |m, p, b| me.call(m, p, b, false));
        }
    }

    fn fresh(s: &Server) -> Arc<Server> {
        test_reset();
        let s = Arc::new(Server { api: s.api.clone(), admin: s.admin.clone(), pubkey: s.pubkey.clone() });
        s.wire();
        s
    }

    #[test]
    fn demo_until_activated_then_premium_and_the_key_is_one_installation() {
        let _g = test_lock();
        let s = fresh(&server());
        assert_eq!(view(), LicenseView::None, "a fresh installation is Demo");
        let key = s.key("premium-monthly", "NEW");
        assert!(activate("XXXX-not-a-key").is_err());
        assert!(activate(&jarvis_license::generate_key()).unwrap_err().contains("Ключ не найден"));
        assert_eq!(view(), LicenseView::None);
        let st = activate(&key).expect("the first activation works");
        assert_eq!(st["state"], "premium");
        assert_eq!(view(), LicenseView::Premium);
        // the same key on a second installation (a fresh identity) is refused by the server
        test_reset();
        s.wire();
        assert_eq!(view(), LicenseView::None);
        let err = activate(&key).unwrap_err();
        assert!(err.contains("другом компьютере"), "{}", err);
        assert_eq!(view(), LicenseView::None, "a refused activation gives nothing");
    }

    #[test]
    fn a_token_of_another_installation_or_forged_is_not_believed() {
        let _g = test_lock();
        let s = fresh(&server());
        let key = s.key("premium-yearly", "NEW");
        activate(&key).unwrap();
        let good = with_state(|st| st.token.clone());
        // a token cut and pasted: the signature no longer fits
        let (payload, sig) = good.split_once('.').unwrap();
        let mut forged = payload.to_string();
        forged.push('A');
        with_state(|st| st.token = format!("{}.{}", forged, sig));
        assert_eq!(view(), LicenseView::None);
        // a real token of this server, but made for another installation
        let other = fresh(&server());
        let key2 = other.key("premium-yearly", "NEW");
        activate(&key2).unwrap();
        let theirs = with_state(|st| st.token.clone());
        test_reset();
        s.wire();
        with_state(|st| {
            ensure_seed(st);
            st.token = theirs.clone();
        });
        assert_eq!(view(), LicenseView::None, "a token of another installation (or another server) is of no use");
    }

    #[test]
    fn the_grace_period_the_clock_and_the_expiry() {
        let _g = test_lock();
        let s = fresh(&server());
        let key = s.key("premium-monthly", "NEW");
        activate(&key).unwrap();
        let c = with_state(|st| claims_of(st).unwrap());
        // the computer's clock is what the program sees: check the decision the same way view() does
        let floor = with_state(|st| st.floor);
        assert!(matches!(entitlement(&c, c.iat + 3600, floor), Entitlement::Premium { .. }));
        // 72 hours without the server: refresh is needed, then it works again
        assert_eq!(entitlement(&c, c.refresh_by + 1, floor), Entitlement::NeedsRefresh);
        // the clock turned back below the floor does not extend anything
        assert_eq!(entitlement(&c, floor - 3 * 86_400, floor), Entitlement::NeedsRefresh);
        // the end of the subscription
        let end = c.sub_end.unwrap();
        assert!(matches!(entitlement(&c, end + 1, floor), Entitlement::Expired { .. }));
        // the server shows the expired subscription: a refresh keeps the token, and view() says Expired
        TEST_CLOCK_SHIFT.store(40 * 86_400, std::sync::atomic::Ordering::SeqCst);
        let e = refresh().unwrap_err();
        assert!(e.contains("закончилась"), "{}", e);
        assert_eq!(view(), LicenseView::Expired, "the token still carries the end date: Premium is off, the renewal is offered");
        TEST_CLOCK_SHIFT.store(0, std::sync::atomic::Ordering::SeqCst);
    }

    #[test]
    fn a_revoked_license_becomes_demo_at_the_next_refresh_and_a_network_loss_changes_nothing() {
        let _g = test_lock();
        let s = fresh(&server());
        let key = s.key("premium-yearly", "NEW");
        activate(&key).unwrap();
        // no network: the grace period runs, the rights stay
        let me = s.clone();
        test_setup(&s.pubkey, move |_, _, _| {
            let _ = &me;
            Err("Нет связи с сервером лицензий.".to_string())
        });
        assert!(refresh().is_err());
        assert_eq!(view(), LicenseView::Premium);
        // the administrator revokes it
        s.wire();
        let lid = with_state(|st| claims_of(st).unwrap().lid);
        let (st, v) = s.call("POST", &format!("/admin/licenses/{}/revoke", lid), Some(&json!({ "reason": "test" })), true).unwrap();
        assert_eq!(st, 200, "{}", v);
        assert!(refresh().is_err());
        assert_eq!(view(), LicenseView::None, "a revoked license is Demo again");
    }

    #[test]
    fn renewal_with_a_renewal_key_extends_and_a_new_key_cannot_renew() {
        let _g = test_lock();
        let s = fresh(&server());
        let key = s.key("premium-monthly", "NEW");
        activate(&key).unwrap();
        let end1 = with_state(|st| claims_of(st).unwrap().sub_end.unwrap());
        let other_new = s.key("premium-monthly", "NEW");
        assert!(renew(&other_new).unwrap_err().contains("не ключ продления"));
        let rkey = s.key("premium-monthly", "RENEWAL");
        let st = renew(&rkey).expect("renewal works");
        assert_eq!(st["state"], "premium");
        let end2 = with_state(|st| claims_of(st).unwrap().sub_end.unwrap());
        assert!(end2 > end1 + 27 * 86_400, "the month is added to the end of the current one");
        // the same renewal key twice changes nothing and says so
        assert!(renew(&rkey).is_err());
        assert_eq!(with_state(|st| claims_of(st).unwrap().sub_end.unwrap()), end2);
        // without any license there is nothing to renew
        test_reset();
        s.wire();
        assert!(renew(&s.key("premium-monthly", "RENEWAL")).unwrap_err().contains("нет лицензии"));
    }

    #[test]
    fn tariffs_come_from_the_server() {
        let _g = test_lock();
        let s = fresh(&server());
        let v = tariffs().unwrap();
        let names: Vec<String> = v["tariffs"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap_or("").to_string()).collect();
        assert!(names.contains(&"premium-monthly".to_string()) && names.contains(&"premium-yearly".to_string()), "{:?}", names);
        let _ = s;
    }

    #[test]
    fn a_hand_edited_state_file_is_dropped() {
        let _g = test_lock();
        let s = fresh(&server());
        activate(&s.key("premium-yearly", "NEW")).unwrap();
        let seed = with_state(|st| seed_of(st).unwrap());
        let st = with_state(|st| st.clone());
        let good = mac_of(&seed, &body_of(&st));
        let mut edited = st.clone();
        edited.floor -= 10 * 86_400; // somebody lowers the clock floor
        assert_ne!(mac_of(&seed, &body_of(&edited)), good, "an edited field changes the HMAC, the file is not believed");
    }
}
