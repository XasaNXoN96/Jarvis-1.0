//! «Мои контакты»: people the user can name in a voice command («жене», «маме», «Ивану»). A contact has a name, aliases (the words
//! the user really says), and the addresses for each channel. A name that fits two contacts is never guessed: the caller asks.

use crate::{store, textnorm};
use serde::{Deserialize, Serialize};

const FILE: &str = "contacts.json";

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Contact {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    // international format, digits only: 998901234567
    #[serde(default)]
    pub phone: String,
    // @name without «@»
    #[serde(default)]
    pub telegram: String,
    // the channel used when the command does not name one: "telegram" | "whatsapp" | ""
    #[serde(default)]
    pub preferred: String,
    // when it was last changed (unix seconds): the sync takes the newer version
    #[serde(default)]
    pub updated: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct Book {
    #[serde(default)]
    contacts: Vec<Contact>,
}

#[derive(Debug, PartialEq)]
pub enum Resolved {
    One(Contact),
    Many(Vec<Contact>),
    None,
}

pub fn list() -> Vec<Contact> {
    store::read::<Book>(FILE).0.contacts
}

pub fn digits(phone: &str) -> String {
    phone.chars().filter(|c| c.is_ascii_digit()).collect()
}

pub fn valid_phone(d: &str) -> bool {
    (8..=15).contains(&d.len())
}

pub fn save(mut c: Contact) -> Result<String, String> {
    c.name = c.name.trim().to_string();
    if c.name.is_empty() {
        return Err("У контакта должно быть имя.".into());
    }
    c.phone = digits(&c.phone);
    if !c.phone.is_empty() && !valid_phone(&c.phone) {
        return Err("Номер телефона должен быть в международном формате, 8–15 цифр.".into());
    }
    c.telegram = c.telegram.trim().trim_start_matches('@').to_string();
    if !c.telegram.is_empty() && !c.telegram.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_') {
        return Err("Имя в Telegram: латинские буквы, цифры и «_».".into());
    }
    c.aliases = c.aliases.into_iter().map(|a| a.trim().to_string()).filter(|a| !a.is_empty()).collect();
    if !["", "telegram", "whatsapp"].contains(&c.preferred.as_str()) {
        c.preferred = String::new();
    }
    c.updated = store::now();
    let mut book = store::read::<Book>(FILE).0;
    if c.id.is_empty() {
        c.id = store::new_id();
    }
    let id = c.id.clone();
    match book.contacts.iter_mut().find(|x| x.id == id) {
        Some(x) => *x = c,
        None => book.contacts.push(c),
    }
    store::write(FILE, &book)?;
    Ok(id)
}

// the data comes back from a sync
pub fn replace_all(list: Vec<Contact>) -> Result<(), String> {
    store::write(FILE, &Book { contacts: list })
}

pub fn delete(id: &str) -> bool {
    let mut book = store::read::<Book>(FILE).0;
    let n = book.contacts.len();
    book.contacts.retain(|c| c.id != id);
    book.contacts.len() != n && store::write(FILE, &book).is_ok()
}

// The contacts of a .vcf file (what a phone or an e-mail program exports): the name (FN, else N), the first phone (TEL) and a Telegram name when the
// card has one (X-TELEGRAM or a t.me address in URL). Lines folded by a space, the quoted-printable and the «;type=» marks are read; nothing else of the card is kept
// (no addresses, birthdays, notes, photos). Returns the cards that were understood and how many had no name or no good number.
pub fn parse_vcf(text: &str) -> (Vec<Contact>, usize) {
    // unfold: a line that begins with a space or a tab continues the line before it
    let mut lines: Vec<String> = Vec::new();
    for raw in text.replace("\r\n", "\n").replace('\r', "\n").split('\n') {
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            lines.last_mut().unwrap().push_str(raw[1..].trim_end());
        } else {
            lines.push(raw.trim_end().to_string());
        }
    }
    let (mut out, mut skipped) = (Vec::new(), 0usize);
    let mut cur: Option<(String, String, String, String)> = None; // (fn, n, phone, telegram)
    for l in lines {
        let up = l.to_uppercase();
        if up == "BEGIN:VCARD" {
            cur = Some(Default::default());
        } else if up == "END:VCARD" {
            if let Some((f, n, ph, tg)) = cur.take() {
                let name = if !f.trim().is_empty() { f.trim().to_string() } else { n.trim().to_string() };
                let phone = digits(&ph);
                if name.is_empty() || (phone.is_empty() && tg.is_empty()) || (!phone.is_empty() && !valid_phone(&phone)) {
                    skipped += 1;
                } else {
                    out.push(Contact { name: name.chars().take(60).collect(), phone, telegram: tg, ..Default::default() });
                }
            }
        } else if let Some((key, val)) = l.split_once(':') {
            let Some(c) = cur.as_mut() else { continue };
            let key_up = key.to_uppercase();
            let base = key_up.split(';').next().unwrap_or("");
            let val = if key_up.contains("QUOTED-PRINTABLE") { qp_decode(val) } else { val.to_string() };
            match base {
                "FN" => c.0 = val.replace("\\,", ",").replace("\\;", ";"),
                "N" => c.1 = val.split(';').filter(|p| !p.trim().is_empty()).rev().collect::<Vec<_>>().join(" "),
                "TEL" if c.2.is_empty() => c.2 = if val.trim_start().starts_with('+') || val.trim_start().starts_with("00") { val.clone() } else { val },
                "X-TELEGRAM" | "X-SOCIALPROFILE" if c.3.is_empty() => c.3 = val.trim().trim_start_matches('@').to_string(),
                "URL" if c.3.is_empty() => {
                    if let Some(r) = val.trim().strip_prefix("https://t.me/").or_else(|| val.trim().strip_prefix("http://t.me/")) {
                        c.3 = r.trim_matches('/').to_string();
                    }
                }
                _ => {}
            }
        }
    }
    (out, skipped)
}

fn qp_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' && i + 2 < b.len() + 0 && b.get(i + 1).zip(b.get(i + 2)).map(|(x, y)| x.is_ascii_hexdigit() && y.is_ascii_hexdigit()).unwrap_or(false) {
            out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap_or(b'?'));
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

// adds the cards that are not in the book yet (the same phone or the same Telegram name is the same person: not doubled); each one goes through `save`
// (its own checks). Returns (added, already there, not understood)
pub fn import_vcf(text: &str) -> Result<(usize, usize, usize), String> {
    if text.len() > 3_000_000 {
        return Err("Файл слишком большой для импорта (больше 3 МБ).".into());
    }
    let (cards, skipped) = parse_vcf(text);
    if cards.is_empty() {
        return Err("В файле нет подходящих контактов (нужны имя и телефон или Telegram).".into());
    }
    let have = list();
    let (mut added, mut dup, mut bad) = (0usize, 0usize, skipped);
    let mut seen: Vec<(String, String)> = have.iter().map(|c| (c.phone.clone(), c.telegram.to_lowercase())).collect();
    for c in cards.into_iter().take(500) {
        let key = (c.phone.clone(), c.telegram.to_lowercase());
        if seen.iter().any(|(p, t)| (!key.0.is_empty() && *p == key.0) || (!key.1.is_empty() && *t == key.1)) {
            dup += 1;
            continue;
        }
        match save(c) {
            Ok(_) => {
                added += 1;
                seen.push(key);
            }
            Err(_) => bad += 1,
        }
    }
    Ok((added, dup, bad))
}

pub fn by_id(id: &str) -> Option<Contact> {
    list().into_iter().find(|c| c.id == id)
}

// the words the user says for the person: «жене», «Ивану», «ivanga»
pub fn resolve(phrase: &str) -> Resolved {
    resolve_in(&list(), phrase)
}

pub fn resolve_in(all: &[Contact], phrase: &str) -> Resolved {
    let p = textnorm::normalize(phrase);
    if p.is_empty() {
        return Resolved::None;
    }
    // 1. exact name or alias
    let exact: Vec<&Contact> = all.iter().filter(|c| std::iter::once(&c.name).chain(c.aliases.iter()).any(|n| textnorm::normalize(n) == p)).collect();
    if exact.len() == 1 {
        return Resolved::One(exact[0].clone());
    }
    if exact.len() > 1 {
        return Resolved::Many(exact.into_iter().cloned().collect());
    }
    // 2. the same word with another ending («жене» ~ «жена», «Ивану» ~ «Иван»)
    let stem: Vec<&Contact> = all.iter().filter(|c| std::iter::once(&c.name).chain(c.aliases.iter()).any(|n| textnorm::same_stem(n, &p))).collect();
    if stem.len() == 1 {
        return Resolved::One(stem[0].clone());
    }
    if stem.len() > 1 {
        return Resolved::Many(stem.into_iter().cloned().collect());
    }
    // 3. a typo or a heard-wrong word
    let names: Vec<(usize, String)> = all.iter().enumerate().flat_map(|(i, c)| std::iter::once(c.name.clone()).chain(c.aliases.clone()).map(move |n| (i, n))).collect();
    let refs: Vec<&str> = names.iter().map(|(_, n)| n.as_str()).collect();
    match textnorm::best_match(&p, &refs) {
        textnorm::Match::Exact(i) | textnorm::Match::Fuzzy(i, _) => Resolved::One(all[names[i].0].clone()),
        textnorm::Match::Ambiguous(v) => {
            let mut idx: Vec<usize> = v.into_iter().map(|i| names[i].0).collect();
            idx.dedup();
            if idx.len() == 1 { Resolved::One(all[idx[0]].clone()) } else { Resolved::Many(idx.into_iter().map(|i| all[i].clone()).collect()) }
        }
        textnorm::Match::None => Resolved::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, aliases: &[&str]) -> Contact {
        Contact { id: name.into(), name: name.into(), aliases: aliases.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    #[test]
    fn alias_with_ending() {
        let all = vec![c("Малика", &["жена", "любимая"]), c("Иван", &[])];
        assert!(matches!(resolve_in(&all, "жене"), Resolved::One(x) if x.name == "Малика"));
        assert!(matches!(resolve_in(&all, "Ивану"), Resolved::One(x) if x.name == "Иван"));
        assert_eq!(resolve_in(&all, "начальнику"), Resolved::None);
    }
    #[test]
    fn two_people_are_not_guessed() {
        let all = vec![c("Иван Петров", &["Иван"]), c("Иван Сидоров", &["Иван"])];
        assert!(matches!(resolve_in(&all, "Ивану"), Resolved::Many(v) if v.len() == 2));
    }
    #[test]
    fn save_validates() {
        store::with_temp(|| {
            assert!(save(Contact { name: "".into(), ..Default::default() }).is_err());
            assert!(save(Contact { name: "А".into(), phone: "123".into(), ..Default::default() }).is_err());
            assert!(save(Contact { name: "А".into(), telegram: "bad name".into(), ..Default::default() }).is_err());
            let id = save(Contact { name: "Аня".into(), phone: "+998 90 123-45-67".into(), telegram: "@anya".into(), ..Default::default() }).unwrap();
            let got = by_id(&id).unwrap();
            assert_eq!(got.phone, "998901234567");
            assert_eq!(got.telegram, "anya");
            assert!(delete(&id));
            assert!(list().is_empty());
        });
    }

    #[test]
    fn a_vcard_file_is_read_and_imported_without_doubles() {
        store::with_temp(|| {
            let vcf = "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Малика Ахмедова\r\nTEL;TYPE=CELL:+998 90 123-45-67\r\nTEL;TYPE=HOME:+998711112233\r\nEMAIL:m@x.uz\r\nNOTE:секрет\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:3.0\r\nN:Иванов;Иван;;;\r\nTEL:+7 (900) 111-22-33\r\nURL:https://t.me/ivan_i\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nFN:Без номера\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nFN:Короткий\r\nTEL:123\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nFN:Складной\r\n  Человек\r\nTEL:998935550011\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nFN;ENCODING=QUOTED-PRINTABLE:=D0=9F=D0=B5=D1=82=D1=80\r\nTEL:998901110000\r\nEND:VCARD\r\n";
            let (cards, skipped) = parse_vcf(vcf);
            assert_eq!(skipped, 2, "one without a number, one with a too short number");
            assert_eq!(cards.len(), 4);
            assert_eq!((cards[0].name.as_str(), cards[0].phone.as_str()), ("Малика Ахмедова", "998901234567"), "the first phone, digits only");
            assert_eq!((cards[1].name.as_str(), cards[1].phone.as_str(), cards[1].telegram.as_str()), ("Иван Иванов", "79001112233", "ivan_i"), "N is turned into «Имя Фамилия», a t.me address is the Telegram name");
            assert_eq!(cards[2].name, "Складной Человек", "a folded line is joined");
            assert_eq!(cards[3].name, "Петр", "quoted-printable");
            let (a, d, b) = import_vcf(vcf).unwrap();
            assert_eq!((a, d, b), (4, 0, 2));
            assert_eq!(list().len(), 4);
            assert!(!list().iter().any(|c| c.name.contains("секрет")), "notes are not kept");
            let (a, d, _) = import_vcf(vcf).unwrap();
            assert_eq!((a, d), (0, 4), "the second import adds nobody");
            assert!(import_vcf("это не карточки").is_err());
            assert!(import_vcf(&"x".repeat(3_100_000)).unwrap_err().contains("слишком большой"));
        });
    }
}
