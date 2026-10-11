// Embeds the program icon into jarvis-app.exe (taskbar, Explorer, shortcuts, Alt+Tab)
fn main() {
    println!("cargo:rerun-if-changed=../../resources/icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() == "windows" {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../resources/icons/icon.ico")
            .set("ProductName", "Jarvis")
            .set("FileDescription", "Jarvis voice assistant")
            .set("CompanyName", "Jarvis");
        if let Err(e) = res.compile() {
            // a missing resource compiler must not break the build, the program just has no icon
            println!("cargo:warning=could not embed the icon: {}", e);
        }
    }
}
