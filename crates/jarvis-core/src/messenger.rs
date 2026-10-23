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

// the same request said without a colon: «отправь сообщение маме через Telegram мама, я скоро буду», «пиши моей жене в ватсап еду домой».
// The words after the verb are taken as the person only when they are the name or an alias of a saved contact (the same word with another ending is
// the same word), so an ordinary phrase («отправь файл на почту») is never taken for a message. Then an optional «через / в Telegram», then the text.
pub fn parse_loose(text: &str) -> Option<Parsed> {
    parse_loose_in(&contacts::list(), text)
}

pub fn parse_loose_in(all: &[Contact], text: &str) -> Option<Parsed> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let clean = |w: &str| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
    let verbs = ["напиши", "напишите", "пиши", "отправь", "отправьте", "пошли", "пришли", "сообщи", "черкни", "передай"];
    if words.len() < 3 || !verbs.contains(&clean(words[0]).as_str()) {
        return None;
    }
    let filler = ["сообщение", "сообщения", "смс", "sms", "мне", "моей", "моему", "моя", "мой", "моим", "своей", "своему", "своим"];
    let mut i = 1;
    while i < words.len() && filler.contains(&clean(words[i]).as_str()) {
        i += 1;
    }
    let names_of = |c: &Contact| -> Vec<String> { std::iter::once(c.name.clone()).chain(c.aliases.iter().cloned()).collect() };
    // the longest run of words that is a contact (a full name wins over its first word)
    for k in (1..=3usize.min(words.len().saturating_sub(i))).rev() {
        let who_words: Vec<String> = words[i..i + k].iter().map(|w| clean(w)).collect();
        if who_words.iter().any(|w| w.is_empty() || channel_word(w).is_some() || ["через", "в", "по", "на", "что", "via", "on", "in"].contains(&w.as_str())) {
            continue;
        }
        let who = who_words.join(" ");
        let said: Vec<String> = who_words.iter().map(|w| textnorm::normalize(w)).collect();
        let same = |a: &str, b: &str| a == b || textnorm::same_stem(a, b);
        let known = all.iter().any(|c| {
            names_of(c).iter().any(|n| {
                let nw: Vec<String> = n.split_whitespace().map(textnorm::normalize).collect();
                // the same number of words, each one the same word (with another ending), or the first word of a full name
                (nw.len() == said.len() && nw.iter().zip(&said).all(|(a, b)| same(a, b))) || (said.len() == 1 && nw.first().map(|a| same(a, &said[0])).unwrap_or(false))
            })
        });
        if !known {
            continue;
        }
        let mut j = i + k;
        let mut channel = None;
        if j < words.len() && ["через", "в", "по", "на", "via", "on", "in"].contains(&clean(words[j]).as_str()) && j + 1 < words.len() {
            if let Some(c) = channel_word(&clean(words[j + 1])) {
                channel = Some(c);
                j += 2;
            }
        } else if j < words.len() {
            if let Some(c) = channel_word(&clean(words[j])) {
                channel = Some(c);
                j += 1;
            }
        }
        let body = words[j..].join(" ");
        let body = body.trim_start_matches(|c: char| c == ':' || c == ',' || c == '-' || c == '—' || c.is_whitespace());
        let body = body.strip_prefix("что ").unwrap_or(body).trim();
        if body.is_empty() {
            return None;
        }
        return Some(Parsed { who, body: body.to_string(), channel });
    }
    None
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

// the history for the page, newest first: `q` is a part of the contact name or of the text (any case), `state` is "" / "all", "done" (opened or sent),
// "failed", "cancelled" or "waiting" (not confirmed yet); an unknown state is the same as all
pub fn history_filtered(q: &str, state: &str, n: usize) -> Vec<Outgoing> {
    let q = crate::textnorm::normalize(q);
    history()
        .into_iter()
        .rev()
        .filter(|o| match state {
            "failed" => o.status == Status::Error,
            "cancelled" => o.status == Status::Cancelled,
            "waiting" => !o.status.is_final(),
            "done" => o.status.is_final() && o.status != Status::Error && o.status != Status::Cancelled,
            _ => true,
        })
        .filter(|o| q.is_empty() || crate::textnorm::normalize(&o.contact).contains(&q) || crate::textnorm::normalize(&o.body).contains(&q))
        .take(n)
        .collect()
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

// the address that opens the chat IN THE APPLICATION (no browser page in between): Telegram Desktop by `tg://`, WhatsApp Desktop by `whatsapp://`;
// None for an address that is not one of those chat links
pub fn app_url(url: &str) -> Option<String> {
    if let Some(u) = url.strip_prefix("https://t.me/+") {
        return Some(format!("tg://resolve?phone={}", u));
    }
    if let Some(u) = url.strip_prefix("https://t.me/") {
        return (!u.is_empty() && u.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then(|| format!("tg://resolve?domain={}", u));
    }
    if let Some(r) = url.strip_prefix("https://wa.me/") {
        let (phone, text) = r.split_once("?text=").unwrap_or((r, ""));
        return (!phone.is_empty() && phone.chars().all(|c| c.is_ascii_digit())).then(|| format!("whatsapp://send?phone={}&text={}", phone, text));
    }
    None
}

// the result of the automatic send: the keystroke was made (the message is «sent», delivery is not checked) or the reason why not
pub fn finish_auto(id: &str, result: Result<(), String>) {
    let _g = LOCK.lock();
    if let Some(mut o) = get(id) {
        match result {
            Ok(()) => {
                o.status = Status::Sent;
                o.note = "Чат открыт, текст вставлен и нажат Enter. Дошло ли сообщение, я проверить не могу.".into();
            }
            Err(e) => {
                o.status = Status::ManualSendRequired;
                o.note = e;
            }
        }
        o.updated = store::now();
        let _ = put(&o);
    }
}

// What happens after the chat is opened: Jarvis pastes the text (Telegram has no text in its link; WhatsApp gets it from the link) and presses Enter itself,
// when the window of the messenger is in front. It cannot see whether the message arrived.
pub fn manual_note(channel: &str) -> &'static str {
    if channel == "telegram" {
        "Чат открыт: вставляю текст и нажимаю Enter сам. Если окно Telegram не появится, сообщение останется в истории: отправьте его сами. Дошло ли оно, я проверить не могу."
    } else {
        "Чат открыт с текстом: нажимаю Enter сам. Если окно WhatsApp не появится, отправьте сами. Дошло ли сообщение, я проверить не могу."
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
    fn a_message_said_without_a_colon_is_understood_only_for_a_saved_contact() {
        let all = vec![
            Contact { id: "1".into(), name: "Мама".into(), telegram: "mama".into(), ..Default::default() },
            Contact { id: "2".into(), name: "Няшка".into(), aliases: vec!["жена".into()], telegram: "nyashka".into(), ..Default::default() },
            Contact { id: "3".into(), name: "Тима Ашуров".into(), phone: "998901234567".into(), ..Default::default() },
        ];
        let p = parse_loose_in(&all, "отправь сообщение маме через Telegram мама, я через какое-то время буду").unwrap();
        assert_eq!((p.who.as_str(), p.channel, p.body.as_str()), ("маме", Some("telegram"), "мама, я через какое-то время буду"));
        let p = parse_loose_in(&all, "пиши моей няшке через телеграм привет, как дела").unwrap();
        assert_eq!((p.who.as_str(), p.channel, p.body.as_str()), ("няшке", Some("telegram"), "привет, как дела"));
        let p = parse_loose_in(&all, "напиши жене в ватсап еду домой").unwrap();
        assert_eq!((p.who.as_str(), p.channel, p.body.as_str()), ("жене", Some("whatsapp"), "еду домой"));
        let p = parse_loose_in(&all, "напиши Тиме Ашурову что встреча в пять").unwrap();
        assert_eq!((p.who.as_str(), p.channel, p.body.as_str()), ("тиме ашурову", None, "встреча в пять"));
        // not a contact: not a message
        assert!(parse_loose_in(&all, "отправь файл на почту босса").is_none());
        assert!(parse_loose_in(&all, "напиши программу на питоне").is_none());
        // no text: nothing to send
        assert!(parse_loose_in(&all, "отправь маме через телеграм").is_none());
        assert!(parse_loose_in(&all, "открой телеграм").is_none());
    }

    #[test]
    fn the_chat_opens_in_the_application_and_only_known_links_are_converted() {
        assert_eq!(app_url("https://t.me/tima").as_deref(), Some("tg://resolve?domain=tima"));
        assert_eq!(app_url("https://t.me/+998901234567").as_deref(), Some("tg://resolve?phone=998901234567"));
        assert_eq!(app_url("https://wa.me/998901234567?text=%D0%BF").as_deref(), Some("whatsapp://send?phone=998901234567&text=%D0%BF"));
        for bad in ["https://t.me/ti ma", "https://t.me/", "https://t.me/a&b=c", "https://wa.me/12ab?text=x", "https://evil.test/", "tg://resolve?domain=x", "javascript:alert(1)"] {
            assert!(app_url(bad).is_none(), "{}", bad);
        }
    }

    #[test]
    fn the_result_of_the_automatic_send_is_written_honestly() {
        store::with_temp(|| {
            contacts::save(Contact { id: "t".into(), name: "Тима".into(), telegram: "tima".into(), ..Default::default() }).unwrap();
            let Prepared::Ready(o) = prepare(&Parsed { who: "Тима".into(), body: "привет".into(), channel: None }) else { panic!() };
            stage(&o).unwrap();
            confirm(&o.id, &LinkAdapter { channel: "telegram".into() }).unwrap();
            finish_auto(&o.id, Err("окно мессенджера не появилось".into()));
            let r = get(&o.id).unwrap();
            assert!(r.status == Status::ManualSendRequired && r.note.contains("не появилось"), "{:?} {}", r.status, r.note);
            finish_auto(&o.id, Ok(()));
            let r = get(&o.id).unwrap();
            assert!(r.status == Status::Sent && r.note.contains("не могу"), "the keystroke is not a proof of delivery: {}", r.note);
            finish_auto("no-such-id", Ok(()));
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
            assert!(!r.url.contains("секретный") && r.note.contains("вставляю текст"), "{}", r.note);
            assert_eq!(get(&o.id).unwrap().body, "секретный текст", "the text stays in the history to be pasted");
            assert!(manual_note("whatsapp").contains("с текстом"));
        });
    }

    #[test]
    fn the_history_is_filtered_by_a_word_and_by_state() {
        store::with_temp(|| {
            let mk = |id: &str, who: &str, body: &str, st: Status, at: u64| Outgoing { id: id.into(), contact_id: "c".into(), contact: who.into(), channel: "whatsapp".into(), body: body.into(), status: st, created: at, updated: at, note: String::new(), url: String::new() };
            for o in [mk("1", "Малика", "буду поздно", Status::ManualSendRequired, 1), mk("2", "Иван", "созвон в пять", Status::Cancelled, 2), mk("3", "Малика", "купи хлеб", Status::AwaitingConfirmation, 3), mk("4", "Олег", "ок", Status::Error, 4)] {
                put(&o).unwrap();
            }
            assert_eq!(history_filtered("", "", 10).len(), 4);
            assert_eq!(history_filtered("", "", 10)[0].id, "4", "newest first");
            assert_eq!(history_filtered("МАЛИКА", "", 10).len(), 2, "a contact, any case");
            assert_eq!(history_filtered("хлеб", "", 10).len(), 1, "a word of the text");
            assert_eq!(history_filtered("", "waiting", 10).iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), vec!["3"]);
            assert_eq!(history_filtered("", "failed", 10).len(), 1);
            assert_eq!(history_filtered("", "cancelled", 10).len(), 1);
            assert_eq!(history_filtered("", "done", 10).iter().map(|o| o.id.as_str()).collect::<Vec<_>>(), vec!["1"], "the one that was opened for a manual send is done; failed and cancelled are not");
            assert_eq!(history_filtered("Малика", "waiting", 10).len(), 1, "both filters together");
            assert_eq!(history_filtered("", "garbage", 10).len(), 4);
            assert_eq!(history_filtered("", "", 2).len(), 2);
        });
    }
}
