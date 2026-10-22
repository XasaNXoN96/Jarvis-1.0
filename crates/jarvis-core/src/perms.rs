//! The permissions centre: what Jarvis may do on its own, one switch per ability. The handlers ask `allowed(Cap)` before they act, so
//! a switch that is off really stops the ability (it is not a decoration). Switching one on or off needs the owner password when one is set.

use crate::{owner, store};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const FILE: &str = "permissions.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Cap {
    // text of the user goes to the AI provider (translation, answers)
    AiNetwork,
    // messages to contacts
    Messenger,
    // memory and diary: writing
    Memory,
    // the screen is captured on the explicit command
    ScreenCapture,
    // files are searched and opened
    FileSearch,
    // tasks that run by themselves on a schedule
    Scheduler,
    // programs are downloaded / installed
    Install,
    // smart home devices
    SmartHome,
}

pub const ALL: [Cap; 8] = [Cap::AiNetwork, Cap::Messenger, Cap::Memory, Cap::ScreenCapture, Cap::FileSearch, Cap::Scheduler, Cap::Install, Cap::SmartHome];

impl Cap {
    pub fn id(self) -> &'static str {
        match self {
            Cap::AiNetwork => "ai_network",
            Cap::Messenger => "messenger",
            Cap::Memory => "memory",
            Cap::ScreenCapture => "screen_capture",
            Cap::FileSearch => "file_search",
            Cap::Scheduler => "scheduler",
            Cap::Install => "install",
            Cap::SmartHome => "smart_home",
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Cap::AiNetwork => "Отправлять текст ИИ-провайдеру",
            Cap::Messenger => "Писать сообщения контактам",
            Cap::Memory => "Запоминать и вести дневник",
            Cap::ScreenCapture => "Снимать экран по команде",
            Cap::FileSearch => "Искать и открывать файлы",
            Cap::Scheduler => "Выполнять задания по расписанию",
            Cap::Install => "Скачивать и ставить программы",
            Cap::SmartHome => "Управлять умным домом",
        }
    }
    pub fn from_id(s: &str) -> Option<Cap> {
        ALL.iter().copied().find(|c| c.id() == s)
    }
    // on by default: what the program did before this centre existed, and the harmless ones. Installing and the smart home start OFF.
    fn default_on(self) -> bool {
        !matches!(self, Cap::Install | Cap::SmartHome)
    }
}

#[derive(Serialize, Deserialize, Default)]
struct File {
    #[serde(default)]
    off: BTreeMap<String, bool>,
    #[serde(default)]
    on: BTreeMap<String, bool>,
}

pub fn allowed(c: Cap) -> bool {
    let f = store::read::<File>(FILE).0;
    if f.on.get(c.id()).copied().unwrap_or(false) {
        return true;
    }
    if f.off.get(c.id()).copied().unwrap_or(false) {
        return false;
    }
    c.default_on()
}

// the message for the user when an ability is off
pub fn denied(c: Cap) -> String {
    format!("Это выключено в центре разрешений: «{}». Включите в Настройки → Безопасность и данные.", c.title())
}

pub fn check(c: Cap) -> Result<(), String> {
    if allowed(c) { Ok(()) } else { Err(denied(c)) }
}

pub fn set(c: Cap, on: bool) -> Result<(), String> {
    owner::guard()?;
    let mut f = store::read::<File>(FILE).0;
    f.on.remove(c.id());
    f.off.remove(c.id());
    if on {
        f.on.insert(c.id().into(), true);
    } else {
        f.off.insert(c.id().into(), true);
    }
    store::write(FILE, &f)
}

pub fn json() -> serde_json::Value {
    serde_json::json!(ALL.iter().map(|c| serde_json::json!({ "id": c.id(), "title": c.title(), "on": allowed(*c) })).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_switches() {
        store::with_temp(|| {
            owner::reset_for_tests();
            assert!(allowed(Cap::Messenger));
            assert!(!allowed(Cap::Install));
            set(Cap::Messenger, false).unwrap();
            assert!(!allowed(Cap::Messenger));
            assert!(check(Cap::Messenger).unwrap_err().contains("центре разрешений"));
            set(Cap::Install, true).unwrap();
            assert!(allowed(Cap::Install));
            set(Cap::Messenger, true).unwrap();
            assert!(allowed(Cap::Messenger));
            assert_eq!(Cap::from_id("memory"), Some(Cap::Memory));
            assert_eq!(json().as_array().unwrap().len(), ALL.len());
        });
    }

    #[test]
    fn changing_a_switch_needs_the_owner_when_a_password_is_set() {
        store::with_temp(|| {
            owner::reset_for_tests();
            owner::set_password("пароль-владельца", "").unwrap();
            owner::lock();
            assert_eq!(set(Cap::Memory, false).unwrap_err(), owner::DEFAULT_MESSAGE);
            assert!(allowed(Cap::Memory));
            owner::unlock("пароль-владельца").unwrap();
            set(Cap::Memory, false).unwrap();
            assert!(!allowed(Cap::Memory));
        });
    }
}
