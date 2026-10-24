// "Что на экране": a picture of the main screen for a question to the AI. Taken only when the user asks and only when
// "Анализ экрана" is allowed in the settings; the picture is kept in memory and a temporary file that is deleted at once.

// JPEG of the primary screen, at most `max_w` px wide. Taken by the program itself (GDI, no other program is started); only when the PowerShell way is
// the last resort it is used. The reason of a failure is told as it is (not the first line of a log).

// what PowerShell wrote to the error stream: «#< CLIXML» and then XML. Only the text of the errors is wanted; the first line is just a label.
pub fn clixml_reason(stderr: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    let mut rest = stderr;
    while let Some(i) = rest.find("S=\"Error\">") {
        let from = i + "S=\"Error\">".len();
        let Some(e) = rest[from..].find("</S>") else { break };
        let t = rest[from..from + e].replace("_x000D__x000A_", " ").replace("_x000D_", " ").replace("_x000A_", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&amp;", "&");
        let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() && !t.starts_with("At line") && !t.starts_with("+ ") && !parts.contains(&t) {
            parts.push(t);
        }
        rest = &rest[from + e..];
    }
    if parts.is_empty() {
        // not a CLIXML log: the first line that says something
        return stderr.lines().map(|l| l.trim()).find(|l| !l.is_empty() && !l.starts_with("#<")).unwrap_or("").chars().take(200).collect();
    }
    parts.join("; ").chars().take(300).collect()
}

#[cfg(target_os = "windows")]
mod native {
    use winapi::shared::windef::HDC;
    use winapi::um::libloaderapi::{GetModuleHandleA, GetProcAddress};
    use winapi::um::wingdi::*;
    use winapi::um::winuser::*;

    // «per monitor aware»: the real pixels of the screen, not the scaled ones (a program without a manifest of its own sees a screen made smaller by the
    // scaling of Windows). Only for this thread and only while the picture is taken; Windows older than 10 1703 do not have it: the old size is used then.
    unsafe fn dpi_aware_thread() -> Option<isize> {
        let user32 = GetModuleHandleA(b"user32.dll\0".as_ptr() as *const i8);
        if user32.is_null() {
            return None;
        }
        let f = GetProcAddress(user32, b"SetThreadDpiAwarenessContext\0".as_ptr() as *const i8);
        if f.is_null() {
            return None;
        }
        let set: extern "system" fn(isize) -> isize = std::mem::transmute(f);
        let old = set(-4); // DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
        if old == 0 { Some(set(-3)).filter(|o| *o != 0) } else { Some(old) }
    }

    unsafe fn restore_dpi(old: Option<isize>) {
        if let Some(o) = old {
            let user32 = GetModuleHandleA(b"user32.dll\0".as_ptr() as *const i8);
            if !user32.is_null() {
                let f = GetProcAddress(user32, b"SetThreadDpiAwarenessContext\0".as_ptr() as *const i8);
                if !f.is_null() {
                    let set: extern "system" fn(isize) -> isize = std::mem::transmute(f);
                    set(o);
                }
            }
        }
    }

    // the whole primary screen as BGRA
    pub fn grab() -> Result<(Vec<u8>, usize, usize), String> {
        unsafe {
            let old = dpi_aware_thread();
            let r = grab_inner();
            restore_dpi(old);
            r
        }
    }

    unsafe fn grab_inner() -> Result<(Vec<u8>, usize, usize), String> {
        let (w, h) = (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        if w <= 0 || h <= 0 || w > 16384 || h > 16384 {
            return Err("размер экрана не определён".into());
        }
        let hdc: HDC = GetDC(std::ptr::null_mut());
        if hdc.is_null() {
            return Err("экран не открывается для снимка (возможно, компьютер заблокирован или идёт вход в Windows)".into());
        }
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, w, h);
        if mem.is_null() || bmp.is_null() {
            if !bmp.is_null() { DeleteObject(bmp as _); }
            if !mem.is_null() { DeleteDC(mem); }
            ReleaseDC(std::ptr::null_mut(), hdc);
            return Err("не хватает памяти для снимка экрана".into());
        }
        let old = SelectObject(mem, bmp as _);
        let mut ok = BitBlt(mem, 0, 0, w, h, hdc, 0, 0, SRCCOPY | CAPTUREBLT) != 0;
        let mut buf = vec![0u8; (w as usize) * (h as usize) * 4];
        if ok {
            let mut bi: BITMAPINFO = std::mem::zeroed();
            bi.bmiHeader = BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w, biHeight: -h, biPlanes: 1, biBitCount: 32, biCompression: BI_RGB, biSizeImage: 0, biXPelsPerMeter: 0, biYPelsPerMeter: 0, biClrUsed: 0, biClrImportant: 0 };
            ok = GetDIBits(mem, bmp, 0, h as u32, buf.as_mut_ptr() as *mut _, &mut bi, DIB_RGB_COLORS) != 0;
        }
        SelectObject(mem, old);
        DeleteObject(bmp as _);
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), hdc);
        if !ok {
            return Err("Windows не отдал картинку экрана (заблокированный экран или защищённое окно)".into());
        }
        Ok((buf, w as usize, h as usize))
    }
}

// BGRA of the screen → a JPEG of at most `max_w` px wide (pure, tested)
pub fn jpeg_from_bgra(bgra: &[u8], w: usize, h: usize, max_w: u32, quality: u8) -> Result<Vec<u8>, String> {
    if w == 0 || h == 0 || bgra.len() < w * h * 4 {
        return Err("пустая картинка экрана".into());
    }
    let mut rgb = Vec::with_capacity(w * h * 3);
    for px in bgra[..w * h * 4].chunks_exact(4) {
        rgb.extend_from_slice(&[px[2], px[1], px[0]]);
    }
    let img = image::RgbImage::from_raw(w as u32, h as u32, rgb).ok_or("картинка экрана повреждена")?;
    let img = if img.width() > max_w {
        let nh = ((img.height() as u64 * max_w as u64) / img.width() as u64).max(1) as u32;
        image::imageops::resize(&img, max_w, nh, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, quality).encode_image(&img).map_err(|e| format!("снимок не сжат: {}", e))?;
    Ok(out)
}

#[cfg(target_os = "windows")]
pub fn capture_sized(max_w: u32, quality: u8) -> Result<Vec<u8>, String> {
    match native::grab().and_then(|(b, w, h)| jpeg_from_bgra(&b, w, h, max_w, quality)) {
        Ok(j) => Ok(j),
        // the last resort: the old way through PowerShell (its own reason is told with the first one)
        Err(first) => capture_powershell().map_err(|second| format!("снимок экрана не получился: {}; запасной способ: {}", first, second)),
    }
}

// the picture for a question to the AI and for the phone: 1600 px
#[cfg(target_os = "windows")]
pub fn capture() -> Result<Vec<u8>, String> {
    capture_sized(1600, 70)
}

#[cfg(target_os = "windows")]
fn capture_powershell() -> Result<Vec<u8>, String> {
    // a name nobody can guess (a planted file or link of the same name would be written through)
    let mut nonce = [0u8; 8];
    {
        use rand::RngCore;
        rand::rngs::OsRng.fill_bytes(&mut nonce);
    }
    let path = std::env::temp_dir().join(format!("jarvis_screen_{}_{}.jpg", std::process::id(), nonce.iter().map(|b| format!("{:02x}", b)).collect::<String>()));
    let out = crate::psrun::quiet(crate::psrun::Script::Screen, &[("JARVIS_SCREEN_OUT", path.display().to_string())])
        .map_err(|e| format!("снимок экрана отклонён защитой: {}", e))?
        .output()
        .map_err(|e| format!("PowerShell не запустился: {}", e))?;
    let data = std::fs::read(&path);
    let _ = std::fs::remove_file(&path);
    match data {
        Ok(d) if d.len() > 1000 => Ok(d),
        _ => {
            let why = clixml_reason(&String::from_utf8_lossy(&out.stderr));
            Err(if why.is_empty() { "PowerShell не создал картинку".to_string() } else { why })
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn capture_sized(_max_w: u32, _quality: u8) -> Result<Vec<u8>, String> {
    Err("снимок экрана доступен только в Windows".into())
}

#[cfg(not(target_os = "windows"))]
pub fn capture() -> Result<Vec<u8>, String> {
    Err("снимок экрана доступен только в Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_real_reason_is_taken_out_of_the_log_of_powershell() {
        let log = "#< CLIXML\n<Objs Version=\"1.1.0.1\" xmlns=\"http://schemas.microsoft.com/powershell/2004/04\"><S S=\"Error\">Add-Type : Cannot add type. Compilation errors_x000D__x000A_</S><S S=\"Error\">At line:1 char:1_x000D__x000A_</S><S S=\"Error\">+ Add-Type -MemberDefinition_x000D__x000A_</S><S S=\"Error\">Exception calling &quot;CopyFromScreen&quot; with &quot;3&quot; argument(s): &quot;The handle is invalid&quot;_x000D__x000A_</S></Objs>";
        let r = clixml_reason(log);
        assert!(r.contains("Cannot add type") && r.contains("The handle is invalid") && !r.contains("CLIXML") && !r.contains("At line") && !r.contains("_x000D_"), "{}", r);
        assert_eq!(clixml_reason("простая ошибка\nвторая"), "простая ошибка");
        assert_eq!(clixml_reason("#< CLIXML\n"), "");
        assert_eq!(clixml_reason(""), "");
    }

    #[test]
    fn the_picture_of_the_screen_is_a_small_jpeg_of_the_right_size() {
        // 200x100 of one colour: B=10, G=120, R=240 (BGRA)
        let (w, h) = (200usize, 100usize);
        let bgra: Vec<u8> = (0..w * h).flat_map(|_| [10u8, 120, 240, 255]).collect();
        let j = jpeg_from_bgra(&bgra, w, h, 100, 60).unwrap();
        assert_eq!(&j[..2], &[0xFF, 0xD8], "a JPEG");
        let img = image::load_from_memory_with_format(&j, image::ImageFormat::Jpeg).unwrap().to_rgb8();
        assert_eq!((img.width(), img.height()), (100, 50), "narrowed to the asked width, the proportion kept");
        let px = img.get_pixel(50, 25);
        assert!(px[0] > 200 && px[2] < 40, "red stays red (the order of the colours is right): {:?}", px);
        // not enlarged
        let big = jpeg_from_bgra(&bgra, w, h, 1600, 60).unwrap();
        assert_eq!(image::load_from_memory_with_format(&big, image::ImageFormat::Jpeg).unwrap().width(), 200);
        assert!(jpeg_from_bgra(&bgra[..100], w, h, 100, 60).is_err() && jpeg_from_bgra(&[], 0, 0, 100, 60).is_err());
    }
}
