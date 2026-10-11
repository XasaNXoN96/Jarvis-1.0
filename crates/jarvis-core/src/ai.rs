// AI chat: real answers of the chosen provider inside the Jarvis window.
//
// - Providers: everything that speaks the OpenAI "chat/completions" API (OpenAI, OpenRouter, Google Gemini through its
//   OpenAI-compatible endpoint, Ollama, any server with a base URL) and Anthropic (own "messages" API).
// - The API key stays in this module / the settings file (encrypted with DPAPI on Windows). It goes to the provider
//   only in a request header, never into a URL, a log line, the chat history or the window.
// - Conversations are stored one file each in <config dir>/chats/. A new conversation never sees an old one.
// - Nothing is sent until the user sends a message (or switched on "answer phrases that are not commands").

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::APP_CONFIG_DIR;

// how much history goes to the model: the last messages, within a size budget
pub const MAX_CONTEXT_MESSAGES: usize = 30;
pub const MAX_CONTEXT_CHARS: usize = 12_000;
pub const MAX_MESSAGE_CHARS: usize = 8_000;
const MAX_STORED_MESSAGES: usize = 500;

// ---------------------------------------------------------------------------------------------
// provider
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
    OpenAiCompatible,
    Anthropic,
}

#[derive(Clone)]
pub struct ProviderConf {
    pub kind: Kind,
    pub name: String,  // for messages: "OpenAI", "Gemini", ...
    pub base: String,  // without a trailing slash
    pub model: String,
    pub key: String,
}

// the key never appears in Debug output
impl std::fmt::Debug for ProviderConf {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ProviderConf {{ {} {} model={} key={} }}", self.name, self.base, self.model, if self.key.is_empty() { "none" } else { "***" })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AiError {
    NoProvider,
    NoKey(String),
    NoModel,
    NoUrl,
    Empty,
    TooLong,
    Busy,
    Auth(String),
    Quota(String),
    ModelUnknown(String),
    Network(String),
    Timeout,
    Server(u16),
    BadResponse,
    Cancelled,
    Other(String),
}

impl AiError {
    // what the user sees: what happened and what to do next
    pub fn user_text(&self) -> String {
        match self {
            AiError::NoProvider => "ИИ-провайдер не выбран. Откройте Настройки → ИИ-провайдер, выберите провайдера и введите ключ.".into(),
            AiError::NoKey(p) => format!("Не введён ключ API для {}. Откройте Настройки → ИИ-провайдер и вставьте ключ.", p),
            AiError::NoModel => "Не указана модель. Откройте Настройки → ИИ-провайдер и впишите название модели (например, gpt-4o-mini).".into(),
            AiError::NoUrl => "Не указан адрес сервера. Откройте Настройки → ИИ-провайдер и впишите адрес (base URL).".into(),
            AiError::Empty => "Сообщение пустое. Напишите вопрос.".into(),
            AiError::TooLong => format!("Сообщение слишком длинное (больше {} символов). Сократите его.", MAX_MESSAGE_CHARS),
            AiError::Busy => "ИИ ещё отвечает. Дождитесь ответа или нажмите «Стоп».".into(),
            AiError::Auth(p) => format!("Провайдер ({}) не принял ключ. Проверьте ключ в Настройках (без пробелов, не просрочен ли) или создайте новый.", p),
            AiError::Quota(p) => format!("Провайдер ({}) сообщает: закончились средства или достигнут лимит запросов. Проверьте баланс и тариф у провайдера или подождите минуту.", p),
            AiError::ModelUnknown(m) => format!("Провайдер не знает модель «{}». Проверьте название модели в Настройках → ИИ-провайдер.", m),
            AiError::Network(why) => format!("Нет соединения с сервером ИИ ({}). Проверьте интернет и адрес сервера.", why),
            AiError::Timeout => "Сервер ИИ слишком долго не отвечает. Попробуйте ещё раз или выберите другую модель.".into(),
            AiError::Server(code) => format!("Сервер ИИ вернул ошибку {}. Это проблема на стороне провайдера: попробуйте позже.", code),
            AiError::BadResponse => "Сервер ИИ вернул ответ в непонятном формате. Проверьте адрес сервера и модель в Настройках.".into(),
            AiError::Cancelled => "Остановлено.".into(),
            AiError::Other(text) => format!("Не получилось получить ответ: {}", text),
        }
    }
}

fn is_local(url: &str) -> bool {
    let u = url.to_lowercase();
    u.contains("://localhost") || u.contains("://127.") || u.contains("://[::1]") || u.contains("://192.168.") || u.contains("://10.")
}

// the provider of the settings; the errors say what is missing
pub fn conf_from(provider: &str, model: &str, key: &str, url: &str) -> Result<ProviderConf, AiError> {
    let url = url.trim().trim_end_matches('/').to_string();
    let (kind, name, base, key_needed) = match provider {
        "openai" => (Kind::OpenAiCompatible, "OpenAI", "https://api.openai.com/v1".to_string(), true),
        "openrouter" => (Kind::OpenAiCompatible, "OpenRouter", "https://openrouter.ai/api/v1".to_string(), true),
        "gemini" => (Kind::OpenAiCompatible, "Google Gemini", "https://generativelanguage.googleapis.com/v1beta/openai".to_string(), true),
        "anthropic" => (Kind::Anthropic, "Anthropic", if url.is_empty() { "https://api.anthropic.com".to_string() } else { url.clone() }, true),
        "ollama" => (Kind::OpenAiCompatible, "Ollama", format!("{}/v1", if url.is_empty() { "http://127.0.0.1:11434".to_string() } else { url.clone() }), false),
        "custom" => {
            if url.is_empty() {
                return Err(AiError::NoUrl);
            }
            (Kind::OpenAiCompatible, "ИИ-сервер", url.clone(), !is_local(&url))
        }
        _ => return Err(AiError::NoProvider),
    };
    if key_needed && key.trim().is_empty() {
        return Err(AiError::NoKey(name.to_string()));
    }
    if model.trim().is_empty() {
        return Err(AiError::NoModel);
    }
    Ok(ProviderConf { kind, name: name.to_string(), base, model: model.trim().to_string(), key: key.trim().to_string() })
}

pub fn conf_from_settings() -> Result<ProviderConf, AiError> {
    let s = crate::uistate::settings();
    conf_from(&s.ai_provider, &s.ai_model, &s.ai_key, &s.ai_url)
}

pub fn is_configured() -> bool {
    conf_from_settings().is_ok()
}

// ---------------------------------------------------------------------------------------------
// conversations
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ChatMsg {
    pub role: String, // user | assistant | error (error is shown, never sent to the model)
    pub content: String,
    #[serde(default)]
    pub ts: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chat {
    pub id: String,
    pub title: String,
    pub created: u64,
    pub updated: u64,
    pub messages: Vec<ChatMsg>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub struct Store {
    pub(crate) dir: PathBuf,
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Store { dir }
    }

    fn path(&self, id: &str) -> Option<PathBuf> {
        if valid_id(id) { Some(self.dir.join(format!("{}.json", id))) } else { None }
    }

    pub fn create(&self) -> Chat {
        let id = format!("c{}-{}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0), COUNTER.fetch_add(1, Ordering::SeqCst));
        let chat = Chat { id: id.clone(), title: "Новый диалог".into(), created: now(), updated: now(), messages: Vec::new() };
        self.save(&chat);
        self.set_active(&id);
        chat
    }

    pub fn load(&self, id: &str) -> Option<Chat> {
        let text = std::fs::read_to_string(self.path(id)?).ok()?;
        serde_json::from_str(&text).ok()
    }

    // written to a temporary file first, so a crash cannot leave half a conversation
    pub fn save(&self, chat: &Chat) {
        let Some(path) = self.path(&chat.id) else { return };
        let mut chat = chat.clone();
        if chat.messages.len() > MAX_STORED_MESSAGES {
            let cut = chat.messages.len() - MAX_STORED_MESSAGES;
            chat.messages.drain(..cut);
        }
        if let Ok(text) = serde_json::to_string(&chat) {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, text).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
    }

    pub fn delete(&self, id: &str) -> bool {
        match self.path(id) {
            Some(p) => std::fs::remove_file(p).is_ok(),
            None => false,
        }
    }

    pub fn clear(&self, id: &str) -> bool {
        match self.load(id) {
            Some(mut chat) => {
                chat.messages.clear();
                chat.title = "Новый диалог".into();
                chat.updated = now();
                self.save(&chat);
                true
            }
            None => false,
        }
    }

    pub fn list(&self) -> Vec<Value> {
        let mut chats: Vec<Chat> = std::fs::read_dir(&self.dir)
            .map(|rd| {
                rd.flatten()
                    .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                    .filter_map(|e| std::fs::read_to_string(e.path()).ok())
                    .filter_map(|t| serde_json::from_str::<Chat>(&t).ok())
                    .collect()
            })
            .unwrap_or_default();
        chats.sort_by(|a, b| b.updated.cmp(&a.updated));
        chats.iter().map(|c| json!({ "id": c.id, "title": c.title, "updated": c.updated, "count": c.messages.len() })).collect()
    }

    pub fn active(&self) -> Option<String> {
        let id = std::fs::read_to_string(self.dir.join("active")).ok()?.trim().to_string();
        if self.load(&id).is_some() { Some(id) } else { None }
    }

    pub fn set_active(&self, id: &str) {
        if valid_id(id) {
            let _ = std::fs::write(self.dir.join("active"), id);
        }
    }

    // the active conversation, a new one when there is none
    pub fn active_or_new(&self) -> Chat {
        match self.active().and_then(|id| self.load(&id)) {
            Some(c) => c,
            None => self.create(),
        }
    }

    pub fn append(&self, id: &str, role: &str, content: &str) -> Option<Chat> {
        let mut chat = self.load(id)?;
        if role == "user" && chat.messages.iter().all(|m| m.role != "user") {
            chat.title = content.trim().chars().take(40).collect::<String>().replace('\n', " ");
        }
        chat.messages.push(ChatMsg { role: role.into(), content: content.into(), ts: now() });
        chat.updated = now();
        self.save(&chat);
        Some(chat)
    }
}

static STORE: Lazy<Store> = Lazy::new(|| {
    let dir = APP_CONFIG_DIR.get().cloned().unwrap_or_else(|| std::env::temp_dir().join("jarvis")).join("chats");
    Store::new(dir)
});

pub fn store() -> &'static Store {
    &STORE
}

// ---------------------------------------------------------------------------------------------
// request / response
// ---------------------------------------------------------------------------------------------
pub fn system_prompt() -> String {
    if crate::i18n::get_language() == "en" {
        "You are Jarvis, a voice assistant on the user's computer. Answer in the user's language, briefly and to the point. Put code in Markdown code blocks.".to_string()
    } else {
        "Ты — Jarvis, голосовой ассистент на компьютере пользователя. Отвечай на языке пользователя, кратко и по делу. Код оформляй в блоках Markdown.".to_string()
    }
}

// the last messages that fit the budget, starting with a user message, only user / assistant roles
pub fn context_for_model(messages: &[ChatMsg]) -> Vec<ChatMsg> {
    let usable: Vec<&ChatMsg> = messages.iter().filter(|m| (m.role == "user" || m.role == "assistant") && !m.content.trim().is_empty()).collect();
    let mut picked: Vec<ChatMsg> = Vec::new();
    let mut total = 0usize;
    for m in usable.iter().rev() {
        let len = m.content.chars().count();
        if picked.len() >= MAX_CONTEXT_MESSAGES || (total + len > MAX_CONTEXT_CHARS && !picked.is_empty()) {
            break;
        }
        total += len;
        picked.push((*m).clone());
    }
    picked.reverse();
    while picked.first().map(|m| m.role != "user").unwrap_or(false) {
        picked.remove(0);
    }
    picked
}

// Anthropic wants the roles to alternate
fn alternate(messages: &[ChatMsg]) -> Vec<ChatMsg> {
    let mut out: Vec<ChatMsg> = Vec::new();
    for m in messages {
        match out.last_mut() {
            Some(last) if last.role == m.role => {
                last.content.push_str("\n\n");
                last.content.push_str(&m.content);
            }
            _ => out.push(m.clone()),
        }
    }
    out
}

pub fn build_body(conf: &ProviderConf, history: &[ChatMsg], system: &str) -> Value {
    match conf.kind {
        Kind::OpenAiCompatible => {
            let mut msgs = vec![json!({ "role": "system", "content": system })];
            for m in history {
                msgs.push(json!({ "role": m.role, "content": m.content }));
            }
            json!({ "model": conf.model, "messages": msgs, "stream": false })
        }
        Kind::Anthropic => {
            let msgs: Vec<Value> = alternate(history).iter().map(|m| json!({ "role": m.role, "content": m.content })).collect();
            json!({ "model": conf.model, "max_tokens": 2048, "system": system, "messages": msgs })
        }
    }
}

pub fn endpoint(conf: &ProviderConf) -> String {
    match conf.kind {
        Kind::OpenAiCompatible => format!("{}/chat/completions", conf.base),
        Kind::Anthropic => format!("{}/v1/messages", conf.base),
    }
}

pub fn parse_answer(conf: &ProviderConf, body: &Value) -> Result<String, AiError> {
    let text = match conf.kind {
        Kind::OpenAiCompatible => body.pointer("/choices/0/message/content").and_then(|v| v.as_str()).map(|s| s.to_string()),
        Kind::Anthropic => {
            let parts: Vec<String> = body
                .get("content")
                .and_then(|c| c.as_array())
                .map(|a| a.iter().filter_map(|p| p.get("text").and_then(|t| t.as_str()).map(|s| s.to_string())).collect())
                .unwrap_or_default();
            if parts.is_empty() { None } else { Some(parts.join("")) }
        }
    };
    match text {
        Some(t) if !t.trim().is_empty() => Ok(t.trim().to_string()),
        _ => Err(AiError::BadResponse),
    }
}

// an error message of the provider without the key and without noise
fn provider_message(body: &str, key: &str) -> String {
    let v: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let msg = v
        .pointer("/error/message")
        .or_else(|| v.get("message"))
        .or_else(|| v.pointer("/error"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    let mut msg = if key.len() >= 6 { msg.replace(key, "***") } else { msg };
    if msg.chars().count() > 160 {
        msg = msg.chars().take(160).collect::<String>() + "…";
    }
    msg
}

pub fn map_status(conf: &ProviderConf, status: u16, body: &str) -> AiError {
    let low = body.to_lowercase();
    match status {
        401 | 403 => AiError::Auth(conf.name.clone()),
        402 => AiError::Quota(conf.name.clone()),
        429 => AiError::Quota(conf.name.clone()),
        404 => {
            if low.contains("model") { AiError::ModelUnknown(conf.model.clone()) } else { AiError::Other("адрес сервера не найден (404): проверьте адрес в Настройках".into()) }
        }
        400 | 422 => {
            if low.contains("model") { AiError::ModelUnknown(conf.model.clone()) } else {
                let m = provider_message(body, &conf.key);
                AiError::Other(if m.is_empty() { "запрос отклонён сервером (400)".into() } else { m })
            }
        }
        500..=599 => AiError::Server(status),
        _ => AiError::Other(format!("ответ сервера {}", status)),
    }
}

#[cfg(feature = "lua")]
fn client() -> Result<reqwest::blocking::Client, AiError> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|e| AiError::Other(e.to_string()))
}

#[cfg(feature = "lua")]
fn with_key(conf: &ProviderConf, req: reqwest::blocking::RequestBuilder) -> reqwest::blocking::RequestBuilder {
    // the key only in a header, never in the URL
    match conf.kind {
        Kind::OpenAiCompatible => {
            if conf.key.is_empty() { req } else { req.header("Authorization", format!("Bearer {}", conf.key)) }
        }
        Kind::Anthropic => req.header("x-api-key", conf.key.clone()).header("anthropic-version", "2023-06-01"),
    }
}

#[cfg(feature = "lua")]
fn net_error(conf: &ProviderConf, e: reqwest::Error) -> AiError {
    if e.is_timeout() {
        AiError::Timeout
    } else {
        // reqwest errors can contain the URL; the URL has no key, but cut it anyway
        let why = if e.is_connect() { "сервер недоступен".to_string() } else { "ошибка сети".to_string() };
        let _ = conf;
        AiError::Network(why)
    }
}

// one blocking request; the caller handles cancellation
#[cfg(feature = "lua")]
pub fn request_answer(conf: &ProviderConf, history: &[ChatMsg], system: &str) -> Result<String, AiError> {
    let body = build_body(conf, history, system);
    let resp = with_key(conf, client()?.post(endpoint(conf)).json(&body)).send().map_err(|e| net_error(conf, e))?;
    let status = resp.status().as_u16();
    let text = resp.text().map_err(|e| net_error(conf, e))?;
    if !(200..300).contains(&status) {
        return Err(map_status(conf, status, &text));
    }
    let v: Value = serde_json::from_str(&text).map_err(|_| AiError::BadResponse)?;
    parse_answer(conf, &v)
}

#[cfg(not(feature = "lua"))]
pub fn request_answer(_conf: &ProviderConf, _history: &[ChatMsg], _system: &str) -> Result<String, AiError> {
    Err(AiError::Other("в этой сборке нет сетевого модуля".into()))
}

// "check the connection" button: asks for the model list
#[cfg(feature = "lua")]
pub fn test_connection(conf: &ProviderConf) -> Result<(), AiError> {
    let url = match conf.kind {
        Kind::OpenAiCompatible => format!("{}/models", conf.base),
        Kind::Anthropic => format!("{}/v1/models", conf.base),
    };
    let resp = with_key(conf, client()?.get(url)).send().map_err(|e| net_error(conf, e))?;
    let status = resp.status().as_u16();
    if (200..300).contains(&status) {
        Ok(())
    } else {
        let text = resp.text().unwrap_or_default();
        Err(map_status(conf, status, &text))
    }
}

#[cfg(not(feature = "lua"))]
pub fn test_connection(_conf: &ProviderConf) -> Result<(), AiError> {
    Err(AiError::Other("в этой сборке нет сетевого модуля".into()))
}

// ---------------------------------------------------------------------------------------------
// generation jobs: one at a time per conversation, can be stopped
// ---------------------------------------------------------------------------------------------
static JOBS: Lazy<Mutex<HashMap<String, Arc<AtomicBool>>>> = Lazy::new(|| Mutex::new(HashMap::new()));

pub fn is_busy(chat_id: &str) -> bool {
    JOBS.lock().contains_key(chat_id)
}

pub fn stop(chat_id: &str) -> bool {
    match JOBS.lock().get(chat_id) {
        Some(flag) => {
            flag.store(true, Ordering::SeqCst);
            true
        }
        None => false,
    }
}

// text for the voice: no code blocks, no Markdown marks, not too long
pub fn speakable(answer: &str) -> String {
    let mut out = String::new();
    let mut in_code = false;
    for line in answer.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            if !in_code {
                out.push_str("Код показан в окне. ");
            }
            continue;
        }
        if in_code {
            continue;
        }
        let clean: String = line.chars().filter(|c| !matches!(c, '*' | '`' | '#' | '_' | '>')).collect();
        out.push_str(clean.trim());
        out.push(' ');
    }
    let out = out.trim().to_string();
    if out.chars().count() > 700 {
        let cut: String = out.chars().take(700).collect();
        match cut.rfind(|c| c == '.' || c == '!' || c == '?') {
            Some(i) if i > 200 => cut[..=i].to_string(),
            _ => cut,
        }
    } else {
        out
    }
}

pub struct SendOutcome {
    pub chat_id: String,
    pub handle: std::thread::JoinHandle<()>,
}

// Validates, stores the user message and starts the generation in the background.
// Validation errors come back at once (nothing is stored, nothing is sent anywhere).
pub fn send_with(store: &'static Store, conf: Result<ProviderConf, AiError>, chat_id: Option<&str>, text: &str, speak: bool) -> Result<SendOutcome, AiError> {
    let text = text.trim();
    if text.is_empty() {
        return Err(AiError::Empty);
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(AiError::TooLong);
    }
    let conf = conf?;
    let chat = match chat_id.and_then(|id| store.load(id)) {
        Some(c) => c,
        None => store.active_or_new(),
    };
    let id = chat.id.clone();
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut jobs = JOBS.lock();
        if jobs.contains_key(&id) {
            return Err(AiError::Busy);
        }
        jobs.insert(id.clone(), cancel.clone());
    }
    store.set_active(&id);
    let chat = store.append(&id, "user", text).unwrap_or(chat);
    let history = context_for_model(&chat.messages);
    info!("AI: request to {} (model {}), {} messages of context", conf.name, conf.model, history.len());

    let job_id = id.clone();
    let handle = std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        let (conf2, history2) = (conf.clone(), history);
        std::thread::spawn(move || {
            let _ = tx.send(request_answer(&conf2, &history2, &system_prompt()));
        });
        // wait for the answer, but react to "Stop" within a fraction of a second
        let result = loop {
            if cancel.load(Ordering::SeqCst) {
                break Err(AiError::Cancelled);
            }
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(r) => break r,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Err(AiError::BadResponse),
            }
        };
        match &result {
            Ok(answer) => {
                store.append(&job_id, "assistant", answer);
                if speak && crate::tts::is_enabled() {
                    crate::tts::speak(&speakable(answer));
                }
            }
            Err(AiError::Cancelled) => {
                store.append(&job_id, "error", &AiError::Cancelled.user_text());
            }
            Err(e) => {
                warn!("AI: request failed: {:?}", e);
                store.append(&job_id, "error", &e.user_text());
            }
        }
        JOBS.lock().remove(&job_id);
    });
    Ok(SendOutcome { chat_id: id, handle })
}

pub fn send(chat_id: Option<&str>, text: &str, speak: bool) -> Result<String, AiError> {
    send_with(store(), conf_from_settings(), chat_id, text, speak).map(|o| o.chat_id)
}

#[cfg(test)]
mod tests;
