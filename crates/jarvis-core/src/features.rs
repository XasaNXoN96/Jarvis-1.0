//! The router of the text/voice commands of the new features (memory, translation, messenger). One entry: `handle(text, original)`;
//! `None` = not for these features, the normal command search goes on. Nothing here executes silently: the messenger always waits
//! for a confirmation, and every answer says what really happened.

use crate::{confirm, contacts, memory, messenger, store, textnorm, translate};
use parking_lot::Mutex;

pub struct Reply {
    pub text: String,
}

fn r(s: impl Into<String>) -> Option<Reply> {
    Some(Reply { text: s.into() })
}

// the message that waits for «да» / «нет» (and when it was prepared)
static PENDING_MSG: Mutex<Option<(String, u64)>> = parking_lot::const_mutex(None);
const PENDING_TTL: u64 = 120;

#[cfg(all(feature = "lua", target_os = "windows"))]
fn open(target: &str) -> Result<(), String> {
    crate::lua::shell_open(target)
}
#[cfg(not(all(feature = "lua", target_os = "windows")))]
fn open(_target: &str) -> Result<(), String> {
    Err("открытие ссылок доступно только в Windows".into())
}

pub fn pending_message() -> Option<String> {
    let p = PENDING_MSG.lock();
    p.as_ref().filter(|(_, t)| store::now() - t <= PENDING_TTL).map(|(id, _)| id.clone())
}

fn adapter_for(channel: &str) -> messenger::LinkAdapter {
    messenger::LinkAdapter { channel: channel.to_string() }
}

fn describe(o: &messenger::Outgoing) -> String {
    match o.status {
        messenger::Status::ManualSendRequired => format!("Открыл чат с «{}» в {} с готовым текстом. Нажмите «Отправить» там — сам я отправить не могу и не узнаю, отправили ли вы.", o.contact, if o.channel == "whatsapp" { "WhatsApp" } else { "Telegram" }),
        messenger::Status::Sent => format!("Отправлено: «{}».", o.contact),
        messenger::Status::Error => format!("Не отправлено: {}.", o.note),
        s => format!("Сообщение «{}»: {}.", o.contact, s.label()),
    }
}

// the same «да» as by voice, from the buttons of the window: the adapter is called once, a link is opened for the manual send
pub fn confirm_message(id: &str) -> Result<messenger::Outgoing, String> {
    let channel = messenger::get(id).map(|o| o.channel).ok_or("такого сообщения нет")?;
    let o = messenger::confirm(id, &adapter_for(&channel))?;
    if pending_message().as_deref() == Some(id) {
        *PENDING_MSG.lock() = None;
    }
    if o.status == messenger::Status::ManualSendRequired {
        open(&o.url).map_err(|e| format!("Не удалось открыть чат: {}", e))?;
    }
    Ok(o)
}

pub fn cancel_message(id: &str) -> Result<messenger::Outgoing, String> {
    if pending_message().as_deref() == Some(id) {
        *PENDING_MSG.lock() = None;
    }
    messenger::cancel(id)
}

fn answer_pending(text: &str) -> Option<Reply> {
    let id = pending_message()?;
    match confirm::classify(text) {
        confirm::Answer::Yes => {
            *PENDING_MSG.lock() = None;
            match messenger::get(&id).map(|o| o.channel) {
                Some(ch) => match messenger::confirm(&id, &adapter_for(&ch)) {
                    Ok(o) => {
                        if o.status == messenger::Status::ManualSendRequired {
                            if let Err(e) = open(&o.url) {
                                return r(format!("Не удалось открыть чат: {}. Текст сообщения сохранён в истории.", e));
                            }
                        }
                        r(describe(&o))
                    }
                    Err(e) => r(e),
                },
                None => r("Сообщение не найдено."),
            }
        }
        confirm::Answer::No => {
            *PENDING_MSG.lock() = None;
            match messenger::cancel(&id) {
                Ok(_) => r("Отменено, ничего не отправлено."),
                Err(e) => r(e),
            }
        }
        confirm::Answer::Other => None,
    }
}

fn do_translate(req: &translate::Request) -> String {
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let ask = |system: &str, user: &str| crate::ai::complete(system, user, &cancel).map_err(|e| e.user_text());
    match translate::translate(req, &ask) {
        Ok(t) => t,
        Err(e) => e,
    }
}

fn do_messenger(p: &messenger::Parsed) -> Option<Reply> {
    match messenger::prepare(p) {
        messenger::Prepared::Ask(q) => r(q),
        messenger::Prepared::Ready(o) => {
            if let Err(e) = messenger::stage(&o) {
                return r(e);
            }
            *PENDING_MSG.lock() = Some((o.id.clone(), store::now()));
            r(format!("Сообщение для «{}» ({}): «{}». Отправить? Скажите «да» или «нет».", o.contact, if o.channel == "whatsapp" { "WhatsApp" } else { "Telegram" }, o.body))
        }
    }
}

fn do_memory(text: &str) -> Option<Reply> {
    if let Some(fact) = memory::parse_remember(text) {
        return match memory::add(&fact, "command") {
            Ok(memory::Added::Added(_)) => r(format!("Запомнил: {}.", fact)),
            Ok(memory::Added::Duplicate(_)) => r("Это я уже помню."),
            Ok(memory::Added::Conflict(_, old)) => {
                // the old fact is replaced only when the user says so
                r(format!("Раньше я помнил иначе: «{}». Если нужно заменить, скажите «забудь» и это старое, затем повторите.", old))
            }
            Err(e) => r(e),
        };
    }
    if let Some(q) = memory::parse_recall(text) {
        let hits = memory::search(&q);
        return if hits.is_empty() {
            r("Ничего об этом не помню.")
        } else {
            r(hits.iter().take(5).map(|e| e.text.clone()).collect::<Vec<_>>().join(". "))
        };
    }
    if let Some(q) = memory::parse_forget(text) {
        let hits = memory::search(&q);
        return match hits.len() {
            0 => r("Такого в памяти нет."),
            1 => {
                memory::delete(&hits[0].id);
                r(format!("Забыл: {}.", hits[0].text))
            }
            n => r(format!("Подходит {} записей, уточните, какую забыть: {}.", n, hits.iter().take(3).map(|e| format!("«{}»", e.text)).collect::<Vec<_>>().join(", "))),
        };
    }
    None
}

// `text` is the original phrase of the user (capitals and punctuation kept)
pub fn handle(text: &str) -> Option<Reply> {
    if let Some(a) = answer_pending(text) {
        return Some(a);
    }
    if let Some(req) = translate::parse(text) {
        return r(do_translate(&req));
    }
    if let Some(p) = messenger::parse(text) {
        return do_messenger(&p);
    }
    let _ = (&contacts::list, &textnorm::normalize);
    do_memory(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_flow() {
        store::with_temp(|| {
            assert_eq!(handle("запомни, что мой кот — Барсик").unwrap().text, "Запомнил: мой кот — Барсик.");
            assert!(handle("что ты помнишь про кота").unwrap().text.contains("Барсик"));
            assert!(handle("запомни мой пароль 1234").unwrap().text.contains("не запоминаю"));
            assert!(handle("забудь про кота").unwrap().text.starts_with("Забыл"));
            assert_eq!(handle("что ты помнишь про кота").unwrap().text, "Ничего об этом не помню.");
            assert!(handle("открой браузер").is_none());
        });
    }
    #[test]
    fn messenger_waits_for_the_answer() {
        store::with_temp(|| {
            contacts::save(contacts::Contact { name: "Малика".into(), aliases: vec!["жена".into()], phone: "998901234567".into(), ..Default::default() }).unwrap();
            let q = handle("напиши жене: буду поздно").unwrap().text;
            assert!(q.contains("Отправить?"), "{}", q);
            // a different phrase does not answer it
            assert!(handle("открой браузер").is_none());
            let id = pending_message().unwrap();
            assert_eq!(messenger::get(&id).unwrap().status, messenger::Status::AwaitingConfirmation);
            assert_eq!(handle("нет").unwrap().text, "Отменено, ничего не отправлено.");
            assert_eq!(messenger::get(&id).unwrap().status, messenger::Status::Cancelled);
            assert!(pending_message().is_none());
            // unknown person: a question, nothing staged
            assert!(handle("напиши начальнику: привет").unwrap().text.contains("Не нашёл"));
            assert!(pending_message().is_none());
        });
    }
}
