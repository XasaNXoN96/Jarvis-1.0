//! Jarvis Messenger: «напиши жене: …». A message is prepared, shown, and sent only after the user confirms it. A message is called
//! «отправлено» only when the adapter itself confirms delivery to the service; the Telegram and WhatsApp adapters here open the chat
//! with the text filled in (`ManualSendRequired`): the user presses «Отправить» there. History keeps every state honestly.

use crate::contacts::{self, Contact, Resolved};
use crate::{store, textnorm};
use serde::{Deserialize, Serialize};

const FILE: &str = "messages_out.json";
const HISTORY_MAX: usize = 200;
pub const MAX_BODY: usize = 4000;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Prepared,
    AwaitingConfirmation,
    Sending,
    Sent,
    Error,
    Cancelled,
    ManualSendRequired,
}

impl Status {
    pub fn is_final(self) -> bool {
        matches!(self, Status::Sent | Status::Error | Status::Cancelled | Status::ManualSendRequired)
    }
    pub fn label(self) -> &'static str {
        match self {
            Status::Prepared => "подготовлено",
            Status::AwaitingConfirmation => "ждёт подтверждения",
            Status::Sending => "отправляется",
            Status::Sent => "отправлено",
            Status::Error => "ошибка",
            Status::Cancelled => "отменено",
            Status::ManualSendRequired => "нужно нажать «Отправить» в приложении",
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Outgoing {
    pub id: String,
    pub contact_id: String,
    pub contact: String,
    pub channel: String,
    pub body: String,
    pub status: Status,
    pub created: u64,
    pub updated: u64,
    #[serde(default)]
    pub note: String,
    #[serde(default)]
    pub url: String,
}

#[derive(Debug, PartialEq)]
pub struct Parsed {
    pub who: String,
    pub body: String,
    pub channel: Option<&'static str>,
}

fn channel_word(w: &str) -> Option<&'static str> {
    let w = textnorm::normalize(w);
    match w.as_str() {
        "телеграм" | "телеграме" | "телеграму" | "telegram" | "telegramda" | "telegramga" | "тг" => Some("telegram"),
        "ватсап" | "вацап" | "ватсапе" | "ватсапп" | "whatsapp" | "whatsappda" | "whatsappga" | "вотсап" => Some("whatsapp"),
        _ => None,
    }
}

// RU / EN / UZ: «напиши жене: привет», «отправь маме в телеграм: еду», «send a message to wife: hi», «xotinimga yoz: salom»
pub fn parse(text: &str) -> Option<Parsed> {
    let t = text.trim();
    let low = t.to_lowercase();
    let (head, body) = {
        let i = t.find(':').or_else(|| low.find(", что ").map(|i| i + 1))?;
        let b: String = t[i..].trim_start_matches(':').trim_start_matches(',').trim().trim_start_matches("что ").trim().to_string();
        (low[..low.char_indices().map(|(k, _)| k).find(|k| *k >= i).unwrap_or(low.len())].to_string(), b)
    };
    if body.is_empty() {
        return None;
    }
    let words: Vec<&str> = head.split_whitespace().collect();
    if words.is_empty() {
        return None;
    }
    // the verb at the start
    let ru = ["напиши", "отправь", "отправить", "пошли", "скажи", "сообщи", "черкни"];
    let en = ["send", "text", "message", "write", "tell"];
    let first = words[0].trim_matches(|c: char| !c.is_alphanumeric());
    let uz_verb = words.last().map(|w| ["yoz", "yubor", "ayt", "yozing"].contains(&w.trim_matches(|c: char| !c.is_alphanumeric()))).unwrap_or(false);
    let mut rest: Vec<&str> = Vec::new();
    let mut channel = None;
    if ru.contains(&first) || en.contains(&first) {
        let skip = ["сообщение", "смс", "sms", "a", "an", "message", "to", "сообщи", "в", "через", "via", "on", "in", "по", "что"];
        for w in &words[1..] {
            let wn = w.trim_matches(|c: char| !c.is_alphanumeric());
            if let Some(c) = channel_word(wn) {
                channel = Some(c);
            } else if !skip.contains(&wn) {
                rest.push(wn);
            }
        }
    } else if uz_verb {
        for w in &words[..words.len() - 1] {
            let wn = w.trim_matches(|c: char| !c.is_alphanumeric());
            if let Some(c) = channel_word(wn) {
                channel = Some(c);
            } else if !["xabar", "sms", "ga"].contains(&wn) {
                rest.push(wn);
            }
        }
    } else {
        return None;
    }
    if rest.is_empty() {
        return None;
    }
    Some(Parsed { who: rest.join(" "), body, channel })
}

pub enum Prepared {
    Ready(Outgoing),
    // a question for the user (several contacts, no contact, no address, channel to choose)
    Ask(String),
}

fn pick_channel(c: &Contact, asked: Option<&str>) -> Result<String, String> {
    let has = |ch: &str| match ch {
        "telegram" => !c.telegram.is_empty(),
        "whatsapp" => !c.phone.is_empty(),
        _ => false,
    };
    if let Some(ch) = asked {
        return if has(ch) { Ok(ch.to_string()) } else { Err(format!("У контакта «{}» нет адреса для {}. Добавьте его в «Мои контакты».", c.name, if ch == "telegram" { "Telegram" } else { "WhatsApp" })) };
    }
    if has(&c.preferred) {
        return Ok(c.preferred.clone());
    }
    match (has("telegram"), has("whatsapp")) {
        (true, false) => Ok("telegram".into()),
        (false, true) => Ok("whatsapp".into()),
        (true, true) => Err(format!("Куда написать «{}»: в Telegram или в WhatsApp?", c.name)),
        (false, false) => Err(format!("У контакта «{}» нет ни номера, ни имени в Telegram. Добавьте их в «Мои контакты».", c.name)),
    }
}

pub fn prepare(p: &Parsed) -> Prepared {
    prepare_in(&contacts::list(), p)
}

pub fn prepare_in(all: &[Contact], p: &Parsed) -> Prepared {
    if p.body.chars().count() > MAX_BODY {
        return Prepared::Ask(format!("Сообщение длиннее {} символов.", MAX_BODY));
    }
    let c = match contacts::resolve_in(all, &p.who) {
        Resolved::One(c) => c,
        Resolved::Many(v) => return Prepared::Ask(format!("Кому именно: {}?", v.iter().map(|c| c.name.clone()).collect::<Vec<_>>().join(" или "))),
        Resolved::None => return Prepared::Ask(format!("Не нашёл контакт «{}». Добавьте его в «Мои контакты» или скажите имя иначе.", p.who)),
    };
    let channel = match pick_channel(&c, p.channel) {
        Ok(ch) => ch,
        Err(q) => return Prepared::Ask(q),
    };
    let now = store::now();
    Prepared::Ready(Outgoing { id: store::new_id(), contact_id: c.id, contact: c.name, channel, body: p.body.clone(), status: Status::AwaitingConfirmation, created: now, updated: now, note: String::new(), url: String::new() })
}

pub enum Outcome {
    // the service confirmed the delivery
    Delivered,
    // the chat is opened with the text: the user has to press «Отправить»
    Manual(String),
    Failed(String),
}

pub trait Adapter {
    fn send(&self, c: &Contact, body: &str) -> Outcome;
}

fn pct(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect()
}

// WhatsApp (wa.me) and Telegram (t.me) have no way to send for the user without their account: the chat opens, the user sends
pub struct LinkAdapter {
    pub channel: String,
}
impl Adapter for LinkAdapter {
    fn send(&self, c: &Contact, body: &str) -> Outcome {
        match self.channel.as_str() {
            "whatsapp" if !c.phone.is_empty() => Outcome::Manual(format!("https://wa.me/{}?text={}", c.phone, pct(body))),
            "telegram" if !c.telegram.is_empty() => Outcome::Manual(format!("https://t.me/{}", c.telegram)),
            "telegram" if !c.phone.is_empty() => Outcome::Manual(format!("https://t.me/+{}", c.phone)),
            ch => Outcome::Failed(format!("для канала «{}» у контакта нет адреса", ch)),
        }
    }
}

pub fn history() -> Vec<Outgoing> {
    store::read::<Vec<Outgoing>>(FILE).0
}

// the history is changed under this lock: a read-modify-write of two threads (a button and a spoken «да») must not lose a line or let both of them send
static LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

fn put(o: &Outgoing) -> Result<(), String> {
    let mut h = history();
    match h.iter_mut().find(|x| x.id == o.id) {
        Some(x) => *x = o.clone(),
        None => h.push(o.clone()),
    }
    let n = h.len();
    if n > HISTORY_MAX {
        h.drain(..n - HISTORY_MAX);
    }
    store::write(FILE, &h)
}

pub fn get(id: &str) -> Option<Outgoing> {
    history().into_iter().find(|o| o.id == id)
}

// remembers a prepared message (it waits for the confirmation)
pub fn stage(o: &Outgoing) -> Result<(), String> {
    let _g = LOCK.lock();
    put(o)
}

pub fn cancel(id: &str) -> Result<Outgoing, String> {
    let _g = LOCK.lock();
    let mut o = get(id).ok_or("такого сообщения нет")?;
    if o.status.is_final() {
        return Err(format!("Сообщение уже в состоянии «{}».", o.status.label()));
    }
    o.status = Status::Cancelled;
    o.updated = store::now();
    put(&o)?;
    Ok(o)
}

// the user said «да»: the adapter is called once; a message that is final is never sent again
pub fn confirm(id: &str, adapter: &dyn Adapter) -> Result<Outgoing, String> {
    // the message is claimed (→ «отправляется») under the lock, so only ONE of two simultaneous «да» reaches the adapter
    let mut o = {
        let _g = LOCK.lock();
        let mut o = get(id).ok_or("такого сообщения нет")?;
        if o.status != Status::AwaitingConfirmation && o.status != Status::Prepared {
            return Err(format!("Сообщение уже в состоянии «{}»: второй раз не отправляю.", o.status.label()));
        }
        if contacts::by_id(&o.contact_id).is_none() {
            return Err("контакт удалён".into());
        }
        o.status = Status::Sending;
        o.updated = store::now();
        put(&o)?;
        o
    };
    let c = contacts::by_id(&o.contact_id);
    let outcome = match &c {
        Some(c) => adapter.send(c, &o.body),
        None => Outcome::Failed("контакт удалён".into()),
    };
    match outcome {
        Outcome::Delivered => o.status = Status::Sent,
        Outcome::Manual(url) => {
            o.status = Status::ManualSendRequired;
            o.url = url;
            o.note = manual_note(&o.channel).into();
        }
        Outcome::Failed(e) => {
            o.status = Status::Error;
            o.note = e;
        }
    }
    o.updated = store::now();
    let _g = LOCK.lock();
    put(&o)?;
    Ok(o)
}

// What the user has to do after the chat is opened. WhatsApp gets the text in the link; the Telegram link opens the chat only (it has no place for a text), so the text must be pasted by hand.
pub fn manual_note(channel: &str) -> &'static str {
    if channel == "telegram" {
        "Чат открыт БЕЗ текста (Telegram не принимает текст из ссылки): вставьте сообщение сами, оно сохранено в истории, и нажмите «Отправить». Я не могу узнать, отправили ли вы."
    } else {
        "Чат открыт с текстом: нажмите «Отправить» в приложении. Я не могу узнать, отправили ли вы."
    }
}

pub fn clear_history() -> bool {
    let _g = LOCK.lock();
    store::write(FILE, &Vec::<Outgoing>::new()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctc(name: &str, aliases: &[&str], phone: &str, tg: &str) -> Contact {
        Contact { id: name.into(), name: name.into(), aliases: aliases.iter().map(|s| s.to_string()).collect(), phone: phone.into(), telegram: tg.into(), preferred: String::new(), updated: 0 }
    }

    #[test]
    fn parses_three_languages() {
        assert_eq!(parse("напиши жене: привет, я задержусь").unwrap(), Parsed { who: "жене".into(), body: "привет, я задержусь".into(), channel: None });
        assert_eq!(parse("Отправь маме в телеграм: еду домой").unwrap(), Parsed { who: "маме".into(), body: "еду домой".into(), channel: Some("telegram") });
        assert_eq!(parse("send a message to wife: I am late").unwrap(), Parsed { who: "wife".into(), body: "I am late".into(), channel: None });
        assert_eq!(parse("xotinimga yoz: salom").unwrap().who, "xotinimga");
        assert_eq!(parse("напиши жене, что опаздываю").unwrap().body, "опаздываю");
        assert!(parse("открой браузер").is_none());
        assert!(parse("напиши жене:").is_none());
    }
    #[test]
    fn prepares_and_asks() {
        let all = vec![ctc("Малика", &["жена"], "998901234567", ""), ctc("Иван", &[], "", "ivan"), ctc("Иван Второй", &["Иван"], "99890", "x")];
        let p = |who: &str| Parsed { who: who.into(), body: "hi".into(), channel: None };
        match prepare_in(&all, &p("жене")) {
            Prepared::Ready(o) => {
                assert_eq!(o.channel, "whatsapp");
                assert_eq!(o.status, Status::AwaitingConfirmation);
            }
            _ => panic!(),
        }
        assert!(matches!(prepare_in(&all, &p("Ивану")), Prepared::Ask(q) if q.contains("Кому именно")));
        assert!(matches!(prepare_in(&all, &p("начальнику")), Prepared::Ask(q) if q.contains("Не нашёл")));
        let both = vec![ctc("Аня", &[], "998901234567", "anya")];
        assert!(matches!(prepare_in(&both, &p("Ане")), Prepared::Ask(q) if q.contains("Telegram или в WhatsApp")));
        let tg = Parsed { who: "Ане".into(), body: "hi".into(), channel: Some("telegram") };
        assert!(matches!(prepare_in(&both, &tg), Prepared::Ready(o) if o.channel == "telegram"));
        let no_addr = vec![ctc("Боб", &[], "", "")];
        assert!(matches!(prepare_in(&no_addr, &p("Бобу")), Prepared::Ask(q) if q.contains("нет ни номера")));
    }

    struct Fake(Outcome2);
    enum Outcome2 {
        Delivered,
        Failed,
    }
    impl Adapter for Fake {
        fn send(&self, _: &Contact, _: &str) -> Outcome {
            match self.0 {
                Outcome2::Delivered => Outcome::Delivered,
                Outcome2::Failed => Outcome::Failed("нет сети".into()),
            }
        }
    }

    fn staged() -> Outgoing {
        contacts::save(Contact { id: "m".into(), name: "Малика".into(), phone: "998901234567".into(), ..Default::default() }).unwrap();
        let Prepared::Ready(o) = prepare(&Parsed { who: "Малика".into(), body: "привет мир".into(), channel: None }) else { panic!() };
        stage(&o).unwrap();
        o
    }

    #[test]
    fn nothing_is_sent_before_confirmation_and_never_twice() {
        store::with_temp(|| {
            let o = staged();
            assert_eq!(get(&o.id).unwrap().status, Status::AwaitingConfirmation);
            let sent = confirm(&o.id, &Fake(Outcome2::Delivered)).unwrap();
            assert_eq!(sent.status, Status::Sent);
            assert!(confirm(&o.id, &Fake(Outcome2::Delivered)).is_err());
            assert!(cancel(&o.id).is_err());
        });
    }
    #[test]
    fn links_are_manual_not_sent() {
        store::with_temp(|| {
            let o = staged();
            let r = confirm(&o.id, &LinkAdapter { channel: "whatsapp".into() }).unwrap();
            assert_eq!(r.status, Status::ManualSendRequired);
            assert_eq!(r.url, "https://wa.me/998901234567?text=%D0%BF%D1%80%D0%B8%D0%B2%D0%B5%D1%82%20%D0%BC%D0%B8%D1%80");
            assert!(!r.note.is_empty());
        });
    }
    #[test]
    fn errors_and_cancel_are_recorded() {
        store::with_temp(|| {
            let o = staged();
            assert_eq!(confirm(&o.id, &Fake(Outcome2::Failed)).unwrap().status, Status::Error);
            let o2 = staged();
            assert_eq!(cancel(&o2.id).unwrap().status, Status::Cancelled);
            assert!(confirm(&o2.id, &Fake(Outcome2::Delivered)).is_err());
            assert_eq!(history().len(), 2);
        });
    }

    struct Counting(std::sync::atomic::AtomicUsize);
    impl Adapter for Counting {
        fn send(&self, _: &Contact, _: &str) -> Outcome {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(60));
            Outcome::Delivered
        }
    }

    #[test]
    fn two_simultaneous_confirmations_send_once() {
        store::with_temp(|| {
            let o = staged();
            let a = std::sync::Arc::new(Counting(std::sync::atomic::AtomicUsize::new(0)));
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    let (a, id) = (a.clone(), o.id.clone());
                    std::thread::spawn(move || confirm(&id, &*a).is_ok())
                })
                .collect();
            let ok = handles.into_iter().map(|h| h.join().unwrap()).filter(|x| *x).count();
            assert_eq!(ok, 1, "exactly one confirmation wins");
            assert_eq!(a.0.load(std::sync::atomic::Ordering::SeqCst), 1, "the adapter is called once");
            assert_eq!(get(&o.id).unwrap().status, Status::Sent);
            assert_eq!(history().len(), 1);
        });
    }

    #[test]
    fn a_deleted_contact_stops_the_confirmation_before_anything_is_claimed() {
        store::with_temp(|| {
            let o = staged();
            assert!(contacts::delete("m"));
            assert_eq!(confirm(&o.id, &Fake(Outcome2::Delivered)).unwrap_err(), "контакт удалён");
            assert_eq!(get(&o.id).unwrap().status, Status::AwaitingConfirmation, "the message stays confirmable/cancellable");
        });
    }

    #[test]
    fn the_telegram_chat_is_said_to_be_opened_without_the_text() {
        store::with_temp(|| {
            contacts::save(Contact { id: "t".into(), name: "Тима".into(), telegram: "tima".into(), ..Default::default() }).unwrap();
            let Prepared::Ready(o) = prepare(&Parsed { who: "Тима".into(), body: "секретный текст".into(), channel: None }) else { panic!() };
            stage(&o).unwrap();
            let r = confirm(&o.id, &LinkAdapter { channel: "telegram".into() }).unwrap();
            assert_eq!(r.status, Status::ManualSendRequired);
            assert_eq!(r.url, "https://t.me/tima");
            assert!(!r.url.contains("секретный") && r.note.contains("БЕЗ текста"), "{}", r.note);
            assert_eq!(get(&o.id).unwrap().body, "секретный текст", "the text stays in the history to be pasted");
            assert!(manual_note("whatsapp").contains("с текстом"));
        });
    }
}
