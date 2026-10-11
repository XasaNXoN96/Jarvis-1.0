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
        crate::dialog::collect(&text);
        // also show it in the Jarvis window
        #[cfg(feature = "jarvis_app")]
        crate::ipc::send(crate::ipc::IpcEvent::Output { text });
        Ok(())
    })?;
    jarvis.set("out", out_fn)?;

    // @ jarvis.reply(text, title?)
    // the reply of a command to the user: it is shown in the window, kept in the one conversation (next to the question
    // that caused it) and shown on the console. The voice is a separate call, so a reply is never said twice.
    let reply_fn = lua.create_function(|_, (text, title): (String, Option<String>)| {
        use std::io::Write;
        let line = match &title {
            Some(t) if !t.is_empty() => format!("{}: {}", t, text),
            _ => text.clone(),
        };
        let _ = writeln!(std::io::stdout(), "{}", line);
        #[cfg(feature = "jarvis_app")]
        crate::ipc::send(crate::ipc::IpcEvent::Output { text: line });
        // in the conversation the title stays with the message ("Ищу: нейросеть"), unless the message already names it
        let shown = match &title {
            Some(t) if !t.is_empty() && !matches!(t.as_str(), "Джарвис" | "Jarvis") && !text.to_lowercase().contains(&t.to_lowercase()) => format!("{}: {}", t, text),
            _ => text.clone(),
        };
        crate::dialog::reply(&shown, "command");
        #[cfg(test)]
        TEST_REPLIES.with(|r| r.borrow_mut().push(shown.clone()));
        Ok(())
    })?;
    jarvis.set("reply", reply_fn)?;

    // @ jarvis.speak(text)
    // @TODO: update when TTS will be implemented
    let speak_fn = lua.create_function(|_, text: String| {
        log::info!("[Lua] SPEAK: {}", text);
        crate::tts::speak(&text);
        Ok(())
    })?;
    jarvis.set("speak", speak_fn)?;

    // jarvis.text.preview(text, n) / jarvis.text.len(text): cut by CHARACTERS, not bytes.
    // Lua's string.sub counts bytes: cutting Russian text there splits a letter in two and the broken string
    // can no longer be passed to jarvis.reply / jarvis.out ("invalid UTF-8"). These helpers never split a character.
    let text_api = lua.create_table()?;
    text_api.set("preview", lua.create_function(|_, (text, n): (mlua::String, Option<usize>)| {
        Ok(preview(&text.to_string_lossy(), n.unwrap_or(80)))
    })?)?;
    text_api.set("len", lua.create_function(|_, text: mlua::String| Ok(text.to_string_lossy().chars().count()))?)?;
    // jarvis.text.lower(text): lower case for any language (Lua's string.lower knows only Latin letters)
    text_api.set("lower", lua.create_function(|_, text: mlua::String| Ok(text.to_string_lossy().to_lowercase().replace('ё', "е")))?)?;
    jarvis.set("text", text_api)?;

    // jarvis.tts.stop() / jarvis.tts.set_enabled(bool) / jarvis.tts.is_enabled()
    let tts = lua.create_table()?;
    tts.set("stop", lua.create_function(|_, ()| { crate::tts::stop(); Ok(()) })?)?;
    tts.set("set_enabled", lua.create_function(|_, on: bool| { crate::tts::set_enabled(on); Ok(()) })?)?;
    tts.set("diagnose", lua.create_function(|_, ()| { crate::tts::diagnose(); Ok(()) })?)?;
    tts.set("is_enabled", lua.create_function(|_, ()| Ok(crate::tts::is_enabled()))?)?;
    // jarvis.tts.can_speak() -> true | false, reason: whether a phrase can be said right now (speech on, a voice installed)
    tts.set("can_speak", lua.create_function(|_, ()| Ok(match crate::tts::can_speak() { Ok(()) => (true, None), Err(why) => (false, Some(why)) }))?)?;
    jarvis.set("tts", tts)?;

    // jarvis.app.set_mic_muted(bool): switch the microphone off (text-only) / on
    let app = lua.create_table()?;
    app.set("set_mic_muted", lua.create_function(|_, muted: bool| {
        crate::uistate::set_mic_muted(muted);
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
    // jarvis.app.needs_toast(): true when nobody would see or hear the reply (no window open, the voice is off)
    app.set("needs_toast", lua.create_function(|_, ()| {
        #[cfg(feature = "jarvis_app")]
        let window = crate::ipc::has_clients();
        #[cfg(not(feature = "jarvis_app"))]
        let window = false;
        Ok(!window && !crate::tts::is_enabled())
    })?)?;
    // jarvis.app.set_browser("opera" | "edge" | "chrome" | "firefox" | "brave" | "yandex" | ""): the browser for sites
    app.set("set_browser", lua.create_function(|_, key: String| {
        crate::uistate::update_settings(&serde_json::json!({ "browser": key }));
        Ok(crate::uistate::settings().browser == key)
    })?)?;
    // jarvis.app.city() / jarvis.app.set_city(name): the one city of the program (the weather block and the weather command)
    app.set("city", lua.create_function(|_, ()| Ok(crate::uistate::settings().city))?)?;
    app.set("set_city", lua.create_function(|_, name: String| {
        crate::uistate::update_settings(&serde_json::json!({ "city": name }));
        Ok(())
    })?)?;
    // jarvis.app.once(key, seconds): true the first time, false when the same key was used less than `seconds` ago
    // (the same program started twice in a row is started once); jarvis.app.forget(key): the action failed, allow it again
    app.set("once", lua.create_function(|_, (key, seconds): (String, Option<f64>)| {
        let secs = seconds.unwrap_or(crate::guard::ACTION_WINDOW.as_secs_f64()).clamp(0.1, 60.0);
        Ok(crate::guard::first(&format!("lua:{}", key), std::time::Duration::from_secs_f64(secs)))
    })?)?;
    app.set("forget", lua.create_function(|_, key: String| {
        crate::guard::forget(&format!("lua:{}", key));
        Ok(())
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

    // test seam: when JARVIS_TEST_RUNNER is set, scripts run on a non-Windows machine send their PowerShell calls to it
    // (the automated tests of the document mode use a fake Word / Excel / Notepad); never set in normal use
    jarvis.set("test_runner", std::env::var("JARVIS_TEST_RUNNER").ok())?;

    // jarvis.apps.resolve(phrase) -> { id, name, start, process, office, path } for a program named in the phrase
    // (spoken variants, "ворт" = "Word"); `path` is where it is installed when found. nil when no program is named.
    let apps = lua.create_table()?;
    apps.set("resolve", lua.create_function(|lua, phrase: String| {
        match crate::apps::find_in_phrase(&phrase) {
            Some(m) => {
                let t = lua.create_table()?;
                t.set("id", m.app.id)?;
                t.set("name", m.app.name)?;
                t.set("start", m.app.start)?;
                t.set("process", m.app.process)?;
                t.set("office", m.app.office)?;
                t.set("path", crate::apps::locate(m.app))?;
                // the website of the same program (Discord, Telegram, Spotify): opened when the program is not installed
                t.set("web", crate::sites::by_id(m.app.id).map(|s| s.url))?;
                Ok(mlua::Value::Table(t))
            }
            None => Ok(mlua::Value::Nil),
        }
    })?)?;
    jarvis.set("apps", apps)?;

    // jarvis.sites.direct(phrase) -> url, name | nil: "открой YouTube" names a known site
    // jarvis.sites.find(name) -> url, name | nil: the site called exactly so ("ютуб", "яндекс карты")
    let sites = lua.create_table()?;
    sites.set("direct", lua.create_function(|_, phrase: String| Ok(crate::sites::direct_open(&phrase).map(|s| (s.url, s.name)).unzip()))?)?;
    sites.set("find", lua.create_function(|_, name: String| Ok(crate::sites::find(&crate::sites::words(&name)).map(|s| (s.url, s.name)).unzip()))?)?;
    jarvis.set("sites", sites)?;

    // jarvis.ai.screen(question) -> answer | nil, reason: a question about the screen (only when allowed in the settings)
    let ai = lua.create_table()?;
    ai.set("screen", lua.create_function(|_, question: String| {
        Ok(match crate::ai::ask_about_screen(&question) {
            Ok(a) => (Some(a), None),
            Err(e) => (None, Some(e.user_text())),
        })
    })?)?;
    // jarvis.ai.ready() -> true | false, reason: the AI is switched on and a provider is set up (nothing is sent)
    ai.set("ready", lua.create_function(|_, ()| {
        Ok(match crate::ai::conf_from_settings() {
            Ok(_) => (true, None),
            Err(e) => (false, Some(e.user_text())),
        })
    })?)?;
    jarvis.set("ai", ai)?;

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
    // jarvis.remote.send(kind, data) -> reply | nil, error   (kind: ring, stop, link, text)
    remote.set("send", lua.create_function(|_, (kind, data): (String, Option<String>)| {
        Ok(match crate::remote::send_to_phone(&kind, data.as_deref().unwrap_or("")) {
            Ok(m) => (Some(m), None),
            Err(e) => (None, Some(e)),
        })
    })?)?;
    // jarvis.remote.phone() -> { online, battery, charging, last_seen, ... }
    remote.set("phone", lua.create_function(|lua, ()| {
        let j = crate::remote::phone_json();
        let t = lua.create_table()?;
        t.set("online", j["online"].as_bool().unwrap_or(false))?;
        t.set("battery", j["battery"].as_u64())?;
        t.set("charging", j["charging"].as_bool())?;
        t.set("last_seen", j["last_seen"].as_u64())?;
        Ok(t)
    })?)?;
    jarvis.set("remote", remote)?;
    
    Ok(())
}
// the first `n` characters of the text on one line ("…" when something was cut)
pub fn preview(text: &str, n: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= n {
        flat
    } else {
        format!("{}…", flat.chars().take(n).collect::<String>().trim_end())
    }
}

// the replies of the scripts run by this thread (tests read what a command answered without racing other tests)
#[cfg(test)]
thread_local! {
    pub static TEST_REPLIES: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
}

#[cfg(test)]
mod preview_tests {
    use super::preview;

    #[test]
    fn the_preview_never_splits_a_character() {
        assert_eq!(preview("Привет,\n  мир", 80), "Привет, мир");
        assert_eq!(preview(&"ж".repeat(81), 80), format!("{}…", "ж".repeat(80)));
        let emoji = "🙂".repeat(100);
        let p = preview(&emoji, 10);
        assert_eq!(p, format!("{}…", "🙂".repeat(10)));
        let long = "a б 🌍 ".repeat(5000);
        assert!(preview(&long, 200).chars().count() <= 201);
        assert_eq!(preview("", 80), "");
        assert_eq!(preview("   \n\t ", 80), "");
        // a combined emoji may be shortened at a joiner, but the result is always valid text
        let family = "👨‍👩‍👧‍👦".repeat(30);
        assert!(std::str::from_utf8(preview(&family, 7).as_bytes()).is_ok());
    }
}
