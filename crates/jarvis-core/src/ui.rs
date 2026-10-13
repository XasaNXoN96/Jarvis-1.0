// The Jarvis window: a small web page (chat, orb, commands, settings) shown in its own
// app-style browser window (Edge / Chrome without address bar).
//
// This server listens on 127.0.0.1 only (never on the network). The page talks to Jarvis
// over the local WebSocket (ipc, port 9712) and loads the command list from /commands.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Command;
use std::time::Duration;

use serde_json::json;

use crate::{i18n, COMMANDS_LIST};

pub const PORT: u16 = 9714;
const PAGE: &str = include_str!("ui_page.html");
const ICON: &[u8] = include_bytes!("../../../resources/icons/128x128.png");

pub fn url() -> String {
    format!("http://127.0.0.1:{}/", PORT)
}

pub fn start() {
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(l) => l,
        Err(e) => {
            warn!("UI: cannot listen on port {}: {} (is another Jarvis running?)", PORT, e);
            return;
        }
    };
    info!("UI: window page on {}", url());
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            std::thread::spawn(move || {
                let _ = handle(stream);
            });
        }
    });
}

fn respond(stream: &mut TcpStream, status: &str, content_type: &str, body: &str) {
    let _ = write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
        status,
        content_type,
        body.as_bytes().len(),
        body
    );
}

fn commands_json() -> String {
    let lang = i18n::get_language();
    let mut list = Vec::new();
    if let Some(commands) = COMMANDS_LIST.get() {
        for pack in commands {
            for cmd in &pack.commands {
                let phrases = cmd.get_phrases(&lang);
                if phrases.is_empty() {
                    continue;
                }
                let pack_name = pack.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                let group = crate::uistate::group_key(&pack_name, &cmd.id);
                list.push(json!({
                    "id": cmd.id,
                    "pack": pack_name,
                    "group": group,
                    "phrases": phrases.iter().take(5).collect::<Vec<_>>(),
                    "description": cmd.description,
                    // the words the command is recognized by (synonyms): searched, not shown
                    "keywords": catalog_keywords(&cmd.id, &cmd.get_triggers(&lang)),
                    // on = runs when said; partial = only under a condition; unavailable = explains why it cannot;
                    // duplicate = a newer command does the same; off = its section is switched off by the user
                    "status": if crate::uistate::is_group_disabled(group) { "off" } else { crate::uistate::command_status(&cmd.id).map(|s| s.0).unwrap_or("on") },
                    "note": crate::uistate::command_status(&cmd.id).map(|s| s.1).unwrap_or(""),
                }));
            }
        }
    }
    let mut groups: Vec<_> = crate::uistate::GROUPS
        .iter()
        .map(|g| json!({ "key": g.0, "emoji": g.1, "title": g.2, "about": g.3, "disabled": crate::uistate::is_group_disabled(g.0), "locked": g.0 == "control" }))
        .collect();
    groups.extend(crate::uistate::UNAVAILABLE_GROUPS.iter().map(|g| json!({ "key": g.0, "emoji": g.1, "title": g.2, "about": g.3, "unavailable": true })));
    json!({ "language": lang, "commands": list, "groups": groups }).to_string()
}

// the words a command is found by in the catalog: its triggers, and for the program and website commands the names
// of the programs and sites they know ("телега", "тотал командер", "ютуб")
fn catalog_keywords(id: &str, triggers: &[String]) -> String {
    let mut words: Vec<String> = triggers.to_vec();
    match id {
        "assistant_program_open" | "assistant_program_close" => {
            for a in crate::apps::APPS {
                words.push(a.name.to_string());
                words.extend(a.aliases.iter().map(|x| x.to_string()));
            }
        }
        "assistant_website" => {
            for site in crate::sites::SITES {
                words.push(site.name.to_string());
                words.extend(site.aliases.iter().map(|x| x.to_string()));
            }
        }
        _ => {}
    }
    words.join(" ")
}

fn pack_name(path: &std::path::Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

fn modules_json() -> serde_json::Value {
    let mut counts: std::collections::HashMap<&'static str, usize> = std::collections::HashMap::new();
    if let Some(commands) = COMMANDS_LIST.get() {
        for pack in commands {
            let name = pack_name(&pack.path);
            for cmd in &pack.commands {
                *counts.entry(crate::uistate::group_key(&name, &cmd.id)).or_insert(0) += 1;
            }
        }
    }
    let list: Vec<_> = crate::uistate::GROUPS
        .iter()
        .map(|g| json!({
            "key": g.0, "emoji": g.1, "title": g.2, "about": g.3,
            "commands": counts.get(g.0).copied().unwrap_or(0),
            "disabled": crate::uistate::is_group_disabled(g.0),
            "locked": g.0 == "control",
        }))
        .collect();
    json!(list)
}

// "Check" button: proves that audio really arrives, step by step
fn mic_check() -> serde_json::Value {
    use crate::recorder::diag::{self, Attempt, Stage};
    use std::sync::atomic::Ordering;
    if crate::MIC_MUTED.load(Ordering::SeqCst) && crate::VOICE_ACTIVE.load(Ordering::SeqCst) {
        return json!({ "ok": false, "lines": ["Микрофон выключен переключателем на странице: включите его и повторите проверку"] });
    }
    let report = if crate::VOICE_ACTIVE.load(Ordering::SeqCst) && crate::recorder::is_open() {
        // the live stream Jarvis listens to: count frames and watch the level for a moment
        let before = crate::recorder::frames_received();
        let mut peak = 0u32;
        for _ in 0..12 {
            std::thread::sleep(Duration::from_millis(100));
            peak = peak.max(crate::MIC_LEVEL.load(Ordering::Relaxed));
        }
        let got = crate::recorder::frames_received().saturating_sub(before);
        let base = diag::LAST_REPORT.lock().clone();
        let idx = crate::recorder::active_index();
        let name = base.attempts.iter().find(|a| a.index == idx).map(|a| a.name.clone()).unwrap_or_else(|| "системный по умолчанию".into());
        let (stage, detail) = if got == 0 {
            (Stage::NoData, "поток открыт, но за секунду не пришло ни одного блока звука".to_string())
        } else {
            (diag::stage_from_peak(peak), String::new())
        };
        let a = Attempt { index: idx, name, stage, cause: None, detail, peak };
        diag::Report { devices: base.devices, attempts: vec![a], working: if got > 0 { Some(idx) } else { None }, summary: base.summary }
    } else {
        let r = crate::recorder::probe_all();
        diag::store(r.clone());
        r
    };
    let mut lines: Vec<String> = Vec::new();
    lines.push(if report.devices.is_empty() { "Устройства записи, которые видит Windows: нет".to_string() } else { format!("Устройств записи найдено: {} ({})", report.devices.len(), report.devices.join(", ")) });
    for a in &report.attempts { lines.push(diag::stage_text(a)); }
    if report.working.is_none() && !report.summary.is_empty() { lines.push(report.summary.clone()); }
    if !report.summary.is_empty() && report.working.is_some() { lines.push(report.summary.clone()); }
    let ok = report.attempts.iter().any(|a| a.stage == Stage::Ok);
    json!({ "ok": ok, "working": report.working.is_some(), "lines": lines })
}

fn settings_page_json() -> serde_json::Value {
    let mut v = crate::uistate::settings_json();
    let devices = crate::recorder::get_audio_devices();
    let selected = crate::DB.get().map(|d| d.read().microphone).unwrap_or(-1);
    v["mic_devices"] = json!(devices);
    v["mic_selected"] = json!(selected);
    v
}

fn apply_settings(patch: &serde_json::Value) -> serde_json::Value {
    crate::uistate::update_settings(patch);
    if let Some(idx) = patch.get("mic_device").and_then(|v| v.as_i64()) {
        if let Some(db) = crate::DB.get() {
            db.write().microphone = idx as i32;
            let snapshot = db.read().clone();
            let _ = crate::db::save_settings(&snapshot);
        }
    }
    json!({ "ok": true })
}

fn run_action(body: &serde_json::Value) -> serde_json::Value {
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
    match name {
        "reaction_preview" => {
            let id = crate::uistate::settings_json()["reaction_voice"].as_str().unwrap_or("").to_string();
            crate::voices::play_preview(&id);
            json!({ "ok": true })
        }
        // "Озвучить" under an answer of the conversation: read aloud by the voice of Jarvis, the setting is not changed
        "speak_text" => {
            let text = crate::ai::speakable(body["text"].as_str().unwrap_or(""));
            if text.trim().is_empty() {
                return json!({ "ok": false, "message": "Нечего озвучивать" });
            }
            match crate::tts::can_speak() {
                Ok(()) => {
                    crate::tts::speak_requested(&text);
                    json!({ "ok": true })
                }
                Err(e) => json!({ "ok": false, "message": format!("Не могу озвучить: {}", e) }),
            }
        }
        "tts_test" => {
            crate::tts::set_enabled(true);
            let text = if i18n::get_language() == "en" { "Hello. This is my voice. How do I sound?" } else { "Здравствуйте. Это мой голос. Как я звучу?" };
            crate::tts::speak(text);
            json!({ "ok": true })
        }
        "tts_toggle" => {
            let on = body.get("value").and_then(|v| v.as_bool()).unwrap_or(true);
            crate::tts::set_enabled(on);
            json!({ "ok": true })
        }
        "mic_muted" => {
            let muted = body.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
            crate::uistate::set_mic_muted(muted);
            json!({ "ok": true })
        }
        // ---- diagnostics: the checks run in the background; the report is built from their last results ----
        "diag_run" => json!({ "ok": true, "started": crate::diag::start() }),
        // ---- notifications ----
        "notify_test" => {
            let id = crate::notify::push("command", "Проверка уведомлений", "Так выглядят уведомления Джарвиса.", false);
            json!({ "ok": true, "shown": id.is_some() })
        }
        "notes_clear" => {
            crate::notify::clear();
            json!({ "ok": true })
        }
        // ---- stored data (asked and confirmed in the window) ----
        // "put every dialog into the archive" (the button of the dialogs page)
        "history_archive_all" => {
            if body.get("confirm").and_then(|v| v.as_str()) != Some("archive") {
                return json!({ "ok": false, "message": "Нужно подтверждение" });
            }
            let (archived, kept) = crate::ai::store().archive_all();
            json!({ "ok": true, "archived": archived, "kept": kept })
        }
        // the privacy section: the whole history, the archive included
        "history_delete_all" => {
            if body.get("confirm").and_then(|v| v.as_str()) != Some("delete") {
                return json!({ "ok": false, "message": "Нужно подтверждение" });
            }
            let (deleted, kept) = crate::ai::store().delete_all();
            let archived = crate::ai::store().archive_clear();
            crate::uistate::log_event("info", &format!("История удалена: диалогов {}, из архива {}", deleted, archived));
            json!({ "ok": true, "deleted": deleted + archived, "kept": kept })
        }
        "settings_reset" => {
            if body.get("confirm").and_then(|v| v.as_str()) != Some("reset") {
                return json!({ "ok": false, "message": "Нужно подтверждение" });
            }
            crate::uistate::reset_settings();
            json!({ "ok": true })
        }
        "autostart" => {
            let on = body.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
            match crate::autostart::set(on) {
                Ok(()) => json!({ "ok": true, "state": crate::autostart::state() }),
                Err(e) => json!({ "ok": false, "message": e, "state": crate::autostart::state() }),
            }
        }
        // "Открыть папку с логами"
        "open_logs" => match crate::APP_LOG_DIR.get() {
            #[cfg(all(feature = "lua", target_os = "windows"))]
            Some(d) => match crate::lua::shell_open(&d.display().to_string()) {
                Ok(()) => json!({ "ok": true }),
                Err(e) => json!({ "ok": false, "message": e }),
            },
            #[cfg(not(all(feature = "lua", target_os = "windows")))]
            Some(d) => json!({ "ok": false, "message": format!("Папка логов: {}", d.display()) }),
            None => json!({ "ok": false, "message": "Папка логов не определена" }),
        },
        "ai_key_forget" => {
            crate::uistate::forget_ai_key();
            json!({ "ok": true })
        }
        "cancel_timer" => {
            #[cfg(feature = "lua")]
            {
                let id = body.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
                return json!({ "ok": crate::lua::cancel_timer(id) });
            }
            #[cfg(not(feature = "lua"))]
            json!({ "ok": false })
        }
        "group" => {
            let key = body.get("key").and_then(|v| v.as_str()).unwrap_or("");
            let disabled = body.get("disabled").and_then(|v| v.as_bool()).unwrap_or(false);
            if key.is_empty() || key == "control" {
                return json!({ "ok": false, "message": "Этот раздел отключить нельзя" });
            }
            crate::uistate::set_group_disabled(key, disabled);
            json!({ "ok": true })
        }
        "ai_test" => ai_test(),
        // the phone remote card of the home page: the same server as the voice command "включи удалённое управление"
        "remote_on" => match crate::remote::start() {
            Ok(url) => json!({ "ok": true, "enabled": true, "url": url }),
            Err(e) => json!({ "ok": false, "enabled": crate::remote::is_running(), "message": format!("Не удалось включить пульт: {}", e) }),
        },
        // the access from any network, without a VPN (a Cloudflare quick tunnel): only by the user's switch
        "remote_tunnel" => {
            if body["value"].as_bool().unwrap_or(false) {
                match crate::tunnel::enable() {
                    Ok(()) => json!({ "ok": true, "tunnel": crate::tunnel::json() }),
                    Err(e) => json!({ "ok": false, "message": e }),
                }
            } else {
                crate::tunnel::disable();
                json!({ "ok": true, "tunnel": crate::tunnel::json() })
            }
        }
        // "Новая ссылка": every phone with the old link loses access (a lost phone, a link shown to someone)
        "remote_new_link" => match crate::remote::renew_token() {
            Ok(_) => json!({ "ok": true, "url": if crate::remote::is_running() { crate::remote::url() } else { String::new() } }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "remote_off" => {
            crate::remote::stop();
            // the listener closes within a fraction of a second
            std::thread::sleep(Duration::from_millis(250));
            json!({ "ok": true, "enabled": crate::remote::is_running() })
        }
        // a second start of Jarvis asks the running one to show its window (instead of opening another window)
        "show_window" => json!({ "ok": open_window() }),
        // "Полностью закрыть JARVIS" of the browser window (the Edge/Chrome window has no window.ipc): the same full
        // exit as the WebView2 window and the tray — the answer goes out first, then Jarvis ends
        "app_exit" => match EXIT_HOOK.get() {
            Some(_) => {
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_millis(200));
                    if let Some(exit) = EXIT_HOOK.get() {
                        exit();
                    }
                });
                json!({ "ok": true })
            }
            None => json!({ "ok": false }),
        },
        // the tray asks the browser window to close itself ("Свернуть в трей" when there is no WebView2 window)
        "hide_window" => {
            hide_window();
            json!({ "ok": true })
        }
        // the "Программы" page: rebuild the index, start a program of the index (never a path from the page itself)
        "apps_rescan" => {
            let started = crate::appindex::rescan_background();
            json!({ "ok": true, "started": started })
        }
        "app_launch" => {
            let launch = body.get("launch").and_then(|v| v.as_str()).unwrap_or("");
            match crate::appindex::apps().into_iter().find(|a| a.launch == launch) {
                Some(a) => match crate::appindex::launch(&a) {
                    Ok(()) => { crate::filesearch::note_launch(&a.name, true); json!({ "ok": true, "message": format!("Запускаю {}", a.name) }) }
                    Err(e) => { crate::filesearch::note_launch(&a.name, false); json!({ "ok": false, "message": e }) }
                },
                None => json!({ "ok": false, "message": "Этой программы нет в списке. Обновите список программ." }),
            }
        }
        // the "Файлы" page
        "file_search" => {
            let f = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let q = crate::filesearch::query_from_form(&f("query"), &f("kind"), &f("scope"), &f("when"), &f("content"));
            match crate::filesearch::start_query(q) {
                Ok(d) => json!({ "ok": true, "message": format!("Ищу: {}", d) }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "file_cancel" => json!({ "ok": true, "stopped": crate::filesearch::cancel() }),
        "file_open" | "file_reveal" => {
            let path = body.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match crate::filesearch::open_result_path(path, name == "file_reveal") {
                Ok(f) => json!({ "ok": true, "message": format!("{}: {}", if name == "file_reveal" { "Показываю в папке" } else { "Открываю" }, f) }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        // phone remote: what the PC can send to the phone page (find the phone, a link, a text)
        "phone_send" => {
            let kind = body.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let data = body.get("data").and_then(|v| v.as_str()).unwrap_or("");
            match crate::remote::send_to_phone(kind, data) {
                Ok(m) => json!({ "ok": true, "message": m }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "voice_install" => match crate::voice_install::start() {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "mic_check" => mic_check(),
        "restart_voice" => {
            // starts a fresh Jarvis without --text (after this one has closed and freed its ports) and quits
            // The new copy is started directly (no cmd line: its quoting broke the path, and Jarvis quit without a
            // new one); it waits until this one has released the microphone and its ports (--wait-instance).
            if let Ok(exe) = std::env::current_exe() {
                let mut next = Command::new(&exe);
                next.arg("--wait-instance");
                crate::null_stdio(&mut next);
                if next.spawn().is_ok() {
                    std::thread::spawn(|| {
                        std::thread::sleep(Duration::from_millis(400));
                        // the same full exit as the tray and the window (the microphone is released)
                        match EXIT_HOOK.get() {
                            Some(exit) => exit(),
                            None => std::process::exit(0),
                        }
                    });
                    return json!({ "ok": true });
                }
            }
            json!({ "ok": false, "message": "Не удалось перезапустить" })
        }
        _ => json!({ "ok": false, "message": "unknown action" }),
    }
}

// "check the connection": asks the provider for its model list with the saved key
fn ai_test() -> serde_json::Value {
    match crate::ai::conf_from_settings() {
        Err(e) => json!({ "ok": false, "message": e.user_text() }),
        Ok(conf) => match crate::ai::test_connection(&conf) {
            Ok(()) => json!({ "ok": true, "message": format!("Соединение с {} работает", conf.name) }),
            Err(e) => json!({ "ok": false, "message": e.user_text() }),
        },
    }
}

// ---- scenarios of the "Конструктор" ----
fn scenarios_json() -> serde_json::Value {
    json!({ "scenarios": crate::scenario::list(), "kinds": crate::scenario::kinds_json(), "templates": crate::scenario::templates(), "running": crate::scenario::running_json() })
}

// the page "Сообщения": clear the list (memory only), read one message aloud
fn messages_action(body: &serde_json::Value) -> serde_json::Value {
    match body["op"].as_str().unwrap_or("") {
        "clear" => json!({ "ok": true, "message": format!("Список очищен ({})", crate::social::clear()) }),
        "speak" => match crate::social::get(body["id"].as_u64().unwrap_or(0)) {
            None => json!({ "ok": false, "message": "Сообщения уже нет в списке" }),
            Some(m) => match crate::tts::can_speak() {
                Ok(()) => {
                    crate::tts::speak_requested(&format!("{}: {}", m.title.replace(" · ", ", "), m.text));
                    json!({ "ok": true })
                }
                Err(e) => json!({ "ok": false, "message": format!("Не могу озвучить: {}", e) }),
            },
        },
        _ => json!({ "ok": false, "message": "unknown op" }),
    }
}

// the panel "Задачи": the same file as the voice commands (tasks.rs)
fn tasks_action(body: &serde_json::Value) -> serde_json::Value {
    let op = body["op"].as_str().unwrap_or("");
    let text = body["text"].as_str().unwrap_or("");
    let old = body["old"].as_str().unwrap_or("");
    let index = body["index"].as_u64().unwrap_or(u64::MAX) as usize;
    let done = |r: Result<String, String>| match r {
        Ok(m) => json!({ "ok": true, "message": m }),
        Err(e) => json!({ "ok": false, "message": e }),
    };
    match op {
        "add" => done(crate::tasks::add(text).map(|n| format!("Задача добавлена ({}/{})", n, crate::tasks::MAX))),
        "edit" => done(crate::tasks::edit(index, old, text).map(|_| "Задача изменена".to_string())),
        "remove" => done(crate::tasks::remove(index, old).map(|_| "Задача удалена".to_string())),
        "clear" => done(crate::tasks::clear().map(|n| format!("Удалено задач: {}", n))),
        // read aloud with the voice of Jarvis; without a working voice the user is told why
        "speak" => done(crate::tasks::list().and_then(|l| {
            crate::tts::can_speak().map_err(|e| format!("Не могу озвучить: {}", e))?;
            crate::tts::speak_requested(&crate::tasks::spoken(&l));
            Ok("Озвучиваю задачи".to_string())
        })),
        _ => json!({ "ok": false, "message": "unknown op" }),
    }
}

fn scenario_action(body: &serde_json::Value) -> serde_json::Value {
    let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "save" => {
            let sc: crate::scenario::Scenario = match body.get("scenario").cloned().map(serde_json::from_value) {
                Some(Ok(s)) => s,
                _ => return json!({ "ok": false, "message": "Сценарий не прочитан" }),
            };
            match crate::scenario::upsert(sc) {
                Ok(s) => {
                    // a phrase that is also a command: the scenario wins, the user is told
                    let note = crate::COMMANDS_LIST
                        .get()
                        .and_then(|list| crate::commands::fetch_by_triggers(&s.phrase, list))
                        .map(|(_, c)| format!("Фраза похожа на команду «{}»: теперь её выполнит сценарий.", c.id))
                        .unwrap_or_default();
                    json!({ "ok": true, "scenario": s, "note": note })
                }
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "delete" => json!({ "ok": crate::scenario::delete(id) }),
        "enable" => json!({ "ok": crate::scenario::set_enabled(id, body.get("value").and_then(|v| v.as_bool()).unwrap_or(true)) }),
        // a test run of the saved scenario, or of one step that is being edited
        // "Тест сценария": what the editor shows now (the order of the steps as they were just moved, even before
        // saving), through the same run() as the phrase; only the stored scenario keeps the result in its history
        "test" => {
            let edited: Option<crate::scenario::Scenario> = body.get("scenario").cloned().and_then(|v| serde_json::from_value(v).ok());
            match edited.or_else(|| crate::scenario::get(id)) {
                Some(s) => {
                    let unsaved = crate::scenario::get(&s.id).map(|stored| stored.steps != s.steps || stored.name != s.name).unwrap_or(true);
                    let result = if unsaved { crate::scenario::run_draft(&s) } else { crate::scenario::run(&s) };
                    json!({ "ok": true, "result": result, "unsaved": unsaved })
                }
                None => json!({ "ok": false, "message": "Добавьте шаги и нажмите «Тест сценария» ещё раз" }),
            }
        }
        "status" => json!({ "ok": true, "running": crate::scenario::running_json() }),
        "test_step" => {
            let st: crate::scenario::Step = match body.get("step").cloned().map(serde_json::from_value) {
                Some(Ok(s)) => s,
                _ => return json!({ "ok": false, "message": "Шаг не прочитан" }),
            };
            match crate::scenario::run_step(&st) {
                Ok(()) => json!({ "ok": true, "message": "Шаг выполнен" }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "export" => json!({ "ok": true, "doc": crate::scenario::export_json() }),
        "import" => {
            let replace = body.get("replace").and_then(|v| v.as_bool()).unwrap_or(false);
            match crate::scenario::import_json(body.get("doc").unwrap_or(&json!({})), replace) {
                Ok((added, skipped)) => json!({ "ok": true, "added": added, "skipped": skipped }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        _ => json!({ "ok": false, "message": "unknown operation" }),
    }
}

// ---- AI chat (the window talks to it over local HTTP; the key never leaves the backend) ----
fn chat_json(id: &str) -> serde_json::Value {
    match crate::ai::store().load_with_live(id) {
        Some(c) => json!({ "ok": true, "id": c.id, "title": c.title, "messages": c.messages, "busy": crate::ai::is_busy(&c.id), "rev": crate::ai::rev() }),
        None => json!({ "ok": false, "message": "Диалог не найден" }),
    }
}

fn chats_json() -> serde_json::Value {
    let store = crate::ai::store();
    let active = store.active().unwrap_or_default();
    let conf = crate::ai::conf_from_settings();
    let s = crate::uistate::settings();
    json!({
        "active": active,
        "chats": store.list(),
        "rev": crate::ai::rev(),
        "configured": conf.is_ok(),
        "problem": conf.as_ref().err().map(|e| e.user_text()).unwrap_or_default(),
        "provider": conf.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
        "model": s.ai_model,
    })
}

fn chat_action(body: &serde_json::Value) -> serde_json::Value {
    let store = crate::ai::store();
    let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
    match op {
        "new" => {
            let c = store.create();
            json!({ "ok": true, "id": c.id })
        }
        "select" => {
            if store.load(id).is_some() {
                store.set_active(id);
                json!({ "ok": true, "id": id })
            } else {
                json!({ "ok": false, "message": "Диалог не найден" })
            }
        }
        "send" => {
            let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("");
            let chat = if id.is_empty() { None } else { Some(id) };
            match crate::ai::send(chat, text, true) {
                Ok(chat_id) => json!({ "ok": true, "id": chat_id }),
                Err(e) => json!({ "ok": false, "message": e.user_text() }),
            }
        }
        // a picture attached to a question: sent to the provider only because the user attached it
        "image" => {
            let f = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            let chat = if id.is_empty() { None } else { Some(id) };
            match crate::ai::ask_about_image(chat, &f("text"), &f("name"), &f("mime"), f("data")) {
                Ok(chat_id) => json!({ "ok": true, "id": chat_id }),
                Err(e) => json!({ "ok": false, "message": e.user_text() }),
            }
        }
        // ONE entry for everything typed in the window: the same router as voice (commands first, then the AI)
        "ask" => {
            let text = body.get("text").and_then(|v| v.as_str()).unwrap_or("").trim();
            if text.is_empty() {
                return json!({ "ok": false, "message": "Сообщение пустое. Напишите вопрос или команду." });
            }
            if text.chars().count() > crate::ai::MAX_MESSAGE_CHARS {
                return json!({ "ok": false, "message": format!("Сообщение слишком длинное (больше {} символов).", crate::ai::MAX_MESSAGE_CHARS) });
            }
            // the page sends the same request id again when it retries: the message is handled once
            let rid = body.get("rid").and_then(|v| v.as_str()).unwrap_or("");
            if crate::guard::request_seen(rid) {
                return json!({ "ok": true, "id": store.active().unwrap_or_default(), "repeat": true });
            }
            if crate::remote::submit(crate::origin::Source::Window, text) {
                json!({ "ok": true, "id": store.active().unwrap_or_default() })
            } else {
                json!({ "ok": false, "message": "Jarvis ещё запускается: повторите через секунду." })
            }
        }
        // stop the generation AND the voice
        "stop" => {
            crate::tts::stop();
            json!({ "ok": crate::ai::stop(id) })
        }
        "clear" => json!({ "ok": !crate::ai::is_busy(id) && store.clear(id) }),
        // "delete a dialog" puts it into the archive (Settings → «Архив диалогов»)
        "delete" => {
            if crate::ai::is_busy(id) {
                return json!({ "ok": false, "message": "Дождитесь конца ответа и попробуйте снова." });
            }
            match store.archive(id) {
                Ok(()) => json!({ "ok": true, "archived": true }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "archive_list" => json!({ "ok": true, "items": store.archive_list(body.get("query").and_then(|v| v.as_str()).unwrap_or("")) }),
        "archive_get" => match store.archive_get(id) {
            Some(v) => json!({ "ok": true, "chat": v }),
            None => json!({ "ok": false, "message": "В архиве нет такого диалога" }),
        },
        "archive_restore" => match store.archive_restore(id) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "archive_delete" => json!({ "ok": store.archive_delete(id) }),
        "archive_clear" => json!({ "ok": true, "deleted": store.archive_clear() }),
        _ => json!({ "ok": false, "message": "unknown operation" }),
    }
}

// QR code (SVG) of the phone-remote link
fn qr_svg(data: &str) -> Option<String> {
    use qrcode::render::svg;
    let code = qrcode::QrCode::new(data.as_bytes()).ok()?;
    Some(
        code.render::<svg::Color>()
            .min_dimensions(300, 300)
            .quiet_zone(true)
            .dark_color(svg::Color("#04101f"))
            .light_color(svg::Color("#ffffff"))
            .build(),
    )
}

// the avatar photo chosen by the user lives in the config dir (never leaves this computer)
fn avatar_path() -> Option<std::path::PathBuf> {
    crate::APP_CONFIG_DIR.get().map(|d| d.join("avatar.img"))
}

const MAX_AVATAR_BYTES: usize = 6 * 1024 * 1024;

struct Request {
    method: String,
    path: String,
    query: String,
    host_ok: bool,
    custom_header: bool,
    body: Vec<u8>,
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Request> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let header_end = loop {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > 32 * 1024 {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
    };
    let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let target = parts.next().unwrap_or("");
    let path = target.split('?').next().unwrap_or("").to_string();
    let query = target.split_once('?').map(|(_, q)| q.to_string()).unwrap_or_default();

    let mut host_ok = false;
    let mut custom_header = false;
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_lowercase(), v.trim());
            match k.as_str() {
                "host" => host_ok = v == format!("127.0.0.1:{}", PORT) || v == format!("localhost:{}", PORT),
                "content-length" => content_length = v.parse().unwrap_or(0),
                // a web page on another site cannot add this header without our permission
                "x-jarvis" => custom_header = v == "1",
                _ => {}
            }
        }
    }
    if content_length > MAX_AVATAR_BYTES {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    while buf.len() < header_end + content_length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    let end = buf.len().min(header_end + content_length);
    Ok(Request { method, path, query, host_ok, custom_header, body: buf[header_end..end].to_vec() })
}

fn respond_bytes(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let _ = write!(
        stream,
        "HTTP/1.1 {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        status,
        content_type,
        body.len()
    );
    let _ = stream.write_all(body);
}

fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("image/png")
    } else if data.len() > 12 && &data[0..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn handle(mut stream: TcpStream) -> std::io::Result<()> {
    let req = read_request(&mut stream)?;

    // only our own pages: protects against DNS-rebinding tricks from web pages
    if !req.host_ok {
        respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
        return Ok(());
    }
    // everything this server does (a phrase, a scenario test, an action) comes through the door of the window
    let _door = crate::origin::enter(crate::origin::Source::Window);

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => respond(&mut stream, "200 OK", "text/html", PAGE),
        ("GET", "/icon.png") | ("GET", "/favicon.ico") => respond_bytes(&mut stream, "200 OK", "image/png", ICON),
        ("GET", "/apps") => {
            let list: Vec<serde_json::Value> = crate::appindex::apps()
                .iter()
                .map(|a| json!({ "name": a.name, "launch": a.launch, "source": crate::appindex::source_title(&a.source), "ok": crate::appindex::launchable(a) }))
                .collect();
            let body = json!({
                "apps": list,
                "scanned_at": crate::appindex::scanned_at(),
                "scanning": crate::appindex::is_scanning(),
                "error": crate::appindex::last_error(),
            });
            respond(&mut stream, "200 OK", "application/json", &body.to_string());
        }
        ("GET", "/files") => {
            respond(&mut stream, "200 OK", "application/json", &crate::filesearch::state_json().to_string());
        }
        ("GET", "/remote") => {
            // phone remote state for the window (the secret link is shown only to this window): every address of
            // this PC a phone can reach, best first (home Wi-Fi, the hotspot of this PC, a VPN for other networks)
            let on = crate::remote::is_running();
            let addrs: Vec<serde_json::Value> = if on {
                crate::remote::addresses()
                    .into_iter()
                    .map(|a| json!({ "ip": a.ip, "kind": a.kind, "label": a.label, "url": crate::remote::url_for(&a.ip, false), "url_https": crate::remote::url_for(&a.ip, true) }))
                    .collect()
            } else {
                Vec::new()
            };
            let body = serde_json::json!({
                "enabled": on,
                "url": if on { crate::remote::url() } else { String::new() },
                "url_https": if on { crate::remote::url_https() } else { String::new() },
                "addresses": addrs,
                "last_client": crate::remote::last_client(),
                "tunnel": crate::tunnel::json(),
            })
            .to_string();
            respond(&mut stream, "200 OK", "application/json", &body);
        }
        // the QR code of the link from any network (the tunnel address with the secret)
        ("GET", "/remote-qr-net") => match crate::tunnel::public_link() {
            None => respond(&mut stream, "404 Not Found", "text/plain", "the internet access is off"),
            Some(link) => match qr_svg(&link) {
                Some(svg) => respond_bytes(&mut stream, "200 OK", "image/svg+xml", svg.as_bytes()),
                None => respond(&mut stream, "500 Internal Server Error", "text/plain", "qr failed"),
            },
        },
        ("GET", "/remote-qr") | ("GET", "/remote-qr-https") => {
            if !crate::remote::is_running() {
                respond(&mut stream, "404 Not Found", "text/plain", "remote control is off");
            } else {
                let https = req.path == "/remote-qr-https";
                // ?ip=… : only an address of this PC (never a link to somewhere else)
                let ip = req.query.split('&').find_map(|kv| kv.strip_prefix("ip=")).map(|v| v.to_string());
                let link = match ip {
                    Some(ip) if crate::remote::addresses().iter().any(|a| a.ip == ip) => crate::remote::url_for(&ip, https),
                    _ => if https { crate::remote::url_https() } else { crate::remote::url() },
                };
                match qr_svg(&link) {
                    Some(svg) => respond_bytes(&mut stream, "200 OK", "image/svg+xml", svg.as_bytes()),
                    None => respond(&mut stream, "500 Internal Server Error", "text/plain", "qr failed"),
                }
            }
        }
        ("GET", "/state") => {
            *LAST_STATE_POLL.lock() = Some(std::time::Instant::now());
            // the window says whether it is visible: results of commands pop up as notifications only when it is not
            if req.query.split('&').any(|p| p == "v=1") {
                *LAST_VISIBLE_POLL.lock() = Some(std::time::Instant::now());
                crate::notify::window_seen();
            }
            respond(&mut stream, "200 OK", "application/json", &crate::uistate::state_json().to_string())
        }
        ("GET", "/diag") => respond(&mut stream, "200 OK", "application/json", &crate::diag::json().to_string()),
        // the report from the last results: nothing is checked, sent or changed by copying it
        ("GET", "/diag-report") => respond(&mut stream, "200 OK", "text/plain", &crate::diag::report()),
        ("GET", "/settings") => respond(&mut stream, "200 OK", "application/json", &settings_page_json().to_string()),
        ("GET", "/voices") => {
            let list: Vec<_> = crate::tts::list_voices().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
            respond(&mut stream, "200 OK", "application/json", &json!(list).to_string());
        }
        ("GET", "/reaction-voices") => {
            let list: Vec<_> = crate::voices::list_voices().iter().map(|v| json!({ "id": v.voice.id, "name": v.voice.name })).collect();
            respond(&mut stream, "200 OK", "application/json", &json!(list).to_string());
        }
        ("GET", "/modules") => respond(&mut stream, "200 OK", "application/json", &modules_json().to_string()),
        ("POST", "/settings") | ("POST", "/action") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            let result = if req.path == "/settings" { apply_settings(&body) } else { run_action(&body) };
            respond(&mut stream, "200 OK", "application/json", &result.to_string());
        }
        // the browser window was closed with its own cross: Jarvis keeps running in the tray, said once per run.
        // A reload of the page also says "closed": if the page asks for its state again soon, nothing is said.
        ("POST", "/window-closed") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            browser_window_closed();
            respond(&mut stream, "200 OK", "application/json", "{\"ok\":true}");
        }
        ("GET", "/voice-install") => respond(&mut stream, "200 OK", "application/json", &crate::voice_install::status_json().to_string()),
        ("GET", "/chats") => respond(&mut stream, "200 OK", "application/json", &chats_json().to_string()),
        ("GET", "/scenarios") => respond(&mut stream, "200 OK", "application/json", &scenarios_json().to_string()),
        ("GET", "/tasks") => respond(&mut stream, "200 OK", "application/json", &crate::tasks::json().to_string()),
        ("GET", "/messages") => respond(&mut stream, "200 OK", "application/json", &crate::social::json().to_string()),
        ("POST", "/messages") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            respond(&mut stream, "200 OK", "application/json", &messages_action(&body).to_string());
        }
        ("GET", "/telegram") => respond(&mut stream, "200 OK", "application/json", &crate::telegram::json().to_string()),
        ("POST", "/telegram") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            respond(&mut stream, "200 OK", "application/json", &crate::telegram::action(&body).to_string());
        }
        ("POST", "/tasks") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            respond(&mut stream, "200 OK", "application/json", &tasks_action(&body).to_string());
        }
        ("POST", "/scenario") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            respond(&mut stream, "200 OK", "application/json", &scenario_action(&body).to_string());
        }
        ("POST", "/chat") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            respond(&mut stream, "200 OK", "application/json", &chat_action(&body).to_string());
        }
        ("GET", p) if p.starts_with("/chat/") => respond(&mut stream, "200 OK", "application/json", &chat_json(&p["/chat/".len()..]).to_string()),
        ("GET", "/commands") => respond(&mut stream, "200 OK", "application/json", &commands_json()),
        ("GET", "/avatar") => {
            match avatar_path().and_then(|p| std::fs::read(p).ok()) {
                Some(data) => {
                    let ct = sniff_image(&data).unwrap_or("application/octet-stream");
                    respond_bytes(&mut stream, "200 OK", ct, &data);
                }
                None => respond(&mut stream, "404 Not Found", "text/plain", "no avatar"),
            }
        }
        ("POST", "/avatar") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let path = match avatar_path() {
                Some(p) => p,
                None => {
                    respond(&mut stream, "500 Internal Server Error", "text/plain", "no config dir");
                    return Ok(());
                }
            };
            if req.body.is_empty() {
                let _ = std::fs::remove_file(&path);
                info!("UI: avatar removed");
                respond(&mut stream, "200 OK", "text/plain", "removed");
            } else if sniff_image(&req.body).is_none() {
                respond(&mut stream, "400 Bad Request", "text/plain", "not an image");
            } else {
                match std::fs::write(&path, &req.body) {
                    Ok(()) => {
                        info!("UI: avatar saved ({} bytes)", req.body.len());
                        respond(&mut stream, "200 OK", "text/plain", "saved");
                    }
                    Err(e) => respond(&mut stream, "500 Internal Server Error", "text/plain", &e.to_string()),
                }
            }
        }
        _ => respond(&mut stream, "404 Not Found", "text/plain", "not found"),
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn try_start(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    Command::new("cmd")
        .args(args)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// What the own window of Jarvis does with an answer of its close dialog ("Свернуть в трей" / "Полностью закрыть" /
// "Отмена"; "ack" = the dialog is on the screen). An unknown answer never exits.
#[derive(Debug, PartialEq, Clone, Copy)]
pub enum CloseAction {
    Hide,
    Exit,
    Stay,
    Ack,
}

pub fn close_action(answer: &str) -> Option<CloseAction> {
    Some(match answer {
        "tray" => CloseAction::Hide,
        "exit" => CloseAction::Exit,
        "cancel" => CloseAction::Stay,
        "ack" => CloseAction::Ack,
        _ => return None,
    })
}

// The window of Jarvis that belongs to the program (the WebView2 window of jarvis-app on Windows): when it is there,
// "open the window" shows it instead of starting a browser window.
static WINDOW_HOST: once_cell::sync::OnceCell<Box<dyn Fn() -> bool + Send + Sync>> = once_cell::sync::OnceCell::new();

pub fn set_window_host(show: Box<dyn Fn() -> bool + Send + Sync>) {
    let _ = WINDOW_HOST.set(show);
}

pub fn has_window_host() -> bool {
    WINDOW_HOST.get().is_some()
}

// The full exit of Jarvis (jarvis-app: stops the voice, releases the microphone, ends the process).
static EXIT_HOOK: once_cell::sync::OnceCell<Box<dyn Fn() + Send + Sync>> = once_cell::sync::OnceCell::new();

pub fn set_exit_hook(exit: Box<dyn Fn() + Send + Sync>) {
    let _ = EXIT_HOOK.set(exit);
}

// the full exit from anywhere (a voice command, the console): the hook of jarvis-app, else a plain exit
pub fn exit_now() {
    match EXIT_HOOK.get() {
        Some(exit) => exit(),
        None => std::process::exit(0),
    }
}

// The browser window cannot be hidden from outside: it is asked to close itself. The page sees the counter grow
// in its state (a page opened later starts from the current value and is not closed by an old request).
static CLOSE_REQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn request_browser_close() {
    CLOSE_REQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    info!("UI: the browser window is asked to close (Jarvis stays in the tray)");
}

pub fn browser_close_counter() -> u64 {
    CLOSE_REQ.load(std::sync::atomic::Ordering::SeqCst)
}

// "Свернуть в трей" from outside the window loop: the own window hides itself (jarvis-app sets the hook; it asks the
// browser window to close when there is no own window); without the hook only the browser window exists
static WINDOW_HIDE: once_cell::sync::OnceCell<Box<dyn Fn() -> bool + Send + Sync>> = once_cell::sync::OnceCell::new();

pub fn set_window_hide(hide: Box<dyn Fn() -> bool + Send + Sync>) {
    let _ = WINDOW_HIDE.set(hide);
}

pub fn hide_window() {
    if let Some(hide) = WINDOW_HIDE.get() {
        if hide() {
            return;
        }
    }
    request_browser_close();
}

static LAST_STATE_POLL: parking_lot::Mutex<Option<std::time::Instant>> = parking_lot::Mutex::new(None);
// a page of Jarvis asked for its state in the last seconds: a browser window is open (it polls every 1.5 s, every
// 4 s when hidden), so "show the window" must not start one more
// (only a page that is ON THE SCREEN counts: a minimized window or a background tab would make "Открыть" show
// nothing, so a new window opens then)
pub fn browser_page_alive() -> bool {
    LAST_VISIBLE_POLL.lock().map(|t| t.elapsed() < Duration::from_secs(5)).unwrap_or(false)
}

static LAST_VISIBLE_POLL: parking_lot::Mutex<Option<std::time::Instant>> = parking_lot::Mutex::new(None);

static TRAY_NOTICE_SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
const RELOAD_GRACE: Duration = Duration::from_secs(4);

fn browser_window_closed() {
    let closed_at = std::time::Instant::now();
    std::thread::spawn(move || {
        std::thread::sleep(RELOAD_GRACE);
        let polled_again = LAST_STATE_POLL.lock().map(|t| t > closed_at).unwrap_or(false);
        if !closed_window_notice_due(polled_again, &TRAY_NOTICE_SAID) {
            return;
        }
        info!("UI: the browser window was closed, Jarvis keeps running in the tray");
        crate::notify::push("system", "JARVIS", &crate::i18n::t("window-closed-tray"), false);
    });
}

// said only when no page came back (not a reload) and only the first time in this run of Jarvis
fn closed_window_notice_due(polled_again: bool, said: &std::sync::atomic::AtomicBool) -> bool {
    !polled_again && !said.swap(true, std::sync::atomic::Ordering::SeqCst)
}

// Opens the Jarvis window. Returns true when a window was shown or a browser window was started.
pub fn open_window() -> bool {
    if let Some(show) = WINDOW_HOST.get() {
        if show() {
            return true;
        }
    }
    open_browser_window()
}

// The window in Edge / Chrome (app mode): used when the own window of Jarvis is not available.
pub fn open_browser_window() -> bool {
    let link = url();
    #[cfg(target_os = "windows")]
    {
        let app = format!("--app={}", link);
        // Edge ships with Windows; Chrome as a second choice (both give a window without address bar)
        for browser in ["msedge", "chrome"] {
            if try_start(&["/C", "start", "", browser, &app, "--window-size=1040,740"]) {
                info!("UI: window opened in {}", browser);
                return true;
            }
        }
        // last resort: default browser in a normal tab
        let mut rd = Command::new("rundll32");
        rd.args(["url.dll,FileProtocolHandler", &link]);
        crate::null_stdio(&mut rd);
        let ok = rd.spawn().is_ok();
        if ok {
            info!("UI: opened in the default browser");
        }
        ok
    }
    #[cfg(not(target_os = "windows"))]
    {
        info!("UI: open {} in a browser", link);
        let _ = Command::new("xdg-open").arg(&link).spawn();
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_closed_browser_window_is_told_once_and_not_after_a_reload() {
        let said = std::sync::atomic::AtomicBool::new(false);
        assert!(!closed_window_notice_due(true, &said), "a reload says nothing");
        assert!(closed_window_notice_due(false, &said), "the first real close is told");
        assert!(!closed_window_notice_due(false, &said), "never twice in one run");
        let before = browser_close_counter();
        request_browser_close();
        assert!(browser_close_counter() > before);
    }

    #[test]
    fn the_answers_of_the_close_dialog_and_the_window_host() {
        assert_eq!(close_action("tray"), Some(CloseAction::Hide));
        assert_eq!(close_action("exit"), Some(CloseAction::Exit));
        assert_eq!(close_action("cancel"), Some(CloseAction::Stay));
        assert_eq!(close_action("ack"), Some(CloseAction::Ack));
        assert_eq!(close_action("none"), None, "an unknown answer never exits");
        assert_eq!(close_action(""), None);
    }

    #[test]
    fn the_same_message_from_the_window_is_handled_once() {
        // a retry or a double press sends the same request id: the second one never reaches the router again
        let rid = format!("w-test-{}", rand::random::<u32>());
        let first = chat_action(&json!({ "op": "ask", "text": "сколько времени", "rid": rid }));
        assert!(first.get("repeat").is_none(), "the first one is handled: {}", first);
        let second = chat_action(&json!({ "op": "ask", "text": "сколько времени", "rid": rid }));
        assert_eq!(second["repeat"], true, "the second one is recognized as the same message: {}", second);
    }

    #[test]
    fn history_and_settings_are_never_deleted_without_the_confirmation() {
        let _g = crate::uistate::TEST_LOCK.lock();
        let store = crate::ai::store();
        let chat = store.create();
        store.append(&chat.id, "user", "не удалять");
        crate::uistate::set_for_tests(crate::uistate::UiSettings { dnd: true, ..Default::default() });
        let before = store.stats();
        for body in [json!({ "name": "history_delete_all" }), json!({ "name": "history_delete_all", "confirm": "yes" }), json!({ "name": "settings_reset" }), json!({ "name": "settings_reset", "confirm": "delete" })] {
            assert_eq!(run_action(&body)["ok"], false, "{} without its confirmation is refused", body);
        }
        // other tests may add messages meanwhile; nothing may disappear
        assert!(store.stats()["chats"].as_u64() >= before["chats"].as_u64(), "nothing was deleted");
        assert!(store.stats()["messages"].as_u64() >= before["messages"].as_u64());
        assert_eq!(store.load(&chat.id).unwrap().messages[0].content, "не удалять");
        assert!(crate::uistate::settings().dnd, "nothing was reset");
        crate::uistate::set_for_tests(Default::default());
    }

    // the window server: every request is in the door of the window, a "source" field of the request changes nothing
    #[test]
    fn a_window_phrase_enters_through_the_window_door_and_ignores_a_source_field() {
        let cap = crate::remote::Capture::start();
        let r = chat_action(&json!({ "op": "ask", "text": "\u{1}открой блокнот", "rid": "door-test-1", "source": "voice", "door": "internal", "internal": true }));
        assert_eq!(r["ok"], true, "{}", r);
        let got = cap.taken();
        assert_eq!(got.len(), 1, "{:?}", got);
        assert_eq!(got[0].source, crate::origin::Source::Window);
        assert!(!got[0].via_script);
        assert_eq!(got[0].text, "открой блокнот");
    }

}
