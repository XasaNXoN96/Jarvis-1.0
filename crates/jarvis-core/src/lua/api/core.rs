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
    tts.set("is_enabled", lua.create_function(|_, ()| Ok(crate::tts::is_enabled()))?)?;
    jarvis.set("tts", tts)?;

    // jarvis.app.set_mic_muted(bool): switch the microphone off (text-only) / on
    let app = lua.create_table()?;
    app.set("set_mic_muted", lua.create_function(|_, muted: bool| {
        crate::MIC_MUTED.store(muted, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })?)?;
    app.set("is_mic_muted", lua.create_function(|_, ()| Ok(crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst)))?)?;
    jarvis.set("app", app)?;

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