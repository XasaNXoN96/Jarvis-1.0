//! The system window «Открыть файл» of Windows, for the button «Обзор…» of the Games page: the user goes to any drive and any folder,
//! chooses the program of a game and confirms. Only the path comes back; nothing is opened, read or started by this module.
//! (A shortcut that the user chooses is resolved by Windows to the program it points to.) Not Windows: there is no such window.

#[cfg(target_os = "windows")]
pub fn pick_exe() -> Result<Option<String>, String> {
    use winapi::um::combaseapi::{CoInitializeEx, CoUninitialize};
    use winapi::um::commdlg::{CommDlgExtendedError, GetOpenFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_PATHMUSTEXIST, OPENFILENAMEW};
    use winapi::um::objbase::COINIT_APARTMENTTHREADED;
    use winapi::um::winuser::GetForegroundWindow;
    let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
    let mut buf = vec![0u16; 32768];
    // «text\0pattern\0 … \0\0»
    let filter: Vec<u16> = "Программы игр (*.exe)\0*.exe\0\0".encode_utf16().collect();
    let title = wide("Выберите программу игры");
    unsafe {
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        // the window of Jarvis is the one in front: the dialog opens over it and not behind it
        ofn.hwndOwner = GetForegroundWindow();
        ofn.lpstrFilter = filter.as_ptr();
        ofn.nFilterIndex = 1;
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.Flags = OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_HIDEREADONLY | OFN_NOCHANGEDIR;
        let com = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED);
        let ok = GetOpenFileNameW(&mut ofn);
        let err = if ok == 0 { CommDlgExtendedError() } else { 0 };
        if com >= 0 {
            CoUninitialize();
        }
        if ok == 0 {
            // 0 = the user closed the window without choosing
            return if err == 0 { Ok(None) } else { Err(format!("Окно выбора файла не открылось (код {}).", err)) };
        }
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Ok(Some(String::from_utf16_lossy(&buf[..end])))
    }
}

#[cfg(not(target_os = "windows"))]
pub fn pick_exe() -> Result<Option<String>, String> {
    Err("Выбор файла через окно Windows работает только в Windows. Впишите полный путь вручную.".into())
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn outside_windows_the_dialog_says_so_and_does_not_hang() {
        assert!(super::pick_exe().unwrap_err().contains("только в Windows"));
    }
}
