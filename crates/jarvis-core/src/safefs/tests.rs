use super::*;
use std::cell::Cell;
use std::fs;
use std::rc::Rc;

struct Places {
    _root: tempfile::TempDir,
    home: PathBuf,      // the allowed folder
    protected: PathBuf, // a protected place (Windows, Program Files, Jarvis)
    data: PathBuf,      // the settings of Jarvis
}

impl Drop for Places {
    fn drop(&mut self) {
        policy::set_test_allowed(Vec::new());
        policy::set_test_protected(Vec::new());
        policy::set_test_data(None);
    }
}

fn places() -> Places {
    let root = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(root.path()).unwrap();
    let home = base.join("home").join("Documents");
    let protected = base.join("Windows");
    let data = base.join("JarvisData");
    for d in [&home, &protected, &data] {
        fs::create_dir_all(d).unwrap();
    }
    policy::set_test_allowed(vec![home.clone()]);
    policy::set_test_protected(vec![protected.clone()]);
    policy::set_test_data(Some(data.clone()));
    Places { _root: root, home, protected, data }
}

// a folder link: a symbolic link on Unix, a junction (no privilege needed) on Windows
fn link_dir(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().map(|o| o.status.success()).unwrap_or(false)
    }
}

fn link_file(link: &Path, target: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).is_ok()
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
}

fn is_link(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

#[test]
fn ordinary_operations_inside_the_allowed_folder_work() {
    let pl = places();
    let f = pl.home.join("a").join("b").join("note.txt");
    write(&f, b"one").unwrap();
    write(&f, b"two").unwrap();
    assert_eq!(fs::read_to_string(&f).unwrap(), "two");
    append(&f, b"+3").unwrap();
    assert_eq!(read(&f).unwrap(), b"two+3");
    mkdir_all(&pl.home.join("x").join("y")).unwrap();
    assert!(pl.home.join("x").join("y").is_dir());
    remove(&f).unwrap();
    assert!(!f.exists());
    // an empty folder goes, a folder with something in it does not (a command never deletes a tree)
    remove(&pl.home.join("x").join("y")).unwrap();
    fs::write(pl.home.join("a").join("keep.txt"), "k").unwrap();
    assert!(remove(&pl.home.join("a")).is_err());
    assert!(pl.home.join("a").join("keep.txt").exists());
    // no temporary file stays
    let names: Vec<String> = fs::read_dir(pl.home.join("a").join("b")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
    assert!(names.is_empty(), "{:?}", names);
}

#[test]
fn a_refusal_of_the_policy_changes_nothing() {
    let pl = places();
    let in_protected = pl.protected.join("evil.dll");
    let in_data = pl.data.join("settings.json");
    let outside = pl.home.parent().unwrap().join("elsewhere").join("x.txt");
    for p in [&in_protected, &in_data, &outside] {
        assert!(matches!(write(p, b"x"), Err(SafeError::Denied(_))), "{}", p.display());
        assert!(matches!(append(p, b"x"), Err(SafeError::Denied(_))));
        assert!(matches!(mkdir_all(&p.with_extension("d")), Err(SafeError::Denied(_))));
        assert!(!p.exists() && !p.with_extension("d").exists());
    }
    fs::write(&in_protected, "orig").unwrap();
    assert!(matches!(remove(&in_protected), Err(SafeError::Denied(_))));
    assert_eq!(fs::read_to_string(&in_protected).unwrap(), "orig");
    fs::write(&in_data, "secret").unwrap();
    assert!(matches!(read(&in_data), Err(SafeError::Denied(_))));
    // a relative path and a «..» spelling
    assert!(write(Path::new("relative.txt"), b"x").is_err());
    assert!(matches!(write(&pl.home.join("..").join("..").join("Windows").join("a.dll"), b"x"), Err(SafeError::Denied(_))));
    assert!(!pl.protected.join("a.dll").exists());
}

// THE bypass: the folder of the path is replaced by a link into a protected place after the first check
#[test]
fn a_folder_replaced_by_a_link_between_the_check_and_the_write_does_not_lead_out() {
    let pl = places();
    let sub = pl.home.join("sub");
    fs::create_dir_all(&sub).unwrap();
    let moved = pl.home.join("sub_moved");
    let (s, m, p) = (sub.clone(), moved.clone(), pl.protected.clone());
    let swap_refused = Rc::new(Cell::new(false));
    let linked = Rc::new(Cell::new(false));
    let (sr, lk) = (swap_refused.clone(), linked.clone());
    set_between(move || {
        // the attacker: move the folder away and put a link of the same name in its place
        if fs::rename(&s, &m).is_err() {
            sr.set(true); // Windows: the held folder cannot be renamed
        } else {
            lk.set(link_dir(&s, &p));
        }
    });
    let r = write(&sub.join("f.txt"), b"payload");
    assert!(!pl.protected.join("f.txt").exists(), "the file went through the link into the protected place ({:?})", r);
    #[cfg(unix)]
    {
        // the held folder is the judged object: the bytes are where that folder stands now
        assert!(linked.get());
        assert!(r.is_ok());
        assert_eq!(fs::read_to_string(moved.join("f.txt")).unwrap(), "payload");
    }
    #[cfg(windows)]
    {
        // the folder could not be moved: the write is where it was judged
        assert!(swap_refused.get(), "a held folder was renamed: the lock does not work");
        assert!(r.is_ok());
        assert_eq!(fs::read_to_string(sub.join("f.txt")).unwrap(), "payload");
    }
}

// the same for a folder ABOVE the file (the allowed folder itself is replaced)
#[test]
fn a_folder_above_replaced_by_a_link_does_not_lead_out_either() {
    let pl = places();
    let up = pl.home.join("up");
    let down = up.join("down");
    fs::create_dir_all(&down).unwrap();
    let moved = pl.home.join("up_moved");
    let (u, m, p) = (up.clone(), moved.clone(), pl.protected.clone());
    set_between(move || {
        if fs::rename(&u, &m).is_ok() {
            let _ = link_dir(&u, &p);
        }
    });
    let _ = write(&down.join("f.txt"), b"payload");
    assert!(!pl.protected.join("down").join("f.txt").exists());
    assert!(!pl.protected.join("f.txt").exists());
}

#[test]
fn a_file_replaced_by_a_link_is_itself_replaced_and_the_target_is_not_touched() {
    let pl = places();
    let victim = pl.protected.join("victim.txt");
    fs::write(&victim, "original").unwrap();
    let f = pl.home.join("f.txt");
    fs::write(&f, "mine").unwrap();
    let (ff, v) = (f.clone(), victim.clone());
    let made = Rc::new(Cell::new(false));
    let mk = made.clone();
    set_between(move || {
        let _ = fs::remove_file(&ff);
        mk.set(link_file(&ff, &v));
    });
    // the second judgement looks at the name as it is NOW: a link that leads into the protected place is refused (when it
    // were not looked at, the name would be replaced, never followed)
    let r = write(&f, b"new");
    if !made.get() {
        eprintln!("NOT TESTED: no right to create file links here");
    }
    assert_eq!(fs::read_to_string(&victim).unwrap(), "original", "written through the link");
    if made.get() {
        assert!(matches!(r, Err(SafeError::Denied(_))), "{:?}", r);
    }
    assert!(r.is_err() || (!is_link(&f) && fs::read_to_string(&f).unwrap() == "new"));
}

#[test]
fn an_append_refuses_a_link_that_appeared_after_the_check() {
    let pl = places();
    let victim = pl.protected.join("victim.log");
    fs::write(&victim, "original").unwrap();
    let f = pl.home.join("log.txt");
    let (ff, v) = (f.clone(), victim.clone());
    let made = Rc::new(Cell::new(false));
    let mk = made.clone();
    set_between(move || mk.set(link_file(&ff, &v)));
    let r = append(&f, b" EVIL");
    assert_eq!(fs::read_to_string(&victim).unwrap(), "original", "written through the link ({:?})", r);
    if made.get() {
        assert!(r.is_err());
    } else {
        eprintln!("NOT TESTED: no right to create file links here");
    }
}

#[test]
fn a_hard_link_to_a_protected_file_is_not_written_through() {
    let pl = places();
    let victim = pl.protected.join("victim.txt");
    fs::write(&victim, "original").unwrap();
    let hl = pl.home.join("hl.txt");
    fs::hard_link(&victim, &hl).unwrap();
    assert!(append(&hl, b" EVIL").is_err());
    assert_eq!(fs::read_to_string(&victim).unwrap(), "original");
    // a write replaces the name, never the content of the shared file
    write(&hl, b"new").unwrap();
    assert_eq!(fs::read_to_string(&victim).unwrap(), "original");
    assert_eq!(fs::read_to_string(&hl).unwrap(), "new");
}

#[test]
fn a_remove_deletes_the_link_and_never_its_target() {
    let pl = places();
    let victim_dir = pl.protected.join("keep");
    fs::create_dir_all(&victim_dir).unwrap();
    fs::write(victim_dir.join("a.txt"), "a").unwrap();
    // a link in the allowed folder that leads out: the policy judges the real place and refuses; nothing is deleted
    let l = pl.home.join("l");
    if link_dir(&l, &victim_dir) {
        assert!(remove(&l).is_err() || !victim_dir.join("a.txt").exists() == false);
        assert!(victim_dir.join("a.txt").exists());
    } else {
        eprintln!("NOT TESTED: no right to create links here");
    }
    // a file replaced by a link to a protected file after the check: the link goes, the file stays
    let f = pl.home.join("f.txt");
    fs::write(&f, "x").unwrap();
    let victim = pl.protected.join("victim.txt");
    fs::write(&victim, "original").unwrap();
    let (ff, v) = (f.clone(), victim.clone());
    set_between(move || {
        let _ = fs::remove_file(&ff);
        let _ = link_file(&ff, &v);
    });
    let _ = remove(&f);
    assert_eq!(fs::read_to_string(&victim).unwrap(), "original");
}

#[test]
fn a_read_follows_a_link_but_judges_where_it_led() {
    let pl = places();
    let secret = pl.data.join("secret.txt");
    fs::write(&secret, "token").unwrap();
    let f = pl.home.join("r.txt");
    fs::write(&f, "plain").unwrap();
    assert_eq!(read(&f).unwrap(), b"plain");
    let (ff, s) = (f.clone(), secret.clone());
    let made = Rc::new(Cell::new(false));
    let mk = made.clone();
    set_between(move || {
        let _ = fs::remove_file(&ff);
        mk.set(link_file(&ff, &s));
    });
    let r = read(&f);
    if made.get() {
        assert!(r.is_err(), "the settings of Jarvis were read through a link");
    } else {
        eprintln!("NOT TESTED: no right to create file links here");
    }
}

#[test]
fn the_folder_chain_refuses_a_link() {
    let pl = places();
    let target = pl.home.join("real");
    fs::create_dir_all(target.join("inner")).unwrap();
    let l = pl.home.join("lnk");
    if !link_dir(&l, &target) {
        eprintln!("NOT TESTED: no right to create links here");
        return;
    }
    // the last component
    assert!(imp::open_dir_chain(&l, false).is_err());
    // a component in the middle (the walk by openat refuses it on Unix; on Windows the operating system follows the
    // middle ones and the second judgement of the real place is what protects: see the tests above)
    #[cfg(unix)]
    assert!(imp::open_dir_chain(&l.join("inner"), false).is_err());
    assert!(imp::open_dir_chain(&target.join("inner"), false).is_ok());
}

// every file function of the Lua API goes through here: no direct operation is left in it
#[test]
fn the_lua_file_api_has_no_direct_file_operations() {
    let src = include_str!("../lua/api/fs.rs").replace("\r\n", "\n");
    let code = src.split("#[cfg(test)]").next().unwrap().replace("safefs::", "");
    for bad in ["fs::write(", "fs::read(", "fs::read_to_string(", "fs::remove_file(", "fs::remove_dir(", "fs::remove_dir_all(", "fs::create_dir_all(", "OpenOptions", "write_atomic", "File::open", "File::create", "fs::rename("] {
        assert!(!code.contains(bad), "lua/api/fs.rs still calls {} by itself", bad);
    }
    assert!(src.contains("safefs::write") && src.contains("safefs::append") && src.contains("safefs::remove") && src.contains("safefs::mkdir_all") && src.contains("safefs::read"));
}
