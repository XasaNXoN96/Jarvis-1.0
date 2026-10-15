//! Who may do what: the one place that decides whether a command (or a paid operation of the window) may run.
//!
//! Levels: FREE (Demo), PREMIUM (needs a valid license), ADMIN (only Jarvis Admin), SYSTEM (never locked).
//! A command that is not on the list of `resources/access.toml` is PREMIUM: a new command is paid until the owner frees it.
//! The check is made BEFORE the operation runs (commands::execute_command, the scenario runner, the operations of the window),
//! not only in the interface; when anything is wrong (the policy is broken, the license state is unknown) the paid operation
//! is refused, never allowed.
//!
//! The edition is chosen at build time: `user-edition` = Jarvis User (levels are enforced); without it = Jarvis Admin (the owner:
//! every command runs, the license manager is available).

use once_cell::sync::Lazy;
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Free,
    Premium,
    Admin,
    System,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edition {
    Admin,
    User,
}

#[cfg(feature = "user-edition")]
pub const EDITION: Edition = Edition::User;
#[cfg(not(feature = "user-edition"))]
pub const EDITION: Edition = Edition::Admin;

pub fn edition_name() -> &'static str {
    match EDITION {
        Edition::Admin => "admin",
        Edition::User => "user",
    }
}

// what the license state says right now (the licensing client fills it; see licensing.rs)
#[derive(Clone, Debug, PartialEq)]
pub enum LicenseView {
    // no license (Demo)
    None,
    Premium,
    // the subscription is over
    Expired,
    // the program was not in touch with the server for too long or the clock was turned back
    NeedsRefresh,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Why {
    NeedsPremium,
    Expired,
    NeedsRefresh,
    AdminOnly,
    PolicyBroken,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Denied {
    pub why: Why,
    pub level: Level,
    pub message: String,
}

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

const POLICY_TEXT: &str = include_str!("../../../resources/access.toml");

struct Policy {
    levels: HashMap<String, Level>,
}

static POLICY: Lazy<Result<Policy, String>> = Lazy::new(|| parse_policy(POLICY_TEXT));

fn parse_policy(text: &str) -> Result<Policy, String> {
    let doc: toml::Value = toml::from_str(text).map_err(|e| format!("access.toml: {}", e))?;
    let mut levels = HashMap::new();
    for (section, level) in [("free", Level::Free), ("system", Level::System), ("admin", Level::Admin)] {
        let ids = doc.get(section).and_then(|s| s.get("ids")).and_then(|v| v.as_array()).ok_or_else(|| format!("access.toml: нет раздела [{}] ids", section))?;
        for id in ids {
            let id = id.as_str().ok_or("access.toml: id не строка")?;
            if levels.insert(id.to_string(), level).is_some() {
                return Err(format!("access.toml: команда {} указана дважды", id));
            }
        }
    }
    Ok(Policy { levels })
}

pub fn policy_ok() -> bool {
    POLICY.is_ok()
}

// the level of a command: only what the owner listed is free / system / admin; everything else, known or new, is Premium
pub fn level_of(id: &str) -> Level {
    match &*POLICY {
        Ok(p) => p.levels.get(id).copied().unwrap_or(Level::Premium),
        Err(_) => Level::Premium,
    }
}

pub fn free_ids() -> Vec<String> {
    match &*POLICY {
        Ok(p) => {
            let mut v: Vec<String> = p.levels.iter().filter(|(_, l)| **l == Level::Free).map(|(k, _)| k.clone()).collect();
            v.sort();
            v
        }
        Err(_) => Vec::new(),
    }
}

// the decision itself, without any global state (the tests and the real check use it)
pub fn decide(edition: Edition, view: &LicenseView, level: Level, policy_ok: bool) -> Result<(), Denied> {
    match (edition, level) {
        (Edition::Admin, _) => Ok(()),
        (_, Level::System) => Ok(()),
        (Edition::User, Level::Admin) => Err(Denied { why: Why::AdminOnly, level, message: "Эта функция есть только в административной версии Jarvis.".into() }),
        (Edition::User, Level::Free) if policy_ok => Ok(()),
        (Edition::User, Level::Free) | (Edition::User, Level::Premium) => {
            if !policy_ok {
                return Err(Denied { why: Why::PolicyBroken, level, message: "Не удалось прочитать правила доступа программы. Платные функции отключены: переустановите Jarvis.".into() });
            }
            match view {
                LicenseView::Premium => Ok(()),
                LicenseView::None => Err(Denied { why: Why::NeedsPremium, level, message: "Эта функция доступна в Jarvis Premium. Активируйте Premium, чтобы использовать её.".into() }),
                LicenseView::Expired => Err(Denied { why: Why::Expired, level, message: "Ваша подписка Jarvis Premium закончилась. Продлите её, чтобы пользоваться Premium.".into() }),
                LicenseView::NeedsRefresh => Err(Denied { why: Why::NeedsRefresh, level, message: "Нужно подключиться к интернету, чтобы подтвердить лицензию Jarvis Premium.".into() }),
            }
        }
    }
}

// the real check of a command / operation id
pub fn check(id: &str) -> Result<(), Denied> {
    let level = level_of(id);
    let result = decide(EDITION, &crate::licensing::view(), level, policy_ok());
    if let Err(d) = &result {
        crate::licensing::note_denied(id, d);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_embedded_policy_is_valid_and_new_commands_are_premium() {
        assert!(policy_ok(), "{:?}", POLICY.as_ref().err());
        assert_eq!(level_of("assistant_info_time"), Level::Free);
        assert_eq!(level_of("stop_1"), Level::System);
        assert_eq!(level_of("assistant_file_search"), Level::Premium);
        assert_eq!(level_of("a_command_nobody_has_heard_of"), Level::Premium, "a new command is paid by default");
        assert_eq!(level_of(""), Level::Premium);
        let n = free_ids().len();
        assert!((28..=40).contains(&n), "about 30 free commands, there are {}", n);
    }

    #[test]
    fn a_broken_or_ambiguous_policy_is_refused() {
        assert!(parse_policy("not toml {{").is_err());
        assert!(parse_policy("[free]\nids=[\"a\"]\n[system]\nids=[\"a\"]\n[admin]\nids=[]").is_err(), "one command on two lists");
        assert!(parse_policy("[free]\nids=[\"a\"]").is_err(), "a missing section");
        assert!(parse_policy("[free]\nids=[1]\n[system]\nids=[]\n[admin]\nids=[]").is_err());
    }

    #[test]
    fn the_decision_table() {
        use LicenseView::*;
        for view in [None, Premium, Expired, NeedsRefresh] {
            // system and free work in every state; the admin edition runs everything
            assert!(decide(Edition::User, &view, Level::System, true).is_ok());
            assert!(decide(Edition::User, &view, Level::Free, true).is_ok());
            assert!(decide(Edition::Admin, &view, Level::Premium, true).is_ok());
            assert!(decide(Edition::Admin, &view, Level::Admin, true).is_ok());
            // the customer's program never runs an ADMIN operation, a license does not change that
            assert_eq!(decide(Edition::User, &view, Level::Admin, true).unwrap_err().why, Why::AdminOnly);
        }
        assert!(decide(Edition::User, &Premium, Level::Premium, true).is_ok());
        assert_eq!(decide(Edition::User, &None, Level::Premium, true).unwrap_err().why, Why::NeedsPremium);
        assert_eq!(decide(Edition::User, &Expired, Level::Premium, true).unwrap_err().why, Why::Expired);
        assert_eq!(decide(Edition::User, &NeedsRefresh, Level::Premium, true).unwrap_err().why, Why::NeedsRefresh);
        // a broken policy: the paid operations are refused (even with a license), the system ones stay
        assert_eq!(decide(Edition::User, &Premium, Level::Premium, false).unwrap_err().why, Why::PolicyBroken);
        assert_eq!(decide(Edition::User, &Premium, Level::Free, false).unwrap_err().why, Why::PolicyBroken);
        assert!(decide(Edition::User, &None, Level::System, false).is_ok());
        assert!(decide(Edition::User, &None, Level::Premium, true).unwrap_err().message.contains("Jarvis Premium"));
    }

    // the inventory: every FREE and SYSTEM id is a real command of the packs; nothing dangerous is free
    #[test]
    fn the_lists_name_real_commands_and_nothing_dangerous_is_free() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let mut all: HashMap<String, (String, String)> = HashMap::new();
        for e in std::fs::read_dir(&root).unwrap().flatten() {
            let toml_file = e.path().join("command.toml");
            if let Ok(text) = std::fs::read_to_string(&toml_file) {
                let doc: toml::Value = toml::from_str(&text).unwrap();
                for c in doc.get("commands").and_then(|c| c.as_array()).into_iter().flatten() {
                    let id = c.get("id").and_then(|v| v.as_str()).unwrap().to_string();
                    let risk = c.get("risk").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    let ty = c.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string();
                    all.insert(id, (risk, ty));
                }
            }
        }
        assert!(all.len() > 100, "{}", all.len());
        let p = POLICY.as_ref().unwrap();
        for (id, level) in &p.levels {
            let (risk, ty) = all.get(id).unwrap_or_else(|| panic!("{} is on the list of access.toml but there is no such command", id));
            if *level == Level::Free {
                assert!(risk != "high", "{} is free but its risk is high", id);
                assert!(ty != "cli" && ty != "ahk", "{} is free but starts a program ({})", id, ty);
            }
        }
        // the file-, program-, document-, input- and automation-commands are all paid
        for id in ["assistant_file_search", "assistant_program_open", "assistant_office_dictate", "assistant_object_start", "assistant_session", "key_copy", "mouse_click", "windows_2", "assistant_power_off", "assistant_screen", "assistant_note_add", "assistant_task_add", "assistant_website", "browser_open"] {
            assert!(all.contains_key(id), "{} is not a command", id);
            assert_eq!(level_of(id), Level::Premium, "{}", id);
        }
        // the ones that stop things are never locked
        for id in ["stop_1", "terminate_1"] {
            assert_eq!(level_of(id), Level::System);
        }
    }

    // the whole chain with the real check: only in the customer's edition the levels are enforced; the admin edition runs everything
    fn cmd(id: &str) -> crate::commands::JCommand {
        serde_json::from_value(serde_json::json!({ "id": id, "type": "voice" })).expect("a minimal command")
    }

    #[cfg(feature = "user-edition")]
    #[test]
    fn user_edition_refuses_paid_commands_in_execute_command_before_anything_runs() {
        let _g = crate::licensing::test_lock();
        crate::licensing::test_reset();
        let path = std::path::PathBuf::from("none");
        // a new, unlisted command is paid
        let e = crate::commands::execute_command(&path, &cmd("brand_new_command"), None, None).unwrap_err();
        assert!(e.contains("Jarvis Premium"), "{}", e);
        // a free one and a system one go through (the type "voice" does nothing)
        assert!(crate::commands::execute_command(&path, &cmd("assistant_info_time"), None, None).is_ok());
        assert!(crate::commands::execute_command(&path, &cmd("stop_1"), None, None).is_ok());
        // the refusal is remembered for the window (the offer), the command is not
        assert_eq!(crate::licensing::prompt_json()["why"], "premium");
        assert_eq!(crate::licensing::prompt_json()["id"], "brand_new_command");
        // the window operations and the scenario runner are refused the same way
        assert!(check("ui_file_search").is_err());
        assert!(crate::scenario::run_step(&crate::scenario::Step { kind: "url".into(), param: "https://example.com".into(), window: String::new() }).unwrap_err().contains("Jarvis Premium"));
        // an administrator-only operation is never allowed in the customer's program
        assert!(decide(EDITION, &LicenseView::Premium, Level::Admin, true).is_err());
    }

    #[cfg(not(feature = "user-edition"))]
    #[test]
    fn admin_edition_runs_everything() {
        let path = std::path::PathBuf::from("none");
        assert!(crate::commands::execute_command(&path, &cmd("brand_new_command"), None, None).is_ok());
        assert!(check("ui_file_search").is_ok());
    }
}
