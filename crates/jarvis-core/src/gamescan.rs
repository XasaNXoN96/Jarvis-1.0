//! The scan of the computer for installed games (Games page → «Найти игры на этом компьютере»).
//!
//! It READS the lists of the game stores — Steam (`libraryfolders.vdf`, `appmanifest_*.acf`), Epic Games (`*.item` manifests),
//! GOG (the registry) — and looks for the main program of every game in its folder. Nothing is installed, started or changed.
//! A game that is a known online game, or whose folder holds an anti-cheat (EasyAntiCheat, BattlEye, Vanguard, FACEIT …), is NOT
//! added by the scan: it is shown with the reason. The user may add such a game by hand if it is a single-player one; the guard of the
//! game mode (it does not run while an anti-cheat program is running) stays.

use crate::gamelist::{self, Entry};
use crate::gamemode::{refuse_reason, Genre};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub name: String,
    // "Steam" | "Epic" | "GOG"
    pub store: String,
    pub exe: String,
    pub dir: PathBuf,
    pub genre: Genre,
    // why the scan did not add it (an online game, an anti-cheat in the folder, no program found)
    pub skipped: Option<String>,
}

// ---------------------------------------------------------------------------------------------
// parsers (plain, tested)
// ---------------------------------------------------------------------------------------------
// the value of a line `"key"		"value"` of a Valve text file (VDF / ACF); backslashes are doubled in it
fn vdf_value(line: &str, key: &str) -> Option<String> {
    let t = line.trim();
    let rest = t.strip_prefix('"')?.strip_prefix(key)?.strip_prefix('"')?;
    let rest = rest.trim_start().strip_prefix('"')?;
    let end = rest.rfind('"')?;
    Some(rest[..end].replace("\\\\", "\\"))
}

// the folders of the Steam libraries from libraryfolders.vdf
pub fn parse_library_folders(text: &str) -> Vec<PathBuf> {
    text.lines().filter_map(|l| vdf_value(l, "path")).map(PathBuf::from).collect()
}

// (name, installdir) of one game from appmanifest_<id>.acf
pub fn parse_acf(text: &str) -> Option<(String, String)> {
    let mut name = None;
    let mut dir = None;
    for l in text.lines() {
        if name.is_none() {
            name = vdf_value(l, "name");
        }
        if dir.is_none() {
            dir = vdf_value(l, "installdir");
        }
    }
    match (name, dir) {
        (Some(n), Some(d)) if !n.is_empty() && !d.is_empty() => Some((n, d)),
        _ => None,
    }
}

// (name, install folder, launch program) of one Epic manifest
pub fn parse_epic_item(text: &str) -> Option<(String, PathBuf, Option<String>)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let name = v.get("DisplayName")?.as_str()?.to_string();
    let dir = v.get("InstallLocation")?.as_str()?;
    let exe = v.get("LaunchExecutable").and_then(|e| e.as_str()).and_then(|e| e.rsplit(['\\', '/']).next()).map(|e| e.to_lowercase()).filter(|e| !e.is_empty());
    if name.is_empty() || dir.is_empty() {
        return None;
    }
    Some((name, PathBuf::from(dir), exe))
}

// programs of a game folder that are not the game
const NOT_THE_GAME: &[&str] = &["unins", "uninst", "crash", "report", "redist", "vcredist", "vc_redist", "dxsetup", "dotnet", "directx", "setup", "installer", "prereq", "easyanticheat", "beservice", "updater", "support", "helper", "ue4prereq", "launcher"];

// the main program of a game: the biggest .exe of the folder (to a depth of four) that is not an installer, a crash reporter …
pub fn find_main_exe(dir: &Path, depth: u32) -> Option<String> {
    let mut best: Option<(u64, String)> = None;
    fn walk(dir: &Path, depth: u32, best: &mut Option<(u64, String)>, seen: &mut usize) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            *seen += 1;
            if *seen > 20_000 {
                return;
            }
            let p = e.path();
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                if depth > 0 {
                    walk(&p, depth - 1, best, seen);
                }
            } else if let Some(name) = p.file_name().and_then(|n| n.to_str()).map(|n| n.to_lowercase()) {
                if name.ends_with(".exe") && !NOT_THE_GAME.iter().any(|w| name.contains(w)) {
                    let size = e.metadata().map(|m| m.len()).unwrap_or(0);
                    if best.as_ref().map(|(s, _)| size > *s).unwrap_or(true) {
                        *best = Some((size, name));
                    }
                }
            }
        }
    }
    let mut seen = 0;
    walk(dir, depth, &mut best, &mut seen);
    best.map(|(_, n)| n)
}

// an anti-cheat in the game folder (a folder or a file of the usual names), to a depth of three
pub fn anticheat_in(dir: &Path) -> Option<String> {
    const MARKERS: &[&str] = &["easyanticheat", "easyanticheat_eos", "battleye", "beclient", "beservice", "vanguard", "faceit", "punkbuster", "xigncode", "gameguard", "denuvo-anti-cheat"];
    fn walk(dir: &Path, depth: u32, seen: &mut usize) -> Option<String> {
        let rd = std::fs::read_dir(dir).ok()?;
        for e in rd.flatten() {
            *seen += 1;
            if *seen > 8_000 {
                return None;
            }
            let name = e.file_name().to_string_lossy().to_lowercase();
            if let Some(m) = MARKERS.iter().find(|m| name.starts_with(**m)) {
                return Some((*m).to_string());
            }
            if depth > 0 && e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false) {
                if let Some(f) = walk(&e.path(), depth - 1, seen) {
                    return Some(f);
                }
            }
        }
        None
    }
    let mut seen = 0;
    walk(dir, 3, &mut seen)
}

// well-known single-player games by the name of their program: (title, kind). A hint for the list (the name and the kind), not a promise
// that the game works: the names of the programs are the usual ones of the Steam / Epic versions and may differ in another edition.
pub fn known_game(exe: &str) -> Option<(&'static str, Genre)> {
    const KNOWN: &[(&str, &str, Genre)] = &[
        ("iw3sp.exe", "Call of Duty 4: Modern Warfare (кампания)", Genre::Shooter),
        ("iw4sp.exe", "Call of Duty: Modern Warfare 2 (кампания)", Genre::Shooter),
        ("iw5sp.exe", "Call of Duty: Modern Warfare 3 (кампания)", Genre::Shooter),
        ("farcry3.exe", "Far Cry 3", Genre::Shooter),
        ("farcry4.exe", "Far Cry 4", Genre::Shooter),
        ("farcry5.exe", "Far Cry 5", Genre::Shooter),
        ("crysis.exe", "Crysis", Genre::Shooter),
        ("metro2033.exe", "Metro 2033", Genre::Shooter),
        ("bioshockinfinite.exe", "BioShock Infinite", Genre::Shooter),
        ("skyrim.exe", "The Elder Scrolls V: Skyrim", Genre::Action),
        ("tesv.exe", "The Elder Scrolls V: Skyrim", Genre::Action),
        ("skyrimse.exe", "Skyrim Special Edition", Genre::Action),
        ("fallout3.exe", "Fallout 3", Genre::Action),
        ("falloutnv.exe", "Fallout: New Vegas", Genre::Action),
        ("fallout4.exe", "Fallout 4", Genre::Action),
        ("witcher3.exe", "The Witcher 3: Wild Hunt", Genre::Action),
        ("cyberpunk2077.exe", "Cyberpunk 2077", Genre::Action),
        ("rdr2.exe", "Red Dead Redemption 2", Genre::Action),
        ("gta5.exe", "Grand Theft Auto V (сюжет)", Genre::Action),
        ("gtaiv.exe", "Grand Theft Auto IV", Genre::Action),
        ("gtasa.exe", "Grand Theft Auto: San Andreas", Genre::Action),
        ("eldenring.exe", "Elden Ring", Genre::Action),
        ("sekiro.exe", "Sekiro", Genre::Action),
        ("darksoulsiii.exe", "Dark Souls III", Genre::Action),
        ("tombraider.exe", "Tomb Raider", Genre::Action),
        ("sottr.exe", "Shadow of the Tomb Raider", Genre::Action),
        ("rottr.exe", "Rise of the Tomb Raider", Genre::Action),
        ("godofwar.exe", "God of War", Genre::Action),
        ("acodyssey.exe", "Assassin's Creed Odyssey", Genre::Action),
        ("acorigins.exe", "Assassin's Creed Origins", Genre::Action),
        ("acvalhalla.exe", "Assassin's Creed Valhalla", Genre::Action),
        ("re2.exe", "Resident Evil 2", Genre::Action),
        ("re3.exe", "Resident Evil 3", Genre::Action),
        ("re4.exe", "Resident Evil 4", Genre::Action),
        ("re8.exe", "Resident Evil Village", Genre::Action),
        ("forzahorizon4.exe", "Forza Horizon 4", Genre::Racing),
        ("forzahorizon5.exe", "Forza Horizon 5", Genre::Racing),
        ("forzamotorsport7.exe", "Forza Motorsport 7", Genre::Racing),
        ("nfs13.exe", "Need for Speed: Most Wanted", Genre::Racing),
        ("nfsc.exe", "Need for Speed: Carbon", Genre::Racing),
        ("speed.exe", "Need for Speed: Most Wanted (2005)", Genre::Racing),
        ("needforspeedheat.exe", "Need for Speed Heat", Genre::Racing),
        ("dirtrally2.exe", "DiRT Rally 2.0", Genre::Racing),
        ("assettocorsa.exe", "Assetto Corsa", Genre::Racing),
        ("trackmania.exe", "Trackmania", Genre::Racing),
    ];
    KNOWN.iter().find(|(e, _, _)| *e == exe).map(|(_, t, g)| (*t, *g))
}

// the kind of a game by its name (a guess; the user changes it in the list)
pub fn guess_genre(name: &str) -> Genre {
    let n = name.to_lowercase();
    const RACING: &[&str] = &["forza", "need for speed", "gran turismo", "dirt", "grid", "f1 ", "wrc", "assetto", "burnout", "trackmania", "race", "racing", "rally", "drift", "truck", "speed", "midnight club", "flatout", "hot wheels", "motogp", "ride ", "horizon chase", "art of rally"];
    const ACTION: &[&str] = &["assassin", "witcher", "skyrim", "elder scrolls", "fallout", "tomb raider", "god of war", "dark souls", "elden ring", "sekiro", "batman", "spider-man", "red dead", "grand theft", "gta", "cyberpunk", "resident evil", "devil may cry", "hollow knight", "zelda", "prince of persia", "dishonored", "mass effect", "dragon age", "uncharted", "horizon", "nier", "control", "alan wake", "hitman", "mafia", "yakuza", "lego", "mortal kombat", "sleeping dogs", "far cry", "watch dogs", "kingdom come"];
    if RACING.iter().any(|w| n.contains(w)) {
        Genre::Racing
    } else if ACTION.iter().any(|w| n.contains(w)) {
        Genre::Action
    } else {
        Genre::Shooter
    }
}

// one game of a store -> the result of the scan (with the reason when it is not to be added)
pub fn judge(name: &str, store: &str, dir: &Path, known_exe: Option<String>) -> Found {
    let exe = known_exe.or_else(|| find_main_exe(dir, 4));
    let genre = exe.as_deref().and_then(known_game).map(|k| k.1).unwrap_or_else(|| guess_genre(name));
    let skipped = match &exe {
        None => Some("не нашёл программу игры в её папке".to_string()),
        Some(e) => {
            if let Some(why) = refuse_reason(e) {
                Some(why)
            } else if let Some(ac) = anticheat_in(dir) {
                Some(format!("в папке игры защита от читов ({}): скорее всего игра сетевая. Если это одиночная игра, добавьте её вручную", ac))
            } else {
                None
            }
        }
    };
    Found { name: name.to_string(), store: store.to_string(), exe: exe.unwrap_or_default(), dir: dir.to_path_buf(), genre, skipped }
}

// ---------------------------------------------------------------------------------------------
// the stores
// ---------------------------------------------------------------------------------------------
// the libraries of Steam under a Steam folder (the folder itself and the ones in libraryfolders.vdf)
pub fn steam_games(steam_root: &Path) -> Vec<Found> {
    let mut libs = vec![steam_root.to_path_buf()];
    if let Ok(t) = std::fs::read_to_string(steam_root.join("steamapps").join("libraryfolders.vdf")) {
        for l in parse_library_folders(&t) {
            if !libs.contains(&l) {
                libs.push(l);
            }
        }
    }
    let mut out = Vec::new();
    for lib in libs {
        let apps = lib.join("steamapps");
        let Ok(rd) = std::fs::read_dir(&apps) else { continue };
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            if !(fname.starts_with("appmanifest_") && fname.ends_with(".acf")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
            let Some((name, installdir)) = parse_acf(&text) else { continue };
            // Steam's own tools (runtimes, redistributables) are not games
            let low = name.to_lowercase();
            if low.starts_with("steamworks") || low.contains("redistributable") || low.starts_with("proton") || low.contains("steam linux runtime") || low.starts_with("steam controller") {
                continue;
            }
            let dir = apps.join("common").join(&installdir);
            if installdir.contains("..") || !dir.is_dir() {
                continue;
            }
            out.push(judge(&name, "Steam", &dir, None));
        }
    }
    out
}

pub fn epic_games(manifests: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(manifests) else { return out };
    for e in rd.flatten() {
        if e.path().extension().and_then(|x| x.to_str()) != Some("item") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(e.path()) else { continue };
        let Some((name, dir, exe)) = parse_epic_item(&text) else { continue };
        if !dir.is_dir() {
            continue;
        }
        out.push(judge(&name, "Epic", &dir, exe));
    }
    out
}

#[cfg(target_os = "windows")]
fn registry_string(key: winapi::shared::minwindef::HKEY, name: &str) -> Option<String> {
    use winapi::um::winreg::RegQueryValueExW;
    let wname: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    let mut kind: u32 = 0;
    let mut len: u32 = 0;
    unsafe {
        if RegQueryValueExW(key, wname.as_ptr(), std::ptr::null_mut(), &mut kind, std::ptr::null_mut(), &mut len) != 0 || len == 0 || len > 8192 {
            return None;
        }
        let mut buf = vec![0u16; (len as usize + 1) / 2 + 1];
        if RegQueryValueExW(key, wname.as_ptr(), std::ptr::null_mut(), &mut kind, buf.as_mut_ptr() as *mut u8, &mut len) != 0 {
            return None;
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..end]))
    }
}

// the games of GOG Galaxy: HKLM\SOFTWARE\WOW6432Node\GOG.com\Games\<id> (gameName, path, exe)
#[cfg(target_os = "windows")]
pub fn gog_games() -> Vec<Found> {
    use winapi::shared::minwindef::HKEY;
    use winapi::um::winnt::KEY_READ;
    use winapi::um::winreg::{RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, HKEY_LOCAL_MACHINE};
    let mut out = Vec::new();
    unsafe {
        let base: Vec<u16> = "SOFTWARE\\WOW6432Node\\GOG.com\\Games".encode_utf16().chain(std::iter::once(0)).collect();
        let mut root: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, base.as_ptr(), 0, KEY_READ, &mut root) != 0 {
            return out;
        }
        for i in 0..400u32 {
            let mut name = [0u16; 256];
            let mut len = name.len() as u32;
            if RegEnumKeyExW(root, i, name.as_mut_ptr(), &mut len, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) != 0 {
                break;
            }
            let sub = String::from_utf16_lossy(&name[..len as usize]);
            let subw: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
            let mut k: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(root, subw.as_ptr(), 0, KEY_READ, &mut k) != 0 {
                continue;
            }
            let (game, path, exe) = (registry_string(k, "gameName"), registry_string(k, "path"), registry_string(k, "exe"));
            RegCloseKey(k);
            if let (Some(game), Some(path)) = (game, path) {
                let dir = PathBuf::from(path);
                if dir.is_dir() {
                    let exe = exe.and_then(|e| e.rsplit(['\\', '/']).next().map(|x| x.to_lowercase())).filter(|e| e.ends_with(".exe"));
                    out.push(judge(&game, "GOG", &dir, exe));
                }
            }
        }
        RegCloseKey(root);
    }
    out
}

#[cfg(not(target_os = "windows"))]
pub fn gog_games() -> Vec<Found> {
    Vec::new()
}

// the Steam folder: the registry of the user, else the usual places
#[cfg(target_os = "windows")]
fn steam_root() -> Option<PathBuf> {
    use winapi::shared::minwindef::HKEY;
    use winapi::um::winnt::KEY_READ;
    use winapi::um::winreg::{RegCloseKey, RegOpenKeyExW, HKEY_CURRENT_USER};
    unsafe {
        let key: Vec<u16> = "Software\\Valve\\Steam".encode_utf16().chain(std::iter::once(0)).collect();
        let mut k: HKEY = std::ptr::null_mut();
        if RegOpenKeyExW(HKEY_CURRENT_USER, key.as_ptr(), 0, KEY_READ, &mut k) == 0 {
            let p = registry_string(k, "SteamPath");
            RegCloseKey(k);
            if let Some(p) = p {
                let pb = PathBuf::from(p.replace('/', "\\"));
                if pb.is_dir() {
                    return Some(pb);
                }
            }
        }
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(pf) = std::env::var(var) {
            let p = Path::new(&pf).join("Steam");
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn steam_root() -> Option<PathBuf> {
    None
}

// everything the stores know about, each game once (by program name)
pub fn scan_all() -> Vec<Found> {
    let mut all = Vec::new();
    if let Some(root) = steam_root() {
        all.extend(steam_games(&root));
    }
    if let Ok(pd) = std::env::var("ProgramData") {
        all.extend(epic_games(&Path::new(&pd).join("Epic").join("EpicGamesLauncher").join("Data").join("Manifests")));
    }
    all.extend(gog_games());
    let mut seen = std::collections::HashSet::new();
    all.retain(|f| f.exe.is_empty() || seen.insert(f.exe.clone()));
    all.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    all
}

// ---------------------------------------------------------------------------------------------
// the run in the background and its state
// ---------------------------------------------------------------------------------------------
#[derive(Default, Clone)]
struct State {
    running: bool,
    done: bool,
    found: Vec<Found>,
    added: usize,
}

static STATE: Lazy<Mutex<State>> = Lazy::new(|| Mutex::new(State::default()));

// adds the games of a scan that are fit; the number added (new ones)
pub fn add_found(found: &[Found]) -> usize {
    let mut n = 0;
    for f in found.iter().filter(|f| f.skipped.is_none() && !f.exe.is_empty()) {
        let e = Entry { exe: f.exe.clone(), name: f.name.clone(), genre: f.genre, source: "scan".into() };
        if let Ok(gamelist::Added::New) = gamelist::add(e, false) {
            n += 1;
        }
    }
    n
}

// a scan in the background: the games found are added at once (the automatic way), the rest is shown with the reasons
pub fn start() -> Result<(), String> {
    {
        let mut s = STATE.lock();
        if s.running {
            return Err("поиск уже идёт".into());
        }
        *s = State { running: true, ..Default::default() };
    }
    std::thread::spawn(|| {
        let found = scan_all();
        let added = add_found(&found);
        crate::gamemode::reload_list();
        crate::gamemode::note_event(&format!("поиск игр закончен: найдено {}, добавлено новых {}", found.len(), added));
        let mut s = STATE.lock();
        *s = State { running: false, done: true, found, added };
    });
    Ok(())
}

pub fn status_json() -> Value {
    let s = STATE.lock().clone();
    let items: Vec<Value> = s.found.iter().map(|f| json!({ "name": f.name, "store": f.store, "exe": f.exe, "genre": f.genre.name(), "skipped": f.skipped })).collect();
    json!({ "running": s.running, "done": s.done, "added": s.added, "found": items })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path, size: usize) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, vec![b'x'; size]).unwrap();
    }

    #[test]
    fn the_libraries_and_games_of_steam_are_read_from_its_text_files() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t\t\"label\"\t\t\"\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\Games\\\\SteamLibrary\"\n\t}\n}\n";
        assert_eq!(parse_library_folders(vdf), vec![PathBuf::from("C:\\Program Files (x86)\\Steam"), PathBuf::from("D:\\Games\\SteamLibrary")]);
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"22370\"\n\t\"name\"\t\t\"Fallout 3 - Game of the Year Edition\"\n\t\"installdir\"\t\t\"Fallout 3 goty\"\n}\n";
        assert_eq!(parse_acf(acf), Some(("Fallout 3 - Game of the Year Edition".to_string(), "Fallout 3 goty".to_string())));
        assert_eq!(parse_acf("\"AppState\"\n{\n}\n"), None);
        assert_eq!(parse_acf(""), None);
    }

    #[test]
    fn the_epic_manifest_gives_the_name_the_folder_and_the_program() {
        let item = r#"{"DisplayName":"Alan Wake","InstallLocation":"D:\\Epic\\AlanWake","LaunchExecutable":"Binaries\\AlanWake.exe","AppName":"x"}"#;
        assert_eq!(parse_epic_item(item), Some(("Alan Wake".to_string(), PathBuf::from("D:\\Epic\\AlanWake"), Some("alanwake.exe".to_string()))));
        assert_eq!(parse_epic_item("{}"), None);
        assert_eq!(parse_epic_item("не json"), None);
    }

    #[test]
    fn the_main_program_is_the_biggest_one_that_is_not_an_installer() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("unins000.exe"), 5000);
        touch(&d.path().join("CrashReporter.exe"), 6000);
        touch(&d.path().join("vc_redist.x64.exe"), 9000);
        touch(&d.path().join("bin").join("Game.exe"), 4000);
        touch(&d.path().join("bin").join("deep").join("Other.exe"), 1000);
        touch(&d.path().join("readme.txt"), 99999);
        assert_eq!(find_main_exe(d.path(), 4), Some("game.exe".to_string()));
        assert_eq!(find_main_exe(&d.path().join("missing"), 4), None);
    }

    #[test]
    fn an_anti_cheat_in_the_folder_is_seen() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("game.exe"), 10);
        assert_eq!(anticheat_in(d.path()), None);
        touch(&d.path().join("EasyAntiCheat").join("x.dll"), 10);
        assert_eq!(anticheat_in(d.path()), Some("easyanticheat".to_string()));
        let d2 = tempfile::tempdir().unwrap();
        touch(&d2.path().join("Binaries").join("BattlEye").join("BEClient.dll"), 10);
        assert_eq!(anticheat_in(d2.path()), Some("battleye".to_string()));
    }

    #[test]
    fn well_known_games_have_their_kind_and_title_and_the_table_has_no_refused_programs() {
        assert_eq!(known_game("forzahorizon5.exe").unwrap().1, Genre::Racing);
        assert_eq!(known_game("witcher3.exe").unwrap().1, Genre::Action);
        assert_eq!(known_game("iw4sp.exe").unwrap().1, Genre::Shooter);
        assert_eq!(known_game("unknown.exe"), None);
        // nothing in the table is a program that the list refuses (an online game, a non-game)
        for exe in ["iw3sp.exe", "farcry3.exe", "gta5.exe", "re4.exe", "forzahorizon4.exe", "nfs13.exe", "speed.exe", "eldenring.exe", "trackmania.exe"] {
            assert!(refuse_reason(exe).is_none(), "{}", exe);
        }
    }

    #[test]
    fn the_kind_of_a_game_is_guessed_by_its_name() {
        assert_eq!(guess_genre("Forza Horizon 5"), Genre::Racing);
        assert_eq!(guess_genre("Need for Speed Most Wanted"), Genre::Racing);
        assert_eq!(guess_genre("The Witcher 3: Wild Hunt"), Genre::Action);
        assert_eq!(guess_genre("Skyrim Special Edition"), Genre::Action);
        assert_eq!(guess_genre("Call of Duty: Modern Warfare 2"), Genre::Shooter);
        assert_eq!(guess_genre("Какая-то игра"), Genre::Shooter);
    }

    #[test]
    fn a_steam_library_is_scanned_and_online_games_and_anti_cheat_folders_are_not_added() {
        let root = tempfile::tempdir().unwrap();
        let steam = root.path();
        let apps = steam.join("steamapps");
        std::fs::create_dir_all(apps.join("common")).unwrap();
        let acf = |id: u32, name: &str, dir: &str| std::fs::write(apps.join(format!("appmanifest_{}.acf", id)), format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{}\"\n\t\"name\"\t\t\"{}\"\n\t\"installdir\"\t\t\"{}\"\n}}\n", id, name, dir)).unwrap();
        // a single-player game
        acf(1, "Forza Horizon 4", "ForzaHorizon4");
        touch(&apps.join("common").join("ForzaHorizon4").join("ForzaHorizon4.exe"), 5000);
        // an online game by its program
        acf(2, "Counter-Strike 2", "cs2");
        touch(&apps.join("common").join("cs2").join("cs2.exe"), 5000);
        // a game with an anti-cheat folder
        acf(3, "Some Shooter", "someshooter");
        touch(&apps.join("common").join("someshooter").join("shooter.exe"), 5000);
        touch(&apps.join("common").join("someshooter").join("EasyAntiCheat").join("a.dll"), 1);
        // a tool of Steam, a game without a folder, a game with no program
        acf(4, "Steamworks Common Redistributables", "Steamworks Shared");
        std::fs::create_dir_all(apps.join("common").join("Steamworks Shared")).unwrap();
        acf(5, "Ghost", "NoFolder");
        acf(6, "Empty", "Empty");
        std::fs::create_dir_all(apps.join("common").join("Empty")).unwrap();
        let found = steam_games(steam);
        let by = |n: &str| found.iter().find(|f| f.name == n).unwrap_or_else(|| panic!("{} is not in {:?}", n, found.iter().map(|f| &f.name).collect::<Vec<_>>()));
        assert_eq!(by("Forza Horizon 4").exe, "forzahorizon4.exe");
        assert_eq!(by("Forza Horizon 4").skipped, None);
        assert_eq!(by("Forza Horizon 4").genre, Genre::Racing);
        assert!(by("Counter-Strike 2").skipped.as_ref().unwrap().contains("сетевая игра"));
        assert!(by("Some Shooter").skipped.as_ref().unwrap().contains("защита от читов"));
        assert!(by("Empty").skipped.as_ref().unwrap().contains("не нашёл программу"));
        assert!(found.iter().all(|f| f.name != "Steamworks Common Redistributables" && f.name != "Ghost"));
        // the games that are fit enter the list, the others do not
        gamelist::test_reset();
        assert_eq!(add_found(&found), 1);
        let list = gamelist::load();
        assert_eq!(list.len(), 1);
        assert_eq!((list[0].exe.as_str(), list[0].source.as_str(), list[0].genre), ("forzahorizon4.exe", "scan", Genre::Racing));
        // again: nothing new
        assert_eq!(add_found(&found), 0);
        gamelist::test_reset();
    }

    #[test]
    fn epic_games_are_scanned_with_the_program_of_the_manifest() {
        let d = tempfile::tempdir().unwrap();
        let game = d.path().join("Game");
        touch(&game.join("Bin").join("Big.exe"), 9000);
        touch(&game.join("Bin").join("Small.exe"), 100);
        let manifests = d.path().join("Manifests");
        std::fs::create_dir_all(&manifests).unwrap();
        let item = serde_json::json!({ "DisplayName": "Tomb Raider", "InstallLocation": game.display().to_string(), "LaunchExecutable": "Bin\\Small.exe" });
        std::fs::write(manifests.join("a.item"), item.to_string()).unwrap();
        std::fs::write(manifests.join("note.txt"), "x").unwrap();
        let found = epic_games(&manifests);
        assert_eq!(found.len(), 1);
        assert_eq!((found[0].exe.as_str(), found[0].genre, found[0].store.as_str()), ("small.exe", Genre::Action, "Epic"), "the manifest names the program");
    }
}
