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
        let _g = CLIPBOARD_LOCK.lock();
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
}