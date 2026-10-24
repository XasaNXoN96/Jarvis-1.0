// «найди …»: the results of a search in the Internet are shown by Jarvis itself, one small window for each KIND of result: photos (Wikimedia
// Commons), videos (YouTube search page), links (DuckDuckGo, then Wikipedia), text (the article of Wikipedia). All without a key and without an account. Only the text of the request leaves the computer, and only after the user asked to search.
// Nothing is invented: when nothing could be obtained, the state says why. The window never receives an address from the page: the
// «Открыть в браузере» button names a result by its number and Jarvis opens the address it kept itself (https/http only, no password in it).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

const MAX_HITS: usize = 10;
const MAX_LINKS: usize = 10;
const MAX_PHOTOS: usize = 24;
const MAX_VIDEOS: usize = 8;
const DDG: &str = "https://html.duckduckgo.com/html/";

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

// one thing found: a photo, a video, a link or the text; `thumb` is a picture to draw (https), `note` is the words under the title
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Item {
    pub title: String,
    pub url: String,
    pub thumb: String,
    pub note: String,
}

#[derive(Default)]
struct Latest {
    id: u64,
    query: String,
    pending: u8,
    error: Option<String>,
    errors: Vec<String>,
    // why a kind has nothing: (kind, reason); the window says it instead of staying silent
    notes: Vec<(String, String)>,
    photos: Vec<Item>,
    videos: Vec<Item>,
    links: Vec<Item>,
    text: Option<Item>,
}

impl Latest {
    fn is_empty(&self) -> bool {
        self.photos.is_empty() && self.videos.is_empty() && self.links.is_empty() && self.text.is_none()
    }
}

static LATEST: Lazy<Mutex<Latest>> = Lazy::new(|| Mutex::new(Latest::default()));
static COUNTER: AtomicU64 = AtomicU64::new(0);

// percent-encoding of a query value
fn enc(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{:02X}", b) }).collect()
}

fn dec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// text of a piece of HTML: tags are removed, the common entities are decoded, spaces are collapsed
fn clean(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    let out = out.replace("&amp;", "&").replace("&quot;", "\"").replace("&#x27;", "'").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// an address that may be shown and opened: http(s), a host with a dot, no login and password
pub fn safe_url(u: &str) -> Option<String> {
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://"))?;
    let host_part = rest.split(|c| c == '/' || c == '?' || c == '#').next().unwrap_or("");
    if host_part.is_empty() || host_part.contains('@') || !host_part.contains('.') || u.chars().any(|c| c.is_whitespace() || c.is_control()) || u.len() > 2000 {
        return None;
    }
    Some(u.to_string())
}

pub fn host_of(url: &str) -> String {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://")).unwrap_or(url);
    rest.split(|c| c == '/' || c == '?' || c == '#').next().unwrap_or("").trim_start_matches("www.").to_string()
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{}=\"", name);
    let s = tag.find(&key)? + key.len();
    let e = tag[s..].find('"')?;
    Some(tag[s..s + e].to_string())
}

// the page of DuckDuckGo: every result has a link with the class «result__a» (the address is inside «uddg=») and a «result__snippet»
pub fn parse_ddg(html: &str) -> Vec<Hit> {
    let mut hits: Vec<Hit> = Vec::new();
    let marks: Vec<usize> = html.match_indices("result__a").map(|(i, _)| i).collect();
    for (k, &m) in marks.iter().enumerate() {
        let Some(tag_start) = html[..m].rfind("<a") else { continue };
        let Some(tag_end) = html[m..].find('>').map(|e| m + e) else { continue };
        let tag = &html[tag_start..tag_end];
        let Some(href) = attr(tag, "href").map(|h| h.replace("&amp;", "&")) else { continue };
        let target = match href.find("uddg=") {
            Some(p) => dec(href[p + 5..].split('&').next().unwrap_or("")),
            None if href.starts_with("//") => format!("https:{}", href),
            None => href,
        };
        // advertisements and DuckDuckGo's own pages are not results
        let Some(url) = safe_url(&target) else { continue };
        if host_of(&url).ends_with("duckduckgo.com") {
            continue;
        }
        let Some(close) = html[tag_end..].find("</a>").map(|e| tag_end + e) else { continue };
        let title = clean(&html[tag_end + 1..close]);
        let limit = marks.get(k + 1).copied().unwrap_or(html.len());
        let snippet = html[close..limit].find("result__snippet").and_then(|p| {
            let from = close + p;
            let s = html[from..].find('>')? + from + 1;
            let e = html[s..].find("</a>").or_else(|| html[s..].find("</td>")).or_else(|| html[s..].find("</div>"))? + s;
            Some(clean(&html[s..e]))
        }).unwrap_or_default();
        if title.is_empty() || hits.iter().any(|h| h.url == url) {
            continue;
        }
        hits.push(Hit { title, url, snippet });
        if hits.len() >= MAX_HITS {
            break;
        }
    }
    hits
}

// the answer of the search API of Wikipedia
pub fn parse_wiki(v: &Value, lang: &str) -> Vec<Hit> {
    let Some(list) = v.pointer("/query/search").and_then(|s| s.as_array()) else { return Vec::new() };
    list.iter()
        .filter_map(|e| {
            let title = e.get("title")?.as_str()?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let snippet = clean(e.get("snippet").and_then(|s| s.as_str()).unwrap_or(""));
            Some(Hit { url: format!("https://{}.wikipedia.org/wiki/{}", lang, enc(&title.replace(' ', "_"))), title: format!("{} — Википедия", title), snippet })
        })
        .take(MAX_HITS)
        .collect()
}

fn wiki_lang(q: &str) -> &'static str {
    if q.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)) { "ru" } else { "en" }
}

#[cfg(all(feature = "lua", not(test)))]
fn http_get(url: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(15))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) Jarvis")
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(url).send().map_err(|e| if e.is_timeout() { "поиск не отвечает".to_string() } else { "нет соединения с поиском".to_string() })?;
    if !resp.status().is_success() {
        return Err(format!("поиск ответил кодом {}", resp.status().as_u16()));
    }
    resp.text().map_err(|_| "непонятный ответ поиска".to_string())
}

#[cfg(any(not(feature = "lua"), test))]
fn http_get(_url: &str) -> Result<String, String> {
    // (the automated tests never go to the Internet)
    let _ = Duration::from_secs(0);
    Err("в этой сборке нет сетевого модуля".into())
}

// the photos: Wikimedia Commons (free images, a thumbnail and the page of the file); no key
pub fn parse_commons(v: &Value) -> Vec<Item> {
    let Some(pages) = v.pointer("/query/pages").and_then(|p| p.as_object()) else { return Vec::new() };
    let mut rows: Vec<(i64, Item)> = pages
        .values()
        .filter_map(|e| {
            let info = e.get("imageinfo")?.as_array()?.first()?;
            if !info.get("mime").and_then(|m| m.as_str()).unwrap_or("").starts_with("image/") {
                return None;
            }
            let thumb = safe_url(info.get("thumburl").and_then(|t| t.as_str())?)?;
            let page = safe_url(info.get("descriptionurl").and_then(|t| t.as_str())?)?;
            let name = e.get("title")?.as_str()?.trim_start_matches("File:");
            let title = name.rsplit_once('.').map(|(a, _)| a).unwrap_or(name).replace('_', " ");
            Some((e.get("index").and_then(|i| i.as_i64()).unwrap_or(0), Item { title, url: page, thumb, note: String::new() }))
        })
        .collect();
    rows.sort_by_key(|r| r.0);
    rows.into_iter().map(|r| r.1).take(MAX_PHOTOS).collect()
}

// the videos: the search page of YouTube carries its data as JSON; every video has «"videoRenderer":{"videoId":"…»
pub fn parse_youtube(html: &str) -> Vec<Item> {
    let unesc = |t: &str| t.replace("\\u0026", "&").replace("\\\"", "\"").replace("\\\\", "\\").replace("\\/", "/");
    let mut out: Vec<Item> = Vec::new();
    let key = "\"videoRenderer\":{\"videoId\":\"";
    let marks: Vec<usize> = html.match_indices(key).map(|(i, _)| i + key.len()).collect();
    for (k, &m) in marks.iter().enumerate() {
        let id: String = html[m..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
        if id.len() != 11 || out.iter().any(|v| v.url.ends_with(&id)) {
            continue;
        }
        let end = marks.get(k + 1).copied().unwrap_or(html.len().min(m + 6000)).min(html.len());
        let chunk = &html[m..end];
        let text_after = |key: &str| -> Option<String> {
            let p = chunk.find(key)? + key.len();
            let mut e = p;
            let b = chunk.as_bytes();
            while e < b.len() && !(b[e] == b'"' && b[e - 1] != b'\\') {
                e += 1;
            }
            (e <= chunk.len() && chunk.is_char_boundary(p) && chunk.is_char_boundary(e)).then(|| unesc(&chunk[p..e]))
        };
        let Some(title) = text_after("\"title\":{\"runs\":[{\"text\":\"").filter(|t| !t.trim().is_empty()) else { continue };
        let note = text_after("\"ownerText\":{\"runs\":[{\"text\":\"").unwrap_or_default();
        out.push(Item { title, url: format!("https://www.youtube.com/watch?v={}", id), thumb: format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", id), note });
        if out.len() >= MAX_VIDEOS {
            break;
        }
    }
    out
}

// the text: the first words of the best article of Wikipedia (the extract API, plain text)
pub fn parse_extract(v: &Value, lang: &str) -> Option<Item> {
    let pages = v.pointer("/query/pages")?.as_object()?;
    let e = pages.values().min_by_key(|e| e.get("index").and_then(|i| i.as_i64()).unwrap_or(i64::MAX))?;
    let title = e.get("title")?.as_str()?.trim().to_string();
    let text = e.get("extract")?.as_str()?.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() || text.chars().count() < 40 {
        return None;
    }
    Some(Item { url: format!("https://{}.wikipedia.org/wiki/{}", lang, enc(&title.replace(' ', "_"))), title, thumb: String::new(), note: text })
}

fn links_section(query: &str) -> Result<Vec<Item>, String> {
    let mut errors = Vec::new();
    let mut hits = Vec::new();
    match http_get(&format!("{}?q={}&kl=ru-ru", std::env::var("JARVIS_WEBSEARCH_DDG").unwrap_or_else(|_| DDG.to_string()), enc(query))) {
        Ok(html) => hits = parse_ddg(&html),
        Err(e) => errors.push(e),
    }
    if hits.len() < 3 {
        let lang = wiki_lang(query);
        let url = format!("https://{}.wikipedia.org/w/api.php?action=query&list=search&srsearch={}&format=json&srlimit={}&utf8=1", lang, enc(query), MAX_HITS);
        match http_get(&url).and_then(|b| serde_json::from_str::<Value>(&b).map_err(|_| "непонятный ответ Википедии".to_string())) {
            Ok(v) => {
                for h in parse_wiki(&v, lang) {
                    if hits.len() < MAX_LINKS && !hits.iter().any(|x| x.url == h.url) {
                        hits.push(h);
                    }
                }
            }
            Err(e) => errors.push(e),
        }
    }
    if hits.is_empty() && !errors.is_empty() {
        return Err(errors.join("; "));
    }
    Ok(hits.into_iter().map(|h| Item { title: h.title, url: h.url, thumb: String::new(), note: h.snippet }).take(MAX_LINKS).collect())
}

fn photos_section(query: &str) -> Result<Vec<Item>, String> {
    let url = format!("https://commons.wikimedia.org/w/api.php?action=query&generator=search&gsrnamespace=6&gsrsearch={}&gsrlimit={}&prop=imageinfo&iiprop=url%7Cmime&iiurlwidth=480&format=json&utf8=1", enc(query), MAX_PHOTOS + 2);
    let v = serde_json::from_str::<Value>(&http_get(&url)?).map_err(|_| "непонятный ответ с фотографиями".to_string())?;
    Ok(parse_commons(&v))
}

fn videos_section(query: &str) -> Result<Vec<Item>, String> {
    Ok(parse_youtube(&http_get(&format!("https://www.youtube.com/results?search_query={}&hl=ru", enc(query)))?))
}

fn text_section(query: &str) -> Result<Option<Item>, String> {
    let lang = wiki_lang(query);
    let url = format!("https://{}.wikipedia.org/w/api.php?action=query&generator=search&gsrsearch={}&gsrlimit=1&prop=extracts&exintro=1&explaintext=1&exchars=2500&format=json&utf8=1", lang, enc(query));
    let v = serde_json::from_str::<Value>(&http_get(&url)?).map_err(|_| "непонятный ответ Википедии".to_string())?;
    Ok(parse_extract(&v, lang))
}

// the request of the user: checked here, the four searches run in their own threads (the answer of the Internet may take seconds)
pub fn start(query: &str) -> Result<u64, String> {
    let q = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if q.is_empty() {
        return Err("Нечего искать: пустой запрос.".into());
    }
    if q.chars().count() > 300 {
        return Err("Запрос слишком длинный для поиска.".into());
    }
    let id = COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    *LATEST.lock() = Latest { id, query: q.clone(), pending: 4, ..Default::default() };
    // every section writes its own part; the last one to finish says «ready»
    let done = move |id: u64, put: Box<dyn FnOnce(&mut Latest) + Send>| {
        let mut l = LATEST.lock();
        if l.id != id {
            return; // a newer search has started: the old answer is not shown
        }
        put(&mut l);
        l.errors.dedup();
        l.pending = l.pending.saturating_sub(1);
        if l.pending == 0 && l.is_empty() && !l.errors.is_empty() {
            l.error = Some(l.errors.join("; "));
        }
    };
    let (q1, q2, q3, q4) = (q.clone(), q.clone(), q.clone(), q);
    std::thread::spawn(move || {
        let r = photos_section(&q1);
        done(id, Box::new(move |l| match r {
            Ok(v) if v.is_empty() => l.notes.push(("photos".into(), "фотографий не нашлось".into())),
            Ok(v) => l.photos = v,
            Err(e) => { l.notes.push(("photos".into(), e.clone())); l.errors.push(e) }
        }));
    });
    std::thread::spawn(move || {
        let r = videos_section(&q2);
        done(id, Box::new(move |l| match r {
            Ok(v) if v.is_empty() => l.notes.push(("videos".into(), "YouTube не вернул список видео (страница могла измениться)".into())),
            Ok(v) => l.videos = v,
            Err(e) => { l.notes.push(("videos".into(), e.clone())); l.errors.push(e) }
        }));
    });
    std::thread::spawn(move || {
        let r = links_section(&q3);
        done(id, Box::new(move |l| match r {
            Ok(v) if v.is_empty() => l.notes.push(("links".into(), "ссылок не нашлось".into())),
            Ok(v) => l.links = v,
            Err(e) => { l.notes.push(("links".into(), e.clone())); l.errors.push(e) }
        }));
    });
    std::thread::spawn(move || {
        let r = text_section(&q4);
        done(id, Box::new(move |l| match r {
            Ok(None) => l.notes.push(("text".into(), "статьи в Википедии не нашлось".into())),
            Ok(Some(v)) => l.text = Some(v),
            Err(e) => { l.notes.push(("text".into(), e.clone())); l.errors.push(e) }
        }));
    });
    Ok(id)
}

fn item_json(i: &Item) -> Value {
    json!({ "title": i.title, "host": host_of(&i.url), "thumb": i.thumb, "note": i.note })
}

// for the window: what the page may see (a thumbnail to draw and the host of the page; the address of a page stays here)
pub fn snapshot() -> Value {
    let l = LATEST.lock();
    json!({
        "id": l.id,
        "query": l.query,
        "loading": l.pending > 0,
        "error": l.error,
        "photos": l.photos.iter().map(item_json).collect::<Vec<_>>(),
        "videos": l.videos.iter().map(item_json).collect::<Vec<_>>(),
        "links": l.links.iter().map(item_json).collect::<Vec<_>>(),
        "text": l.text.as_ref().map(item_json),
        "notes": l.notes.iter().map(|(k, why)| json!({ "kind": k, "why": why })).collect::<Vec<_>>(),
    })
}

// the address of the item number `n` of the kind `kind` of the search number `id` (only the latest search is kept)
pub fn hit_url(id: u64, kind: &str, n: usize) -> Option<String> {
    let l = LATEST.lock();
    if l.id != id {
        return None;
    }
    match kind {
        "photos" => l.photos.get(n).map(|i| i.url.clone()),
        "videos" => l.videos.get(n).map(|i| i.url.clone()),
        "links" => l.links.get(n).map(|i| i.url.clone()),
        "text" if n == 0 => l.text.as_ref().map(|i| i.url.clone()),
        _ => None,
    }
}

// (the last search is one global state: the tests that touch it take this lock)
#[cfg(test)]
pub(crate) static TEST_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[cfg(test)]
pub(crate) fn set_for_test(query: &str, links: Vec<Item>) -> u64 {
    let id = COUNTER.fetch_add(1, Ordering::SeqCst) + 1;
    *LATEST.lock() = Latest { id, query: query.into(), links, ..Default::default() };
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<div class="result results_links"><h2 class="result__title"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fborsch%3Fa%3D1&amp;rut=abc">Рецепт <b>борща</b> &amp; свёкла</a></h2>
<a class="result__snippet" href="//duckduckgo.com/l/?uddg=x">Как сварить <b>борщ</b> за час: свёкла, капуста &quot;и мясо&quot;.</a></div>
<div class="result"><a class="result__a" href="https://ads.duckduckgo.com/y.js?ad=1">Реклама</a></div>
<div class="result"><a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=http%3A%2F%2Fuser%3Apass%40bad.example%2F&amp;rut=1">С паролем в адресе</a></div>
<div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=javascript%3Aalert(1)">Скрипт</a></div>
<div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fsecond.org%2F">Второй сайт</a><a class="result__snippet" href="x">Второй текст</a></div>
<div class="result"><a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fborsch%3Fa%3D1">Тот же адрес</a></div>"#;

    #[test]
    fn results_of_duckduckgo_are_read_cleaned_and_filtered() {
        let h = parse_ddg(PAGE);
        assert_eq!(h.len(), 2, "{:?}", h);
        assert_eq!(h[0].title, "Рецепт борща & свёкла");
        assert_eq!(h[0].url, "https://example.com/borsch?a=1");
        assert_eq!(h[0].snippet, "Как сварить борщ за час: свёкла, капуста \"и мясо\".");
        assert_eq!((h[1].url.as_str(), h[1].snippet.as_str()), ("https://second.org/", "Второй текст"));
        assert!(parse_ddg("<html>ничего</html>").is_empty() && parse_ddg("").is_empty());
    }

    #[test]
    fn at_most_ten_results_and_no_broken_page_panics() {
        let many: String = (0..14).map(|i| format!(r#"<a class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fs{}.org%2F">Сайт {}</a>"#, i, i)).collect();
        assert_eq!(parse_ddg(&many).len(), 10, "the most that is kept");
        for broken in ["result__a", "<a class=\"result__a\"", "<a class=\"result__a\" href=\"", "<a class=\"result__a\" href=\"//x\">без конца", "%"] {
            let _ = parse_ddg(broken);
        }
        assert_eq!(dec("a%20b%2"), "a b%2");
        assert_eq!(dec("%D0%B1%D0%BE%D1%80%D1%89"), "борщ");
    }

    #[test]
    fn wikipedia_results_get_an_address_and_a_clean_text() {
        let v = json!({ "query": { "search": [
            { "title": "Борщ", "snippet": "<span class=\"searchmatch\">Борщ</span> — суп &quot;красный&quot;" },
            { "title": "Свёкла столовая", "snippet": "" },
            { "title": "", "snippet": "x" }
        ] } });
        let h = parse_wiki(&v, "ru");
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].title, "Борщ — Википедия");
        assert_eq!(h[0].snippet, "Борщ — суп \"красный\"");
        assert_eq!(h[1].url, "https://ru.wikipedia.org/wiki/%D0%A1%D0%B2%D1%91%D0%BA%D0%BB%D0%B0_%D1%81%D1%82%D0%BE%D0%BB%D0%BE%D0%B2%D0%B0%D1%8F");
        assert!(parse_wiki(&json!({}), "ru").is_empty());
        assert_eq!((wiki_lang("рецепт борща"), wiki_lang("rust language")), ("ru", "en"));
    }

    #[test]
    fn only_safe_addresses_are_kept() {
        assert!(safe_url("https://example.com/a?b=c").is_some());
        for bad in ["javascript:alert(1)", "file:///c:/x", "https://user:pw@example.com/", "https://localhost/", "https:///x", "https://exa mple.com/", "ftp://example.com/", ""] {
            assert!(safe_url(bad).is_none(), "{}", bad);
        }
        assert_eq!(host_of("https://www.example.com/a?b"), "example.com");
    }

    #[test]
    fn photos_of_commons_are_sorted_filtered_and_named() {
        let v = json!({ "query": { "pages": {
            "11": { "title": "File:Zoom_market_2.jpg", "index": 2, "imageinfo": [{ "mime": "image/jpeg", "thumburl": "https://upload.wikimedia.org/t2.jpg", "descriptionurl": "https://commons.wikimedia.org/wiki/File:Zoom_market_2.jpg" }] },
            "10": { "title": "File:Zoom market 1.png", "index": 1, "imageinfo": [{ "mime": "image/png", "thumburl": "https://upload.wikimedia.org/t1.png", "descriptionurl": "https://commons.wikimedia.org/wiki/File:Zoom_market_1.png" }] },
            "12": { "title": "File:Clip.webm", "index": 3, "imageinfo": [{ "mime": "video/webm", "thumburl": "https://upload.wikimedia.org/t3.jpg", "descriptionurl": "https://commons.wikimedia.org/wiki/File:Clip.webm" }] },
            "13": { "title": "File:Bad.jpg", "index": 4, "imageinfo": [{ "mime": "image/jpeg", "thumburl": "javascript:alert(1)", "descriptionurl": "https://commons.wikimedia.org/wiki/File:Bad.jpg" }] }
        } } });
        let p = parse_commons(&v);
        assert_eq!(p.len(), 2, "{:?}", p);
        assert_eq!((p[0].title.as_str(), p[0].thumb.as_str()), ("Zoom market 1", "https://upload.wikimedia.org/t1.png"));
        assert_eq!(p[1].title, "Zoom market 2");
        assert!(parse_commons(&json!({})).is_empty());
    }

    #[test]
    fn videos_of_youtube_are_read_from_the_page_data() {
        let page = r#"var ytInitialData = {"contents":[{"videoRenderer":{"videoId":"abcDEF12345","thumbnail":{},"title":{"runs":[{"text":"Zoom \"Market\" & обзор"}],"accessibility":{}},"ownerText":{"runs":[{"text":"Канал Один"}]}}},
{"videoRenderer":{"videoId":"zzzzzzzzzzz","title":{"runs":[{"text":"Второе видео"}]},"ownerText":{"runs":[{"text":"Канал Два"}]}}},
{"videoRenderer":{"videoId":"abcDEF12345","title":{"runs":[{"text":"Дубль"}]}}},
{"videoRenderer":{"videoId":"short","title":{"runs":[{"text":"Не видео"}]}}}]}"#;
        let v = parse_youtube(page);
        assert_eq!(v.len(), 2, "{:?}", v);
        assert_eq!(v[0].title, "Zoom \"Market\" & обзор");
        assert_eq!((v[0].url.as_str(), v[0].note.as_str()), ("https://www.youtube.com/watch?v=abcDEF12345", "Канал Один"));
        assert_eq!(v[0].thumb, "https://i.ytimg.com/vi/abcDEF12345/mqdefault.jpg");
        assert_eq!(v[1].title, "Второе видео");
        assert!(parse_youtube("").is_empty() && parse_youtube("\"videoRenderer\":{\"videoId\":\"").is_empty());
    }

    #[test]
    fn the_text_is_the_start_of_the_best_article() {
        let v = json!({ "query": { "pages": {
            "5": { "title": "Zoom Market", "index": 2, "extract": "Zoom   Market — онлайн-магазин электроники и товаров для дома,\nработающий с 2019 года." },
            "6": { "title": "Другая", "index": 1, "extract": "Другая статья про совсем иное, достаточно длинная для показа в окне." }
        } } });
        let t = parse_extract(&v, "ru").unwrap();
        assert_eq!(t.title, "Другая");
        assert_eq!(t.url, "https://ru.wikipedia.org/wiki/%D0%94%D1%80%D1%83%D0%B3%D0%B0%D1%8F");
        let one = json!({ "query": { "pages": { "5": { "title": "Zoom Market", "extract": "Zoom   Market — онлайн-магазин электроники и товаров для дома,\nработающий с 2019 года." } } } });
        assert!(parse_extract(&one, "ru").unwrap().note.starts_with("Zoom Market — онлайн-магазин"));
        assert!(parse_extract(&json!({ "query": { "pages": { "5": { "title": "X", "extract": "коротко" } } } }), "ru").is_none());
        assert!(parse_extract(&json!({}), "ru").is_none());
    }

    #[test]
    fn the_window_sees_the_host_and_thumbs_only_and_opens_by_number() {
        let _l = TEST_LOCK.lock();
        let id = set_for_test("борщ", vec![Item { title: "Т".into(), url: "https://www.example.com/secret?x=1".into(), thumb: String::new(), note: "с".into() }]);
        let s = snapshot();
        assert_eq!(s["id"], id);
        assert_eq!(s["links"][0]["host"], "example.com");
        assert!(!s.to_string().contains("secret"), "the page gets no addresses: {}", s);
        assert_eq!(hit_url(id, "links", 0).as_deref(), Some("https://www.example.com/secret?x=1"));
        assert!(hit_url(id, "links", 1).is_none() && hit_url(id + 1, "links", 0).is_none() && hit_url(id, "photos", 0).is_none() && hit_url(id, "text", 0).is_none() && hit_url(id, "other", 0).is_none());
    }

    #[test]
    fn a_search_in_the_tests_ends_with_the_reason_and_never_with_made_up_results() {
        let _l = TEST_LOCK.lock();
        let id = start("слон").unwrap();
        for _ in 0..200 {
            if !snapshot()["loading"].as_bool().unwrap() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let s = snapshot();
        assert_eq!(s["id"], id);
        assert!(!s["loading"].as_bool().unwrap(), "{}", s);
        assert!(s["notes"].as_array().unwrap().len() >= 4 && s["notes"][0]["why"].is_string(), "every kind says why it is empty: {}", s);
        assert!(s["photos"].as_array().unwrap().is_empty() && s["text"].is_null() && s["error"].is_string(), "{}", s);
    }

    #[test]
    fn a_request_is_checked_before_anything_is_sent() {
        let _l = TEST_LOCK.lock();
        assert!(start("   ").is_err());
        assert!(start(&"а".repeat(301)).is_err());
    }
}
