//! The license manager of Jarvis ADMIN: the owner's window to the license server (keys, tariffs, audit).
//!
//! This module is not compiled into Jarvis User at all (`#[cfg(not(feature = "user-edition"))]` in lib.rs and in the window server),
//! so the customer's program neither contains the code nor the address of the administration. The administrator token is kept
//! sealed with the Windows data protection (secret.rs) in the owner's own data folder; it is never put in a log, a build or a page.
use serde_json::{json, Value};

const FILE: &str = "license-admin.json";

fn path() -> Option<std::path::PathBuf> {
    crate::APP_CONFIG_DIR.get().map(|d| d.join(FILE))
}

fn read() -> (String, String) {
    let Some(p) = path() else { return (String::new(), String::new()) };
    let v: Value = std::fs::read_to_string(p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let url = v.get("url").and_then(|x| x.as_str()).unwrap_or("").to_string();
    let token = crate::secret::open(v.get("token").and_then(|x| x.as_str()).unwrap_or(""));
    (url, token)
}

pub fn settings_json() -> Value {
    let (url, token) = read();
    // the token itself never leaves this module
    json!({ "url": url, "has_token": !token.is_empty() })
}

pub fn set_settings(url: &str, token: Option<&str>) -> Result<Value, String> {
    let url = url.trim().trim_end_matches('/').to_string();
    if !url.is_empty() && !url.starts_with("https://") && !url.starts_with("http://127.0.0.1") && !url.starts_with("http://localhost") {
        return Err("Адрес сервера должен начинаться с https:// (без шифрования можно только для localhost).".into());
    }
    let (_, old_token) = read();
    let token = match token {
        Some(t) if !t.trim().is_empty() => t.trim().to_string(),
        _ => old_token,
    };
    let p = path().ok_or("Папка данных ещё не готова.")?;
    let v = json!({ "url": url, "token": crate::secret::seal(&token) });
    crate::write_atomic(&p, v.to_string()).map_err(|e| e.to_string())?;
    Ok(settings_json())
}

pub fn forget_token() {
    let (url, _) = read();
    if let Some(p) = path() {
        let _ = crate::write_atomic(&p, json!({ "url": url, "token": "" }).to_string());
    }
}

// only the administration part of the server, only these verbs, no way out of the address
fn allowed(method: &str, path: &str) -> bool {
    matches!(method, "GET" | "POST" | "PUT") && path.starts_with("/admin/") && !path.contains("..") && !path.contains("//") && !path.contains('\\') && path.len() < 300
}

#[cfg(feature = "lua")]
pub fn call(method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value), String> {
    if !allowed(method, path) {
        return Err("Недопустимый запрос к серверу лицензий.".into());
    }
    let (url, token) = read();
    if url.is_empty() || token.is_empty() {
        return Err("Сначала укажите адрес сервера лицензий и токен администратора.".into());
    }
    let client = reqwest::blocking::Client::builder().timeout(std::time::Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let full = format!("{}{}", url, path);
    let b = match method {
        "GET" => client.get(&full),
        "PUT" => client.put(&full).json(body.unwrap_or(&Value::Null)),
        _ => client.post(&full).json(body.unwrap_or(&Value::Null)),
    };
    let resp = b.bearer_auth(token).send().map_err(|_| "Нет связи с сервером лицензий.".to_string())?;
    let status = resp.status().as_u16();
    Ok((status, resp.json().unwrap_or(Value::Null)))
}

#[cfg(not(feature = "lua"))]
pub fn call(_m: &str, _p: &str, _b: Option<&Value>) -> Result<(u16, Value), String> {
    Err("Сетевая часть не включена в эту сборку.".into())
}

pub fn action(body: &Value) -> Value {
    let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "settings_get" => json!({ "ok": true, "settings": settings_json() }),
        "settings_set" => match set_settings(body.get("url").and_then(|v| v.as_str()).unwrap_or(""), body.get("token").and_then(|v| v.as_str())) {
            Ok(s) => json!({ "ok": true, "settings": s }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "forget_token" => {
            forget_token();
            json!({ "ok": true, "settings": settings_json() })
        }
        "call" => {
            let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
            let p = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match call(method, p, body.get("body")) {
                Ok((status, v)) if status < 400 => json!({ "ok": true, "data": v }),
                Ok((status, v)) => json!({ "ok": false, "status": status, "message": v.get("message").and_then(|m| m.as_str()).unwrap_or("Сервер ответил ошибкой."), "code": v.get("code") }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        _ => json!({ "ok": false, "message": "неизвестная операция" }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_administration_part_of_the_server_can_be_reached() {
        assert!(allowed("GET", "/admin/licenses?status=ACTIVE"));
        assert!(allowed("POST", "/admin/licenses/abc/revoke"));
        assert!(allowed("PUT", "/admin/tariffs"));
        assert!(!allowed("DELETE", "/admin/licenses"));
        assert!(!allowed("GET", "/licenses/activate"));
        assert!(!allowed("GET", "/admin/../health"));
        assert!(!allowed("GET", "/admin//x"));
        assert!(!allowed("GET", "/health"));
        assert!(call("GET", "/health", None).is_err());
    }

    #[test]
    fn an_insecure_address_is_refused() {
        assert!(set_settings("http://license.example.com", Some("jla_x")).is_err());
        assert!(set_settings("ftp://x", None).is_err());
    }
}
