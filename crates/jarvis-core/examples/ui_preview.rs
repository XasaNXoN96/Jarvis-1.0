fn main() {
    let dir = std::env::temp_dir().join("jarvis_preview_cfg5");
    std::fs::create_dir_all(&dir).unwrap();
    jarvis_core::i18n::init("ru");
    let _ = jarvis_core::APP_CONFIG_DIR.set(dir);
    if let Ok(c) = jarvis_core::commands::parse_commands() { let _ = jarvis_core::COMMANDS_LIST.set(c); }
    jarvis_core::ui::set_exit_hook(Box::new(|| { eprintln!("EXIT_HOOK_CALLED"); std::process::exit(7) }));
    jarvis_core::ui::start();
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(14));
        let n = |id: &str, app: &str, aumid: &str, t: &[&str]| { jarvis_core::notify::on_external(id, app, aumid, &t.iter().map(|x| x.to_string()).collect::<Vec<_>>(), false); };
        n("1", "Telegram Desktop", "", &["Иван", "Привет! Ты завтра в офисе?"]);
        n("2", "Google Chrome", "Chrome", &["Мария", "Встречаемся в 6 у метро", "web.whatsapp.com"]);
        n("3", "Discord", "", &["Олег", "Заходи в голосовой канал"]);
        n("4", "Google Chrome", "Chrome", &["Скачивание завершено", "file.zip"]);
        n("5", "Instagram", "", &["anna_k", "отметила вас на фото"]);
    });
    std::thread::sleep(std::time::Duration::from_secs(3600));
}
