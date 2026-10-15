//! The database and every rule of licensing. Each operation that changes a license runs in ONE immediate transaction
//! (SQLite takes the write lock at its start), so two parallel requests cannot both bind one key to two installations.
use ed25519_dalek::SigningKey;
use jarvis_license::{generate_key, key_for_log, key_hash, normalize_key, sha256_hex, sign_token, verify_proof, Claims, Period, AUDIENCE, TOKEN_VERSION};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde_json::{json, Value};
use std::path::Path;

pub const PROOF_WINDOW: i64 = 10 * 60;
pub const MIN_CUSTOM_DAYS: i64 = 7; // the shortest subscription an administrator may set by hand: one week
pub const MAX_BATCH: u32 = 200;

#[derive(Debug, Clone)]
pub struct Failure {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}

pub fn fail<T>(status: u16, code: &'static str, message: impl Into<String>) -> Result<T, Failure> {
    Err(Failure { status, code, message: message.into() })
}

impl From<rusqlite::Error> for Failure {
    fn from(e: rusqlite::Error) -> Self {
        // the text of a database error never goes to the customer (it can name tables); it is for the log of the server
        eprintln!("database error: {}", e);
        Failure { status: 500, code: "DB_ERROR", message: "Ошибка сервера. Повторите попытку позже.".into() }
    }
}

#[derive(Clone, Debug)]
pub struct License {
    pub id: String,
    pub key_hint: String,
    pub purpose: String,
    pub tariff_id: Option<String>,
    pub plan_name: String,
    pub period_kind: String,
    pub period_amount: i64,
    pub price_cents: i64,
    pub currency: String,
    pub status: String,
    pub created_at: i64,
    pub activate_by: Option<i64>,
    pub start_not_before: Option<i64>,
    pub note: String,
    pub activated_at: Option<i64>,
    pub sub_start: Option<i64>,
    pub sub_end: Option<i64>,
    pub installation_id: Option<String>,
    pub last_check: Option<i64>,
    pub revoked_at: Option<i64>,
    pub revoke_reason: Option<String>,
    pub renews_license_id: Option<String>,
    pub batch_id: Option<String>,
}

const COLS: &str = "id, key_hint, purpose, tariff_id, plan_name, period_kind, period_amount, price_cents, currency, status, created_at, activate_by, start_not_before, note, activated_at, sub_start, sub_end, installation_id, last_check, revoked_at, revoke_reason, renews_license_id, batch_id";

fn license_of(r: &Row) -> rusqlite::Result<License> {
    Ok(License {
        id: r.get(0)?, key_hint: r.get(1)?, purpose: r.get(2)?, tariff_id: r.get(3)?, plan_name: r.get(4)?, period_kind: r.get(5)?, period_amount: r.get(6)?, price_cents: r.get(7)?,
        currency: r.get(8)?, status: r.get(9)?, created_at: r.get(10)?, activate_by: r.get(11)?, start_not_before: r.get(12)?, note: r.get(13)?, activated_at: r.get(14)?,
        sub_start: r.get(15)?, sub_end: r.get(16)?, installation_id: r.get(17)?, last_check: r.get(18)?, revoked_at: r.get(19)?, revoke_reason: r.get(20)?,
        renews_license_id: r.get(21)?, batch_id: r.get(22)?,
    })
}

impl License {
    pub fn period(&self) -> Period {
        match self.period_kind.as_str() {
            "MONTHS" => Period::Months(self.period_amount.max(0) as u32),
            "DAYS" => Period::Days(self.period_amount.max(0) as u32),
            _ => Period::Lifetime,
        }
    }

    // the status as it is NOW: a key nobody activated in time and a subscription that ran out are EXPIRED
    pub fn effective_status(&self, now: i64) -> &'static str {
        match self.status.as_str() {
            "REVOKED" => "REVOKED",
            "SUSPENDED" => "SUSPENDED",
            "USED" => "USED",
            "CREATED" => if self.activate_by.map(|d| now > d).unwrap_or(false) { "EXPIRED" } else { "CREATED" },
            _ => if self.sub_end.map(|e| now >= e).unwrap_or(false) { "EXPIRED" } else { "ACTIVE" },
        }
    }

    pub fn to_json(&self, now: i64) -> Value {
        json!({
            "id": self.id, "key_hint": self.key_hint, "purpose": self.purpose, "tariff_id": self.tariff_id, "plan": self.plan_name,
            "period_kind": self.period_kind, "period_amount": self.period_amount, "price_cents": self.price_cents, "currency": self.currency,
            "status": self.status, "effective_status": self.effective_status(now), "created_at": self.created_at, "activate_by": self.activate_by,
            "start_not_before": self.start_not_before, "note": self.note, "activated_at": self.activated_at, "sub_start": self.sub_start, "sub_end": self.sub_end,
            "days_left": self.sub_end.map(|e| ((e - now).max(0)) / 86_400), "lifetime": self.period_kind == "LIFETIME",
            "bound": self.installation_id.is_some(), "installation": self.installation_id.as_ref().map(|i| i.chars().take(10).collect::<String>()),
            "last_check": self.last_check, "revoked_at": self.revoked_at, "revoke_reason": self.revoke_reason, "renews_license_id": self.renews_license_id, "batch_id": self.batch_id,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Tariff {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub amount: i64,
    pub price_cents: i64,
    pub currency: String,
    pub enabled: bool,
    pub sort: i64,
}

impl Tariff {
    pub fn to_json(&self) -> Value {
        json!({ "id": self.id, "name": self.name, "kind": self.kind, "amount": self.amount, "price_cents": self.price_cents, "currency": self.currency, "enabled": self.enabled, "sort": self.sort })
    }
}

pub struct Issued {
    pub token: String,
    pub license: License,
    pub note: &'static str,
}

pub struct Store {
    conn: Mutex<Connection>,
    pepper: Vec<u8>,
    signing: SigningKey,
    pub grace_hours: i64,
    pub default_activation_days: i64,
}

const MIGRATIONS: &[&str] = &[
    // v1
    "CREATE TABLE tariffs(id TEXT PRIMARY KEY, name TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('MONTHS','DAYS','LIFETIME')), amount INTEGER NOT NULL DEFAULT 0, price_cents INTEGER NOT NULL DEFAULT 0 CHECK(price_cents >= 0), currency TEXT NOT NULL DEFAULT 'USD', enabled INTEGER NOT NULL DEFAULT 1, sort INTEGER NOT NULL DEFAULT 0);
     CREATE TABLE licenses(id TEXT PRIMARY KEY, key_hash TEXT NOT NULL UNIQUE, key_hint TEXT NOT NULL, purpose TEXT NOT NULL CHECK(purpose IN ('NEW','RENEWAL')),
       tariff_id TEXT, plan_name TEXT NOT NULL, period_kind TEXT NOT NULL CHECK(period_kind IN ('MONTHS','DAYS','LIFETIME')), period_amount INTEGER NOT NULL DEFAULT 0, price_cents INTEGER NOT NULL DEFAULT 0, currency TEXT NOT NULL DEFAULT 'USD',
       status TEXT NOT NULL CHECK(status IN ('CREATED','ACTIVE','REVOKED','EXPIRED','SUSPENDED','USED')), created_at INTEGER NOT NULL, activate_by INTEGER, start_not_before INTEGER, note TEXT NOT NULL DEFAULT '',
       activated_at INTEGER, sub_start INTEGER, sub_end INTEGER, installation_id TEXT, last_check INTEGER, revoked_at INTEGER, revoke_reason TEXT, renews_license_id TEXT, batch_id TEXT);
     CREATE INDEX licenses_status ON licenses(status); CREATE INDEX licenses_installation ON licenses(installation_id);
     CREATE TABLE activations(id INTEGER PRIMARY KEY AUTOINCREMENT, license_id TEXT, installation_id TEXT NOT NULL, ts INTEGER NOT NULL, event TEXT NOT NULL, ok INTEGER NOT NULL, code TEXT);
     CREATE TABLE audit(id INTEGER PRIMARY KEY AUTOINCREMENT, ts INTEGER NOT NULL, actor TEXT NOT NULL, action TEXT NOT NULL, license_id TEXT, before_json TEXT, after_json TEXT, detail TEXT);
     CREATE INDEX audit_license ON audit(license_id);
     CREATE TABLE admin_users(id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, token_hash TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL, disabled INTEGER NOT NULL DEFAULT 0);
     CREATE TABLE seen_proofs(sig_hash TEXT PRIMARY KEY, ts INTEGER NOT NULL);",
];

// the standard tariffs of the first start (the prices are the ones of the owner; they are changed only through the admin API)
const DEFAULT_TARIFFS: &[(&str, &str, &str, i64, i64, i64)] = &[
    ("premium-monthly", "Premium Monthly", "MONTHS", 1, 500, 1),
    ("premium-6-months", "Premium 6 Months", "MONTHS", 6, 2500, 2),
    ("premium-yearly", "Premium Yearly", "MONTHS", 12, 5000, 3),
    ("premium-lifetime", "Premium Lifetime", "LIFETIME", 0, 0, 4), // the price is set by the administrator
];

impl Store {
    pub fn open(path: &Path, pepper: Vec<u8>, signing: SigningKey, grace_hours: i64, default_activation_days: i64) -> Result<Store, String> {
        let conn = Connection::open(path).map_err(|e| format!("база {}: {}", path.display(), e))?;
        Self::init(conn, pepper, signing, grace_hours, default_activation_days)
    }

    pub fn open_memory(pepper: Vec<u8>, signing: SigningKey) -> Result<Store, String> {
        Self::init(Connection::open_in_memory().map_err(|e| e.to_string())?, pepper, signing, 72, 365)
    }

    fn init(conn: Connection, pepper: Vec<u8>, signing: SigningKey, grace_hours: i64, default_activation_days: i64) -> Result<Store, String> {
        conn.busy_timeout(std::time::Duration::from_secs(10)).map_err(|e| e.to_string())?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;").map_err(|e| e.to_string())?;
        let store = Store { conn: Mutex::new(conn), pepper, signing, grace_hours, default_activation_days };
        store.migrate()?;
        Ok(store)
    }

    // versioned migrations: each runs once, in a transaction; the version table says where the database is
    fn migrate(&self) -> Result<(), String> {
        let mut conn = self.conn.lock();
        conn.execute_batch("CREATE TABLE IF NOT EXISTS schema_migrations(version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL)").map_err(|e| e.to_string())?;
        let current: i64 = conn.query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        for (i, sql) in MIGRATIONS.iter().enumerate() {
            let v = i as i64 + 1;
            if v <= current {
                continue;
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate).map_err(|e| e.to_string())?;
            tx.execute_batch(sql).map_err(|e| format!("миграция {}: {}", v, e))?;
            tx.execute("INSERT INTO schema_migrations(version, applied_at) VALUES(?1, ?2)", params![v, now_real()]).map_err(|e| e.to_string())?;
            tx.commit().map_err(|e| e.to_string())?;
        }
        // the standard tariffs only into an EMPTY table: a price changed by the owner is never overwritten
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM tariffs", [], |r| r.get(0)).map_err(|e| e.to_string())?;
        if n == 0 {
            for (id, name, kind, amount, price, sort) in DEFAULT_TARIFFS {
                conn.execute("INSERT INTO tariffs(id, name, kind, amount, price_cents, currency, enabled, sort) VALUES(?1,?2,?3,?4,?5,'USD',1,?6)", params![id, name, kind, amount, price, sort]).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    // for the tests of a database that stops accepting writes: nothing may be issued then
    #[doc(hidden)]
    pub fn sabotage_for_tests(&self) {
        let _ = self.conn.lock().execute_batch("PRAGMA query_only=ON;");
    }

    pub fn schema_version(&self) -> i64 {
        self.conn.lock().query_row("SELECT COALESCE(MAX(version), 0) FROM schema_migrations", [], |r| r.get(0)).unwrap_or(0)
    }

    pub fn health(&self) -> bool {
        self.conn.lock().query_row("SELECT 1", [], |r| r.get::<_, i64>(0)).is_ok()
    }

    // a consistent copy of the database into a file (SQLite VACUUM INTO): the backup procedure
    pub fn backup_to(&self, path: &Path) -> Result<(), String> {
        if path.exists() {
            return Err("файл копии уже существует: выберите новое имя".into());
        }
        let p = path.to_str().ok_or("путь копии не в UTF-8")?;
        self.conn.lock().execute("VACUUM INTO ?1", params![p]).map(|_| ()).map_err(|e| e.to_string())
    }

    fn new_id(prefix: &str) -> String {
        use rand::RngCore;
        let mut b = [0u8; 12];
        rand::rngs::OsRng.fill_bytes(&mut b);
        format!("{}-{}", prefix, jarvis_license::hex(&b))
    }

    fn audit(tx: &rusqlite::Transaction, ts: i64, actor: &str, action: &str, license: Option<&str>, before: Option<Value>, after: Option<Value>, detail: &str) -> Result<(), Failure> {
        tx.execute(
            "INSERT INTO audit(ts, actor, action, license_id, before_json, after_json, detail) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            params![ts, actor, action, license, before.map(|v| v.to_string()), after.map(|v| v.to_string()), detail],
        )?;
        Ok(())
    }

    // ---- admin users ----
    pub fn admin_create(&self, name: &str, now: i64) -> Result<String, String> {
        use rand::RngCore;
        let name = name.trim();
        if name.is_empty() || name.len() > 60 {
            return Err("имя администратора: от 1 до 60 символов".into());
        }
        let mut raw = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let token = format!("jla_{}", jarvis_license::hex(&raw));
        let hash = key_hash(&self.pepper, &token);
        self.conn.lock().execute("INSERT INTO admin_users(id, name, token_hash, created_at) VALUES(?1,?2,?3,?4)", params![Self::new_id("A"), name, hash, now]).map_err(|e| format!("не создан (имя занято?): {}", e))?;
        Ok(token)
    }

    // the name of the administrator that owns this token
    pub fn admin_by_token(&self, token: &str) -> Option<String> {
        if token.len() < 20 || token.len() > 200 {
            return None;
        }
        let hash = key_hash(&self.pepper, token);
        self.conn.lock().query_row("SELECT name FROM admin_users WHERE token_hash = ?1 AND disabled = 0", params![hash], |r| r.get(0)).optional().ok().flatten()
    }

    // ---- tariffs ----
    fn tariff_row(r: &Row) -> rusqlite::Result<Tariff> {
        Ok(Tariff { id: r.get(0)?, name: r.get(1)?, kind: r.get(2)?, amount: r.get(3)?, price_cents: r.get(4)?, currency: r.get(5)?, enabled: r.get::<_, i64>(6)? != 0, sort: r.get(7)? })
    }

    pub fn tariffs(&self, only_enabled: bool) -> Result<Vec<Tariff>, Failure> {
        let conn = self.conn.lock();
        let sql = if only_enabled { "SELECT id,name,kind,amount,price_cents,currency,enabled,sort FROM tariffs WHERE enabled=1 ORDER BY sort,id" } else { "SELECT id,name,kind,amount,price_cents,currency,enabled,sort FROM tariffs ORDER BY sort,id" };
        let mut st = conn.prepare(sql)?;
        let rows = st.query_map([], Self::tariff_row)?.collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn tariff_upsert(&self, actor: &str, now: i64, t: &Tariff) -> Result<(), Failure> {
        if t.id.is_empty() || t.id.len() > 40 || !t.id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return fail(400, "BAD_REQUEST", "id тарифа: латиница, цифры, - и _");
        }
        if !["MONTHS", "DAYS", "LIFETIME"].contains(&t.kind.as_str()) || t.price_cents < 0 || t.price_cents > 100_000_000 {
            return fail(400, "BAD_REQUEST", "тариф: тип MONTHS, DAYS или LIFETIME, цена не отрицательная");
        }
        if t.kind == "MONTHS" && !(1..=120).contains(&t.amount) || t.kind == "DAYS" && !(MIN_CUSTOM_DAYS..=3660).contains(&t.amount) {
            return fail(400, "BAD_REQUEST", "срок тарифа вне допустимых границ (дни: от 7, месяцы: от 1 до 120)");
        }
        if t.currency.len() != 3 || !t.currency.chars().all(|c| c.is_ascii_uppercase()) {
            return fail(400, "BAD_REQUEST", "валюта: три заглавные буквы, например USD");
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let before: Option<Value> = tx.query_row("SELECT id,name,kind,amount,price_cents,currency,enabled,sort FROM tariffs WHERE id=?1", params![t.id], Self::tariff_row).optional()?.map(|x| x.to_json());
        tx.execute("INSERT INTO tariffs(id,name,kind,amount,price_cents,currency,enabled,sort) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
                    ON CONFLICT(id) DO UPDATE SET name=?2, kind=?3, amount=?4, price_cents=?5, currency=?6, enabled=?7, sort=?8",
            params![t.id, t.name, t.kind, t.amount, t.price_cents, t.currency, t.enabled as i64, t.sort])?;
        Self::audit(&tx, now, actor, "tariff.upsert", None, before, Some(t.to_json()), &t.id)?;
        tx.commit()?;
        Ok(())
    }

    // ---- creating keys ----
    pub fn create_licenses(&self, actor: &str, now: i64, req: &NewLicenses) -> Result<Vec<(License, String)>, Failure> {
        if req.count == 0 || req.count > MAX_BATCH {
            return fail(400, "BAD_REQUEST", format!("количество ключей: от 1 до {}", MAX_BATCH));
        }
        if req.purpose != "NEW" && req.purpose != "RENEWAL" {
            return fail(400, "BAD_REQUEST", "тип ключа: NEW (новая подписка) или RENEWAL (продление)");
        }
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        // the period: a tariff or an explicit one
        let (tariff_id, plan_name, kind, amount, price, currency) = if let Some(tid) = &req.tariff_id {
            let t: Option<Tariff> = tx.query_row("SELECT id,name,kind,amount,price_cents,currency,enabled,sort FROM tariffs WHERE id=?1", params![tid], Self::tariff_row).optional()?;
            let Some(t) = t else { return fail(404, "UNKNOWN_TARIFF", "такого тарифа нет") };
            if !t.enabled {
                return fail(409, "TARIFF_DISABLED", "тариф выключен");
            }
            (Some(t.id.clone()), t.name.clone(), t.kind.clone(), t.amount, t.price_cents, t.currency.clone())
        } else if let Some((k, a)) = &req.custom {
            let ok = match k.as_str() {
                "DAYS" => (MIN_CUSTOM_DAYS..=3660).contains(a),
                "MONTHS" => (1..=120).contains(a),
                "LIFETIME" => true,
                _ => false,
            };
            if !ok {
                return fail(400, "BAD_REQUEST", "свой срок: дни от 7 до 3660, месяцы от 1 до 120 или бессрочно (LIFETIME)");
            }
            let name = match k.as_str() { "LIFETIME" => "Lifetime".to_string(), "MONTHS" => format!("{} мес.", a), _ => format!("{} дн.", a) };
            (None, name, k.clone(), if k == "LIFETIME" { 0 } else { *a }, 0, "USD".to_string())
        } else {
            return fail(400, "BAD_REQUEST", "укажите тариф или свой срок");
        };
        let price = req.price_cents.unwrap_or(price);
        let currency = req.currency.clone().unwrap_or(currency);
        if !(0..=100_000_000).contains(&price) || currency.len() != 3 {
            return fail(400, "BAD_REQUEST", "цена или валюта указаны неверно");
        }
        let window_days = req.activate_days.unwrap_or(self.default_activation_days);
        if !(1..=3650).contains(&window_days) {
            return fail(400, "BAD_REQUEST", "срок на первую активацию: от 1 до 3650 дней");
        }
        let activate_by = now + window_days * 86_400;
        let batch = Self::new_id("B");
        let mut out = Vec::new();
        for _ in 0..req.count {
            let (key, hash) = loop {
                let key = generate_key();
                let hash = key_hash(&self.pepper, &key);
                let taken: Option<i64> = tx.query_row("SELECT 1 FROM licenses WHERE key_hash=?1", params![hash], |r| r.get(0)).optional()?;
                if taken.is_none() {
                    break (key, hash);
                }
            };
            let id = Self::new_id("L");
            let hint = key.chars().take(8).collect::<String>() + "…";
            tx.execute(
                "INSERT INTO licenses(id,key_hash,key_hint,purpose,tariff_id,plan_name,period_kind,period_amount,price_cents,currency,status,created_at,activate_by,start_not_before,note,batch_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,'CREATED',?11,?12,?13,?14,?15)",
                params![id, hash, hint, req.purpose, tariff_id, plan_name, kind, amount, price, currency, now, activate_by, req.start_not_before, req.note.chars().take(300).collect::<String>(), batch],
            )?;
            let lic = tx.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![id], license_of)?;
            Self::audit(&tx, now, actor, "license.create", Some(&id), None, Some(lic.to_json(now)), &format!("batch {} {}", batch, key_for_log(&key)))?;
            out.push((lic, key));
        }
        tx.commit()?;
        Ok(out)
    }

    // ---- reading ----
    pub fn license(&self, id: &str) -> Result<License, Failure> {
        let conn = self.conn.lock();
        match conn.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![id], license_of).optional()? {
            Some(l) => Ok(l),
            None => fail(404, "NOT_FOUND", "лицензия не найдена"),
        }
    }

    pub fn list(&self, now: i64, status: Option<&str>, q: Option<&str>, limit: usize) -> Result<Vec<License>, Failure> {
        let conn = self.conn.lock();
        let mut st = conn.prepare(&format!("SELECT {} FROM licenses ORDER BY created_at DESC, id LIMIT 2000", COLS))?;
        let all = st.query_map([], license_of)?.collect::<Result<Vec<_>, _>>()?;
        let q = q.map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
        Ok(all
            .into_iter()
            .filter(|l| status.map(|s| s.eq_ignore_ascii_case(l.effective_status(now))).unwrap_or(true))
            .filter(|l| q.as_ref().map(|q| l.id.to_lowercase().contains(q) || l.note.to_lowercase().contains(q) || l.key_hint.to_lowercase().contains(q) || l.plan_name.to_lowercase().contains(q)).unwrap_or(true))
            .take(limit.clamp(1, 500))
            .collect())
    }

    pub fn audit_list(&self, license: Option<&str>, limit: usize) -> Result<Vec<Value>, Failure> {
        let conn = self.conn.lock();
        let sql = "SELECT id, ts, actor, action, license_id, before_json, after_json, detail FROM audit WHERE (?1 IS NULL OR license_id = ?1) ORDER BY id DESC LIMIT ?2";
        let mut st = conn.prepare(sql)?;
        let rows = st
            .query_map(params![license, limit.clamp(1, 1000) as i64], |r| {
                Ok(json!({ "id": r.get::<_, i64>(0)?, "ts": r.get::<_, i64>(1)?, "actor": r.get::<_, String>(2)?, "action": r.get::<_, String>(3)?, "license_id": r.get::<_, Option<String>>(4)?,
                           "before": r.get::<_, Option<String>>(5)?.and_then(|s| serde_json::from_str::<Value>(&s).ok()), "after": r.get::<_, Option<String>>(6)?.and_then(|s| serde_json::from_str::<Value>(&s).ok()), "detail": r.get::<_, Option<String>>(7)? }))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    // ---- the token ----
    fn issue(&self, lic: &License, inst: &str, now: i64) -> String {
        sign_token(
            &self.signing,
            &Claims { v: TOKEN_VERSION, aud: AUDIENCE.into(), lid: lic.id.clone(), tier: "PREMIUM".into(), plan: lic.plan_name.clone(), inst: inst.to_string(), iat: now, sub_end: lic.sub_end, refresh_by: now + self.grace_hours * 3600 },
        )
    }

    // a proof of an installation: the time is near, the signature is right, and it has never been seen
    fn check_proof(tx: &rusqlite::Transaction, now: i64, inst: &str, purpose: &str, ts: i64, subject: &str, sig: &str) -> Result<(), Failure> {
        if (now - ts).abs() > PROOF_WINDOW {
            return fail(400, "STALE_PROOF", "Часы компьютера сильно расходятся со временем сервера. Проверьте дату и время.");
        }
        if !verify_proof(inst, purpose, ts, subject, sig) {
            return fail(400, "BAD_PROOF", "Запрос не подтверждён подписью установки.");
        }
        let h = sha256_hex(sig.as_bytes());
        tx.execute("DELETE FROM seen_proofs WHERE ts < ?1", params![now - 2 * PROOF_WINDOW])?;
        let fresh = tx.execute("INSERT OR IGNORE INTO seen_proofs(sig_hash, ts) VALUES(?1, ?2)", params![h, now])?;
        if fresh == 0 {
            return fail(409, "REPLAY", "Этот запрос уже был обработан.");
        }
        Ok(())
    }

    fn log_attempt(&self, license: Option<&str>, inst: &str, now: i64, event: &str, ok: bool, code: &str) {
        let _ = self.conn.lock().execute("INSERT INTO activations(license_id, installation_id, ts, event, ok, code) VALUES(?1,?2,?3,?4,?5,?6)", params![license, inst.chars().take(64).collect::<String>(), now, event, ok as i64, code]);
    }

    // ---- activation: the FIRST binding of a key to an installation (only online, only here) ----
    pub fn activate(&self, now: i64, key_raw: &str, inst: &str, ts: i64, sig: &str) -> Result<Issued, Failure> {
        let result = self.activate_inner(now, key_raw, inst, ts, sig);
        if let Err(f) = &result {
            // (a failed attempt is kept without the key)
            self.log_attempt(None, inst, now, "activate", false, f.code);
        }
        result
    }

    fn activate_inner(&self, now: i64, key_raw: &str, inst: &str, ts: i64, sig: &str) -> Result<Issued, Failure> {
        let Some(key) = normalize_key(key_raw) else { return fail(404, "UNKNOWN_KEY", "Ключ не найден или введён неверно.") };
        if jarvis_license::parse_public_key(inst).is_none() {
            return fail(400, "BAD_REQUEST", "Неверный идентификатор установки.");
        }
        let hash = key_hash(&self.pepper, &key);
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::check_proof(&tx, now, inst, "activate", ts, &sha256_hex(key.as_bytes()), sig)?;
        let Some(lic) = tx.query_row(&format!("SELECT {} FROM licenses WHERE key_hash=?1", COLS), params![hash], license_of).optional()? else {
            tx.commit()?; // (the proof stays recorded: a replay of a failed attempt is a replay all the same)
            return fail(404, "UNKNOWN_KEY", "Ключ не найден или введён неверно.");
        };
        if lic.purpose == "RENEWAL" {
            return fail(422, "USE_RENEW", "Это ключ продления. Он вводится в окне продления уже активированной программы.");
        }
        match lic.effective_status(now) {
            "REVOKED" => return fail(403, "LICENSE_REVOKED", "Лицензия отозвана."),
            "SUSPENDED" => return fail(403, "LICENSE_SUSPENDED", "Лицензия временно заблокирована."),
            "EXPIRED" if lic.status == "CREATED" => return fail(410, "KEY_EXPIRED", "Срок, в который этот ключ можно было активировать, закончился."),
            "EXPIRED" => return fail(402, "SUBSCRIPTION_EXPIRED", "Подписка по этому ключу закончилась. Нужен ключ продления."),
            _ => {}
        }
        let mut lic = lic;
        match (lic.status.as_str(), lic.installation_id.clone()) {
            ("CREATED", _) => {
                // the first activation: the subscription starts now (or at the date the administrator set, when it is later)
                let start = lic.start_not_before.map(|s| s.max(now)).unwrap_or(now);
                let end = lic.period().end_from(start);
                // the guard: the row must still be unbound (two parallel requests: only one of them changes it)
                let changed = tx.execute(
                    "UPDATE licenses SET status='ACTIVE', activated_at=?2, sub_start=?3, sub_end=?4, installation_id=?5, last_check=?2 WHERE id=?1 AND status='CREATED' AND installation_id IS NULL",
                    params![lic.id, now, start, end, inst],
                )?;
                if changed != 1 {
                    return fail(409, "BOUND_TO_OTHER_DEVICE", "Этот ключ уже активирован на другом устройстве.");
                }
                let after = tx.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![lic.id], license_of)?;
                Self::audit(&tx, now, &format!("installation:{}", inst.chars().take(8).collect::<String>()), "license.activate", Some(&lic.id), Some(lic.to_json(now)), Some(after.to_json(now)), "")?;
                lic = after;
            }
            ("ACTIVE", Some(bound)) if bound == inst => {
                // the same installation asks again (the answer was lost): the same result, nothing is counted twice
                tx.execute("UPDATE licenses SET last_check=?2 WHERE id=?1", params![lic.id, now])?;
            }
            ("ACTIVE", None) => {
                // the binding was reset by the administrator: this installation takes it; the subscription is NOT restarted
                let changed = tx.execute("UPDATE licenses SET installation_id=?2, last_check=?3 WHERE id=?1 AND installation_id IS NULL", params![lic.id, inst, now])?;
                if changed != 1 {
                    return fail(409, "BOUND_TO_OTHER_DEVICE", "Этот ключ уже активирован на другом устройстве.");
                }
                let after = tx.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![lic.id], license_of)?;
                Self::audit(&tx, now, &format!("installation:{}", inst.chars().take(8).collect::<String>()), "license.rebind", Some(&lic.id), Some(lic.to_json(now)), Some(after.to_json(now)), "")?;
                lic = after;
            }
            _ => return fail(409, "BOUND_TO_OTHER_DEVICE", "Этот ключ уже активирован на другом устройстве."),
        }
        tx.execute("INSERT INTO activations(license_id, installation_id, ts, event, ok, code) VALUES(?1,?2,?3,'activate',1,'OK')", params![lic.id, inst, now])?;
        tx.commit()?;
        drop(conn);
        let token = self.issue(&lic, inst, now);
        Ok(Issued { token, license: lic, note: "ACTIVATED" })
    }

    // ---- refresh / validate: the installation shows that it still holds its key ----
    pub fn refresh(&self, now: i64, lid: &str, inst: &str, ts: i64, sig: &str, purpose: &str) -> Result<Issued, Failure> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::check_proof(&tx, now, inst, purpose, ts, lid, sig)?;
        let Some(lic) = tx.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![lid], license_of).optional()? else {
            tx.commit()?;
            return fail(404, "UNKNOWN_LICENSE", "Лицензия не найдена на сервере.");
        };
        if lic.installation_id.as_deref() != Some(inst) {
            tx.commit()?;
            return fail(403, "NOT_BOUND", "Лицензия не привязана к этой установке.");
        }
        match lic.effective_status(now) {
            "REVOKED" => {
                tx.commit()?;
                return fail(403, "LICENSE_REVOKED", "Лицензия отозвана.");
            }
            "SUSPENDED" => {
                tx.commit()?;
                return fail(403, "LICENSE_SUSPENDED", "Лицензия временно заблокирована.");
            }
            "EXPIRED" => {
                if lic.status != "EXPIRED" {
                    tx.execute("UPDATE licenses SET status='EXPIRED', last_check=?2 WHERE id=?1 AND status='ACTIVE'", params![lic.id, now])?;
                }
                tx.commit()?;
                return Err(Failure { status: 402, code: "SUBSCRIPTION_EXPIRED", message: format!("Подписка закончилась {}.", lic.sub_end.unwrap_or(0)) });
            }
            _ => {}
        }
        tx.execute("UPDATE licenses SET last_check=?2 WHERE id=?1", params![lic.id, now])?;
        tx.commit()?;
        drop(conn);
        let token = self.issue(&lic, inst, now);
        Ok(Issued { token, license: lic, note: "OK" })
    }

    // ---- renewing: a key of the kind RENEWAL adds its period to the license of this installation ----
    pub fn renew(&self, now: i64, lid: &str, inst: &str, ts: i64, sig: &str, renewal_key_raw: &str) -> Result<Issued, Failure> {
        let Some(rkey) = normalize_key(renewal_key_raw) else { return fail(404, "UNKNOWN_KEY", "Ключ продления не найден или введён неверно.") };
        let rhash = key_hash(&self.pepper, &rkey);
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        Self::check_proof(&tx, now, inst, "renew", ts, &format!("{}|{}", lid, sha256_hex(rkey.as_bytes())), sig)?;
        let Some(target) = tx.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![lid], license_of).optional()? else {
            tx.commit()?;
            return fail(404, "UNKNOWN_LICENSE", "Лицензия не найдена на сервере.");
        };
        if target.installation_id.as_deref() != Some(inst) || target.purpose != "NEW" {
            tx.commit()?;
            return fail(403, "NOT_BOUND", "Лицензия не привязана к этой установке.");
        }
        if matches!(target.status.as_str(), "REVOKED" | "SUSPENDED") {
            tx.commit()?;
            return fail(403, if target.status == "REVOKED" { "LICENSE_REVOKED" } else { "LICENSE_SUSPENDED" }, "Эту лицензию нельзя продлить: она отозвана или заблокирована.");
        }
        let Some(rl) = tx.query_row(&format!("SELECT {} FROM licenses WHERE key_hash=?1", COLS), params![rhash], license_of).optional()? else {
            tx.commit()?;
            return fail(404, "UNKNOWN_KEY", "Ключ продления не найден или введён неверно.");
        };
        if rl.purpose != "RENEWAL" {
            return fail(422, "NOT_A_RENEWAL_KEY", "Это не ключ продления. Новая подписка активируется в окне активации.");
        }
        // the same installation sends the same key again: what was done stays done, nothing is added twice
        if rl.status == "USED" {
            if rl.renews_license_id.as_deref() == Some(&target.id) && rl.installation_id.as_deref() == Some(inst) {
                tx.commit()?;
                let token = self.issue(&target, inst, now);
                return Ok(Issued { token, license: target, note: "ALREADY_APPLIED" });
            }
            return fail(409, "RENEWAL_USED", "Этот ключ продления уже использован.");
        }
        match rl.effective_status(now) {
            "REVOKED" => return fail(403, "LICENSE_REVOKED", "Ключ продления отозван."),
            "SUSPENDED" => return fail(403, "LICENSE_SUSPENDED", "Ключ продления заблокирован."),
            "EXPIRED" => return fail(410, "KEY_EXPIRED", "Срок, в который этот ключ можно было использовать, закончился."),
            _ => {}
        }
        if target.period_kind == "LIFETIME" && target.sub_end.is_none() {
            return fail(409, "ALREADY_LIFETIME", "У вас уже бессрочная лицензия: продлевать нечего.");
        }
        // the new period is added to the END of the running subscription (paid days are not lost); an ended one starts now
        let base = target.sub_end.map(|e| e.max(now)).unwrap_or(now);
        let (new_end, plan, kind, amount) = match rl.period() {
            Period::Lifetime => (None, "Lifetime".to_string(), "LIFETIME".to_string(), 0),
            p => (p.end_from(base), target.plan_name.clone(), target.period_kind.clone(), target.period_amount),
        };
        let before = target.to_json(now);
        let changed = tx.execute("UPDATE licenses SET status='ACTIVE', sub_end=?2, plan_name=?3, period_kind=?4, period_amount=?5, last_check=?6 WHERE id=?1", params![target.id, new_end, plan, kind, amount, now])?;
        let used = tx.execute("UPDATE licenses SET status='USED', activated_at=?2, installation_id=?3, renews_license_id=?4 WHERE id=?1 AND status='CREATED'", params![rl.id, now, inst, target.id])?;
        if changed != 1 || used != 1 {
            return fail(409, "RENEWAL_USED", "Этот ключ продления уже использован.");
        }
        let after = tx.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![target.id], license_of)?;
        Self::audit(&tx, now, &format!("installation:{}", inst.chars().take(8).collect::<String>()), "license.renew", Some(&target.id), Some(before), Some(after.to_json(now)), &format!("renewal key license {}", rl.id))?;
        tx.commit()?;
        drop(conn);
        let token = self.issue(&after, inst, now);
        Ok(Issued { token, license: after, note: "RENEWED" })
    }

    // ---- administrator operations (every one is audited: who, when, before, after) ----
    fn admin_change(&self, actor: &str, now: i64, id: &str, action: &str, detail: &str, apply: impl FnOnce(&rusqlite::Transaction, &License) -> Result<(), Failure>) -> Result<License, Failure> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let Some(before) = tx.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![id], license_of).optional()? else { return fail(404, "NOT_FOUND", "лицензия не найдена") };
        apply(&tx, &before)?;
        let after = tx.query_row(&format!("SELECT {} FROM licenses WHERE id=?1", COLS), params![id], license_of)?;
        Self::audit(&tx, now, actor, action, Some(id), Some(before.to_json(now)), Some(after.to_json(now)), detail)?;
        tx.commit()?;
        Ok(after)
    }

    pub fn revoke(&self, actor: &str, now: i64, id: &str, reason: &str) -> Result<License, Failure> {
        self.admin_change(actor, now, id, "license.revoke", reason, |tx, _| {
            tx.execute("UPDATE licenses SET status='REVOKED', revoked_at=?2, revoke_reason=?3 WHERE id=?1", params![id, now, reason.chars().take(200).collect::<String>()])?;
            Ok(())
        })
    }

    pub fn suspend(&self, actor: &str, now: i64, id: &str, reason: &str) -> Result<License, Failure> {
        self.admin_change(actor, now, id, "license.suspend", reason, |tx, b| {
            if b.status == "REVOKED" {
                return fail(409, "REVOKED", "отозванную лицензию нельзя заблокировать повторно");
            }
            tx.execute("UPDATE licenses SET status='SUSPENDED', revoke_reason=?2 WHERE id=?1", params![id, reason.chars().take(200).collect::<String>()])?;
            Ok(())
        })
    }

    pub fn resume(&self, actor: &str, now: i64, id: &str) -> Result<License, Failure> {
        self.admin_change(actor, now, id, "license.resume", "", |tx, b| {
            if b.status != "SUSPENDED" {
                return fail(409, "NOT_SUSPENDED", "лицензия не заблокирована");
            }
            let back = if b.activated_at.is_some() { "ACTIVE" } else { "CREATED" };
            tx.execute("UPDATE licenses SET status=?2, revoke_reason=NULL WHERE id=?1", params![id, back])?;
            Ok(())
        })
    }

    // the binding is dropped: the old installation gets no more confirmations; a new one may take the key (the dates stay)
    pub fn reset_device(&self, actor: &str, now: i64, id: &str, reason: &str) -> Result<License, Failure> {
        self.admin_change(actor, now, id, "license.reset_device", reason, |tx, b| {
            if b.status == "REVOKED" || b.status == "USED" {
                return fail(409, "CANNOT_RESET", "у отозванной или использованной лицензии нечего сбрасывать");
            }
            tx.execute("UPDATE licenses SET installation_id=NULL WHERE id=?1", params![id])?;
            Ok(())
        })
    }

    pub fn extend(&self, actor: &str, now: i64, id: &str, kind: &str, amount: i64, reason: &str) -> Result<License, Failure> {
        let period = match kind {
            "DAYS" if (MIN_CUSTOM_DAYS..=3660).contains(&amount) => Period::Days(amount as u32),
            "MONTHS" if (1..=120).contains(&amount) => Period::Months(amount as u32),
            "LIFETIME" => Period::Lifetime,
            _ => return fail(400, "BAD_REQUEST", "срок продления: дни от 7 до 3660, месяцы от 1 до 120 или LIFETIME"),
        };
        self.admin_change(actor, now, id, "license.extend", &format!("{} {} {}", kind, amount, reason), |tx, b| {
            if b.purpose != "NEW" || b.activated_at.is_none() {
                return fail(409, "NOT_ACTIVATED", "продлить можно только активированную лицензию");
            }
            if b.status == "REVOKED" {
                return fail(409, "REVOKED", "отозванную лицензию продлить нельзя");
            }
            let base = b.sub_end.map(|e| e.max(now));
            match (period, base) {
                (Period::Lifetime, _) => {
                    tx.execute("UPDATE licenses SET sub_end=NULL, plan_name='Lifetime', period_kind='LIFETIME', period_amount=0, status=CASE WHEN status='EXPIRED' THEN 'ACTIVE' ELSE status END WHERE id=?1", params![id])?;
                }
                (_, None) => return fail(409, "ALREADY_LIFETIME", "лицензия уже бессрочная"),
                (p, Some(base)) => {
                    tx.execute("UPDATE licenses SET sub_end=?2, status=CASE WHEN status='EXPIRED' THEN 'ACTIVE' ELSE status END WHERE id=?1", params![id, p.end_from(base)])?;
                }
            }
            Ok(())
        })
    }

    pub fn change_tariff(&self, actor: &str, now: i64, id: &str, tariff_id: &str, recompute: bool) -> Result<License, Failure> {
        self.admin_change(actor, now, id, "license.change_tariff", &format!("{} recompute={}", tariff_id, recompute), |tx, b| {
            let t: Option<Tariff> = tx.query_row("SELECT id,name,kind,amount,price_cents,currency,enabled,sort FROM tariffs WHERE id=?1", params![tariff_id], Self::tariff_row).optional()?;
            let Some(t) = t else { return fail(404, "UNKNOWN_TARIFF", "такого тарифа нет") };
            if b.status == "REVOKED" {
                return fail(409, "REVOKED", "у отозванной лицензии тариф не меняется");
            }
            tx.execute("UPDATE licenses SET tariff_id=?2, plan_name=?3, period_kind=?4, period_amount=?5, price_cents=?6, currency=?7 WHERE id=?1", params![id, t.id, t.name, t.kind, t.amount, t.price_cents, t.currency])?;
            if recompute {
                if let Some(start) = b.sub_start {
                    let period = match t.kind.as_str() { "MONTHS" => Period::Months(t.amount as u32), "DAYS" => Period::Days(t.amount as u32), _ => Period::Lifetime };
                    tx.execute("UPDATE licenses SET sub_end=?2 WHERE id=?1", params![id, period.end_from(start)])?;
                }
            }
            Ok(())
        })
    }

    pub fn set_activation_deadline(&self, actor: &str, now: i64, id: &str, activate_by: i64) -> Result<License, Failure> {
        self.admin_change(actor, now, id, "license.set_deadline", &activate_by.to_string(), |tx, b| {
            if b.status != "CREATED" {
                return fail(409, "ALREADY_ACTIVATED", "срок активации меняется только у неиспользованного ключа");
            }
            tx.execute("UPDATE licenses SET activate_by=?2 WHERE id=?1", params![id, activate_by])?;
            Ok(())
        })
    }

    pub fn set_note(&self, actor: &str, now: i64, id: &str, note: &str) -> Result<License, Failure> {
        self.admin_change(actor, now, id, "license.note", "", |tx, _| {
            tx.execute("UPDATE licenses SET note=?2 WHERE id=?1", params![id, note.chars().take(300).collect::<String>()])?;
            Ok(())
        })
    }

    pub fn renew_stats(&self) -> Result<Value, Failure> {
        let conn = self.conn.lock();
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM licenses", [], |r| r.get(0))?;
        Ok(json!({ "licenses": n }))
    }
}

pub struct NewLicenses {
    pub count: u32,
    pub purpose: String,
    pub tariff_id: Option<String>,
    pub custom: Option<(String, i64)>,
    pub price_cents: Option<i64>,
    pub currency: Option<String>,
    pub activate_days: Option<i64>,
    pub start_not_before: Option<i64>,
    pub note: String,
}

fn now_real() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}
