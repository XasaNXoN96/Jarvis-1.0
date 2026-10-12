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
        let read = run("assistant_task_read", "озвучь задачи");
        assert!(read.contains("1. купить хлеб") && read.contains("2. позвонить врачу"), "{}", read);
        assert!(run("assistant_task_done", "выполнил задачу 1").contains("купить хлеб"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "позвонить врачу 😀\n");
        assert!(run("assistant_task_done", "выполнил задачу 7").contains("номер"), "a wrong number removes nothing");
        // "стереть все задачи": asks first; "нет" keeps them, "да" deletes them
        assert!(run("assistant_task_clear", "стереть все задачи").contains("да"));
        assert_eq!(crate::session::kind().as_deref(), Some("confirm"));
        run("assistant_session", "нет");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "позвонить врачу 😀\n", "kept after no");
        run("assistant_task_clear", "стереть все задачи");
        assert!(run("assistant_session", "да").contains("стёрты"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
        crate::session::end();
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
}
