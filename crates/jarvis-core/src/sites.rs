// The one catalogue of known websites: names as they are said or typed (Russian and English) -> the address.
//
// "открой YouTube" must open YouTube, never a Google search: a direct "open <site>" is recognized here, BEFORE the
// keyword triggers and the intent model (which used to take an English "youtube" for the nearest command "открой
// гугл"). A search ("найди в гугле …") is a different command. Programs that also have a website (Discord, Telegram)
// open as programs when installed; program.lua falls back to the website of the same entry when they are not.

pub struct Site {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub aliases: &'static [&'static str],
}

pub const SITES: &[Site] = &[
    Site { id: "youtube", name: "YouTube", url: "https://www.youtube.com/", aliases: &["ютуб", "ютьюб", "ютюб", "youtube", "you tube"] },
    Site { id: "youtube_music", name: "YouTube Music", url: "https://music.youtube.com/", aliases: &["ютуб музыка", "ютуб музыку", "youtube music", "ютуб мьюзик"] },
    Site { id: "google", name: "Google", url: "https://www.google.com/", aliases: &["гугл", "гугол", "google", "гугл поиск"] },
    Site { id: "gmail", name: "Gmail", url: "https://mail.google.com/", aliases: &["почта", "почту", "gmail", "джимейл", "гугл почта", "гугл почту"] },
    Site { id: "google_drive", name: "Google Диск", url: "https://drive.google.com/", aliases: &["гугл диск", "google drive", "гугл драйв"] },
    Site { id: "google_meet", name: "Google Meet", url: "https://meet.google.com/", aliases: &["гугл мит", "google meet", "гугл meet"] },
    Site { id: "gemini", name: "Google Gemini", url: "https://gemini.google.com/", aliases: &["гемини", "джемини", "gemini", "google gemini", "гугл гемини"] },
    Site { id: "translate", name: "Google Переводчик", url: "https://translate.google.com/", aliases: &["переводчик", "гугл переводчик", "google translate"] },
    Site { id: "github", name: "GitHub", url: "https://github.com/", aliases: &["гитхаб", "гит хаб", "github", "git hub"] },
    Site { id: "chatgpt", name: "ChatGPT", url: "https://chatgpt.com/", aliases: &["чат гпт", "чатгпт", "чат джипити", "chatgpt", "chat gpt"] },
    Site { id: "claude", name: "Claude", url: "https://claude.ai/", aliases: &["клод", "клауд", "claude"] },
    Site { id: "deepseek", name: "DeepSeek", url: "https://chat.deepseek.com/", aliases: &["дипсик", "дип сик", "deepseek", "deep seek"] },
    Site { id: "instagram", name: "Instagram", url: "https://www.instagram.com/", aliases: &["инстаграм", "инстаграмм", "инсту", "insta", "instagram"] },
    Site { id: "facebook", name: "Facebook", url: "https://www.facebook.com/", aliases: &["фейсбук", "facebook"] },
    Site { id: "x", name: "X (Twitter)", url: "https://x.com/", aliases: &["твиттер", "twitter", "икс твиттер"] },
    Site { id: "vk", name: "ВКонтакте", url: "https://vk.com/", aliases: &["вк", "вконтакте", "в контакте", "vk", "vkontakte"] },
    Site { id: "telegram", name: "Telegram Web", url: "https://web.telegram.org/", aliases: &["телеграм", "телеграмм", "телега", "telegram"] },
    Site { id: "whatsapp", name: "WhatsApp Web", url: "https://web.whatsapp.com/", aliases: &["ватсап", "вотсап", "вацап", "whatsapp", "whats app"] },
    Site { id: "discord", name: "Discord", url: "https://discord.com/app", aliases: &["дискорд", "discord"] },
    Site { id: "twitch", name: "Twitch", url: "https://www.twitch.tv/", aliases: &["твич", "twitch"] },
    Site { id: "reddit", name: "Reddit", url: "https://www.reddit.com/", aliases: &["реддит", "редит", "reddit"] },
    Site { id: "tiktok", name: "TikTok", url: "https://www.tiktok.com/", aliases: &["тикток", "тик ток", "tiktok"] },
    Site { id: "pinterest", name: "Pinterest", url: "https://www.pinterest.com/", aliases: &["пинтерест", "pinterest"] },
    Site { id: "netflix", name: "Netflix", url: "https://www.netflix.com/", aliases: &["нетфликс", "netflix"] },
    Site { id: "spotify", name: "Spotify", url: "https://open.spotify.com/", aliases: &["спотифай", "spotify"] },
    Site { id: "wikipedia", name: "Википедия", url: "https://ru.wikipedia.org/", aliases: &["википедия", "википедию", "вики", "wikipedia"] },
    Site { id: "habr", name: "Хабр", url: "https://habr.com/", aliases: &["хабр", "habr"] },
    Site { id: "yandex", name: "Яндекс", url: "https://ya.ru/", aliases: &["яндекс", "yandex", "ya ru", "яндекс поиск"] },
    Site { id: "yandex_maps", name: "Яндекс Карты", url: "https://yandex.ru/maps/", aliases: &["яндекс карты", "яндекс карта", "yandex maps"] },
    Site { id: "yandex_disk", name: "Яндекс Диск", url: "https://disk.yandex.ru/", aliases: &["яндекс диск", "yandex disk"] },
    Site { id: "yandex_music", name: "Яндекс Музыка", url: "https://music.yandex.ru/", aliases: &["яндекс музыка", "яндекс музыку", "yandex music"] },
    Site { id: "kinopoisk", name: "Кинопоиск", url: "https://www.kinopoisk.ru/", aliases: &["кинопоиск", "kinopoisk"] },
    Site { id: "avito", name: "Авито", url: "https://www.avito.ru/", aliases: &["авито", "avito"] },
    Site { id: "ozon", name: "Ozon", url: "https://www.ozon.ru/", aliases: &["озон", "ozon"] },
    Site { id: "wildberries", name: "Wildberries", url: "https://www.wildberries.ru/", aliases: &["вайлдберриз", "валдберис", "вб", "wildberries"] },
    Site { id: "aliexpress", name: "AliExpress", url: "https://aliexpress.com/", aliases: &["алиэкспресс", "али экспресс", "aliexpress"] },
    Site { id: "amazon", name: "Amazon", url: "https://www.amazon.com/", aliases: &["амазон", "amazon"] },
    Site { id: "epic", name: "Epic Games Store", url: "https://store.epicgames.com/", aliases: &["эпик", "эпик геймс", "epic games", "epic"] },
    Site { id: "steam", name: "Steam (магазин)", url: "https://store.steampowered.com/", aliases: &["стим магазин", "steam store"] },
    Site { id: "speedtest", name: "Speedtest", url: "https://www.speedtest.net/", aliases: &["спидтест", "speedtest"] },
];

// the verbs of "open a site" and the filler words around the name
const OPEN_VERBS: &[&str] = &["открой", "открыть", "откройте", "зайди", "зайти", "перейди", "перейти", "запусти", "покажи", "open", "go", "visit", "launch"];
const FILLER: &[&str] = &["на", "в", "сайт", "сайта", "страницу", "мне", "пожалуйста", "to", "the", "website", "site", "please"];

// words: lower case, ё = е, no punctuation ("YouTube!" -> "youtube"; "ya.ru" -> "ya ru")
pub fn words(text: &str) -> Vec<String> {
    text.to_lowercase().replace('ё', "е").split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_string()).collect()
}

// the site whose name is exactly these words
pub fn find(name_words: &[String]) -> Option<&'static Site> {
    if name_words.is_empty() {
        return None;
    }
    let joined = name_words.join(" ");
    SITES.iter().find(|s| s.aliases.iter().any(|a| words(a).join(" ") == joined))
}

// "открой YouTube", "зайди на сайт гитхаб", "open chatgpt" -> the site; None for anything else
// (a search, more words after the name, an unknown name)
pub fn direct_open(text: &str) -> Option<&'static Site> {
    let w = words(text);
    let start = w.iter().position(|x| OPEN_VERBS.contains(&x.as_str()))?;
    if start > 1 {
        return None; // the verb must begin the phrase (one polite word before it is allowed)
    }
    let rest: Vec<String> = w[start + 1..].iter().filter(|x| !FILLER.contains(&x.as_str())).cloned().collect();
    find(&rest)
}

pub fn by_id(id: &str) -> Option<&'static Site> {
    SITES.iter().find(|s| s.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Option<&'static str> {
        direct_open(text).map(|s| s.url)
    }

    #[test]
    fn a_site_named_in_any_language_opens_that_site() {
        assert_eq!(url("Открой YouTube"), Some("https://www.youtube.com/"));
        assert_eq!(url("открой ютуб"), Some("https://www.youtube.com/"));
        assert_eq!(url("Открой Google"), Some("https://www.google.com/"));
        assert_eq!(url("открой гитхаб"), Some("https://github.com/"));
        assert_eq!(url("открой GitHub"), Some("https://github.com/"));
        assert_eq!(url("Открой Instagram"), Some("https://www.instagram.com/"));
        assert_eq!(url("открой дискорд"), Some("https://discord.com/app"));
        assert_eq!(url("открой телеграм"), Some("https://web.telegram.org/"));
        assert_eq!(url("Открой ChatGPT"), Some("https://chatgpt.com/"));
        assert_eq!(url("открой чат гпт"), Some("https://chatgpt.com/"));
        assert_eq!(url("зайди на сайт яндекс карты"), Some("https://yandex.ru/maps/"));
        assert_eq!(url("открой ютуб музыку"), Some("https://music.youtube.com/"), "the longer name is its own site");
        assert_eq!(url("пожалуйста открой deepseek"), Some("https://chat.deepseek.com/"));
        assert_eq!(url("open youtube"), Some("https://www.youtube.com/"));
    }

    #[test]
    fn searches_unknown_names_and_longer_phrases_are_not_a_direct_open() {
        assert_eq!(url("найди в гугле как установить python"), None, "a search is not an open");
        assert_eq!(url("открой ютуб и найди котиков"), None, "more than a name: the website command decides");
        assert_eq!(url("открой блокнот"), None, "a program, not a site");
        assert_eq!(url("открой карты"), None, "ambiguous with the Maps app: the old command keeps it");
        assert_eq!(url("ютуб"), None, "no verb: not a command to open");
        assert_eq!(url("что такое ютуб открой"), None);
    }

    #[test]
    fn every_alias_belongs_to_one_site_and_every_url_is_https() {
        let mut seen = std::collections::HashMap::new();
        for s in SITES {
            assert!(s.url.starts_with("https://"), "{}", s.id);
            for a in s.aliases {
                let k = words(a).join(" ");
                if let Some(other) = seen.insert(k.clone(), s.id) {
                    panic!("«{}» names both {} and {}", k, other, s.id);
                }
            }
        }
    }
}
