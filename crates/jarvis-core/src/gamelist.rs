//! The library of games of the game mode (gamemode.rs) and of the Games page: the ones the user added by hand, by voice, or that the scan of
//! the computer found. It lies in `game_list.json` of the data folder.
//!
//! Every entry has a stable `id` (it never changes, even when the path is changed), a name to show, the program (`game.exe`), the full path
//! of that program (when it is known), where it came from, the kind of the game and the times of adding and of the last launch.
//! A known online game or a program that is not a game never enters the library. Removing an entry removes ONLY the entry: nothing of
//! the game on the disk is touched (there is no code in this program that deletes a file of a game).

use crate::gamemode::{refuse_reason, Genre};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};

pub const MAX_GAMES: usize = 1000;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Entry {
    // stable and unique; made when the entry is made (from the store id, the path or the program name), kept in the file
    pub id: String,
    pub exe: String,
    pub name: String,
    pub genre: Genre,
    // "manual" (typed or chosen in the Games page), "voice" («добавь эту игру»), "scan" (found by the scan of the computer)
    pub source: String,
    // the id of the game in a store: "steam:22370", "epic:<AppName>", "gog:<id>" (empty for the rest)
    pub store_id: String,
    // the full path of the program (empty: only the name is known, the game cannot be started from here)
    pub path: String,
    // the path was named by the user or by a manifest of a store (not guessed from the files of a folder)
    pub verified: bool,
    // extra arguments of the launch (no shell: each one is quoted by itself)
    pub args: Vec<String>,
    pub added: u64,
    pub last_launched: u64,
    pub launches: u32,
}

impl Entry {
    pub fn basic(exe: &str, name: &str, genre: Genre, source: &str) -> Entry {
        Entry { exe: exe.to_string(), name: name.to_string(), genre, source: source.to_string(), ..Default::default() }
    }

    // where it came from, for the filter and the label: steam / epic / gog / manual / voice / scan
    pub fn origin(&self) -> &str {
        match self.store_id.split_once(':') {
            Some((s @ ("steam" | "epic" | "gog"), _)) => s,
            _ => &self.source,
        }
    }
}

pub fn origin_label(origin: &str) -> &'static str {
    match origin {
        "steam" => "Steam",
        "epic" => "Epic Games",
        "gog" => "GOG",
        "voice" => "Голосом",
        "scan" => "Поиск",
        _ => "Вручную",
    }
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

pub fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// a path for comparing: one kind of slash, no quotes, no `\\?\`, no closing slash, small letters
pub fn normalize_path(p: &str) -> String {
    let mut s = p.trim().trim_matches('"').replace('/', "\\");
    if let Some(rest) = s.strip_prefix("\\\\?\\") {
        s = rest.to_string();
    }
    s.trim_end_matches('\\').to_lowercase()
}

// a stable id from what identifies the game: the id in a store, else the path, else the name of the program
pub fn id_for(store_id: &str, path: &str, exe: &str) -> String {
    let key = if !store_id.is_empty() {
        store_id.to_lowercase()
    } else if !path.trim().is_empty() {
        normalize_path(path)
    } else {
        exe.trim().to_lowercase()
    };
    // FNV-1a, 64 bit: the same text always gives the same id
    let mut h: u64 = 0xcbf29ce484222325;
    for b in key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("g{:016x}", h)
}

// an id that no other entry of the list has
fn unique_id(games: &[Entry], want: &str) -> String {
    if !games.iter().any(|g| g.id == want) {
        return want.to_string();
    }
    (2..)
        .map(|n| format!("{}-{}", want, n))
        .find(|c| !games.iter().any(|g| g.id == *c))
        .unwrap_or_else(|| want.to_string())
}

pub fn parse_file(text: &str) -> File {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return File::default() };
    let mut f = File::default();
    let s = |g: &Value, k: &str| g.get(k).and_then(|x| x.as_str()).unwrap_or("").trim().to_string();
    if let Some(list) = v.get("games").and_then(|g| g.as_array()) {
        for g in list.iter().take(MAX_GAMES) {
            let exe = s(g, "exe").to_lowercase();
            if refuse_reason(&exe).is_some() {
                continue;
            }
            let path = s(g, "path").chars().take(520).collect::<String>();
            let store_id = s(g, "store_id");
            // two entries of one program are one only when nothing tells them apart (the same store id, or the same path)
            if f.games.iter().any(|e| (!store_id.is_empty() && e.store_id == store_id) || (!path.is_empty() && !e.path.is_empty() && normalize_path(&e.path) == normalize_path(&path)) || (e.exe == exe && e.path.is_empty() && path.is_empty() && e.store_id.is_empty() && store_id.is_empty())) {
                continue;
            }
            let name = s(g, "name").chars().take(80).collect::<String>();
            let genre = g.get("genre").and_then(|x| x.as_str()).and_then(Genre::from_name).unwrap_or_default();
            let source = match g.get("source").and_then(|x| x.as_str()) {
                Some(x @ ("manual" | "voice" | "scan")) => x.to_string(),
                _ => "manual".to_string(),
            };
            let id = {
                let own = s(g, "id");
                let want = if own.is_empty() || own.len() > 40 || !own.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') { id_for(&store_id, &path, &exe) } else { own };
                unique_id(&f.games, &want)
            };
            let args = g.get("args").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).take(20).map(|x| x.chars().take(200).collect()).collect()).unwrap_or_default();
            f.games.push(Entry {
                id,
                name: if name.is_empty() { exe.clone() } else { name },
                exe,
                genre,
                source,
                store_id,
                path,
                verified: g.get("verified").and_then(|x| x.as_bool()).unwrap_or(false),
                args,
                added: g.get("added").and_then(|x| x.as_u64()).unwrap_or(0),
                last_launched: g.get("last_launched").and_then(|x| x.as_u64()).unwrap_or(0),
                launches: g.get("launches").and_then(|x| x.as_u64()).unwrap_or(0).min(u32::MAX as u64) as u32,
            });
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
                let id = unique_id(&f.games, &id_for("", "", &exe));
                f.games.push(Entry { id, name: exe.clone(), exe, genre, source: "voice".into(), ..Default::default() });
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
    let games: Vec<Value> = f
        .games
        .iter()
        .map(|e| json!({ "id": e.id, "exe": e.exe, "name": e.name, "genre": e.genre.name(), "source": e.source, "store_id": e.store_id, "path": e.path, "verified": e.verified, "args": e.args, "added": e.added, "last_launched": e.last_launched, "launches": e.launches }))
        .collect();
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

pub fn get(id: &str) -> Option<Entry> {
    read().games.into_iter().find(|g| g.id == id)
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
    // it was there: the kind (and the name, and the path) were updated when the new entry carried them
    Known,
}

// the entry of the list that is the same game as `e` (None: a new game)
fn find_same(games: &[Entry], e: &Entry) -> Option<usize> {
    let path = normalize_path(&e.path);
    games.iter().position(|g| {
        (!e.store_id.is_empty() && g.store_id == e.store_id)
            || (!path.is_empty() && !g.path.is_empty() && normalize_path(&g.path) == path)
            // a game that has no path yet (added by voice, an older entry) and the same program with a path: the same game
            || (g.exe == e.exe && (g.path.is_empty() || path.is_empty()) && (g.store_id.is_empty() || e.store_id.is_empty()))
    })
}

// the entry that `e` would be merged into (None: it would be a new game)
pub fn find_duplicate(e: &Entry) -> Option<Entry> {
    let mut e = e.clone();
    e.exe = e.exe.trim().to_lowercase();
    let f = read();
    find_same(&f.games, &e).map(|i| f.games[i].clone())
}

// puts `e` into the list `f`; the way it was done and the id of the entry
fn put(f: &mut File, mut e: Entry, genre_named: bool) -> Result<(Added, String), String> {
    e.exe = e.exe.trim().to_lowercase();
    e.name = e.name.trim().chars().take(80).collect();
    e.path = e.path.trim().trim_matches('"').chars().take(520).collect();
    if let Some(why) = refuse_reason(&e.exe) {
        return Err(why);
    }
    if let Some(i) = find_same(&f.games, &e) {
        let old = &mut f.games[i];
        if genre_named {
            old.genre = e.genre;
        }
        let placeholder = old.name == old.exe || old.name.is_empty();
        if !e.name.is_empty() && e.name != e.exe && ((old.source != "voice" && e.source != "scan") || placeholder) {
            old.name = e.name.clone();
        }
        // a path chosen by the user wins; a scan fills a missing path or renews a path of an earlier scan, and never replaces the user's
        let user_path_stays = old.verified && old.source == "manual" && e.source == "scan";
        if !e.path.is_empty() && !user_path_stays && (old.path.is_empty() || e.source != "scan" || old.source == "scan" || !old.verified) {
            old.path = e.path.clone();
            old.verified = e.verified;
        }
        if old.store_id.is_empty() && !e.store_id.is_empty() {
            old.store_id = e.store_id.clone();
        }
        return Ok((Added::Known, old.id.clone()));
    }
    if f.games.len() >= MAX_GAMES {
        return Err(format!("в библиотеке уже {} игр: уберите ненужные", MAX_GAMES));
    }
    if e.name.is_empty() {
        e.name = e.exe.clone();
    }
    let want = if e.id.is_empty() { id_for(&e.store_id, &e.path, &e.exe) } else { e.id.clone() };
    e.id = unique_id(&f.games, &want);
    if e.added == 0 {
        e.added = now_secs();
    }
    let id = e.id.clone();
    f.games.push(e);
    Ok((Added::New, id))
}

// adds a game. `genre_named`: the user named the kind (an old entry is changed), else the kind of a new entry is the one given
pub fn add_entry(e: Entry, genre_named: bool) -> Result<(Added, String), String> {
    let _g = WRITE.lock();
    let mut f = read();
    let r = put(&mut f, e, genre_named)?;
    write(&f);
    Ok(r)
}

pub fn add(e: Entry, genre_named: bool) -> Result<Added, String> {
    add_entry(e, genre_named).map(|r| r.0)
}

// many games with one write of the file (the scan); how many were new
pub fn add_many(list: Vec<Entry>) -> usize {
    let _g = WRITE.lock();
    let mut f = read();
    let mut n = 0;
    for e in list {
        if let Ok((Added::New, _)) = put(&mut f, e, false) {
            n += 1;
        }
    }
    write(&f);
    n
}

// removes the entries of a program by its name (the voice phrase «забудь эту игру»); only the entries
pub fn remove(exe: &str) -> bool {
    let _g = WRITE.lock();
    let mut f = read();
    let before = f.games.len();
    let exe = exe.trim().to_lowercase();
    f.games.retain(|g| g.exe != exe);
    let changed = f.games.len() != before;
    if changed {
        write(&f);
    }
    changed
}

// removes ONE entry of the library by its id. The files of the game are not looked at, let alone touched.
pub fn remove_id(id: &str) -> bool {
    let _g = WRITE.lock();
    let mut f = read();
    let before = f.games.len();
    f.games.retain(|g| g.id != id);
    let changed = f.games.len() != before;
    if changed {
        write(&f);
    }
    changed
}

pub fn set_genre_id(id: &str, genre: Genre) -> bool {
    let _g = WRITE.lock();
    let mut f = read();
    let Some(g) = f.games.iter_mut().find(|g| g.id == id) else { return false };
    g.genre = genre;
    write(&f);
    true
}

// a new path of the program of a game (the user chose the file again). The id stays. An error when another entry already has that path.
pub fn set_path(id: &str, exe: &str, path: &str) -> Result<(), String> {
    let _g = WRITE.lock();
    let mut f = read();
    let norm = normalize_path(path);
    if f.games.iter().any(|g| g.id != id && !g.path.is_empty() && normalize_path(&g.path) == norm) {
        return Err("Этот файл уже записан у другой игры библиотеки.".into());
    }
    if let Some(why) = refuse_reason(exe) {
        return Err(why);
    }
    let Some(g) = f.games.iter_mut().find(|g| g.id == id) else { return Err("Игры нет в библиотеке.".into()) };
    g.exe = exe.to_lowercase();
    g.path = path.trim().trim_matches('"').to_string();
    // the user named the file: a later scan never replaces it
    g.verified = true;
    g.source = "manual".into();
    write(&f);
    Ok(())
}

// a launch happened: the time and the count (the list of recent games is made of them)
pub fn touch_launch(id: &str) {
    let _g = WRITE.lock();
    let mut f = read();
    if let Some(g) = f.games.iter_mut().find(|g| g.id == id) {
        g.last_launched = now_secs();
        g.launches = g.launches.saturating_add(1);
        write(&f);
    }
}

// ---------------------------------------------------------------------------------------------
// the catalog: search, filters, sorting, pages (the window «Все игры» asks for one page at a time)
// ---------------------------------------------------------------------------------------------
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sort {
    Name,
    Added,
    Recent,
}

impl Sort {
    pub fn from_name(n: &str) -> Sort {
        match n {
            "added" => Sort::Added,
            "recent" => Sort::Recent,
            _ => Sort::Name,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Query {
    pub text: String,
    pub genre: Option<Genre>,
    // "" = every origin
    pub origin: String,
    pub sort: Sort,
    pub offset: usize,
    pub limit: usize,
}

impl Default for Query {
    fn default() -> Self {
        Query { text: String::new(), genre: None, origin: String::new(), sort: Sort::Name, offset: 0, limit: 60 }
    }
}

// (how many entries match, the page asked for)
pub fn query(list: &[Entry], q: &Query) -> (usize, Vec<Entry>) {
    let terms: Vec<String> = q.text.to_lowercase().split_whitespace().map(|t| t.to_string()).collect();
    let mut hits: Vec<(usize, &Entry)> = list
        .iter()
        .enumerate()
        .filter(|(_, e)| q.genre.map(|g| e.genre == g).unwrap_or(true) && (q.origin.is_empty() || e.origin() == q.origin))
        .filter(|(_, e)| {
            let name = e.name.to_lowercase();
            terms.iter().all(|t| name.contains(t.as_str()) || e.exe.contains(t.as_str()))
        })
        .collect();
    match q.sort {
        Sort::Name => hits.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()).then(a.0.cmp(&b.0))),
        Sort::Added => hits.sort_by(|a, b| b.1.added.cmp(&a.1.added).then(b.0.cmp(&a.0))),
        Sort::Recent => hits.sort_by(|a, b| b.1.last_launched.cmp(&a.1.last_launched).then(b.1.added.cmp(&a.1.added)).then(b.0.cmp(&a.0))),
    }
    let total = hits.len();
    let page = hits.into_iter().skip(q.offset).take(q.limit.min(500)).map(|(_, e)| e.clone()).collect();
    (total, page)
}

#[cfg(test)]
pub fn test_reset() {
    TEST_FILE.with(|t| *t.borrow_mut() = File::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(exe: &str, genre: Genre, source: &str) -> Entry {
        Entry::basic(exe, exe, genre, source)
    }

    fn with_path(exe: &str, name: &str, path: &str) -> Entry {
        Entry { path: path.into(), verified: true, ..Entry::basic(exe, name, Genre::Shooter, "manual") }
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
        let id = load()[0].id.clone();
        assert!(set_genre_id(&id, Genre::Action) && load()[0].genre == Genre::Action);
        assert!(!set_genre_id("nope", Genre::Action));
        // refused
        for bad in ["valorant.exe", "chrome.exe", "game", "a\\\\b.exe", ""] {
            assert!(add(e(bad, Genre::Shooter, "manual"), false).is_err(), "{}", bad);
        }
        assert_eq!(load().len(), 1);
        assert!(remove("GAME.exe") && load().is_empty());
        assert!(!remove("game.exe"));
    }

    #[test]
    fn the_library_holds_a_thousand_games_and_not_one_more() {
        test_reset();
        for i in 0..MAX_GAMES {
            assert_eq!(add(e(&format!("g{}.exe", i), Genre::Shooter, "scan"), false), Ok(Added::New));
        }
        assert!(add(e("one-more.exe", Genre::Shooter, "scan"), false).is_err());
        test_reset();
    }

    #[test]
    fn the_file_round_trips_and_the_older_form_and_a_broken_file_are_read() {
        let f = File {
            games: vec![Entry { id: "gabc".into(), exe: "a.exe".into(), name: "Игра А".into(), genre: Genre::Action, source: "scan".into(), store_id: "steam:42".into(), path: "D:\\Игры\\Игра А\\a.exe".into(), verified: true, args: vec!["-windowed".into()], added: 100, last_launched: 200, launches: 3 }],
            options: Options { overlay: true, sound: false },
        };
        assert_eq!(parse_file(&to_json(&f)), f);
        // the older form
        let old = parse_file(r#"{"exes":["x.exe","chrome.exe","y.exe"],"genres":{"y.exe":"racing"}}"#);
        assert_eq!(old.games.iter().map(|g| (g.exe.as_str(), g.genre)).collect::<Vec<_>>(), vec![("x.exe", Genre::Shooter), ("y.exe", Genre::Racing)]);
        assert!(old.games.iter().all(|g| g.id.starts_with('g') && g.source == "voice"));
        // the ids of an older file are the same every time it is read
        assert_eq!(parse_file(r#"{"exes":["x.exe"]}"#).games[0].id, old.games[0].id);
        assert_eq!(parse_file("не json"), File::default());
        // a refused program in the file is dropped, a repeated one stays once, unknown source becomes manual
        let g = parse_file(r#"{"games":[{"exe":"valorant.exe"},{"exe":"z.exe","source":"???"},{"exe":"z.exe"}]}"#);
        assert_eq!(g.games.len(), 1);
        assert_eq!(g.games[0].source, "manual");
    }

    // the file of the previous version (before ids and paths): every entry is read, gets an id, and a second reading gives the same ids
    #[test]
    fn a_file_of_the_previous_version_keeps_every_entry() {
        let old = r#"{"games":[{"exe":"afop.exe","name":"Avatar: Frontiers of Pandora","genre":"shooter","source":"scan"},{"exe":"witcher3.exe","name":"Witcher 3","genre":"action","source":"scan"},{"exe":"mygame.exe","name":"mygame.exe","genre":"racing","source":"voice"}],"options":{"overlay":true,"sound":true}}"#;
        let f = parse_file(old);
        assert_eq!(f.games.len(), 3);
        assert_eq!(f.games[1].name, "Witcher 3");
        assert_eq!(f.games[2].genre, Genre::Racing);
        assert!(f.games.iter().all(|g| g.path.is_empty() && g.added == 0 && !g.id.is_empty()));
        assert!(f.options.overlay && f.options.sound);
        let again = parse_file(&to_json(&f));
        assert_eq!(again, f, "writing and reading again changes nothing");
        assert_eq!(parse_file(old).games.iter().map(|g| g.id.clone()).collect::<Vec<_>>(), f.games.iter().map(|g| g.id.clone()).collect::<Vec<_>>());
    }

    #[test]
    fn a_broken_or_repaired_file_is_kept_beside_the_new_one_and_a_clean_one_is_not() {
        let dir = std::env::temp_dir().join(format!("jarvis-gl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game_list.json");
        let clean = to_json(&File { games: vec![Entry { id: "g1".into(), ..Entry::basic("a.exe", "A", Genre::Racing, "manual") }], options: Options::default() });
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

    #[test]
    fn ids_are_stable_unique_and_do_not_depend_on_the_name_only() {
        test_reset();
        // the same path in other writing: the same game, the same id
        let (a, id1) = add_entry(with_path("game.exe", "Game", "D:\\Games\\One\\game.exe"), false).unwrap();
        assert_eq!(a, Added::New);
        let (b, id2) = add_entry(with_path("game.exe", "Game", "d:/games/one/GAME.EXE"), false).unwrap();
        assert_eq!((b, id2.clone()), (Added::Known, id1.clone()));
        // the same program name and the same title in another folder: another game
        let (c, id3) = add_entry(with_path("game.exe", "Game", "E:\\Other\\game.exe"), false).unwrap();
        assert_eq!(c, Added::New);
        assert_ne!(id3, id1);
        assert_eq!(load().len(), 2);
        // the id does not change when the path changes
        set_path(&id1, "game2.exe", "D:\\Games\\One\\game2.exe").unwrap();
        assert_eq!(get(&id1).unwrap().exe, "game2.exe");
        assert_eq!(get(&id1).unwrap().id, id1);
        // a path of another entry cannot be taken
        assert!(set_path(&id1, "game.exe", "E:\\Other\\game.exe").is_err());
        // an id that does not exist
        assert!(set_path("nope", "x.exe", "C:\\x.exe").is_err());
        test_reset();
    }

    #[test]
    fn a_game_found_twice_by_the_scan_or_added_by_voice_and_then_by_path_is_one_entry() {
        test_reset();
        // by voice (no path), then by hand with a path: one entry that now has the path
        add(Entry::basic("mygame.exe", "mygame.exe", Genre::Racing, "voice"), false).unwrap();
        let (r, _) = add_entry(Entry { path: "D:\\G\\mygame.exe".into(), verified: true, ..Entry::basic("mygame.exe", "My Game", Genre::Racing, "manual") }, false).unwrap();
        assert_eq!(r, Added::Known);
        assert_eq!(load().len(), 1);
        assert_eq!((load()[0].name.as_str(), load()[0].path.as_str(), load()[0].source.as_str()), ("My Game", "D:\\G\\mygame.exe", "voice"));
        // a scan twice with the same store id: one entry; a path chosen by the user is not replaced by the scan
        let scan = |path: &str| Entry { store_id: "steam:22370".into(), path: path.into(), verified: false, ..Entry::basic("fallout3.exe", "Fallout 3", Genre::Action, "scan") };
        assert_eq!(add_many(vec![scan("D:\\S\\fallout3.exe")]), 1);
        assert_eq!(add_many(vec![scan("D:\\S\\fallout3.exe")]), 0);
        assert_eq!(load().len(), 2);
        let id = load().iter().find(|g| g.store_id == "steam:22370").unwrap().id.clone();
        set_path(&id, "fallout3.exe", "F:\\Mine\\fallout3.exe").unwrap();
        add_many(vec![scan("D:\\S\\fallout3.exe")]);
        let g = get(&id).unwrap();
        assert_eq!(g.path, "F:\\Mine\\fallout3.exe");
        // two different games with similar titles are two
        assert_eq!(add_many(vec![Entry { store_id: "steam:1".into(), ..Entry::basic("fallout3goty.exe", "Fallout 3 GOTY", Genre::Action, "scan") }]), 1);
        assert_eq!(load().len(), 3);
        test_reset();
    }

    #[test]
    fn only_the_entry_is_removed_and_a_wrong_id_changes_nothing() {
        test_reset();
        let (_, a) = add_entry(with_path("a.exe", "A", "D:\\A\\a.exe"), false).unwrap();
        let (_, b) = add_entry(with_path("b.exe", "B", "D:\\B\\b.exe"), false).unwrap();
        let before = load();
        assert!(!remove_id("nope"));
        assert_eq!(load(), before, "a wrong id: nothing changed");
        assert!(remove_id(&a));
        assert_eq!(load().iter().map(|g| g.id.clone()).collect::<Vec<_>>(), vec![b]);
        test_reset();
    }

    #[test]
    fn the_catalog_searches_filters_sorts_and_pages_and_never_shows_a_removed_game() {
        test_reset();
        // 200 games, three kinds, three origins
        let mut list = Vec::new();
        for i in 0..200 {
            let genre = [Genre::Shooter, Genre::Racing, Genre::Action][i % 3];
            let store = ["steam:", "epic:", ""][i % 3];
            list.push(Entry { id: format!("g{}", i), store_id: if store.is_empty() { String::new() } else { format!("{}{}", store, i) }, added: 1000 + i as u64, last_launched: if i == 7 { 5000 } else { 0 }, ..Entry::basic(&format!("game{}.exe", i), &format!("Игра номер {}", i), genre, if store.is_empty() { "manual" } else { "scan" }) });
        }
        let all = |q: &Query| query(&list, q);
        let (n, page) = all(&Query { limit: 60, ..Default::default() });
        assert_eq!((n, page.len()), (200, 60));
        // search: a part of the name in Russian, any case, several words; and the name of the program
        assert_eq!(all(&Query { text: "ИГРА НОМЕР 19".into(), ..Default::default() }).0, 12); // 19, 119, 190..199
        assert_eq!(all(&Query { text: "game42".into(), ..Default::default() }).1[0].id, "g42");
        assert_eq!(all(&Query { text: "19 игра".into(), ..Default::default() }).0, 12, "words in any order");
        assert_eq!(all(&Query { text: "нет такой".into(), ..Default::default() }).0, 0);
        // filters
        assert_eq!(all(&Query { genre: Some(Genre::Racing), ..Default::default() }).0, 67);
        assert_eq!(all(&Query { origin: "epic".into(), ..Default::default() }).0, 67);
        assert_eq!(all(&Query { origin: "manual".into(), genre: Some(Genre::Action), ..Default::default() }).0, 66);
        // sorting
        assert_eq!(all(&Query { sort: Sort::Added, limit: 1, ..Default::default() }).1[0].id, "g199");
        assert_eq!(all(&Query { sort: Sort::Recent, limit: 2, ..Default::default() }).1[0].id, "g7", "the launched game is first");
        let names: Vec<String> = all(&Query { sort: Sort::Name, limit: 500, ..Default::default() }).1.iter().map(|e| e.name.to_lowercase()).collect();
        assert!(names.windows(2).all(|w| w[0] <= w[1]));
        // pages do not overlap and cover everything
        let mut seen = std::collections::HashSet::new();
        for off in (0..200).step_by(60) {
            for e in all(&Query { offset: off, limit: 60, sort: Sort::Added, ..Default::default() }).1 {
                assert!(seen.insert(e.id));
            }
        }
        assert_eq!(seen.len(), 200);
        // a removed game is in no result
        for e in list.iter().take(200) {
            add_entry(e.clone(), false).unwrap();
        }
        assert_eq!(load().len(), 200);
        assert!(remove_id("g42"));
        let now = load();
        assert_eq!(query(&now, &Query { text: "game42".into(), ..Default::default() }).0, 0);
        assert_eq!(query(&now, &Query::default()).0, 199);
        assert!(!query(&now, &Query { limit: 500, ..Default::default() }).1.iter().any(|e| e.id == "g42"));
        test_reset();
    }
}
