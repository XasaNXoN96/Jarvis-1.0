// File operations of commands (Lua `jarvis.fs`) that cannot be redirected between the check and the operation.
//
// The problem: policy.rs judges a PATH; between that judgement and the write a folder of the path may be replaced by a
// link (or a file by a link into Windows, into Jarvis settings, ...) and the bytes go where the policy never looked.
//
// The answer, in three steps, the same on every system:
//   1. the real place of the path is judged (links followed: policy::judge_fs_real);
//   2. the FOLDER that will hold the file is opened and kept open: on Unix by walking every component with
//      openat(O_NOFOLLOW) (a link anywhere in the way is refused, and the operation goes through that folder handle:
//      the file name is only ever resolved INSIDE the held folder); on Windows by a handle without FILE_SHARE_DELETE
//      (the folder cannot be renamed, deleted or replaced while it is held; a reparse point is refused);
//   3. the real place of the folder that is HELD is read back from the handle and the policy judges it a second time.
//      So what is judged is the object that is written, not the spelling.
// The last name is never followed: a file is replaced by a temporary file created with O_EXCL / CREATE_NEW and moved
// over it (a link or a hard link of that name is replaced, its target is not touched); an append refuses a link and a
// file with more than one hard link (a hard link to a protected file would be written through).
//
// What this does NOT cover (said plainly, also in the report):
//   * a process of the same user that already may write into the allowed folder can still change the CONTENT of files
//     there, and can race the folder itself on a system without a held handle (Windows: see the tests; Unix: the held
//     folder is the judged object, a moved folder is written where it stands, never somewhere else);
//   * only the file functions of Lua go through here; Jarvis's own files (settings, timers, state of a pack) are written
//     by the program to places it chooses itself;
//   * on a network share or a file system without reparse/hard link information the Windows checks depend on what the
//     file system reports.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::policy::{self, FsMode};

#[derive(Debug)]
pub enum SafeError {
    Denied(String),
    Io(std::io::Error),
}

impl SafeError {
    // the text for a Lua error: the denial as the policy words it, an I/O error with the name of the operation
    pub fn lua_text(&self, label: &str) -> String {
        match self {
            SafeError::Denied(why) => format!("Запрещено правилами безопасности: {}", why),
            SafeError::Io(e) => format!("{} error: {}", label, e),
        }
    }
}

impl std::fmt::Display for SafeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SafeError::Denied(why) => write!(f, "{}", why),
            SafeError::Io(e) => write!(f, "{}", e),
        }
    }
}

impl From<std::io::Error> for SafeError {
    fn from(e: std::io::Error) -> Self {
        SafeError::Io(e)
    }
}

fn denied<T>(why: String) -> Result<T, SafeError> {
    Err(SafeError::Denied(why))
}

// tests act here: after the folder is held and before the second judgement they swap the place (see the tests)
#[cfg(test)]
thread_local! {
    static BETWEEN: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = std::cell::RefCell::new(None);
}
#[cfg(test)]
fn set_between(f: impl FnOnce() + 'static) {
    BETWEEN.with(|b| *b.borrow_mut() = Some(Box::new(f)));
}
fn between() {
    #[cfg(test)]
    if let Some(f) = BETWEEN.with(|b| b.borrow_mut().take()) {
        f();
    }
}

// the held folder, the name inside it and the real place of the name, judged twice
fn prepare(path: &Path, mode: FsMode, create_parents: bool) -> Result<(imp::Dir, OsString, PathBuf), SafeError> {
    let c = policy::real_path(path).or_else(denied)?;
    policy::judge_fs_real(&c, mode).or_else(denied)?;
    let (Some(parent), Some(name)) = (c.parent(), c.file_name()) else {
        return denied(format!("«{}» не файл", c.display()));
    };
    let dir = imp::open_dir_chain(parent, create_parents)?;
    between();
    let target = dir.real_path()?.join(name);
    policy::judge_fs_real(&target, mode).or_else(denied)?;
    Ok((dir, name.to_os_string(), target))
}

// replaces the file whole (creates the folders of the path)
pub fn write(path: &Path, data: &[u8]) -> Result<(), SafeError> {
    let (dir, name, _) = prepare(path, FsMode::Write, true)?;
    dir.write_replace(&name, data)?;
    Ok(())
}

// creates a NEW file: an existing one is never replaced (the error is `AlreadyExists`)
pub fn create_new(path: &Path, data: &[u8]) -> Result<(), SafeError> {
    use std::io::Write;
    let (dir, name, target) = prepare(path, FsMode::Write, true)?;
    // (the handle of the folder stays open while the file is made)
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
    f.write_all(data)?;
    f.flush()?;
    drop(dir);
    let _ = name;
    Ok(())
}

pub fn append(path: &Path, data: &[u8]) -> Result<(), SafeError> {
    let (dir, name, _) = prepare(path, FsMode::Write, true)?;
    dir.append(&name, data)?;
    Ok(())
}

// a file or an EMPTY folder (a command never deletes a tree)
pub fn remove(path: &Path) -> Result<(), SafeError> {
    let (dir, name, _) = prepare(path, FsMode::Remove, false)?;
    dir.remove(&name)?;
    Ok(())
}

pub fn mkdir_all(path: &Path) -> Result<(), SafeError> {
    let c = policy::real_path(path).or_else(denied)?;
    policy::judge_fs_real(&c, FsMode::Write).or_else(denied)?;
    let dir = imp::open_dir_chain(&c, true)?;
    between();
    let real = dir.real_path()?;
    policy::judge_fs_real(&real, FsMode::Write).or_else(denied)?;
    Ok(())
}

// the file is opened first and the place of the OPENED file is judged (a link is followed on reading: the judgement is
// about where it really led)
pub fn read(path: &Path) -> Result<Vec<u8>, SafeError> {
    use std::io::Read;
    let c = policy::real_path(path).or_else(denied)?;
    policy::judge_fs_real(&c, FsMode::Read).or_else(denied)?;
    between();
    let mut f = imp::open_read(&c)?;
    let real = imp::file_real_path(&f)?;
    policy::judge_fs_real(&real, FsMode::Read).or_else(denied)?;
    let mut out = Vec::new();
    f.read_to_end(&mut out)?;
    Ok(out)
}

fn temp_name(name: &std::ffi::OsStr) -> OsString {
    use rand::RngCore;
    let mut n = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut n);
    let mut t = OsString::from(".");
    t.push(name);
    t.push(format!(".{}.tmp", n.iter().map(|b| format!("{:02x}", b)).collect::<String>()));
    t
}

#[cfg(unix)]
mod imp {
    use super::temp_name;
    use std::ffi::{CString, OsStr, OsString};
    use std::fs::File;
    use std::io::{self, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::io::{AsRawFd, FromRawFd, RawFd};
    use std::path::{Component, Path, PathBuf};

    fn cstr(s: &OsStr) -> io::Result<CString> {
        CString::new(s.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "имя с нулевым байтом"))
    }

    fn check(r: i32) -> io::Result<i32> {
        if r < 0 { Err(io::Error::last_os_error()) } else { Ok(r) }
    }

    pub struct Dir {
        fd: RawFd,
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            unsafe { libc::close(self.fd) };
        }
    }

    const DIR_FLAGS: i32 = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;

    // every component is opened from the one before with O_NOFOLLOW: a link anywhere in the way is an error (ELOOP)
    pub fn open_dir_chain(path: &Path, create: bool) -> io::Result<Dir> {
        let root = cstr(OsStr::new("/"))?;
        let mut fd = check(unsafe { libc::open(root.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) })?;
        for comp in path.components() {
            match comp {
                Component::RootDir => {}
                Component::Normal(n) => {
                    let name = cstr(n)?;
                    let mut next = unsafe { libc::openat(fd, name.as_ptr(), DIR_FLAGS) };
                    if next < 0 && create && io::Error::last_os_error().kind() == io::ErrorKind::NotFound {
                        let made = unsafe { libc::mkdirat(fd, name.as_ptr(), 0o777) };
                        if made == 0 || io::Error::last_os_error().kind() == io::ErrorKind::AlreadyExists {
                            next = unsafe { libc::openat(fd, name.as_ptr(), DIR_FLAGS) };
                        }
                    }
                    let err = io::Error::last_os_error();
                    unsafe { libc::close(fd) };
                    if next < 0 {
                        return Err(err);
                    }
                    fd = next;
                }
                _ => {
                    unsafe { libc::close(fd) };
                    return Err(io::Error::new(io::ErrorKind::InvalidInput, "путь не в каноническом виде"));
                }
            }
        }
        Ok(Dir { fd })
    }

    #[cfg(target_os = "linux")]
    fn fd_path(fd: RawFd) -> io::Result<PathBuf> {
        std::fs::read_link(format!("/proc/self/fd/{}", fd))
    }
    // (without /proc the held object cannot be named: the path of the walk is the best there is; the product is Windows)
    #[cfg(not(target_os = "linux"))]
    fn fd_path(_fd: RawFd) -> io::Result<PathBuf> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "место открытой папки неизвестно"))
    }

    pub fn file_real_path(f: &File) -> io::Result<PathBuf> {
        fd_path(f.as_raw_fd())
    }

    pub fn open_read(path: &Path) -> io::Result<File> {
        File::open(path)
    }

    fn stat_at(fd: RawFd, name: &CString) -> io::Result<libc::stat> {
        let mut st: libc::stat = unsafe { std::mem::zeroed() };
        check(unsafe { libc::fstatat(fd, name.as_ptr(), &mut st, libc::AT_SYMLINK_NOFOLLOW) })?;
        Ok(st)
    }

    fn open_in(fd: RawFd, name: &CString, flags: i32) -> io::Result<File> {
        let f = check(unsafe { libc::openat(fd, name.as_ptr(), flags | libc::O_NOFOLLOW | libc::O_CLOEXEC, 0o666 as libc::c_uint) })?;
        Ok(unsafe { File::from_raw_fd(f) })
    }

    impl Dir {
        pub fn real_path(&self) -> io::Result<PathBuf> {
            fd_path(self.fd)
        }

        pub fn write_replace(&self, name: &OsStr, data: &[u8]) -> io::Result<()> {
            let cname = cstr(name)?;
            // (a folder of that name is not replaced by a file; a link or a file is)
            if let Ok(st) = stat_at(self.fd, &cname) {
                if st.st_mode & libc::S_IFMT == libc::S_IFDIR {
                    return Err(io::Error::new(io::ErrorKind::Other, "это папка"));
                }
            }
            let tmp: OsString = temp_name(name);
            let ctmp = cstr(&tmp)?;
            let mut f = open_in(self.fd, &ctmp, libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL)?;
            let res = f.write_all(data).and_then(|_| f.flush()).and_then(|_| check(unsafe { libc::renameat(self.fd, ctmp.as_ptr(), self.fd, cname.as_ptr()) }).map(|_| ()));
            if res.is_err() {
                unsafe { libc::unlinkat(self.fd, ctmp.as_ptr(), 0) };
            }
            res
        }

        pub fn append(&self, name: &OsStr, data: &[u8]) -> io::Result<()> {
            let cname = cstr(name)?;
            let mut f = open_in(self.fd, &cname, libc::O_WRONLY | libc::O_APPEND | libc::O_CREAT)?;
            let mut st: libc::stat = unsafe { std::mem::zeroed() };
            check(unsafe { libc::fstat(f.as_raw_fd(), &mut st) })?;
            if st.st_mode & libc::S_IFMT != libc::S_IFREG {
                return Err(io::Error::new(io::ErrorKind::Other, "это не обычный файл"));
            }
            if st.st_nlink > 1 {
                return Err(io::Error::new(io::ErrorKind::Other, "у файла несколько жёстких ссылок: запись отклонена"));
            }
            f.write_all(data)
        }

        pub fn remove(&self, name: &OsStr) -> io::Result<()> {
            let cname = cstr(name)?;
            let st = stat_at(self.fd, &cname)?;
            let flag = if st.st_mode & libc::S_IFMT == libc::S_IFDIR { libc::AT_REMOVEDIR } else { 0 };
            check(unsafe { libc::unlinkat(self.fd, cname.as_ptr(), flag) }).map(|_| ())
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::temp_name;
    use std::ffi::OsStr;
    use std::fs::{File, OpenOptions};
    use std::io::{self, Write};
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::fs::{FileTypeExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::path::{Path, PathBuf};

    const FILE_SHARE_READ: u32 = 1;
    const FILE_SHARE_WRITE: u32 = 2;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    // the folder is held WITHOUT FILE_SHARE_DELETE: while the handle lives nobody can rename, delete or replace it
    // (nor a folder above it, which would have to move the held one along)
    pub struct Dir {
        _held: File,
        path: PathBuf,
    }

    pub fn file_real_path(f: &File) -> io::Result<PathBuf> {
        use winapi::um::fileapi::GetFinalPathNameByHandleW;
        let mut buf = vec![0u16; 1024];
        loop {
            let n = unsafe { GetFinalPathNameByHandleW(f.as_raw_handle() as _, buf.as_mut_ptr(), buf.len() as u32, 0) } as usize;
            if n == 0 {
                return Err(io::Error::last_os_error());
            }
            if n < buf.len() {
                let s = std::ffi::OsString::from_wide(&buf[..n]).to_string_lossy().to_string();
                return Ok(PathBuf::from(s.strip_prefix("\\\\?\\").map(|r| r.to_string()).unwrap_or(s)));
            }
            buf = vec![0u16; n + 1];
        }
    }

    fn links(f: &File) -> io::Result<u32> {
        use winapi::um::fileapi::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe { GetFileInformationByHandle(f.as_raw_handle() as _, &mut info) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(info.nNumberOfLinks)
    }

    pub fn open_dir_chain(path: &Path, create: bool) -> io::Result<Dir> {
        if create {
            std::fs::create_dir_all(path)?;
        }
        let held = OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        use std::os::windows::fs::MetadataExt;
        let meta = held.metadata()?;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !meta.is_dir() {
            return Err(io::Error::new(io::ErrorKind::Other, "на пути ссылка (точка повторной обработки): отклонено"));
        }
        let real = file_real_path(&held)?;
        Ok(Dir { _held: held, path: real })
    }

    pub fn open_read(path: &Path) -> io::Result<File> {
        OpenOptions::new().read(true).share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE).open(path)
    }

    impl Dir {
        pub fn real_path(&self) -> io::Result<PathBuf> {
            Ok(self.path.clone())
        }

        pub fn write_replace(&self, name: &OsStr, data: &[u8]) -> io::Result<()> {
            let target = self.path.join(name);
            if let Ok(m) = std::fs::symlink_metadata(&target) {
                if m.is_dir() {
                    return Err(io::Error::new(io::ErrorKind::Other, "это папка"));
                }
            }
            let tmp = self.path.join(temp_name(name));
            let mut f = OpenOptions::new().write(true).create_new(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&tmp)?;
            let res = f.write_all(data).and_then(|_| f.flush()).and_then(|_| {
                drop(f);
                std::fs::rename(&tmp, &target)
            });
            if res.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            res
        }

        pub fn append(&self, name: &OsStr, data: &[u8]) -> io::Result<()> {
            use std::os::windows::fs::MetadataExt;
            let target = self.path.join(name);
            let mut f = OpenOptions::new().append(true).create(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(&target)?;
            let meta = f.metadata()?;
            if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 || !meta.is_file() {
                return Err(io::Error::new(io::ErrorKind::Other, "это ссылка или не обычный файл: запись отклонена"));
            }
            if links(&f)? > 1 {
                return Err(io::Error::new(io::ErrorKind::Other, "у файла несколько жёстких ссылок: запись отклонена"));
            }
            f.write_all(data)
        }

        pub fn remove(&self, name: &OsStr) -> io::Result<()> {
            let target = self.path.join(name);
            let m = std::fs::symlink_metadata(&target)?;
            if m.is_dir() || m.file_type().is_symlink_dir() {
                std::fs::remove_dir(&target)
            } else {
                std::fs::remove_file(&target)
            }
        }
    }
}

#[cfg(test)]
mod tests;
