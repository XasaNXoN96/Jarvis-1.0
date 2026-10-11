// The ONE conversation of Jarvis. The home page, the "Диалог" page, the voice and the phone remote all talk through it:
// a question is stored with a request id, and every reply to it (an answer of the AI, a reply of a computer command,
// a prepared local reply, a search note, an error) is stored with the same id in the same history (ai::store()).
// So a question never stays without an answer and an answer never has no question.
//
// The AI path stores its own messages (ai.rs). This module is the same for everything that is NOT the AI.

use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;

use crate::ai;

struct Req {
    rid: String,
    chat_id: String,
    replies: usize,
    at: Instant,
}

static CUR: Lazy<Mutex<Option<Req>>> = Lazy::new(|| Mutex::new(None));
static LAST: Lazy<Mutex<Option<Req>>> = Lazy::new(|| Mutex::new(None));
// lines printed by a command with jarvis.out (a list of commands, a diagnostics report): one reply at the end
static OUT_BUF: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
static LAST_OUTPUT: Lazy<Mutex<String>> = Lazy::new(|| Mutex::new(String::new()));

// a late reply (a file search that finishes seconds later, a timer) still belongs to the request it came from
const LATE_REPLY: Duration = Duration::from_secs(90);

// the user's phrase that is going to be handled by a computer command or a prepared reply
pub fn begin(text: &str, src: &str) -> String {
    let store = ai::store();
    let chat = store.active_or_new();
    let rid = ai::new_rid();
    store.append_with(&chat.id, "user", text.trim(), &rid, src);
    *CUR.lock() = Some(Req { rid: rid.clone(), chat_id: chat.id, replies: 0, at: Instant::now() });
    rid
}

// an internal follow-up of the request that was handled just now ("ищу в интернете" after an AI failure):
// its replies go to the same request, no new question is stored
pub fn continue_last() -> bool {
    let last = LAST.lock().take();
    match last {
        Some(r) if r.at.elapsed() < LATE_REPLY => {
            *CUR.lock() = Some(Req { at: Instant::now(), ..r });
            true
        }
        _ => false,
    }
}

// (chat, request id, late): `late` = the request was already finished (a file search that ends seconds later)
fn target() -> Option<(String, String, bool)> {
    if let Some(r) = CUR.lock().as_mut() {
        r.replies += 1;
        return Some((r.chat_id.clone(), r.rid.clone(), false));
    }
    if let Some(r) = LAST.lock().as_mut() {
        if r.at.elapsed() < LATE_REPLY {
            r.replies += 1;
            return Some((r.chat_id.clone(), r.rid.clone(), true));
        }
    }
    None
}

// a reply of Jarvis; `src`: "command" | "local" | "search"
pub fn reply(text: &str, src: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    *LAST_OUTPUT.lock() = text.to_string();
    if let Some((chat, rid, late)) = target() {
        ai::store().append_with(&chat, "assistant", text, &rid, src);
        // a task that finished after its request (search results, a download): a notification, unless the user
        // is looking at the window. The command has already said it aloud when the voice is on.
        if late && !crate::notify::window_visible() {
            crate::notify::push("task", "Задача выполнена", text, crate::tts::is_enabled());
        }
    }
}

// an error of the request
pub fn fail(text: &str, src: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    *LAST_OUTPUT.lock() = text.to_string();
    if let Some((chat, rid, _)) = target() {
        ai::store().append_with(&chat, "error", text, &rid, src);
    }
    // an error of a computer function (the AI reports its own errors in ai.rs)
    if src != "ai" {
        crate::notify::push("system_error", "Ошибка", text, true);
    }
}

// the request is handled; a command that said nothing still gets an answer ("Выполнено"), so no question is left alone
pub fn collect(line: &str) {
    if CUR.lock().is_some() {
        OUT_BUF.lock().push(line.to_string());
    }
}

pub fn end(ok: bool) {
    let lines: Vec<String> = std::mem::take(&mut *OUT_BUF.lock());
    let text = lines.join("\n");
    let text = text.trim();
    let cur = CUR.lock().take();
    if let Some(mut r) = cur {
        if !text.is_empty() {
            ai::store().append_with(&r.chat_id, "assistant", text, &r.rid, "command");
            *LAST_OUTPUT.lock() = text.lines().next().unwrap_or("").to_string();
            r.replies += 1;
        }
        if r.replies == 0 {
            let store = ai::store();
            if ok {
                store.append_with(&r.chat_id, "assistant", "Выполнено.", &r.rid, "command");
            } else {
                store.append_with(&r.chat_id, "error", "Не получилось выполнить команду.", &r.rid, "command");
                crate::notify::push("system_error", "Ошибка", "Не получилось выполнить команду.", true);
            }
        }
        // the result of a command when nobody looks at the window (a voice command with the window hidden)
        let last = LAST_OUTPUT.lock().clone();
        if r.replies > 0 && !last.is_empty() {
            if !ok {
                // a command that failed and said why: the error is a notification too (shown, not said again)
                crate::notify::push("system_error", "Команда не выполнена", &last, true);
            } else if !crate::notify::window_visible() {
                crate::notify::push("command", "Результат команды", &last, crate::tts::is_enabled());
            }
        }
        *LAST.lock() = Some(Req { at: Instant::now(), ..r });
    }
}

// what Jarvis said last (a reply or an output line), for the status line and the session bar
pub fn last_output() -> String {
    LAST_OUTPUT.lock().clone()
}

pub fn note_output(text: &str) {
    *LAST_OUTPUT.lock() = text.trim().to_string();
}

// the request of the AI (it keeps its own messages) becomes the "last" one: the replies of a follow-up join it
pub fn adopt_ai_request() {
    if let Some((chat_id, rid)) = ai::last_request() {
        *LAST.lock() = Some(Req { rid, chat_id, replies: 1, at: Instant::now() });
    }
}

// the conversation state is global: tests of several modules that use it run one at a time
#[cfg(test)]
pub static TEST_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    // the current request and the store are global: one test at a time
    use super::TEST_LOCK as LOCK;

    fn last_messages(n: usize) -> Vec<ai::ChatMsg> {
        let store = ai::store();
        let chat = store.active_or_new();
        let msgs = store.load(&chat.id).unwrap().messages;
        msgs[msgs.len().saturating_sub(n)..].to_vec()
    }

    #[test]
    fn a_question_and_its_reply_are_stored_together_with_one_request_id() {
        let _g = LOCK.lock();
        let rid = begin("сколько времени", "command");
        reply("Сейчас 12:00", "command");
        end(true);
        let m = last_messages(2);
        assert_eq!((m[0].role.as_str(), m[0].content.as_str()), ("user", "сколько времени"));
        assert_eq!((m[1].role.as_str(), m[1].content.as_str()), ("assistant", "Сейчас 12:00"));
        assert_eq!(m[0].rid, rid);
        assert_eq!(m[1].rid, rid);
        assert_eq!(m[1].src, "command");
    }

    #[test]
    fn a_command_that_says_nothing_still_gets_an_answer_and_a_failure_is_an_error() {
        let _g = LOCK.lock();
        let rid = begin("сделай что-то тихое", "command");
        end(true);
        let m = last_messages(2);
        assert_eq!(m[1].content, "Выполнено.");
        assert_eq!(m[1].rid, rid);

        begin("сломай", "command");
        end(false);
        let m = last_messages(2);
        assert_eq!(m[1].role, "error", "a failed command is shown as an error, not as success");
    }

    #[test]
    fn printed_lines_become_one_reply_not_a_message_per_line() {
        let _g = LOCK.lock();
        begin("что ты умеешь", "command");
        collect("Все команды (3):");
        collect("  таймер");
        collect("  громкость");
        end(true);
        let m = last_messages(2);
        assert_eq!(m[1].content, "Все команды (3):\n  таймер\n  громкость");
        assert_eq!(m.iter().filter(|x| x.role == "assistant").count(), 1);
    }

    #[test]
    fn a_follow_up_joins_the_same_request_and_stores_no_second_question() {
        let _g = LOCK.lock();
        let rid = begin("что такое питон", "ai");
        fail("Нет соединения с сервером ИИ", "ai");
        end(false);
        let before = ai::store().load(&ai::store().active_or_new().id).unwrap().messages.len();
        assert!(continue_last());
        reply("Ищу: питон", "search");
        end(true);
        let chat = ai::store().load(&ai::store().active_or_new().id).unwrap();
        assert_eq!(chat.messages.len(), before + 1, "one more reply, no new question");
        let last = chat.messages.last().unwrap();
        assert_eq!((last.rid.as_str(), last.src.as_str()), (rid.as_str(), "search"));
        // the search reply is labelled, so it cannot be taken for an answer of the AI
        assert_ne!(last.src, "ai");
    }

    #[test]
    fn output_without_any_request_is_not_stored_as_a_message() {
        let _g = LOCK.lock();
        begin("раз", "command");
        end(true);
        *LAST.lock() = None;
        let before = last_messages(1)[0].content.clone();
        reply("Потерянная реплика", "command");
        assert_eq!(last_messages(1)[0].content, before, "no request: nothing is added to the history");
        assert_eq!(last_output(), "Потерянная реплика", "but the status line still shows it");
    }
}
