//! How a message of a messenger or an SMS is read aloud: a short natural phrase in Russian, English or Uzbek, the sender first, the text
//! cleaned (links become the word «ссылка», long texts are shortened), with a privacy level (read everything / only who wrote / only that a
//! message came). The last message is kept in memory only, so «повтори сообщение» can read it again; «стоп» is the ordinary stop of the voice.
//! Nothing is stored on disk except the two settings.

use crate::{store, textnorm};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

const FILE: &str = "sms_reading.json";
pub const MAX_CHARS: usize = 220;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Privacy {
    // sender and text
    Full,
    // only who wrote
    Sender,
    // only that a message came
    Silent,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Conf {
    #[serde(default = "full")]
    pub privacy: Privacy,
    // "ru" (default) | "auto" (the language of the program) | "en" | "uz"
    #[serde(default = "auto")]
    pub lang: String,
}
fn full() -> Privacy {
    Privacy::Full
}
fn auto() -> String {
    // Russian, as the phrase always was; «auto» follows the language of the program
    "ru".into()
}
impl Default for Conf {
    fn default() -> Self {
        Conf { privacy: Privacy::Full, lang: "ru".into() }
    }
}

pub fn conf() -> Conf {
    store::read::<Conf>(FILE).0
}
pub fn set(privacy: Privacy, lang: &str) -> Result<(), String> {
    if !["auto", "ru", "en", "uz"].contains(&lang) {
        return Err("Язык: auto, ru, en или uz.".into());
    }
    store::write(FILE, &Conf { privacy, lang: lang.into() })
}

// «Иван Петров (2)» → «Иван Петров»; «Telegram: Иван» → «Иван»
pub fn clean_sender(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let s = s.trim_end_matches(':').trim();
    // a trailing «(3)» counter of unread messages
    let s = match s.rfind('(') {
        Some(i) if s.ends_with(')') && s[i + 1..s.len() - 1].chars().all(|c| c.is_ascii_digit()) => s[..i].trim(),
        _ => s,
    };
    s.chars().take(60).collect()
}

pub fn clean_text(t: &str, lang: &str) -> String {
    let link = match lang {
        "en" => "link",
        "uz" => "havola",
        _ => "ссылка",
    };
    let words: Vec<String> = t
        .split_whitespace()
        .map(|w| if w.starts_with("http://") || w.starts_with("https://") || w.starts_with("www.") { link.to_string() } else { w.to_string() })
        .collect();
    let joined: String = words.join(" ").chars().filter(|c| !c.is_control()).collect();
    if joined.chars().count() > MAX_CHARS {
        let cut: String = joined.chars().take(MAX_CHARS).collect();
        // at a word end
        let cut = cut.rsplit_once(' ').map(|(a, _)| a.to_string()).unwrap_or(cut);
        format!("{}…", cut.trim_end_matches([',', '.', ' ']))
    } else {
        joined
    }
}

fn pick_lang(setting: &str, program: &str) -> &'static str {
    match if setting == "auto" { program } else { setting } {
        "en" => "en",
        "uz" => "uz",
        _ => "ru",
    }
}

pub fn phrase(network: &str, sender: &str, text: &str, c: &Conf, program_lang: &str) -> String {
    let lang = pick_lang(&c.lang, program_lang);
    let who = clean_sender(sender);
    let body = clean_text(text, lang);
    match (lang, c.privacy) {
        ("en", Privacy::Silent) => format!("You have a new message on {}", network),
        ("uz", Privacy::Silent) => format!("{}da yangi xabar bor", network),
        (_, Privacy::Silent) => format!("Вам пришло сообщение в {}", network),
        ("en", _) => match (who.is_empty(), c.privacy == Privacy::Sender || body.is_empty()) {
            (true, true) => format!("You have a message on {}", network),
            (false, true) => format!("You have a message on {} from {}", network, who),
            (true, false) => format!("You have a message on {}: {}", network, body),
            (false, false) => format!("You have a message on {} from {}: {}", network, who, body),
        },
        ("uz", _) => match (who.is_empty(), c.privacy == Privacy::Sender || body.is_empty()) {
            (true, true) => format!("{}da xabar keldi", network),
            (false, true) => format!("{}da {}dan xabar keldi", network, who),
            (true, false) => format!("{}da xabar keldi: {}", network, body),
            (false, false) => format!("{}da {}dan xabar keldi: {}", network, who, body),
        },
        _ => match (who.is_empty(), c.privacy == Privacy::Sender || body.is_empty()) {
            (true, true) => format!("Вам пришло сообщение в {}", network),
            (false, true) => format!("Вам пришло сообщение в {} от {}", network, who),
            (true, false) => format!("Вам пришло сообщение в {}: {}", network, body),
            (false, false) => format!("Вам пришло сообщение в {} от {}: {}", network, who, body),
        },
    }
}

static LAST: Mutex<Option<(String, String, String)>> = parking_lot::const_mutex(None);

// the phrase for a notification: the first line is the sender, the rest is the text; the message is kept for «повтори»
pub fn say(network: &str, parts: &[String]) -> String {
    let (who, text) = match parts.len() {
        0 => (String::new(), String::new()),
        1 => (String::new(), parts[0].clone()),
        _ => (parts[0].clone(), parts[1..].join(" — ")),
    };
    *LAST.lock() = Some((network.to_string(), who.clone(), text.clone()));
    let c = conf();
    // a message whose text was not read aloud (the silent level, or the speech is off) waits in the list of the unread ones
    let spoken_text = c.privacy == Privacy::Full && crate::tts::is_enabled();
    note_message(network, &who, &text, !spoken_text);
    phrase(network, &who, &text, &c, &crate::i18n::get_language())
}

const QUEUE_MAX: usize = 10;
struct Item {
    net: String,
    who: String,
    text: String,
    unread: bool,
}
#[cfg(not(test))]
static QUEUE: Mutex<Vec<Item>> = parking_lot::const_mutex(Vec::new());
// (a test has its own list: other tests that say a message at the same time must not change it)
#[cfg(test)]
thread_local! {
    static QUEUE_T: std::cell::RefCell<Vec<Item>> = std::cell::RefCell::new(Vec::new());
}
fn with_queue<R>(f: impl FnOnce(&mut Vec<Item>) -> R) -> R {
    #[cfg(not(test))]
    {
        f(&mut QUEUE.lock())
    }
    #[cfg(test)]
    {
        QUEUE_T.with(|q| f(&mut q.borrow_mut()))
    }
}

fn note_message(net: &str, who: &str, text: &str, unread: bool) {
    with_queue(|q| {
        q.push(Item { net: net.to_string(), who: who.to_string(), text: text.to_string(), unread });
        let extra = q.len().saturating_sub(QUEUE_MAX);
        q.drain(..extra);
    });
}

// «что пришло» / «прочитай новые сообщения» (all the unread ones) and «следующее сообщение» / «прочитай дальше» (one): Some(all)
pub fn unread_requested(text: &str) -> Option<bool> {
    match textnorm::normalize(text).as_str() {
        "что пришло" | "какие новые сообщения" | "прочитай новые сообщения" | "прочитай непрочитанные сообщения" | "прочитай все новые сообщения" | "read new messages" | "read my messages" | "yangi xabarlarni o qi" => Some(true),
        "следующее сообщение" | "прочитай дальше" | "прочитай следующее сообщение" | "прочитай следующее" | "next message" | "read the next message" | "keyingi xabar" => Some(false),
        _ => None,
    }
}

// reads (and marks as read) the unread messages, with their text whatever the privacy level is: the owner asked for them
pub fn read_unread(all: bool) -> String {
    let c = Conf { privacy: Privacy::Full, ..conf() };
    let lang = crate::i18n::get_language();
    with_queue(|q| {
        let mut out = Vec::new();
        for it in q.iter_mut().filter(|i| i.unread) {
            out.push(phrase(&it.net, &it.who, &it.text, &c, &lang));
            it.unread = false;
            if !all {
                break;
            }
        }
        if out.is_empty() {
            return "Непрочитанных сообщений нет.".to_string();
        }
        let left = q.iter().filter(|i| i.unread).count();
        let mut text = out.join(". ");
        if !all && left > 0 {
            text.push_str(&format!(". Ещё непрочитанных: {}. Скажите «следующее сообщение».", left));
        }
        text
    })
}

// «повтори сообщение» / «прочитай ещё раз» / «repeat the message»: the last message again, with the text even if the privacy level is lower
pub fn repeat_requested(text: &str) -> bool {
    matches!(textnorm::normalize(text).as_str(), "повтори сообщение" | "повтори последнее сообщение" | "прочитай сообщение ещё раз" | "прочитай сообщение еще раз" | "прочитай еще раз" | "прочитай ещё раз" | "repeat the message" | "read the message again" | "xabarni takrorla")
}

pub fn repeat_last() -> String {
    match LAST.lock().clone() {
        None => "Новых сообщений, которые можно повторить, у меня нет.".into(),
        Some((net, who, text)) => {
            let c = Conf { privacy: Privacy::Full, ..conf() };
            phrase(&net, &who, &text, &c, &crate::i18n::get_language())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(p: Privacy, l: &str) -> Conf {
        Conf { privacy: p, lang: l.into() }
    }

    #[test]
    fn the_russian_phrase_keeps_its_old_shape() {
        assert_eq!(phrase("Telegram", "Иван", "привет, как дела?", &c(Privacy::Full, "ru"), "ru"), "Вам пришло сообщение в Telegram от Иван: привет, как дела?");
        assert_eq!(phrase("Telegram", "", "привет", &c(Privacy::Full, "ru"), "ru"), "Вам пришло сообщение в Telegram: привет");
        assert_eq!(phrase("Telegram", "", "", &c(Privacy::Full, "ru"), "ru"), "Вам пришло сообщение в Telegram");
    }

    #[test]
    fn three_languages_and_three_privacy_levels() {
        assert_eq!(phrase("Telegram", "Ivan", "hi there", &c(Privacy::Full, "en"), "ru"), "You have a message on Telegram from Ivan: hi there");
        assert_eq!(phrase("Telegram", "Ivan", "salom", &c(Privacy::Full, "uz"), "ru"), "Telegramda Ivandan xabar keldi: salom");
        assert_eq!(phrase("Telegram", "Иван", "секрет", &c(Privacy::Sender, "ru"), "ru"), "Вам пришло сообщение в Telegram от Иван");
        assert_eq!(phrase("Telegram", "Иван", "секрет", &c(Privacy::Silent, "en"), "ru"), "You have a new message on Telegram");
        // «auto» follows the language of the program
        assert_eq!(phrase("Telegram", "Ivan", "hi", &c(Privacy::Full, "auto"), "en"), "You have a message on Telegram from Ivan: hi");
        assert!(!phrase("Telegram", "Иван", "пароль 1234", &c(Privacy::Silent, "ru"), "ru").contains("1234"));
    }

    #[test]
    fn senders_and_texts_are_cleaned() {
        assert_eq!(clean_sender("  Иван   Петров (3) "), "Иван Петров");
        assert_eq!(clean_sender("Иван:"), "Иван");
        assert_eq!(clean_sender("Группа (дом)"), "Группа (дом)");
        assert_eq!(clean_text("смотри https://example.com/a?b=1 и www.x.ru", "ru"), "смотри ссылка и ссылка");
        assert_eq!(clean_text("see https://x.org", "en"), "see link");
        let long = "слово ".repeat(100);
        let t = clean_text(&long, "ru");
        assert!(t.chars().count() <= MAX_CHARS + 1 && t.ends_with('…') && !t.contains("  "));
        assert_eq!(clean_text("a\u{7}b\nc", "ru"), "ab c".replace("ab c", "ab c"));
    }

    #[test]
    fn unread_messages_wait_and_are_read_on_request_one_by_one_or_all() {
        store::with_temp(|| {
            set(Privacy::Silent, "ru").unwrap();
            say("Telegram", &["Иван".to_string(), "первое".to_string()]);
            say("WhatsApp", &["Анна".to_string(), "второе".to_string()]);
            say("Telegram", &["Олег".to_string(), "третье".to_string()]);
            assert_eq!(unread_requested("Следующее сообщение"), Some(false));
            assert_eq!(unread_requested("что пришло"), Some(true));
            assert_eq!(unread_requested("открой браузер"), None);
            let one = read_unread(false);
            assert!(one.contains("Иван: первое") && one.contains("Ещё непрочитанных: 2"), "{}", one);
            let rest = read_unread(true);
            assert!(rest.contains("Анна: второе") && rest.contains("Олег: третье") && !rest.contains("первое"), "{}", rest);
            assert_eq!(read_unread(true), "Непрочитанных сообщений нет.");
            // only the last ten are kept
            for i in 0..15 {
                say("Telegram", &["X".to_string(), format!("m{}", i)]);
            }
            let all = read_unread(true);
            assert!(all.contains("m14") && all.contains("m5") && !all.contains("m4:") && !all.contains("m0"), "{}", all);
            // a message that was read aloud in full is not left unread (when the speech is on); here the level is silent, so they waited
        });
    }

    #[test]
    fn the_last_message_can_be_repeated_with_its_text() {
        store::with_temp(|| {
            set(Privacy::Silent, "ru").unwrap();
            let said = say("Telegram", &["Иван".to_string(), "встреча в 5".to_string()]);
            assert_eq!(said, "Вам пришло сообщение в Telegram", "silent level: no text read aloud");
            assert_eq!(repeat_last(), "Вам пришло сообщение в Telegram от Иван: встреча в 5");
            assert!(set(Privacy::Full, "de").is_err());
            assert!(repeat_requested("Повтори сообщение") && !repeat_requested("открой браузер"));
        });
    }
}
