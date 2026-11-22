//! The cloud of OpenAI for the VOICE: the speech of Jarvis with the voices of OpenAI (onyx, ash, echo ...) and the recognition of
//! voice messages by its speech model (the Whisper family). Both are OPTIONAL and off until the owner gives a key:
//!   - the key is the own key of the voice (Settings → Voice) or, when there is none and the AI provider is OpenAI, the key of the AI;
//!   - the text of an answer (speech) and the sound of a voice message (recognition) go to OpenAI: the settings say so next to the key;
//!   - when the cloud cannot speak or recognise, the own voice (Piper) and the offline recognizer (Vosk) take over.
//!
//! Speech: POST https://api.openai.com/v1/audio/speech, JSON {model, input, voice, instructions, response_format: "wav"}, the answer is
//! a WAV file. Recognition: POST https://api.openai.com/v1/audio/transcriptions, multipart (model, language, file), the answer is
//! JSON {"text": ...}. The multipart body is built here (the multipart feature of the HTTP client is not in the build).

use std::path::Path;

pub const PREFIX: &str = "openai:";
const SPEECH: &str = "https://api.openai.com/v1/audio/speech";
const TRANSCRIBE: &str = "https://api.openai.com/v1/audio/transcriptions";
const SPEECH_MODEL: &str = "gpt-4o-mini-tts";
const TRANSCRIBE_MODEL: &str = "gpt-4o-mini-transcribe";
const MAX_INPUT_CHARS: usize = 3800;

// (voice id of the service, name shown in the settings). The male voices come first: they are the ones the owner asked for.
pub const VOICES: [(&str, &str); 6] = [
    ("onyx", "Оникс (мужской, низкий, OpenAI, облако)"),
    ("ash", "Эш (мужской, OpenAI, облако)"),
    ("echo", "Эхо (мужской, OpenAI, облако)"),
    ("ballad", "Баллад (мужской, мягкий, OpenAI, облако)"),
    ("nova", "Нова (женский, OpenAI, облако)"),
    ("shimmer", "Шиммер (женский, OpenAI, облако)"),
];

// "openai:onyx" -> "onyx" (only the known voices: the id becomes a part of a request)
pub fn voice_of(choice: &str) -> Option<&'static str> {
    let name = choice.strip_prefix(PREFIX)?;
    VOICES.iter().find(|(id, _)| *id == name).map(|(id, _)| *id)
}

// The request of the speech. `instructions` keep the character of the butler in every phrase (the model can be told how to speak).
pub fn speech_body(text: &str, voice: &str) -> serde_json::Value {
    serde_json::json!({
        "model": SPEECH_MODEL,
        "input": text.chars().take(MAX_INPUT_CHARS).collect::<String>(),
        "voice": voice,
        "response_format": "wav",
        "instructions": "Говори по-русски спокойно, вежливо и чётко, как невозмутимый британский дворецкий. Не торопись, не читай знаки разметки вслух."
    })
}

// a file of speech must look like one: «RIFF....WAVE» and not just a few bytes of an error page
pub fn looks_like_wav(bytes: &[u8]) -> bool {
    bytes.len() > 1000 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE"
}

// what a refusal of the service means for the person
pub fn explain(status: u16) -> String {
    match status {
        401 => "OpenAI не принял ключ (проверьте ключ)".into(),
        403 => "у ключа нет доступа к голосу или распознаванию речи (проверьте права и страну)".into(),
        402 | 429 => "на аккаунте OpenAI нет средств или исчерпан лимит запросов".into(),
        400 | 404 | 422 => "OpenAI не знает такого голоса или текст не подошёл".into(),
        413 => "звук слишком большой для распознавания".into(),
        500..=599 => format!("сервер OpenAI не отвечает ({})", status),
        _ => format!("ответ сервера OpenAI {}", status),
    }
}

fn client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(8)).timeout(std::time::Duration::from_secs(45)).build().map_err(|e| e.to_string())
}

fn check_endpoint(endpoint: &str) -> Result<(), String> {
    if endpoint.starts_with("https://") || endpoint.starts_with("http://127.0.0.1") {
        Ok(())
    } else {
        Err("адрес облака должен начинаться с https://".into())
    }
}

#[cfg(feature = "lua")]
pub fn synth_at(endpoint: &str, key: &str, text: &str, voice: &str, out: &Path) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("не сохранён ключ OpenAI".into());
    }
    check_endpoint(endpoint)?;
    let resp = client()?
        .post(endpoint)
        .bearer_auth(key.trim())
        .header("Content-Type", "application/json")
        .body(speech_body(text, voice).to_string())
        .send()
        .map_err(|_| "нет связи с сервером OpenAI".to_string())?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(explain(status));
    }
    let bytes = resp.bytes().map_err(|_| "ответ OpenAI оборвался".to_string())?;
    if !looks_like_wav(&bytes) {
        return Err("OpenAI прислал не звук".into());
    }
    std::fs::write(out, &bytes).map_err(|e| e.to_string())
}

#[cfg(not(feature = "lua"))]
pub fn synth_at(_e: &str, _k: &str, _t: &str, _v: &str, _o: &Path) -> Result<(), String> {
    Err("в этой сборке нет сетевого модуля".into())
}

pub fn synth(key: &str, text: &str, voice: &str, out: &Path) -> Result<(), String> {
    synth_at(SPEECH, key, text, voice, out)
}

// ---------------------------------------------------------------------------------------------
// recognition
// ---------------------------------------------------------------------------------------------

// 16-bit mono PCM samples -> a WAV file
pub fn wav_of(samples: &[i16], rate: u32) -> Vec<u8> {
    let len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + samples.len() * 2);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

// a multipart/form-data body: text fields and one file
pub fn multipart_body(boundary: &str, fields: &[(&str, &str)], file_field: &str, file_name: &str, content_type: &str, file: &[u8]) -> Vec<u8> {
    let mut b: Vec<u8> = Vec::with_capacity(file.len() + 512);
    for (name, value) in fields {
        b.extend_from_slice(format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"\r\n\r\n{}\r\n", boundary, name, value).as_bytes());
    }
    b.extend_from_slice(format!("--{}\r\nContent-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\nContent-Type: {}\r\n\r\n", boundary, file_field, file_name, content_type).as_bytes());
    b.extend_from_slice(file);
    b.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());
    b
}

// the text out of the answer: {"text": "..."} (the field is absent in an error answer)
pub fn text_of_answer(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let t = v.get("text")?.as_str()?.trim().to_string();
    if t.is_empty() {
        None
    } else {
        Some(t)
    }
}

#[cfg(feature = "lua")]
pub fn transcribe_at(endpoint: &str, key: &str, samples: &[i16], lang: &str) -> Result<String, String> {
    if key.trim().is_empty() {
        return Err("не сохранён ключ OpenAI".into());
    }
    check_endpoint(endpoint)?;
    if samples.is_empty() {
        return Err("нет звука".into());
    }
    let boundary = format!("----jarvis{:016x}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1));
    let lang: String = lang.chars().filter(|c| c.is_ascii_alphabetic()).take(3).collect();
    let mut fields = vec![("model", TRANSCRIBE_MODEL), ("response_format", "json")];
    if !lang.is_empty() {
        fields.push(("language", lang.as_str()));
    }
    let body = multipart_body(&boundary, &fields, "file", "voice.wav", "audio/wav", &wav_of(samples, 16_000));
    let resp = client()?
        .post(endpoint)
        .bearer_auth(key.trim())
        .header("Content-Type", format!("multipart/form-data; boundary={}", boundary))
        .body(body)
        .send()
        .map_err(|_| "нет связи с сервером OpenAI".to_string())?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(explain(status));
    }
    let answer = resp.text().map_err(|_| "ответ OpenAI оборвался".to_string())?;
    text_of_answer(&answer).ok_or_else(|| "OpenAI не услышал речи".to_string())
}

#[cfg(not(feature = "lua"))]
pub fn transcribe_at(_e: &str, _k: &str, _s: &[i16], _l: &str) -> Result<String, String> {
    Err("в этой сборке нет сетевого модуля".into())
}

pub fn transcribe(key: &str, samples: &[i16], lang: &str) -> Result<String, String> {
    transcribe_at(TRANSCRIBE, key, samples, lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_voices_are_taken() {
        assert_eq!(voice_of("openai:onyx"), Some("onyx"));
        assert_eq!(voice_of("openai:ballad"), Some("ballad"));
        assert_eq!(voice_of("openai:../x"), None);
        assert_eq!(voice_of("openai:"), None);
        assert_eq!(voice_of("yandex:alena"), None);
        assert_eq!(voice_of("onyx"), None);
        // the male voices come first
        assert!(VOICES[..4].iter().all(|(_, l)| l.contains("мужской")));
    }

    #[test]
    fn the_speech_request_is_a_wav_with_the_voice_and_a_cut_text_and_no_key() {
        let b = speech_body(&"я".repeat(9000), "onyx");
        assert_eq!(b["voice"], "onyx");
        assert_eq!(b["response_format"], "wav");
        assert_eq!(b["model"], SPEECH_MODEL);
        assert_eq!(b["input"].as_str().unwrap().chars().count(), MAX_INPUT_CHARS);
        assert!(!b.to_string().to_lowercase().contains("bearer") && !b.to_string().contains("sk-"));
    }

    #[test]
    fn a_wav_is_told_from_an_error_page() {
        let w = wav_of(&vec![0i16; 2000], 16_000);
        assert!(looks_like_wav(&w));
        assert!(!looks_like_wav(b"{\"error\":\"nope\"}") && !looks_like_wav(&[]) && !looks_like_wav(&vec![0u8; 2000]));
    }

    #[test]
    fn the_wav_header_is_right() {
        let w = wav_of(&[1, -1, 2], 16_000);
        assert_eq!(w.len(), 44 + 6);
        assert_eq!(&w[..4], b"RIFF");
        assert_eq!(u32::from_le_bytes([w[4], w[5], w[6], w[7]]), 36 + 6);
        assert_eq!(u32::from_le_bytes([w[24], w[25], w[26], w[27]]), 16_000);
        assert_eq!(u32::from_le_bytes([w[40], w[41], w[42], w[43]]), 6);
        assert_eq!(i16::from_le_bytes([w[46], w[47]]), -1);
    }

    #[test]
    fn the_multipart_body_has_the_fields_and_the_file_between_boundaries() {
        let b = multipart_body("BB", &[("model", "m"), ("language", "ru")], "file", "voice.wav", "audio/wav", b"DATA");
        let s = String::from_utf8_lossy(&b).to_string();
        assert!(s.starts_with("--BB\r\nContent-Disposition: form-data; name=\"model\"\r\n\r\nm\r\n"));
        assert!(s.contains("name=\"language\"\r\n\r\nru\r\n"));
        assert!(s.contains("name=\"file\"; filename=\"voice.wav\"\r\nContent-Type: audio/wav\r\n\r\nDATA\r\n--BB--\r\n"));
        assert!(s.ends_with("--BB--\r\n"));
    }

    #[test]
    fn the_text_is_taken_from_the_answer() {
        assert_eq!(text_of_answer("{\"text\":\"  привет, джарвис \"}"), Some("привет, джарвис".to_string()));
        assert_eq!(text_of_answer("{\"text\":\"\"}"), None);
        assert_eq!(text_of_answer("{\"error\":{\"message\":\"x\"}}"), None);
        assert_eq!(text_of_answer("not json"), None);
    }

    #[test]
    fn the_refusals_are_explained_in_words() {
        assert!(explain(401).contains("ключ") && explain(429).contains("лимит") && explain(503).contains("503"));
    }

    #[test]
    fn a_plain_http_address_of_the_cloud_is_refused_but_the_local_test_server_is_not() {
        assert!(check_endpoint("http://api.openai.com/x").is_err());
        assert!(check_endpoint("https://api.openai.com/x").is_ok());
        assert!(check_endpoint("http://127.0.0.1:9999/x").is_ok());
    }
}
