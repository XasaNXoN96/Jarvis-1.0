// The AI at work on the text of a Word document (Object Interaction Engine, part 5): «исправь ошибки», «перепиши
// официально», «переведи на английский», «сократи», «напиши вступление / объявление».
//
// The adapter (office.lua / office.ps1) reads and writes the document; this module only asks the connected AI and keeps the
// answer until the adapter has used it:
//   * the generation is a task of the engine (objects.rs): «стоп» cancels it, an answer that comes after is dropped;
//   * a text that replaces the document waits for «да» (the adapter asks), and is written only if the document is still the
//     text that was read (a hash of the text is compared by the adapter);
//   * a document or a request with something that looks like a key or a password is not sent to the AI;
//   * the text of the document is kept in memory only until it is used or dropped: not in a file, not in a log.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::objects;

const MAX_DOC_CHARS: usize = 40_000;
pub const DONE_MARKER: &str = "objaidone"; // the internal phrase that tells the dialog «the AI has answered»

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Fix,
    Formal,
    Translate,
    Shorten,
    Intro,
    Announce,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "fix" => Kind::Fix,
            "formal" => Kind::Formal,
            "translate" => Kind::Translate,
            "shorten" => Kind::Shorten,
            "intro" => Kind::Intro,
            "announce" => Kind::Announce,
            _ => return None,
        })
    }

    // the answer replaces the whole text of the document (and needs a «да»); the others are added at the cursor
    pub fn replaces(self) -> bool {
        matches!(self, Kind::Fix | Kind::Formal | Kind::Translate | Kind::Shorten)
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Fix => "исправить ошибки",
            Kind::Formal => "официальный стиль",
            Kind::Translate => "перевод",
            Kind::Shorten => "сократить",
            Kind::Intro => "вступление",
            Kind::Announce => "объявление",
        }
    }

    fn system(self) -> &'static str {
        match self {
            Kind::Fix => "Исправь орфографические, пунктуационные и грамматические ошибки в тексте. Смысл, стиль и абзацы не меняй. Ответь ТОЛЬКО исправленным текстом, без пояснений и без кавычек вокруг.",
            Kind::Formal => "Перепиши текст в официально-деловом стиле. Смысл и факты не меняй, ничего не добавляй. Ответь ТОЛЬКО новым текстом, без пояснений.",
            Kind::Translate => "Переведи текст на язык, названный в задаче. Сохрани абзацы. Ответь ТОЛЬКО переводом, без пояснений.",
            Kind::Shorten => "Сократи текст примерно вдвое, сохранив главное и порядок мыслей. Ответь ТОЛЬКО сокращённым текстом, без пояснений.",
            Kind::Intro => "Напиши короткое вступление (3-5 предложений) по теме из задачи. Ответь ТОЛЬКО текстом вступления, без заголовков и пояснений.",
            Kind::Announce => "Напиши короткое объявление по теме из задачи (до 8 предложений). Ответь ТОЛЬКО текстом объявления, без пояснений.",
        }
    }
}

// ---------------------------------------------------------------------------------------------
// text helpers shared with the adapter: the same normalization as office.ps1 (paragraphs are "\n", no trailing whitespace)
// ---------------------------------------------------------------------------------------------
pub fn normalize(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n").trim_end().to_string()
}

pub fn text_hash(s: &str) -> String {
    let mut h = Sha256::new();
    h.update(normalize(s).as_bytes());
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() / 3 * 4 + 4);
    for chunk in data.chunks(3) {
        let n = (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 4 != 0 {
        return None;
    }
    let value = |c: u8| B64.iter().position(|b| *b == c).map(|p| p as u32);
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    for (i, chunk) in bytes.chunks(4).enumerate() {
        let last = (i + 1) * 4 == bytes.len();
        let pad = chunk.iter().rev().take_while(|c| **c == b'=').count();
        if pad > 2 || (pad > 0 && !last) {
            return None;
        }
        let mut n: u32 = 0;
        for c in &chunk[..4 - pad] {
            n = (n << 6) | value(*c)?;
        }
        n <<= 6 * pad as u32;
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------------------------
// the answer that waits for the adapter
// ---------------------------------------------------------------------------------------------
#[derive(Debug, Clone)]
pub struct Answer {
    pub ok: bool,
    pub kind: Kind,
    pub text: String,     // the new text (when ok)
    pub old_hash: String, // the hash of the document text the AI was given
    pub error: String,    // the reason (when not ok)
}

static RESULT: Lazy<Mutex<Option<Answer>>> = Lazy::new(|| Mutex::new(None));

pub fn peek() -> Option<Answer> {
    RESULT.lock().clone()
}

pub fn drop_answer() {
    *RESULT.lock() = None;
}

// Starts the task. `doc_b64`: the text of the document as the adapter read it (Base64 of UTF-8; empty for «напиши вступление»).
pub fn start(kind: Kind, instruction: &str, doc_b64: &str) -> Result<(), String> {
    let doc = match b64_decode(doc_b64) {
        Some(b) => String::from_utf8(b).map_err(|_| "текст документа не в кодировке UTF-8".to_string())?,
        None => return Err("не удалось прочитать текст документа".into()),
    };
    if kind.replaces() && doc.trim().is_empty() {
        return Err("в документе нет текста: нечего менять".into());
    }
    if doc.chars().count() > MAX_DOC_CHARS {
        return Err(format!("документ слишком большой для ИИ (больше {} символов)", MAX_DOC_CHARS));
    }
    if crate::codeobj::contains_secret(&doc) || crate::codeobj::contains_secret(instruction) {
        return Err("в тексте похоже есть ключ, токен или пароль. Я не отправляю такое в ИИ".into());
    }
    crate::codeobj::ai_ready()?;
    let (task, cancel) = objects::with(|e| e.start_ai(kind.label())).map_err(|e| e.user_text())?;
    objects::with(|e| e.note("user", instruction));
    drop_answer();
    let old_hash = text_hash(&doc);
    let instruction: String = instruction.chars().take(500).collect();
    let user = if doc.trim().is_empty() { format!("Задача: {}", instruction) } else { format!("Задача: {}\n\nТекст:\n{}", instruction, doc) };
    #[cfg(test)]
    let places = crate::policy::test_places();
    std::thread::spawn(move || {
        #[cfg(test)]
        crate::policy::apply_test_places(places);
        run(task, cancel, kind, user, old_hash)
    });
    Ok(())
}

fn run(task: u64, cancel: Arc<AtomicBool>, kind: Kind, user: String, old_hash: String) {
    let result = crate::codeobj::generate(kind.system(), &user, &cancel);
    match result {
        Err(why) if why == "cancelled" => {
            objects::with(|e| e.ai_finished(task, Err("cancelled".into())));
        }
        Err(why) => {
            if objects::with(|e| e.is_current_task(task)) {
                objects::with(|e| e.ai_finished(task, Err(why.clone())));
                *RESULT.lock() = Some(Answer { ok: false, kind, text: String::new(), old_hash, error: why });
                crate::remote::push_internal(DONE_MARKER);
            }
        }
        Ok(answer) => {
            let text = answer.trim().trim_start_matches("```").trim_end_matches("```").trim().to_string();
            if text.is_empty() {
                if objects::with(|e| e.is_current_task(task)) {
                    objects::with(|e| e.ai_finished(task, Err("пустой ответ ИИ".into())));
                    *RESULT.lock() = Some(Answer { ok: false, kind, text: String::new(), old_hash, error: "ИИ вернул пустой ответ".into() });
                    crate::remote::push_internal(DONE_MARKER);
                }
                return;
            }
            // taken only while the task is the current one: a stopped one leaves nothing
            let accepted = objects::with(|e| {
                let ok = e.ai_finished(task, Ok(kind.label().to_string()));
                if ok {
                    *RESULT.lock() = Some(Answer { ok: true, kind, text: text.clone(), old_hash: old_hash.clone(), error: String::new() });
                }
                ok
            });
            if accepted {
                crate::remote::push_internal(DONE_MARKER);
            }
        }
    }
}

#[cfg(test)]
mod tests;
