// What is playing now on this computer, for the phone remote: the title of the window of a known player or a browser
// (AIMP, VLC, Spotify, YouTube in a browser ...). Read only: only the process name and the window title are looked at, nothing is
// changed and nothing leaves the computer except to the phone that holds the secret of the link.
// A title shows what the window says: a video in a background tab of a browser is not seen (the browser names the window by its
// active tab), and a player that is paused still shows its title.
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    pub app: String,
    pub title: String,
}

const BROWSERS: [&str; 6] = ["chrome", "msedge", "firefox", "browser", "opera", "brave"];
const BROWSER_SUFFIXES: [&str; 8] = ["google chrome", "microsoft edge", "mozilla firefox", "yandex", "яндекс", "opera", "brave", "chromium"];

fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(120).collect()
}

// "process|window title" lines (what the PowerShell script prints) -> the players found, the ones with a real title first
pub fn parse(lines: &str) -> Vec<NowPlaying> {
    let mut found: Vec<NowPlaying> = Vec::new();
    for line in lines.lines() {
        let Some((proc_name, title)) = line.split_once('|') else { continue };
        let (proc_name, title) = (proc_name.trim().to_lowercase(), clean(title));
        if title.is_empty() {
            continue;
        }
        let item = if BROWSERS.contains(&proc_name.as_str()) {
            // "Название ролика - YouTube - Google Chrome": the words before «YouTube» are the title
            let parts: Vec<&str> = title.split(" - ").collect();
            let Some(i) = parts.iter().position(|p| p.trim().eq_ignore_ascii_case("youtube") || p.trim().eq_ignore_ascii_case("youtube music")) else { continue };
            let name = parts[..i].join(" - ");
            let kind = if parts[i].to_lowercase().contains("music") { "YouTube Music" } else { "YouTube" };
            if name.trim().is_empty() || BROWSER_SUFFIXES.iter().any(|b| name.to_lowercase() == *b) {
                continue; // the home page of YouTube, not a video
            }
            NowPlaying { app: kind.to_string(), title: name }
        } else {
            let (app, suffix) = match proc_name.as_str() {
                "aimp" => ("AIMP", "aimp"),
                "vlc" => ("VLC", "vlc media player"),
                "spotify" => ("Spotify", "spotify"),
                "wmplayer" => ("Windows Media Player", "windows media player"),
                "foobar2000" => ("foobar2000", "foobar2000"),
                "music.ui" => ("Музыка Windows", "музыка"),
                "mpc-hc" | "mpc-hc64" => ("MPC-HC", "mpc-hc"),
                "potplayermini64" | "potplayermini" | "potplayer" => ("PotPlayer", "potplayer"),
                _ => continue,
            };
            // the window of an idle player says only its own name
            let low = title.to_lowercase();
            let name = if low == suffix || low.starts_with("spotify free") || low.starts_with("spotify premium") { String::new() } else { title.trim_end_matches(|c: char| c == ' ').to_string() };
            let name = match name.to_lowercase().strip_suffix(&format!(" - {}", suffix)) {
                Some(rest) => name[..rest.len()].to_string(),
                None => name,
            };
            NowPlaying { app: app.to_string(), title: name }
        };
        if !found.contains(&item) {
            found.push(item);
        }
    }
    found.sort_by_key(|n| n.title.is_empty());
    found.truncate(4);
    found
}

static CACHE: Lazy<Mutex<Option<(Instant, Vec<NowPlaying>)>>> = Lazy::new(|| Mutex::new(None));

#[cfg(target_os = "windows")]
fn read() -> Vec<NowPlaying> {
    let Ok(mut cmd) = crate::psrun::quiet(crate::psrun::Script::MediaNow, &[]) else { return Vec::new() };
    match cmd.output() {
        Ok(out) if out.status.success() => parse(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

#[cfg(not(target_os = "windows"))]
fn read() -> Vec<NowPlaying> {
    Vec::new()
}

// asked by the phone every few seconds: the answer is kept for three seconds so that one PowerShell serves them all
pub fn now() -> Vec<NowPlaying> {
    let mut g = CACHE.lock();
    if let Some((t, v)) = g.as_ref() {
        if t.elapsed() < Duration::from_secs(3) {
            return v.clone();
        }
    }
    let v = read();
    *g = Some((Instant::now(), v.clone()));
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn youtube_in_a_browser_and_aimp_are_found_by_the_window_titles() {
        let lines = "chrome|Баста - Сансара - YouTube - Google Chrome\nAIMP|Звери - Районы-кварталы\nmsedge|Новости - Microsoft Edge\nexplorer|Папка\n";
        let got = parse(lines);
        assert_eq!(got.len(), 2, "{:?}", got);
        assert!(got.contains(&NowPlaying { app: "YouTube".into(), title: "Баста - Сансара".into() }), "{:?}", got);
        assert!(got.contains(&NowPlaying { app: "AIMP".into(), title: "Звери - Районы-кварталы".into() }));
    }

    #[test]
    fn an_idle_player_and_the_home_page_of_youtube_have_no_title_and_other_windows_are_ignored() {
        let got = parse("AIMP|AIMP\nchrome|YouTube - Google Chrome\nVLC|song.mp3 - VLC media player\nnotepad|x");
        assert_eq!(got.iter().find(|n| n.app == "VLC").map(|n| n.title.as_str()), Some("song.mp3"));
        assert_eq!(got.iter().find(|n| n.app == "AIMP").map(|n| n.title.as_str()), Some(""), "a running player with nothing playing");
        assert!(!got.iter().any(|n| n.app == "YouTube"), "the home page is not a video: {:?}", got);
        assert_eq!(got[0].app, "VLC", "the ones with a title come first");
        assert!(parse("").is_empty() && parse("garbage without a bar").is_empty());
    }

    #[test]
    fn a_title_with_control_characters_is_cleaned_and_cut() {
        let long = format!("spotify|{}\u{7}x - y", "я".repeat(300));
        let got = parse(&long);
        assert_eq!(got.len(), 1);
        assert!(got[0].title.chars().count() <= 120 && !got[0].title.contains('\u{7}'));
    }
}
