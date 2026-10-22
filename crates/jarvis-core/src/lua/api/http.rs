// HTTP Lua API: GET, POST, JSON requests

use mlua::{Lua, Table, Value};
use std::collections::HashMap;
use std::time::Duration;


// ---- where a script may send a request: the internet only. A pack of commands (or a changed one) cannot use `jarvis.http` to reach this
// computer's own servers (the window, the phone remote, a router, a printer, a local database) or a link-local address. A name is resolved
// and every address of it is checked; a redirect is checked at every step. (A name that changes its address between the check and the
// request - «DNS rebinding» - is not stopped by this: the packs of commands are checked by hash before they run, see integrity.rs.)
fn blocked_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v) => v.is_loopback() || v.is_private() || v.is_link_local() || v.is_unspecified() || v.is_broadcast() || v.is_multicast() || v.octets()[0] == 0 || (v.octets()[0] == 100 && (64..128).contains(&v.octets()[1])),
        IpAddr::V6(v) => {
            if let Some(m) = v.to_ipv4_mapped() {
                return blocked_ip(IpAddr::V4(m));
            }
            v.is_loopback() || v.is_unspecified() || v.is_multicast() || (v.segments()[0] & 0xfe00) == 0xfc00 || (v.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

pub(crate) fn check_url(url: &str) -> Result<(), String> {
    use std::net::{IpAddr, ToSocketAddrs};
    let u = reqwest::Url::parse(url).map_err(|_| "адрес не разобран".to_string())?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return Err("разрешены только адреса http и https".into());
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("адрес с логином и паролем не разрешён".into());
    }
    let host = u.host_str().ok_or("в адресе нет узла")?.trim_matches(|c| c == '[' || c == ']').to_lowercase();
    if let Ok(ip) = host.parse::<IpAddr>() {
        return if blocked_ip(ip) { Err("адрес этого компьютера или локальной сети не разрешён".into()) } else { Ok(()) };
    }
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".internal") || host.ends_with(".lan") || !host.contains('.') {
        return Err("адрес этого компьютера или локальной сети не разрешён".into());
    }
    let port = u.port_or_known_default().unwrap_or(443);
    if let Ok(addrs) = (host.as_str(), port).to_socket_addrs() {
        if addrs.into_iter().any(|a| blocked_ip(a.ip())) {
            return Err("имя ведёт на адрес этого компьютера или локальной сети".into());
        }
    }
    Ok(())
}

fn safe_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 {
                attempt.error("слишком много перенаправлений")
            } else if check_url(attempt.url().as_str()).is_err() {
                attempt.error("перенаправление на адрес этого компьютера или локальной сети не разрешено")
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|e| e.to_string())
}

pub fn register(lua: &Lua, jarvis: &Table) -> mlua::Result<()> {
    let http = lua.create_table()?;
    
    // jarvis.http.get(url, headers?)
    let get_fn = lua.create_function(|lua, (url, headers): (String, Option<Table>)| {
        http_request(lua, "GET", &url, None, headers)
    })?;
    http.set("get", get_fn)?;
    
    // jarvis.http.post(url, body, headers?)
    let post_fn = lua.create_function(|lua, (url, body, headers): (String, Option<String>, Option<Table>)| {
        http_request(lua, "POST", &url, body, headers)
    })?;
    http.set("post", post_fn)?;
    
    // jarvis.http.post_json(url, data, headers?)
    let post_json_fn = lua.create_function(|lua, (url, data, headers): (String, Table, Option<Table>)| {
        // convert Lua table to JSON string
        let json_value = table_to_json(lua, data)?;
        let body = serde_json::to_string(&json_value)
            .map_err(|e| mlua::Error::runtime(e.to_string()))?;
        
        // add content-type header
        let mut header_map: HashMap<String, String> = HashMap::new();
        header_map.insert("Content-Type".to_string(), "application/json".to_string());
        
        if let Some(h) = headers {
            for pair in h.pairs::<String, String>() {
                if let Ok((k, v)) = pair {
                    header_map.insert(k, v);
                }
            }
        }
        
        http_request_with_headers(lua, "POST", &url, Some(body), header_map)
    })?;
    http.set("post_json", post_json_fn)?;
    
    // jarvis.http.json(url) - GET + parse JSON
    let json_fn = lua.create_function(|lua, url: String| {
        check_url(&url).map_err(|why| mlua::Error::runtime(format!("Запрещено правилами безопасности: {}", why)))?;
        let client = safe_client().map_err(mlua::Error::runtime)?;

        let response = client.get(&url)
            .send()
            .map_err(|e| mlua::Error::runtime(e.to_string()))?;
        
        if response.status().is_success() {
            let json: serde_json::Value = response.json()
                .map_err(|e| mlua::Error::runtime(e.to_string()))?;
            json_to_lua(lua, json)
        } else {
            Ok(Value::Nil)
        }
    })?;
    http.set("json", json_fn)?;
    
    jarvis.set("http", http)?;
    
    Ok(())
}

fn http_request(
    lua: &Lua,
    method: &str,
    url: &str,
    body: Option<String>,
    headers: Option<Table>,
) -> mlua::Result<Table> {
    let header_map: HashMap<String, String> = if let Some(h) = headers {
        h.pairs::<String, String>()
            .filter_map(|r| r.ok())
            .collect()
    } else {
        HashMap::new()
    };
    
    http_request_with_headers(lua, method, url, body, header_map)
}

fn http_request_with_headers(
    lua: &Lua,
    method: &str,
    url: &str,
    body: Option<String>,
    headers: HashMap<String, String>,
) -> mlua::Result<Table> {
    if let Err(why) = check_url(url) {
        let denied = lua.create_table()?;
        denied.set("ok", false)?;
        denied.set("status", 0)?;
        denied.set("body", format!("Запрещено правилами безопасности: {}", why))?;
        denied.set("denied", true)?;
        return Ok(denied);
    }
    let client = safe_client().map_err(mlua::Error::runtime)?;

    let mut request = match method {
        "POST" => client.post(url),
        "PUT" => client.put(url),
        "DELETE" => client.delete(url),
        _ => client.get(url),
    };
    
    for (k, v) in headers {
        request = request.header(&k, &v);
    }
    
    if let Some(b) = body {
        request = request.body(b);
    }
    
    let result = lua.create_table()?;
    
    match request.send() {
        Ok(response) => {
            result.set("ok", response.status().is_success())?;
            result.set("status", response.status().as_u16())?;
            
            // get headers
            let headers_table = lua.create_table()?;
            for (name, value) in response.headers() {
                if let Ok(v) = value.to_str() {
                    headers_table.set(name.as_str(), v)?;
                }
            }
            result.set("headers", headers_table)?;
            
            // get body
            match response.text() {
                Ok(text) => result.set("body", text)?,
                Err(e) => result.set("body", format!("Error reading body: {}", e))?,
            }
        }
        Err(e) => {
            result.set("ok", false)?;
            result.set("status", 0)?;
            result.set("error", e.to_string())?;
            result.set("body", "")?;
        }
    }
    
    Ok(result)
}

// Convert Lua table to serde_json::Value
fn table_to_json(lua: &Lua, table: Table) -> mlua::Result<serde_json::Value> {
    use serde_json::{Value as JsonValue, Map};
    
    // check if it's an array (sequential integer keys starting from 1)
    let is_array = table.clone().pairs::<i64, Value>()
        .filter_map(|r| r.ok())
        .enumerate()
        .all(|(i, (k, _))| k == (i + 1) as i64);
    
    if is_array && table.len()? > 0 {
        let arr: Vec<JsonValue> = table.sequence_values::<Value>()
            .filter_map(|r| r.ok())
            .map(|v| lua_to_json(lua, v))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(JsonValue::Array(arr))
    } else {
        let mut map = Map::new();
        for pair in table.pairs::<String, Value>() {
            let (k, v) = pair?;
            map.insert(k, lua_to_json(lua, v)?);
        }
        Ok(JsonValue::Object(map))
    }
}

// Convert Lua Value to serde_json::Value
fn lua_to_json(lua: &Lua, value: Value) -> mlua::Result<serde_json::Value> {
    use serde_json::{Value as JsonValue, Number};
    
    match value {
        Value::Nil => Ok(JsonValue::Null),
        Value::Boolean(b) => Ok(JsonValue::Bool(b)),
        Value::Integer(i) => Ok(JsonValue::Number(Number::from(i))),
        Value::Number(n) => {
            Number::from_f64(n)
                .map(JsonValue::Number)
                .ok_or_else(|| mlua::Error::runtime("Invalid float"))
        }
        Value::String(s) => Ok(JsonValue::String(s.to_str()?.to_string())),
        Value::Table(t) => table_to_json(lua, t),
        _ => Err(mlua::Error::runtime("Unsupported type for JSON")),
    }
}

// Convert serde_json::Value to Lua Value
fn json_to_lua(lua: &Lua, json: serde_json::Value) -> mlua::Result<Value> {
    use serde_json::Value as JsonValue;
    
    match json {
        JsonValue::Null => Ok(Value::Nil),
        JsonValue::Bool(b) => Ok(Value::Boolean(b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Integer(i))
            } else if let Some(f) = n.as_f64() {
                Ok(Value::Number(f))
            } else {
                Ok(Value::Nil)
            }
        }
        JsonValue::String(s) => Ok(Value::String(lua.create_string(&s)?)),
        JsonValue::Array(arr) => {
            let table = lua.create_table()?;
            for (i, v) in arr.into_iter().enumerate() {
                table.set(i + 1, json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
        JsonValue::Object(map) => {
            let table = lua.create_table()?;
            for (k, v) in map {
                table.set(k, json_to_lua(lua, v)?)?;
            }
            Ok(Value::Table(table))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_script_reaches_the_internet_and_not_this_computer_or_the_local_network() {
        for bad in [
            "http://127.0.0.1:9713/poll", "http://localhost/", "http://[::1]:80/", "http://192.168.1.1/", "http://10.0.0.5/", "http://172.16.3.4/", "http://169.254.169.254/latest/meta-data/",
            "http://0.0.0.0/", "http://2130706433/", "http://0x7f.1/", "http://[::ffff:127.0.0.1]/", "http://printer.local/", "http://nas/", "http://user:pw@example.com/", "ftp://example.com/", "file:///C:/x", "not a url", "http://100.64.0.1/",
        ] {
            assert!(check_url(bad).is_err(), "{} must be refused", bad);
        }
        for good in ["https://api.open-meteo.com/v1/forecast?x=1", "http://1.1.1.1/", "https://example.com:8443/a"] {
            assert!(check_url(good).is_ok(), "{} must be allowed", good);
        }
    }
}
