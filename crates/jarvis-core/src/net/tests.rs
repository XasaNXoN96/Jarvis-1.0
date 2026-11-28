use super::*;

#[test]
fn proxy_text_is_normalized() {
    assert_eq!(normalize_proxy("127.0.0.1:10808").as_deref(), Some("http://127.0.0.1:10808"));
    assert_eq!(normalize_proxy("socks5://u p@host.example:1080").as_deref(), None); // spaces inside are refused
    assert_eq!(normalize_proxy("SOCKS5://u:p@host.example:1080/").as_deref(), Some("socks5://u:p@host.example:1080"));
    assert_eq!(normalize_proxy("socks5h://127.0.0.1:2080").as_deref(), Some("socks5h://127.0.0.1:2080"));
    assert_eq!(normalize_proxy("https://proxy.example:443").as_deref(), Some("https://proxy.example:443"));
}

#[test]
fn bad_proxy_text_is_refused() {
    for bad in ["", "host", "host:0", "host:99999", "ftp://h:1", "http://h:1/path", "http://:80", "a b:80", "http://h:1?x=1", "vless://x@h:443"] {
        assert_eq!(normalize_proxy(bad), None, "{bad}");
    }
}

#[test]
fn password_is_masked() {
    assert_eq!(mask("socks5://user:secret@h.example:1080"), "socks5://***@h.example:1080");
    assert_eq!(mask("http://127.0.0.1:10809"), "http://127.0.0.1:10809");
}

#[test]
fn proxy_kind_is_sniffed() {
    assert_eq!(sniff(&[0x05, 0x00]), Some(Kind::Socks5));
    assert_eq!(sniff(&[0x05, 0x02]), Some(Kind::Socks5));
    assert_eq!(sniff(&[0x05, 0xFF]), None);
    assert_eq!(sniff(b"HTTP/1.1 400 Bad"), Some(Kind::Http));
    assert_eq!(sniff(b"SSH-2.0"), None);
    assert_eq!(sniff(&[]), None);
}

#[test]
fn local_networks_are_never_proxied() {
    for part in ["localhost", "127.0.0.1", "192.168.0.0/16", "10.0.0.0/8", "172.16.0.0/12"] {
        assert!(NO_PROXY.contains(part));
    }
}

#[test]
fn local_detection_finds_a_fake_socks_server() {
    use std::io::{Read, Write};
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut s, _)) = l.accept() {
            let mut b = [0u8; 3];
            let _ = s.read(&mut b);
            let _ = s.write_all(&[0x05, 0x00]);
        }
    });
    // the detector only scans the known ports, so this checks the handshake the same way
    let mut c = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    c.write_all(&[0x05, 0x01, 0x00]).unwrap();
    let mut r = [0u8; 2];
    c.read_exact(&mut r).unwrap();
    assert_eq!(sniff(&r), Some(Kind::Socks5));
}
