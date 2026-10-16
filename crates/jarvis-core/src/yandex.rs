//! A cloud voice: Yandex SpeechKit (the voices of the Alice line). It is OPTIONAL: it is used only when the user chose one of its
//! voices in Settings → Voice and saved his own API key (the key is kept like the other keys: encrypted by Windows, never shown).
//! The text of the answer goes to Yandex (that is how a cloud voice works) — the settings say so next to the key. When the cloud
//! voice cannot speak (no key, no internet, a refusal), Jarvis's own Piper voice takes over.
//!
//! The request: POST https://tts.api.cloud.yandex.net/speech/v1/tts:synthesize with the header `Authorization: Api-Key <key>` and a
//! form (text, lang, voice, format=lpcm, sampleRateHertz, speed). The answer is raw 16-bit mono PCM, which is wrapped into a WAV
//! file here and played like every other voice.

use std::path::Path;

pub const PREFIX: &str = "yandex:";
const ENDPOINT: &str = "https://tts.api.cloud.yandex.net/speech/v1/tts:synthesize";
const RATE: u32 = 48_000;

// (voice id of the service, name shown in the settings). The list of voices of the service changes: a voice that the service
// no longer knows is answered with an error, which the user sees.
pub const VOICES: [(&str, &str); 6] = [
    ("alena", "Алёна (женский, Яндекс, облако)"),
    ("jane", "Джейн (женский, Яндекс, облако)"),
    ("oksana", "Оксана (женский, Яндекс, облако)"),
    ("filipp", "Филипп (мужской, Яндекс, облако)"),
    ("ermil", "Ермил (мужской, Яндекс, облако)"),
    ("zahar", "Захар (мужской, Яндекс, облако)"),
];

// "yandex:alena" -> "alena" (only the known voices: the id becomes a part of a request)
pub fn voice_of(choice: &str) -> Option<&'static str> {
    let name = choice.strip_prefix(PREFIX)?;
    VOICES.iter().find(|(id, _)| *id == name).map(|(id, _)| *id)
}

// the form of the request (no key in it: the key is a header)
pub fn form(text: &str, voice: &str, speed: f32) -> Vec<(&'static str, String)> {
    vec![
        ("text", text.chars().take(4500).collect()),
        ("lang", "ru-RU".to_string()),
        ("voice", voice.to_string()),
        ("format", "lpcm".to_string()),
        ("sampleRateHertz", RATE.to_string()),
        ("speed", format!("{:.2}", speed.clamp(0.5, 2.0))),
    ]
}

// application/x-www-form-urlencoded (the reqwest form feature is not in the build: a small encoder of our own)
pub fn encode_form(fields: &[(&'static str, String)]) -> String {
    let enc = |s: &str| -> String {
        s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect()
    };
    fields.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

// raw 16-bit mono PCM -> a WAV file
pub fn wav_of(pcm: &[u8]) -> Vec<u8> {
    let len = pcm.len() as u32;
    let mut w = Vec::with_capacity(44 + pcm.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&RATE.to_le_bytes());
    w.extend_from_slice(&(RATE * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&len.to_le_bytes());
    w.extend_from_slice(pcm);
    w
}

// what a refusal of the service means for the person
pub fn explain(status: u16) -> String {
    match status {
        401 | 403 => "Яндекс не принял ключ (проверьте ключ и права сервисного аккаунта на SpeechKit)".into(),
        402 => "на аккаунте Яндекса нет средств или не подключена оплата".into(),
        429 => "лимит запросов Яндекса исчерпан, попробуйте позже".into(),
        400 | 404 | 422 => "Яндекс не знает такого голоса или текст не подошёл".into(),
        500..=599 => format!("сервер Яндекса не отвечает ({})", status),
        _ => format!("ответ сервера Яндекса {}", status),
    }
}

#[cfg(feature = "lua")]
pub fn synth_at(endpoint: &str, key: &str, text: &str, voice: &str, speed: f32, out: &Path) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("не сохранён ключ Яндекса".into());
    }
    if !endpoint.starts_with("https://") && !endpoint.starts_with("http://127.0.0.1") {
        return Err("адрес облачного голоса должен начинаться с https://".into());
    }
    let client = reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(8)).timeout(std::time::Duration::from_secs(40)).build().map_err(|e| e.to_string())?;
    let resp = client
        .post(endpoint)
        .header("Authorization", format!("Api-Key {}", key.trim()))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(encode_form(&form(text, voice, speed)))
        .send()
        .map_err(|_| "нет связи с сервером Яндекса".to_string())?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(explain(status));
    }
    let pcm = resp.bytes().map_err(|_| "ответ Яндекса оборвался".to_string())?;
    // an answer of a few bytes is not speech (a short error page, an empty body)
    if pcm.len() < 1000 {
        return Err("Яндекс прислал пустой звук".into());
    }
    std::fs::write(out, wav_of(&pcm)).map_err(|e| e.to_string())
}

#[cfg(not(feature = "lua"))]
pub fn synth_at(_e: &str, _k: &str, _t: &str, _v: &str, _s: f32, _o: &Path) -> Result<(), String> {
    Err("в этой сборке нет сетевого модуля".into())
}

pub fn synth(key: &str, text: &str, voice: &str, speed: f32, out: &Path) -> Result<(), String> {
    synth_at(ENDPOINT, key, text, voice, speed, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_voices_are_taken() {
        assert_eq!(voice_of("yandex:alena"), Some("alena"));
        assert_eq!(voice_of("yandex:zahar"), Some("zahar"));
        assert_eq!(voice_of("yandex:../x"), None);
        assert_eq!(voice_of("yandex:"), None);
        assert_eq!(voice_of("piper:denis"), None);
        assert_eq!(voice_of(""), None);
    }

    #[test]
    fn the_form_has_the_voice_the_format_and_a_safe_speed_and_never_the_key() {
        let f = form("привет", "alena", 9.0);
        let get = |k: &str| f.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone()).unwrap();
        assert_eq!((get("voice").as_str(), get("format").as_str(), get("lang").as_str()), ("alena", "lpcm", "ru-RU"));
        assert_eq!(get("sampleRateHertz"), "48000");
        assert_eq!(get("speed"), "2.00", "the speed is kept in the range");
        assert!(f.iter().all(|(_, v)| !v.to_lowercase().contains("api-key")));
        assert_eq!(form(&"а".repeat(9000), "alena", 1.0)[0].1.chars().count(), 4500, "a long text is cut");
    }

    #[test]
    fn the_form_is_encoded_so_that_a_text_cannot_add_a_field() {
        let body = encode_form(&form("а&voice=evil=1 +%", "alena", 1.0));
        assert!(body.starts_with("text=%D0%B0%26voice%3Devil%3D1%20%2B%25&lang=ru-RU&voice=alena&"));
        assert_eq!(body.matches("&voice=").count(), 1);
    }

    #[test]
    fn the_wav_header_matches_the_pcm() {
        let w = wav_of(&[0u8; 100]);
        assert_eq!(w.len(), 144);
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..16], b"WAVEfmt ");
        assert_eq!(u32::from_le_bytes([w[4], w[5], w[6], w[7]]), 136);
        assert_eq!(u32::from_le_bytes([w[24], w[25], w[26], w[27]]), RATE);
        assert_eq!(&w[36..40], b"data");
        assert_eq!(u32::from_le_bytes([w[40], w[41], w[42], w[43]]), 100);
    }

    #[test]
    fn refusals_are_explained_in_words() {
        assert!(explain(401).contains("не принял ключ"));
        assert!(explain(402).contains("оплат"));
        assert!(explain(429).contains("лимит"));
        assert!(explain(503).contains("503"));
    }

    // a tiny local server: it answers one request with the given status and body and hands back the request it got
    #[cfg(feature = "lua")]
    fn one_answer(status: u16, body: Vec<u8>) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/", listener.local_addr().unwrap().port());
        let h = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let (mut got, mut buf) = (Vec::new(), [0u8; 4096]);
            loop {
                let n = s.read(&mut buf).unwrap();
                got.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&got).to_string();
                if let Some(i) = text.find("\r\n\r\n") {
                    let want = text.lines().find_map(|l| l.to_lowercase().strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                    if got.len() >= i + 4 + want {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let head = format!("HTTP/1.1 {} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", status, body.len());
            let _ = s.write_all(head.as_bytes());
            let _ = s.write_all(&body);
            String::from_utf8_lossy(&got).to_string()
        });
        (url, h)
    }

    #[cfg(feature = "lua")]
    #[test]
    fn a_good_answer_becomes_a_wav_and_the_key_goes_only_in_the_header() {
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        let (url, h) = one_answer(200, vec![1u8; 4000]);
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("a.wav");
        synth_at(&url, "secret-key-123", "Здравствуйте", "alena", 1.0, &out).unwrap();
        let req = h.join().unwrap();
        assert!(req.to_lowercase().contains("authorization: api-key secret-key-123"));
        assert!(req.contains("voice=alena") && req.contains("format=lpcm"));
        let body = req.split("\r\n\r\n").nth(1).unwrap_or("");
        assert!(!body.contains("secret-key-123"), "the key is not in the form");
        assert_eq!(std::fs::read(&out).unwrap().len(), 44 + 4000);
    }

    #[cfg(feature = "lua")]
    #[test]
    fn a_refusal_and_an_empty_answer_are_errors_and_leave_no_file() {
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost");
        let d = tempfile::tempdir().unwrap();
        let out = d.path().join("b.wav");
        let (url, h) = one_answer(401, b"{}".to_vec());
        let e = synth_at(&url, "k", "текст", "alena", 1.0, &out).unwrap_err();
        let _ = h.join();
        assert!(e.contains("не принял ключ"));
        let (url, h) = one_answer(200, vec![0u8; 10]);
        assert!(synth_at(&url, "k", "текст", "alena", 1.0, &out).unwrap_err().contains("пустой"));
        let _ = h.join();
        assert!(!out.exists());
        assert!(synth_at("http://example.com/", "k", "т", "alena", 1.0, &out).is_err(), "plain http to a foreign host is refused");
        assert!(synth_at(&url, "  ", "т", "alena", 1.0, &out).unwrap_err().contains("ключ"));
    }
}
