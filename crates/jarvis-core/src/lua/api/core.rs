// Core Lua API: log, sleep, print, etc.

use mlua::{Lua, Table, MultiValue};

pub fn register(lua: &Lua, jarvis: &Table) -> mlua::Result<()> {

    // @ jarvis.log(level, message)
    // log something
    let log_fn = lua.create_function(|_, (level, message): (String, String)| {
        match level.to_lowercase().as_str() {
            "debug" => log::debug!("[Lua] {}", message),
            "info" => log::info!("[Lua] {}", message),
            "warn" => log::warn!("[Lua] {}", message),
            "error" => log::error!("[Lua] {}", message),
            _ => log::info!("[Lua] {}", message),
        }
        Ok(())
    })?;
    jarvis.set("log", log_fn)?;
    
    // @ jarvis.print(...)
    // simple print
    let print_fn = lua.create_function(|_, args: MultiValue| {
        let parts: Vec<String> = args.iter()
            .map(|v| format!("{:?}", v))
            .collect();
        log::info!("[Lua] {}", parts.join(" "));
        Ok(())
    })?;
    jarvis.set("print", print_fn)?;
    
    // @ jarvis.sleep(ms)
    // ..zZZ
    let sleep_fn = lua.create_function(|_, ms: u64| {
        std::thread::sleep(std::time::Duration::from_millis(ms));
        Ok(())
    })?;
    jarvis.set("sleep", sleep_fn)?;
    
    // @ jarvis.out(text)
    // print a line to the console (visible in text mode)
    let out_fn = lua.create_function(|_, text: String| {
        use std::io::Write;
        let _ = writeln!(std::io::stdout(), "{}", text);
        // also show it in the Jarvis window
        #[cfg(feature = "jarvis_app")]
        crate::ipc::send(crate::ipc::IpcEvent::Output { text });
        Ok(())
    })?;
    jarvis.set("out", out_fn)?;

    // @ jarvis.speak(text)
    // @TODO: update when TTS will be implemented
    let speak_fn = lua.create_function(|_, text: String| {
        log::info!("[Lua] SPEAK: {}", text);
        crate::tts::speak(&text);
        Ok(())
    })?;
    jarvis.set("speak", speak_fn)?;

    // jarvis.tts.stop() / jarvis.tts.set_enabled(bool) / jarvis.tts.is_enabled()
    let tts = lua.create_table()?;
    tts.set("stop", lua.create_function(|_, ()| { crate::tts::stop(); Ok(()) })?)?;
    tts.set("set_enabled", lua.create_function(|_, on: bool| { crate::tts::set_enabled(on); Ok(()) })?)?;
    tts.set("diagnose", lua.create_function(|_, ()| { crate::tts::diagnose(); Ok(()) })?)?;
    tts.set("is_enabled", lua.create_function(|_, ()| Ok(crate::tts::is_enabled()))?)?;
    jarvis.set("tts", tts)?;

    // jarvis.app.set_mic_muted(bool): switch the microphone off (text-only) / on
    let app = lua.create_table()?;
    app.set("set_mic_muted", lua.create_function(|_, muted: bool| {
        crate::MIC_MUTED.store(muted, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })?)?;
    app.set("is_mic_muted", lua.create_function(|_, ()| Ok(crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst)))?)?;
    // jarvis.app.commands() -> { {pack=, id=, phrases={...}}, ... } for the current language
    app.set("commands", lua.create_function(|lua, ()| {
        let out = lua.create_table()?;
        let lang = crate::i18n::get_language();
        let mut n = 0;
        if let Some(list) = crate::COMMANDS_LIST.get() {
            // by section, in the order of the window (sound, timers, ...)
            for g in crate::uistate::GROUPS {
                for pack in list {
                    let pack_name = pack.path.file_name().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                    for cmd in &pack.commands {
                        if crate::uistate::group_key(&pack_name, &cmd.id) != g.0 {
                            continue;
                        }
                        let phrases = cmd.get_phrases(&lang);
                        if phrases.is_empty() {
                            continue;
                        }
                        n += 1;
                        let row = lua.create_table()?;
                        row.set("pack", pack_name.as_str())?;
                        row.set("group", format!("{} {}", g.1, g.2))?;
                        row.set("id", cmd.id.as_str())?;
                        let ph = lua.create_table()?;
                        for (i, p) in phrases.iter().enumerate() {
                            ph.set(i + 1, p.as_str())?;
                        }
                        row.set("phrases", ph)?;
                        out.set(n, row)?;
                    }
                }
            }
        }
        Ok(out)
    })?)?;
    // jarvis.app.run_text(text): handle a phrase as an ordinary command (it goes to the queue of typed commands)
    app.set("run_text", lua.create_function(|_, text: String| Ok(crate::remote::push_text(&text)))?)?;
    // jarvis.app.show_page("commands"): switch the Jarvis window to a page
    app.set("show_page", lua.create_function(|_, page: String| {
        #[cfg(feature = "jarvis_app")]
        crate::ipc::send(crate::ipc::IpcEvent::ShowPage { page });
        #[cfg(not(feature = "jarvis_app"))]
        let _ = page;
        Ok(())
    })?)?;
    jarvis.set("app", app)?;

    // jarvis.session: a conversation without the wake word (see session.rs)
    let session = lua.create_table()?;
    session.set("start", lua.create_function(|_, (kind, secs): (String, Option<u64>)| { crate::session::start(&kind, secs.unwrap_or(60)); Ok(()) })?)?;
    session.set("finish", lua.create_function(|_, ()| { crate::session::end(); Ok(()) })?)?;
    session.set("kind", lua.create_function(|_, ()| Ok(crate::session::kind()))?)?;
    session.set("touch", lua.create_function(|_, ()| { crate::session::touch(); Ok(()) })?)?;
    session.set("data", lua.create_function(|_, ()| Ok(crate::session::data()))?)?;
    session.set("set_data", lua.create_function(|_, v: String| { crate::session::set_data(&v); Ok(()) })?)?;
    jarvis.set("session", session)?;

    // jarvis.files.search(query): background search on this computer, the answer arrives by itself
    // jarvis.files.results() -> { {path=, name=, dir=, is_dir=}, ... } (last finished search)
    let files = lua.create_table()?;
    files.set("search", lua.create_function(|_, query: String| {
        crate::filesearch::start(&query);
        Ok(())
    })?)?;
    files.set("results", lua.create_function(|lua, ()| {
        let out = lua.create_table()?;
        for (i, item) in crate::filesearch::results().iter().enumerate() {
            let row = lua.create_table()?;
            row.set("path", item.path.display().to_string())?;
            row.set("name", item.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())?;
            row.set("dir", item.path.parent().map(|p| p.display().to_string()).unwrap_or_default())?;
            row.set("is_dir", item.is_dir)?;
            out.set(i + 1, row)?;
        }
        Ok(out)
    })?)?;
    jarvis.set("files", files)?;

    // jarvis.remote.enable() -> url | nil, error ; disable() ; is_enabled()
    let remote = lua.create_table()?;
    remote.set("enable", lua.create_function(|_, ()| {
        Ok(match crate::remote::start() {
            Ok(url) => (Some(url), None),
            Err(e) => (None, Some(e)),
        })
    })?)?;
    remote.set("disable", lua.create_function(|_, ()| { crate::remote::stop(); Ok(()) })?)?;
    remote.set("is_enabled", lua.create_function(|_, ()| Ok(crate::remote::is_running()))?)?;
    remote.set("url", lua.create_function(|_, ()| Ok(crate::remote::url()))?)?;
    jarvis.set("remote", remote)?;
    
    Ok(())
}