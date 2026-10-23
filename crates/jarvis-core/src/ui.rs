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
const PAGE: &str = include_str!(concat!(env!("OUT_DIR"), "/ui_page.html"));

// the page of the window, made by build.rs: Jarvis User gets it WITHOUT the license manager (cut out at build time)
fn page() -> std::borrow::Cow<'static, str> {
    std::borrow::Cow::Borrowed(PAGE)
}
const ICON: &[u8] = include_bytes!("../../../resources/icons/128x128.png");

// the address of the page (no secret: for the log and for people)
pub fn base_url() -> String {
    format!("http://127.0.0.1:{}/", PORT)
}

// the address that OPENS the window: the secret after the # (ui_auth.rs)
pub fn url() -> String {
    crate::ui_auth::with_token(&base_url())
}

pub fn start() {
    // the secret of the window is made before the first request is answered; without it every request is refused
    crate::ui_auth::init();
    let listener = match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(l) => l,
        Err(e) => {
            warn!("UI: cannot listen on port {}: {} (is another Jarvis running?)", PORT, e);
            return;
        }
    };
    info!("UI: window page on {}", base_url());
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
                    // «нужно подтверждение»: a command of a high risk asks «да / нет» before it runs; the level says who may run it
                    "risk": format!("{:?}", crate::commands::effective_risk(&pack.path, cmd)).to_lowercase(),
                    "level": format!("{:?}", crate::access::level_of(&cmd.id)).to_lowercase(),
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

// the license window: activation, renewal, the tariffs of the renewal window. Everything is decided by the server; the program only
// shows its answer. (The administrator's license manager is a separate set of operations of the Admin edition.)
fn license_action(body: &serde_json::Value) -> serde_json::Value {
    let op = body.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let key = body.get("key").and_then(|v| v.as_str()).unwrap_or("");
    let done = |r: Result<serde_json::Value, String>| match r {
        Ok(status) => json!({ "ok": true, "status": status }),
        Err(message) => json!({ "ok": false, "message": message }),
    };
    match op {
        "status" => json!({ "ok": true, "status": crate::licensing::status() }),
        "activate" => done(crate::licensing::activate(key)),
        "renew" => done(crate::licensing::renew(key)),
        "refresh" => done(crate::licensing::refresh().map(|_| crate::licensing::status())),
        "tariffs" => done(crate::licensing::tariffs()),
        _ => json!({ "ok": false, "message": "неизвестная операция" }),
    }
}

// the buttons of the window that do what a paid command does
fn paid_op_id(name: &str) -> Option<&'static str> {
    Some(match name {
        "app_launch" | "ess_launch" => "ui_app_launch",
        "file_search" => "ui_file_search",
        "file_open" | "file_reveal" => "ui_file_open",
        "remote_tunnel" => "ui_remote_tunnel",
        "screen_check" => "ui_screen_check",
        // the library of games (adding, choosing a file, scanning, starting, a new path, the label and the sound): a paid part, like the mode
        "game_add" | "game_add_flagged" | "game_pick_file" | "game_launch" | "game_set_path" | "game_scan" | "game_opts" | "aim_pick_model" | "aim_model_install" | "aim_model_check" | "aim_settings" | "aim_snapshot" | "bind_start" | "bind_confirm" | "bind_manual" | "bind_reset" | "bind_verify" => "ui_game_library",
        _ => return None,
    })
}

fn premium_reason(d: &crate::access::Denied) -> &'static str {
    match d.why {
        crate::access::Why::NeedsPremium => "premium",
        crate::access::Why::Expired => "expired",
        crate::access::Why::NeedsRefresh => "refresh",
        crate::access::Why::AdminOnly => "admin",
        crate::access::Why::PolicyBroken => "policy",
    }
}

static CODE_PROPOSAL: parking_lot::Mutex<Option<crate::codeassist::Proposal>> = parking_lot::const_mutex(None);

fn run_action(body: &serde_json::Value) -> serde_json::Value {
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
    // the paid operations of the window (not commands, so they have their own ids in the access list): checked here, before anything runs
    if let Some(id) = paid_op_id(name) {
        if let Err(d) = crate::access::check(id) {
            return json!({ "ok": false, "message": d.message, "premium": premium_reason(&d) });
        }
    }
    // with a password of the owner set, the actions that change what Jarvis keeps or does need the unlocked program
    if matches!(name, "mem_delete" | "mem_edit" | "ct_save" | "ct_delete" | "out_confirm" | "out_clear" | "sched_add" | "sched_remove" | "sched_toggle" | "kb_add" | "kb_remove" | "kb_clear" | "kb_reindex" | "prog_install" | "undo_do" | "notes_delete" | "notes_edit" | "home_config" | "home_forget" | "home_act" | "perms_set" | "owner_message" | "audit_clear" | "ca_apply" | "pl_add" | "pl_remove" | "persona_set" | "pair_start" | "pair_revoke" | "pair_link" | "remote_http_set" | "remote_key_rotate" | "comp_do" | "sync_folder" | "sync_run" | "sync_clear" | "wol_add" | "wol_remove" | "wol_wake" | "mem_use_ai" | "diag_fix_apply") {
        if let Err(m) = crate::owner::guard() {
            return json!({ "ok": false, "locked": true, "code": "locked", "message": m });
        }
    }
    match name {
        // the «Стоп» button of the work with an object: ends the dictation, cancels a running generation (its late result
        // is dropped), keeps the object. The answer says what really happened.
        "object_stop" => {
            let (reply, paused) = crate::objects::with(|e| {
                let rep = e.pause();
                (crate::objects::stop_reply(&rep, false), rep.paused)
            });
            if paused && matches!(crate::session::kind().as_deref(), Some("object") | Some("dictation") | Some("object_resume")) {
                crate::session::end();
            }
            json!({ "ok": paused, "message": reply })
        }
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
        // the key of the aim of ONE game: the wizard, the confirmation, the key typed by hand, the check, the reset
        "bind_status" => crate::aimbind::status_json(body.get("exe").and_then(|v| v.as_str()).unwrap_or("")),
        "bind_start" => match crate::aimbind::start(body.get("exe").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true, "message": "Перейдите в игру и несколько раз нажмите и подержите свою кнопку прицеливания." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "bind_cancel" => {
            crate::aimbind::cancel();
            json!({ "ok": true, "message": "Определение остановлено, ничего не сохранено." })
        }
        "bind_verify" => match crate::aimbind::verify(body.get("exe").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true, "message": "Перейдите в игру и нажмите сохранённую кнопку." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "bind_confirm" => {
            let exe = body.get("exe").and_then(|v| v.as_str()).unwrap_or("");
            let mode = body.get("mode").and_then(|v| v.as_str()).unwrap_or("hold");
            match crate::aimbind::confirm(exe, body.get("key").and_then(|v| v.as_str()), mode) {
                Ok(k) => json!({ "ok": true, "message": format!("Сохранено для игры {}: {}. Другие игры не затронуты. Помощь прицела по-прежнему включается отдельно.", exe, crate::aimbind::key_label(&k)) }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "bind_manual" => {
            let exe = body.get("exe").and_then(|v| v.as_str()).unwrap_or("");
            let mode = body.get("mode").and_then(|v| v.as_str()).unwrap_or("hold");
            match crate::aimbind::manual(exe, body.get("key").and_then(|v| v.as_str()).unwrap_or(""), mode) {
                Ok(()) => json!({ "ok": true, "message": "Изменение клавиши применяется только к профилю этой игры." }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "bind_reset" => match crate::aimbind::reset(body.get("exe").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true, "message": "Клавиша прицеливания этой игры сброшена. Игра осталась в библиотеке, другие игры не затронуты." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // what is around: the devices of the local network and the Bluetooth devices Windows knows (read only)
        "devices_lan" => match crate::devices::lan_devices() {
            Ok(v) => json!({ "ok": true, "items": v.into_iter().map(|(ip, mac)| json!([ip, mac])).collect::<Vec<_>>() }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "devices_bt" => match crate::devices::bluetooth_devices() {
            Ok(v) => json!({ "ok": true, "items": v.into_iter().map(|(n, st)| json!([n, st])).collect::<Vec<_>>() }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // asks Windows to redraw the icons of the shortcuts (the old picture stays in the cache of the system)
        "icons_refresh" => {
            #[cfg(target_os = "windows")]
            let ok = {
                let mut c = std::process::Command::new("ie4uinit.exe");
                c.arg("-show");
                crate::null_stdio(&mut c);
                c.spawn().is_ok()
            };
            #[cfg(not(target_os = "windows"))]
            let ok = false;
            json!({ "ok": ok, "message": if ok { "Windows просят обновить значки. Если старые остались: удалите ярлык и создайте заново, а при необходимости удалите файлы IconCache.db и iconcache_*.db в %localappdata% и перезапустите проводник." } else { "Доступно только в Windows" } })
        }
        // generation of pictures in the window
        "img_start" => match crate::imagegen::start_job(body.get("prompt").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "img_status" => {
            let j = crate::imagegen::job();
            let preview = if j.state == "done" { crate::imagegen::preview_b64(&j.path) } else { None };
            json!({ "ok": true, "job": j, "preview": preview, "model": crate::imagegen::model_for(&crate::uistate::settings().ai_provider), "provider": crate::uistate::settings().ai_provider })
        }
        "img_cancel" => json!({ "ok": crate::imagegen::cancel_job() }),
        "img_model" => match crate::imagegen::set_model(body.get("value").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // sync of the memory, contacts and notes through a folder; Wake-on-LAN
        "sync_get" => json!({ "ok": true, "status": crate::wsync::status_json() }),
        "sync_folder" => match crate::wsync::set_folder(body.get("path").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true, "message": "Папка выбрана. Нажмите «Синхронизировать»." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "sync_clear" => json!({ "ok": crate::wsync::clear_folder() }),
        "sync_run" => match crate::wsync::sync_now() {
            Ok(r) => json!({ "ok": true, "report": r, "message": format!("Готово: добавлено {}, обновлено {}, удалено {}, конфликтов {} (новая версия победила). Всего записей: {}.", r.added, r.updated, r.deleted, r.conflicts, r.items_total) }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "wol_list" => json!({ "ok": true, "items": crate::wol::list() }),
        "wol_add" => match crate::wol::add(body.get("name").and_then(|v| v.as_str()).unwrap_or(""), body.get("mac").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(_) => json!({ "ok": true, "message": "Добавлено." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "wol_remove" => json!({ "ok": crate::wol::remove(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) }),
        "wol_wake" => match crate::wol::wake(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(m) => json!({ "ok": true, "message": m }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "mem_use_ai" => match crate::memory::set_use_in_ai(body.get("value").and_then(|v| v.as_bool()).unwrap_or(true)) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // components (voices, the model of the aim assist)
        "comp_list" => json!({ "ok": true, "items": crate::components::json() }),
        "comp_do" => match crate::components::run(body.get("id").and_then(|v| v.as_str()).unwrap_or(""), body.get("op").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(m) => json!({ "ok": true, "message": m }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // pairing of phones with a one-time code
        "pair_start" => {
            if !crate::remote::is_running() {
                return json!({ "ok": false, "message": "Сначала включите пульт (переключатель выше): без него телефону некуда подключаться." });
            }
            let code = crate::pairing::start_code(crate::store::now());
            json!({ "ok": true, "code": code })
        }
        // «Копировать ссылку привязки»: the same link as the QR code (the code, while one is shown, stays after «#»; it is not in the request)
        "pair_link" => {
            let ip = body.get("ip").and_then(|v| v.as_str()).map(|s| s.to_string());
            match pairing_link_for(ip.as_deref()) {
                Ok(link) => {
                    crate::audit::record("remote", "pair_link_copy", "medium", "allowed", false, "ссылка привязки скопирована в окне");
                    json!({ "ok": true, "link": link, "has_code": crate::pairing::active_code(crate::store::now()).is_some() })
                }
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        // plain http in the home network: OFF by default; switching it on needs the owner and an explicit confirmation
        "remote_http_set" => {
            let on = body.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
            if on && body.get("confirm").and_then(|v| v.as_str()) != Some("http") {
                return json!({ "ok": false, "message": "Нужно подтверждение: обычный http передаёт секреты открытым текстом." });
            }
            match crate::remote::set_allow_http(on) {
                Ok(()) => json!({ "ok": true, "allow_http": on }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        // a NEW key of the PC: every phone must be paired again and the fingerprint checked again
        "remote_key_rotate" => {
            if body.get("confirm").and_then(|v| v.as_str()) != Some("rotate") {
                return json!({ "ok": false, "message": "Нужно подтверждение: после смены ключа все телефоны придётся привязать заново." });
            }
            match crate::remote::rotate_tls_key() {
                Ok(fp) => json!({ "ok": true, "fingerprint": fp }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "pair_list" => {
            let now = crate::store::now();
            let (code, left) = crate::pairing::active_code(now).map(|(c, l)| (Some(c), l)).unwrap_or((None, 0));
            json!({ "ok": true, "items": crate::pairing::list(), "code": code, "left": left })
        }
        "pair_revoke" => json!({ "ok": crate::pairing::revoke(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) }),
        // character of the AI (humor, language, manner)
        "persona_get" => { let p = crate::persona::get(); json!({ "ok": true, "humor": p.humor, "reply_lang": p.reply_lang, "natural": p.natural }) }
        "persona_set" => match crate::persona::set(body.get("humor").and_then(|v| v.as_u64()).unwrap_or(1) as u8, body.get("reply_lang").and_then(|v| v.as_str()).unwrap_or("auto"), body.get("natural").and_then(|v| v.as_bool()).unwrap_or(true)) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // the programmer's assistant: a proposal (diff), then apply or discard
        "ca_propose" => match crate::codeassist::propose(body.get("path").and_then(|v| v.as_str()).unwrap_or(""), body.get("text").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(p) => {
                let (add, del) = crate::codeassist::changed(&p.diff);
                let (checked, ok, what) = crate::codeassist::quick_check(&p.path, &p.new);
                let out = json!({ "ok": true, "diff": crate::codeassist::render(&p.diff), "added": add, "removed": del, "checked": checked, "check_ok": ok, "check": what, "path": p.path.to_string_lossy() });
                *CODE_PROPOSAL.lock() = Some(p);
                out
            }
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "ca_apply" => {
            let p = CODE_PROPOSAL.lock().take();
            match p {
                None => json!({ "ok": false, "message": "Нет предложенной правки: сделайте предложение." }),
                Some(p) => match crate::codeassist::apply(&p) {
                    Ok(bak) => {
                        crate::actionlog::record(&format!("Правка файла {}", p.path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()), crate::actionlog::Undo::FileRestore(p.path.to_string_lossy().into(), bak.to_string_lossy().into()));
                        json!({ "ok": true, "message": format!("Применено. Копия старого файла: {}. Вернуть можно в «Истории действий».", bak.display()) })
                    }
                    Err(e) => json!({ "ok": false, "message": e }),
                },
            }
        }
        "ca_discard" => { *CODE_PROPOSAL.lock() = None; json!({ "ok": true }) }
        // library of programs
        "pl_get" => json!({ "ok": true, "lib": crate::proglib::json() }),
        "pl_pick" => match crate::gamedialog::pick_exe() {
            Ok(None) => json!({ "ok": true, "cancelled": true }),
            Ok(Some(path)) => match crate::proglib::check_path(&path) {
                Ok(name) => json!({ "ok": true, "path": path, "name": name }),
                Err(e) => json!({ "ok": false, "message": e }),
            },
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "pl_add" => match crate::proglib::add_shortcut(body.get("name").and_then(|v| v.as_str()).unwrap_or(""), body.get("path").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(_) => json!({ "ok": true, "message": "Ярлык добавлен." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "pl_remove" => json!({ "ok": crate::proglib::remove_shortcut(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) }),
        "pl_hide" => json!({ "ok": crate::proglib::hide(body.get("name").and_then(|v| v.as_str()).unwrap_or("")) }),
        "pl_unhide" => json!({ "ok": crate::proglib::unhide_all() }),
        // «Проверить имя»: a few seconds of listening that only shows what the wake-word recognizer heard
        "hist_get" => {
            let door = match body.get("door").and_then(|v| v.as_str()) { Some("pc") => "pc", Some("phone") => "phone", _ => "all" };
            json!({ "ok": true, "h": crate::devhist::json(door, 60) })
        }
        "hist_clear" => {
            let door = match body.get("door").and_then(|v| v.as_str()) { Some("pc") => "pc", Some("phone") => "phone", _ => "all" };
            json!({ "ok": true, "removed": crate::devhist::clear(door) })
        }
        // the journal of critical actions (audit.rs): the window reads it; clearing needs the unlocked program
        "audit_get" => {
            let g = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").chars().take(60).collect::<String>();
            let within = body.get("within").and_then(|v| v.as_u64()).unwrap_or(0);
            json!({ "ok": true, "log": crate::audit::json_filtered(&g("q"), &g("door"), &g("risk"), &g("decision"), within, body.get("op").and_then(|v| v.as_u64()).unwrap_or(0)) })
        }
        "audit_clear" => json!({ "ok": true, "removed": crate::audit::clear() }),
        "name_test_start" => { crate::nametest::start(); json!({ "ok": true, "status": crate::nametest::status_json() }) }
        "name_test_stop" => { crate::nametest::stop(); json!({ "ok": true, "status": crate::nametest::status_json() }) }
        "name_test_status" => json!({ "ok": true, "status": crate::nametest::status_json() }),
        "ess_launch" => match crate::proglib::launch_essential(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(m) => json!({ "ok": true, "message": m }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "pl_launch" => match crate::proglib::launch_shortcut(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(m) => json!({ "ok": true, "message": m }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // reading of messages, microphone watchdog, game statistics, screen region
        "sms_get" => { let c = crate::smsread::conf(); json!({ "ok": true, "privacy": c.privacy, "lang": c.lang }) }
        "sms_set" => {
            let privacy = match body.get("privacy").and_then(|v| v.as_str()).unwrap_or("full") { "sender" => crate::smsread::Privacy::Sender, "silent" => crate::smsread::Privacy::Silent, _ => crate::smsread::Privacy::Full };
            match crate::smsread::set(privacy, body.get("lang").and_then(|v| v.as_str()).unwrap_or("auto")) {
                Ok(()) => json!({ "ok": true }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "sms_repeat" => { let m = crate::smsread::repeat_last(); if crate::tts::is_enabled() { crate::tts::speak(&m); } json!({ "ok": true, "message": m }) }
        "mic_watch" => json!({ "ok": true, "watch": crate::wakewatch::status_json(), "diag": crate::voicediag::json_report() }),
        "gs_summary" => json!({ "ok": true, "stats": crate::gamestats::summary_json() }),
        "gs_report" => json!({ "ok": true, "text": crate::gamestats::report_text() }),
        "gs_clear" => json!({ "ok": crate::gamestats::clear() }),
        // a picture of the screen for the page: the user chooses a region there, and only that region is sent to the AI as a picture of the chat
        "screen_grab" => {
            if let Err(e) = crate::perms::check(crate::perms::Cap::ScreenCapture) {
                return json!({ "ok": false, "message": e });
            }
            if !crate::uistate::settings().ai_screen {
                return json!({ "ok": false, "message": "Анализ экрана выключен: включите его в Настройки → ИИ. Пока он выключен, экран не снимается." });
            }
            match crate::screen::capture() {
                Ok(jpeg) => json!({ "ok": true, "jpeg": crate::docai::b64_encode(&jpeg) }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        // permissions centre, owner password
        "perms_list" => json!({ "ok": true, "items": crate::perms::json() }),
        "perms_set" => match crate::perms::Cap::from_id(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Some(c) => match crate::perms::set(c, body.get("value").and_then(|v| v.as_bool()).unwrap_or(false)) {
                Ok(()) => json!({ "ok": true }),
                Err(e) => json!({ "ok": false, "message": e }),
            },
            None => json!({ "ok": false, "message": "Неизвестное разрешение." }),
        },
        "owner_status" => json!({ "ok": true, "set": crate::owner::is_set(), "unlocked": crate::owner::unlocked(), "message": crate::owner::message() }),
        "owner_set" => match crate::owner::set_password(body.get("new").and_then(|v| v.as_str()).unwrap_or(""), body.get("old").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(code) => json!({ "ok": true, "recovery": code, "message": "Пароль сохранён. Запишите код восстановления: он показан один раз." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "owner_unlock" => match crate::owner::unlock(body.get("password").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true, "message": "Разблокировано на 15 минут." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "owner_lock" => { crate::owner::lock(); json!({ "ok": true }) }
        "owner_remove" => match crate::owner::remove_password(body.get("password").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true, "message": "Пароль снят." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "owner_recover" => match crate::owner::recover(body.get("code").and_then(|v| v.as_str()).unwrap_or(""), body.get("new").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(code) => json!({ "ok": true, "recovery": code, "message": "Пароль заменён. Новый код восстановления показан один раз." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "owner_message" => match crate::owner::set_message(body.get("value").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // scheduler
        "sched_list" => {
            let off = chrono::Local::now().offset().local_minus_utc() as i64;
            json!({ "ok": true, "items": crate::scheduler::list().iter().map(|j| json!({ "id": j.id, "text": j.text, "enabled": j.enabled, "runs": j.runs, "line": crate::scheduler::describe(j, off) })).collect::<Vec<_>>() })
        }
        "sched_add" => {
            let l = chrono::Local::now();
            match crate::scheduler::parse(body.get("text").and_then(|v| v.as_str()).unwrap_or(""), l.naive_local(), l.offset().local_minus_utc() as i64) {
                Some(p) => match crate::perms::check(crate::perms::Cap::Scheduler).and_then(|_| crate::scheduler::add(&p.text, p.at, p.repeat)) {
                    Ok(j) => { crate::actionlog::record(&format!("Задание: {}", p.text), crate::actionlog::Undo::JobAdd(j.id)); json!({ "ok": true, "message": "Поставил в расписание." }) }
                    Err(e) => json!({ "ok": false, "message": e }),
                },
                None => json!({ "ok": false, "message": "Не понял время. Примеры: «каждый день в 9:00 открой почту», «завтра в 8:30 напомни про встречу», «каждые 30 минут пей воду»." }),
            }
        }
        "sched_remove" => json!({ "ok": crate::scheduler::remove(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) }),
        "sched_toggle" => json!({ "ok": crate::scheduler::set_enabled(body.get("id").and_then(|v| v.as_str()).unwrap_or(""), body.get("value").and_then(|v| v.as_bool()).unwrap_or(false)) }),
        // history of actions and undo
        "undo_list" => json!({ "ok": true, "items": crate::actionlog::list().iter().rev().take(50).map(|e| json!({ "id": e.id, "at": e.at, "what": e.what, "undone": e.undone, "can_undo": crate::actionlog::can_undo(e) })).collect::<Vec<_>>() }),
        "undo_do" => match crate::actionlog::undo(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(m) => json!({ "ok": true, "message": m }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // notes and diary
        "notes_list" => { let mut n = crate::notes::list(); n.reverse(); json!({ "ok": true, "items": n.into_iter().take(200).collect::<Vec<_>>() }) }
        "notes_delete" => {
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let note = crate::notes::get(id);
            let done = crate::notes::delete(id);
            match (done, note) {
                (true, Some(n)) => json!({ "ok": true, "message": "Удалено.", "undo": crate::actionlog::record("Удалена запись дневника", crate::actionlog::Undo::NoteRestore(n)) }),
                _ => json!({ "ok": done }),
            }
        }
        "notes_edit" => {
            let (id, text) = (body.get("id").and_then(|v| v.as_str()).unwrap_or(""), body.get("text").and_then(|v| v.as_str()).unwrap_or(""));
            match crate::notes::update(id, text) {
                Ok(old) => {
                    let now = crate::notes::get(id).map(|n| n.text).unwrap_or_default();
                    let undo = crate::actionlog::record("Правка записи дневника", crate::actionlog::Undo::NoteRevert(id.to_string(), old, now));
                    json!({ "ok": true, "message": "Сохранено.", "undo": undo })
                }
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "notes_export" => json!({ "ok": true, "text": crate::notes::export_text() }),
        // smart home
        "home_status" => json!({ "ok": true, "status": crate::smarthome::status_json() }),
        "home_config" => match crate::smarthome::configure(body.get("url").and_then(|v| v.as_str()).unwrap_or(""), body.get("token").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true, "message": "Сохранено. Включите разрешение «Управлять умным домом» в центре разрешений." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "home_forget" => json!({ "ok": crate::smarthome::forget() }),
        "home_devices" => match crate::smarthome::devices() {
            Ok(d) => json!({ "ok": true, "items": d }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "home_act" => {
            let id = body.get("entity_id").and_then(|v| v.as_str()).unwrap_or("");
            match crate::smarthome::devices().and_then(|l| l.into_iter().find(|d| d.entity_id == id).ok_or_else(|| "Такого устройства нет в Home Assistant.".to_string())) {
                Ok(d) => match crate::smarthome::act(&d, if body.get("value").and_then(|v| v.as_bool()).unwrap_or(false) { crate::smarthome::Act::On } else { crate::smarthome::Act::Off }) {
                    Ok(()) => json!({ "ok": true, "message": "Сервер принял команду." }),
                    Err(e) => json!({ "ok": false, "message": e }),
                },
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        // knowledge base
        "kb_status" => { let i = crate::kb::load(); json!({ "ok": true, "folders": i.folders, "files": i.docs.len(), "passages": i.docs.iter().map(|d| d.chunks.len()).sum::<usize>(), "updated": i.updated }) }
        "kb_add" => match crate::kb::add_folder(body.get("path").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "kb_remove" => json!({ "ok": crate::kb::remove_folder(body.get("path").and_then(|v| v.as_str()).unwrap_or("")) }),
        "kb_clear" => json!({ "ok": crate::kb::clear() }),
        "kb_reindex" => match crate::kb::reindex() {
            Ok((f, p)) => json!({ "ok": true, "message": format!("Прочитано файлов: {}, отрывков: {}.", f, p) }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "kb_search" => json!({ "ok": true, "items": crate::kb::search(body.get("q").and_then(|v| v.as_str()).unwrap_or(""), 8) }),
        // catalogue of programs (winget)
        "prog_search" => match crate::progsearch::search(body.get("q").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(l) => json!({ "ok": true, "items": l }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "prog_install" => {
            if let Err(e) = crate::perms::check(crate::perms::Cap::Install) {
                return json!({ "ok": false, "message": e });
            }
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !crate::progsearch::valid_id(&id) {
                return json!({ "ok": false, "message": "Неверный идентификатор программы." });
            }
            if !crate::progsearch::begin(&id) {
                return json!({ "ok": false, "message": "Уже идёт другая установка." });
            }
            let p = crate::progsearch::Package { name: body.get("name").and_then(|v| v.as_str()).unwrap_or(&id).chars().take(80).collect(), id, version: String::new(), source: "winget".into() };
            std::thread::spawn(move || { let _ = crate::progsearch::install(&p); });
            json!({ "ok": true, "message": "Запускаю установку. Окно установщика покажется само: молча ничего не ставится." })
        }
        "prog_status" => json!({ "ok": true, "status": crate::progsearch::status() }),
        "prog_cancel" => json!({ "ok": crate::progsearch::cancel() }),
        "prog_history" => { let mut h = crate::progsearch::history(); h.reverse(); json!({ "ok": true, "items": h.into_iter().take(50).collect::<Vec<_>>() }) }
        // memory, contacts and the messenger (memory.rs, contacts.rs, messenger.rs)
        "mem_list" => { let m = crate::memory::load(); json!({ "ok": true, "items": m.items, "auto": m.auto_remember, "use_in_ai": m.use_in_ai }) }
        "mem_add" => match crate::memory::add_with(body.get("text").and_then(|v| v.as_str()).unwrap_or(""), "window", body.get("replace").and_then(|v| v.as_bool()).unwrap_or(false)) {
            Ok(crate::memory::Added::Added(_)) => json!({ "ok": true, "message": "Запомнил." }),
            Ok(crate::memory::Added::Duplicate(_)) => json!({ "ok": true, "message": "Это уже есть в памяти." }),
            Ok(crate::memory::Added::Conflict(id, old)) => json!({ "ok": false, "conflict": true, "id": id, "message": format!("Раньше было иначе: «{}». Заменить?", old) }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "mem_edit" => {
            let (id, text) = (body.get("id").and_then(|v| v.as_str()).unwrap_or(""), body.get("text").and_then(|v| v.as_str()).unwrap_or(""));
            match crate::memory::update(id, text) {
                Ok(old) => {
                    let now = crate::memory::get(id).map(|e| e.text).unwrap_or_default();
                    let undo = crate::actionlog::record("Правка записи памяти", crate::actionlog::Undo::MemoryRevert(id.to_string(), old, now));
                    json!({ "ok": true, "message": "Сохранено.", "undo": undo })
                }
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "mem_delete" => {
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let entry = crate::memory::get(id);
            let done = crate::memory::delete(id);
            match (done, entry) {
                (true, Some(e)) => json!({ "ok": true, "message": "Удалено.", "undo": crate::actionlog::record("Удалена запись памяти", crate::actionlog::Undo::MemoryRestore(e)) }),
                _ => json!({ "ok": done }),
            }
        }
        "mem_export" => json!({ "ok": true, "text": crate::memory::export_text() }),
        "mem_auto" => match crate::memory::set_auto(body.get("value").and_then(|v| v.as_bool()).unwrap_or(false)) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "ct_list" => json!({ "ok": true, "contacts": crate::contacts::list() }),
        "ct_save" => match serde_json::from_value::<crate::contacts::Contact>(body.get("contact").cloned().unwrap_or(json!({}))) {
            Ok(c) => match crate::contacts::save(c) {
                Ok(id) => json!({ "ok": true, "id": id, "message": "Контакт сохранён." }),
                Err(e) => json!({ "ok": false, "message": e }),
            },
            Err(_) => json!({ "ok": false, "message": "Неверные данные контакта." }),
        },
        "ct_delete" => json!({ "ok": crate::contacts::delete(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) }),
        "out_list" => { let mut h = crate::messenger::history(); h.reverse(); json!({ "ok": true, "items": h.into_iter().take(50).map(|o| { let label = o.status.label(); let mut v = serde_json::to_value(&o).unwrap_or(json!({})); v["label"] = json!(label); v }).collect::<Vec<_>>() }) }
        "out_confirm" => match crate::features::confirm_message(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(o) => json!({ "ok": o.status != crate::messenger::Status::Error, "status": o.status, "message": if o.status == crate::messenger::Status::ManualSendRequired { "Чат открыт с текстом: нажмите «Отправить» в приложении.".to_string() } else { o.note.clone() } }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "out_cancel" => match crate::features::cancel_message(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(_) => json!({ "ok": true, "message": "Отменено." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "out_clear" => json!({ "ok": crate::messenger::clear_history() }),
        // the backup of the settings in one file (without secrets) and back
        "backup_export" => json!({ "ok": true, "backup": crate::backup::export() }),
        "backup_import" => match crate::backup::import(body.get("backup").unwrap_or(&json!(null))) {
            Ok(n) => {
                crate::uistate::reload();
                json!({ "ok": true, "message": format!("Настройки восстановлены ({} файл.). Часть из них (список игр, макросы) применится после перезапуска Jarvis.", n) })
            }
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // Esc of the window: the voice stops at once (the phrase that is read and the ones waiting in the queue)
        "tts_stop" => {
            // asked before the stop: did the voice really speak (the page lets Esc do its other things when it did not)
            let was_speaking = crate::tts::is_speaking() || crate::tts::is_playing();
            if was_speaking {
                // (an AI answer still arriving stops too, or its next sentence would start the voice again)
                if let Some(id) = crate::ai::store().active() {
                    crate::ai::stop(&id);
                }
            }
            crate::tts::stop();
            json!({ "ok": true, "was_speaking": was_speaking })
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
        "diag_fix_preview" => match crate::diag::fix_preview(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Some(p) => json!({ "ok": true, "preview": p }),
            None => json!({ "ok": false, "message": "Такого исправления нет." }),
        },
        // nothing changes without "confirm": true, which the window sends only after the owner read the preview
        "diag_fix_apply" => crate::diag::fix_apply(body.get("id").and_then(|v| v.as_str()).unwrap_or(""), body.get("confirm").and_then(|v| v.as_bool()) == Some(true)),
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
        "cloud_tts_key_forget" => {
            crate::uistate::forget_cloud_tts_key();
            json!({ "ok": true })
        }
        "ai_search_key_forget" => {
            crate::uistate::forget_ai_search_key();
            json!({ "ok": true })
        }
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
        // "Проверить экран": the same way as the voice command «что на экране»; the picture goes to the provider only
        // because of this press, and only when the analysis of the screen is allowed in the settings
        "screen_check" => screen_check(),
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
                    Ok(()) => { crate::filesearch::note_launch(&a.name, true); crate::proglib::note_launch(&a.name, crate::store::now()); json!({ "ok": true, "message": format!("Запускаю {}", a.name) }) }
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
        // the game mode (gamemode.rs): the same phrases as by voice, on buttons
        "game_mode" => {
            let on = body.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
            if on {
                if let Err(d) = crate::access::check("game_mode") {
                    return json!({ "ok": false, "message": d.message });
                }
            }
            let msg = crate::gamemode::handle(if on { "включи игровой режим" } else { "выключи игровой режим" }).unwrap_or_default();
            json!({ "ok": true, "message": msg })
        }
        "game_aim" => {
            let on = body.get("value").and_then(|v| v.as_bool()).unwrap_or(false);
            if on {
                if let Err(d) = crate::access::check("game_mode") {
                    return json!({ "ok": false, "message": d.message });
                }
            }
            let msg = crate::gamemode::handle(if on { "включи помощь прицела" } else { "выключи помощь прицела" }).unwrap_or_else(|| "Сначала включите игровой режим.".into());
            json!({ "ok": true, "message": msg })
        }
        // the Games page (gamelist.rs, gamescan.rs, gamemacro.rs)
        "game_add" | "game_add_flagged" => {
            let raw = body.get("path").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            let genre_named = body.get("genre").and_then(|v| v.as_str()).and_then(crate::gamemode::Genre::from_name);
            let title = body.get("title").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            // a full path (the button «Обзор…» or typed): the file is checked; only a name «game.exe» is still taken (it cannot be started)
            let e = if raw.contains(['\\', '/']) || raw.contains(':') {
                match crate::gamelaunch::check_path(&raw) {
                    Ok(v) => crate::gamelist::Entry { path: v.path, verified: true, ..crate::gamelist::Entry::basic(&v.exe, if title.is_empty() { &v.name } else { &title }, genre_named.unwrap_or_default(), "manual") },
                    Err(why) => return json!({ "ok": false, "message": why }),
                }
            } else {
                let exe = crate::gamemode::exe_of_input(&raw);
                if !exe.ends_with(".exe") {
                    return json!({ "ok": false, "message": "Выберите файл игры кнопкой «Обзор…» или впишите путь, например D:\\Games\\Игра\\game.exe." });
                }
                crate::gamelist::Entry::basic(&exe, &title, genre_named.unwrap_or_default(), "manual")
            };
            // a game that is already there is not added twice: the page asks whether to renew its path and kind
            if let Some(old) = crate::gamelist::find_duplicate(&e) {
                if body.get("renew").and_then(|v| v.as_bool()) != Some(true) {
                    return json!({ "ok": false, "duplicate": true, "id": old.id, "name": old.name, "message": format!("«{}» уже есть в библиотеке.", old.name) });
                }
            }
            match crate::gamelist::add_entry(e.clone(), genre_named.is_some()) {
                Ok((r, id)) => {
                    crate::gameicon::forget(&id);
                    crate::gamemode::reload_list();
                    crate::gamemode::note_event(&format!("игра {} вручную: {}", if r == crate::gamelist::Added::New { "добавлена" } else { "обновлена" }, e.exe));
                    json!({ "ok": true, "id": id, "new": r == crate::gamelist::Added::New, "message": if r == crate::gamelist::Added::New { format!("Игра «{}» добавлена в библиотеку.", crate::gamelist::get(&id).map(|g| g.name).unwrap_or_default()) } else { "Запись игры обновлена.".to_string() } })
                }
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        // the system window «Открыть файл» of Windows; only a path comes back (the file is checked, not opened or started)
        "game_pick_file" => match crate::gamedialog::pick_exe() {
            Ok(None) => json!({ "ok": true, "cancelled": true }),
            Ok(Some(path)) => match crate::gamelaunch::check_path(&path) {
                Ok(v) => json!({ "ok": true, "path": v.path, "exe": v.exe, "name": v.name }),
                Err(why) => json!({ "ok": false, "path": path, "message": why }),
            },
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // «Запустить»: only the game the user pressed; the start does not wait for the end of the game
        "game_launch" => match crate::gamelaunch::launch(body.get("id").and_then(|v| v.as_str()).unwrap_or("")) {
            Ok(m) => json!({ "ok": true, "message": m }),
            Err(e) => json!({ "ok": false, "message": e, "relink": true }),
        },
        // «Удалить»: ONLY the entry of the library goes (and the picture that Jarvis kept for it). The files of the game are not touched.
        "game_remove" => {
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let ok = crate::gamelist::remove_id(id);
            if ok {
                crate::gameicon::forget(id);
                crate::gamemode::reload_list();
                crate::gamemode::note_event("игра убрана из библиотеки");
            }
            json!({ "ok": ok, "message": if ok { "Игра убрана из библиотеки Jarvis. Файлы игры остались на компьютере." } else { "Этой игры нет в библиотеке." } })
        }
        "game_genre" => {
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match body.get("genre").and_then(|v| v.as_str()).and_then(crate::gamemode::Genre::from_name) {
                Some(g) if crate::gamelist::set_genre_id(id, g) => {
                    crate::gamemode::reload_list();
                    json!({ "ok": true })
                }
                _ => json!({ "ok": false, "message": "Не удалось сменить жанр." }),
            }
        }
        // a new path for a game whose file was moved (the id stays)
        "game_set_path" => {
            let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("");
            match crate::gamelaunch::check_path(body.get("path").and_then(|v| v.as_str()).unwrap_or("")) {
                Ok(v) => match crate::gamelist::set_path(id, &v.exe, &v.path) {
                    Ok(()) => {
                        crate::gameicon::forget(id);
                        crate::gamemode::reload_list();
                        json!({ "ok": true, "message": "Путь к игре обновлён." })
                    }
                    Err(e) => json!({ "ok": false, "message": e }),
                },
                Err(why) => json!({ "ok": false, "message": why }),
            }
        }
        // the model of the aim assist (aimmodel.rs): the official download, a file of the user, the cancel, the check, the settings
        "aim_model_install" => match crate::aimmodel::start_install(&crate::gamemode::profile_aim()) {
            Ok(()) => json!({ "ok": true, "message": format!("Загрузка модели началась (около {} МБ).", crate::aimmodel::download_size_mb()) }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // the snapshot of the last frame with the marks: written to a file of the data folder (the user decides whom to show it)
        "aim_snapshot" => match crate::aimassist::snapshot_png() {
            Some(png) => {
                let dir = crate::APP_DIR.join("aim");
                let _ = std::fs::create_dir_all(&dir);
                let file = dir.join("last_aim.png");
                match crate::write_atomic(&file, &png) {
                    Ok(()) => json!({ "ok": true, "message": format!("{} Снимок сохранён: {}", crate::aimassist::snapshot_text(), file.display()), "saved": file.display().to_string() }),
                    Err(e) => json!({ "ok": false, "message": format!("снимок не сохранился: {}", e) }),
                }
            }
            None => json!({ "ok": false, "message": crate::aimassist::snapshot_text() }),
        },
        "aim_model_cancel" => json!({ "ok": true, "message": if crate::aimmodel::cancel() { "Отменяю…" } else { "Сейчас ничего не устанавливается." } }),
        "aim_pick_model" => match crate::gamedialog::pick_model() {
            Ok(None) => json!({ "ok": true, "cancelled": true }),
            Ok(Some(path)) => match crate::aimmodel::start_manual(std::path::PathBuf::from(path), &crate::gamemode::profile_aim()) {
                Ok(()) => json!({ "ok": true, "message": "Проверяю файл модели…" }),
                Err(e) => json!({ "ok": false, "message": e }),
            },
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "aim_model_check" => match crate::aimmodel::start_check(&crate::gamemode::aim_config()) {
            Ok(()) => json!({ "ok": true, "message": "Проверяю модель…" }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "aim_settings" if body.get("for_game").and_then(|v| v.as_bool()) == Some(true) => {
            // the crosshair is kept for the game that the aim assist saw last in front (the Jarvis window is in front now)
            let s = body.get("settings").unwrap_or(&json!(null)).clone();
            let game = crate::aimassist::last_game();
            let cross = match (s.get("cx").and_then(|v| v.as_i64()), s.get("cy").and_then(|v| v.as_i64())) {
                (Some(cx), Some(cy)) => crate::aimassist::save_game_cross(&game, cx, cy).map(|_| Some((cx, cy))),
                _ => Ok(None),
            };
            match cross.and_then(|c| crate::aimassist::save_game_settings(&game, &s).map(|n| (c, n))) {
                Ok((c, n)) if c.is_some() || n > 0 => json!({ "ok": true, "message": format!("Для игры {} запомнено: перекрестие {}, других чисел: {}. Они подставляются сами при её запуске.", game, c.map(|(x, y)| format!("({:+}%, {:+}%)", x, y)).unwrap_or_else(|| "не менялось".into()), n) }),
                Ok(_) => json!({ "ok": false, "message": "нет чисел для сохранения" }),
                Err(e) => json!({ "ok": false, "message": e }),
            }
        }
        "aim_settings" => match crate::aimassist::save_settings(body.get("settings").unwrap_or(&json!(null)), &crate::gamemode::profile_aim()) {
            Ok(_) => json!({ "ok": true, "message": "Настройки сохранены. Устройство и размер кадра применяются при следующем включении помощи прицела." }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        // «Мой голос» (voiceprint.rs): recording the voice, testing it, the lock on / off, the strictness, deleting
        "voice_enroll_start" | "voice_test_start" => {
            if crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst) {
                return json!({ "ok": false, "message": "Микрофон выключен: включите его, чтобы записать голос." });
            }
            if name == "voice_enroll_start" {
                crate::voiceprint::start_enrollment();
            } else {
                crate::voiceprint::start_test();
            }
            // listen at once, without the wake word (like a click on the orb)
            crate::LISTEN_NOW.store(true, std::sync::atomic::Ordering::SeqCst);
            json!({ "ok": true })
        }
        "voice_enroll_cancel" => {
            crate::voiceprint::cancel_enrollment();
            json!({ "ok": true })
        }
        "voice_print_delete" => {
            crate::voiceprint::delete();
            json!({ "ok": true })
        }
        "voice_print_enable" => match crate::voiceprint::set_enabled(body.get("value").and_then(|v| v.as_bool()).unwrap_or(false)) {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "voice_print_strict" => {
            crate::voiceprint::set_strictness(body.get("level").and_then(|v| v.as_u64()).unwrap_or(1).min(2) as u8);
            json!({ "ok": true })
        }
        "game_scan" => match crate::gamescan::start() {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "game_opts" => {
            let mut o = crate::gamelist::options();
            if let Some(b) = body.get("overlay").and_then(|v| v.as_bool()) {
                o.overlay = b;
            }
            if let Some(b) = body.get("sound").and_then(|v| v.as_bool()) {
                o.sound = b;
            }
            crate::gamelist::set_options(o);
            json!({ "ok": true })
        }
        "macro_delete" => {
            let ok = crate::gamemacro::delete(body.get("macro").and_then(|v| v.as_str()).unwrap_or(""));
            json!({ "ok": ok, "message": if ok { "Макрос удалён." } else { "Такого макроса нет." } })
        }
        // updates: check, download, install — each one on a button (see updater.rs)
        "update_check" => match crate::updater::check() {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "update_download" => match crate::updater::download() {
            Ok(()) => json!({ "ok": true }),
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "update_install" => match crate::updater::install() {
            Ok(()) => {
                // the installer closes the program itself; this is only the clean exit if it is still running
                std::thread::spawn(|| {
                    std::thread::sleep(Duration::from_secs(8));
                    match EXIT_HOOK.get() {
                        Some(exit) => exit(),
                        None => std::process::exit(0),
                    }
                });
                json!({ "ok": true })
            }
            Err(e) => json!({ "ok": false, "message": e }),
        },
        "voice_install" => match crate::voice_install::start(body.get("voice").and_then(|v| v.as_str())) {
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

// "check the screen": one picture of the main screen and one short question; the answer is shown, nothing is stored
fn screen_check() -> serde_json::Value {
    match crate::ai::ask_about_screen("Опиши одним коротким предложением, что сейчас показано на экране.") {
        Ok(answer) => json!({ "ok": true, "message": format!("Экран прочитан. Ответ ИИ: {}", answer.trim().chars().take(160).collect::<String>()) }),
        Err(e) => json!({ "ok": false, "message": e.user_text() }),
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
            if let Err(d) = crate::access::check("ui_chat_image") {
                return json!({ "ok": false, "message": d.message, "premium": premium_reason(&d) });
            }
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

// The pairing link of the window: https://<address>/#pin=<pin of the key of the PC>[&c=<code>] (remote::pair_link). The QR code and the button
// «Копировать ссылку привязки» both take it from here, so they always carry the same link. `ip` is only taken when it is an address of this PC.
// Everything after «#» stays on the phone; the link is made on demand and is never logged.
pub(crate) fn pairing_link_for(ip: Option<&str>) -> Result<String, String> {
    if !crate::remote::is_running() {
        return Err("Пульт выключен: включите его, чтобы получить ссылку привязки.".into());
    }
    let addrs = crate::remote::addresses();
    let ip = match ip {
        Some(ip) if addrs.iter().any(|a| a.ip == ip) => ip.to_string(),
        _ => addrs.first().map(|a| a.ip.clone()).unwrap_or_else(|| "localhost".to_string()),
    };
    crate::remote::pair_link(&ip).map_err(|e| format!("Ключ ПК недоступен, ссылку привязки сделать нельзя: {}", e))
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
    // the secret of the window (header X-Jarvis-Token, or jt= in the address for images)
    token: Option<String>,
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
    let mut token = crate::ui_auth::from_query(&query);
    let mut content_length = 0usize;
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            let (k, v) = (k.trim().to_lowercase(), v.trim());
            match k.as_str() {
                "host" => host_ok = v == format!("127.0.0.1:{}", PORT) || v == format!("localhost:{}", PORT),
                "content-length" => content_length = v.parse().unwrap_or(0),
                // a web page on another site cannot add this header without our permission
                "x-jarvis" => custom_header = v == "1",
                "x-jarvis-token" => token = Some(v.to_string()),
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
    Ok(Request { method, path, query, host_ok, custom_header, token, body: buf[header_end..end].to_vec() })
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

// a picture that may stay in the cache of the window for an hour (the icons of the games: the address changes when the path does)
fn respond_bytes_cached(stream: &mut TcpStream, content_type: &str, body: &[u8]) {
    let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: private, max-age=3600\r\nConnection: close\r\n\r\n", content_type, body.len());
    let _ = stream.write_all(body);
}

// «a=1&b=текст%20с%20пробелами» -> the value of one key, decoded ("" when there is none)
fn query_param(query: &str, key: &str) -> String {
    let raw = query.split('&').find_map(|kv| kv.split_once('=').filter(|(k, _)| *k == key).map(|(_, v)| v)).unwrap_or("");
    let bytes = raw.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() + 0 && bytes.get(i + 1).zip(bytes.get(i + 2)).map(|(a, b)| a.is_ascii_hexdigit() && b.is_ascii_hexdigit()).unwrap_or(false) => {
                out.push(u8::from_str_radix(&raw[i + 1..i + 3], 16).unwrap_or(b'?'));
                i += 2;
            }
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
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
    // the page and its icon are open (they hold no data and no secret); everything else needs the secret of the window.
    // A Host check and the X-Jarvis header only stop a web page; a program sends any header, so the secret decides
    // (ui_auth.rs). No secret could be made: nothing passes.
    let open = req.method == "GET" && matches!(req.path.as_str(), "/" | "/icon.png" | "/favicon.ico");
    if !open && !crate::ui_auth::check(req.token.as_deref()) {
        if crate::guard::first("ui:no-secret", Duration::from_secs(60)) {
            warn!("UI: a request without the secret of the window was refused ({} {})", req.method, req.path);
        }
        respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
        return Ok(());
    }
    // everything this server does (a phrase, a scenario test, an action) comes through the door of the window
    let _door = crate::origin::enter(crate::origin::Source::Window);

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => respond(&mut stream, "200 OK", "text/html", &page()),
        ("GET", "/icon.png") | ("GET", "/favicon.ico") => respond_bytes(&mut stream, "200 OK", "image/png", ICON),
        ("GET", "/apps") => {
            let list: Vec<serde_json::Value> = crate::appindex::apps()
                .iter()
                .map(|a| json!({ "name": a.name, "launch": a.launch, "source": crate::appindex::source_title(&a.source), "ok": crate::appindex::launchable(a), "category": crate::appindex::category(a, crate::appindex::launchable(a)) }))
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
            // phone remote state for the window: every address of this PC a phone can reach, best first (home Wi-Fi, the hotspot of this
            // PC, a VPN for other networks). The links are https and carry NO secret; the pin of the key of the PC is shown for the eyes.
            let on = crate::remote::is_running();
            let http = crate::remote::allow_http();
            let addrs: Vec<serde_json::Value> = if on {
                crate::remote::addresses()
                    .into_iter()
                    .map(|a| json!({ "ip": a.ip, "kind": a.kind, "label": a.label, "url": crate::remote::url_for(&a.ip, true), "url_https": crate::remote::url_for(&a.ip, true), "url_http": if http { crate::remote::url_for(&a.ip, false) } else { String::new() } }))
                    .collect()
            } else {
                Vec::new()
            };
            let (fingerprint, pin) = match crate::remote_tls::current_pin() {
                Ok((_, pin, fp)) => (fp, pin),
                Err(_) => (String::new(), String::new()),
            };
            let body = serde_json::json!({
                "enabled": on,
                "url": if on { crate::remote::url() } else { String::new() },
                "url_https": if on { crate::remote::url_https() } else { String::new() },
                "addresses": addrs,
                "last_client": crate::remote::last_client(),
                "tunnel": crate::tunnel::json(),
                "fingerprint": fingerprint,
                "pin": pin,
                "allow_http": http,
                "tls_problem": crate::remote::tls_problem(),
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
        // the QR code for pairing: https://<address>/#pin=<pin of the key of the PC>[&c=<code>] (no secret of the link in it)
        ("GET", "/remote-qr") | ("GET", "/remote-qr-https") => {
            if !crate::remote::is_running() {
                respond(&mut stream, "404 Not Found", "text/plain", "remote control is off");
            } else {
                // ?ip=… : only an address of this PC (never a link to somewhere else); the same link as the button «Копировать ссылку привязки»
                let ip = req.query.split('&').find_map(|kv| kv.strip_prefix("ip=")).map(|v| v.to_string());
                match pairing_link_for(ip.as_deref()).ok().and_then(|l| qr_svg(&l)) {
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
        ("GET", "/license") => {
            let mut v = crate::licensing::status();
            v["prompt"] = crate::licensing::prompt_json();
            respond(&mut stream, "200 OK", "application/json", &v.to_string());
        }
        ("POST", "/license") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            respond(&mut stream, "200 OK", "application/json", &license_action(&body).to_string());
        }
        #[cfg(not(feature = "user-edition"))]
        ("POST", "/license-admin") => {
            if !req.custom_header {
                respond(&mut stream, "403 Forbidden", "text/plain", "forbidden");
                return Ok(());
            }
            let body: serde_json::Value = serde_json::from_slice(&req.body).unwrap_or(json!({}));
            respond(&mut stream, "200 OK", "application/json", &crate::licmgr::action(&body).to_string());
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
        ("GET", "/voiceprint") => respond(&mut stream, "200 OK", "application/json", &crate::voiceprint::status_json().to_string()),
        ("GET", "/games-list") => {
            let q = |k: &str| query_param(&req.query, k);
            let n = |k: &str, d: usize| q(k).parse::<usize>().unwrap_or(d);
            respond(&mut stream, "200 OK", "application/json", &crate::gamemode::games_list_json(&q("q"), &q("genre"), &q("origin"), &q("sort"), n("offset", 0), n("limit", 60)).to_string())
        }
        ("GET", "/game-icon") => {
            // the picture is made once and kept; no picture = 404 and the page draws its own icon. The browser keeps it for an hour.
            match crate::gameicon::icon_for(&query_param(&req.query, "id")) {
                Some((data, kind)) => respond_bytes_cached(&mut stream, kind, &data),
                None => respond(&mut stream, "404 Not Found", "text/plain", "no icon"),
            }
        }
        ("GET", "/games-report") => respond(&mut stream, "200 OK", "text/plain", &crate::gamemode::report_text()),
        ("GET", "/games") => respond(&mut stream, "200 OK", "application/json", &crate::gamemode::games_json().to_string()),
        ("GET", "/aim") => {
            let cfg = crate::gamemode::aim_config();
            crate::aimmodel::ensure_checked(&cfg);
            let mut v = crate::aimmodel::status_json(&cfg);
            v["run"] = crate::aimassist::status_json(&cfg);
            respond(&mut stream, "200 OK", "application/json", &v.to_string())
        }
        ("GET", "/aim-snapshot") => match crate::aimassist::snapshot_png() {
            Some(png) => respond_bytes(&mut stream, "200 OK", "image/png", &png),
            None => respond(&mut stream, "404 Not Found", "text/plain", "no snapshot"),
        },
        ("GET", "/aim-report") => respond(&mut stream, "200 OK", "text/plain", &crate::aimmodel::report_text(&crate::gamemode::aim_config())),
        ("GET", "/monitor") => {
            let s = crate::sysstat::sample();
            respond(&mut stream, "200 OK", "application/json", &crate::sysstat::monitor_json(&s, cfg!(target_os = "windows")).to_string())
        }
        ("GET", "/game") => respond(&mut stream, "200 OK", "application/json", &crate::gamemode::status_json().to_string()),
        ("GET", "/update") => respond(&mut stream, "200 OK", "application/json", &crate::updater::status_json().to_string()),
        ("GET", "/voice-install") => respond(&mut stream, "200 OK", "application/json", &crate::voice_install::status_json().to_string()),
        ("GET", "/chats") => respond(&mut stream, "200 OK", "application/json", &chats_json().to_string()),
        ("GET", "/scenarios") => respond(&mut stream, "200 OK", "application/json", &scenarios_json().to_string()),
        ("GET", "/tasks") => respond(&mut stream, "200 OK", "application/json", &crate::tasks::json().to_string()),
        ("GET", "/devices") => respond(&mut stream, "200 OK", "application/json", &crate::devices::json().to_string()),
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
                match crate::write_atomic(&path, &req.body) {
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
    let mut c = Command::new("cmd");
    c.args(args).creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    crate::null_stdio(&mut c);
    c.status().map(|s| s.success()).unwrap_or(false)
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
        info!("UI: open {} in a browser", base_url());
        let _ = Command::new("xdg-open").arg(&link).spawn();
        false
    }
}

// ---------------------------------------------------------------- the gateway of the phone
// What the phone page (remote.rs) may read and do besides sending a phrase: a short list, every item named here. The phone is a door that is
// open to whoever has its secret, so nothing that changes keys, the password, the license, files, the system or the program itself is on it.
// Everything runs under the phone door (origin::Source::Phone), like a phrase from the phone.
const PHONE_SETTINGS: [&str; 5] = ["voice_name", "voice_rate", "tts_volume", "assistant_name", "city"];
const PHONE_ACTIONS: [&str; 7] = ["app_launch", "ess_launch", "pl_launch", "game_launch", "game_mode", "speak_text", "apps_rescan"];

// programs that are only the leftovers of an installer (the same words as the window's list)
fn is_junk_program(name: &str) -> bool {
    let l = name.to_lowercase();
    ["uninstall", "удалить", "удаление", "readme", "help", "справка", "documentation", "документация", "website", "веб-сайт", "license", "лицензи", "release notes", "redistributable", "updater", "обновлени", "setup"].iter().any(|w| l.contains(w))
}

pub fn phone_get(route: &str, query: &str) -> Option<serde_json::Value> {
    let _door = crate::origin::enter(crate::origin::Source::Phone);
    Some(match route {
        "apps" => {
            let lib = crate::proglib::load();
            let hidden: std::collections::HashSet<String> = lib.hidden.iter().map(|h| h.to_lowercase()).collect();
            let apps: Vec<serde_json::Value> = crate::appindex::apps()
                .iter()
                .filter(|a| {
                    let ok = crate::appindex::launchable(a);
                    let cat = crate::appindex::category(a, ok);
                    ok && (cat == "user" || (cat == "other" && a.launch.to_lowercase().starts_with("shell:appsfolder"))) && !is_junk_program(&a.name) && !hidden.contains(&a.name.to_lowercase())
                })
                .map(|a| json!({ "name": a.name, "launch": a.launch }))
                .collect();
            json!({ "essentials": crate::proglib::essentials_json(), "apps": apps, "shortcuts": lib.shortcuts.iter().map(|s| json!({ "id": s.id, "name": s.name })).collect::<Vec<_>>() })
        }
        "tasks" => crate::tasks::json(),
        "chats" => chats_json(),
        r if r.starts_with("chat/") => chat_json(&r["chat/".len()..]),
        "commands" => {
            let v: serde_json::Value = serde_json::from_str(&commands_json()).unwrap_or(json!({}));
            // the phone needs the phrases and the state only
            let items = v.get("commands").or_else(|| v.get("items")).cloned().unwrap_or_else(|| if v.is_array() { v.clone() } else { json!([]) });
            json!({ "commands": items.as_array().map(|a| a.iter().map(|c| json!({ "id": c["id"], "group": c["group"], "phrases": c["phrases"], "description": c["description"], "status": c["status"] })).collect::<Vec<_>>()).unwrap_or_default() })
        }
        "scenarios" => json!({ "scenarios": crate::scenario::list().iter().map(|s| json!({ "id": s.id, "name": s.name, "phrase": s.phrase, "enabled": s.enabled, "steps": s.steps.len() })).collect::<Vec<_>>() }),
        "devices" => crate::devices::json(),
        "messages" => crate::social::json(),
        "games" => {
            let q = |k: &str| query_param(query, k);
            let mut v = crate::gamemode::games_list_json(&q("q"), "", "", "recent", 0, 60);
            v["game"] = crate::gamemode::status_json();
            v
        }
        "monitor" => crate::sysstat::monitor_json(&crate::sysstat::sample(), cfg!(target_os = "windows")),
        "diag" => {
            let d = crate::diag::json();
            json!({ "running": d["running"], "at": d["at"], "checks": d["checks"] })
        }
        "license" => {
            let mut v = crate::licensing::status();
            if let Some(o) = v.as_object_mut() {
                o.remove("key");
                o.remove("license_key");
            }
            v
        }
        "settings" => {
            let all = crate::uistate::settings_json();
            let mut out = serde_json::Map::new();
            for k in PHONE_SETTINGS {
                if let Some(x) = all.get(k) {
                    out.insert(k.to_string(), x.clone());
                }
            }
            serde_json::Value::Object(out)
        }
        _ => return None,
    })
}

// {"name": "...", ...}: an action of the window that the phone is allowed to ask for (the list above), a task, a scenario test run, a message read aloud,
// or a change of the few voice settings
pub fn phone_act(body: &serde_json::Value) -> serde_json::Value {
    let _door = crate::origin::enter(crate::origin::Source::Phone);
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let result = phone_act_inner(body);
    // the journal of critical actions: what the phone asked for (the name of the action, never its parameters) and what was decided
    let refused = result.get("ok").and_then(|v| v.as_bool()) == Some(false);
    let not_allowed = refused && result.get("message").and_then(|v| v.as_str()).map(|m| m.contains("недоступно с телефона")).unwrap_or(false);
    let sub = body.get("op").and_then(|v| v.as_str()).map(|o| format!("{}.{}", name, o)).unwrap_or_else(|| name.clone());
    if not_allowed {
        crate::audit::record("gateway", &sub, "", "refused", false, "действие не входит в список разрешённых для телефона");
    } else if PHONE_ACTIONS.contains(&name.as_str()) || matches!(name.as_str(), "task" | "scenario" | "setting" | "message") {
        crate::audit::record("gateway", &sub, "medium", if refused { "refused" } else { "allowed" }, false, if refused { "не выполнено" } else { "выполнено" });
    }
    result
}

fn phone_act_inner(body: &serde_json::Value) -> serde_json::Value {
    let name = body.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let bad = json!({ "ok": false, "message": "Это действие недоступно с телефона." });
    match name {
        n if PHONE_ACTIONS.contains(&n) => run_action(body),
        "task" => match body.get("op").and_then(|v| v.as_str()).unwrap_or("") {
            "add" | "remove" | "speak" => tasks_action(body),
            _ => bad,
        },
        "message" => match body.get("op").and_then(|v| v.as_str()).unwrap_or("") {
            "speak" => messages_action(body),
            _ => bad,
        },
        // only a run of a SAVED scenario by its id (a scenario that came inside the request is never taken) and the switch on/off
        "scenario" => match body.get("op").and_then(|v| v.as_str()).unwrap_or("") {
            "test" => scenario_action(&json!({ "op": "test", "id": body.get("id").cloned().unwrap_or(json!("")) })),
            "enable" => scenario_action(&json!({ "op": "enable", "id": body.get("id").cloned().unwrap_or(json!("")), "value": body.get("value").and_then(|v| v.as_bool()).unwrap_or(true) })),
            _ => bad,
        },
        "setting" => {
            let key = body.get("key").and_then(|v| v.as_str()).unwrap_or("");
            if !PHONE_SETTINGS.contains(&key) {
                return bad;
            }
            let value = body.get("value").cloned().unwrap_or(json!(null));
            apply_settings(&json!({ key: value }))
        }
        _ => bad,
    }
}

#[cfg(test)]
mod tests {
    // the program is locked: the guarded actions of the window refuse, the phone cannot unlock it or change the password, and the journal says so
    #[test]
    fn a_locked_program_refuses_and_the_phone_cannot_unlock_it() {
        let _s = crate::uistate::TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            crate::owner::set_password("пароль-для-теста", "").unwrap();
            crate::owner::lock();
            let r = run_action(&json!({ "name": "mem_delete", "id": "1" }));
            assert_eq!((r["ok"].as_bool(), r["locked"].as_bool()), (Some(false), Some(true)), "{}", r);
            let r = run_action(&json!({ "name": "audit_clear" }));
            assert_eq!(r["locked"].as_bool(), Some(true), "the journal cannot be wiped by a locked program: {}", r);
            for body in [
                json!({ "name": "owner_unlock", "password": "пароль-для-теста" }), json!({ "name": "owner_set", "new": "другой-пароль", "old": "" }),
                json!({ "name": "owner_recover", "code": "X", "new": "другой-пароль" }), json!({ "name": "audit_clear" }), json!({ "name": "perms_set", "id": "memory", "on": false }),
                json!({ "name": "pair_start" }), json!({ "name": "sync_run" }), json!({ "name": "home_act" }),
            ] {
                let r = phone_act(&body);
                assert!(r["message"].as_str().unwrap_or("").contains("недоступно с телефона"), "{} -> {}", body, r);
            }
            assert!(!crate::owner::unlocked(), "nothing the phone sent unlocked the program");
            let log = crate::audit::list(50);
            assert!(log.iter().any(|e| e.kind == "gateway" && e.decision == "refused" && e.door == "phone"), "{:?}", log);
        });
    }

    // a fix of the diagnostics page changes something only after the owner confirmed it; the phone cannot do it; a locked program refuses; the journal keeps every outcome
    #[test]
    fn a_fix_of_the_diagnostics_needs_the_confirmation_of_the_owner() {
        let _s = crate::uistate::TEST_LOCK.lock();
        crate::store::with_temp(|| {
            crate::owner::reset_for_tests();
            let muted = || crate::MIC_MUTED.load(std::sync::atomic::Ordering::SeqCst);
            crate::uistate::set_mic_muted(true);
            // the preview changes nothing
            let r = run_action(&json!({ "name": "diag_fix_preview", "id": "mic_unmute" }));
            assert!(r["preview"].as_str().unwrap().contains("Включить микрофон") && muted(), "{}", r);
            // no confirmation / a confirmation that is not a plain true: refused, nothing changed, the refusal is in the journal
            for body in [json!({ "name": "diag_fix_apply", "id": "mic_unmute" }), json!({ "name": "diag_fix_apply", "id": "mic_unmute", "confirm": "true" }), json!({ "name": "diag_fix_apply", "id": "mic_unmute", "confirm": false })] {
                let r = run_action(&body);
                assert_eq!((r["ok"].as_bool(), r["needs_confirm"].as_bool()), (Some(false), Some(true)), "{}", r);
                assert!(muted());
            }
            // an unknown fix (a command, a path) is not a fix
            for id in ["", "shell", "C:\\Windows\\System32\\cmd.exe", "all"] {
                assert_eq!(run_action(&json!({ "name": "diag_fix_apply", "id": id, "confirm": true }))["ok"], false);
            }
            assert!(muted());
            // the phone cannot confirm fixes
            let r = phone_act(&json!({ "name": "diag_fix_apply", "id": "mic_unmute", "confirm": true }));
            assert!(muted(), "{}", r);
            // a locked program refuses even a confirmed fix
            crate::owner::set_password("пароль-для-теста", "").unwrap();
            crate::owner::lock();
            let r = run_action(&json!({ "name": "diag_fix_apply", "id": "mic_unmute", "confirm": true }));
            assert_eq!((r["locked"].as_bool(), r["code"].as_str()), (Some(true), Some("locked")));
            assert!(muted());
            // a wrong password does not unlock it, and the fix is refused again
            assert!(crate::owner::unlock("не-тот-пароль").is_err());
            let r = run_action(&json!({ "name": "diag_fix_apply", "id": "mic_unmute", "confirm": true }));
            assert_eq!(r["code"].as_str(), Some("locked"));
            assert!(muted());
            // the owner confirmed: done once; a second time there is nothing to change
            crate::owner::unlock("пароль-для-теста").unwrap();
            let r = run_action(&json!({ "name": "diag_fix_apply", "id": "mic_unmute", "confirm": true }));
            assert_eq!((r["ok"].as_bool(), r["applied"].as_bool()), (Some(true), Some(true)), "{}", r);
            assert!(!muted());
            assert_eq!(run_action(&json!({ "name": "diag_fix_apply", "id": "mic_unmute", "confirm": true }))["applied"], false);
            let outcomes: Vec<String> = crate::audit::list(100).into_iter().rev().filter(|e| e.kind == "diag_fix").map(|e| e.decision).collect();
            assert_eq!(outcomes, ["declined", "declined", "declined", "applied", "nothing_to_do"]);
        });
    }

    // missing and broken parameters of the gateway are an answer, not a crash, and nothing of them reaches the journal
    #[test]
    fn the_gateway_survives_missing_and_broken_parameters() {
        let _s = crate::uistate::TEST_LOCK.lock();
        crate::store::with_temp(|| {
            for body in [json!({}), json!(null), json!("текст"), json!([1, 2]), json!({ "name": 5 }), json!({ "name": "task" }), json!({ "name": "task", "op": "add" }), json!({ "name": "setting", "key": "city" }), json!({ "name": "scenario", "op": "test" })] {
                let r = phone_act(&body);
                assert!(r.get("ok").is_some(), "{} -> {}", body, r);
            }
            assert!(phone_get("../../etc/passwd", "").is_none() && phone_get("", "").is_none() && phone_get("chat", "").is_none());
            let raw = std::fs::read_to_string(crate::store::file("audit_log.json")).unwrap_or_default();
            assert!(!raw.contains("текст"), "the parameters are not written to the journal");
        });
    }

    // «Мой голос» through the window: the buttons start the recording at once (without the wake word) and the lock cannot be on without a print
    #[test]
    fn the_voice_buttons_start_the_recording_the_listening_and_refuse_the_lock_without_a_print() {
        let _l = crate::voiceprint::TEST_LOCK.lock();
        crate::voiceprint::delete();
        crate::MIC_MUTED.store(false, std::sync::atomic::Ordering::SeqCst);
        crate::LISTEN_NOW.store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(run_action(&json!({ "name": "voice_print_enable", "value": true }))["ok"], false);
        let r = run_action(&json!({ "name": "voice_enroll_start" }));
        assert_eq!(r["ok"], true);
        assert!(crate::voiceprint::capturing() && crate::LISTEN_NOW.load(std::sync::atomic::Ordering::SeqCst));
        let st = crate::voiceprint::status_json();
        assert_eq!((st["enrolling"].as_bool(), st["step"].as_u64(), st["total"].as_u64()), (Some(true), Some(0), Some(5)));
        assert!(st["prompt"].as_str().unwrap().contains("Джарвис"));
        run_action(&json!({ "name": "voice_enroll_cancel" }));
        assert!(!crate::voiceprint::capturing());
        // a muted microphone cannot record
        crate::MIC_MUTED.store(true, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(run_action(&json!({ "name": "voice_enroll_start" }))["ok"], false);
        crate::MIC_MUTED.store(false, std::sync::atomic::Ordering::SeqCst);
        crate::LISTEN_NOW.store(false, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(run_action(&json!({ "name": "voice_print_strict", "level": 9 }))["ok"], true);
        assert_eq!(crate::voiceprint::status_json()["strictness"], 2, "the level is kept in its range");
        crate::voiceprint::delete();
    }

    // ---- the library of games through the actions of the window (Admin edition: every action is allowed; the User edition is tested above) ----
    #[cfg(not(feature = "user-edition"))]
    mod library {
        use super::*;
        use crate::gamelist;

        fn temp(tag: &str) -> std::path::PathBuf {
            let d = std::env::temp_dir().join(format!("jarvis-ui-lib-{}-{}", tag, std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            d
        }

        fn exe_in(dir: &std::path::Path, folder: &str, file: &str) -> String {
            let d = dir.join(folder);
            std::fs::create_dir_all(&d).unwrap();
            let p = d.join(file);
            std::fs::write(&p, b"MZ game").unwrap();
            p.to_string_lossy().to_string()
        }

        fn list(text: &str) -> serde_json::Value {
            crate::gamemode::games_list_json(text, "", "", "name", 0, 200)
        }

        #[test]
        fn a_game_chosen_by_path_enters_the_library_the_catalog_and_the_search_and_survives_a_restart() {
            gamelist::test_reset();
            let d = temp("add");
            let path = exe_in(&d, "Моя игра с пробелами", "Супер Гонка.exe");
            let r = run_action(&json!({ "name": "game_add", "path": path, "title": "", "genre": "racing" }));
            assert_eq!(r["ok"], true, "{}", r);
            assert_eq!(r["new"], true);
            let id = r["id"].as_str().unwrap().to_string();
            // the name comes from the file, the kind is the chosen one, the path is kept, the file was not started
            let g = gamelist::get(&id).unwrap();
            assert_eq!((g.name.as_str(), g.genre, g.exe.as_str(), g.source.as_str(), g.verified), ("Супер Гонка", crate::gamemode::Genre::Racing, "супер гонка.exe", "manual", true));
            assert_eq!(g.launches, 0);
            // the catalog and the search see it (a part of the name, other case)
            assert_eq!(list("")["total"], 1);
            assert_eq!(list("")["items"][0]["id"], id.as_str());
            assert_eq!(list("ГОНКА")["matched"], 1);
            assert_eq!(list("ничего")["matched"], 0);
            assert_eq!(crate::gamemode::games_json()["total"], 1);
            assert_eq!(crate::gamemode::games_json()["recent"][0]["launch"], "path");
            // a restart: the file as it is written is read back to the same library
            let written = gamelist::to_json(&gamelist::File { games: gamelist::load(), options: gamelist::options() });
            assert_eq!(gamelist::parse_file(&written).games, gamelist::load());
            std::fs::remove_dir_all(&d).ok();
            gamelist::test_reset();
        }

        #[test]
        fn a_repeated_add_is_asked_about_and_makes_no_duplicate_and_bad_files_are_refused() {
            gamelist::test_reset();
            let d = temp("dup");
            let path = exe_in(&d, "Game", "game.exe");
            let first = run_action(&json!({ "name": "game_add", "path": path, "title": "Игра", "genre": "action" }));
            let id = first["id"].as_str().unwrap().to_string();
            // the same file again: told, offered, nothing changed
            let again = run_action(&json!({ "name": "game_add", "path": path, "title": "Другое имя" }));
            assert_eq!((again["ok"].as_bool(), again["duplicate"].as_bool(), again["id"].as_str()), (Some(false), Some(true), Some(id.as_str())));
            assert_eq!(gamelist::load().len(), 1);
            assert_eq!(gamelist::get(&id).unwrap().name, "Игра");
            // the same file in other writing is the same game
            let other = run_action(&json!({ "name": "game_add", "path": path.to_uppercase().replace("\\", "/") }));
            assert_ne!(other["ok"], true);
            // renewed on the user's word
            let renew = run_action(&json!({ "name": "game_add", "path": path, "title": "Новое имя", "renew": true }));
            assert_eq!(renew["ok"], true);
            assert_eq!(renew["new"], false);
            assert_eq!(gamelist::load().len(), 1);
            assert_eq!(gamelist::get(&id).unwrap().name, "Новое имя");
            // not a game: a document, a picture, a missing file, a browser, an online game, a bare word
            for (file, body) in [("отчёт.docx", "x"), ("фото.png", "x"), ("архив.zip", "x")] {
                let p = d.join(file);
                std::fs::write(&p, body).unwrap();
                let r = run_action(&json!({ "name": "game_add", "path": p.to_string_lossy() }));
                assert_eq!(r["ok"], false, "{}", file);
                assert!(r["message"].as_str().unwrap().contains(".exe"), "{}", r);
            }
            assert_eq!(run_action(&json!({ "name": "game_add", "path": d.join("нет.exe").to_string_lossy() }))["message"], crate::gamelaunch::NOT_FOUND);
            for bad in ["chrome.exe", "valorant.exe"] {
                let p = exe_in(&d, "x", bad);
                assert_eq!(run_action(&json!({ "name": "game_add", "path": p }))["ok"], false, "{}", bad);
            }
            assert_eq!(run_action(&json!({ "name": "game_add", "path": "просто слово" }))["ok"], false);
            assert_eq!(gamelist::load().len(), 1);
            std::fs::remove_dir_all(&d).ok();
            gamelist::test_reset();
        }

        #[test]
        fn removing_takes_only_the_entry_and_the_files_of_the_game_stay() {
            gamelist::test_reset();
            let d = temp("rm");
            let path = exe_in(&d, "Game", "game.exe");
            let save = d.join("Game").join("save1.sav");
            let cfg = d.join("Game").join("settings.ini");
            std::fs::write(&save, b"save").unwrap();
            std::fs::write(&cfg, b"cfg").unwrap();
            let id = run_action(&json!({ "name": "game_add", "path": path, "title": "Игра" }))["id"].as_str().unwrap().to_string();
            // the user said «no» in the window: the page sends nothing; and a call with a wrong id changes nothing
            assert_eq!(run_action(&json!({ "name": "game_remove", "id": "g-wrong" }))["ok"], false);
            assert_eq!(gamelist::load().len(), 1);
            // «yes»
            let r = run_action(&json!({ "name": "game_remove", "id": id }));
            assert_eq!(r["ok"], true);
            assert!(r["message"].as_str().unwrap().contains("Файлы игры остались"));
            assert!(gamelist::load().is_empty());
            // every file of the game, its folder and its saves are as they were
            assert!(std::path::Path::new(&path).is_file() && save.is_file() && cfg.is_file());
            assert_eq!(std::fs::read(&save).unwrap(), b"save");
            // it is in no result any more
            assert_eq!(list("игра")["matched"], 0);
            assert_eq!(list("")["total"], 0);
            assert_eq!(crate::gamemode::games_json()["total"], 0);
            std::fs::remove_dir_all(&d).ok();
            gamelist::test_reset();
        }

        #[test]
        fn the_launch_button_starts_the_chosen_game_and_a_moved_file_asks_for_a_new_path() {
            gamelist::test_reset();
            let d = temp("run");
            let path = exe_in(&d, "Игра", "game.exe");
            let id = run_action(&json!({ "name": "game_add", "path": path, "title": "Игра" }))["id"].as_str().unwrap().to_string();
            // adding did not start it
            assert_eq!(gamelist::get(&id).unwrap().launches, 0);
            // the file is moved: the message of the task, and the page is told to offer the choice of a file
            std::fs::remove_file(&path).unwrap();
            let r = run_action(&json!({ "name": "game_launch", "id": id }));
            assert_eq!((r["ok"].as_bool(), r["message"].as_str(), r["relink"].as_bool()), (Some(false), Some("Файл игры не найден. Выберите новый путь к игре"), Some(true)));
            // a new path for the same game: the id stays, the library has one game
            let new_path = exe_in(&d, "Новая папка", "game.exe");
            let r = run_action(&json!({ "name": "game_set_path", "id": id, "path": new_path }));
            assert_eq!(r["ok"], true, "{}", r);
            assert_eq!(gamelist::load().len(), 1);
            assert_eq!(gamelist::get(&id).unwrap().path, new_path);
            // an unknown id and a path that is not a program
            assert_eq!(run_action(&json!({ "name": "game_launch", "id": "nope" }))["ok"], false);
            assert_eq!(run_action(&json!({ "name": "game_set_path", "id": id, "path": d.join("a.txt").to_string_lossy() }))["ok"], false);
            std::fs::remove_dir_all(&d).ok();
            gamelist::test_reset();
        }

        #[test]
        fn the_genre_changes_by_id_and_the_catalog_filters_by_it() {
            gamelist::test_reset();
            let d = temp("genre");
            let a = run_action(&json!({ "name": "game_add", "path": exe_in(&d, "a", "a.exe"), "title": "А", "genre": "shooter" }))["id"].as_str().unwrap().to_string();
            run_action(&json!({ "name": "game_add", "path": exe_in(&d, "b", "b.exe"), "title": "Б", "genre": "shooter" }));
            assert_eq!(run_action(&json!({ "name": "game_genre", "id": a, "genre": "racing" }))["ok"], true);
            assert_eq!(crate::gamemode::games_list_json("", "racing", "", "name", 0, 50)["matched"], 1);
            assert_eq!(crate::gamemode::games_list_json("", "shooter", "", "name", 0, 50)["matched"], 1);
            assert_eq!(run_action(&json!({ "name": "game_genre", "id": a, "genre": "nonsense" }))["ok"], false);
            std::fs::remove_dir_all(&d).ok();
            gamelist::test_reset();
        }

        #[test]
        fn a_library_of_a_hundred_and_of_two_hundred_games_is_paged_without_gaps_and_the_numbers_are_the_real_ones() {
            for n in [100usize, 200] {
                gamelist::test_reset();
                let list: Vec<gamelist::Entry> = (0..n)
                    .map(|i| gamelist::Entry { path: format!("D:\\Игры\\Игра {}\\g{}.exe", i, i), verified: true, store_id: if i % 2 == 0 { format!("steam:{}", i + 1) } else { String::new() }, ..gamelist::Entry::basic(&format!("g{}.exe", i), &format!("Игра {}", i), crate::gamemode::Genre::Action, if i % 2 == 0 { "scan" } else { "manual" }) })
                    .collect();
                assert_eq!(gamelist::add_many(list.clone()), n);
                // the same scan again: nothing new, the count stays the real one
                assert_eq!(gamelist::add_many(list), 0);
                let s = crate::gamemode::games_json();
                assert_eq!(s["total"], n);
                assert_eq!(s["recent"].as_array().unwrap().len(), 12, "the main page shows a few games, not the library");
                let mut seen = std::collections::HashSet::new();
                let mut off = 0;
                loop {
                    let page = crate::gamemode::games_list_json("", "", "", "added", off, 60);
                    assert_eq!(page["total"], n);
                    let items = page["items"].as_array().unwrap();
                    if items.is_empty() {
                        break;
                    }
                    for g in items {
                        assert!(seen.insert(g["id"].as_str().unwrap().to_string()), "no game twice in the pages");
                    }
                    off += items.len();
                }
                assert_eq!(seen.len(), n);
                // the filter by the origin: half of them came by the scan of Steam
                assert_eq!(crate::gamemode::games_list_json("", "", "steam", "name", 0, 200)["matched"], n / 2);
                assert_eq!(crate::gamemode::games_list_json("", "", "manual", "name", 0, 200)["matched"], n / 2);
            }
            gamelist::test_reset();
        }

        #[test]
        fn the_address_of_a_search_is_decoded_with_russian_letters_and_plus_and_a_broken_escape() {
            assert_eq!(query_param("q=%D0%BC%D0%BE%D1%80+%D1%83%D1%82%D0%BE%D0%BF%D0%B8%D1%8F&sort=name", "q"), "мор утопия");
            assert_eq!(query_param("q=a&sort=added", "sort"), "added");
            assert_eq!(query_param("q=100%", "q"), "100%");
            assert_eq!(query_param("q=%zz", "q"), "%zz");
            assert_eq!(query_param("a=1", "q"), "");
            assert_eq!(query_param("qq=1&q=2", "q"), "2");
        }
    }


    // Demo (no licence): the actions of the Games page that start or change the paid function are refused, the cleaning-up ones are not
    #[cfg(feature = "user-edition")]
    #[test]
    fn without_a_licence_the_games_page_refuses_adding_scanning_and_switching_on_but_lets_the_user_clean_up() {
        let _g = crate::licensing::test_lock();
        crate::licensing::test_reset();
        crate::gamelist::test_reset();
        for (name, extra) in [("game_add", json!({ "path": "game.exe" })), ("game_add_flagged", json!({ "path": "game.exe" })), ("game_scan", json!({})), ("game_opts", json!({ "overlay": true })), ("game_mode", json!({ "value": true })), ("game_aim", json!({ "value": true })), ("game_launch", json!({ "id": "g1" })), ("game_pick_file", json!({})), ("aim_model_install", json!({})), ("aim_pick_model", json!({})), ("aim_model_check", json!({})), ("aim_settings", json!({ "settings": { "fps": 20 } })), ("aim_snapshot", json!({})), ("game_set_path", json!({ "id": "g1", "path": "D:\\x.exe" }))] {
            let mut body = extra;
            body["name"] = json!(name);
            let r = run_action(&body);
            assert_eq!(r["ok"], false, "{}: {}", name, r);
            assert!(r["message"].as_str().unwrap_or("").contains("Premium"), "{}: {}", name, r);
        }
        assert!(crate::gamelist::load().is_empty(), "nothing was added");
        assert!(!crate::gamelist::options().overlay);
        // removing, changing the kind and deleting a macro are cleaning up: they are never behind the paywall
        let r = run_action(&json!({ "name": "game_remove", "id": "nothing" }));
        assert_eq!(r["message"], "Этой игры нет в библиотеке.");
        let r = run_action(&json!({ "name": "macro_delete", "macro": "нет такого" }));
        assert_eq!(r["message"], "Такого макроса нет.");
    }

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
    fn the_stop_button_really_cancels_the_generation_and_keeps_the_object() {
        let _g = crate::session::TEST_LOCK.lock();
        let (flag, task) = crate::objects::with(|e| {
            e.reset();
            e.open_app("vscode").unwrap();
            e.bind("vscode", crate::objects::Kind::CodeFile, "calculator.py", None, None, crate::objects::Detect::ProcessTitle).unwrap();
            let (t, f) = e.start_ai("напиши калькулятор").unwrap();
            (f, t)
        });
        assert_eq!(crate::uistate::state_json()["object"]["state"], "AI_TASK");
        let r = run_action(&json!({ "name": "object_stop" }));
        assert_eq!(r["ok"], true, "{}", r);
        assert!(flag.load(std::sync::atomic::Ordering::SeqCst), "the generation was really told to stop");
        assert!(r["message"].as_str().unwrap().contains("Генерация отменена"), "{}", r);
        assert_eq!(crate::uistate::state_json()["object"]["state"], "PAUSED");
        assert_eq!(crate::uistate::state_json()["object"]["name"], "calculator.py", "the object stays");
        assert!(!crate::objects::with(|e| e.ai_finished(task, Ok("поздний ответ".into()))), "a late answer is dropped");
        // nothing to stop any more: the button does not claim success
        assert_eq!(run_action(&json!({ "name": "object_stop" }))["ok"], false);
        crate::objects::with(|e| e.reset());
    }

    #[test]
    fn the_screen_check_takes_no_picture_without_the_consent_or_a_provider() {
        let _g = crate::uistate::TEST_LOCK.lock();
        // no provider: the answer says what is missing, nothing is sent
        crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
        let r = run_action(&json!({ "name": "screen_check" }));
        assert_eq!(r["ok"], false, "{}", r);
        // a provider, but the analysis of the screen is not allowed: refused before any picture is taken
        crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_provider: "gemini".into(), ai_key: "key-123".into(), ai_screen: false, ..Default::default() });
        let r = run_action(&json!({ "name": "screen_check" }));
        assert_eq!(r["ok"], false, "{}", r);
        assert!(r["message"].as_str().unwrap().contains("Анализ экрана не разрешён"), "{}", r);
        // the AI switched off: the same
        crate::uistate::set_for_tests(crate::uistate::UiSettings { ai_provider: "gemini".into(), ai_key: "key-123".into(), ai_screen: true, ai_enabled: false, ..Default::default() });
        assert_eq!(run_action(&json!({ "name": "screen_check" }))["ok"], false);
        crate::uistate::set_for_tests(crate::uistate::UiSettings::default());
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

    // one request through the real handler of the page server, over a real local connection
    fn http(raw: &str) -> String {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let _ = handle(stream);
        });
        let mut c = std::net::TcpStream::connect(addr).unwrap();
        c.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
        c.write_all(raw.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        let _ = c.read_to_end(&mut bytes);
        let _ = server.join();
        String::from_utf8_lossy(&bytes).to_string()
    }

    fn get(path: &str, extra: &str) -> String {
        http(&format!("GET {} HTTP/1.1\r\nHost: 127.0.0.1:9714\r\n{}Connection: close\r\n\r\n", path, extra))
    }

    fn post(path: &str, body: &str, extra: &str) -> String {
        http(&format!("POST {} HTTP/1.1\r\nHost: 127.0.0.1:9714\r\nX-Jarvis: 1\r\n{}Content-Length: {}\r\nConnection: close\r\n\r\n{}", path, extra, body.len(), body))
    }

    fn status(answer: &str) -> &str {
        answer.lines().next().unwrap_or("")
    }

    const DATA: &[&str] = &["/state", "/remote", "/remote-qr", "/remote-qr-net", "/remote-qr-https", "/diag", "/diag-report", "/games", "/games-list", "/game-icon", "/games-report", "/aim", "/aim-report", "/aim-snapshot", "/voiceprint", "/settings", "/voices", "/modules", "/chats", "/scenarios", "/tasks", "/messages", "/telegram", "/commands", "/apps", "/files", "/avatar", "/voice-install", "/reaction-voices", "/devices"];

    // a program on this computer (a web page is stopped before, by the Host) sends any header it likes: without the secret of the
    // window nothing is read and nothing is done, whatever else the request says
    #[test]
    fn without_the_secret_of_the_window_nothing_is_read_and_nothing_is_done() {
        let _g = crate::ui_auth::TEST_LOCK.lock();
        crate::ui_auth::reset_for_tests(None, false);
        crate::ui_auth::init();
        let secret = crate::ui_auth::token().unwrap();
        for path in DATA {
            let a = get(path, "");
            assert_eq!(status(&a), "HTTP/1.1 403 Forbidden", "{} without the secret: {}", path, a);
            assert!(a.ends_with("forbidden"), "{}: no data in a refusal", path);
            // the X-Jarvis header, an Origin of the window and a guess are not the secret
            for extra in ["X-Jarvis: 1\r\n".to_string(), "Origin: http://127.0.0.1:9714\r\nReferer: http://127.0.0.1:9714/\r\n".to_string(), format!("X-Jarvis-Token: {}\r\n", "0".repeat(64)), "X-Jarvis-Token: \r\n".to_string(), format!("X-Jarvis-Token: {}\r\n", &secret[..63])] {
                assert_eq!(status(&get(path, &extra)), "HTTP/1.1 403 Forbidden", "{} with {:?}", path, extra);
            }
            assert_eq!(status(&get(&format!("{}?jt={}", path, "1".repeat(64)), "")), "HTTP/1.1 403 Forbidden", "{} with a wrong jt=", path);
        }
        for (path, body) in [("/action", r#"{"name":"app_exit"}"#), ("/action", r#"{"name":"remote_on"}"#), ("/action", r#"{"name":"remote_tunnel","value":true}"#), ("/chat", r#"{"op":"ask","text":"выключи компьютер"}"#), ("/tasks", r#"{"op":"clear"}"#), ("/scenario", r#"{"op":"test_step"}"#), ("/settings", r#"{"ai_key":"x"}"#), ("/telegram", r#"{"op":"forget"}"#), ("/messages", r#"{"op":"clear"}"#), ("/avatar", "x"), ("/window-closed", "")] {
            let a = post(path, body, "");
            assert_eq!(status(&a), "HTTP/1.1 403 Forbidden", "POST {} with X-Jarvis but without the secret: {}", path, a);
        }
        // the page and its icon are open, and hold no secret
        let page = get("/", "");
        assert_eq!(status(&page), "HTTP/1.1 200 OK");
        assert!(!page.contains(&secret), "the page does not contain the secret");
        assert_eq!(status(&get("/icon.png", "")), "HTTP/1.1 200 OK");
        assert_eq!(status(&get("/favicon.ico", "")), "HTTP/1.1 200 OK");
        // a POST to the page is not the open GET
        assert_eq!(status(&post("/", "", "")), "HTTP/1.1 403 Forbidden");
        crate::ui_auth::reset_for_tests(None, false);
    }

    #[test]
    fn with_the_secret_the_window_works_as_before_and_the_host_check_still_applies() {
        let _g = crate::ui_auth::TEST_LOCK.lock();
        crate::ui_auth::reset_for_tests(None, false);
        crate::ui_auth::init();
        let t = crate::ui_auth::token().unwrap();
        let a = get("/state", &format!("X-Jarvis-Token: {}\r\n", t));
        assert_eq!(status(&a), "HTTP/1.1 200 OK", "{}", a);
        assert!(a.contains("application/json"));
        // an image cannot send a header: the secret in the address
        assert_ne!(status(&get(&format!("/avatar?jt={}&t=1", t), "")), "HTTP/1.1 403 Forbidden");
        assert_eq!(status(&get(&format!("/state?jt={}", t), "")), "HTTP/1.1 200 OK");
        // a POST still needs X-Jarvis as well as the secret (a page of a web site cannot add it)
        let raw = format!("POST /tasks HTTP/1.1\r\nHost: 127.0.0.1:9714\r\nX-Jarvis-Token: {}\r\nContent-Length: 14\r\nConnection: close\r\n\r\n{{\"op\":\"clear\"}}", t);
        assert_eq!(status(&http(&raw)), "HTTP/1.1 403 Forbidden", "the secret alone does not open a POST");
        // the Host check (DNS rebinding) still applies with the right secret
        let rebind = http(&format!("GET /state HTTP/1.1\r\nHost: evil.example:9714\r\nX-Jarvis-Token: {}\r\nConnection: close\r\n\r\n", t));
        assert_eq!(status(&rebind), "HTTP/1.1 403 Forbidden");
        crate::ui_auth::reset_for_tests(None, false);
    }

    // the secret could not be made (or the file is gone and memory is empty): the server is closed, not open
    #[test]
    fn when_the_protection_is_unavailable_the_server_refuses_everything_but_the_page() {
        let _g = crate::ui_auth::TEST_LOCK.lock();
        crate::ui_auth::reset_for_tests(None, true);
        assert!(!crate::ui_auth::init());
        for path in DATA {
            for extra in ["", "X-Jarvis-Token: \r\n", "X-Jarvis: 1\r\n"] {
                assert_eq!(status(&get(path, extra)), "HTTP/1.1 403 Forbidden", "{} {:?}", path, extra);
            }
            assert_eq!(status(&get(&format!("{}?jt=", path), "")), "HTTP/1.1 403 Forbidden", "{} with an empty jt=", path);
        }
        assert_eq!(status(&post("/action", r#"{"name":"app_exit"}"#, "X-Jarvis-Token: \r\n")), "HTTP/1.1 403 Forbidden");
        assert_eq!(status(&get("/", "")), "HTTP/1.1 200 OK", "the page itself opens (it can say that the secret is missing)");
        crate::ui_auth::reset_for_tests(None, false);
    }


    // the page is made by build.rs: in the customer's build the administrator's parts are not in it at all; in the owner's build they are
    #[test]
    fn jarvis_user_gets_a_page_without_the_license_manager() {
        let source = include_str!("ui_page.html");
        assert_eq!(source.matches("ADMIN-BEGIN").count(), source.matches("ADMIN-END").count());
        assert!(source.contains("page-licmgr") && source.contains("license-admin"), "the source has the administrator's page");
        for needed in ["lic-dlg", "exp-dlg", "prem-dlg", "Да, продлить", "Нет, выйти", "Активировать Premium", "Позже"] {
            assert!(PAGE.contains(needed), "{}", needed);
        }
        if cfg!(feature = "user-edition") {
            for marker in ["ADMIN-BEGIN", "ADMIN-END", "license-admin", "page-licmgr", "lmCall", "Создать ключи", "jla_", "Токен администратора", "data-page=\"licmgr\""] {
                assert!(!PAGE.contains(marker), "the user page still has {}", marker);
            }
        } else {
            assert!(PAGE.contains("page-licmgr") && PAGE.contains("license-admin"));
        }
    }
}

#[cfg(test)]
pub(crate) fn qr_svg_for_tests(data: &str) -> Option<String> {
    qr_svg(data)
}
