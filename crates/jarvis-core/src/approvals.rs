// Approvals: what Jarvis proposes to do by himself waits here until the owner says yes.
//
// The idea is the approval flow of the proactive agent of OpenJarvis (https://github.com/open-jarvis/OpenJarvis,
// src/openjarvis/tools/approval_store.py and proactive_tools.py, Apache-2.0): an action is a phrase for the usual command pipeline
// with a tier (how much it matters) and a key (what kind of thing it is); the owner answers «да 3f2a», «нет 3f2a»,
// «всегда да 3f2a», «всегда нет 3f2a» or «да всё»; «всегда» is remembered for the key. Own code, own rules:
//   - nothing runs at the first ask: there is no «trivial» tier, only what the owner has already allowed with «всегда да» runs by itself;
//   - the tier is set by the CODE from the words of the phrase (a message, a call: High), never by the model that proposed it;
//   - High is never remembered: a message is asked about every time;
//   - a phrase that touches power, deleting, installing or remote control is refused outright, whoever proposed it;
//   - a decision can only be made for an id that is waiting (a stray «да» in a talk is not an answer).
// The store is a small file (approvals.json); everything here is pure and tested, the file work is at the end.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const MAX_PENDING: usize = 12;
pub const LIFETIME_SECS: u64 = 12 * 3600;
pub const MAX_PHRASE_CHARS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    // opening a program, a sound, a timer: asked once, then remembered when the owner says «всегда»
    Low,
    // everything else: asked every time unless remembered
    Medium,
    // a message, a call: asked every time, never remembered
    High,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pending,
    Approved,
    Denied,
    Expired,
    Done,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Memory {
    AlwaysApprove,
    AlwaysDeny,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    // the phrase that goes into the usual pipeline when it is approved
    pub phrase: String,
    // why it is proposed, in a few words
    pub why: String,
    pub key: String,
    pub tier: Tier,
    pub status: Status,
    pub created: u64,
    pub expires: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Store {
    pub actions: Vec<Action>,
    pub memory: BTreeMap<String, Memory>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Proposed {
    // allowed in advance (remembered): run it now
    Run,
    // refused: forbidden, remembered «нет», a repeat, or too many are waiting
    Skip,
    // waits for the owner: this id
    Queued(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Id(String),
    All,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub target: Target,
    pub approve: bool,
    pub always: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Processed {
    pub id: String,
    pub phrase: String,
    pub approved: bool,
    pub remembered: bool,
}

// ---------------------------------------------------------------------------------------------
// what may be proposed, and how much it matters
// ---------------------------------------------------------------------------------------------

// What nobody may start by himself, whoever proposed it. (The owner can still say these himself.)
const FORBIDDEN: &[&str] = &[
    "выключи компьютер", "выключить компьютер", "выключи пк", "заверши работу", "перезагруз", "перезапусти компьютер", "спящий режим", "гибернац", "выйди из системы", "заблокируй компьютер",
    "очисти корзину", "очистить корзину", "удали", "удалить", "сотри", "форматир", "затри", "shutdown", "restart", "reboot", "delete", "format ", "rm ", "del ",
    "включи удалённое", "включи удаленное", "удалённое управление", "удаленное управление", "установи", "установить", "скачай", "скачать", "оплат", "переведи деньги", "пароль", "password",
    "powershell", "cmd", "реестр", "regedit", "[[", "http://", "https://", "<", "|", ";", "&", "`", "$(",
];

pub fn is_allowed_phrase(phrase: &str) -> bool {
    let p = phrase.trim().to_lowercase();
    if p.is_empty() || p.chars().count() > MAX_PHRASE_CHARS || p.chars().any(|c| c.is_control()) {
        return false;
    }
    !FORBIDDEN.iter().any(|f| p.contains(f))
}

// the tier by the words of the phrase (the model never chooses it)
pub fn tier_of(phrase: &str) -> Tier {
    let p = phrase.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| p.contains(w));
    if has(&["напиши", "написать", "отправь", "отправить", "ответь", "ответить", "позвони", "позвонить", "сообщени", "письм", "telegram", "телеграм", "whatsapp", "ватсап", "звонок"]) {
        Tier::High
    } else if has(&["открой", "открыть", "запусти", "включи музыку", "включи", "напомни", "таймер", "поставь", "найди", "покажи", "погод"]) {
        Tier::Low
    } else {
        Tier::Medium
    }
}

// «открой блокнот» -> «run:открой блокнот»: the kind of thing, for «всегда да»; the words after the first two are the object, so the
// remembered permission is for «открой блокнот» and not for everything that starts with «открой»
pub fn key_of(phrase: &str) -> String {
    let words: Vec<String> = phrase.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_string()).collect();
    format!("run:{}", words.iter().take(3).cloned().collect::<Vec<_>>().join(" "))
}

// ---------------------------------------------------------------------------------------------
// the answers of the owner
// ---------------------------------------------------------------------------------------------

const YES: &[&str] = &["да", "yes", "ok", "окей", "ок", "одобряю", "разрешаю", "approve", "давай", "согласен"];
const NO: &[&str] = &["нет", "no", "отказ", "отклони", "отклоняю", "запрети", "запрещаю", "deny", "не"];
const ALWAYS: &[&str] = &["всегда", "always"];
const ALL: &[&str] = &["все", "всё", "всех", "all"];

fn words_of(text: &str) -> Vec<String> {
    text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_string()).collect()
}

// «да 3f2a», «{3f2a} нет», «всегда да 3f2a», «да всё», «yes all», «3f2a да, 7b1c нет»: only for the ids that wait (or «всё» while something waits)
pub fn parse_reply(text: &str, pending_ids: &[String]) -> Vec<Decision> {
    let w = words_of(text);
    let is = |i: usize, set: &[&str]| w.get(i).map(|t| set.contains(&t.as_str())).unwrap_or(false);
    let target_at = |i: usize| -> Option<Target> {
        let t = w.get(i)?;
        if ALL.contains(&t.as_str()) && !pending_ids.is_empty() {
            Some(Target::All)
        } else if pending_ids.iter().any(|p| p == t) {
            Some(Target::Id(t.clone()))
        } else {
            None
        }
    };
    let mut out: Vec<Decision> = Vec::new();
    let mut i = 0;
    while i < w.len() {
        let mut j = i;
        let always = is(j, ALWAYS);
        if always {
            j += 1;
        }
        // «да 3f2a» / «всегда да 3f2a»
        if is(j, YES) || is(j, NO) {
            let approve = is(j, YES);
            if let Some(t) = target_at(j + 1) {
                out.push(Decision { target: t, approve, always });
                i = j + 2;
                continue;
            }
        }
        // «3f2a да» / «3f2a всегда да»
        if let Some(t) = target_at(j) {
            let mut k = j + 1;
            let always2 = always || is(k, ALWAYS);
            if is(k, ALWAYS) {
                k += 1;
            }
            if is(k, YES) || is(k, NO) {
                out.push(Decision { target: t, approve: is(k, YES), always: always2 });
                i = k + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------------------------
// the store
// ---------------------------------------------------------------------------------------------

impl Store {
    pub fn pending(&self) -> Vec<&Action> {
        self.actions.iter().filter(|a| a.status == Status::Pending).collect()
    }

    pub fn pending_ids(&self) -> Vec<String> {
        self.pending().iter().map(|a| a.id.clone()).collect()
    }

    // those that waited too long
    pub fn expire(&mut self, now: u64) {
        for a in self.actions.iter_mut() {
            if a.status == Status::Pending && now >= a.expires {
                a.status = Status::Expired;
            }
        }
        // the old finished ones are forgotten (the store stays small)
        let keep_from = now.saturating_sub(7 * 24 * 3600);
        self.actions.retain(|a| a.status == Status::Pending || a.status == Status::Approved || a.created >= keep_from);
    }

    // `id_seed` makes the id (four characters); a taken one is shifted
    pub fn propose(&mut self, phrase: &str, why: &str, now: u64, id_seed: u64) -> Proposed {
        let phrase: String = phrase.split_whitespace().collect::<Vec<_>>().join(" ");
        if !is_allowed_phrase(&phrase) {
            return Proposed::Skip;
        }
        let (tier, key) = (tier_of(&phrase), key_of(&phrase));
        match self.memory.get(&key) {
            Some(Memory::AlwaysDeny) => return Proposed::Skip,
            Some(Memory::AlwaysApprove) if tier != Tier::High => return Proposed::Run,
            _ => {}
        }
        if self.actions.iter().any(|a| a.status == Status::Pending && a.phrase.to_lowercase() == phrase.to_lowercase()) {
            return Proposed::Skip;
        }
        if self.pending().len() >= MAX_PENDING {
            return Proposed::Skip;
        }
        let mut n = id_seed;
        let id = loop {
            let id = format!("{:04x}", n & 0xffff);
            if !self.actions.iter().any(|a| a.id == id) {
                break id;
            }
            n = n.wrapping_add(1);
        };
        self.actions.push(Action { id: id.clone(), phrase, why: why.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(120).collect(), key, tier, status: Status::Pending, created: now, expires: now + LIFETIME_SECS });
        Proposed::Queued(id)
    }

    pub fn decide(&mut self, decisions: &[Decision]) -> Vec<Processed> {
        let mut out = Vec::new();
        for d in decisions {
            let ids: Vec<String> = match &d.target {
                Target::All => self.pending_ids(),
                Target::Id(id) => vec![id.clone()],
            };
            for id in ids {
                let Some(a) = self.actions.iter_mut().find(|a| a.id == id && a.status == Status::Pending) else { continue };
                a.status = if d.approve { Status::Approved } else { Status::Denied };
                // High is never remembered
                let remember = d.always && a.tier != Tier::High;
                if remember {
                    self.memory.insert(a.key.clone(), if d.approve { Memory::AlwaysApprove } else { Memory::AlwaysDeny });
                }
                out.push(Processed { id: a.id.clone(), phrase: a.phrase.clone(), approved: d.approve, remembered: remember });
            }
        }
        out
    }

    // the approved ones, once: they are marked done
    pub fn take_approved(&mut self) -> Vec<Action> {
        let mut out = Vec::new();
        for a in self.actions.iter_mut().filter(|a| a.status == Status::Approved) {
            a.status = Status::Done;
            out.push(a.clone());
        }
        out
    }
}

// the text for the owner: what waits and how to answer
pub fn ask_text(pending: &[&Action], honorific: &str) -> String {
    if pending.is_empty() {
        return "Предложений нет.".to_string();
    }
    let mut s = format!("Предлагаю, {}:", honorific);
    for a in pending {
        s.push_str(&format!(" [{}] {}{}.", a.id, a.phrase, if a.why.is_empty() { String::new() } else { format!(" — {}", a.why) }));
    }
    s.push_str(" Ответьте «да» и номер, «нет» и номер, «всегда да» и номер, чтобы запомнить, или «да всё».");
    s
}

pub fn answer_text(done: &[Processed]) -> String {
    if done.is_empty() {
        return String::new();
    }
    let yes = done.iter().filter(|d| d.approved).count();
    let no = done.len() - yes;
    let mut parts = Vec::new();
    if yes > 0 {
        parts.push(format!("выполняю: {}", done.iter().filter(|d| d.approved).map(|d| d.phrase.clone()).collect::<Vec<_>>().join("; ")));
    }
    if no > 0 {
        parts.push(format!("отменено: {}", no));
    }
    let remembered = done.iter().filter(|d| d.remembered).count();
    if remembered > 0 {
        parts.push(format!("запомнил решений: {}", remembered));
    }
    format!("Принято, {}.", parts.join("; "))
}

// ---------------------------------------------------------------------------------------------
// the file
// ---------------------------------------------------------------------------------------------

pub fn load(path: &std::path::Path) -> Store {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

pub fn save(path: &std::path::Path, store: &Store) -> bool {
    match serde_json::to_string_pretty(store) {
        Ok(json) => crate::write_atomic(path, json).is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests;
