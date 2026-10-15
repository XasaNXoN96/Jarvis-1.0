//! The routes. Public routes (activation, refresh, renewal, tariffs, health) are protected by the proof of the installation and by
//! rate limits; administrator routes need a bearer token that belongs to an administrator (its hash is in the database). Knowing a
//! URL, or sending a header like `is_admin`, gives nothing.
use crate::store::{fail, Failure, License, NewLicenses, Store, Tariff};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};

pub struct Request {
    pub method: String,
    // the path with the query
    pub path: String,
    // lower-case names
    pub headers: HashMap<String, String>,
    pub body: String,
    pub ip: String,
}

pub struct Response {
    pub status: u16,
    pub body: String,
}

#[derive(Default)]
struct Limiter {
    seen: HashMap<String, VecDeque<i64>>,
}

impl Limiter {
    fn blocked(&mut self, key: &str, now: i64, max: usize, window: i64) -> bool {
        let q = self.seen.entry(key.to_string()).or_default();
        while q.front().map(|t| now - t > window).unwrap_or(false) {
            q.pop_front();
        }
        q.len() >= max
    }

    fn note(&mut self, key: &str, now: i64) {
        self.seen.entry(key.to_string()).or_default().push_back(now);
        if self.seen.len() > 20_000 {
            self.seen.retain(|_, q| q.back().map(|t| now - t < 3600).unwrap_or(false));
        }
    }
}

pub struct Api {
    pub store: Store,
    pub renew_info: String,
    limiter: Mutex<Limiter>,
}

const WINDOW: i64 = 600;
const MAX_FAILS_PER_IP: usize = 10; // wrong keys / proofs per address per 10 minutes
const MAX_FAILS_GLOBAL: usize = 300; // the same for the whole server (a guess spread over many addresses)
const MAX_ADMIN_FAILS: usize = 5;

fn respond(status: u16, v: Value) -> Response {
    Response { status, body: v.to_string() }
}

fn failure(f: Failure) -> Response {
    respond(f.status, json!({ "ok": false, "code": f.code, "message": f.message }))
}

fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(|x| x.as_str())
}

fn i(v: &Value, k: &str) -> Option<i64> {
    v.get(k).and_then(|x| x.as_i64())
}

fn query_param(path: &str, name: &str) -> Option<String> {
    let q = path.split_once('?')?.1;
    q.split('&').find_map(|p| {
        let (k, v) = p.split_once('=')?;
        (k == name).then(|| v.replace('+', " ").chars().take(100).collect())
    })
}

impl Api {
    pub fn new(store: Store, renew_info: String) -> Api {
        Api { store, renew_info, limiter: Mutex::new(Limiter::default()) }
    }

    pub fn handle(&self, req: &Request, now: i64) -> Response {
        let path = req.path.split('?').next().unwrap_or("/").trim_end_matches('/').to_string();
        let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let body: Value = if req.method == "POST" || req.method == "PUT" {
            match serde_json::from_str(if req.body.trim().is_empty() { "{}" } else { &req.body }) {
                Ok(v) => v,
                Err(_) => return failure(Failure { status: 400, code: "BAD_REQUEST", message: "Тело запроса не JSON.".into() }),
            }
        } else {
            Value::Null
        };
        match (req.method.as_str(), segs.as_slice()) {
            ("GET", ["health"]) => respond(200, json!({ "ok": self.store.health(), "schema": self.store.schema_version(), "time": now })),
            ("GET", ["tariffs"]) => match self.store.tariffs(true) {
                Ok(t) => respond(200, json!({ "ok": true, "tariffs": t.iter().map(Tariff::to_json).collect::<Vec<_>>(), "info": self.renew_info, "server_time": now })),
                Err(f) => failure(f),
            },
            ("POST", ["licenses", op]) if ["activate", "refresh", "validate", "renew"].contains(op) => self.customer(op, &body, req, now),
            (_, ["admin", rest @ ..]) => self.admin(req, rest, &body, now),
            _ => respond(404, json!({ "ok": false, "code": "NOT_FOUND", "message": "нет такого адреса" })),
        }
    }

    // ---- the customer's program ----
    fn customer(&self, op: &str, body: &Value, req: &Request, now: i64) -> Response {
        let (ipk, glob) = (format!("fail:{}", req.ip), "fail:*".to_string());
        {
            let mut l = self.limiter.lock();
            if l.blocked(&ipk, now, MAX_FAILS_PER_IP, WINDOW) || l.blocked(&glob, now, MAX_FAILS_GLOBAL, WINDOW) {
                return failure(Failure { status: 429, code: "RATE_LIMITED", message: "Слишком много неудачных попыток. Подождите несколько минут.".into() });
            }
        }
        let (Some(inst), Some(ts), Some(sig)) = (s(body, "installation"), i(body, "ts"), s(body, "sig")) else {
            return failure(Failure { status: 400, code: "BAD_REQUEST", message: "Не хватает полей запроса.".into() });
        };
        let result = match op {
            "activate" => match s(body, "key") {
                Some(k) if k.len() <= 80 => self.store.activate(now, k, inst, ts, sig),
                _ => return failure(Failure { status: 400, code: "BAD_REQUEST", message: "Введите ключ.".into() }),
            },
            "refresh" | "validate" => match s(body, "lid") {
                Some(lid) if lid.len() <= 60 => self.store.refresh(now, lid, inst, ts, sig, op),
                _ => return failure(Failure { status: 400, code: "BAD_REQUEST", message: "Не хватает номера лицензии.".into() }),
            },
            _ => match (s(body, "lid"), s(body, "key")) {
                (Some(lid), Some(k)) if lid.len() <= 60 && k.len() <= 80 => self.store.renew(now, lid, inst, ts, sig, k),
                _ => return failure(Failure { status: 400, code: "BAD_REQUEST", message: "Введите ключ продления.".into() }),
            },
        };
        match result {
            Ok(issued) => {
                let l = &issued.license;
                let mut out = json!({ "ok": true, "result": issued.note, "lid": l.id, "plan": l.plan_name, "sub_end": l.sub_end, "lifetime": l.sub_end.is_none(), "server_time": now });
                if op != "validate" {
                    out["token"] = json!(issued.token);
                }
                respond(200, out)
            }
            Err(f) => {
                // only an unknown key, a wrong proof or an attempt on a foreign key counts against the address
                if matches!(f.code, "UNKNOWN_KEY" | "BAD_PROOF" | "BOUND_TO_OTHER_DEVICE" | "NOT_BOUND" | "UNKNOWN_LICENSE" | "NOT_A_RENEWAL_KEY") {
                    let mut l = self.limiter.lock();
                    l.note(&ipk, now);
                    l.note(&glob, now);
                }
                failure(f)
            }
        }
    }

    // ---- the administrator ----
    fn admin(&self, req: &Request, rest: &[&str], body: &Value, now: i64) -> Response {
        let ipk = format!("adminfail:{}", req.ip);
        if self.limiter.lock().blocked(&ipk, now, MAX_ADMIN_FAILS, 300) {
            return failure(Failure { status: 429, code: "RATE_LIMITED", message: "Слишком много неудачных попыток входа.".into() });
        }
        let token = req.headers.get("authorization").and_then(|h| h.strip_prefix("Bearer ")).unwrap_or("").trim().to_string();
        let Some(actor) = self.store.admin_by_token(&token) else {
            self.limiter.lock().note(&ipk, now);
            return failure(Failure { status: 401, code: "UNAUTHORIZED", message: "Нужна авторизация администратора.".into() });
        };
        let actor = format!("admin:{}", actor);
        let m = req.method.as_str();
        let lic = |r: Result<License, Failure>| match r {
            Ok(l) => respond(200, json!({ "ok": true, "license": l.to_json(now) })),
            Err(f) => failure(f),
        };
        match (m, rest) {
            ("POST", ["licenses"]) => {
                let custom = body.get("custom").and_then(|c| Some((s(c, "kind")?.to_string(), i(c, "amount").unwrap_or(0))));
                let req2 = NewLicenses {
                    count: i(body, "count").unwrap_or(1).clamp(0, 10_000) as u32,
                    purpose: s(body, "purpose").unwrap_or("NEW").to_string(),
                    tariff_id: s(body, "tariff_id").map(|x| x.to_string()),
                    custom,
                    price_cents: i(body, "price_cents"),
                    currency: s(body, "currency").map(|x| x.to_string()),
                    activate_days: i(body, "activate_days"),
                    start_not_before: i(body, "start_not_before"),
                    note: s(body, "note").unwrap_or("").to_string(),
                };
                match self.store.create_licenses(&actor, now, &req2) {
                    Ok(list) => respond(200, json!({ "ok": true, "licenses": list.iter().map(|(l, key)| { let mut j = l.to_json(now); j["key"] = json!(key); j }).collect::<Vec<_>>(), "note": "Полные ключи показаны один раз: сервер хранит только их хеши." })),
                    Err(f) => failure(f),
                }
            }
            ("GET", ["licenses"]) => match self.store.list(now, query_param(&req.path, "status").as_deref(), query_param(&req.path, "q").as_deref(), query_param(&req.path, "limit").and_then(|v| v.parse().ok()).unwrap_or(200)) {
                Ok(list) => respond(200, json!({ "ok": true, "licenses": list.iter().map(|l| l.to_json(now)).collect::<Vec<_>>(), "server_time": now })),
                Err(f) => failure(f),
            },
            ("GET", ["licenses", id]) => lic(self.store.license(id)),
            ("POST", ["licenses", id, op]) => {
                let reason = s(body, "reason").unwrap_or("").to_string();
                match *op {
                    "revoke" => lic(self.store.revoke(&actor, now, id, &reason)),
                    "suspend" => lic(self.store.suspend(&actor, now, id, &reason)),
                    "resume" => lic(self.store.resume(&actor, now, id)),
                    "reset-device" => lic(self.store.reset_device(&actor, now, id, &reason)),
                    "extend" => lic(self.store.extend(&actor, now, id, s(body, "kind").unwrap_or(""), i(body, "amount").unwrap_or(0), &reason)),
                    "change-tariff" => lic(self.store.change_tariff(&actor, now, id, s(body, "tariff_id").unwrap_or(""), body.get("recompute").and_then(|v| v.as_bool()).unwrap_or(false))),
                    "deadline" => match i(body, "activate_by") {
                        Some(t) => lic(self.store.set_activation_deadline(&actor, now, id, t)),
                        None => failure(Failure { status: 400, code: "BAD_REQUEST", message: "нужно activate_by (unix-время)".into() }),
                    },
                    "note" => lic(self.store.set_note(&actor, now, id, s(body, "note").unwrap_or(""))),
                    _ => respond(404, json!({ "ok": false, "code": "NOT_FOUND", "message": "нет такого действия" })),
                }
            }
            ("GET", ["audit"]) => match self.store.audit_list(query_param(&req.path, "license").as_deref(), query_param(&req.path, "limit").and_then(|v| v.parse().ok()).unwrap_or(200)) {
                Ok(rows) => respond(200, json!({ "ok": true, "audit": rows })),
                Err(f) => failure(f),
            },
            ("GET", ["tariffs"]) => match self.store.tariffs(false) {
                Ok(t) => respond(200, json!({ "ok": true, "tariffs": t.iter().map(Tariff::to_json).collect::<Vec<_>>() })),
                Err(f) => failure(f),
            },
            ("PUT", ["tariffs"]) => {
                let t = Tariff {
                    id: s(body, "id").unwrap_or("").to_string(),
                    name: s(body, "name").unwrap_or("").chars().take(60).collect(),
                    kind: s(body, "kind").unwrap_or("").to_string(),
                    amount: i(body, "amount").unwrap_or(0),
                    price_cents: i(body, "price_cents").unwrap_or(-1),
                    currency: s(body, "currency").unwrap_or("USD").to_string(),
                    enabled: body.get("enabled").and_then(|v| v.as_bool()).unwrap_or(true),
                    sort: i(body, "sort").unwrap_or(0),
                };
                match self.store.tariff_upsert(&actor, now, &t) {
                    Ok(()) => respond(200, json!({ "ok": true, "tariff": t.to_json() })),
                    Err(f) => failure(f),
                }
            }
            _ => respond(404, json!({ "ok": false, "code": "NOT_FOUND", "message": "нет такого адреса" })),
        }
    }
}

pub fn not_found() -> Failure {
    Failure { status: 404, code: "NOT_FOUND", message: "нет такого адреса".into() }
}

#[allow(dead_code)]
fn unused() -> Result<(), Failure> {
    fail(500, "X", "")
}
