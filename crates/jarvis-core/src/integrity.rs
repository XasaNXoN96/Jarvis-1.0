// Are the command files the ones this program was built with?
//
// A command is a script with powers (`risk`, `sandbox`). Whoever can change a script of the installed Jarvis can give
// it any power, so the program knows the SHA-256 of every file of the command packs of its own build (build.rs puts
// the list into the program). A command whose file (its command.toml, its script, the shared _prelude.lua, a .ps1 it
// starts) is not on the list, or differs from it, still runs, but only as a plain "low" command of the Standard level:
// it can talk, set a timer, read; it cannot start a program, write a file or press a key.
//
// The list is checked only in a build that the CI made (it has the commit in it): a developer's build changes the
// scripts all the time. A new commands-update.zip is trusted only together with the program of the same build.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use once_cell::sync::Lazy;
use sha2::{Digest, Sha256};

// "<sha256>  <path inside resources/commands>" per line
const MANIFEST: &str = include_str!(concat!(env!("OUT_DIR"), "/commands_manifest.txt"));

static LISTED: Lazy<HashMap<String, String>> = Lazy::new(|| parse(MANIFEST));

pub fn parse(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let (hash, path) = l.split_once("  ")?;
            (hash.len() == 64).then(|| (path.trim().to_string(), hash.to_lowercase()))
        })
        .collect()
}

// the integrity is checked in the builds of the CI only
pub fn enforced() -> bool {
    #[cfg(test)]
    if let Some(v) = TEST_ENFORCED.with(|t| t.get()) {
        return v;
    }
    crate::config::BUILD_SHA.map(|s| !s.is_empty()).unwrap_or(false)
}

#[cfg(test)]
thread_local! {
    static TEST_ENFORCED: std::cell::Cell<Option<bool>> = std::cell::Cell::new(None);
    static TEST_LIST: std::cell::RefCell<Option<HashMap<String, String>>> = std::cell::RefCell::new(None);
    static TEST_ROOT: std::cell::RefCell<Option<PathBuf>> = std::cell::RefCell::new(None);
}

// the test names its own folder of commands and list
#[cfg(test)]
pub(crate) struct TestSetup;
#[cfg(test)]
impl TestSetup {
    pub(crate) fn new(root: &Path, list: HashMap<String, String>) -> TestSetup {
        TEST_ENFORCED.with(|t| t.set(Some(true)));
        TEST_LIST.with(|t| *t.borrow_mut() = Some(list));
        TEST_ROOT.with(|t| *t.borrow_mut() = Some(root.to_path_buf()));
        TestSetup
    }
}
#[cfg(test)]
impl Drop for TestSetup {
    fn drop(&mut self) {
        TEST_ENFORCED.with(|t| t.set(None));
        TEST_LIST.with(|t| *t.borrow_mut() = None);
        TEST_ROOT.with(|t| *t.borrow_mut() = None);
    }
}

fn listed(rel: &str) -> Option<String> {
    #[cfg(test)]
    if let Some(l) = TEST_LIST.with(|t| t.borrow().clone()) {
        return l.get(rel).cloned();
    }
    LISTED.get(rel).cloned()
}

fn listed_count() -> usize {
    #[cfg(test)]
    if let Some(l) = TEST_LIST.with(|t| t.borrow().clone()) {
        return l.len();
    }
    LISTED.len()
}

pub fn commands_root() -> PathBuf {
    #[cfg(test)]
    if let Some(r) = TEST_ROOT.with(|t| t.borrow().clone()) {
        return r;
    }
    crate::APP_DIR.join(crate::config::COMMANDS_PATH)
}

pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(h.finalize().iter().map(|b| format!("{:02x}", b)).collect())
}

// the path of a file inside the commands folder, as the list spells it ("assistant/power.lua"); None when the file is
// not inside, is a link (a link is another name for some other content) or is spelled with `..`
fn relative(root: &Path, file: &Path) -> Option<String> {
    if std::fs::symlink_metadata(file).ok()?.file_type().is_symlink() {
        return None;
    }
    let root_c = std::fs::canonicalize(root).ok()?;
    // the folder is resolved, the name of the file is kept as it is spelled
    let name = file.file_name()?;
    let parent_c = std::fs::canonicalize(file.parent()?).ok()?;
    let rel = parent_c.join(name);
    let rel = rel.strip_prefix(&root_c).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        match c {
            Component::Normal(s) => parts.push(s.to_string_lossy().to_string()),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

// this very file is the one the build listed
pub fn file_is_listed(file: &Path) -> bool {
    if !enforced() {
        return true;
    }
    let root = commands_root();
    let Some(rel) = relative(&root, file) else { return false };
    match (listed(&rel), sha256_file(file)) {
        (Some(want), Ok(have)) => want == have,
        _ => false,
    }
}

// What a Lua command is built of: its command.toml, its script and the shared prelude. All of them listed: trusted.
pub fn pack_command_is_trusted(pack_dir: &Path, script: &str) -> bool {
    if !enforced() {
        return true;
    }
    let root = commands_root();
    let prelude = root.join("_prelude.lua");
    let mut parts = vec![pack_dir.join("command.toml"), prelude];
    if !script.is_empty() {
        parts.push(pack_dir.join(script));
    }
    let ok = parts.iter().all(|p| file_is_listed(p));
    if !ok {
        note_once(pack_dir, script);
    }
    ok
}

static NOTED: Lazy<parking_lot::Mutex<std::collections::HashSet<String>>> = Lazy::new(|| parking_lot::Mutex::new(std::collections::HashSet::new()));

fn note_once(pack_dir: &Path, script: &str) {
    let key = format!("{}/{}", pack_dir.display(), script);
    if NOTED.lock().insert(key) {
        warn!("Command files of {} ({}) are not the ones of this build: the commands run as low / standard", pack_dir.display(), if script.is_empty() { "command.toml" } else { script });
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    // files that differ from the build
    pub modified: Vec<String>,
    // files that the build does not have
    pub extra: Vec<String>,
    // files of the build that are not there
    pub missing: Vec<String>,
    pub checked: usize,
}

impl Report {
    pub fn clean(&self) -> bool {
        self.modified.is_empty() && self.extra.is_empty() && self.missing.is_empty()
    }
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        // a link out of the commands folder is not followed: it is reported as an extra file
        let is_dir = e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false);
        if is_dir {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

// the whole folder of commands against the list (for the diagnostics)
pub fn scan() -> Report {
    let root = commands_root();
    let mut files = Vec::new();
    walk(&root, &mut files);
    let mut report = Report::default();
    let mut seen = std::collections::HashSet::new();
    for f in files {
        let Some(rel) = f.strip_prefix(&root).ok().map(|r| r.to_string_lossy().replace('\\', "/")) else { continue };
        if rel.ends_with('~') || rel.ends_with(".bak") || rel.ends_with(".swp") {
            continue;
        }
        report.checked += 1;
        seen.insert(rel.clone());
        match (listed(&rel), sha256_file(&f)) {
            (Some(want), Ok(have)) if want == have => {}
            (Some(_), _) => report.modified.push(rel),
            (None, _) => report.extra.push(rel),
        }
    }
    #[cfg(test)]
    let all: Vec<String> = TEST_LIST.with(|t| t.borrow().clone()).map(|l| l.keys().cloned().collect()).unwrap_or_else(|| LISTED.keys().cloned().collect());
    #[cfg(not(test))]
    let all: Vec<String> = LISTED.keys().cloned().collect();
    for rel in all {
        if !seen.contains(&rel) {
            report.missing.push(rel);
        }
    }
    report.modified.sort();
    report.extra.sort();
    report.missing.sort();
    report
}

pub fn listed_files() -> usize {
    listed_count()
}

// ---- the archive of a commands update --------------------------------------------------------------------------------

// A name inside commands-update.zip that is safe to unpack into the resources folder: it stays under `commands/`,
// is relative, has no `..`, no drive, no stream, no device name, no backslash tricks
pub fn zip_entry_is_safe(name: &str) -> Result<(), String> {
    archive_name_is_safe(name, "commands")
}

// the same rules for any archive: every name stays under the one folder `root`
pub fn archive_name_is_safe(name: &str, root: &str) -> Result<(), String> {
    let n = name.replace('\\', "/");
    if n.is_empty() || n.contains('\0') {
        return Err("пустое или недопустимое имя".into());
    }
    if n.starts_with('/') || n.chars().nth(1) == Some(':') {
        return Err(format!("абсолютный путь: {}", name));
    }
    let parts: Vec<&str> = n.trim_end_matches('/').split('/').collect();
    if parts.first() != Some(&root) {
        return Err(format!("файл вне папки {}: {}", root, name));
    }
    for p in &parts {
        if p.is_empty() || *p == ".." || *p == "." {
            return Err(format!("недопустимая часть пути в {}", name));
        }
        if p.contains(':') {
            return Err(format!("поток данных (:) в {}", name));
        }
        if p.ends_with('.') || p.ends_with(' ') {
            return Err(format!("имя кончается точкой или пробелом: {}", name));
        }
        let stem = p.split('.').next().unwrap_or("").to_lowercase();
        if matches!(stem.as_str(), "con" | "prn" | "aux" | "nul" | "com1" | "com2" | "com3" | "com4" | "com5" | "com6" | "com7" | "com8" | "com9" | "lpt1" | "lpt2" | "lpt3" | "lpt4" | "lpt5" | "lpt6" | "lpt7" | "lpt8" | "lpt9") {
            return Err(format!("имя устройства в {}", name));
        }
    }
    Ok(())
}

// ---- what an archive says it holds (read from its central directory, nothing is unpacked) ------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZipEntry {
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]]))
}
fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

// The entries of a zip file. It refuses (the answer is an error, never a guess): a file that is not a plain zip, zip64,
// several disks, an encrypted entry, an entry that is a link or a device (the Unix mode in the attributes), a name that is
// not UTF-8, a directory listing that does not fit in the file.
pub fn zip_entries(bytes: &[u8]) -> Result<Vec<ZipEntry>, String> {
    let bad = |why: &str| Err(format!("архив не принят: {}", why));
    if bytes.len() < 22 {
        return bad("слишком короткий файл");
    }
    // the end record: the last 22 bytes, or before a comment of up to 65535 bytes
    let from = bytes.len().saturating_sub(22 + 65535);
    let eocd = (from..=bytes.len() - 22).rev().find(|&i| le32(bytes, i) == Some(0x0605_4b50));
    let Some(e) = eocd else { return bad("нет записи конца архива") };
    let (disk, cd_disk, here, total) = (le16(bytes, e + 4).unwrap_or(1), le16(bytes, e + 6).unwrap_or(1), le16(bytes, e + 8).unwrap_or(0), le16(bytes, e + 10).unwrap_or(0));
    let (cd_size, cd_off) = (le32(bytes, e + 12).unwrap_or(0), le32(bytes, e + 16).unwrap_or(0));
    if disk != 0 || cd_disk != 0 || here != total {
        return bad("архив из нескольких частей");
    }
    if total == 0xFFFF || cd_size == 0xFFFF_FFFF || cd_off == 0xFFFF_FFFF {
        return bad("zip64 не поддерживается");
    }
    let (cd_off, cd_size) = (cd_off as usize, cd_size as usize);
    if cd_off.checked_add(cd_size).map(|end| end > e).unwrap_or(true) {
        return bad("оглавление не помещается в файл");
    }
    let mut out = Vec::new();
    let mut at = cd_off;
    for _ in 0..total {
        if le32(bytes, at) != Some(0x0201_4b50) {
            return bad("повреждено оглавление");
        }
        let made_by = le16(bytes, at + 4).unwrap_or(0);
        let flags = le16(bytes, at + 8).unwrap_or(0);
        let usize_ = le32(bytes, at + 24).unwrap_or(0);
        let (nlen, xlen, clen) = (le16(bytes, at + 28).unwrap_or(0) as usize, le16(bytes, at + 30).unwrap_or(0) as usize, le16(bytes, at + 32).unwrap_or(0) as usize);
        let attrs = le32(bytes, at + 38).unwrap_or(0);
        let Some(raw) = bytes.get(at + 46..at + 46 + nlen) else { return bad("имя за пределами файла") };
        if flags & 1 != 0 {
            return bad("зашифрованная запись");
        }
        let name = match std::str::from_utf8(raw) {
            Ok(n) => n.to_string(),
            Err(_) => return bad("имя не в UTF-8"),
        };
        // the Unix mode lives in the upper half of the attributes when the archive was made on Unix
        if made_by >> 8 == 3 {
            let kind = (attrs >> 16) & 0o170000;
            if kind != 0 && kind != 0o100000 && kind != 0o040000 {
                return bad(&format!("запись «{}» не файл и не папка (ссылка или устройство)", name));
            }
        }
        if usize_ == 0xFFFF_FFFF {
            return bad("zip64 не поддерживается");
        }
        out.push(ZipEntry { is_dir: name.ends_with('/'), name, size: usize_ as u64 });
        at += 46 + nlen + xlen + clen;
    }
    Ok(out)
}

// The whole check before an archive is unpacked: every name under `root`, no links, not too many files, not too big.
pub fn check_archive(path: &Path, root: &str, max_files: usize, max_total: u64) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|_| "архив не читается".to_string())?;
    let entries = zip_entries(&bytes)?;
    if entries.is_empty() || entries.len() > max_files {
        return Err(format!("в архиве {} записей (допустимо до {})", entries.len(), max_files));
    }
    let mut total = 0u64;
    for e in &entries {
        archive_name_is_safe(&e.name, root)?;
        total = total.saturating_add(e.size);
    }
    if total > max_total {
        return Err(format!("распакованный размер {} МБ больше допустимых {} МБ", total / 1_000_000, max_total / 1_000_000));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    fn copy_dir(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap();
        for e in fs::read_dir(from).unwrap().flatten() {
            let (src, dst) = (e.path(), to.join(e.file_name()));
            if src.is_dir() {
                copy_dir(&src, &dst);
            } else {
                fs::copy(&src, &dst).unwrap();
            }
        }
    }

    fn repo_commands() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands")
    }

    // the list of this build is the folder of this build
    #[test]
    fn the_folder_of_the_build_is_clean_against_its_own_list() {
        let _s = TestSetup::new(&repo_commands(), LISTED.clone());
        let r = scan();
        assert!(r.clean(), "{:?}", r);
        assert!(r.checked >= 60 && listed_files() == r.checked, "{} files checked, {} listed", r.checked, listed_files());
        // the real commands of the packs are trusted
        for (pack, script) in [("assistant", "power.lua"), ("assistant", "office.lua"), ("input", "input.lua"), ("calculator", "")] {
            assert!(pack_command_is_trusted(&repo_commands().join(pack), script), "{}/{}", pack, script);
        }
    }

    #[test]
    fn a_changed_script_an_extra_file_and_a_changed_prelude_are_not_trusted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("commands");
        copy_dir(&repo_commands(), &root);
        let _s = TestSetup::new(&root, LISTED.clone());
        let power = root.join("assistant").join("power.lua");
        assert!(pack_command_is_trusted(&root.join("assistant"), "power.lua"));
        // one more line in a script
        let mut text = fs::read_to_string(&power).unwrap();
        text.push_str("\nrun('shutdown', {'/s'})\n");
        fs::write(&power, &text).unwrap();
        assert!(!pack_command_is_trusted(&root.join("assistant"), "power.lua"), "a changed script");
        assert!(pack_command_is_trusted(&root.join("assistant"), "timer.lua"), "the other scripts of the pack are untouched");
        // a script that is new
        fs::write(root.join("assistant").join("new.lua"), "say('x')").unwrap();
        assert!(!pack_command_is_trusted(&root.join("assistant"), "new.lua"), "a file that the build does not have");
        // a new pack with a new toml
        fs::create_dir_all(root.join("mine")).unwrap();
        fs::write(root.join("mine").join("command.toml"), "[[commands]]\nid = \"x\"\ntype = \"lua\"\nrisk = \"high\"\n").unwrap();
        fs::write(root.join("mine").join("s.lua"), "say('x')").unwrap();
        assert!(!pack_command_is_trusted(&root.join("mine"), "s.lua"));
        // the report names all three kinds
        fs::remove_file(root.join("assistant").join("timer.lua")).unwrap();
        let r = scan();
        assert!(r.modified.contains(&"assistant/power.lua".to_string()), "{:?}", r);
        assert!(r.extra.contains(&"assistant/new.lua".to_string()) && r.extra.contains(&"mine/command.toml".to_string()), "{:?}", r);
        assert!(r.missing.contains(&"assistant/timer.lua".to_string()), "{:?}", r);
        // the prelude is a part of every script: a changed prelude makes every command untrusted
        fs::write(root.join("_prelude.lua"), format!("{}\n-- x", fs::read_to_string(repo_commands().join("_prelude.lua")).unwrap())).unwrap();
        assert!(!pack_command_is_trusted(&root.join("input"), "input.lua"));
        assert!(!pack_command_is_trusted(&root.join("calculator"), ""), "even a command with no script of its own");
    }

    #[test]
    fn a_changed_toml_makes_the_whole_pack_untrusted() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("commands");
        copy_dir(&repo_commands(), &root);
        let _s = TestSetup::new(&root, LISTED.clone());
        let toml = root.join("calculator").join("command.toml");
        let text = fs::read_to_string(&toml).unwrap();
        fs::write(&toml, text.replace("risk = \"high\"", "risk = \"low\"")).unwrap();
        assert!(!pack_command_is_trusted(&root.join("calculator"), ""));
        assert!(pack_command_is_trusted(&root.join("input"), "input.lua"));
    }

    // a link in the commands folder that points out of it does not make a foreign file trusted
    #[cfg(unix)]
    #[test]
    fn a_link_out_of_the_commands_folder_is_not_a_listed_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("commands");
        copy_dir(&repo_commands(), &root);
        let _s = TestSetup::new(&root, LISTED.clone());
        let outside = dir.path().join("evil.lua");
        fs::write(&outside, "say('x')").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("assistant").join("timer_link.lua")).unwrap();
        assert!(!file_is_listed(&root.join("assistant").join("timer_link.lua")));
        assert!(!file_is_listed(&outside), "a file outside is not inside the folder");
        // a listed file replaced by a link to another listed file is a different content
        fs::remove_file(root.join("assistant").join("greet.lua")).ok();
        std::os::unix::fs::symlink(root.join("assistant").join("timer.lua"), root.join("assistant").join("smalltalk.lua.link")).ok();
        fs::remove_file(root.join("assistant").join("smalltalk.lua")).unwrap();
        std::os::unix::fs::symlink(root.join("assistant").join("timer.lua"), root.join("assistant").join("smalltalk.lua")).unwrap();
        assert!(!file_is_listed(&root.join("assistant").join("smalltalk.lua")));
    }

    // a developer's build is not checked, and an official build with no list is not trusted at all
    #[test]
    fn a_build_without_the_commit_is_not_checked() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("x.lua"), "x").unwrap();
        // no TestSetup: enforced() follows the build of the test, which has no commit
        assert!(!enforced());
        assert!(pack_command_is_trusted(dir.path(), "x.lua"));
        assert!(file_is_listed(&dir.path().join("x.lua")));
        let _s = TestSetup::new(dir.path(), HashMap::new());
        assert!(!file_is_listed(&dir.path().join("x.lua")), "enforced, and the list does not have it");
    }

    #[test]
    fn the_names_in_a_commands_zip_stay_in_the_commands_folder() {
        for ok in ["commands/", "commands/assistant/", "commands/assistant/power.lua", "commands\\assistant\\power.lua", "commands/_prelude.lua", "commands/browser/ahk/chrome.ahk"] {
            assert!(zip_entry_is_safe(ok).is_ok(), "{}", ok);
        }
        for bad in [
            "../commands/a.lua",
            "commands/../../Windows/x.dll",
            "commands/assistant/../../../x",
            "/etc/passwd",
            "C:\\Windows\\x.dll",
            "c:/x",
            "commands/a.lua:stream",
            "commands/CON",
            "commands/nul.txt",
            "commands/a.lua.",
            "commands/a.lua ",
            "other/x.lua",
            "x.lua",
            "commands//x.lua",
            "commands/./x.lua",
            "",
            "commands/a\0b",
            "..\\x",
            "commands\\..\\..\\x",
        ] {
            assert!(zip_entry_is_safe(bad).is_err(), "{:?}", bad);
        }
    }

    // a zip made by hand (stored entries): (name, made_by, attributes, flags)
    pub(crate) fn make_zip(entries: &[(&str, u16, u32, u16)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut cd = Vec::new();
        for (name, made_by, attrs, flags) in entries {
            let off = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0]);
            out.extend_from_slice(&flags.to_le_bytes());
            out.extend_from_slice(&[0; 10]); // method, time, date, crc
            out.extend_from_slice(&5u32.to_le_bytes()); // compressed
            out.extend_from_slice(&5u32.to_le_bytes()); // uncompressed
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b"hello");
            cd.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            cd.extend_from_slice(&made_by.to_le_bytes());
            cd.extend_from_slice(&[20, 0]);
            cd.extend_from_slice(&flags.to_le_bytes());
            cd.extend_from_slice(&[0; 10]);
            cd.extend_from_slice(&5u32.to_le_bytes());
            cd.extend_from_slice(&5u32.to_le_bytes());
            cd.extend_from_slice(&(name.len() as u16).to_le_bytes());
            cd.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]); // extra, comment, disk, internal attrs
            cd.extend_from_slice(&attrs.to_le_bytes());
            cd.extend_from_slice(&off.to_le_bytes());
            cd.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&cd);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        out.extend_from_slice(&(cd.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&[0, 0]);
        out
    }

    fn check_bytes(bytes: &[u8], root: &str) -> Result<(), String> {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.zip");
        fs::write(&p, bytes).unwrap();
        check_archive(&p, root, 100, 1_000_000)
    }

    #[test]
    fn a_good_archive_is_accepted_and_every_unsafe_one_is_refused_before_unpacking() {
        let unix_file = (3u16 << 8, 0o100644u32 << 16);
        let good = make_zip(&[("piper/", 0, 0, 0), ("piper/piper.exe", unix_file.0, unix_file.1, 0), ("piper/espeak-ng-data/voices", 0, 0, 0)]);
        assert_eq!(zip_entries(&good).unwrap().len(), 3);
        check_bytes(&good, "piper").unwrap();
        // a way out of the folder, in every spelling
        for bad in ["piper/../evil.dll", "../evil.dll", "/evil.dll", "C:/evil.dll", "C:\\evil.dll", "piper\\..\\evil.dll", "other/x.txt", "piper/a.txt:evil", "piper/con", "piper/a/./b", "piper//x", "piper/x."] {
            let z = make_zip(&[("piper/ok.txt", 0, 0, 0), (bad, 0, 0, 0)]);
            assert!(check_bytes(&z, "piper").is_err(), "{:?} was accepted", bad);
        }
        // a link, a device, an encrypted entry
        let link = make_zip(&[("piper/l", 3 << 8, 0o120777u32 << 16, 0)]);
        assert!(check_bytes(&link, "piper").unwrap_err().contains("ссылка"));
        let dev = make_zip(&[("piper/d", 3 << 8, 0o020666u32 << 16, 0)]);
        assert!(check_bytes(&dev, "piper").is_err());
        let enc = make_zip(&[("piper/e", 0, 0, 1)]);
        assert!(check_bytes(&enc, "piper").unwrap_err().contains("зашифр"));
        // not a zip, cut in the middle, a listing that points outside the file
        assert!(check_bytes(b"not a zip at all, just text bytes", "piper").is_err());
        assert!(check_bytes(&good[..good.len() / 2], "piper").is_err());
        let mut broken = good.clone();
        let n = broken.len();
        broken[n - 6..n - 2].copy_from_slice(&0x7fff_ffffu32.to_le_bytes()); // the offset of the listing
        assert!(check_bytes(&broken, "piper").is_err());
        // too many files, too big in total
        let many: Vec<(String, u16, u32, u16)> = (0..101).map(|i| (format!("piper/f{}", i), 0, 0, 0)).collect();
        let refs: Vec<(&str, u16, u32, u16)> = many.iter().map(|(a, b, c, d)| (a.as_str(), *b, *c, *d)).collect();
        assert!(check_bytes(&make_zip(&refs), "piper").is_err());
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.zip");
        fs::write(&p, &good).unwrap();
        assert!(check_archive(&p, "piper", 100, 10).unwrap_err().contains("МБ"), "a total size limit");
    }

    // the real archive of Piper, when the test is given one (the CI of the installer downloads it)
    #[test]
    fn the_real_piper_archive_passes_when_present() {
        let Ok(p) = std::env::var("JARVIS_TEST_PIPER_ZIP") else {
            eprintln!("NOT TESTED: JARVIS_TEST_PIPER_ZIP is not set");
            return;
        };
        check_archive(Path::new(&p), "piper", 5000, 800_000_000).unwrap();
    }
}
