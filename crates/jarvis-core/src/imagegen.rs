//! Generation of pictures (and, separately, video) through the AI provider of the user. Pictures: the «images/generations» call of an
//! OpenAI-compatible provider (OpenAI, Gemini's compatible endpoint, or the user's own server). The result is checked to be a real PNG /
//! JPEG / WebP before it is saved, and the file path is returned, never a promise. Video has its own adapter that says what is missing:
//! there is no video generation call common to the supported providers, so nothing is faked.

use crate::{perms, store};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const FILE: &str = "imagegen.json";
pub const MAX_PROMPT: usize = 1500;
const MAX_IMAGE: usize = 25 * 1024 * 1024;

#[derive(Serialize, Deserialize, Default)]
struct Conf {
    #[serde(default)]
    model: String,
    #[serde(default)]
    size: String,
}

pub fn set_model(model: &str) -> Result<(), String> {
    let m = model.trim();
    if m.len() > 80 || m.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err("Название модели без пробелов, до 80 символов.".into());
    }
    let mut c: Conf = store::read::<Conf>(FILE).0;
    c.model = m.to_string();
    store::write(FILE, &c)
}

pub fn default_model(provider: &str) -> &'static str {
    match provider {
        "openai" => "gpt-image-1",
        "gemini" => "imagen-3.0-generate-002",
        _ => "",
    }
}

pub fn model_for(provider: &str) -> String {
    let c: Conf = store::read::<Conf>(FILE).0;
    if c.model.is_empty() { default_model(provider).to_string() } else { c.model }
}

pub fn request_body(model: &str, prompt: &str, size: &str) -> Value {
    let mut b = json!({ "model": model, "prompt": prompt, "n": 1 });
    if !size.is_empty() {
        b["size"] = json!(size);
    }
    // the old DALL·E models answer with an address unless a picture inside the answer is asked for; gpt-image-1 always sends the picture
    if model.starts_with("dall-e") || model.starts_with("imagen") {
        b["response_format"] = json!("b64_json");
    }
    b
}

// "png" | "jpg" | "webp" by the first bytes; None = not a picture
pub fn image_kind(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        Some("png")
    } else if b.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("jpg")
    } else if b.len() > 12 && &b[..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

#[derive(Debug)]
pub enum Payload {
    Bytes(Vec<u8>),
    Url(String),
}

pub fn parse_response(v: &Value) -> Result<Payload, String> {
    if let Some(e) = v.get("error") {
        let m = e.get("message").and_then(|m| m.as_str()).unwrap_or("ошибка провайдера");
        return Err(m.chars().take(300).collect());
    }
    let d = v.get("data").and_then(|d| d.get(0)).ok_or("в ответе нет картинки")?;
    if let Some(b) = d.get("b64_json").and_then(|b| b.as_str()) {
        let bytes = crate::docai::b64_decode(b).ok_or("картинка в ответе повреждена")?;
        return Ok(Payload::Bytes(bytes));
    }
    if let Some(u) = d.get("url").and_then(|u| u.as_str()) {
        return Ok(Payload::Url(u.to_string()));
    }
    Err("в ответе нет картинки".into())
}

pub fn out_dir() -> PathBuf {
    crate::filesearch::known_folder("pictures").unwrap_or_else(|| PathBuf::from(".")).join("Jarvis")
}

fn file_stem(prompt: &str) -> String {
    let words: String = prompt.chars().map(|c| if c.is_alphanumeric() { c } else { ' ' }).collect::<String>().split_whitespace().take(4).collect::<Vec<_>>().join("-");
    let w: String = words.chars().take(40).collect();
    format!("{}-{}", chrono::Local::now().format("%Y%m%d-%H%M%S"), if w.is_empty() { "image".into() } else { w })
}

pub fn save(bytes: &[u8], prompt: &str, dir: &Path) -> Result<PathBuf, String> {
    if bytes.len() > MAX_IMAGE {
        return Err("Картинка слишком большая.".into());
    }
    let kind = image_kind(bytes).ok_or("Провайдер вернул не картинку.")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("не удалось создать папку {}: {}", dir.display(), e))?;
    let path = dir.join(format!("{}.{}", file_stem(prompt), kind));
    crate::write_atomic(&path, bytes).map_err(|e| format!("не удалось сохранить: {}", e))?;
    Ok(path)
}

#[cfg(feature = "lua")]
pub fn generate_with(base: &str, key: &str, model: &str, prompt: &str, dir: &Path) -> Result<PathBuf, String> {
    let prompt = prompt.trim();
    if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT {
        return Err(format!("Описание картинки: от 1 до {} символов.", MAX_PROMPT));
    }
    if model.is_empty() {
        return Err("Не выбрана модель картинок: укажите её в Настройках → ИИ.".into());
    }
    let client = reqwest::blocking::Client::builder().connect_timeout(std::time::Duration::from_secs(8)).timeout(std::time::Duration::from_secs(180)).build().map_err(|e| e.to_string())?;
    let mut req = client.post(format!("{}/images/generations", base.trim_end_matches('/'))).json(&request_body(model, prompt, ""));
    if !key.is_empty() {
        req = req.bearer_auth(key);
    }
    let resp = req.send().map_err(|e| format!("Провайдер не ответил: {}", e.without_url()))?;
    let code = resp.status().as_u16();
    let text = resp.text().unwrap_or_default();
    let v: Value = serde_json::from_str(&text).map_err(|_| format!("Провайдер ответил кодом {} без JSON", code))?;
    if code == 401 || code == 403 {
        return Err("Провайдер не принял ключ.".into());
    }
    let payload = parse_response(&v).map_err(|e| if code >= 400 { format!("Код {}: {}", code, e) } else { e })?;
    let bytes = match payload {
        Payload::Bytes(b) => b,
        Payload::Url(u) => {
            if !u.starts_with("https://") {
                return Err("Провайдер дал небезопасную ссылку на картинку.".into());
            }
            let r = client.get(&u).send().map_err(|e| format!("Не удалось скачать картинку: {}", e.without_url()))?;
            if r.content_length().map(|l| l as usize > MAX_IMAGE).unwrap_or(false) {
                return Err("Картинка слишком большая.".into());
            }
            r.bytes().map_err(|e| e.to_string())?.to_vec()
        }
    };
    save(&bytes, prompt, dir)
}

// the provider of the settings
#[cfg(feature = "lua")]
pub fn generate(prompt: &str) -> Result<PathBuf, String> {
    perms::check(perms::Cap::AiNetwork)?;
    let s = crate::uistate::settings();
    if !matches!(s.ai_provider.as_str(), "openai" | "gemini" | "custom") {
        return Err("Картинки умеют делать OpenAI, Gemini и свой совместимый сервер. Выберите одного из них в Настройках → ИИ.".into());
    }
    let conf = crate::ai::conf_from_settings().map_err(|e| e.user_text())?;
    let model = {
        let m = model_for(&s.ai_provider);
        if m.is_empty() { s.ai_model.clone() } else { m }
    };
    generate_with(&conf.base, &conf.key, &model, prompt, &out_dir())
}

#[cfg(not(feature = "lua"))]
pub fn generate(_prompt: &str) -> Result<PathBuf, String> {
    Err("нет сети в этой сборке".into())
}

// ---- the job shown in the window: one at a time, with a state; «Отмена» drops the result (the request itself cannot be recalled) ----
#[derive(Clone, Debug, Default, Serialize)]
pub struct Job {
    // "idle" | "running" | "done" | "error" | "cancelled"
    pub state: String,
    pub prompt: String,
    pub path: String,
    pub message: String,
    pub started: u64,
}

static JOB: parking_lot::Mutex<Option<Job>> = parking_lot::const_mutex(None);
static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn job() -> Job {
    JOB.lock().clone().unwrap_or(Job { state: "idle".into(), ..Default::default() })
}

pub fn cancel_job() -> bool {
    let mut g = JOB.lock();
    match g.as_mut() {
        Some(j) if j.state == "running" => {
            GEN.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            j.state = "cancelled".into();
            j.message = "Отменено: результат будет отброшен (запрос уже ушёл провайдеру, платный он или нет — решает провайдер).".into();
            true
        }
        _ => false,
    }
}

// `run` makes the picture (the real call in the program, a stub in tests); the answer is applied only if the job was not cancelled meanwhile
pub fn start_job_with(prompt: &str, run: Box<dyn FnOnce(&str) -> Result<PathBuf, String> + Send>) -> Result<(), String> {
    let prompt = prompt.trim().to_string();
    if prompt.is_empty() || prompt.chars().count() > MAX_PROMPT {
        return Err(format!("Описание картинки: от 1 до {} символов.", MAX_PROMPT));
    }
    {
        let mut g = JOB.lock();
        if g.as_ref().map(|j| j.state == "running").unwrap_or(false) {
            return Err("Уже идёт генерация: дождитесь или отмените её.".into());
        }
        *g = Some(Job { state: "running".into(), prompt: prompt.clone(), path: String::new(), message: "Запрос отправлен провайдеру…".into(), started: crate::store::now() });
    }
    let my = GEN.load(std::sync::atomic::Ordering::SeqCst);
    std::thread::spawn(move || {
        // a panic of the call must end the job with an error: a job «running» for ever would refuse every next picture
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&prompt))).unwrap_or_else(|_| Err("Генерация остановилась из-за внутренней ошибки. Попробуйте ещё раз.".to_string()));
        let mut g = JOB.lock();
        if GEN.load(std::sync::atomic::Ordering::SeqCst) != my {
            // cancelled: the file, if it was made, is removed
            if let Ok(p) = &result {
                let _ = std::fs::remove_file(p);
            }
            return;
        }
        if let Some(j) = g.as_mut() {
            match result {
                Ok(p) => {
                    j.state = "done".into();
                    j.path = p.to_string_lossy().to_string();
                    j.message = "Готово: файл сохранён.".into();
                }
                Err(e) => {
                    j.state = "error".into();
                    j.message = e;
                }
            }
        }
    });
    Ok(())
}

pub fn start_job(prompt: &str) -> Result<(), String> {
    perms::check(perms::Cap::AiNetwork)?;
    start_job_with(prompt, Box::new(|p| generate(p)))
}

// the finished picture for the window (small enough to carry in the answer)
pub fn preview_b64(path: &str) -> Option<String> {
    let p = Path::new(path);
    if !p.starts_with(out_dir()) && !cfg!(test) {
        return None;
    }
    let b = std::fs::read(p).ok().filter(|b| b.len() < 8_000_000)?;
    image_kind(&b)?;
    Some(crate::docai::b64_encode(&b))
}

// video: its own adapter. No supported provider has one common call, so the answer says what is needed.
pub fn generate_video(_prompt: &str) -> Result<PathBuf, String> {
    Err("Создание видео пока недоступно: для него нужен отдельный сервис (например, Sora или Veo) и его ключ, а общего способа у подключённых ИИ-провайдеров нет. Картинки работают.".into())
}

// «нарисуй …» / «создай картинку …» / «generate an image of …»
pub fn parse(text: &str) -> Option<(bool, String)> {
    let t = text.trim();
    let low = t.to_lowercase();
    for (p, video) in [("нарисуй ", false), ("создай картинку ", false), ("сгенерируй картинку ", false), ("создай изображение ", false), ("generate an image of ", false), ("generate image of ", false), ("draw ", false), ("создай видео ", true), ("сгенерируй видео ", true), ("generate a video of ", true), ("rasm chiz ", false)] {
        if low.starts_with(p) {
            let body: String = t.chars().skip(p.chars().count()).collect();
            let body = body.trim().trim_start_matches(':').trim().to_string();
            return Some((video, body)).filter(|(_, b)| !b.is_empty());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // the tests of the job share ONE job state: they run one after another
    static JOB_TEST_LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());
    use std::io::{Read, Write};

    const PNG: [u8; 12] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];

    #[test]
    fn what_is_a_picture() {
        assert_eq!(image_kind(&PNG), Some("png"));
        assert_eq!(image_kind(&[0xff, 0xd8, 0xff, 0xe0]), Some("jpg"));
        assert_eq!(image_kind(b"<html>error page</html>"), None);
        assert_eq!(image_kind(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
    }

    #[test]
    fn requests_and_answers() {
        assert_eq!(request_body("dall-e-3", "кот", "")["response_format"], "b64_json");
        assert!(request_body("gpt-image-1", "кот", "1024x1024").get("response_format").is_none());
        assert!(matches!(parse_response(&json!({"data":[{"b64_json": crate::docai::b64_encode(&PNG)}]})), Ok(Payload::Bytes(b)) if b == PNG));
        assert!(matches!(parse_response(&json!({"data":[{"url":"https://x/y.png"}]})), Ok(Payload::Url(_))));
        assert!(parse_response(&json!({"error":{"message":"billing hard limit"}})).unwrap_err().contains("billing"));
        assert!(parse_response(&json!({"data":[]})).is_err());
    }

    #[test]
    fn phrases() {
        assert_eq!(parse("Нарисуй рыжего кота на крыше"), Some((false, "рыжего кота на крыше".into())));
        assert_eq!(parse("создай видео про океан"), Some((true, "про океан".into())));
        assert_eq!(parse("открой браузер"), None);
        assert!(generate_video("x").unwrap_err().contains("недоступно"));
    }

    #[test]
    fn the_job_has_honest_states_and_a_cancelled_result_is_dropped() {
        let _t = JOB_TEST_LOCK.lock();
        let d = tempfile::tempdir().unwrap();
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let dir = d.path().to_path_buf();
        start_job_with("кот", Box::new(move |_| { rx.recv().ok(); save(&PNG, "кот", &dir) })).unwrap();
        assert_eq!(job().state, "running");
        assert!(start_job_with("ещё", Box::new(|_| Err("x".into()))).unwrap_err().contains("Уже идёт"));
        assert!(cancel_job());
        tx.send(()).unwrap();
        for _ in 0..50 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            if std::fs::read_dir(d.path()).unwrap().next().is_none() { break; }
        }
        assert_eq!(job().state, "cancelled");
        assert!(std::fs::read_dir(d.path()).unwrap().next().is_none(), "the file of a cancelled job is removed");
        // a normal job
        let dir = d.path().to_path_buf();
        start_job_with("пёс", Box::new(move |_| save(&PNG, "пёс", &dir))).unwrap();
        for _ in 0..100 { if job().state != "running" { break; } std::thread::sleep(std::time::Duration::from_millis(20)); }
        let j = job();
        assert_eq!(j.state, "done");
        assert!(preview_b64(&j.path).is_some());
        // an error is shown as it is
        start_job_with("x", Box::new(|_| Err("лимит провайдера".into()))).unwrap();
        for _ in 0..100 { if job().state != "running" { break; } std::thread::sleep(std::time::Duration::from_millis(20)); }
        assert_eq!((job().state.as_str(), job().message.as_str()), ("error", "лимит провайдера"));
        assert!(start_job_with("", Box::new(|_| Err("x".into()))).is_err());
    }

    #[test]
    fn a_non_picture_is_never_saved() {
        let d = tempfile::tempdir().unwrap();
        assert!(save(b"not an image", "кот", d.path()).is_err());
        assert!(std::fs::read_dir(d.path()).unwrap().next().is_none());
        let p = save(&PNG, "рыжий кот", d.path()).unwrap();
        assert!(p.extension().unwrap() == "png" && p.exists());
    }

    #[test]
    fn the_provider_is_called_and_the_file_is_the_proof() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let body = json!({"data":[{"b64_json": crate::docai::b64_encode(&PNG)}]}).to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut s, _) = l.accept().unwrap();
            let mut buf = vec![0u8; 8192];
            let n = s.read(&mut buf).unwrap();
            tx.send(String::from_utf8_lossy(&buf[..n]).to_string()).unwrap();
            let _ = write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
        });
        let d = tempfile::tempdir().unwrap();
        let p = generate_with(&format!("http://{}/v1", addr), "sk-test", "gpt-image-1", "кот в шляпе", d.path()).unwrap();
        assert!(p.exists());
        let req = rx.recv().unwrap();
        assert!(req.starts_with("POST /v1/images/generations") && req.to_lowercase().contains("authorization: bearer sk-test") && req.contains("кот в шляпе"));
        assert!(generate_with("http://127.0.0.1:1/v1", "", "m", "", d.path()).is_err());
        assert!(generate_with("http://127.0.0.1:1/v1", "", "", "кот", d.path()).unwrap_err().contains("модель"));
    }

    #[test]
    fn a_job_that_panics_ends_in_an_error_and_does_not_block_the_next_one() {
        let _t = JOB_TEST_LOCK.lock();
        // (the jobs share one state: wait until no other test's job is running)
        for _ in 0..200 { if job().state != "running" { break; } std::thread::sleep(std::time::Duration::from_millis(20)); }
        start_job_with("падает", Box::new(|_| panic!("внутренний сбой"))).unwrap();
        for _ in 0..200 { if job().state != "running" { break; } std::thread::sleep(std::time::Duration::from_millis(20)); }
        let j = job();
        assert_eq!(j.state, "error", "a panic must not leave the job «running» for ever");
        assert!(!j.message.is_empty());
        assert!(start_job_with("следующая", Box::new(|_| Err("x".into()))).is_ok(), "a new job can start");
        for _ in 0..200 { if job().state != "running" { break; } std::thread::sleep(std::time::Duration::from_millis(20)); }
    }
}
