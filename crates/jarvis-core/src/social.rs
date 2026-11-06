// Social networks among the notifications of other programs (notify.rs reads them from Windows, only with the
// permission of the user and of Windows). Nothing logs into any account: a message is seen only because the
// program of the network (or its site open in a browser) showed a Windows notification about it.
//
// - which network a notification is from: the program (Telegram Desktop, WhatsApp, Discord…) or, for a browser
//   notification, the site the browser names in it (web.whatsapp.com, instagram.com, vk.com…);
// - the list of the page "Сообщения": in memory only, never written to disk, gone when Jarvis closes.

use std::collections::VecDeque;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

pub struct Network {
    pub key: &'static str,
    pub title: &'static str,
    // names of the program (whole words of the name Windows reports, or the start of a part of its id)
    pub apps: &'static [&'static str],
    // sites: a browser notification that names one of them is from this network
    pub sites: &'static [&'static str],
}

pub const NETWORKS: &[Network] = &[
    Network { key: "telegram", title: "Telegram", apps: &["telegram"], sites: &["web.telegram.org"] },
    Network { key: "whatsapp", title: "WhatsApp", apps: &["whatsapp"], sites: &["web.whatsapp.com", "whatsapp.com"] },
    Network { key: "instagram", title: "Instagram", apps: &["instagram"], sites: &["instagram.com"] },
    Network { key: "vk", title: "ВКонтакте", apps: &["vk", "vk messenger", "вконтакте"], sites: &["vk.com", "vk.ru"] },
    Network { key: "discord", title: "Discord", apps: &["discord"], sites: &["discord.com"] },
    Network { key: "viber", title: "Viber", apps: &["viber"], sites: &[] },
    Network { key: "messenger", title: "Messenger", apps: &["messenger", "facebook"], sites: &["messenger.com", "facebook.com"] },
    Network { key: "skype", title: "Skype", apps: &["skype"], sites: &["web.skype.com", "skype.com"] },
    Network { key: "signal", title: "Signal", apps: &["signal"], sites: &[] },
    Network { key: "ok", title: "Одноклассники", apps: &["одноклассники"], sites: &["ok.ru"] },
    Network { key: "x", title: "X (Twitter)", apps: &["twitter"], sites: &["x.com", "twitter.com"] },
];

pub fn network(key: &str) -> Option<&'static Network> {
    NETWORKS.iter().find(|n| n.key == key)
}

const BROWSERS: &[&str] = &["google chrome", "chrome", "microsoft edge", "msedge", "edge", "firefox", "opera", "yandex", "яндекс браузер", "brave", "vivaldi"];

pub fn is_browser(app: &str, aumid: &str) -> bool {
    BROWSERS.iter().any(|b| crate::notify::app_matches(b, app, aumid))
}

// "web.whatsapp.com" is named in "web.whatsapp.com", "https://web.whatsapp.com/", "via web.whatsapp.com";
// "x.com" is not found inside "box.com" (the site is a whole host name or its end after a dot)
fn names_site(text: &str, site: &str) -> bool {
    let t = text.to_lowercase();
    t.split(|c: char| c.is_whitespace() || c == '/' || c == '(' || c == ')' || c == ',')
        .map(|w| w.trim_start_matches("https:").trim_start_matches("http:").trim_matches(|c: char| c == '.' || c == ':' || c == '«' || c == '»'))
        .any(|host| host == site || host.ends_with(&format!(".{}", site)))
}

// a line of a browser notification that only names the site ("web.whatsapp.com", "via web.whatsapp.com",
// "https://web.whatsapp.com/"): the browser's own line. A site mentioned inside a message does not count, so a page
// cannot make its notification look like a message of another network.
fn site_line(text: &str, site: &str) -> bool {
    let words = text.split_whitespace().count();
    words >= 1 && words <= 2 && names_site(text, site)
}

// the network of a notification: by its program, or, for a browser, by the site line of its texts.
// A generic word ("messenger", "facebook") loses to a network named in the same program name ("Signal Messenger",
// "VK Messenger" are Signal and ВКонтакте)
pub fn detect(app: &str, aumid: &str, texts: &[String]) -> Option<&'static Network> {
    if !is_browser(app, aumid) {
        let found: Vec<&'static Network> = NETWORKS.iter().filter(|n| n.apps.iter().any(|a| crate::notify::app_matches(a, app, aumid))).collect();
        return found.iter().find(|n| n.key != "messenger").or(found.first()).copied();
    }
    NETWORKS.iter().find(|n| n.sites.iter().any(|s| texts.iter().any(|t| site_line(t, s))))
}

// the texts of a browser notification without the line that only names the site
pub fn without_site_lines(net: &Network, texts: &[String]) -> Vec<String> {
    texts
        .iter()
        .filter(|t| {
            !net.sites.iter().any(|s| site_line(t, s))
        })
        .cloned()
        .collect()
}

// ---------------------------------------------------------------------------------------------
// the page "Сообщения": the last notifications of the allowed programs, in memory only
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Serialize, Debug, PartialEq)]
pub struct Message {
    pub id: u64,
    pub at: u64,
    // "" for a program that is not one of the networks
    pub net: String,
    pub app: String,
    pub title: String,
    pub text: String,
}

const KEEP: usize = 200;
static LIST: Lazy<Mutex<VecDeque<Message>>> = Lazy::new(|| Mutex::new(VecDeque::new()));
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn push(net: &str, app: &str, title: &str, text: &str) -> u64 {
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let mut l = LIST.lock();
    l.push_front(Message { id, at: now(), net: net.to_string(), app: app.chars().take(80).collect(), title: title.chars().take(200).collect(), text: text.chars().take(1000).collect() });
    l.truncate(KEEP);
    id
}

pub fn last_id() -> u64 {
    LIST.lock().front().map(|m| m.id).unwrap_or(0)
}

pub fn get(id: u64) -> Option<Message> {
    LIST.lock().iter().find(|m| m.id == id).cloned()
}

pub fn clear() -> usize {
    let mut l = LIST.lock();
    let n = l.len();
    l.clear();
    n
}

pub fn json() -> Value {
    let l = LIST.lock();
    let mut counts = serde_json::Map::new();
    for m in l.iter() {
        let k = if m.net.is_empty() { "other" } else { m.net.as_str() };
        let c = counts.get(k).and_then(|v| v.as_u64()).unwrap_or(0) + 1;
        counts.insert(k.to_string(), json!(c));
    }
    json!({
        "messages": l.iter().collect::<Vec<_>>(),
        "counts": counts,
        "networks": NETWORKS.iter().map(|n| json!({ "key": n.key, "title": n.title })).collect::<Vec<_>>(),
        "last_id": l.front().map(|m| m.id).unwrap_or(0),
    })
}

// ---------------------------------------------------------------------------------------------
// A click on a message opens the program it came from (the page names only the number of the message; Jarvis keeps the rest).
// ---------------------------------------------------------------------------------------------

/// What to open for a message: a program of the list of programs, or (for a message of a browser notification about a network) the site of the network.
#[derive(Debug, PartialEq)]
pub enum OpenTarget {
    Program(String),
    Site(String),
}

fn words_of(t: &str) -> String {
    t.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_lowercase()).collect::<Vec<_>>().join(" ")
}

/// The best program of the list for a name that Windows reported («Telegram» -> «Telegram Desktop»): the same name first, then a name that begins with it,
/// then a name that has it as whole words; of several the shortest name wins. `programs` are (name, launch) of the programs that can be started.
pub fn pick_program(app: &str, programs: &[(String, String)]) -> Option<String> {
    let want = words_of(app);
    if want.is_empty() {
        return None;
    }
    let mut best: Option<(u8, usize, &String)> = None;
    for (name, launch) in programs {
        let n = words_of(name);
        let rank = if n == want {
            0
        } else if n.starts_with(&format!("{} ", want)) {
            1
        } else if format!(" {} ", n).contains(&format!(" {} ", want)) {
            2
        } else {
            continue;
        };
        let cand = (rank, n.len(), launch);
        if best.map(|b| (cand.0, cand.1) < (b.0, b.1)).unwrap_or(true) {
            best = Some(cand);
        }
    }
    best.map(|b| b.2.clone())
}

/// What a message opens. A browser notification about a network opens the site of the network; any other message opens its program.
pub fn open_target(m: &Message, programs: &[(String, String)]) -> Result<OpenTarget, String> {
    if is_browser(&m.app, "") {
        if let Some(site) = network(&m.net).and_then(|n| n.sites.first()) {
            return Ok(OpenTarget::Site(format!("https://{}", site)));
        }
        return Err("это сообщение пришло из браузера: откройте его вкладку сами".into());
    }
    match pick_program(&m.app, programs) {
        Some(launch) => Ok(OpenTarget::Program(launch)),
        None => Err(format!("«{}» нет в списке программ: обновите список на странице «Программы»", m.app)),
    }
}

#[cfg(test)]
pub fn clear_for_tests() {
    LIST.lock().clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_programs_of_the_networks_are_recognized_by_name_or_id() {
        assert_eq!(detect("Telegram Desktop", "TelegramMessengerLLP.TelegramDesktop_t4vj0pshhgkwm!Telegram.TelegramDesktop.Store", &[]).unwrap().key, "telegram");
        assert_eq!(detect("", "5319275A.WhatsAppDesktop_cv1g1gvanyjgm!App", &[]).unwrap().key, "whatsapp");
        assert_eq!(detect("Discord", "com.squirrel.Discord.Discord", &[]).unwrap().key, "discord");
        assert_eq!(detect("VK Messenger", "", &[]).unwrap().key, "vk");
        assert_eq!(detect("Viber", "", &[]).unwrap().key, "viber");
        assert!(detect("Почта", "microsoft.windowscommunicationsapps", &[]).is_none());
        assert!(detect("Signalr Tool", "", &[]).is_none(), "a whole word, not the start of another word");
        assert_eq!(detect("Signal Messenger", "", &[]).unwrap().key, "signal", "the named network wins over the word messenger");
        assert_eq!(detect("Messenger", "", &[]).unwrap().key, "messenger");
        assert_eq!(detect("Facebook", "", &[]).unwrap().key, "messenger");
    }

    #[test]
    fn a_browser_notification_belongs_to_the_site_it_names() {
        assert_eq!(detect("Google Chrome", "Chrome", &s(&["Иван", "привет, как дела?", "web.whatsapp.com"])).unwrap().key, "whatsapp");
        assert_eq!(detect("Microsoft Edge", "MSEdge", &s(&["Новое сообщение", "через https://www.instagram.com/"])).unwrap().key, "instagram");
        assert_eq!(detect("Firefox", "", &s(&["Мария", "vk.com"])).unwrap().key, "vk");
        assert!(detect("Google Chrome", "Chrome", &s(&["Скачивание завершено", "file.zip"])).is_none(), "an ordinary browser notification is no network");
        assert!(detect("Google Chrome", "Chrome", &s(&["box.com", "news"])).is_none(), "x.com is not found inside box.com");
        assert!(detect("Google Chrome", "Chrome", &s(&["telegram"])).is_none(), "a browser is judged by the site, not by a word of the text");
        // lookalike hosts are not the network
        for fake in ["evil-whatsapp.com", "web.whatsapp.com.evil.ru", "instagram.com.phish.io", "notvk.com", "vk.com-login.top"] {
            assert!(detect("Google Chrome", "Chrome", &s(&["Новое сообщение", fake])).is_none(), "{}", fake);
        }
        // a site named inside the text of another page does not turn it into that network
        assert!(detect("Google Chrome", "Chrome", &s(&["Акция", "войдите через web.whatsapp.com и получите приз", "promo-site.example"])).is_none());
    }

    #[test]
    fn the_line_with_only_the_site_is_dropped_from_a_browser_notification() {
        let wa = network("whatsapp").unwrap();
        assert_eq!(without_site_lines(wa, &s(&["Иван", "привет", "web.whatsapp.com"])), s(&["Иван", "привет"]));
        assert_eq!(without_site_lines(wa, &s(&["смотри web.whatsapp.com там всё"])), s(&["смотри web.whatsapp.com там всё"]), "a message that mentions the site stays");
    }

    #[test]
    fn the_list_keeps_the_newest_two_hundred_in_memory_and_counts_by_network() {
        let _g = crate::uistate::TEST_LOCK.lock();
        clear_for_tests();
        for i in 0..205 {
            push(if i % 2 == 0 { "telegram" } else { "" }, "Telegram Desktop", &format!("Иван {}", i), "привет");
        }
        let j = json();
        assert_eq!(j["messages"].as_array().unwrap().len(), KEEP);
        assert_eq!(j["messages"][0]["title"], "Иван 204", "newest first");
        assert_eq!(j["counts"]["telegram"].as_u64().unwrap() + j["counts"]["other"].as_u64().unwrap(), KEEP as u64);
        let id = last_id();
        assert_eq!(get(id).unwrap().title, "Иван 204");
        assert_eq!(clear(), KEEP);
        assert_eq!(last_id(), 0);
    }

    #[test]
    fn a_message_opens_the_program_it_came_from() {
        let progs = vec![
            ("Telegram Desktop".to_string(), "C:\\tg.lnk".to_string()),
            ("Telegram".to_string(), "C:\\tg2.lnk".to_string()),
            ("WhatsApp".to_string(), "shell:AppsFolder\\WA".to_string()),
            ("Discord Canary".to_string(), "C:\\dc.lnk".to_string()),
            ("Notepad".to_string(), "C:\\np.lnk".to_string()),
        ];
        // the same name wins over a longer one
        assert_eq!(pick_program("Telegram", &progs), Some("C:\\tg2.lnk".to_string()));
        assert_eq!(pick_program("telegram desktop", &progs), Some("C:\\tg.lnk".to_string()));
        assert_eq!(pick_program("WhatsApp", &progs), Some("shell:AppsFolder\\WA".to_string()));
        assert_eq!(pick_program("Discord", &progs), Some("C:\\dc.lnk".to_string()));
        // a part of a word is not a name; an empty or unknown name opens nothing
        assert_eq!(pick_program("Tele", &progs), None);
        assert_eq!(pick_program("", &progs), None);
        assert_eq!(pick_program("Unknown app", &progs), None);
        let msg = |app: &str, net: &str| Message { id: 1, at: 0, net: net.into(), app: app.into(), title: "t".into(), text: "x".into() };
        assert_eq!(open_target(&msg("Telegram", "telegram"), &progs), Ok(OpenTarget::Program("C:\\tg2.lnk".to_string())));
        // a browser notification opens the site of the network, never an address that came with the message
        assert_eq!(open_target(&msg("Google Chrome", "whatsapp"), &progs), Ok(OpenTarget::Site("https://web.whatsapp.com".to_string())));
        assert!(open_target(&msg("Google Chrome", ""), &progs).is_err());
        assert!(open_target(&msg("Signal", "signal"), &progs).unwrap_err().contains("нет в списке"));
    }
}
