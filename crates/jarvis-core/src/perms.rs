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
    // multi-step control of the computer by a plan
    Automation,
    // the phone may look at the screen of this computer through the remote
    RemoteScreen,
    // the linked phone may send its voice (audio of the microphone of the page / application) to this computer for recognition
    RemoteMicrophone,
    // the linked phone may drive the mouse and the keyboard of this computer (a session started on purpose; off by default)
    RemoteInput,
    // the linked phone may look through the web camera of this computer (a session started on purpose, a sign on the screen; off by default)
    RemoteCamera,
}

pub const ALL: [Cap; 13] = [Cap::AiNetwork, Cap::Messenger, Cap::Memory, Cap::ScreenCapture, Cap::FileSearch, Cap::Scheduler, Cap::Install, Cap::SmartHome, Cap::Automation, Cap::RemoteScreen, Cap::RemoteMicrophone, Cap::RemoteInput, Cap::RemoteCamera];

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
            Cap::Automation => "automation",
            Cap::RemoteScreen => "remote_screen",
            Cap::RemoteMicrophone => "remote_microphone",
            Cap::RemoteInput => "remote_input",
            Cap::RemoteCamera => "remote_camera",
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
            Cap::Automation => "Управлять компьютером по плану (многошаговые задачи)",
            Cap::RemoteScreen => "Показывать экран компьютера на привязанном телефоне",
            Cap::RemoteMicrophone => "Принимать голос с привязанного телефона",
            Cap::RemoteInput => "Разрешить телефону управлять мышью и клавиатурой этого компьютера",
            Cap::RemoteCamera => "Показывать веб-камеру этого компьютера на привязанном телефоне",
        }
    }
    pub fn from_id(s: &str) -> Option<Cap> {
        ALL.iter().copied().find(|c| c.id() == s)
    }
    // one line about what the switch does and what it risks (shown under the switch)
    pub fn about(self) -> &'static str {
        match self {
            Cap::AiNetwork => "Тексты вопросов уходят выбранному ИИ-провайдеру. Выключено: ответы только по готовым командам.",
            Cap::Messenger => "Открывает чат с контактом и готовит сообщение. Отправку подтверждаете вы.",
            Cap::Memory => "Записи памяти и дневника. Выключено: ничего нового не запоминается.",
            Cap::ScreenCapture => "Снимок экрана только по вашей команде; в ИИ уходит, если включён анализ экрана.",
            Cap::FileSearch => "Поиск и открытие ваших файлов по голосу.",
            Cap::Scheduler => "Задания выполняются сами, когда придёт время, без вашего слова.",
            Cap::Install => "Скачивание и установка программ (winget). Риск: сторонний установщик.",
            Cap::SmartHome => "Включение и выключение устройств в Home Assistant.",
            Cap::Automation => "ИИ составляет план действий, вы подтверждаете «да». Риск: мышь и клавиатура.",
            Cap::RemoteScreen => "Телефон видит снимок вашего экрана. Риск: на экране могут быть пароли.",
            Cap::RemoteMicrophone => "Телефон отправляет голос на ПК для распознавания.",
            Cap::RemoteInput => "Телефон нажимает мышью и клавишами на этом ПК. Красная полоса, сеанс сам заканчивается. Риск: высокий.",
            Cap::RemoteCamera => "Телефон видит вашу веб-камеру. Красная полоса и лампочка камеры; сеанс сам заканчивается. Риск: высокий.",
        }
    }
    // «high»: gives somebody else the eyes or the hands of this computer
    pub fn risk(self) -> &'static str {
        match self {
            Cap::RemoteInput | Cap::RemoteCamera | Cap::RemoteScreen | Cap::Automation | Cap::Install => "high",
            Cap::AiNetwork | Cap::Messenger | Cap::SmartHome | Cap::Scheduler | Cap::ScreenCapture | Cap::RemoteMicrophone => "medium",
            Cap::Memory | Cap::FileSearch => "low",
        }
    }
    pub fn default_state(self) -> bool {
        self.default_on()
    }
    // on by default: what the program did before this centre existed, and the harmless ones. Installing and the smart home start OFF.
    fn default_on(self) -> bool {
        !matches!(self, Cap::Install | Cap::SmartHome | Cap::Automation | Cap::RemoteScreen | Cap::RemoteInput | Cap::RemoteCamera)
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
    store::write(FILE, &f)?;
    // every change is in the security journal (who and through which door is written by the journal itself)
    crate::audit::record("permission", c.id(), c.risk(), "allowed", true, if on { "разрешение включено" } else { "разрешение выключено" });
    Ok(())
}

pub fn json() -> serde_json::Value {
    serde_json::json!(ALL.iter().map(|c| serde_json::json!({ "id": c.id(), "title": c.title(), "on": allowed(*c), "about": c.about(), "risk": c.risk(), "default": c.default_state() })).collect::<Vec<_>>())
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

    // a switch that nothing asks is a decoration: every ability must be asked by some code outside this file (and a risky one is off by default)
    #[test]
    fn every_switch_is_asked_by_the_code_and_the_risky_ones_start_off() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut text = String::new();
        fn walk(d: &std::path::Path, out: &mut String) {
            for e in std::fs::read_dir(d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().map(|x| x == "rs").unwrap_or(false) && p.file_name().unwrap() != "perms.rs" {
                    out.push_str(&std::fs::read_to_string(&p).unwrap_or_default());
                }
            }
        }
        walk(&dir, &mut text);
        for c in ALL {
            let name = format!("Cap::{:?}", c);
            let asked = text.contains(&format!("perms::{}", name)) || text.contains(&format!("{}", name)) && (text.contains(&format!("check({})", name)) || text.contains(&format!("allowed({})", name)) || text.contains(&format!("check(crate::perms::{})", name)));
            assert!(asked, "{} is never asked by the code: the switch would be a decoration", name);
            assert!(!c.about().is_empty() && !c.title().is_empty());
            if c.risk() == "high" && !matches!(c, Cap::RemoteScreen | Cap::Automation | Cap::Install | Cap::RemoteInput | Cap::RemoteCamera) {
                panic!("{:?}", c);
            }
            if matches!(c, Cap::RemoteInput | Cap::RemoteCamera | Cap::RemoteScreen | Cap::Automation | Cap::Install) {
                assert!(!c.default_state(), "{:?} is risky and must start off", c);
            }
        }
        // ids are unique and survive a round trip
        let mut ids: Vec<&str> = ALL.iter().map(|c| c.id()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ALL.len());
        assert!(ALL.iter().all(|c| Cap::from_id(c.id()) == Some(*c)));
    }

    #[test]
    fn a_change_of_a_switch_is_written_to_the_security_journal() {
        store::with_temp(|| {
            owner::reset_for_tests();
            set(Cap::RemoteCamera, true).unwrap();
            set(Cap::RemoteCamera, false).unwrap();
            let l = crate::audit::list(20);
            let mine: Vec<_> = l.iter().filter(|e| e.kind == "permission" && e.subject == "remote_camera").collect();
            assert_eq!(mine.len(), 2, "{:?}", l.iter().map(|e| (&e.kind, &e.subject)).collect::<Vec<_>>());
            assert!(mine.iter().any(|e| e.reason.contains("включено")) && mine.iter().any(|e| e.reason.contains("выключено")));
            assert!(mine.iter().all(|e| e.risk == "high"));
            // a refused change (locked) is not written as a change
            owner::set_password("кот и три пирожка", "").unwrap();
            owner::lock();
            let before = crate::audit::list(50).iter().filter(|e| e.kind == "permission").count();
            assert!(set(Cap::RemoteCamera, true).is_err());
            assert_eq!(crate::audit::list(50).iter().filter(|e| e.kind == "permission").count(), before);
            owner::reset_for_tests();
        });
    }

    #[test]
    fn the_phone_voice_is_on_by_default_is_kept_in_the_file_and_survives_a_restart() {
        store::with_temp(|| {
            owner::reset_for_tests();
            assert!(allowed(Cap::RemoteMicrophone), "on by default: the behaviour before the ability existed");
            assert_eq!(Cap::from_id("remote_microphone"), Some(Cap::RemoteMicrophone));
            assert!(Cap::from_id("camera").is_none(), "there is no camera ability");
            set(Cap::RemoteMicrophone, false).unwrap();
            // the state is the file permissions.json of the data folder: a new start reads it again (nothing is kept in memory)
            let raw = std::fs::read_to_string(store::file(FILE)).unwrap();
            assert!(raw.contains("remote_microphone"), "{}", raw);
            assert!(!allowed(Cap::RemoteMicrophone));
            set(Cap::RemoteMicrophone, true).unwrap();
            assert!(allowed(Cap::RemoteMicrophone));
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
