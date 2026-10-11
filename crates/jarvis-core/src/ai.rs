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
// (short on purpose: every extra message is paid for in time to the first word of the answer)
pub const MAX_CONTEXT_MESSAGES: usize = 16;
pub const MAX_CONTEXT_CHARS: usize = 8_000;
pub const MAX_MESSAGE_CHARS: usize = 8_000;
const MAX_STORED_MESSAGES: usize = 500;
// how long the provider may stay silent (no answer started, or a stream that stopped) before the error is shown
const STALL_LIMIT: Duration = Duration::from_secs(30);
// no first fragment of a stream after this long: the same question is asked once more without a stream
// a stream that did not work: the next questions are asked without it for a while
// (per server address: a problem of one provider says nothing about another)
static STREAM_BROKEN: Lazy<Mutex<HashMap<String, std::time::Instant>>> = Lazy::new(|| Mutex::new(HashMap::new()));
const STREAM_RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
// Gemini is asked to think little (a long hidden "thinking" is a common reason for a slow first word);
// a provider that does not know the parameter is asked without it from then on
static NO_REASONING_PARAM: Lazy<Mutex<std::collections::HashSet<String>>> = Lazy::new(|| Mutex::new(std::collections::HashSet::new()));
// how long to wait for the first fragment of a stream (milliseconds; shortened in the tests)
static FIRST_FRAGMENT_MS: AtomicU64 = AtomicU64::new(15_000);

fn stream_allowed(conf: &ProviderConf) -> bool {
    match STREAM_BROKEN.lock().get(&conf.base) {
        Some(t) => t.elapsed() > STREAM_RETRY_AFTER,
        None => true,
    }
}

fn reasoning_param_ok(conf: &ProviderConf) -> bool {
    !NO_REASONING_PARAM.lock().contains(&conf.base)
}

// the provider refused "reasoning_effort": true when this is the first time (the question is asked again without it)
fn reasoning_refused(conf: &ProviderConf, status: u16, body: &str) -> bool {
    (status == 400 || status == 422) && body.to_lowercase().contains("reasoning") && NO_REASONING_PARAM.lock().insert(conf.base.clone())
}

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
    // switched off in the privacy section: nothing is sent to any AI provider
    Disabled,
    NoProvider,
    NoKey(String),
    NoModel,
    NoUrl,
    Empty,
    TooLong,
    Busy,
    // the same question was asked a moment ago: it is answered once, the repeat is dropped quietly
    Duplicate,
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
    // errors after which an internet search is a reasonable plan B (the provider cannot answer right now)
    pub fn allows_web_search(&self) -> bool {
        matches!(self, AiError::Network(_) | AiError::Timeout | AiError::Server(_) | AiError::Quota(_))
    }

    // what the user sees: what happened and what to do next
    pub fn user_text(&self) -> String {
        match self {
            AiError::Disabled => "Запросы к ИИ выключены в Настройках → Приватность. Эта фраза требует ИИ, поэтому никуда не отправлена; команды компьютера работают как обычно.".into(),
            AiError::NoProvider => "ИИ-провайдер не выбран. Откройте Настройки → ИИ-провайдер, выберите провайдера и введите ключ.".into(),
            AiError::NoKey(p) => format!("Не введён ключ API для {}. Откройте Настройки → ИИ-провайдер и вставьте ключ.", p),
            AiError::NoModel => "Не указана модель. Откройте Настройки → ИИ-провайдер и впишите название модели (например, gpt-4o-mini).".into(),
            AiError::NoUrl => "Не указан адрес сервера. Откройте Настройки → ИИ-провайдер и впишите адрес (base URL).".into(),
            AiError::Empty => "Сообщение пустое. Напишите вопрос.".into(),
            AiError::TooLong => format!("Сообщение слишком длинное (больше {} символов). Сократите его.", MAX_MESSAGE_CHARS),
            AiError::Busy => "ИИ ещё отвечает. Дождитесь ответа или нажмите «Стоп».".into(),
            AiError::Duplicate => "Этот вопрос уже отправлен.".into(),
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
    // an empty field, or just the provider's name typed as a model ("gemini"), gets a sensible default
    let typed = model.trim();
    let model = if typed.is_empty() || typed.eq_ignore_ascii_case(provider) || typed.eq_ignore_ascii_case(name) {
        match provider {
            "gemini" => "gemini-flash-latest",
            "openai" => "gpt-4o-mini",
            "openrouter" => "openai/gpt-4o-mini",
            "anthropic" => "claude-sonnet-5-5",
            _ => return Err(AiError::NoModel),
        }
    } else {
        typed
    };
    Ok(ProviderConf { kind, name: name.to_string(), base, model: model.to_string(), key: key.trim().to_string() })
}

pub fn conf_from_settings() -> Result<ProviderConf, AiError> {
    let s = crate::uistate::settings();
    if !s.ai_enabled {
        return Err(AiError::Disabled);
    }
    conf_from(&s.ai_provider, &s.ai_model, &s.ai_key, &s.ai_url)
}

pub fn is_configured() -> bool {
    conf_from_settings().is_ok()
}

// ---------------------------------------------------------------------------------------------
// conversations
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
pub struct ChatMsg {
    pub role: String, // user | assistant | error (error is shown, never sent to the model)
    pub content: String,
    #[serde(default)]
    pub ts: u64,
    // request id: a question and every reply to it share it (one history for the home page and the dialog page)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rid: String,
    // who produced the message: "ai" (the provider), "command" (a computer command), "local" (a prepared reply), "search"
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub src: String,
    // the answer is still being generated (only in what the window receives, never stored)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub streaming: bool,
}

// bumped on every change of any conversation: the window reloads a conversation only when this number changes
static REV: AtomicU64 = AtomicU64::new(1);
pub fn rev() -> u64 {
    REV.load(Ordering::SeqCst)
}
fn bump() {
    REV.fetch_add(1, Ordering::SeqCst);
}

// the answer that is being generated right now (kept in memory: writing the file for every fragment would be slow)
static LIVE: Lazy<Mutex<HashMap<String, (String, String)>>> = Lazy::new(|| Mutex::new(HashMap::new()));
pub fn live_set(chat_id: &str, rid: &str, content: &str) {
    LIVE.lock().insert(chat_id.to_string(), (rid.to_string(), content.to_string()));
    bump();
}
pub fn live_clear(chat_id: &str) {
    LIVE.lock().remove(chat_id);
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

// the chat and the request id of the latest question to the AI
static LAST_REQUEST: Lazy<Mutex<Option<(String, String)>>> = Lazy::new(|| Mutex::new(None));
pub fn last_request() -> Option<(String, String)> {
    LAST_REQUEST.lock().clone()
}

// an identifier of one request (a question and its answers)
pub fn new_rid() -> String {
    format!("r{}-{}", SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0), COUNTER.fetch_add(1, Ordering::SeqCst))
}

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
        bump();
    }

    // the conversation as the window shows it: with the answer that is still being generated
    pub fn load_with_live(&self, id: &str) -> Option<Chat> {
        let mut chat = self.load(id)?;
        if let Some((rid, content)) = LIVE.lock().get(id) {
            chat.messages.push(ChatMsg { role: "assistant".into(), content: content.clone(), ts: now(), rid: rid.clone(), src: "ai".into(), streaming: true });
        }
        Some(chat)
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

    // how much is stored (for the privacy section): never the content
    pub fn stats(&self) -> Value {
        let list = self.list();
        let messages: u64 = list.iter().filter_map(|c| c["count"].as_u64()).sum();
        json!({ "chats": list.len(), "messages": messages, "dir": self.dir.display().to_string() })
    }

    // "delete the whole history" (asked and confirmed in the window): every conversation that is not answering right now.
    // Returns (deleted, kept because busy).
    pub fn delete_all(&self) -> (usize, usize) {
        let (mut deleted, mut kept) = (0, 0);
        for c in self.list() {
            let id = c["id"].as_str().unwrap_or("");
            if is_busy(id) {
                kept += 1;
            } else if self.delete(id) {
                deleted += 1;
            }
        }
        let _ = std::fs::remove_file(self.dir.join("active"));
        bump();
        (deleted, kept)
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
        self.append_with(id, role, content, "", "")
    }

    pub fn append_with(&self, id: &str, role: &str, content: &str, rid: &str, src: &str) -> Option<Chat> {
        let mut chat = self.load(id)?;
        if role == "user" && chat.messages.iter().all(|m| m.role != "user") {
            chat.title = content.trim().chars().take(40).collect::<String>().replace('\n', " ");
        }
        chat.messages.push(ChatMsg { role: role.into(), content: content.into(), ts: now(), rid: rid.into(), src: src.into(), streaming: false });
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
    // computer commands and their replies ("открой ворд" - "Открываю Word") are not a talk with the model
    let usable: Vec<&ChatMsg> = messages
        .iter()
        .filter(|m| (m.role == "user" || m.role == "assistant") && !m.content.trim().is_empty() && (m.src.is_empty() || m.src == "ai"))
        .collect();
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
    build_body_opts(conf, history, system, false)
}

// `stream`: the answer arrives in fragments (server-sent events) instead of one piece at the end
pub fn build_body_opts(conf: &ProviderConf, history: &[ChatMsg], system: &str, stream: bool) -> Value {
    match conf.kind {
        Kind::OpenAiCompatible => {
            let mut msgs = vec![json!({ "role": "system", "content": system })];
            for m in history {
                msgs.push(json!({ "role": m.role, "content": m.content }));
            }
            let mut body = json!({ "model": conf.model, "messages": msgs, "stream": stream });
            if conf.name.contains("Gemini") && reasoning_param_ok(conf) {
                body["reasoning_effort"] = json!("low");
            }
            body
        }
        Kind::Anthropic => {
            let msgs: Vec<Value> = alternate(history).iter().map(|m| json!({ "role": m.role, "content": m.content })).collect();
            let mut body = json!({ "model": conf.model, "max_tokens": 2048, "system": system, "messages": msgs });
            if stream {
                body["stream"] = json!(true);
            }
            body
        }
    }
}

// ---------------------------------------------------------------------------------------------
// streaming: one line of a server-sent-events answer
// ---------------------------------------------------------------------------------------------
#[derive(Debug, PartialEq)]
pub enum Sse {
    Delta(String),
    Done,
    Error(String),
    Skip,
}

pub fn parse_sse_line(kind: &Kind, line: &str) -> Sse {
    let Some(data) = line.strip_prefix("data:") else { return Sse::Skip };
    let data = data.trim();
    if data.is_empty() {
        return Sse::Skip;
    }
    if data == "[DONE]" {
        return Sse::Done;
    }
    let Ok(v) = serde_json::from_str::<Value>(data) else { return Sse::Skip };
    if let Some(e) = v.get("error") {
        let msg = e.get("message").and_then(|m| m.as_str()).or_else(|| e.as_str()).unwrap_or("ошибка в потоке ответа");
        return Sse::Error(msg.chars().take(160).collect());
    }
    match kind {
        Kind::OpenAiCompatible => match v.pointer("/choices/0/delta/content").and_then(|c| c.as_str()) {
            Some(text) if !text.is_empty() => Sse::Delta(text.to_string()),
            _ => Sse::Skip,
        },
        Kind::Anthropic => match v.get("type").and_then(|t| t.as_str()) {
            Some("content_block_delta") => match v.pointer("/delta/text").and_then(|c| c.as_str()) {
                Some(text) if !text.is_empty() => Sse::Delta(text.to_string()),
                _ => Sse::Skip,
            },
            Some("message_stop") => Sse::Done,
            Some("error") => Sse::Error(v.pointer("/error/message").and_then(|m| m.as_str()).unwrap_or("ошибка в потоке ответа").chars().take(160).collect()),
            _ => Sse::Skip,
        },
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

// ONE client for the whole program: its connections are kept alive, so the second question does not pay for a new
// TCP + TLS handshake (a new client per request was one of the delays). TLS is checked as usual.
#[cfg(feature = "lua")]
fn client() -> Result<reqwest::blocking::Client, AiError> {
    static CLIENT: Lazy<Result<reqwest::blocking::Client, String>> = Lazy::new(|| {
        reqwest::blocking::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(120))
            .pool_idle_timeout(Duration::from_secs(120))
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())
    });
    match &*CLIENT {
        Ok(c) => Ok(c.clone()),
        Err(e) => Err(AiError::Other(e.clone())),
    }
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
    if reasoning_refused(conf, status, &text) {
        return request_answer(conf, history, system);
    }
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

// What the timing report needs from inside the request.
#[derive(Default)]
pub struct Probe {
    pub request_ms: Option<u64>, // submit -> the request goes out
    pub streamed: bool,
}

// The answer as a stream: `on_delta` gets every fragment as soon as it arrives. A server that answers with a plain JSON
// (no stream) is handled too, its whole answer comes as one fragment.
#[cfg(feature = "lua")]
pub fn request_answer_stream(
    conf: &ProviderConf,
    history: &[ChatMsg],
    system: &str,
    cancel: &AtomicBool,
    t0: std::time::Instant,
    probe: &mut Probe,
    on_delta: &mut dyn FnMut(&str),
) -> Result<String, AiError> {
    use std::io::BufRead;
    let body = build_body_opts(conf, history, system, true);
    probe.request_ms = Some(t0.elapsed().as_millis() as u64);
    // "identity": a compressed stream may be held back by the server or the client until a buffer is full
    let resp = with_key(conf, client()?.post(endpoint(conf)).header("Accept", "text/event-stream").header("Accept-Encoding", "identity").json(&body)).send().map_err(|e| net_error(conf, e))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        let text = resp.text().unwrap_or_default();
        if reasoning_refused(conf, status, &text) {
            return request_answer_stream(conf, history, system, cancel, t0, probe, on_delta);
        }
        // a server that does not know "stream": ask once more the usual way
        if (status == 400 || status == 422) && text.to_lowercase().contains("stream") {
            let answer = request_answer(conf, history, system)?;
            on_delta(&answer);
            return Ok(answer);
        }
        return Err(map_status(conf, status, &text));
    }
    let is_sse = resp.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).map(|v| v.contains("event-stream")).unwrap_or(false);
    if !is_sse {
        let text = resp.text().map_err(|e| net_error(conf, e))?;
        let v: Value = serde_json::from_str(&text).map_err(|_| AiError::BadResponse)?;
        let answer = parse_answer(conf, &v)?;
        on_delta(&answer);
        return Ok(answer);
    }
    probe.streamed = true;
    let mut reader = std::io::BufReader::new(resp);
    let (mut full, mut line) = (String::new(), String::new());
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(AiError::Cancelled);
        }
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => match parse_sse_line(&conf.kind, line.trim_end()) {
                Sse::Delta(d) => {
                    full.push_str(&d);
                    on_delta(&d);
                }
                Sse::Done => break,
                Sse::Error(m) => return Err(AiError::Other(m.replace(&conf.key, "***"))),
                Sse::Skip => {}
            },
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => return Err(AiError::Timeout),
            Err(_) => {
                // the connection broke: keep what arrived if there is something
                if full.trim().is_empty() {
                    return Err(AiError::Network("соединение оборвалось".into()));
                }
                break;
            }
        }
    }
    if full.trim().is_empty() { Err(AiError::BadResponse) } else { Ok(full.trim().to_string()) }
}

#[cfg(not(feature = "lua"))]
#[allow(clippy::too_many_arguments)]
pub fn request_answer_stream(_conf: &ProviderConf, _history: &[ChatMsg], _system: &str, _cancel: &AtomicBool, _t0: std::time::Instant, _probe: &mut Probe, _on_delta: &mut dyn FnMut(&str)) -> Result<String, AiError> {
    Err(AiError::Other("в этой сборке нет сетевого модуля".into()))
}

// ---------------------------------------------------------------------------------------------
// the state of the connection (for the "AI connection" card): only what a real request or check has shown
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conn {
    Unverified,
    Checking,
    Connected,
    Error,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Status {
    pub conn: Conn,
    pub detail: String,
    pub at: u64,
}

impl Status {
    pub fn unverified() -> Status {
        Status { conn: Conn::Unverified, detail: String::new(), at: 0 }
    }
    // a key in the settings proves nothing: only a real answer or a real check changes the state
    pub fn after_ok(&self) -> Status {
        Status { conn: Conn::Connected, detail: String::new(), at: now() }
    }
    pub fn after_error(&self, e: &AiError) -> Status {
        if *e == AiError::Cancelled { self.clone() } else { Status { conn: Conn::Error, detail: first_sentence(&e.user_text()), at: now() } }
    }
}

static STATUS: Lazy<Mutex<Status>> = Lazy::new(|| Mutex::new(Status::unverified()));

// the settings changed: what was confirmed before is not confirmed any more
pub fn status_reset() {
    *STATUS.lock() = Status::unverified();
}

// when the provider last really answered (a request or a check that worked)
static LAST_OK: AtomicU64 = AtomicU64::new(0);

fn status_set(conn: Conn, detail: &str) {
    if conn == Conn::Connected {
        LAST_OK.store(now(), Ordering::SeqCst);
    }
    *STATUS.lock() = Status { conn, detail: detail.to_string(), at: now() };
    crate::uistate::log_event("ai", &format!("{}{}", match conn { Conn::Connected => "ИИ: подключение подтверждено", Conn::Error => "ИИ: ошибка", Conn::Checking => "ИИ: проверка подключения", Conn::Unverified => "ИИ: подключение не проверено" }, if detail.is_empty() { String::new() } else { format!(" — {}", detail) }));
}

fn first_sentence(text: &str) -> String {
    text.split(". ").next().unwrap_or(text).trim_end_matches('.').chars().take(140).collect()
}

fn status_error(e: &AiError) {
    if *e != AiError::Cancelled {
        status_set(Conn::Error, &first_sentence(&e.user_text()));
        // the notification of an AI failure (the text is the explanation, never the question or a key)
        if !matches!(e, AiError::Duplicate | AiError::Busy | AiError::Empty | AiError::Disabled) {
            crate::notify::push("ai_error", "Ошибка ИИ", &first_sentence(&e.user_text()), true);
        }
    }
}

pub fn status_json() -> Value {
    let conf = conf_from_settings();
    let timing = last_timing().map(|t| serde_json::to_value(t).unwrap_or(Value::Null)).unwrap_or(Value::Null);
    match conf {
        Err(e) => json!({ "state": "DISCONNECTED", "configured": false, "provider": "", "model": "", "detail": first_sentence(&e.user_text()), "timing": timing }),
        Ok(c) => {
            let s = STATUS.lock();
            let (state, detail) = match s.conn {
                Conn::Connected => ("CONNECTED", s.detail.clone()),
                Conn::Error => ("ERROR", s.detail.clone()),
                Conn::Checking => ("CHECKING", "проверяю…".to_string()),
                Conn::Unverified => ("DISCONNECTED", "ключ задан, подключение ещё не проверено".to_string()),
            };
            let last_ok = LAST_OK.load(Ordering::SeqCst);
            json!({ "state": state, "configured": true, "provider": c.name, "model": c.model, "detail": detail, "at": s.at, "last_ok": if last_ok == 0 { Value::Null } else { json!(last_ok) }, "timing": timing })
        }
    }
}

// ---------------------------------------------------------------------------------------------
// timing of the last answer
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Debug, Serialize, Default, PartialEq)]
pub struct Timing {
    pub streamed: bool,
    pub chunks: usize,
    pub chars: usize,
    pub model: String,
    pub to_request_ms: u64,                 // submit -> the request goes out
    pub first_chunk_ms: Option<u64>,        // submit -> the first fragment of the answer
    pub total_ms: u64,                      // submit -> the whole answer
    pub first_speech_queued_ms: Option<u64>, // submit -> the first phrase is handed to the voice
    pub first_speech_started_ms: Option<u64>, // submit -> the voice starts working on it
    pub fallback: bool,                     // the stream gave nothing and the answer was asked once more without it
}

static LAST_TIMING: Lazy<Mutex<Option<Timing>>> = Lazy::new(|| Mutex::new(None));

pub fn last_timing() -> Option<Timing> {
    LAST_TIMING.lock().clone()
}

// ---------------------------------------------------------------------------------------------
// reading an answer aloud while it is being generated: whole sentences, no code, no Markdown marks
// ---------------------------------------------------------------------------------------------
pub struct SentenceSpeaker {
    buf: String,
    in_code: bool,
    pending: String,
    spoken: usize,
    limit: usize,
}

impl SentenceSpeaker {
    pub fn new() -> Self {
        SentenceSpeaker { buf: String::new(), in_code: false, pending: String::new(), spoken: 0, limit: 900 }
    }

    fn clean(s: &str) -> String {
        let s: String = s.chars().filter(|c| !matches!(c, '*' | '`' | '#' | '_' | '>')).collect();
        s.trim().trim_start_matches("- ").trim().to_string()
    }

    // sentences (about 40+ characters each) that are complete now
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.buf.push_str(delta);
        let mut out = Vec::new();
        // complete lines first
        while let Some(i) = self.buf.find('\n') {
            let line: String = self.buf[..i].to_string();
            self.buf.drain(..=i);
            self.take_line(&line, true, &mut out);
        }
        // the unfinished line: only the sentences that are already closed in it
        if !self.in_code {
            let tail = self.buf.clone();
            let mut cut = 0;
            let chars: Vec<(usize, char)> = tail.char_indices().collect();
            for (k, (i, c)) in chars.iter().enumerate() {
                if matches!(c, '.' | '!' | '?' | '…') && chars.get(k + 1).map(|(_, n)| n.is_whitespace()).unwrap_or(false) {
                    cut = i + c.len_utf8();
                }
            }
            if cut > 0 {
                let done: String = tail[..cut].to_string();
                self.buf = tail[cut..].trim_start().to_string();
                self.add_text(&done, &mut out);
            }
        }
        out
    }

    fn take_line(&mut self, line: &str, _complete: bool, out: &mut Vec<String>) {
        if line.trim_start().starts_with("```") {
            self.in_code = !self.in_code;
            if !self.in_code {
                self.add_text("Код показан в окне.", out);
            }
            return;
        }
        if self.in_code {
            return;
        }
        self.add_text(line, out);
        self.flush_pending(out, true);
    }

    fn add_text(&mut self, text: &str, out: &mut Vec<String>) {
        let clean = Self::clean(text);
        if clean.is_empty() {
            return;
        }
        if !self.pending.is_empty() {
            self.pending.push(' ');
        }
        self.pending.push_str(&clean);
        self.flush_pending(out, false);
    }

    // short sentences are joined: a voice that says every three words separately sounds choppy
    fn flush_pending(&mut self, out: &mut Vec<String>, force: bool) {
        if self.pending.chars().count() >= 40 || (force && !self.pending.is_empty()) {
            let text = std::mem::take(&mut self.pending);
            if self.spoken < self.limit {
                self.spoken += text.chars().count();
                out.push(text);
            }
        }
    }

    // the rest when the answer is complete
    pub fn finish(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        let rest = std::mem::take(&mut self.buf);
        if !self.in_code {
            self.add_text(&rest, &mut out);
        }
        self.flush_pending(&mut out, true);
        out
    }
}

// "check the connection" button: asks for the model list; the result becomes the state of the connection
pub fn test_connection(conf: &ProviderConf) -> Result<(), AiError> {
    status_set(Conn::Checking, "");
    let r = test_connection_raw(conf);
    match &r {
        Ok(()) => status_set(Conn::Connected, ""),
        Err(e) => status_error(e),
    }
    r
}

#[cfg(feature = "lua")]
fn test_connection_raw(conf: &ProviderConf) -> Result<(), AiError> {
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
fn test_connection_raw(_conf: &ProviderConf) -> Result<(), AiError> {
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
    pub handle: std::thread::JoinHandle<Timing>,
}

// Validates, stores the user message and starts the generation in the background.
// Validation errors come back at once (nothing is stored, nothing is sent anywhere).
pub fn send_with(store: &'static Store, conf: Result<ProviderConf, AiError>, chat_id: Option<&str>, text: &str, speak: bool) -> Result<SendOutcome, AiError> {
    send_with_hook(store, conf, chat_id, text, speak, None)
}

// The same, and `on_error` is called (in the background thread) when the answer could not be obtained.
pub type ErrorHook = Box<dyn FnOnce(&AiError) + Send + 'static>;

pub fn send_with_hook(store: &'static Store, conf: Result<ProviderConf, AiError>, chat_id: Option<&str>, text: &str, speak: bool, on_error: Option<ErrorHook>) -> Result<SendOutcome, AiError> {
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
        // the same question right after it was asked (a double press, a phrase heard twice) is asked once
        if !crate::guard::first(&format!("ai:{}:{}", id, crate::guard::norm(text)), crate::guard::REPEAT_WINDOW) {
            return Err(AiError::Duplicate);
        }
        jobs.insert(id.clone(), cancel.clone());
    }
    store.set_active(&id);
    let rid_text = text.to_string();
    let history = {
        // the question is part of the context; it is stored below together with its request id
        let mut msgs = chat.messages.clone();
        msgs.push(ChatMsg { role: "user".into(), content: rid_text, src: "ai".into(), ..Default::default() });
        context_for_model(&msgs)
    };
    info!("AI: request to {} (model {}), {} messages of context", conf.name, conf.model, history.len());

    let job_id = id.clone();
    let rid = new_rid();
    *LAST_REQUEST.lock() = Some((id.clone(), rid.clone()));
    // the question is stored together with its request id; the answer (or the error) gets the same one
    store.append_with(&id, "user", text, &rid, "ai");
    let t0 = std::time::Instant::now();
    let handle = std::thread::spawn(move || {
        // the messages of the request thread carry its number: after a fallback only the new thread is listened to
        enum Msg {
            Delta(u8, String),
            Done(u8, Result<String, AiError>, Probe),
        }
        let (tx, rx) = mpsc::channel::<Msg>();
        let history = Arc::new(history);
        let stream_cancel = Arc::new(AtomicBool::new(false));
        let start_request = |generation: u8, streaming: bool, stream_cancel: Arc<AtomicBool>| {
            let (tx, conf2, history2) = (tx.clone(), conf.clone(), history.clone());
            std::thread::spawn(move || {
                let mut probe = Probe::default();
                let tx2 = tx.clone();
                let r = if streaming {
                    request_answer_stream(&conf2, &history2, &system_prompt(), &stream_cancel, t0, &mut probe, &mut |d| {
                        let _ = tx2.send(Msg::Delta(generation, d.to_string()));
                    })
                } else {
                    // without a stream the whole answer comes as one fragment
                    probe.request_ms = Some(t0.elapsed().as_millis() as u64);
                    let r = request_answer(&conf2, &history2, &system_prompt());
                    if let Ok(a) = &r {
                        let _ = tx2.send(Msg::Delta(generation, a.clone()));
                    }
                    r
                };
                let _ = tx.send(Msg::Done(generation, r, probe));
            });
        };
        let (mut generation, mut fell_back) = (1u8, false);
        start_request(generation, stream_allowed(&conf), stream_cancel.clone());

        let mut speaker = SentenceSpeaker::new();
        let say_aloud = speak;
        let (mut shown, mut chunks) = (String::new(), 0usize);
        let (mut first_chunk, mut first_queued, mut first_started) = (None::<u64>, None::<u64>, None::<u64>);
        let mut speak_now = |parts: Vec<String>, first_queued: &mut Option<u64>| {
            for part in parts {
                if say_aloud && crate::tts::is_enabled() {
                    if first_queued.is_none() {
                        *first_queued = Some(t0.elapsed().as_millis() as u64);
                    }
                    crate::tts::speak_queued(&part);
                }
            }
        };
        // wait for the answer; show every fragment at once, react to "Stop" within a fraction of a second
        let mut probe_out = Probe::default();
        let mut last_activity = std::time::Instant::now();
        let result = loop {
            if cancel.load(Ordering::SeqCst) {
                stream_cancel.store(true, Ordering::SeqCst);
                break Err(AiError::Cancelled);
            }
            if first_chunk.is_none() && !fell_back && last_activity.elapsed() > Duration::from_millis(FIRST_FRAGMENT_MS.load(Ordering::SeqCst)) && stream_allowed(&conf) {
                // a stream that says nothing: ask once more the usual way and remember it for a while
                warn!("AI: no first fragment, asking again without a stream");
                crate::uistate::log_event("ai", "Поток не ответил: повторяю запрос без потока");
                stream_cancel.store(true, Ordering::SeqCst);
                STREAM_BROKEN.lock().insert(conf.base.clone(), std::time::Instant::now());
                fell_back = true;
                generation += 1;
                last_activity = std::time::Instant::now();
                start_request(generation, false, Arc::new(AtomicBool::new(false)));
                continue;
            }
            if last_activity.elapsed() > STALL_LIMIT {
                // nothing arrives any more: stop waiting, the request thread is told to give up
                stream_cancel.store(true, Ordering::SeqCst);
                break Err(AiError::Timeout);
            }
            match rx.recv_timeout(Duration::from_millis(60)) {
                Ok(Msg::Delta(g, _)) | Ok(Msg::Done(g, _, _)) if g != generation => continue,
                Ok(Msg::Delta(_, d)) => {
                    last_activity = std::time::Instant::now();
                    chunks += 1;
                    if first_chunk.is_none() {
                        first_chunk = Some(t0.elapsed().as_millis() as u64);
                    }
                    shown.push_str(&d);
                    live_set(&job_id, &rid, &shown);
                    let parts = speaker.push(&d);
                    speak_now(parts, &mut first_queued);
                }
                Ok(Msg::Done(_, r, p)) => {
                    probe_out = p;
                    break r;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break Err(AiError::BadResponse),
            }
        };
        let total_ms = t0.elapsed().as_millis() as u64;
        live_clear(&job_id);
        match &result {
            Ok(answer) => {
                store.append_with(&job_id, "assistant", answer, &rid, "ai");
                // the part that was not complete as a sentence
                let parts = speaker.finish();
                speak_now(parts, &mut first_queued);
                status_set(Conn::Connected, "");
            }
            Err(AiError::Cancelled) => {
                // what arrived before "Stop" is kept; the voice stops with it
                if !shown.trim().is_empty() {
                    store.append_with(&job_id, "assistant", shown.trim(), &rid, "ai");
                }
                store.append_with(&job_id, "error", &AiError::Cancelled.user_text(), &rid, "ai");
                crate::tts::stop();
            }
            Err(e) => {
                warn!("AI: request failed: {:?}", e);
                if !shown.trim().is_empty() {
                    store.append_with(&job_id, "assistant", shown.trim(), &rid, "ai");
                }
                store.append_with(&job_id, "error", &e.user_text(), &rid, "ai");
                status_error(e);
            }
        }
        if let Some(started) = crate::tts::last_speech_started() {
            if first_queued.is_some() && started >= t0 {
                first_started = Some(started.duration_since(t0).as_millis() as u64);
            }
        }
        let timing = Timing {
            streamed: probe_out.streamed,
            chunks,
            chars: shown.chars().count(),
            model: conf.model.clone(),
            to_request_ms: probe_out.request_ms.unwrap_or(0),
            first_chunk_ms: first_chunk,
            total_ms,
            first_speech_queued_ms: first_queued,
            first_speech_started_ms: first_started,
            fallback: fell_back,
        };
        info!("AI timing: request out {} ms, first fragment {:?} ms, total {} ms, streamed={}, fragments={}, voice queued {:?} ms", timing.to_request_ms, timing.first_chunk_ms, timing.total_ms, timing.streamed, timing.chunks, timing.first_speech_queued_ms);
        *LAST_TIMING.lock() = Some(timing.clone());
        JOBS.lock().remove(&job_id);
        if let (Err(e), Some(hook)) = (&result, on_error) {
            if *e != AiError::Cancelled {
                hook(e);
            }
        }
        timing
    });
    Ok(SendOutcome { chat_id: id, handle })
}

// a question from the command line / voice: the answer is shown in the chat and read aloud; `on_error` runs on failure
pub fn send_question(text: &str, on_error: ErrorHook) -> Result<String, AiError> {
    send_with_hook(store(), conf_from_settings(), None, text, true, Some(on_error)).map(|o| o.chat_id)
}

pub fn send(chat_id: Option<&str>, text: &str, speak: bool) -> Result<String, AiError> {
    send_with(store(), conf_from_settings(), chat_id, text, speak).map(|o| o.chat_id)
}

#[cfg(test)]
mod tests;
