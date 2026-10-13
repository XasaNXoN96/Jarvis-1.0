// Who may read the data folder of Jarvis (keys, the secrets of the window and of the phone remote, the history of the
// dialogs, the settings).
//
// Windows: the folder gets its own PROTECTED access list (no inheritance from the parent, which in a fallback place such
// as a program folder or a shared folder may give "Users" the right to read): the owner of the folder (the user who runs
// Jarvis), SYSTEM and Administrators, full control, passed on to what is inside. Unix (development only): mode 0700 for
// the folder, 0600 for a secret file.
//
// What this stops: another Windows user (a standard account on the same computer), a program of the group "Users",
// a backup tool or a network share that follows the inheritance of a wider parent. What this does NOT stop: a program
// that runs as the SAME user (it is the owner), an administrator, the SYSTEM account.

use std::path::Path;

#[cfg(unix)]
pub fn harden_dir(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

#[cfg(unix)]
pub fn private_file(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| e.to_string())
}

// (a file inside a hardened folder has the access list of the folder: nothing more to do)
#[cfg(windows)]
pub fn private_file(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(windows)]
pub fn harden_dir(dir: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr::null_mut;
    use winapi::shared::minwindef::{BOOL, FALSE};
    use winapi::um::accctrl::SE_FILE_OBJECT;
    use winapi::um::aclapi::SetNamedSecurityInfoW;
    use winapi::shared::sddl::ConvertStringSecurityDescriptorToSecurityDescriptorW;
    use winapi::um::securitybaseapi::GetSecurityDescriptorDacl;
    use winapi::um::winbase::LocalFree;
    use winapi::um::winnt::{ACL, DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR};

    // D: DACL, P: protected (no inheritance from the parent), AI: auto-inherited to the children;
    // SY SYSTEM, BA Administrators, OW the owner (the user), all with full control, inherited by files (OI) and folders (CI)
    let sddl: Vec<u16> = "D:PAI(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)".encode_utf16().chain(Some(0)).collect();
    let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let mut sd: PSECURITY_DESCRIPTOR = null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(), 1, &mut sd, null_mut()) == 0 {
            return Err(format!("список доступа не собран (код {})", std::io::Error::last_os_error()));
        }
        let mut present: BOOL = FALSE;
        let mut defaulted: BOOL = FALSE;
        let mut dacl: *mut ACL = null_mut();
        let got = GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
        let result = if got == 0 || present == FALSE || dacl.is_null() {
            Err("в собранном описании нет списка доступа".to_string())
        } else {
            let code = SetNamedSecurityInfoW(wide.as_ptr() as *mut u16, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION, null_mut(), null_mut(), dacl, null_mut());
            if code == 0 { Ok(()) } else { Err(format!("права на папку не изменены (код {})", code)) }
        };
        LocalFree(sd as *mut _);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn the_folder_is_for_the_owner_alone_and_a_secret_file_too() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("data");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        harden_dir(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        let f = dir.join("token");
        std::fs::write(&f, "x").unwrap();
        std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o644)).unwrap();
        private_file(&f).unwrap();
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
    }

    // the secret of the window is written for the owner alone
    #[cfg(unix)]
    #[test]
    fn the_secret_of_the_window_is_a_private_file() {
        use std::os::unix::fs::PermissionsExt;
        let _g = crate::ui_auth::TEST_LOCK.lock();
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("ui_token");
        crate::ui_auth::reset_for_tests(Some(f.clone()), false);
        assert!(crate::ui_auth::init());
        assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        crate::ui_auth::reset_for_tests(None, false);
    }

    // the real access list, read back by the system tool (not our own code): nobody but SYSTEM, Administrators and the owner
    #[cfg(windows)]
    #[test]
    fn the_folder_has_a_protected_list_with_no_users_group_and_it_reaches_new_files() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("data");
        std::fs::create_dir(&dir).unwrap();
        // a file made BEFORE the change gets the new list as well
        let before = dir.join("before.txt");
        std::fs::write(&before, "x").unwrap();
        harden_dir(&dir).unwrap();
        let after = dir.join("after.txt");
        std::fs::write(&after, "x").unwrap();
        let show = |p: &Path| -> String {
            let o = std::process::Command::new("icacls").arg(p).output().unwrap();
            String::from_utf8_lossy(&o.stdout).to_string()
        };
        for p in [&dir, &before, &after] {
            let t = show(p);
            for foreign in ["Everyone", "BUILTIN\\Users", "Authenticated Users", "NT AUTHORITY\\INTERACTIVE"] {
                assert!(!t.contains(foreign), "{} lets {} in:\n{}", p.display(), foreign, t);
            }
            assert!(t.contains("NT AUTHORITY\\SYSTEM") && t.contains("BUILTIN\\Administrators"), "{}", t);
        }
        // the folder does not inherit (protected): none of its entries is marked (I), «inherited from the parent»
        assert!(!show(&dir).contains("(I)"), "{}", show(&dir));
    }
}
