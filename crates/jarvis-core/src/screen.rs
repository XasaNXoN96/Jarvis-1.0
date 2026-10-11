// "Что на экране": a picture of the main screen for a question to the AI. Taken only when the user asks and only when
// "Анализ экрана" is allowed in the settings; the picture is kept in memory and a temporary file that is deleted at once.

// JPEG of the primary screen, at most 1600 px wide (enough to read the interface, small enough to send)
#[cfg(target_os = "windows")]
pub fn capture() -> Result<Vec<u8>, String> {
    use std::os::windows::process::CommandExt;
    let path = std::env::temp_dir().join(format!("jarvis_screen_{}.jpg", std::process::id()));
    let script = format!(
        "Add-Type -AssemblyName System.Windows.Forms,System.Drawing; \
         $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; \
         $bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height; \
         $g = [System.Drawing.Graphics]::FromImage($bmp); $g.CopyFromScreen($b.Location, [System.Drawing.Point]::Empty, $b.Size); \
         $w = [Math]::Min(1600, $b.Width); $h = [int]($b.Height * $w / $b.Width); \
         $small = New-Object System.Drawing.Bitmap $bmp, $w, $h; \
         $enc = [System.Drawing.Imaging.ImageCodecInfo]::GetImageEncoders() | Where-Object {{ $_.MimeType -eq 'image/jpeg' }}; \
         $p = New-Object System.Drawing.Imaging.EncoderParameters 1; \
         $p.Param[0] = New-Object System.Drawing.Imaging.EncoderParameter ([System.Drawing.Imaging.Encoder]::Quality), 70L; \
         $small.Save('{}', $enc, $p)",
        path.display().to_string().replace('\'', "''")
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(0x0800_0000)
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
