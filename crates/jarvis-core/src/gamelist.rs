//! The list of games of the game mode (gamemode.rs): the ones the user added by hand, by voice, or that the scan of the computer found.
//! It lies in `game_list.json` of the data folder. Every entry is a program name (`game.exe`), a name to show, the kind of the game
//! (shooter / racing / action) and where it came from. A known online game or a program that is not a game never enters the list.

use crate::gamemode::{refuse_reason, Genre};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

pub const MAX_GAMES: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub exe: String,
    pub name: String,
    pub genre: Genre,
    // "manual" (typed in the Games page), "voice" («добавь эту игру»), "scan" (found by the scan of the computer)
    pub source: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    // a small label over the game while the mode is on
    pub overlay: bool,
    // a short sound when a phrase of the game was carried out
    pub sound: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { overlay: false, sound: false }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct File {
    pub games: Vec<Entry>,
    pub options: Options,
}

static WRITE: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

#[cfg(test)]
thread_local! {
    static TEST_FILE: std::cell::RefCell<File> = std::cell::RefCell::new(File::default());
}

pub fn parse_file(text: &str) -> File {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return File::default() };
    let mut f = File::default();
    if let Some(list) = v.get("games").and_then(|g| g.as_array()) {
        for g in list.iter().take(MAX_GAMES) {
            let exe = g.get("exe").and_then(|x| x.as_str()).unwrap_or("").trim().to_lowercase();
            if refuse_reason(&exe).is_some() || f.games.iter().any(|e| e.exe == exe) {
                continue;
            }
            let name = g.get("name").and_then(|x| x.as_str()).unwrap_or("").trim().chars().take(80).collect::<String>();
            let genre = g.get("genre").and_then(|x| x.as_str()).and_then(Genre::from_name).unwrap_or_default();
            let source = match g.get("source").and_then(|x| x.as_str()) {
                Some(s @ ("manual" | "voice" | "scan")) => s.to_string(),
                _ => "manual".to_string(),
            };
            f.games.push(Entry { name: if name.is_empty() { exe.clone() } else { name }, exe, genre, source });
        }
    }
    // the older form of the file: «exes» and «genres»
    if f.games.is_empty() {
        if let Some(exes) = v.get("exes").and_then(|e| e.as_array()) {
            for e in exes.iter().filter_map(|x| x.as_str()).take(MAX_GAMES) {
                let exe = e.trim().to_lowercase();
                if refuse_reason(&exe).is_some() || f.games.iter().any(|g| g.exe == exe) {
                    continue;
                }
                let genre = v.get("genres").and_then(|g| g.get(&exe)).and_then(|x| x.as_str()).and_then(Genre::from_name).unwrap_or_default();
                f.games.push(Entry { name: exe.clone(), exe, genre, source: "voice".into() });
            }
        }
    }
    if let Some(o) = v.get("options") {
        f.options.overlay = o.get("overlay").and_then(|x| x.as_bool()).unwrap_or(false);
        f.options.sound = o.get("sound").and_then(|x| x.as_bool()).unwrap_or(false);
    }
    f
}

pub fn to_json(f: &File) -> String {
    let games: Vec<Value> = f.games.iter().map(|e| json!({ "exe": e.exe, "name": e.name, "genre": e.genre.name(), "source": e.source })).collect();
    serde_json::to_string_pretty(&json!({ "games": games, "options": { "overlay": f.options.overlay, "sound": f.options.sound } })).unwrap_or_default()
}

fn read() -> File {
    #[cfg(test)]
    {
        return TEST_FILE.with(|t| t.borrow().clone());
    }
    #[cfg(not(test))]
    {
        std::fs::read_to_string(crate::APP_DIR.join("game_list.json")).map(|t| parse_file(&t)).unwrap_or_default()
    }
}

fn write(f: &File) {
    #[cfg(test)]
    {
        TEST_FILE.with(|t| *t.borrow_mut() = f.clone());
    }
    #[cfg(not(test))]
    {
        let path = crate::APP_DIR.join("game_list.json");
        if let Ok(old) = std::fs::read_to_string(&path) {
            crate::keep_copy_if_repaired(&path, &old, &to_json(&parse_file(&old)));
        }
        let _ = crate::write_atomic(&path, to_json(f));
    }
}

pub fn load() -> Vec<Entry> {
    read().games
}

pub fn options() -> Options {
    read().options
}

pub fn set_options(o: Options) {
    let _g = WRITE.lock();
    let mut f = read();
    f.options = o;
    write(&f);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Added {
    New,
    // it was there: the kind (and the name) were updated when the new entry carried them
    Known,
}

// adds a game. `genre_named`: the user named the kind (an old entry is changed), else the kind of a new entry is the one given
pub fn add(e: Entry, genre_named: bool) -> Result<Added, String> {
    let exe = e.exe.trim().to_lowercase();
    if let Some(why) = refuse_reason(&exe) {
        return Err(why);
    }
    let _g = WRITE.lock();
    let mut f = read();
    let name = e.name.trim().chars().take(80).collect::<String>();
    if let Some(old) = f.games.iter_mut().find(|x| x.exe == exe) {
        if genre_named {
            old.genre = e.genre;
        }
        if !name.is_empty() && old.source != "voice" && e.source != "scan" {
            old.name = name;
        }
        write(&f);
        return Ok(Added::Known);
    }
    if f.games.len() >= MAX_GAMES {
        return Err(format!("в списке уже {} игр: уберите ненужные", MAX_GAMES));
    }
    f.games.push(Entry { name: if name.is_empty() { exe.clone() } else { name }, exe, genre: e.genre, source: e.source });
    write(&f);
    Ok(Added::New)
}

pub fn remove(exe: &str) -> bool {
    let _g = WRITE.lock();
    let mut f = read();
    let before = f.games.len();
    f.games.retain(|g| g.exe != exe.trim().to_lowercase());
    let changed = f.games.len() != before;
    if changed {
        write(&f);
    }
    changed
}

pub fn set_genre(exe: &str, genre: Genre) -> bool {
    let _g = WRITE.lock();
    let mut f = read();
    let Some(g) = f.games.iter_mut().find(|g| g.exe == exe.trim().to_lowercase()) else { return false };
    g.genre = genre;
    write(&f);
    true
}

#[cfg(test)]
pub fn test_reset() {
    TEST_FILE.with(|t| *t.borrow_mut() = File::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(exe: &str, genre: Genre, source: &str) -> Entry {
        Entry { exe: exe.into(), name: exe.into(), genre, source: source.into() }
    }

    #[test]
    fn games_are_added_changed_and_removed_and_the_refused_ones_never_enter() {
        test_reset();
        assert_eq!(add(e("Game.EXE", Genre::Shooter, "manual"), false), Ok(Added::New));
        assert_eq!(load()[0].exe, "game.exe");
        // again: known; a named kind changes it
        assert_eq!(add(e("game.exe", Genre::Racing, "voice"), false), Ok(Added::Known));
        assert_eq!(load()[0].genre, Genre::Shooter);
        assert_eq!(add(e("game.exe", Genre::Racing, "voice"), true), Ok(Added::Known));
        assert_eq!(load()[0].genre, Genre::Racing);
        assert!(set_genre("game.exe", Genre::Action) && load()[0].genre == Genre::Action);
        assert!(!set_genre("nope.exe", Genre::Action));
        // refused
        for bad in ["valorant.exe", "chrome.exe", "game", "a\\\\b.exe", ""] {
            assert!(add(e(bad, Genre::Shooter, "manual"), false).is_err(), "{}", bad);
        }
        assert_eq!(load().len(), 1);
        assert!(remove("GAME.exe") && load().is_empty());
        assert!(!remove("game.exe"));
    }

    #[test]
    fn the_list_is_never_longer_than_the_limit() {
        test_reset();
        for i in 0..MAX_GAMES {
            assert_eq!(add(e(&format!("g{}.exe", i), Genre::Shooter, "scan"), false), Ok(Added::New));
        }
        assert!(add(e("one-more.exe", Genre::Shooter, "scan"), false).is_err());
        test_reset();
    }

    #[test]
    fn the_file_round_trips_and_the_older_form_and_a_broken_file_are_read() {
        let f = File { games: vec![Entry { exe: "a.exe".into(), name: "Игра А".into(), genre: Genre::Action, source: "scan".into() }], options: Options { overlay: true, sound: false } };
        assert_eq!(parse_file(&to_json(&f)), f);
        // the older form
        let old = parse_file(r#"{"exes":["x.exe","chrome.exe","y.exe"],"genres":{"y.exe":"racing"}}"#);
        assert_eq!(old.games.iter().map(|g| (g.exe.as_str(), g.genre)).collect::<Vec<_>>(), vec![("x.exe", Genre::Shooter), ("y.exe", Genre::Racing)]);
        assert_eq!(parse_file("не json"), File::default());
        // a refused program in the file is dropped, a repeated one stays once, unknown source becomes manual
        let g = parse_file(r#"{"games":[{"exe":"valorant.exe"},{"exe":"z.exe","source":"???"},{"exe":"z.exe"}]}"#);
        assert_eq!(g.games.len(), 1);
        assert_eq!(g.games[0].source, "manual");
    }

    #[test]
    fn a_broken_or_repaired_file_is_kept_beside_the_new_one_and_a_clean_one_is_not() {
        let dir = std::env::temp_dir().join(format!("jarvis-gl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game_list.json");
        let clean = to_json(&File { games: vec![Entry { exe: "a.exe".into(), name: "A".into(), genre: Genre::Racing, source: "manual".into() }], options: Options::default() });
        assert!(crate::keep_copy_if_repaired(&path, &clean, &to_json(&parse_file(&clean))).is_none(), "a clean file needs no copy");
        assert!(crate::keep_copy_if_repaired(&path, "  ", "{}").is_none(), "an empty file holds nothing");
        // broken text: the copy holds it as it was
        let broken = "{\"games\": [{\"exe\": \"b.exe\"";
        let copy = crate::keep_copy_if_repaired(&path, broken, &to_json(&parse_file(broken))).expect("a copy");
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), broken);
        // a bad entry that the reading drops
        let partly = r#"{"games":[{"exe":"ok.exe"},{"exe":"valorant.exe"}]}"#;
        assert!(crate::keep_copy_if_repaired(&path, partly, &to_json(&parse_file(partly))).is_some());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_options_are_kept() {
        test_reset();
        assert_eq!(options(), Options::default());
        set_options(Options { overlay: true, sound: true });
        assert_eq!(options(), Options { overlay: true, sound: true });
        test_reset();
    }
}
