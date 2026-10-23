//! The icons of the games of the library. Where an icon comes from, in this order:
//!   1. the picture that was made before and kept in `game_icons/` of the data folder (named by the id of the game);
//!   2. the icon of the program file of the game, taken by Windows (the file is only read; Windows only);
//!   3. for a Steam game: a picture that Steam already keeps on this computer (`appcache/librarycache`), when it is there;
//!   4. nothing: the page draws its own plain game icon.
//! Nothing is downloaded from the Internet. No file of a game is changed, and no rights of an administrator are asked.

use crate::gamelist;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::PathBuf;

// the ids that gave no icon in this run (with the path they had): the page asks again after a restart or when the path changes
static NONE: Lazy<Mutex<HashSet<String>>> = Lazy::new(|| Mutex::new(HashSet::new()));
// one picture is made at a time: a page with two hundred games asks for two hundred icons at once
static MAKING: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

fn dir() -> PathBuf {
    #[cfg(test)]
    {
        return std::env::temp_dir().join(format!("jarvis-icons-{}", std::process::id()));
    }
    #[cfg(not(test))]
    {
        crate::APP_DIR.join("game_icons")
    }
}

// the id goes into a file name: only what the library itself makes (letters, digits, - and _)
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 40 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

pub fn sniff(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else {
        None
    }
}

fn cached(id: &str) -> Option<Vec<u8>> {
    for ext in ["png", "jpg"] {
        if let Ok(d) = std::fs::read(dir().join(format!("{}.{}", id, ext))) {
            if sniff(&d).is_some() {
                return Some(d);
            }
        }
    }
    None
}

fn keep(id: &str, data: &[u8]) {
    let ext = if sniff(data) == Some("image/jpeg") { "jpg" } else { "png" };
    let d = dir();
    if std::fs::create_dir_all(&d).is_ok() {
        let _ = crate::write_atomic(&d.join(format!("{}.{}", id, ext)), data);
    }
}

// the picture of the game (the bytes and their type), made once and kept; None: the page draws the plain icon
pub fn icon_for(id: &str) -> Option<(Vec<u8>, &'static str)> {
    if !safe_id(id) {
        return None;
    }
    if let Some(d) = cached(id) {
        let t = sniff(&d)?;
        return Some((d, t));
    }
    let e = gamelist::get(id)?;
    let key = format!("{}|{}", id, e.path);
    if NONE.lock().contains(&key) {
        return None;
    }
    let _one = MAKING.lock();
    // (another request may have made it while this one waited)
    if let Some(d) = cached(id) {
        let t = sniff(&d)?;
        return Some((d, t));
    }
    let made = (!e.path.is_empty() && std::path::Path::new(&e.path).is_file()).then(|| exe_icon_png(&e.path)).flatten().or_else(|| steam_picture(&e.store_id));
    match made {
        Some(d) => {
            keep(id, &d);
            let t = sniff(&d)?;
            Some((d, t))
        }
        None => {
            NONE.lock().insert(key);
            None
        }
    }
}

// the kept picture of a game is forgotten (the path changed, the game left the library). Only the file of this program's own folder
// that is named by the id is removed; nothing of the game.
pub fn forget(id: &str) {
    if !safe_id(id) {
        return;
    }
    for ext in ["png", "jpg"] {
        let _ = std::fs::remove_file(dir().join(format!("{}.{}", id, ext)));
    }
    NONE.lock().retain(|k| !k.starts_with(&format!("{}|", id)));
}

// a picture that Steam keeps for a game: the small icon, else the cover; the old and the new places of the library cache
fn steam_picture(store_id: &str) -> Option<Vec<u8>> {
    let app = store_id.strip_prefix("steam:")?;
    if app.is_empty() || !app.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let root = crate::gamescan::steam_root()?;
    let cache = root.join("appcache").join("librarycache");
    let names = [format!("{}_icon.jpg", app), format!("{}_library_600x900.jpg", app), format!("{}_header.jpg", app)];
    let mut places: Vec<PathBuf> = names.iter().map(|n| cache.join(n)).collect();
    // the newer layout: a folder per application, the pictures named by a hash
    if let Ok(rd) = std::fs::read_dir(cache.join(app)) {
        for f in rd.flatten().take(40) {
            places.push(f.path());
        }
    }
    places.into_iter().filter_map(|p| std::fs::read(p).ok()).find(|d| d.len() < 2_000_000 && sniff(d).is_some())
}

// ---------------------------------------------------------------------------------------------
// the icon of a program file (Windows) and the PNG writer
// ---------------------------------------------------------------------------------------------
fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
    }
    !c
}

// RGBA (8 bits, top row first) -> a PNG
pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    if w == 0 || h == 0 || rgba.len() != (w as usize) * (h as usize) * 4 {
        return None;
    }
    let mut raw = Vec::with_capacity((w as usize * 4 + 1) * h as usize);
    for row in rgba.chunks(w as usize * 4) {
        raw.push(0); // no filter
        raw.extend_from_slice(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    z.write_all(&raw).ok()?;
    let idat = z.finish().ok()?;
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], body: &[u8]| {
        out.extend_from_slice(&(body.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(body);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8 bits, RGBA
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &idat);
    chunk(b"IEND", &[]);
    Some(out)
}

// (user32 has it, the winapi crate does not declare it): the icon of the size asked for from a program file
#[cfg(target_os = "windows")]
#[link(name = "user32")]
extern "system" {
    fn PrivateExtractIconsW(file: *const u16, index: i32, cx: i32, cy: i32, icons: *mut winapi::shared::windef::HICON, ids: *mut u32, count: u32, flags: u32) -> u32;
}

#[cfg(target_os = "windows")]
pub(crate) fn exe_icon_png(path: &str) -> Option<Vec<u8>> {
    use winapi::shared::windef::HICON;
    use winapi::um::wingdi::{CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
    use winapi::um::winuser::{DestroyIcon, GetIconInfo, ICONINFO};
    let wpath: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        // the biggest picture that the file has, up to 128 px
        let mut icon: HICON = std::ptr::null_mut();
        let mut icon_id = 0u32;
        let n = PrivateExtractIconsW(wpath.as_ptr(), 0, 128, 128, &mut icon, &mut icon_id, 1, 0);
        if n == 0 || n == u32::MAX || icon.is_null() {
            return None;
        }
        let mut info: ICONINFO = std::mem::zeroed();
        let got = GetIconInfo(icon, &mut info);
        let result = (|| {
            if got == 0 || info.hbmColor.is_null() {
                return None;
            }
            let mut bm: BITMAP = std::mem::zeroed();
            if GetObjectW(info.hbmColor as _, std::mem::size_of::<BITMAP>() as i32, &mut bm as *mut _ as *mut _) == 0 {
                return None;
            }
            let (w, h) = (bm.bmWidth as u32, bm.bmHeight as u32);
            if w == 0 || h == 0 || w > 256 || h > 256 {
                return None;
            }
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let read = |bitmap| -> Option<Vec<u8>> {
                let mut bi: BITMAPINFO = std::mem::zeroed();
                bi.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w as i32, biHeight: -(h as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, ..std::mem::zeroed() };
                let mut buf = vec![0u8; (w * h * 4) as usize];
                let lines = GetDIBits(dc, bitmap, 0, h, buf.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS);
                (lines as u32 == h).then_some(buf)
            };
            let color = read(info.hbmColor);
            let mask = if info.hbmMask.is_null() { None } else { read(info.hbmMask) };
            DeleteDC(dc);
            let mut px = color?;
            // an old icon has no alpha channel: its mask says what is see-through (black = the picture, white = nothing)
            if px.chunks(4).all(|p| p[3] == 0) {
                let mask = mask?;
                for (p, m) in px.chunks_mut(4).zip(mask.chunks(4)) {
                    p[3] = if m[0] == 0 { 255 } else { 0 };
                }
            }
            // BGRA -> RGBA
            for p in px.chunks_mut(4) {
                p.swap(0, 2);
            }
            encode_png(w, h, &px)
        })();
        if !info.hbmColor.is_null() {
            DeleteObject(info.hbmColor as _);
        }
        if !info.hbmMask.is_null() {
            DeleteObject(info.hbmMask as _);
        }
        DestroyIcon(icon);
        result
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn exe_icon_png(_path: &str) -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gamemode::Genre;

    #[test]
    fn a_png_is_written_that_a_reader_can_unpack_back_to_the_same_pixels() {
        use std::io::Read;
        let (w, h) = (3u32, 2u32);
        let px: Vec<u8> = (0..(w * h * 4) as u8).collect();
        let png = encode_png(w, h, &px).unwrap();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        // walk the chunks: every CRC is right; the picture inside is the one that was given
        let (mut i, mut idat, mut kinds) = (8, Vec::new(), Vec::new());
        while i < png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let body = &png[i + 4..i + 8 + len];
            assert_eq!(u32::from_be_bytes(png[i + 8 + len..i + 12 + len].try_into().unwrap()), crc32(body), "crc of {:?}", &body[..4]);
            kinds.push(String::from_utf8_lossy(&body[..4]).to_string());
            if &body[..4] == b"IDAT" {
                idat.extend_from_slice(&body[4..]);
            }
            if &body[..4] == b"IHDR" {
                assert_eq!((u32::from_be_bytes(body[4..8].try_into().unwrap()), u32::from_be_bytes(body[8..12].try_into().unwrap()), body[12], body[13]), (w, h, 8, 6));
            }
            i += 12 + len;
        }
        assert_eq!(kinds, vec!["IHDR", "IDAT", "IEND"]);
        let mut raw = Vec::new();
        flate2::read::ZlibDecoder::new(&idat[..]).read_to_end(&mut raw).unwrap();
        assert_eq!(raw.len(), ((w * 4 + 1) * h) as usize);
        assert_eq!(&raw[1..1 + 12], &px[..12]);
        assert_eq!(raw[0], 0);
        assert!(encode_png(2, 2, &[0; 3]).is_none() && encode_png(0, 2, &[]).is_none());
        // the check value of the standard (the CRC-32 of "123456789")
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn an_icon_is_made_once_kept_forgotten_and_a_bad_id_never_reaches_a_file_name() {
        gamelist::test_reset();
        let _ = std::fs::remove_dir_all(dir());
        // a game with no file and no store: no icon, and the page draws its own; nothing is made again for the same path
        let (_, id) = gamelist::add_entry(gamelist::Entry::basic("a.exe", "A", Genre::Action, "voice"), false).unwrap();
        assert!(icon_for(&id).is_none());
        assert!(NONE.lock().iter().any(|k| k.starts_with(&format!("{}|", id))));
        // a kept picture is given back (the order: kept first)
        let png = encode_png(1, 1, &[255, 0, 0, 255]).unwrap();
        keep(&id, &png);
        let (bytes, kind) = icon_for(&id).unwrap();
        assert_eq!((bytes, kind), (png.clone(), "image/png"));
        // forgotten: the file of the picture is gone and nothing else
        let other = dir().join("other.txt");
        std::fs::write(&other, b"x").unwrap();
        forget(&id);
        assert!(cached(&id).is_none());
        assert!(other.exists());
        // ids with a way up or a slash do not make a path
        for bad in ["../x", "a/b", "a\\b", "", "g1;rm", &"a".repeat(41)] {
            assert!(icon_for(bad).is_none(), "{}", bad);
            forget(bad);
        }
        std::fs::remove_dir_all(dir()).ok();
        gamelist::test_reset();
    }

    #[test]
    fn a_steam_picture_that_is_already_on_the_computer_is_recognised_by_its_content_not_by_its_name() {
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(sniff(b"\x89PNG\r\n"), Some("image/png"));
        assert_eq!(sniff(b"MZ\x90\x00"), None);
        assert!(steam_picture("epic:1").is_none());
        assert!(steam_picture("steam:12 34").is_none());
        assert!(steam_picture("steam:").is_none());
    }
}
