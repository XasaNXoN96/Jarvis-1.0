use super::*;

#[test]
fn word_autocorrect_is_not_a_mismatch() {
    let typed = "Он сказал \"привет\" - и ушёл... It's fine.\nВторой абзац";
    let word = "Он сказал \u{201C}привет\u{201D} \u{2013} и ушёл\u{2026} It\u{2019}s fine.\nВторой абзац\r";
    assert!(fold(word).contains(&fold(typed)));
    assert!(fold("a\u{00A0}b  c").contains(&fold("a b c")));
}

#[test]
fn real_differences_are_still_found() {
    let typed = "Привет, мир";
    for word in ["Привет, мор", "Привет мир", "Привет, мир!", "", "Другой текст"] {
        if word == "Привет, мир!" {
            assert!(fold(word).contains(&fold(typed)), "text that only continues is still a hit");
        } else {
            assert!(!fold(word).contains(&fold(typed)), "{:?}", word);
        }
    }
    // a changed digit, a missing word and a changed quote CONTENT are not folded away
    assert!(!fold("Цена 100 рублей").contains(&fold("Цена 101 рублей")));
    assert!(!fold("он сказал «да»").contains(&fold("он сказал «нет»")));
}

#[test]
fn base64_and_hashes_round_trip() {
    for s in ["", "a", "ab", "abc", "Привет, мир 😀\nстрока"] {
        assert_eq!(b64_decode(&b64_encode(s.as_bytes())).unwrap(), s.as_bytes());
    }
    assert!(b64_decode("!!!").is_none());
    assert_eq!(text_hash("a\r\nb\r"), text_hash("a\nb"));
    assert_ne!(text_hash("a"), text_hash("b"));
}
