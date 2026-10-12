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

#[cfg(target_os = "windows")]
fn junction_for(dir: &Path) -> Option<PathBuf> {
    use std::os::windows::process::CommandExt;
    let base = PathBuf::from(std::env::var_os("ProgramData")?).join("JarvisAlias");
    if !is_ascii(&base) {
        return None;
    }
    std::fs::create_dir_all(&base).ok()?;
    let link = base.join(alias_name(dir));
    // a working alias is reused; one whose target is gone (the program was moved or reinstalled) is made again
    if link.exists() {
        return Some(link);
    }
    let _ = std::fs::remove_dir(&link);
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/C", "mklink", "/J"]).arg(&link).arg(dir);
    crate::null_stdio(&mut cmd);
    cmd.creation_flags(0x0800_0000);
    let ok = cmd.status().map(|s| s.success()).unwrap_or(false);
    if ok && link.exists() {
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
}
