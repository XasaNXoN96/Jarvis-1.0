//! How the AI talks: the level of humor, the language of the answers and the natural manner of speech. Only the style of the answer
//! changes (a line added to the system prompt); what the program may do never depends on it. Humor is never used for serious topics
//! (health, money, safety, grief, law), at any level.

use crate::store;
use serde::{Deserialize, Serialize};

const FILE: &str = "persona.json";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Persona {
    // 0 serious · 1 a light touch · 2 moderate · 3 playful
    #[serde(default = "one")]
    pub humor: u8,
    // "auto" (the language of the user's message) | "ru" | "en" | "uz"
    #[serde(default = "auto")]
    pub reply_lang: String,
    // a living manner: short sentences, no office language, a question when the request is unclear
    #[serde(default = "yes")]
    pub natural: bool,
}
fn one() -> u8 {
    1
}
fn auto() -> String {
    "auto".into()
}
fn yes() -> bool {
    true
}
impl Default for Persona {
    fn default() -> Self {
        Persona { humor: 1, reply_lang: "auto".into(), natural: true }
    }
}

pub fn get() -> Persona {
    let mut p = store::read::<Persona>(FILE).0;
    p.humor = p.humor.min(3);
    if !["auto", "ru", "en", "uz"].contains(&p.reply_lang.as_str()) {
        p.reply_lang = "auto".into();
    }
    p
}

pub fn set(humor: u8, reply_lang: &str, natural: bool) -> Result<(), String> {
    if humor > 3 {
        return Err("Уровень юмора от 0 до 3.".into());
    }
    if !["auto", "ru", "en", "uz"].contains(&reply_lang) {
        return Err("Язык ответов: auto, ru, en или uz.".into());
    }
    store::write(FILE, &Persona { humor, reply_lang: reply_lang.into(), natural })
}

pub fn humor_name(level: u8) -> &'static str {
    match level {
        0 => "серьёзно",
        1 => "чуть-чуть",
        2 => "умеренно",
        _ => "весело",
    }
}

// the lines for the system prompt
pub fn prompt_for(p: &Persona) -> String {
    let mut parts: Vec<&str> = Vec::new();
    parts.push(match p.humor {
        0 => "Не шути: отвечай серьёзно и по делу.",
        1 => "Шути редко и лёгко, только когда это уместно и пользователь сам настроен на шутку.",
        2 => "Допускай умеренный юмор: одна короткая шутка или ироничная фраза, если она к месту.",
        _ => "Отвечай живо и с юмором, можно с игрой слов, но не в ущерб точности ответа.",
    });
    if p.humor > 0 {
        parts.push("Никогда не шути о здоровье, деньгах, безопасности, юридических вопросах, горе и о самом человеке; в таких темах отвечай серьёзно.");
    }
    parts.push(match p.reply_lang.as_str() {
        "ru" => "Всегда отвечай по-русски.",
        "en" => "Always answer in English.",
        "uz" => "Har doim o'zbek tilida javob ber.",
        _ => "Отвечай на том языке (русский, английский или узбекский), на котором написан последний вопрос пользователя.",
    });
    if p.natural {
        parts.push("Говори естественно, как живой человек: короткими фразами, без канцелярита, без перечислений, если ответ произносится голосом; если запрос неясен, задай один короткий уточняющий вопрос вместо догадок.");
    }
    parts.join(" ")
}

pub fn prompt() -> String {
    prompt_for(&get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_level_has_its_own_line_and_serious_topics_are_always_protected() {
        let lines: Vec<String> = (0..=3).map(|h| prompt_for(&Persona { humor: h, ..Default::default() })).collect();
        for i in 0..4 {
            for j in i + 1..4 {
                assert_ne!(lines[i], lines[j]);
            }
        }
        assert!(lines[0].contains("Не шути") && !lines[0].contains("Никогда не шути о здоровье"));
        for l in &lines[1..] {
            assert!(l.contains("Никогда не шути о здоровье"));
        }
    }

    #[test]
    fn language_and_manner() {
        let en = prompt_for(&Persona { reply_lang: "en".into(), natural: false, humor: 1 });
        assert!(en.contains("Always answer in English") && !en.contains("естественно"));
        assert!(prompt_for(&Persona::default()).contains("последний вопрос"));
        assert!(prompt_for(&Persona { reply_lang: "uz".into(), ..Default::default() }).contains("o'zbek"));
    }

    #[test]
    fn settings_are_kept_and_checked() {
        store::with_temp(|| {
            assert_eq!(get(), Persona::default());
            set(3, "ru", false).unwrap();
            assert_eq!(get(), Persona { humor: 3, reply_lang: "ru".into(), natural: false });
            assert!(set(4, "ru", true).is_err());
            assert!(set(1, "de", true).is_err());
            assert_eq!(get().humor, 3, "a refused change leaves the old one");
            std::fs::write(store::file(FILE), r#"{"humor": 9, "reply_lang": "xx"}"#).unwrap();
            assert_eq!(get(), Persona { humor: 3, reply_lang: "auto".into(), natural: true });
        });
    }
}
