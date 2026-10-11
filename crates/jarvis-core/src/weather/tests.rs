use super::*;
use std::io::{Read, Write};
use std::net::TcpListener;

#[test]
fn weather_codes_have_a_text_and_unknown_ones_are_not_invented() {
    assert_eq!(wmo(0, true).0, "ясно");
    assert_eq!(wmo(0, false).1, "🌙");
    assert_eq!(wmo(61, true).0, "дождь");
    assert_eq!(wmo(95, true).0, "гроза");
    assert_eq!(wmo(12345, true).0, "без описания");
}

#[test]
fn geocoding_and_forecast_answers_are_parsed() {
    let g = json!({ "results": [{ "name": "Ташкент", "country": "Узбекистан", "latitude": 41.26, "longitude": 69.22 }] });
    assert_eq!(parse_geocode(&g), Some(("Ташкент, Узбекистан".to_string(), 41.26, 69.22)));
    assert_eq!(parse_geocode(&json!({})), None, "a city that is not found is not guessed");
    assert_eq!(parse_geocode(&json!({ "results": [] })), None);
    let f = json!({ "current": { "temperature_2m": 8.44, "weather_code": 3, "is_day": 1, "wind_speed_10m": 2.5 } });
    let w = parse_forecast(&f, "Ташкент").unwrap();
    assert_eq!((w.temperature, w.text.as_str()), (8.4, "пасмурно"));
    assert!(w.updated > 0);
    assert_eq!(parse_forecast(&json!({ "current": {} }), "x"), None);
}

// a tiny local server that answers the two requests like open-meteo does
fn mock(geo_ok: bool) -> (String, String) {
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for _ in 0..2 {
            let Ok((mut s, _)) = listener.accept() else { return };
            let mut buf = [0u8; 4096];
            let n = s.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = if req.contains("/geo") {
                if geo_ok { json!({ "results": [{ "name": "Москва", "country": "Россия", "latitude": 55.75, "longitude": 37.61 }] }) } else { json!({}) }
            } else {
                json!({ "current": { "temperature_2m": -3.0, "weather_code": 71, "is_day": 0, "wind_speed_10m": 4.0 } })
            }
            .to_string();
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        }
    });
    (format!("http://127.0.0.1:{}/geo", port), format!("http://127.0.0.1:{}/api", port))
}

#[test]
fn the_whole_request_works_against_a_server_and_reports_a_missing_city() {
    let (geo, api) = mock(true);
    let w = fetch_from("Москва", &geo, &api).unwrap();
    assert_eq!((w.place.as_str(), w.temperature, w.text.as_str()), ("Москва, Россия", -3.0, "снег"));
    let (geo, api) = mock(false);
    let e = fetch_from("Несуществующий", &geo, &api).unwrap_err();
    assert!(e.contains("не найден"), "{}", e);
}

#[test]
fn no_internet_is_an_error_with_a_reason() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
    let e = fetch_from("Москва", &format!("http://127.0.0.1:{}/geo", port), &format!("http://127.0.0.1:{}/api", port)).unwrap_err();
    assert!(e.contains("нет соединения"), "{}", e);
}
