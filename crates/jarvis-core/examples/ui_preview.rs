fn main() {
    let dir = std::env::temp_dir().join("jarvis_preview_cfg2");
    std::fs::create_dir_all(&dir).unwrap();
    let apps = serde_json::json!({"scanned_at": 1760000000u64, "apps": [
        {"name": "Total Commander", "launch": "/tmp/claude-0/-home-user-Jarvis-1-0/e43ae347-ed24-5d10-ac40-3f45d2667fea/scratchpad/fapps/Total Commander.lnk", "source": "start_menu", "exe": ""},
        {"name": "Double Commander", "launch": "/tmp/claude-0/-home-user-Jarvis-1-0/e43ae347-ed24-5d10-ac40-3f45d2667fea/scratchpad/fapps/Double Commander.lnk", "source": "desktop", "exe": ""},
        {"name": "Calculator", "launch": "shell:AppsFolder\\Microsoft.WindowsCalculator!App", "source": "registered", "exe": ""},
        {"name": "Old Tool", "launch": "C:/gone/old.lnk", "source": "app_paths", "exe": "old.exe"}]});
    std::fs::write(dir.join("apps_index.json"), apps.to_string()).unwrap();
    let _ = jarvis_core::APP_CONFIG_DIR.set(dir);
    if let Ok(c) = jarvis_core::commands::parse_commands() { let _ = jarvis_core::COMMANDS_LIST.set(c); }
    jarvis_core::ui::start();
    std::thread::sleep(std::time::Duration::from_secs(600));
}
