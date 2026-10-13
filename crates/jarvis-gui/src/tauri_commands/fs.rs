#[cfg(target_os = "linux")]
use std::fs::metadata;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

use std::process::Command;

// The path comes from the page of the old interface: it must be a full path of something that exists, with nothing
// that explorer would read as a switch (a leading «/» of an option) or that breaks a command line. This crate is not part of
// the shipped build (the window of the program is jarvis-app); the check keeps it safe if it is ever used again.
fn path_is_safe(path: &str) -> bool {
  let p = std::path::Path::new(path);
  // (on Windows a path is absolute only with a drive or a share: «/select» and «\\?\GLOBALROOT» style switches are not)
  p.is_absolute() && p.exists() && !path.chars().any(|c| c.is_control() || c == '"')
}

// taken from https://github.com/tauri-apps/tauri/issues/4062#issuecomment-1338048169
#[tauri::command]
pub fn show_in_folder(path: String) {
  if !path_is_safe(&path) {
    return;
  }
  #[cfg(target_os = "windows")]
  {
    Command::new("explorer")
        .args(["/select,", &path]) // The comma after select is not a typo
        .spawn()
        .unwrap();
  }

  #[cfg(target_os = "linux")]
  {
    if path.contains(",") {
      // see https://gitlab.freedesktop.org/dbus/dbus/-/issues/76
      let new_path = match metadata(&path).unwrap().is_dir() {
        true => path,
        false => {
          let mut path2 = PathBuf::from(path);
          path2.pop();
          path2.into_os_string().into_string().unwrap()
        }
      };
      Command::new("xdg-open")
          .arg(&new_path)
          .spawn()
          .unwrap();
    } else {
      Command::new("dbus-send")
          .args(["--session", "--dest=org.freedesktop.FileManager1", "--type=method_call",
                "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems",
                format!("array:string:\"file://{path}\"").as_str(), "string:\"\""])
          .spawn()
          .unwrap();
    }
  }

  #[cfg(target_os = "macos")]
  {
    Command::new("open")
        .args(["-R", &path])
        .spawn()
        .unwrap();
  }
}

#[cfg(test)]
mod tests {
  use super::path_is_safe;

  #[test]
  fn only_a_full_path_of_something_that_exists_is_shown() {
    let here = std::env::current_dir().unwrap();
    assert!(path_is_safe(&here.to_string_lossy()));
    for bad in ["", "relative/file.txt", "/select,evil", "..\\..\\x", "C:\\nonexistent-dir-xyz\\a.txt", "x\"y", "a\nb"] {
      assert!(!path_is_safe(bad), "{:?}", bad);
    }
  }
}
