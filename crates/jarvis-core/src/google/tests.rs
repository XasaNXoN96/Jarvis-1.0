use super::*;

#[test]
fn the_pkce_challenge_is_the_one_of_the_rfc() {
    // RFC 7636, appendix B
    assert_eq!(pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
}

#[test]
fn base64_of_the_url_alphabet_has_no_padding() {
    assert_eq!(b64url(b""), "");
    assert_eq!(b64url(b"f"), "Zg");
    assert_eq!(b64url(b"fo"), "Zm8");
    assert_eq!(b64url(b"foo"), "Zm9v");
    assert_eq!(b64url(&[0xfb, 0xff, 0xfe]), "-__-");
}

#[test]
fn random_strings_are_long_enough_and_of_the_allowed_characters_and_not_the_same() {
    let (a, b) = (random_string(64), random_string(64));
    assert_eq!(a.len(), 64);
    assert_ne!(a, b);
    assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || "-._~".contains(c)));
}

#[test]
fn percent_encoding_round_trips() {
    for s in ["abc", "a b&c=d/é", "https://x.y/z?q=1", "+%"] {
        assert_eq!(pct_decode(&pct(s)), s);
    }
    assert_eq!(pct("a b"), "a%20b");
    assert_eq!(pct_decode("a+b%20c%"), "a b c%");
    assert_eq!(pct_decode("%zz%4"), "%zz%4");
}

#[test]
fn the_sign_in_address_has_everything_and_no_secret() {
    let u = auth_url("id-1.apps.googleusercontent.com", "http://127.0.0.1:5555/", "CHAL", "STATE");
    assert!(u.starts_with("https://accounts.google.com/o/oauth2/v2/auth?"));
    for part in ["client_id=id-1.apps.googleusercontent.com", "redirect_uri=http%3A%2F%2F127.0.0.1%3A5555%2F", "response_type=code", "code_challenge=CHAL", "code_challenge_method=S256", "access_type=offline", "prompt=consent", "state=STATE"] {
        assert!(u.contains(part), "{} is missing in {}", part, u);
    }
    assert!(u.contains("calendar.readonly") && u.contains("gmail.readonly") && !u.contains("gmail.modify") && !u.contains("gmail.send"), "read-only scopes only: {}", u);
    assert!(!u.to_lowercase().contains("secret"));
}

#[test]
fn the_callback_gives_the_code_only_for_our_state() {
    assert_eq!(parse_callback("GET /?code=4%2F0AbC&state=S1 HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n", "S1"), Ok("4/0AbC".to_string()));
    assert!(parse_callback("GET /?code=x&state=OTHER HTTP/1.1", "S1").unwrap_err().starts_with("чужой"));
    assert!(parse_callback("GET /?code=x HTTP/1.1", "S1").unwrap_err().starts_with("чужой"), "no state: not ours");
    assert!(parse_callback("GET /favicon.ico HTTP/1.1", "S1").unwrap_err().starts_with("в запросе"));
    assert!(parse_callback("POST /?code=x&state=S1 HTTP/1.1", "S1").unwrap_err().starts_with("не тот"));
    assert!(parse_callback("GET /?error=access_denied&state=S1 HTTP/1.1", "S1").unwrap_err().contains("отказали"));
    assert!(parse_callback("GET /?code=&state=S1 HTTP/1.1", "S1").is_err());
    assert!(parse_callback("", "S1").is_err());
}

#[test]
fn the_token_answers_are_read_and_the_errors_explained() {
    let t = parse_token_response(r#"{"access_token":"ya29.x","expires_in":3599,"refresh_token":"1//r","scope":"s","token_type":"Bearer"}"#).unwrap();
    assert_eq!((t.access.as_str(), t.expires_in, t.refresh.as_deref()), ("ya29.x", 3599, Some("1//r")));
    let t2 = parse_token_response(r#"{"access_token":"a"}"#).unwrap();
    assert_eq!((t2.expires_in, t2.refresh), (3000, None));
    assert!(parse_token_response(r#"{"error":"invalid_client"}"#).unwrap_err().contains("идентификатор и секрет"));
    assert!(parse_token_response(r#"{"error":"invalid_grant"}"#).unwrap_err().contains("заново"));
    assert!(parse_token_response("<html>").is_err() && parse_token_response("{}").is_err());
}

#[test]
fn the_form_is_encoded_and_has_the_grant() {
    let f = form(&[("code", "4/0A b"), ("grant_type", "authorization_code")]);
    assert_eq!(f, "code=4%2F0A%20b&grant_type=authorization_code");
}

#[test]
fn the_events_are_told_with_their_time_and_the_cancelled_ones_are_skipped() {
    let body = r#"{"items":[
        {"summary":"Встреча  с командой","start":{"dateTime":"2026-11-22T10:00:00+05:00"}},
        {"summary":"День рождения","start":{"date":"2026-11-23"}},
        {"summary":"Отменено","status":"cancelled","start":{"dateTime":"2026-11-22T11:00:00+05:00"}},
        {"start":{"dateTime":"2026-11-22T12:30:00Z"}}
    ]}"#;
    assert_eq!(parse_events(body), vec!["в 10:00 — Встреча с командой", "весь день — День рождения", "в 12:30 — без названия"]);
    assert!(parse_events("").is_empty() && parse_events("{}").is_empty() && parse_events("[1]").is_empty());
}

#[test]
fn the_mail_answers_are_read() {
    assert_eq!(parse_unread(r#"{"id":"INBOX","messagesUnread":12}"#), Some(12));
    assert_eq!(parse_unread("{}"), None);
    assert_eq!(parse_message_ids(r#"{"messages":[{"id":"18c4f"},{"id":"../x"},{"id":"a1"}]}"#), vec!["18c4f", "a1"], "an id with strange characters is never used in an address");
    assert_eq!(sender_name("Иван Петров <ivan@x.ru>"), "Иван Петров");
    assert_eq!(sender_name("\"Bank\" <b@x.com>"), "Bank");
    assert_eq!(sender_name("<a@b.c>"), "a@b.c");
    let m = r#"{"payload":{"headers":[{"name":"From","value":"Иван <i@x.ru>"},{"name":"subject","value":"  Счёт   на оплату "}]}}"#;
    assert_eq!(parse_message_line(m).as_deref(), Some("от Иван: Счёт на оплату"));
    assert_eq!(parse_message_line(r#"{"payload":{"headers":[]}}"#), None);
}
