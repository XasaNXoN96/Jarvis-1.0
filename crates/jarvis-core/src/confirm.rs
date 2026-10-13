// One confirmation for every dangerous command.
//
// A command whose (trusted) risk is "high" does not run on the phrase alone. Jarvis asks "Подтвердить? да / нет" and
// runs the SAME command only on an answer that
//  - comes through the SAME door as the request (a "да" from the room does not confirm a request from the phone, and
//    the other way round),
//  - comes within 30 seconds,
//  - and runs exactly the command and the words that were asked about: the confirmation is a one-time grant for
//    (command id, the phrase, the door). Another phrase ("выключи компьютер через час" after "...через минуту"), another
//    command or another door has no grant and is asked again.
//
// While such a command runs, its high operations (kill a process by force, switch the power off, empty the recycle
// bin) are allowed only inside a confirmed run (policy.rs asks `is_confirmed`).
//
// The word "да", the voice that said it and the phone that sent it are NOT proof of who the person is: the
// confirmation only protects against an accident and against a phrase that was heard or sent by mistake.

use std::cell::RefCell;
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

use crate::origin::Source;
use crate::policy::Risk;

pub const TTL: Duration = Duration::from_secs(30);

// the commands that carry out what the user is already being asked about in their own dialog (a file chosen from a list,
// the question "удалить файл X?"): their phrases are answers, not new requests
const CONTINUATIONS: &[&str] = &["assistant_session"];

#[derive(Clone, Debug)]
pub struct Pending {
    pub key: String,
    pub command_id: String,
    pub text: String,
    pub source: Source,
    pub question: String,
    created: Instant,
}

static PENDING: Lazy<Mutex<Option<Pending>>> = Lazy::new(|| Mutex::new(None));
static GRANT: Lazy<Mutex<Option<(String, Instant)>>> = Lazy::new(|| Mutex::new(None));

thread_local! {
    // the command id of the confirmed run on this thread
    static CONFIRMED: RefCell<Option<String>> = RefCell::new(None);
}

pub struct ConfirmedRun(Option<String>);

impl Drop for ConfirmedRun {
    fn drop(&mut self) {
        CONFIRMED.with(|c| *c.borrow_mut() = self.0.take());
    }
}

// from here on the command `id` of this thread runs with the confirmation of the user (or without it)
pub fn confirmed_run(id: &str, confirmed: bool) -> ConfirmedRun {
    ConfirmedRun(CONFIRMED.with(|c| c.replace(if confirmed { Some(id.to_string()) } else { None })))
}

// may a high operation of this command run now?
pub fn is_confirmed(command_id: &str) -> bool {
    CONTINUATIONS.contains(&command_id) || CONFIRMED.with(|c| c.borrow().as_deref() == Some(command_id))
}

// the words of the phrase, so that spaces, case and the punctuation at the end make no other request
fn normalized(text: &str) -> String {
    crate::guard::norm(text)
}

pub fn key_of(command_id: &str, text: &str, source: Source) -> String {
    let mut h = Sha256::new();
    h.update(command_id.as_bytes());
    h.update([0]);
    h.update(normalized(text).as_bytes());
    h.update([0]);
    h.update(source.as_str().as_bytes());
    h.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

#[derive(Debug, PartialEq, Eq)]
pub enum Gate {
    // run it; `confirmed` when the user said yes to exactly this
    Run { confirmed: bool },
    // not yet: this is the question for the user (the same command is run again after «да»)
    Ask { question: String },
}

// What to do with a command that was found for a phrase. `risk` is the risk that counts (a command whose files are not
// the ones of the build counts as low: it cannot do anything dangerous anyway).
pub fn gate(command_id: &str, risk: Risk, text: &str, source: Source) -> Gate {
    if risk != Risk::High || CONTINUATIONS.contains(&command_id) {
        return Gate::Run { confirmed: false };
    }
    let key = key_of(command_id, text, source);
    if take_grant(&key) {
        return Gate::Run { confirmed: true };
    }
    let question = format!("Это опасное действие: «{}». Подтвердить? Скажите «да» или «нет».", text.trim());
    *PENDING.lock() = Some(Pending { key, command_id: command_id.to_string(), text: text.to_string(), source, question: question.clone(), created: Instant::now() });
    Gate::Ask { question }
}

fn take_grant(key: &str) -> bool {
    let mut g = GRANT.lock();
    match g.take() {
        Some((k, at)) => k == key && at.elapsed() < TTL,
        None => false,
    }
}

// the open question (None when there is none or it is too old)
pub fn pending() -> Option<Pending> {
    let mut p = PENDING.lock();
    if let Some(x) = p.as_ref() {
        if x.created.elapsed() > TTL {
            *p = None;
            return None;
        }
    }
    p.clone()
}

pub fn pending_for(source: Source) -> bool {
    pending().map(|p| p.source == source).unwrap_or(false)
}

pub fn clear() {
    *PENDING.lock() = None;
    *GRANT.lock() = None;
}

#[derive(Debug, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Other,
}

// «да» / «нет» and nothing else: a long phrase that contains «да» is a new request, not an answer
pub fn classify(text: &str) -> Answer {
    let lower = text.to_lowercase().replace('ё', "е");
    let words: Vec<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    if words.is_empty() || words.len() > 4 {
        return Answer::Other;
    }
    let has = |list: &[&str]| words.iter().any(|w| list.contains(w));
    if has(&["нет", "не", "no", "not", "отмена", "отмени", "отменить", "стоп", "cancel", "stop"]) {
        return Answer::No;
    }
    if has(&["да", "yes", "конечно", "давай", "подтверждаю", "подтвердить", "подтверди", "выполняй", "yep", "sure"]) {
        return Answer::Yes;
    }
    Answer::Other
}

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    // there is no question, or the phrase is not an answer to it (the question is dropped): handle it as a new phrase
    NotAnAnswer,
    // an answer from another door: it does not answer this question, which stays open
    OtherDoor,
    // «да» from the right door: run this command again (it will find its grant)
    Rerun { text: String, source: Source },
    // «нет»
    Cancelled,
}

pub fn reply(text: &str, source: Source) -> Reply {
    let Some(p) = pending() else { return Reply::NotAnAnswer };
    if p.source != source {
        return Reply::OtherDoor;
    }
    match classify(text) {
        Answer::Yes => {
            *PENDING.lock() = None;
            *GRANT.lock() = Some((p.key, Instant::now()));
            Reply::Rerun { text: p.text, source: p.source }
        }
        Answer::No => {
            clear();
            Reply::Cancelled
        }
        Answer::Other => {
            *PENDING.lock() = None;
            Reply::NotAnAnswer
        }
    }
}

// tests that touch the (global) question run one at a time
#[cfg(test)]
pub(crate) static TEST_LOCK: Mutex<()> = parking_lot::const_mutex(());

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> parking_lot::MutexGuard<'static, ()> {
        let g = TEST_LOCK.lock();
        clear();
        g
    }

    fn asked(id: &str, text: &str, src: Source) -> bool {
        matches!(gate(id, Risk::High, text, src), Gate::Ask { .. })
    }

    #[test]
    fn only_a_high_command_is_asked_about() {
        let _g = fresh();
        assert_eq!(gate("assistant_info_time", Risk::Low, "сколько времени", Source::Voice), Gate::Run { confirmed: false });
        assert_eq!(gate("assistant_volume_up", Risk::Medium, "громче", Source::Phone), Gate::Run { confirmed: false });
        assert!(pending().is_none());
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Voice));
        assert!(pending().is_some());
    }

    #[test]
    fn yes_from_the_same_door_runs_the_same_command_once() {
        let _g = fresh();
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Phone));
        let r = reply("Да!", Source::Phone);
        assert_eq!(r, Reply::Rerun { text: "выключи компьютер".into(), source: Source::Phone });
        assert!(pending().is_none());
        assert_eq!(gate("assistant_power_off", Risk::High, "выключи компьютер", Source::Phone), Gate::Run { confirmed: true });
        // one time: the next request is asked about again
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Phone));
    }

    #[test]
    fn a_different_phrase_command_or_door_has_no_grant() {
        let _g = fresh();
        for (id, text, src) in [
            ("assistant_power_off", "выключи компьютер через час", Source::Phone),
            ("assistant_power_restart", "выключи компьютер", Source::Phone),
            ("assistant_power_off", "выключи компьютер", Source::Telegram),
            ("assistant_power_off", "выключи компьютер", Source::Voice),
        ] {
            clear();
            assert!(asked("assistant_power_off", "выключи компьютер", Source::Phone));
            assert!(matches!(reply("да", Source::Phone), Reply::Rerun { .. }));
            assert!(asked(id, text, src), "{} {} {:?}", id, text, src);
        }
        // spaces, case and the dot at the end are the same request
        clear();
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Phone));
        assert!(matches!(reply("да", Source::Phone), Reply::Rerun { .. }));
        assert_eq!(gate("assistant_power_off", Risk::High, "  Выключи   компьютер. ", Source::Phone), Gate::Run { confirmed: true });
    }

    #[test]
    fn yes_from_another_door_does_not_answer_and_the_question_stays() {
        let _g = fresh();
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Phone));
        for other in [Source::Voice, Source::Window, Source::Telegram, Source::Console, Source::Ipc, Source::Script, Source::Internal] {
            assert_eq!(reply("да", other), Reply::OtherDoor, "{:?}", other);
        }
        assert!(pending().is_some());
        assert!(matches!(reply("да", Source::Phone), Reply::Rerun { .. }));
    }

    #[test]
    fn no_and_any_other_phrase_drop_the_question() {
        let _g = fresh();
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Voice));
        assert_eq!(reply("нет", Source::Voice), Reply::Cancelled);
        assert!(pending().is_none());
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Voice));
        assert_eq!(reply("который час", Source::Voice), Reply::NotAnAnswer);
        assert!(pending().is_none());
        // «да» later has nothing to answer
        assert_eq!(reply("да", Source::Voice), Reply::NotAnAnswer);
        // a long phrase that has «да» in it is a new request
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Voice));
        assert_eq!(reply("да выключи же его наконец быстрее", Source::Voice), Reply::NotAnAnswer);
    }

    #[test]
    fn the_answer_words() {
        for yes in ["да", "Да.", "да!", "yes", "конечно", "давай", "да, подтверждаю", "подтверди"] {
            assert_eq!(classify(yes), Answer::Yes, "{}", yes);
        }
        for no in ["нет", "не надо", "отмена", "стоп", "no", "нет, отмени", "да нет"] {
            assert_eq!(classify(no), Answer::No, "{}", no);
        }
        for other in ["", "выключи компьютер", "ладно потом", "дата", "дальше", "один два три четыре пять да"] {
            assert_eq!(classify(other), Answer::Other, "{:?}", other);
        }
    }

    #[test]
    fn a_grant_is_only_for_a_short_time_and_a_question_too() {
        let _g = fresh();
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Voice));
        // an old question
        PENDING.lock().as_mut().unwrap().created = Instant::now() - TTL - Duration::from_secs(1);
        assert!(pending().is_none());
        assert_eq!(reply("да", Source::Voice), Reply::NotAnAnswer);
        // an old grant
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Voice));
        assert!(matches!(reply("да", Source::Voice), Reply::Rerun { .. }));
        GRANT.lock().as_mut().unwrap().1 = Instant::now() - TTL - Duration::from_secs(1);
        assert!(asked("assistant_power_off", "выключи компьютер", Source::Voice), "an old grant is not used");
    }

    #[test]
    fn a_confirmed_run_is_only_for_its_own_command_and_thread() {
        assert!(!is_confirmed("assistant_power_off"));
        {
            let _c = confirmed_run("assistant_power_off", true);
            assert!(is_confirmed("assistant_power_off"));
            assert!(!is_confirmed("assistant_power_restart"));
            std::thread::spawn(|| assert!(!is_confirmed("assistant_power_off"))).join().unwrap();
            {
                let _n = confirmed_run("assistant_power_off", false);
                assert!(!is_confirmed("assistant_power_off"));
            }
            assert!(is_confirmed("assistant_power_off"));
        }
        assert!(!is_confirmed("assistant_power_off"));
        // the session command carries out what is being asked in the dialog itself
        assert!(is_confirmed("assistant_session"));
    }

    #[test]
    fn the_key_covers_the_command_the_words_and_the_door() {
        let k = key_of("a", "выключи", Source::Phone);
        assert_eq!(k, key_of("a", "  ВЫКЛЮЧИ. ", Source::Phone));
        assert_ne!(k, key_of("b", "выключи", Source::Phone));
        assert_ne!(k, key_of("a", "выключи сейчас", Source::Phone));
        assert_ne!(k, key_of("a", "выключи", Source::Voice));
        assert_eq!(k.len(), 64);
    }
}
