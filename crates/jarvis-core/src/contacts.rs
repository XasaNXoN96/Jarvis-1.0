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

pub fn delete(id: &str) -> bool {
    let mut book = store::read::<Book>(FILE).0;
    let n = book.contacts.len();
    book.contacts.retain(|c| c.id != id);
    book.contacts.len() != n && store::write(FILE, &book).is_ok()
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
}
