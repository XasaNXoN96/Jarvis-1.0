#[cfg(test)]
mod tests {
    use crate::lua::{CommandContext, LuaError, SandboxLevel, execute};

    use std::path::PathBuf;
    use std::time::Duration;
    use tempfile::tempdir;
    use std::fs;
    
    fn create_test_context(cmd_path: PathBuf) -> CommandContext {
        CommandContext {
            phrase: "test phrase".to_string(),
            command_id: "test_cmd".to_string(),
            command_path: cmd_path,
            language: "en".to_string(),
            slots: None,
        }
    }
    
    #[test]
    fn test_minimal_sandbox() {
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("test.lua");
        
        fs::write(&script_path, r#"
            jarvis.log("info", "test log")
            return { chain = false }
        "#).unwrap();
        
        let context = create_test_context(dir.path().to_path_buf());
        let result = execute(
            &script_path,
            context,
            SandboxLevel::Minimal,
            Duration::from_secs(5),
        );
        
        assert!(result.is_ok());
        assert_eq!(result.unwrap().chain, false);
    }
    
    #[test]
    fn test_state_persistence() {
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("test.lua");
        
        // first run - set state
        fs::write(&script_path, r#"
            jarvis.state.set("key", "value")
            return true
        "#).unwrap();
        
        let context = create_test_context(dir.path().to_path_buf());
        execute(&script_path, context, SandboxLevel::Standard, Duration::from_secs(5)).unwrap();
        
        // second run - read state
        fs::write(&script_path, r#"
            local val = jarvis.state.get("key")
            if val == "value" then
                return true
            else
                error("State not persisted")
            end
        "#).unwrap();
        
        let context = create_test_context(dir.path().to_path_buf());
        let result = execute(&script_path, context, SandboxLevel::Standard, Duration::from_secs(5));
        
        assert!(result.is_ok());
    }
    
    #[test]
    fn test_timeout() {
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("test.lua");
        
        fs::write(&script_path, r#"
            while true do end
        "#).unwrap();
        
        let context = create_test_context(dir.path().to_path_buf());
        let result = execute(
            &script_path,
            context,
            SandboxLevel::Minimal,
            Duration::from_millis(100),
        );
        
        assert!(matches!(result, Err(LuaError::Timeout)));
    }
    
    #[test]
    fn test_sandbox_fs_escape() {
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("test.lua");
        
        fs::write(&script_path, r#"
            local ok, err = pcall(function()
                jarvis.fs.read("../../../etc/passwd")
            end)
            if ok then
                error("Should have been blocked")
            end
            return true
        "#).unwrap();
        
        let context = create_test_context(dir.path().to_path_buf());
        let result = execute(&script_path, context, SandboxLevel::Standard, Duration::from_secs(5));
        
        assert!(result.is_ok());
    }

    // ---- "озвучь текст" / "прочитай буфер": Russian text from the clipboard (regression of "invalid UTF-8") ----
    fn speech_script() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant/speech.lua")
    }

    // runs the real command; returns its result and the last reply it gave
    fn read_clipboard(text: &str) -> (Result<crate::lua::CommandResult, LuaError>, String) {
        read_clipboard_with(text, Ok(()))
    }

    fn read_clipboard_with(text: &str, voice: Result<(), String>) -> (Result<crate::lua::CommandResult, LuaError>, String) {
        let _g = CLIPBOARD_LOCK.lock();
        *crate::tts::TEST_CAN_SPEAK.lock() = Some(voice);
        crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
        *crate::lua::api::system::TEST_CLIPBOARD.lock() = Some(text.to_string());
        let script = speech_script();
        let ctx = CommandContext {
            phrase: "прочитай буфер обмена".into(),
            command_id: "assistant_read_clipboard".into(),
            command_path: script.parent().unwrap().to_path_buf(),
            language: "ru".into(),
            slots: None,
        };
        let r = execute(&script, ctx, SandboxLevel::Full, Duration::from_secs(10));
        *crate::lua::api::system::TEST_CLIPBOARD.lock() = None;
        *crate::tts::TEST_CAN_SPEAK.lock() = None;
        let last = crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().last().cloned().unwrap_or_default());
        (r, last)
    }

    static CLIPBOARD_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    #[test]
    fn the_old_byte_cut_of_russian_text_was_the_cause_of_invalid_utf8() {
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("t.lua");
        // 81 Cyrillic letters = 162 bytes: string.sub(1, 80) ends in the middle of the 40th letter
        fs::write(&script_path, r#"
            local text = string.rep("ж", 81)
            local ok_old = pcall(jarvis.out, text:sub(1, 81))
            local p = jarvis.text.preview(text, 80)
            jarvis.out(p)
            return { chain = (not ok_old) and jarvis.text.len(p) == 81 and p:sub(-3) == "…" }
        "#).unwrap();
        let r = execute(&script_path, create_test_context(dir.path().to_path_buf()), SandboxLevel::Standard, Duration::from_secs(5)).unwrap();
        assert!(r.chain, "a byte cut breaks the text, the character cut does not");
    }

    #[test]
    fn reading_russian_mixed_and_multiline_clipboard_text_works() {
        let cases = [
            "Привет, это проверка озвучки русского текста из буфера обмена. Здесь больше восьмидесяти символов, чтобы обрезка сработала!",
            "Mixed текст: Hello, мир! Ёлка, «кавычки», — тире… 123 € ✓ Ўзбекча ғ қ ҳ",
            "Первая строка\nВторая line\r\nТретья строка\tс табуляцией",
        ];
        for text in cases {
            let (r, shown) = read_clipboard(text);
            assert!(r.is_ok(), "the command failed for {:?}: {:?}", text, r.err().map(|e| e.to_string()));
            assert!(shown.starts_with("Читаю: "), "the reply shows what is read: {:?}", shown);
            let preview = shown.trim_start_matches("Читаю: ");
            assert!(!preview.contains('\n') && !preview.contains('\u{FFFD}'), "one clean line: {:?}", preview);
            let first_word = text.split_whitespace().next().unwrap();
            assert!(preview.starts_with(first_word), "the Russian letters are kept: {:?}", preview);
        }
    }

    #[test]
    fn an_empty_clipboard_says_so_instead_of_pretending_to_read() {
        for text in ["", "   \n\t  "] {
            let (r, shown) = read_clipboard(text);
            assert!(r.is_ok());
            assert!(shown.contains("Нет текста"), "{:?}", shown);
            assert!(!shown.starts_with("Читаю"), "nothing is reported as read");
        }
    }

    #[test]
    fn the_clipboard_text_goes_only_to_the_local_voice() {
        let src = fs::read_to_string(speech_script()).unwrap();
        assert!(!src.contains("text:sub(1, 80)"), "the byte cut is gone");
        assert!(!src.contains("jarvis.ai") && !src.contains("jarvis.http"), "no AI, no network in the speech command");
    }

    #[test]
    fn emoji_and_very_long_clipboard_text_are_read_without_breaking() {
        let long = "Длинный текст 📚 для проверки озвучки. ".repeat(250); // ~10 000 characters
        let cases = [
            "Привет 👋 мир 🌍! Семья: 👨‍👩‍👧‍👦, флаг 🇺🇿, кожа 👍🏽 — всё должно остаться.".to_string(),
            "😀😃😄😁😆😅🤣😂🙂🙃😉😊😇🥰😍🤩😘😗😚😙😋😛😜🤪😝🤑🤗🤭🤫🤔🤐🤨😐😑😶😏😒🙄😬🤥😌😔😪🤤😴😷🤒🤕🤢🤮🤧🥵🥶🥴😵🤯🤠🥳😎🤓🧐😕😟🙁😮😯😲😳🥺😦😧😨😰😥😢😭😱😖😣😞😓😩😫🥱😤😡😠🤬".to_string(),
            long.clone(),
        ];
        for text in cases {
            let (r, shown) = read_clipboard(&text);
            assert!(r.is_ok(), "the command failed: {:?}", r.err().map(|e| e.to_string()));
            let preview = shown.strip_prefix("Читаю: ").unwrap_or_else(|| panic!("{:?}", shown));
            assert!(preview.chars().count() <= 81, "the preview is short even for a long text");
            assert!(!preview.contains('\u{FFFD}'), "no broken characters: {:?}", preview);
            let first: String = text.chars().take(5).collect();
            assert!(preview.starts_with(&first), "{:?}", preview);
        }
    }

    #[test]
    fn error_texts_in_the_changed_scripts_are_cut_by_characters() {
        // office.lua, selftest.lua and volume.lua cut the error text of PowerShell with jarvis.text.preview
        let dir = tempdir().unwrap();
        let script_path = dir.path().join("t.lua");
        fs::write(&script_path, r#"
            local r = { stderr = string.rep("Ошибка доступа ⛔ к файлу «отчёт.docx». ", 20) }
            local office = jarvis.text.preview(r.stderr, 200)       -- office.lua (open / delete)
            local selftest = jarvis.text.preview(r.stderr or "", 200) -- selftest.lua
            local volume = jarvis.text.preview(({}).stderr or "", 200) -- volume.lua with no error text
            jarvis.reply(office, "Файл")
            jarvis.reply(selftest)
            return { chain = jarvis.text.len(office) == 201 and jarvis.text.len(selftest) == 201 and volume == "" }
        "#).unwrap();
        let r = execute(&script_path, create_test_context(dir.path().to_path_buf()), SandboxLevel::Full, Duration::from_secs(5)).unwrap();
        assert!(r.chain);
        for name in ["office.lua", "selftest.lua", "volume.lua", "speech.lua"] {
            let src = fs::read_to_string(speech_script().parent().unwrap().join(name)).unwrap();
            assert!(!src.contains("stderr or \"\"):sub(") && !src.contains("err:gsub(\"%s+$\", \"\"):sub("), "{}: a byte cut of an error text is left", name);
        }
    }

    #[test]
    fn reading_aloud_is_never_reported_when_the_voice_cannot_speak() {
        for why in ["голос Джарвиса (Piper) не установлен", "озвучка выключена"] {
            let (r, shown) = read_clipboard_with("Привет, это текст для чтения 📚", Err(why.to_string()));
            assert!(r.is_ok());
            assert!(!shown.starts_with("Читаю"), "nothing is said to be read: {:?}", shown);
            assert!(shown.contains("Не могу прочитать вслух") && shown.contains(why), "the reason is told: {:?}", shown);
        }
    }

    // the keyboard/mouse commands answer what they did and never claim an action they could not do
    // (not on Windows: there the test would press real keys on the test machine)
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn keyboard_and_mouse_commands_run_and_report() {
        let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/input/input.lua");
        for (id, reply) in [("key_copy", "Скопировано"), ("mouse_double", "Двойной клик"), ("win_maximize", "Окно развёрнуто"), ("web_tab_new", "Новая вкладка")] {
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
            let ctx = CommandContext { phrase: "x".into(), command_id: id.into(), command_path: script.parent().unwrap().to_path_buf(), language: "ru".into(), slots: None };
            let r = execute(&script, ctx, SandboxLevel::Full, Duration::from_secs(5));
            assert!(r.is_ok(), "{}: {:?}", id, r.err().map(|e| e.to_string()));
            let last = crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().last().cloned().unwrap_or_default());
            assert_eq!(last, reply, "{}", id);
        }
    }

    // the task list from start to end in a temporary home folder (not on Windows: USERPROFILE is the real one there)
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn tasks_are_added_read_done_and_cleared_only_after_yes() {
        // the session is global: one test at a time with the other tests that use it
        let _s = crate::session::TEST_LOCK.lock();
        let _g = CLIPBOARD_LOCK.lock();
        let home = tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("Documents")).unwrap();
        std::env::set_var("HOME", home.path());
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        let run = |id: &str, phrase: &str| -> String {
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
            let script = dir.join(if id == "assistant_session" { "office.lua" } else { "tasks.lua" });
            let ctx = CommandContext { phrase: phrase.into(), command_id: id.into(), command_path: dir.clone(), language: "ru".into(), slots: None };
            let r = execute(&script, ctx, SandboxLevel::Full, Duration::from_secs(5));
            assert!(r.is_ok(), "{} {}: {:?}", id, phrase, r.err().map(|e| e.to_string()));
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().join(" | "))
        };
        let file = home.path().join("Documents/Jarvis-tasks.txt");
        assert!(run("assistant_task_add", "добавь задачу купить хлеб").contains("купить хлеб"));
        run("assistant_task_add", "новая задача позвонить врачу 😀");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "купить хлеб\nпозвонить врачу 😀\n");
        // the limits of the panel "Задачи" hold for the voice too: 200 characters, 30 tasks
        assert!(run("assistant_task_add", &format!("добавь задачу {}", "я".repeat(201))).contains("200"));
        assert_eq!(std::fs::read_to_string(&file).unwrap().lines().count(), 2, "a too long task is not added");
        let read = run("assistant_task_read", "озвучь задачи");
        assert!(read.contains("1. купить хлеб") && read.contains("2. позвонить врачу"), "{}", read);
        assert!(run("assistant_task_done", "выполнил задачу 1").contains("купить хлеб"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "позвонить врачу 😀\n");
        assert!(run("assistant_task_done", "выполнил задачу 7").contains("номер"), "a wrong number removes nothing");
        // "стереть все задачи" is a high-risk command: Jarvis asks first (confirm.rs, the application does it before the script
        // runs); "нет" keeps the tasks, "да" from the same door runs the command that was asked about and clears them
        let _q = crate::confirm::TEST_LOCK.lock();
        crate::confirm::clear();
        use crate::confirm::{gate, reply, Gate, Reply};
        use crate::origin::Source;
        use crate::policy::Risk;
        let phrase = "стереть все задачи";
        assert!(matches!(gate("assistant_task_clear", Risk::High, phrase, Source::Window), Gate::Ask { .. }));
        assert_eq!(reply("нет", Source::Window), Reply::Cancelled);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "позвонить врачу 😀\n", "kept after no");
        assert!(matches!(gate("assistant_task_clear", Risk::High, phrase, Source::Window), Gate::Ask { .. }));
        assert_eq!(reply("да", Source::Voice), Reply::OtherDoor, "a «да» of another door does not confirm it");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "позвонить врачу 😀\n");
        assert!(matches!(reply("да", Source::Window), Reply::Rerun { .. }));
        let Gate::Run { confirmed } = gate("assistant_task_clear", Risk::High, phrase, Source::Window) else { panic!("asked again after the yes") };
        assert!(confirmed);
        let _c = crate::confirm::confirmed_run("assistant_task_clear", confirmed);
        assert!(run("assistant_task_clear", phrase).contains("стёрты"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
        // a file saved by Notepad with a BOM: the first task is read without it (as the panel reads it)
        std::fs::write(&file, "\u{feff}с бомом\nвторая\n").unwrap();
        let read = run("assistant_task_read", "озвучь задачи");
        assert!(read.contains("1. с бомом") && !read.contains('\u{feff}'), "{:?}", read);
    }

    // every script of every command compiles in the Lua of Jarvis (5.5: e.g. a loop variable cannot be assigned)
    #[test]
    fn every_command_script_compiles() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let prelude = fs::read_to_string(root.join("_prelude.lua")).unwrap();
        let lua = mlua::Lua::new();
        let mut checked = 0;
        let mut errors = Vec::new();
        for pack in fs::read_dir(&root).unwrap().flatten() {
            if !pack.path().is_dir() { continue; }
            for f in fs::read_dir(pack.path()).unwrap().flatten() {
                let p = f.path();
                if p.extension().map(|e| e == "lua").unwrap_or(false) {
                    let src = format!("{}\n{}", prelude, fs::read_to_string(&p).unwrap());
                    if let Err(e) = lua.load(&src).set_name(p.display().to_string()).into_function() {
                        errors.push(format!("{}: {}", p.display(), e));
                    }
                    checked += 1;
                }
            }
        }
        assert!(checked > 20, "scripts found: {}", checked);
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }

    #[test]
    fn the_talk_mode_needs_the_ai_and_the_browser_can_be_chosen_by_voice() {
        let _s = crate::session::TEST_LOCK.lock();
        let _u = crate::uistate::TEST_LOCK.lock();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        let run = |script: &str, id: &str, phrase: &str| -> String {
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
            let ctx = CommandContext { phrase: phrase.into(), command_id: id.into(), command_path: dir.clone(), language: "ru".into(), slots: None };
            let r = execute(&dir.join(script), ctx, SandboxLevel::Full, Duration::from_secs(5));
            assert!(r.is_ok(), "{}: {:?}", id, r.err().map(|e| e.to_string()));
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().join(" | "))
        };
        crate::session::end();
        // no AI set up / AI switched off: the talk mode does not start and says why
        crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
        let r = run("chat.lua", "assistant_chat_start", "чат");
        assert!(r.contains("ИИ-провайдер не выбран"), "{}", r);
        assert!(crate::session::kind().is_none());
        crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_enabled: false, ai_provider: "gemini".into(), ai_key: "k".into(), ..Default::default() });
        assert!(run("chat.lua", "assistant_chat_start", "чат").contains("выключены"));
        assert!(crate::session::kind().is_none());
        // set up: the session starts, "закончи чат" ends it
        crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_provider: "gemini".into(), ai_key: "k".into(), ..Default::default() });
        assert!(run("chat.lua", "assistant_chat_start", "чат").contains("Режим разговора включён"));
        assert_eq!(crate::session::kind().as_deref(), Some("chat"));
        run("chat.lua", "assistant_chat_end", "закончи чат");
        assert!(crate::session::kind().is_none());
        // the browser by voice: the same setting as the window
        assert!(run("extra.lua", "assistant_browser_set", "переключи на оперу").contains("Opera"));
        assert_eq!(crate::uistate::settings().browser, "opera");
        run("extra.lua", "assistant_browser_set", "переключи на эдж");
        assert_eq!(crate::uistate::settings().browser, "edge");
        crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
    }

    // the website command opens the site of the catalogue, a search stays a search (Linux: "open" only logs)
    #[test]
    fn the_website_command_uses_the_one_catalogue() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        let run = |phrase: &str| -> String {
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
            let ctx = CommandContext { phrase: phrase.into(), command_id: "assistant_website".into(), command_path: dir.clone(), language: "ru".into(), slots: None };
            let r = execute(&dir.join("website.lua"), ctx, SandboxLevel::Full, Duration::from_secs(5));
            assert!(r.is_ok(), "{}: {:?}", phrase, r.err().map(|e| e.to_string()));
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().join(" | "))
        };
        assert!(run("открой YouTube").contains("YouTube"), "not a Google search");
        assert!(run("Открой ChatGPT").contains("ChatGPT"));
        assert!(run("зайди на сайт гитхаб").contains("GitHub"));
        assert!(run("открой сайт ютуб музыка").contains("YouTube Music"));
        assert!(run("найди в гугле как установить python").contains("как установить python"));
        assert!(run("открой сайт непонятныйсайт").contains("Не знаю такой сайт"), "an unknown name is said to be a search");
        assert!(run("зайди на сайт example.com").contains("example.com"));
    }

    // the phone commands queue only real messages and say honestly when the phone is not connected
    #[test]
    fn the_phone_commands_send_to_the_phone_page_and_never_pretend() {
        let _r = crate::remote::PHONE_TEST_LOCK.lock();
        let _g = CLIPBOARD_LOCK.lock();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        let run = |id: &str, phrase: &str, clip: &str| -> String {
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
            *crate::lua::api::system::TEST_CLIPBOARD.lock() = Some(clip.to_string());
            let ctx = CommandContext { phrase: phrase.into(), command_id: id.into(), command_path: dir.clone(), language: "ru".into(), slots: None };
            let r = execute(&dir.join("phone.lua"), ctx, SandboxLevel::Full, Duration::from_secs(5));
            *crate::lua::api::system::TEST_CLIPBOARD.lock() = None;
            assert!(r.is_ok(), "{} {}: {:?}", id, phrase, r.err().map(|e| e.to_string()));
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().join(" | "))
        };
        let q = |k: &str, d: &str| vec![(k.to_string(), d.to_string())];

        crate::remote::phone_for_tests(false, false);
        assert!(run("assistant_phone_find", "найди телефон", "").contains("выключен"), "remote off: nothing pretends to ring");
        assert!(crate::remote::queued_for_tests().is_empty());

        crate::remote::phone_for_tests(true, true);
        assert!(run("assistant_phone_find", "найди телефон", "").contains("звонит"));
        assert_eq!(crate::remote::queued_for_tests(), q("ring", ""));
        crate::remote::phone_for_tests(true, true);
        assert!(run("assistant_phone_stop", "останови сигнал телефона", "").contains("остановлен"));

        crate::remote::phone_for_tests(true, false);
        let r = run("assistant_phone_find", "где мой телефон", "");
        assert!(r.contains("не на связи") && !r.contains("звонит"), "the phone page is not open: {}", r);

        crate::remote::phone_for_tests(true, true);
        assert!(run("assistant_phone_link", "отправь ссылку на телефон", "просто текст").contains("нет ссылки"));
        assert!(crate::remote::queued_for_tests().is_empty());
        assert!(run("assistant_phone_link", "отправь ссылку на телефон", " https://example.com/x?a=1 \n").contains("отправлена"));
        assert_eq!(crate::remote::queued_for_tests(), q("link", "https://example.com/x?a=1"));
        crate::remote::phone_for_tests(true, true);
        assert!(run("assistant_phone_link", "открой ютуб на телефоне", "").contains("YouTube"));
        assert_eq!(crate::remote::queued_for_tests(), q("link", "https://www.youtube.com/"), "a site of the catalogue, not the clipboard");
        crate::remote::phone_for_tests(true, true);
        assert!(run("assistant_phone_find", "найди файл на телефоне", "").contains("не умеет"));
        assert!(crate::remote::queued_for_tests().is_empty(), "a file search does not ring the phone");

        for (phrase, clip, sent) in [
            ("отправь на телефон купить молоко", "", "купить молоко"),
            ("Отправь текст Позвонить маме на телефон", "", "Позвонить маме"),
            ("отправь буфер обмена на телефон", "Код подъезда 1234 🙂", "Код подъезда 1234 🙂"),
            ("скинь на телефон", "адрес: ул. Ленина 5", "адрес: ул. Ленина 5"),
        ] {
            crate::remote::phone_for_tests(true, true);
            assert!(run("assistant_phone_text", phrase, clip).contains("отправлен"), "{}", phrase);
            assert_eq!(crate::remote::queued_for_tests(), q("text", sent), "{}", phrase);
        }
        crate::remote::phone_for_tests(true, true);
        assert!(run("assistant_phone_text", "отправь на телефон", "  ").contains("Нечего отправлять"));
        assert!(run("assistant_phone_text", "отправь файл на телефон", "").contains("не файлы"), "a file is not sent as the word «файл»");
        assert!(crate::remote::queued_for_tests().is_empty());
        assert!(crate::remote::queued_for_tests().is_empty());
        crate::remote::phone_for_tests(false, false);
    }

    // installed programs from the index: one fitting program starts, several are listed and the answer chooses
    // not on Windows: the scripts would really open the test files there
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn installed_programs_start_and_an_ambiguous_name_asks_which_one() {
        let _s = crate::session::TEST_LOCK.lock();
        let _a = crate::appindex::TEST_LOCK.lock();
        let base = std::env::temp_dir().join(format!("jarvis_prog_{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let mk = |name: &str| {
            let p = base.join(format!("{}.lnk", name));
            std::fs::write(&p, b"x").unwrap();
            crate::appindex::IndexedApp { name: name.into(), launch: p.to_string_lossy().to_string(), source: "start_menu".into(), exe: String::new() }
        };
        crate::appindex::set_for_tests(vec![mk("Total Commander 64-bit"), mk("Double Commander"), mk("Krita")]);
        // the scripts keep their state next to them ("открой блокнот" remembers the document program): use a copy
        let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        let dir = base.join("assistant");
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["program.lua", "office.lua"] { std::fs::copy(src.join(f), dir.join(f)).unwrap(); }
        std::fs::copy(src.join("../_prelude.lua"), base.join("_prelude.lua")).unwrap();
        let run = |id: &str, script: &str, phrase: &str| -> String {
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
            let ctx = CommandContext { phrase: phrase.into(), command_id: id.into(), command_path: dir.clone(), language: "ru".into(), slots: None };
            let r = execute(&dir.join(script), ctx, SandboxLevel::Full, Duration::from_secs(5));
            assert!(r.is_ok(), "{} {}: {:?}", id, phrase, r.err().map(|e| e.to_string()));
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().join(" | "))
        };
        let r = run("assistant_program_open", "program.lua", "открой krita");
        assert!(r.contains("Открываю") && r.contains("Krita"), "{}", r);
        let r = run("assistant_program_open", "program.lua", "открой командер");
        assert!(r.contains("1. ") && r.contains("2. ") && r.contains("Total Commander") && r.contains("Double Commander"), "{}", r);
        assert!(!r.contains("Открываю"), "nothing is started before the answer: {}", r);
        assert_eq!(crate::session::kind().as_deref(), Some("app_choice"));
        let second = crate::appindex::pending()[1].name.clone();
        let r = run("assistant_session", "office.lua", "второй");
        assert!(r.contains(&second), "{}", r);
        assert_eq!(crate::session::kind(), None);
        // a removed program is reported honestly
        let r = run("assistant_program_open", "program.lua", "открой несуществующуюпрограмму");
        assert!(r.contains("Не нашёл"), "{}", r);
        // the fixed list still works first
        let r = run("assistant_program_open", "program.lua", "открой блокнот");
        assert!(r.contains("Блокнот"), "{}", r);
        // closing works only for the fixed list: an ambiguous name is never opened instead
        let r = run("assistant_program_close", "program.lua", "закрой командер");
        assert!(r.contains("основного списка") && !r.contains("Какую программу"), "{}", r);
        crate::session::end();
        crate::appindex::set_for_tests(Vec::new());
        let _ = std::fs::remove_dir_all(&base);
    }

    // found files: several results are never opened at random; a number, "следующий" and "ещё" choose
    // not on Windows: the scripts would really open the test files there
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn found_files_are_opened_only_when_the_user_says_which_one() {
        let _s = crate::session::TEST_LOCK.lock();
        let _f = crate::filesearch::TEST_LOCK.lock();
        let tmp = tempdir().unwrap();
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        // the scripts keep their state next to them: work on a copy so the repository stays clean
        let cmd_dir = tmp.path().join("assistant");
        std::fs::create_dir_all(&cmd_dir).unwrap();
        for f in ["files.lua", "office.lua"] { std::fs::copy(dir.join(f), cmd_dir.join(f)).unwrap(); }
        std::fs::copy(dir.join("../_prelude.lua"), tmp.path().join("_prelude.lua")).unwrap();
        let paths: Vec<PathBuf> = (1..=10).map(|i| { let p = tmp.path().join(format!("отчёт {}.txt", i)); std::fs::write(&p, b"x").unwrap(); p }).collect();
        crate::filesearch::set_results_for_tests(paths.clone());
        let run = |id: &str, script: &str, phrase: &str| -> String {
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
            let ctx = CommandContext { phrase: phrase.into(), command_id: id.into(), command_path: cmd_dir.clone(), language: "ru".into(), slots: None };
            let r = execute(&cmd_dir.join(script), ctx, SandboxLevel::Full, Duration::from_secs(5));
            assert!(r.is_ok(), "{} {}: {:?}", id, phrase, r.err().map(|e| e.to_string()));
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().join(" | "))
        };
        let r = run("assistant_file_open", "files.lua", "открой найденный файл");
        assert!(r.contains("Какой открыть") && !r.contains("Открываю"), "{}", r);
        assert_eq!(crate::session::kind().as_deref(), Some("file"));
        let r = run("assistant_session", "office.lua", "открой");
        assert!(r.contains("номер") && !r.contains("Открываю"), "still asks: {}", r);
        let r = run("assistant_session", "office.lua", "второй");
        assert!(r.contains("отчёт 2.txt"), "{}", r);
        let r = run("assistant_session", "office.lua", "открой");
        assert!(r.contains("Открываю") && r.contains("отчёт 2.txt"), "{}", r);
        let r = run("assistant_file_next", "files.lua", "найди следующий файл");
        assert!(r.contains("3. отчёт 3.txt"), "{}", r);
        let r = run("assistant_file_folder", "files.lua", "покажи где находится этот файл");
        assert!(r.contains(&tmp.path().display().to_string()), "the folder of the chosen one: {}", r);
        // a number inside another phrase does not choose a file; "четвёртый" does
        crate::filesearch::set_results_for_tests(paths.clone());
        crate::session::start("file", 60);
        crate::session::set_data("");
        run("assistant_session", "office.lua", "таймер на 2 минуты");
        assert_eq!(crate::filesearch::selected(), None, "a timer phrase chose a file");
        crate::session::start("file", 60);
        let r = run("assistant_session", "office.lua", "четвёртый");
        assert!(r.contains("отчёт 4.txt"), "{}", r);
        let r = run("assistant_session", "office.lua", "вторник");
        assert!(!r.contains("отчёт 2.txt"), "«вторник» is not «второй»: {}", r);
        let r = run("assistant_file_open", "files.lua", "открой пятый файл");
        assert!(r.contains("отчёт 5.txt"), "{}", r);
        let r = run("assistant_file_more", "files.lua", "покажи ещё результаты");
        assert!(r.contains("Результаты 9–10 из 10"), "{}", r);
        // one result: opened without a question
        crate::filesearch::set_results_for_tests(vec![paths[0].clone()]);
        run("assistant_file_search", "files.lua", "найди файл отчёт 1"); // a new search forgets the old choice
        crate::filesearch::cancel();
        crate::filesearch::set_results_for_tests(vec![paths[0].clone()]);
        let r = run("assistant_file_open", "files.lua", "открой найденный файл");
        assert!(r.contains("Открываю") && r.contains("отчёт 1.txt"), "{}", r);
        let r = run("assistant_file_search", "files.lua", "найди файл");
        assert!(r.contains("Что искать"), "{}", r);
        let r = run("assistant_file_cancel", "files.lua", "останови поиск");
        assert!(r.contains("ничего не ищу") || r.contains("остановлен"), "{}", r);
        crate::session::end();
        crate::filesearch::set_results_for_tests(Vec::new());
    }

    // a tasks file in the old Windows encoding is read, never overwritten with only the new task
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn an_old_ansi_tasks_file_is_kept_when_a_task_is_added() {
        let _s = crate::session::TEST_LOCK.lock();
        let home = tempdir().unwrap();
        std::fs::create_dir_all(home.path().join("Documents")).unwrap();
        std::fs::write(home.path().join("Documents/Jarvis-tasks.txt"), [0xEC, 0xEE, 0xEB, 0xEE, 0xEA, 0xEE, b'\r', b'\n']).unwrap(); // "молоко" in cp1251
        std::env::set_var("HOME", home.path());
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
        let ctx = CommandContext { phrase: "добавь задачу купить хлеб".into(), command_id: "assistant_task_add".into(), command_path: dir.clone(), language: "ru".into(), slots: None };
        assert!(execute(&dir.join("tasks.lua"), ctx, SandboxLevel::Full, Duration::from_secs(5)).is_ok());
        let text = std::fs::read_to_string(home.path().join("Documents/Jarvis-tasks.txt")).unwrap();
        assert_eq!(text, "молоко\nкупить хлеб\n");
    }

    // "рабочий режим" ends the game mode only: a reminder of the user stays
    #[test]
    fn work_mode_cancels_only_the_game_mode_reminders() {
        use crate::lua::api::timer;
        let mine = format!("позвонить маме {}", rand::random::<u32>());
        let id_mine = timer::start_timer(3600, "Напоминание".into(), mine.clone());
        let id_game = timer::start_timer(3600, "Игровой режим".into(), format!("Время игры вышло {}", rand::random::<u32>()));
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        let ctx = CommandContext { phrase: "рабочий режим".into(), command_id: "assistant_game_off".into(), command_path: dir.clone(), language: "ru".into(), slots: None };
        assert!(execute(&dir.join("game.lua"), ctx, SandboxLevel::Full, Duration::from_secs(5)).is_ok());
        let ids: Vec<u64> = timer::list_json().iter().filter_map(|t| t["id"].as_u64()).collect();
        assert!(ids.contains(&id_mine), "the user's reminder was cancelled");
        assert!(!ids.contains(&id_game), "the game mode reminder stays");
        timer::cancel_titled("Напоминание");
    }

    // a file name with cmd syntax never goes to the "cmd /C start" fallback of opening
    #[test]
    fn names_with_cmd_syntax_never_reach_cmd() {
        use crate::lua::api::system::cmd_fallback_allowed;
        for bad in ["C:\\Docs\\Отчёт & План.docx", "x&calc", "a|b", "100%.txt", "(1).pdf", "say \"hi\".txt", "a^b", "wow!.txt"] {
            assert!(!cmd_fallback_allowed(bad), "{}", bad);
        }
        assert!(cmd_fallback_allowed("C:\\Users\\Сергей\\Documents\\отчёт 2024.docx"));
        assert!(cmd_fallback_allowed("https://www.youtube.com/"));
    }

    // tasks and notes in a home folder with Cyrillic letters and spaces ("C:\Users\Сергей Иванов")
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn tasks_and_notes_work_in_a_cyrillic_home_folder() {
        let _s = crate::session::TEST_LOCK.lock();
        let base = tempdir().unwrap();
        let home = base.path().join("Сергей Иванов");
        std::fs::create_dir_all(home.join("Documents")).unwrap();
        std::env::set_var("HOME", &home);
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant");
        let run = |id: &str, script: &str, phrase: &str| -> String {
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
            let ctx = CommandContext { phrase: phrase.into(), command_id: id.into(), command_path: dir.clone(), language: "ru".into(), slots: None };
            assert!(execute(&dir.join(script), ctx, SandboxLevel::Full, Duration::from_secs(5)).is_ok(), "{}", phrase);
            crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().join(" | "))
        };
        run("assistant_task_add", "tasks.lua", "добавь задачу купить хлеб");
        run("assistant_note_add", "notes.lua", "запиши идею позвонить в банк");
        assert_eq!(std::fs::read_to_string(home.join("Documents/Jarvis-tasks.txt")).unwrap(), "купить хлеб\n");
        assert!(std::fs::read_to_string(home.join("Documents/Jarvis-notes.txt")).unwrap().contains("позвонить в банк"));
        assert!(run("assistant_task_read", "tasks.lua", "озвучь задачи").contains("купить хлеб"));
        assert!(run("assistant_note_read", "notes.lua", "что я записывал").contains("Последние"));
    }

    // ---- the answer of a command is shown AND said (once), or only shown when speech is off ----
    fn answer(id: &str, script: &str, phrase: &str) -> (String, Vec<String>) {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant").join(script);
        crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
        let before = crate::tts::TEST_SPOKEN.lock().len();
        let ctx = CommandContext { phrase: phrase.into(), command_id: id.into(), command_path: path.parent().unwrap().to_path_buf(), language: "ru".into(), slots: None };
        execute(&path, ctx, SandboxLevel::Full, Duration::from_secs(10)).unwrap();
        let reply = crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().last().cloned().unwrap_or_default());
        // (other tests may speak at the same time: only the phrases of this command are counted)
        let spoken: Vec<String> = crate::tts::TEST_SPOKEN.lock()[before..].to_vec();
        (reply, spoken)
    }

    #[test]
    fn time_date_battery_and_help_are_said_aloud_once_and_shown() {
        let _g = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(true);
        let (shown, spoken) = answer("assistant_info_time", "info.lua", "сколько времени");
        assert!(shown.contains(':'), "the time is shown: {}", shown);
        assert_eq!(spoken.iter().filter(|s| s.starts_with("Сейчас ")).count(), 1, "said exactly once: {:?}", spoken);
        let (shown, spoken) = answer("assistant_info_date", "info.lua", "какое сегодня число");
        assert!(shown.contains("Сегодня"), "{}", shown);
        assert_eq!(spoken.iter().filter(|s| s.starts_with("Сегодня") && shown.ends_with(s.as_str())).count(), 1, "the date is said once: {:?}", spoken);
        let (shown, spoken) = answer("assistant_info_battery", "info.lua", "какой заряд батареи");
        assert!(!shown.is_empty());
        assert_eq!(spoken.iter().filter(|s| s.contains("Батарея") || s.contains("Заряд")).count(), 1, "{:?}", spoken);
        let (shown, spoken) = answer("assistant_help", "smalltalk.lua", "что ты умеешь");
        assert!(shown.contains("команд"));
        assert_eq!(spoken.iter().filter(|s| s.starts_with("Показываю все команды")).count(), 1, "{:?}", spoken);
    }

    #[test]
    fn with_speech_off_the_answer_is_only_shown() {
        let _g = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(false);
        let (shown, spoken) = answer("assistant_info_date", "info.lua", "какое сегодня число");
        crate::tts::set_enabled(true);
        assert!(shown.contains("Сегодня"), "the answer stays in the window: {}", shown);
        assert!(!spoken.iter().any(|s| s.starts_with("Сегодня")), "nothing is said: {:?}", spoken);
    }

    // ---- "открой тотал командер": "Открываю …" when Windows took it, an error when the program never appears ----
    static PROCESS_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

    fn open_program(now: &[&str], later: &[&str], after: u32) -> (String, Vec<String>) {
        let _g = PROCESS_LOCK.lock();
        // the fixed list of programs, not the installed-programs index (other tests fill it): an empty index
        let _i = crate::appindex::TEST_LOCK.lock();
        crate::appindex::set_for_tests(vec![]);
        // every check is a new request (the "already starting" guard of a double command is not tested here)
        crate::guard::forget("lua:open:Total Commander");
        *crate::lua::api::system::TEST_PROCESSES.lock() = Some((now.iter().map(|s| s.to_string()).collect(), later.iter().map(|s| s.to_string()).collect(), after));
        let r = answer("assistant_program_open", "program.lua", "открой тотал командер");
        *crate::lua::api::system::TEST_PROCESSES.lock() = None;
        r
    }

    // On Windows the command really starts the program: the CI machine has no Total Commander, so the honest
    // answer is checked there instead
    #[cfg(target_os = "windows")]
    #[test]
    fn opening_a_program_that_is_not_installed_is_reported_on_windows() {
        let _t = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(true);
        let _i = crate::appindex::TEST_LOCK.lock();
        crate::appindex::set_for_tests(vec![]);
        crate::guard::forget("lua:open:Total Commander");
        let (shown, _) = answer("assistant_program_open", "program.lua", "открой тотал командер");
        assert!(shown.contains("не удалось запустить") || shown.contains("Открываю"), "{}", shown);
        assert!(!shown.contains("Открываю") || crate::lua::api::system::process_running("TOTALCMD64.EXE") || crate::lua::api::system::process_running("TOTALCMD.EXE"), "never «Открываю» for a program that did not start: {}", shown);
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn opening_a_program_is_confirmed_and_a_launch_that_never_appears_is_reported() {
        let _t = crate::uistate::TEST_LOCK.lock();
        crate::tts::set_enabled(true);
        // the process appears a moment later: "Открываю", no second phrase
        let (_, spoken) = open_program(&[], &["totalcmd64.exe"], 2);
        assert_eq!(spoken.iter().filter(|s| s.as_str() == "Открываю Total Commander" || s.starts_with("Открываю Total Commander. ")).count(), 1, "{:?}", spoken);
        assert!(!spoken.iter().any(|s| s.contains("не запустилась")), "{:?}", spoken);
        // already running before (it only brings its window forward): no waiting, no false alarm
        let (_, spoken) = open_program(&["TOTALCMD64.EXE"], &[], 0);
        assert!(!spoken.iter().any(|s| s.contains("не запустилась")), "{:?}", spoken);
        // "калькулятор": calc.exe hands over to CalculatorApp.exe and ends — that process is waited for, no false alarm
        let spoken = {
            let _p = PROCESS_LOCK.lock();
            let _i = crate::appindex::TEST_LOCK.lock();
            crate::appindex::set_for_tests(vec![]);
            crate::guard::forget("lua:open:Калькулятор");
            *crate::lua::api::system::TEST_PROCESSES.lock() = Some((vec![], vec!["CalculatorApp.exe".into()], 2));
            let (_, spoken) = answer("assistant_program_open", "program.lua", "открой калькулятор");
            *crate::lua::api::system::TEST_PROCESSES.lock() = None;
            spoken
        };
        assert!(!spoken.iter().any(|s| s.contains("не запустилась")), "{:?}", spoken);
        // the process never appears: the user hears that it did not start
        crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
        let (shown, spoken) = open_program(&[], &[], 0);
        assert!(spoken.iter().any(|s| s == "Total Commander не запустилась"), "{:?}", spoken);
        assert!(shown.contains("не запустилась"), "{}", shown);
    }

    // (not on Windows: there the commands would really open Explorer and the browser on the developer's PC)
    #[cfg(not(target_os = "windows"))]
    #[test]
    fn open_drive_d_in_russian_and_open_youtube_and_search_are_understood() {
        let (shown, _) = answer("assistant_disk_open", "places.lua", "открой диск д");
        assert!(shown.contains("D:"), "{}", shown);
        let (shown, _) = answer("assistant_disk_open", "places.lua", "открой диск дэ");
        assert!(shown.contains("D:"), "{}", shown);
        // the search is for "котиков", not for "ютуб и найди котиков"
        let (shown, _) = answer("assistant_website", "website.lua", "открой ютуб и найди котиков");
        assert!(shown.ends_with("котиков") && !shown.contains("ютуб"), "{}", shown);
    }

    // Total Commander runs as TOTALCMD.EXE (32-bit copy) or TOTALCMD64.EXE: "закрой Total Commander" finds the one that
    // really runs; a program that does not run is "не запущена"
    #[test]
    fn close_finds_the_process_the_program_really_runs_as() {
        let _p = PROCESS_LOCK.lock();
        crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
        // (on Windows taskkill is real: the fake process does not exist, so taskkill answers 128 «не запущена» there
        // and only the "nothing runs" case below can be checked)
        if !cfg!(target_os = "windows") {
            *crate::lua::api::system::TEST_PROCESSES.lock() = Some((vec!["TOTALCMD.EXE".into(), "explorer.exe".into()], vec![], 0));
            let (shown, _) = answer("assistant_program_close", "program.lua", "закрой Total Commander");
            assert!(!shown.contains("не запущена"), "the 32-bit copy runs: {}", shown);
            *crate::lua::api::system::TEST_PROCESSES.lock() = Some((vec!["TOTALCMD64.EXE".into()], vec![], 0));
            let (shown, _) = answer("assistant_program_close", "program.lua", "закрой Total Commander");
            assert!(!shown.contains("не запущена"), "the 64-bit copy runs: {}", shown);
        }
        *crate::lua::api::system::TEST_PROCESSES.lock() = Some((vec!["explorer.exe".into()], vec![], 0));
        let (shown, _) = answer("assistant_program_close", "program.lua", "закрой Total Commander");
        assert!(shown.contains("не запущена"), "nothing of it runs: {}", shown);
        *crate::lua::api::system::TEST_PROCESSES.lock() = None;
    }

    #[test]
    fn close_word_asks_the_program_to_close_and_never_kills_unsaved_work() {
        // (the list of fake processes is global: not together with the tests that set it)
        let _p = PROCESS_LOCK.lock();
        let (shown, _) = answer("assistant_program_close", "program.lua", "закрой ворд");
        if cfg!(target_os = "windows") {
            // really asked on Windows: Word is not running on the CI machine (taskkill code 128)
            assert!(shown.contains("несохранённые изменения") || shown.contains("не запущена"), "{}", shown);
        } else {
            assert!(shown.contains("несохранённые изменения"), "{}", shown);
        }
        // the kill is only for the explicit word, and the script asks taskkill without /f otherwise
        let script = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands/assistant/program.lua")).unwrap();
        assert!(script.contains(r#"force and { "/f", "/im", n } or { "/im", n }"#));
        if !cfg!(target_os = "windows") {
            let (shown, _) = answer("assistant_program_force_close", "program.lua", "закрой ворд принудительно");
            assert!(shown.contains("принудительно"), "{}", shown);
        }
    }

    // The trust boundary of the commands: no command pack runs a shell (cmd /C) with anything taken from the phrase.
    // A "cli" command is a fixed program with fixed arguments, and no Lua script of the packs uses system.exec.
    #[test]
    fn no_command_pack_runs_a_shell_with_the_words_of_the_phrase() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands");
        let mut lua_files = 0;
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            for e in std::fs::read_dir(&dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().map(|x| x == "lua").unwrap_or(false) {
                    lua_files += 1;
                    let text = std::fs::read_to_string(&p).unwrap();
                    assert!(!text.contains("system.exec"), "{} runs a shell", p.display());
                } else if p.file_name().map(|n| n == "command.toml").unwrap_or(false) {
                    let text = std::fs::read_to_string(&p).unwrap();
                    for line in text.lines().filter(|l| l.trim_start().starts_with("cli_cmd") || l.trim_start().starts_with("cli_args")) {
                        assert!(!line.contains('{') && !line.contains('%'), "{}: a cli command takes a placeholder: {}", p.display(), line);
                        assert!(!line.contains('&') && !line.contains('|') && !line.contains(';'), "{}: shell operators in a cli command: {}", p.display(), line);
                    }
                }
            }
        }
        assert!(lua_files > 20, "the packs were found ({} scripts)", lua_files);
        // and the only Lua door to a shell is closed below the "full" sandbox
        assert!(!crate::lua::sandbox::SandboxLevel::Standard.allows_exec());
        assert!(!crate::lua::sandbox::SandboxLevel::from_str("").allows_exec());
    }
    // «поставь таймер на 2 секунды» -> after two seconds the notification of the timer exists (and is meant to be said)
    #[test]
    fn a_timer_set_by_voice_ends_with_a_notification() {
        let _u = crate::uistate::TEST_LOCK.lock();
        // the phone is off while the timer rings: its note must not land in the queue a phone test is reading
        let _p = crate::remote::PHONE_TEST_LOCK.lock();
        crate::remote::phone_for_tests(false, false);
        crate::tts::set_enabled(true);
        let marker = format!("{}", rand::random::<u32>());
        let (shown, _) = answer("assistant_timer_set", "timer.lua", "поставь таймер на 2 секунды");
        assert!(shown.contains("Запущен на 2"), "the timer was set: {}", shown);
        let t0 = std::time::Instant::now();
        let mut found = None;
        while t0.elapsed() < Duration::from_secs(8) && found.is_none() {
            std::thread::sleep(Duration::from_millis(200));
            found = crate::notify::recent(40).into_iter().find(|n| n.kind == "timer" && n.text.contains("Время вышло"));
        }
        let _ = marker;
        let n = found.expect("no notification of kind timer after the time was up");
        assert!(n.shown && n.spoken, "shown and said by the settings: {:?}", n);
        crate::lua::api::timer::cancel_titled("Таймер");
    }

    // a real script asks Jarvis to run a phrase: the phrase keeps the door of the command that is running,
    // whatever the script (or the text it was started with) says
    #[test]
    fn run_text_of_a_script_keeps_the_door_of_the_running_command() {
        let dir = tempdir().unwrap();
        let script = dir.path().join("ask.lua");
        std::fs::write(&script, "jarvis.app.run_text('\\1открой блокнот')\njarvis.app.run_text('source=voice')").unwrap();
        let cap = crate::remote::Capture::start();
        let run = |door: Option<crate::origin::Source>| {
            let ctx = CommandContext { phrase: "x".into(), command_id: "t".into(), command_path: dir.path().to_path_buf(), language: "ru".into(), slots: None };
            let go = || execute(&script, ctx, SandboxLevel::Full, Duration::from_secs(5)).unwrap();
            match door {
                Some(d) => crate::origin::run_as(d, go),
                None => go(),
            };
        };
        run(Some(crate::origin::Source::Phone));
        run(Some(crate::origin::Source::Telegram));
        run(None);
        let got: Vec<(crate::origin::Source, bool)> = cap.taken().into_iter().map(|i| (i.source, i.via_script)).collect();
        use crate::origin::Source::*;
        assert_eq!(got, vec![(Phone, true), (Phone, true), (Telegram, true), (Telegram, true), (Script, true), (Script, true)]);
    }

    // a script the test writes: it raises an error when something it expects is not so
    fn run_script(body: &str, sandbox: SandboxLevel) -> Result<(), String> {
        let dir = tempdir().unwrap();
        let script = dir.path().join("t.lua");
        std::fs::write(&script, body).unwrap();
        let ctx = CommandContext { phrase: "x".into(), command_id: "t".into(), command_path: dir.path().to_path_buf(), language: "ru".into(), slots: None };
        execute(&script, ctx, sandbox, Duration::from_secs(5)).map(|_| ()).map_err(|e| e.to_string())
    }

    // io.popen / io.open / os.execute / os.getenv / require / load: the ways around every rule of policy.rs
    #[test]
    fn a_script_has_no_io_no_os_that_acts_and_no_way_to_load_code() {
        for sandbox in [SandboxLevel::Minimal, SandboxLevel::Standard, SandboxLevel::Full] {
            let r = run_script(
                r#"
                for _, name in ipairs({ "io", "package", "require", "debug", "load", "loadstring", "loadfile", "dofile" }) do
                  if _G[name] ~= nil then error(name .. " is available") end
                end
                if os ~= nil then
                  for _, name in ipairs({ "execute", "exit", "remove", "rename", "setlocale", "getenv", "tmpname" }) do
                    if os[name] ~= nil then error("os." .. name .. " is available") end
                  end
                  if type(os.time()) ~= "number" or type(os.date("%H")) ~= "string" then error("the clock is gone") end
                end
                "#,
                sandbox,
            );
            assert!(r.is_ok(), "{:?}: {:?}", sandbox, r);
        }
    }

    // jarvis.system.exec no longer runs a line of text; run() takes a named program with parameters of a known shape
    #[test]
    fn exec_is_refused_and_run_is_limited_to_the_named_operations() {
                let _r = crate::policy::enter_command("t", "medium", std::path::Path::new("/tmp"));
        let r = run_script(
            r#"
            local ok, err = pcall(jarvis.system.exec, "calc")
            if ok then error("exec ran") end
            if not tostring(err):find("Запрещено") then error("exec: " .. tostring(err)) end
            local r = jarvis.system.run("cmd", { "/C", "calc" })
            if not r.denied or r.success then error("cmd /C ran") end
            r = jarvis.system.run("sh", { "-c", "id" })
            if not r.denied then error("sh ran") end
            r = jarvis.system.run("powershell", { "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", "calc" })
            if not r.denied then error("a free PowerShell script ran") end
            -- a forced kill is riskier than a medium command
            r = jarvis.system.run("taskkill", { "/f", "/im", "notepad.exe" })
            if not r.denied then error("a medium command ended a process by force") end
            r = jarvis.system.run("taskkill", { "/im", "csrss.exe" })
            if not r.denied then error("a protected process") end
            -- a named operation within the risk is not refused (the program does not exist here: it fails, it is not denied)
            r = jarvis.system.run("shutdown", { "/a" })
            if r.denied then error("shutdown /a was refused: " .. tostring(r.stderr)) end
            "#,
            SandboxLevel::Full,
        );
        assert!(r.is_ok(), "{:?}", r);
    }

    // the file functions of a script: the user's folders yes, Windows / Jarvis / links out of them no
    #[test]
    fn the_file_functions_of_a_script_follow_the_protected_places() {
                let pack = tempdir().unwrap();
        let base = crate::policy::tests::plain(&std::fs::canonicalize(pack.path()).unwrap());
        let (home, windows, elsewhere) = (base.join("Documents"), base.join("Windows"), base.join("elsewhere"));
        for d in [&home, &windows, &elsewhere] {
            std::fs::create_dir_all(d).unwrap();
        }
        crate::policy::set_test_allowed(vec![home.clone()]);
        crate::policy::set_test_protected(vec![windows.clone()]);
        #[cfg(unix)]
        std::os::unix::fs::symlink(&windows, home.join("link")).unwrap();
        let q = |p: &std::path::Path| p.to_string_lossy().replace('\\', "\\\\");
        let body = format!(
            r#"
            local function denied(f, ...)
              local ok, err = pcall(f, ...)
              if ok then error("allowed: " .. tostring(select(1, ...))) end
              if not tostring(err):find("Запрещено") then error("not a policy refusal: " .. tostring(err)) end
            end
            -- the user's folder
            if not jarvis.fs.write("{h}/tasks.txt", "a") then error("write") end
            if not jarvis.fs.append("{h}/tasks.txt", "b") then error("append") end
            if jarvis.fs.read_text("{h}/tasks.txt") ~= "ab" then error("read") end
            if not jarvis.fs.mkdir("{h}/sub/deeper") then error("mkdir") end
            if not jarvis.fs.remove("{h}/tasks.txt") then error("remove") end
            -- Windows, a place that is not the user's, a way out with dots, a relative path, a link
            denied(jarvis.fs.write, "{w}/a.txt", "x")
            denied(jarvis.fs.mkdir, "{w}/new")
            denied(jarvis.fs.write, "{e}/a.txt", "x")
            denied(jarvis.fs.write, "{h}/../Windows/a.txt", "x")
            denied(jarvis.fs.write, "a.txt", "x")
            denied(jarvis.fs.remove, "{h}")
            denied(jarvis.fs.write, "{h}/CON", "x")
            if "{link}" ~= "" then denied(jarvis.fs.write, "{h}/link/a.txt", "x") end
            "#,
            h = q(&home),
            w = q(&windows),
            e = q(&elsewhere),
            link = if cfg!(unix) { "yes" } else { "" }
        );
        let ctx = CommandContext { phrase: "x".into(), command_id: "t".into(), command_path: pack.path().to_path_buf(), language: "ru".into(), slots: None };
        let script = pack.path().join("t.lua");
        std::fs::write(&script, body).unwrap();
        let r = execute(&script, ctx, SandboxLevel::Full, Duration::from_secs(5)).map(|_| ()).map_err(|e| e.to_string());
        crate::policy::set_test_allowed(Vec::new());
        crate::policy::set_test_protected(Vec::new());
        assert!(r.is_ok(), "{:?}", r);
        assert!(!windows.join("a.txt").exists() && !windows.join("new").exists() && !elsewhere.join("a.txt").exists());
    }

    // the same file functions of a script of the Standard level, which can reach only its own folder
    #[test]
    fn a_script_of_the_standard_level_cannot_write_into_the_folder_of_its_command() {
                let pack = tempdir().unwrap();
        let base = std::fs::canonicalize(pack.path()).unwrap();
        // the folder of a command is a part of the installed Jarvis
        crate::policy::set_test_protected(vec![base.clone()]);
        let r2 = {
            let ctx = CommandContext { phrase: "x".into(), command_id: "t".into(), command_path: base.clone(), language: "ru".into(), slots: None };
            let script = base.join("t.lua");
            std::fs::write(&script, r#"local ok, err = pcall(jarvis.fs.write, "own.txt", "x"); if ok then error("wrote into the folder of the command") end"#).unwrap();
            execute(&script, ctx, SandboxLevel::Standard, Duration::from_secs(5)).map(|_| ()).map_err(|e| e.to_string())
        };
        crate::policy::set_test_protected(Vec::new());
        assert!(r2.is_ok(), "{:?}", r2);
        assert!(!base.join("own.txt").exists());
    }

    fn copy_tree(from: &std::path::Path, to: &std::path::Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let (a, b) = (e.path(), to.join(e.file_name()));
            if a.is_dir() {
                copy_tree(&a, &b);
            } else {
                std::fs::copy(&a, &b).unwrap();
            }
        }
    }

    // (not on Windows: there `run` really starts taskkill)
    #[cfg(not(target_os = "windows"))]
    // the real path of a command: a script that is not the one of the build runs as a plain low command of the
    // Standard level, so the process it starts is refused and the file it writes is not written
    #[test]
    fn a_command_whose_script_is_not_the_one_of_the_build_runs_without_its_powers() {
        let dir = tempdir().unwrap();
        let root = dir.path().join("commands");
        copy_tree(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/commands"), &root);
        let list = crate::integrity::parse(include_str!(concat!(env!("OUT_DIR"), "/commands_manifest.txt")));
        let _s = crate::integrity::TestSetup::new(&root, list);
        let pack = root.join("steam");
        let toml_text = std::fs::read_to_string(pack.join("command.toml")).unwrap();
        let file: crate::commands::JCommandsList = toml::from_str(&toml_text).unwrap();
        let steam_close = file.commands.iter().find(|c| c.id == "steam_2").unwrap().clone();
        assert_eq!(steam_close.risk, "high");
        // the script of the build: a forced kill of steam is within the declared power (nothing is started here)
        crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
        {
            // without the confirmation of the user the forced kill is refused even for the script of the build
            let _n = crate::confirm::confirmed_run("steam_2", false);
            assert!(crate::commands::execute_command(&pack, &steam_close, Some("закрой стим"), None).is_ok());
            let said = crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().last().cloned().unwrap_or_default());
            assert!(!said.contains("Closed") && !said.contains("Закрыт"), "an unconfirmed kill must not report success: {}", said);
        }
        crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow_mut().clear());
        let _confirmed = crate::confirm::confirmed_run("steam_2", true);
        assert!(crate::commands::execute_command(&pack, &steam_close, Some("закрой стим"), None).is_ok());
        let said = crate::lua::api::core::TEST_REPLIES.with(|r| r.borrow().last().cloned().unwrap_or_default());
        assert!(said.contains("Closed") || said.contains("Закрыт"), "the trusted command ran its kill: {}", said);
        // the same command with a script that is not the build's: it may not start anything
        std::fs::write(pack.join("actions.lua"), r#"local r = run("taskkill", { "/f", "/im", "steam.exe" }); if r.denied then error("DENIED: " .. tostring(r.stderr)) end"#).unwrap();
        let err = crate::commands::execute_command(&pack, &steam_close, Some("закрой стим"), None).unwrap_err();
        assert!(err.contains("DENIED"), "{}", err);
        // and the Full-level functions are gone for it (the Standard level has no jarvis.system.run)
        std::fs::write(pack.join("actions.lua"), r#"if jarvis.system.run ~= nil or jarvis.system.clipboard.set ~= nil then error("FULL LEVEL") end"#).unwrap();
        assert!(crate::commands::execute_command(&pack, &steam_close, Some("закрой стим"), None).is_ok(), "an untrusted script runs at the Standard level");
    }

}

