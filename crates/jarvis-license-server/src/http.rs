//! A thin HTTP loop over tiny_http. The server listens on a local address behind a reverse proxy that terminates TLS
//! (Caddy / nginx): the program of the customer talks https to the proxy only. Nothing secret is ever logged here.
use crate::api::{Api, Request, Response};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

pub const MAX_BODY: u64 = 16 * 1024;

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn serve_one(api: &Api, mut rq: tiny_http::Request, ip_header: &str) {
    let mut headers = HashMap::new();
    for h in rq.headers() {
        headers.insert(h.field.as_str().as_str().to_lowercase(), h.value.as_str().to_string());
    }
    let ip = if !ip_header.is_empty() { headers.get(&ip_header.to_lowercase()).map(|v| v.split(',').next().unwrap_or("").trim().to_string()).unwrap_or_default() } else { String::new() };
    let ip = if ip.is_empty() { rq.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default() } else { ip };
    let mut body = String::new();
    let too_big = rq.body_length().map(|l| l as u64 > MAX_BODY).unwrap_or(false);
    if !too_big {
        let _ = rq.as_reader().take(MAX_BODY).read_to_string(&mut body);
    }
    let req = Request { method: rq.method().as_str().to_uppercase(), path: rq.url().to_string(), headers, body, ip };
    let resp: Response = if too_big { Response { status: 413, body: r#"{"ok":false,"code":"TOO_BIG","message":"запрос слишком большой"}"#.into() } } else { api.handle(&req, now()) };
    let mut out = tiny_http::Response::from_string(resp.body).with_status_code(resp.status);
    out.add_header(tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json; charset=utf-8"[..]).unwrap());
    out.add_header(tiny_http::Header::from_bytes(&b"Cache-Control"[..], &b"no-store"[..]).unwrap());
    let _ = rq.respond(out);
}

// runs forever: one thread per request pulled from the listener (the database serializes the writes)
pub fn run(api: Arc<Api>, bind: &str, ip_header: String) -> Result<(), String> {
    let server = tiny_http::Server::http(bind).map_err(|e| format!("не удалось слушать {}: {}", bind, e))?;
    let server = Arc::new(server);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let (srv, api, hdr) = (server.clone(), api.clone(), ip_header.clone());
        handles.push(std::thread::spawn(move || {
            while let Ok(rq) = srv.recv() {
                serve_one(&api, rq, &hdr);
            }
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

// for the integration tests: a server on a free local port, returns its address
pub fn spawn_test_server(api: Arc<Api>) -> String {
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("bind"));
    let addr = format!("http://{}", server.server_addr().to_ip().expect("ip"));
    for _ in 0..4 {
        let (srv, api) = (server.clone(), api.clone());
        std::thread::spawn(move || {
            while let Ok(rq) = srv.recv() {
                serve_one(&api, rq, "");
            }
        });
    }
    addr
}
