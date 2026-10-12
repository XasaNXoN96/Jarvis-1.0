// The menu of the Telegram bot: buttons under the messages (inline keyboard). A button never carries a command of its
// own: it names an entry of the table below, and the entry holds the phrase that is sent to the SAME queue as the text
// typed in the window (so the buttons, the voice and the phone remote share one set of commands and one set of
// confirmations). Pure data and functions, tested without a network.
//
// callback_data (at most 64 bytes):
//   m:<page>   open a page of the menu
//   r:<key>    press an entry (a dangerous one first asks "Да / Отмена")
//   y:<key>    "Да" under such a question (valid for one minute, once)
//   a:<name>   an action of the bot itself: status, tasks, screen
//   td:<n>:<h> "done" for the task number n whose text has the short hash h
//   x          "Отмена"

use serde_json::{json, Value};

pub struct Item {
    pub key: &'static str,
    pub label: &'static str,
    // what is sent to Jarvis as if it was typed
    pub phrase: &'static str,
    // a question with "Да / Отмена" before it is run
    pub ask: Option<&'static str>,
    // after the phrase, "да" is sent too: the "Да" button was the user's confirmation, Jarvis asks once more as it does for every
    // dangerous command and the button answers it (a delay of a minute stays, "отменить" is offered under the answer)
    pub then_yes: bool,
}

const fn item(key: &'static str, label: &'static str, phrase: &'static str) -> Item {
    Item { key, label, phrase, ask: None, then_yes: false }
}

pub const ITEMS: &[Item] = &[
    item("t1", "1 мин", "поставь таймер на 1 минуту"),
    item("t5", "5 мин", "поставь таймер на 5 минут"),
    item("t10", "10 мин", "поставь таймер на 10 минут"),
    item("t30", "30 мин", "поставь таймер на 30 минут"),
    item("tleft", "⏳ Сколько осталось", "сколько осталось"),
    item("tstop", "✖ Отменить таймер", "отмени таймер"),
    item("mplay", "⏯ Пауза / играть", "пауза"),
    item("mnext", "⏭ Следующий", "следующий трек"),
    item("mprev", "⏮ Предыдущий", "предыдущий трек"),
    item("vup", "🔊 Громче", "громче"),
    item("vdown", "🔉 Тише", "тише"),
    item("vmute", "🔇 Выключить звук", "выключи звук"),
    item("vunmute", "🔈 Включить звук", "включи звук"),
    item("bup", "☀ Ярче", "ярче"),
    item("bdown", "🌙 Темнее", "темнее"),
    item("wmin", "🗕 Свернуть все окна", "сверни все окна"),
    item("wlock", "🔒 Заблокировать", "заблокируй компьютер"),
    item("wtask", "📈 Диспетчер задач", "диспетчер задач"),
    item("wlang", "⌨ Сменить раскладку", "смени раскладку"),
    item("time", "🕒 Который час", "сколько времени"),
    item("taskread", "🔈 Озвучить задачи на ПК", "озвучь задачи"),
    Item { key: "pcsleep", label: "😴 Спящий режим", phrase: "спящий режим", ask: Some("Перевести компьютер в спящий режим? Удалённо его потом не разбудить."), then_yes: false },
    Item { key: "pwoff", label: "⏻ Выключить компьютер", phrase: "выключи компьютер", ask: Some("Выключить компьютер? Он выключится через минуту, это можно отменить."), then_yes: true },
    Item { key: "pwrestart", label: "🔄 Перезагрузить", phrase: "перезагрузи компьютер", ask: Some("Перезагрузить компьютер? Он перезагрузится через минуту, это можно отменить."), then_yes: true },
    Item { key: "pwhib", label: "🛌 Гибернация", phrase: "гибернация", ask: Some("Отправить компьютер в гибернацию сейчас? Удалённо его потом не разбудить."), then_yes: true },
    Item { key: "pwlogoff", label: "🚪 Выйти из системы", phrase: "выйди из системы", ask: Some("Выйти из учётной записи Windows? Несохранённые данные могут пропасть."), then_yes: true },
    item("pwcancel", "↩ Отменить выключение", "отмени выключение"),
];

pub fn find(key: &str) -> Option<&'static Item> {
    ITEMS.iter().find(|i| i.key == key)
}

pub struct Btn {
    pub label: String,
    pub data: String,
}

fn b(label: &str, data: String) -> Btn {
    Btn { label: label.to_string(), data }
}

fn run(key: &str) -> Btn {
    let it = find(key).expect("a menu entry");
    b(it.label, format!("r:{}", key))
}

fn open(label: &str, page: &str) -> Btn {
    b(label, format!("m:{}", page))
}

pub fn keyboard(rows: Vec<Vec<Btn>>) -> Value {
    json!({ "inline_keyboard": rows.iter().map(|r| r.iter().map(|x| json!({ "text": x.label, "callback_data": x.data })).collect::<Vec<_>>()).collect::<Vec<_>>() })
}

pub fn back() -> Vec<Btn> {
    vec![open("◀ Меню", "main")]
}

// one button under every answer: a way back to the menu
pub fn menu_button() -> Value {
    keyboard(vec![back_row()])
}

fn back_row() -> Vec<Btn> {
    vec![open("☰ Меню", "main")]
}

// the keyboard under the answer of a power command: a way to cancel it
pub fn after_power() -> Value {
    keyboard(vec![vec![run("pwcancel")], back_row()])
}

// the text and the keyboard of a page of the menu
pub fn page(name: &str, allow_screen: bool) -> Option<(String, Value)> {
    let (text, rows): (&str, Vec<Vec<Btn>>) = match name {
        "main" => {
            let mut rows = vec![
                vec![open("⏱ Таймер", "timer"), open("🎵 Медиа", "media")],
                vec![open("🔊 Звук и яркость", "sound"), open("🪟 Окна", "windows")],
                vec![b("📋 Задачи", "a:tasks".into()), run("time")],
                vec![open("⚡ Питание ПК", "power"), b("📊 Статус", "a:status".into())],
            ];
            if allow_screen {
                rows.push(vec![b("🖼 Снимок экрана", "a:screen".into())]);
            }
            ("JARVIS — выберите действие. Любую команду можно и написать, и сказать голосовым сообщением.", rows)
        }
        "timer" => ("⏱ Таймер", vec![vec![run("t1"), run("t5")], vec![run("t10"), run("t30")], vec![run("tleft"), run("tstop")], back()]),
        "media" => ("🎵 Медиа", vec![vec![run("mprev"), run("mplay"), run("mnext")], back()]),
        "sound" => ("🔊 Звук и яркость", vec![vec![run("vdown"), run("vup")], vec![run("vmute"), run("vunmute")], vec![run("bdown"), run("bup")], back()]),
        "windows" => ("🪟 Окна и ПК", vec![vec![run("wmin"), run("wlock")], vec![run("wtask"), run("wlang")], back()]),
        "power" => ("⚡ Питание ПК — каждое действие спросит подтверждение", vec![vec![run("pwoff"), run("pwrestart")], vec![run("pwhib"), run("pwlogoff")], vec![run("pcsleep"), run("pwcancel")], back()]),
        _ => return None,
    };
    Some((text.to_string(), keyboard(rows)))
}

// "Да / Отмена" under the question of a dangerous entry
pub fn confirm_keyboard(key: &str) -> Value {
    keyboard(vec![vec![b("✅ Да, выполнить", format!("y:{}", key)), b("✖ Отмена", "x".into())]])
}

// a short hash of a task text: a button of an old list never marks another task done
pub fn task_hash(text: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut h);
    format!("{:06x}", h.finish() & 0xff_ffff)
}

// the tasks as a message with a "done" button for the first ten
pub fn tasks_message(list: &[String]) -> (String, Value) {
    if list.is_empty() {
        return ("📋 Задач нет. Чтобы добавить, напишите: добавь задачу купить хлеб".into(), menu_button());
    }
    let mut text = format!("📋 Задачи ({}):", list.len());
    for (i, t) in list.iter().enumerate() {
        text.push_str(&format!("\n{}. {}", i + 1, t));
    }
    text.push_str("\n\nДобавить: «добавь задачу …». Кнопка ✅ отмечает задачу выполненной.");
    let mut rows: Vec<Vec<Btn>> = Vec::new();
    let mut row: Vec<Btn> = Vec::new();
    for (i, t) in list.iter().enumerate().take(10) {
        row.push(b(&format!("✅ {}", i + 1), format!("td:{}:{}", i, task_hash(t))));
        if row.len() == 5 {
            rows.push(std::mem::take(&mut row));
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows.push(back_row());
    (text, keyboard(rows))
}

#[derive(Debug, PartialEq)]
pub enum Press {
    Page(String),
    Run(String),
    Yes(String),
    Action(String),
    TaskDone(usize, String),
    Cancel,
    Unknown,
}

pub fn parse(data: &str) -> Press {
    if data == "x" {
        return Press::Cancel;
    }
    let Some((kind, rest)) = data.split_once(':') else { return Press::Unknown };
    match kind {
        "m" => Press::Page(rest.to_string()),
        "r" if find(rest).is_some() => Press::Run(rest.to_string()),
        "y" if find(rest).is_some() => Press::Yes(rest.to_string()),
        "a" if matches!(rest, "status" | "tasks" | "screen") => Press::Action(rest.to_string()),
        "td" => match rest.split_once(':') {
            Some((n, h)) if h.len() == 6 && h.chars().all(|c| c.is_ascii_hexdigit()) => n.parse().map(|n| Press::TaskDone(n, h.to_string())).unwrap_or(Press::Unknown),
            _ => Press::Unknown,
        },
        _ => Press::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_data(kb: &Value) -> Vec<String> {
        kb["inline_keyboard"].as_array().unwrap().iter().flat_map(|r| r.as_array().unwrap().iter().map(|b| b["callback_data"].as_str().unwrap().to_string())).collect()
    }

    #[test]
    fn every_button_is_understood_and_fits_telegram() {
        for p in ["main", "timer", "media", "sound", "windows", "power"] {
            let (text, kb) = page(p, true).unwrap();
            assert!(!text.is_empty());
            for d in all_data(&kb) {
                assert!(d.len() <= 64, "{}", d);
                assert_ne!(parse(&d), Press::Unknown, "{} on page {}", d, p);
            }
        }
        assert!(page("nope", true).is_none());
        let (_, kb) = tasks_message(&["а".into(), "б".into()]);
        for d in all_data(&kb) {
            assert_ne!(parse(&d), Press::Unknown, "{}", d);
        }
    }

    #[test]
    fn the_screen_button_is_only_there_when_the_user_allowed_it() {
        assert!(!all_data(&page("main", false).unwrap().1).contains(&"a:screen".to_string()));
        assert!(all_data(&page("main", true).unwrap().1).contains(&"a:screen".to_string()));
    }

    #[test]
    fn every_dangerous_entry_asks_first_and_nothing_else_does() {
        for it in ITEMS {
            let dangerous = it.phrase.contains("выключи компьютер") || it.phrase.contains("перезагрузи") || it.phrase.contains("гибернац") || it.phrase.contains("выйди из системы") || it.phrase.contains("спящий");
            assert_eq!(it.ask.is_some(), dangerous, "{}", it.key);
            if it.then_yes {
                assert!(it.ask.is_some(), "{}: the answer «да» is only sent after the user's own confirmation", it.key);
            }
        }
        // the power page offers nothing that runs without a question except the cancel
        for d in all_data(&page("power", false).unwrap().1) {
            if let Press::Run(k) = parse(&d) {
                assert!(find(&k).unwrap().ask.is_some() || k == "pwcancel", "{}", k);
            }
        }
    }

    #[test]
    fn keys_are_unique_and_a_foreign_callback_is_unknown() {
        let mut keys: Vec<&str> = ITEMS.iter().map(|i| i.key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), ITEMS.len());
        for bad in ["", "r:", "r:format c:", "y:../x", "a:shell", "m", "td:x:zzzzzz", "td:1:12", "run:pwoff", "r:pwoff;rm"] {
            assert!(matches!(parse(bad), Press::Unknown | Press::Page(_)), "{}", bad);
        }
        assert_eq!(parse("x"), Press::Cancel);
        assert_eq!(parse("r:pwoff"), Press::Run("pwoff".into()));
        assert_eq!(parse("y:pwoff"), Press::Yes("pwoff".into()));
        assert_eq!(parse("td:3:0a1b2c"), Press::TaskDone(3, "0a1b2c".into()));
    }

    #[test]
    fn tasks_are_listed_with_done_buttons_and_the_hash_follows_the_text() {
        let (text, kb) = tasks_message(&["купить хлеб".into(), "позвонить".into()]);
        assert!(text.contains("1. купить хлеб") && text.contains("2. позвонить"));
        assert!(all_data(&kb).contains(&format!("td:0:{}", task_hash("купить хлеб"))));
        assert_ne!(task_hash("а"), task_hash("б"));
        assert!(tasks_message(&[]).0.contains("Задач нет"));
        let many: Vec<String> = (0..30).map(|i| format!("задача {}", i)).collect();
        assert_eq!(all_data(&tasks_message(&many).1).iter().filter(|d| d.starts_with("td:")).count(), 10, "buttons for the first ten");
    }
}
