//! A short journal of what happened to the microphone and the wake-word loop (device opened/closed/lost, reopening tries,
//! silence, the microphone switched off), with timestamps. It answers «why did Jarvis not hear me»: it records only states and
//! errors of the audio path, never the sound and never the recognised text.

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

const KEEP: usize = 60;

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Event {
    pub at: u64,
    // "device" | "watch" | "recognizer" | "stream"
    pub kind: String,
    pub text: String,
    pub op: u64,
}

static EVENTS: Mutex<Vec<Event>> = parking_lot::const_mutex(Vec::new());

pub fn note(kind: &str, text: &str) {
    note_at(crate::store::now(), kind, text);
}

fn note_at(at: u64, kind: &str, text: &str) {
    let mut e = EVENTS.lock();
    // the same state repeated is not a new event
    if e.last().map(|l| l.kind == kind && l.text == text).unwrap_or(false) {
        return;
    }
    e.push(Event { at, kind: kind.into(), text: text.chars().take(160).collect(), op: crate::origin::op() });
    if e.len() > KEEP {
        let n = e.len() - KEEP;
        e.drain(..n);
    }
}

pub fn recent(n: usize) -> Vec<Event> {
    let e = EVENTS.lock();
    e.iter().rev().take(n).cloned().collect()
}

pub fn clear() {
    EVENTS.lock().clear();
}

/// The one-sentence answer to «why was the wake word missed», from the state of the stream and the last events.
pub fn explain(watch: &Value, last: Option<&Event>) -> String {
    let base = match watch["state"].as_str().unwrap_or("") {
        "paused" => "Микрофон выключен: слово «Джарвис» не слушается.",
        "silent" => "Микрофон открыт, но звука нет совсем (тишина): проверьте уровень и отключение в Windows.",
        "stalled" if watch["gave_up"].as_bool().unwrap_or(false) => "Звук с микрофона не приходит, повторные открытия не помогли: проверьте, что он подключён и не занят другой программой.",
        "stalled" => "Звук с микрофона перестал приходить, Джарвис переоткрывает его.",
        _ => "Звук с микрофона идёт: если слово не сработало, дело в распознавании (громкость, шум, другое произношение имени).",
    };
    match last {
        Some(e) => format!("{} Последнее событие: {} ({}).", base, e.text, e.kind),
        None => base.to_string(),
    }
}

pub fn json_report() -> Value {
    let w = crate::wakewatch::status_json();
    let ev = recent(20);
    json!({ "explain": explain(&w, ev.first()), "watch": w, "events": ev })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journal_is_short_and_newest_first_and_skips_repeats() {
        clear();
        for i in 0..100 {
            note_at(i, "device", &format!("e{}", i));
        }
        note_at(200, "device", "e99");
        assert_eq!(EVENTS.lock().len(), KEEP);
        let r = recent(2);
        assert_eq!(r[0].text, "e99");
        assert_eq!(r[1].text, "e98");
        clear();
    }

    #[test]
    fn the_explanation_names_the_real_cause() {
        let e = Event { at: 1, kind: "device".into(), text: "микрофон не открылся".into(), op: 0 };
        assert!(explain(&json!({"state":"paused"}), None).contains("выключен"));
        assert!(explain(&json!({"state":"silent"}), None).contains("тишина"));
        assert!(explain(&json!({"state":"stalled","gave_up":true}), Some(&e)).contains("не помогли"));
        assert!(explain(&json!({"state":"stalled","gave_up":false}), None).contains("переоткрывает"));
        let ok = explain(&json!({"state":"ok"}), Some(&e));
        assert!(ok.contains("распознавании") && ok.contains("микрофон не открылся"));
    }
}
