// Programs written in C/C++ (Piper, the Vosk speech recognizer) take file paths in the ANSI code page: in a folder with
// non-Latin letters ("C:\Users\Иван\...", "C:\Jarvis Тест") they cannot open their own files (Piper crashes with
// 0xC0000409, Vosk says "Failed to load model"). They are given a plain-ASCII alias of the folder: a junction under
// %ProgramData%\JarvisAlias (no administrator rights needed). Real Windows runs of the CI showed both failures.

use std::path::{Path, PathBuf};

pub fn is_ascii(p: &Path) -> bool {
    p.to_string_lossy().is_ascii()
}

// the name of the alias: the same folder always gets the same ASCII name
pub fn alias_name(dir: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    dir.to_string_lossy().to_lowercase().hash(&mut h);
    format!("{:016x}", h.finish())
}

// the folder itself when its name is ASCII already, or when no alias could be made
#[cfg(target_os = "windows")]
pub fn ascii_dir(dir: PathBuf) -> PathBuf {
    if is_ascii(&dir) {
        return dir;
    }
    junction_for(&dir).unwrap_or(dir)
}

#[cfg(not(target_os = "windows"))]
pub fn ascii_dir(dir: PathBuf) -> PathBuf {
    dir
}

// does the link lead to the very folder `dir` (both resolved by the system)
pub fn leads_to(link: &Path, dir: &Path) -> bool {
    match (std::fs::canonicalize(link), std::fs::canonicalize(dir)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(target_os = "windows")]
fn junction_for(dir: &Path) -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    let base = PathBuf::from(std::env::var_os("ProgramData")?).join("JarvisAlias");
    if !is_ascii(&base) {
        return None;
    }
    std::fs::create_dir_all(&base).ok()?;
    let link = base.join(alias_name(dir));
    // an alias is reused only when it leads to THIS folder: %ProgramData% is open to every user of the computer and the name
    // is predictable, so another account could have made a link of that name to a folder of its own (with its own piper.exe).
    // One that leads elsewhere (or whose target is gone: the program was moved or reinstalled) is removed and made again;
    // when it cannot be removed, no alias is used at all.
    if link.exists() {
        if leads_to(&link, dir) {
            return Some(link);
        }
        warn!("The alias {} does not lead to {}: it is replaced", link.display(), dir.display());
    }
    let _ = std::fs::remove_dir(&link);
    if std::fs::symlink_metadata(&link).is_ok() {
        warn!("The foreign alias {} cannot be removed: no alias is used", link.display());
        return None;
    }
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/C", "mklink", "/J"]).arg(&link).arg(dir);
    crate::null_stdio(&mut cmd);
    cmd.creation_flags(0x0800_0000);
    let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    if ok && link.exists() && leads_to(&link, dir) {
        Some(link)
    } else {
        warn!("Could not make an ASCII alias for {}", dir.display());
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ascii_alias_of_a_folder_is_stable_plain_and_different_for_other_folders() {
        let a = alias_name(Path::new("C:\\Jarvis Тест Full\\piper"));
        assert!(a.is_ascii() && a.len() == 16, "{}", a);
        assert_eq!(a, alias_name(Path::new("c:\\jarvis тест full\\PIPER")), "the same folder in another case");
        assert_ne!(a, alias_name(Path::new("C:\\Users\\Иван\\Jarvis\\piper")));
        // an ASCII folder is used as it is (no alias)
        assert_eq!(ascii_dir(PathBuf::from("C:\\Jarvis\\piper")), PathBuf::from("C:\\Jarvis\\piper"));
        assert!(!is_ascii(Path::new("C:\\Users\\Иван")));
    }

    // a link of that name made by someone else must not be taken for the alias
    #[test]
    fn an_alias_is_trusted_only_when_it_leads_to_the_folder() {
        let d = tempfile::tempdir().unwrap();
        let mine = d.path().join("piper");
        let theirs = d.path().join("attacker");
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&theirs).unwrap();
        let link = d.path().join("alias");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(&theirs, &link).is_ok();
        #[cfg(windows)]
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(&link).arg(&theirs).output().map(|o| o.status.success()).unwrap_or(false);
        assert!(made, "the test needs a folder link");
        assert!(!leads_to(&link, &mine), "an alias to a foreign folder was trusted");
        assert!(leads_to(&link, &theirs));
        assert!(!leads_to(&d.path().join("missing"), &mine));
        // removing a link removes the link only
        std::fs::remove_dir(&link).or_else(|_| std::fs::remove_file(&link)).unwrap();
        assert!(theirs.exists());
    }
}
