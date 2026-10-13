// "Что на экране": a picture of the main screen for a question to the AI. Taken only when the user asks and only when
// "Анализ экрана" is allowed in the settings; the picture is kept in memory and a temporary file that is deleted at once.

// JPEG of the primary screen, at most 1600 px wide (enough to read the interface, small enough to send)
#[cfg(target_os = "windows")]
pub fn capture() -> Result<Vec<u8>, String> {
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
        _ => Err(format!("снимок экрана не получился{}", if out.stderr.is_empty() { String::new() } else { format!(": {}", String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or("")) })),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn capture() -> Result<Vec<u8>, String> {
    Err("снимок экрана доступен только в Windows".into())
}
