//! The recorded phrase of the voice pack for a command: «Открываю ютуб», «Громче», «Сейчас скажу время» are not the general «Выполнено» but a clip
//! that was recorded for exactly this command (files `cmd_*.mp3` of a pack, e.g. Luna 2.0 and Jarvis New). The table here only says which clip belongs
//! to which command; whether the current pack has the clip is checked when it is played, and a pack without it keeps its general reactions.
//! Only clear matches are listed: a command without a clip (or a doubtful one) keeps the general «ok» sound.

/// The clips for commands that always do the same thing (by the id of the command): one of them is chosen at random.
const FIXED: &[(&str, &[&str])] = &[
    ("assistant_volume_up", &["cmd_volume_up"]),
    ("assistant_volume_down", &["cmd_volume_down"]),
    ("assistant_volume_mute", &["cmd_volume_mute"]),
    ("assistant_volume_unmute", &["cmd_volume_unmute"]),
    ("assistant_brightness_up", &["cmd_brightness_up"]),
    ("assistant_brightness_down", &["cmd_brightness_down"]),
    ("assistant_timer_set", &["cmd_timer_set"]),
    ("assistant_reminder", &["cmd_remind"]),
    ("assistant_power_off", &["cmd_shutdown_pc"]),
    ("assistant_power_restart", &["cmd_restart_pc"]),
    ("assistant_power_cancel", &["cmd_cancel_shutdown"]),
    ("assistant_info_time", &["cmd_what_time"]),
    ("assistant_info_date", &["cmd_what_date"]),
    ("assistant_media_playpause", &["cmd_media_play_pause"]),
    ("assistant_media_next", &["cmd_media_next"]),
    ("assistant_media_prev", &["cmd_media_prev"]),
    ("assistant_game_on", &["cmd_game_mode"]),
    ("steam_1", &["cmd_game_mode"]),
    ("assistant_tts_stop", &["cmd_speak_stop"]),
    ("assistant_translate", &["cmd_translate"]),
    ("assistant_translate_clip", &["cmd_translate"]),
    ("assistant_wiki", &["cmd_search_wiki"]),
    ("assistant_maps", &["cmd_open_maps"]),
    ("assistant_office_dictate", &["cmd_dictate"]),
    ("assistant_office_new", &["cmd_input_new_doc"]),
    ("assistant_greet", &["hello1", "hello2"]),
    ("assistant_howareyou", &["howare1", "howare2"]),
    ("jarvis_4", &["joke1", "joke2", "joke3", "joke4", "joke5", "joke6", "joke7", "joke8"]),
    ("windows_1", &["cmd_minimize_all"]),
    ("windows_2", &["cmd_empty_trash"]),
    ("windows_3", &["cmd_open_task_manager"]),
    ("windows_4", &["cmd_screenshot"]),
    ("windows_5", &["cmd_lock_pc"]),
    ("windows_6", &["cmd_sleep_pc"]),
    ("windows_8", &["cmd_input_switch_lang"]),
    ("browser_open", &["cmd_browser_open"]),
    ("calculator_1", &["cmd_open_calculator"]),
    ("web_tab_new", &["cmd_browser_new_tab"]),
    ("web_tab_close", &["cmd_browser_close_tab"]),
    ("web_tab_reopen", &["cmd_browser_reopen_tab"]),
    ("web_tab_next", &["cmd_browser_next_tab"]),
    ("web_tab_prev", &["cmd_browser_prev_tab"]),
    ("web_back", &["cmd_browser_back"]),
    ("web_refresh", &["cmd_browser_refresh"]),
    ("web_downloads", &["cmd_browser_downloads"]),
    ("web_incognito", &["cmd_browser_incognito"]),
    ("web_find", &["cmd_input_find"]),
    ("key_undo", &["cmd_input_undo"]),
    ("key_redo", &["cmd_input_redo"]),
    ("key_select_all", &["cmd_input_select_all"]),
    ("key_copy", &["cmd_input_copy"]),
    ("key_paste", &["cmd_input_paste"]),
    ("key_cut", &["cmd_input_cut"]),
    ("key_print", &["cmd_input_print"]),
    ("key_save", &["cmd_input_save"]),
    ("win_switch", &["cmd_switch_window"]),
    ("win_maximize", &["cmd_maximize_window"]),
    ("win_minimize", &["cmd_minimize_window"]),
];

/// The commands whose clip depends on WHAT was named («открой телеграм», «открой загрузки»): the clip is found by the words that were said.
const BY_WORDS: &[&str] = &["assistant_program_open", "assistant_website", "assistant_place_open"];

/// (words, clip): a part of the phrase that starts at the beginning of a word. The first line that matches wins, so the longer names come first
/// («яндекс музыка» before «яндекс»).
const WORDS: &[(&str, &str)] = &[
    (" ютуб музык", "cmd_open_youtube_music"), (" ютьюб музык", "cmd_open_youtube_music"), (" youtube music", "cmd_open_youtube_music"),
    (" ютуб", "cmd_open_youtube"), (" ютюб", "cmd_open_youtube"), (" ютьюб", "cmd_open_youtube"), (" youtube", "cmd_open_youtube"),
    (" яндекс музык", "cmd_open_yandex_music"), (" яндекс карт", "cmd_open_yandex_maps"), (" яндекс диск", "cmd_open_yandex_disk"), (" яндекс", "cmd_open_yandex"),
    (" гугл диск", "cmd_open_google_drive"),
    (" телеграм", "cmd_open_telegram"), (" telegram", "cmd_open_telegram"),
    (" ватсап", "cmd_open_whatsapp"), (" вотсап", "cmd_open_whatsapp"), (" whatsapp", "cmd_open_whatsapp"),
    (" дискорд", "cmd_open_discord"), (" discord", "cmd_open_discord"),
    (" инстаграм", "cmd_open_instagram"), (" instagram", "cmd_open_instagram"),
    (" тикток", "cmd_open_tiktok"), (" tiktok", "cmd_open_tiktok"),
    (" твич", "cmd_open_twitch"), (" twitch", "cmd_open_twitch"),
    (" вконтакт", "cmd_open_vk"), (" вк ", "cmd_open_vk"),
    (" гитхаб", "cmd_open_github"), (" github", "cmd_open_github"),
    (" гмейл", "cmd_open_gmail"), (" gmail", "cmd_open_gmail"),
    (" стим ", "cmd_open_steam"), (" steam", "cmd_open_steam"),
    (" блокнот", "cmd_open_notepad"), (" пейнт", "cmd_open_paint"), (" paint", "cmd_open_paint"),
    (" калькулятор", "cmd_open_calculator"), (" проводник", "cmd_open_explorer"),
    (" терминал", "cmd_open_terminal"), (" командную строк", "cmd_open_terminal"), (" powershell", "cmd_open_terminal"),
    (" эксель", "cmd_open_excel"), (" excel", "cmd_open_excel"), (" ворд", "cmd_open_word"), (" word", "cmd_open_word"),
    (" пауэрпоинт", "cmd_open_powerpoint"), (" powerpoint", "cmd_open_powerpoint"),
    (" обс ", "cmd_open_obs"), (" obs", "cmd_open_obs"),
    (" чатгпт", "cmd_open_chatgpt"), (" chatgpt", "cmd_open_chatgpt"), (" клод", "cmd_open_claude"), (" claude", "cmd_open_claude"),
    (" джемини", "cmd_open_gemini"), (" gemini", "cmd_open_gemini"), (" дипсик", "cmd_open_deepseek"), (" deepseek", "cmd_open_deepseek"),
    (" курсор", "cmd_open_cursor"), (" cursor", "cmd_open_cursor"),
    (" диспетчер", "cmd_open_task_manager"),
    (" настройки звук", "cmd_open_sound_settings"), (" параметры звук", "cmd_open_sound_settings"),
    (" настройки экран", "cmd_open_display_settings"), (" параметры экран", "cmd_open_display_settings"), (" настройки дисплея", "cmd_open_display_settings"),
    (" блютуз", "cmd_open_bluetooth"), (" bluetooth", "cmd_open_bluetooth"),
    (" вайфай", "cmd_open_wifi"), (" вай фай", "cmd_open_wifi"), (" wifi", "cmd_open_wifi"), (" wi fi", "cmd_open_wifi"),
    (" обновлени", "cmd_open_update"), (" автозагрузк", "cmd_open_startup_apps"), (" хранилищ", "cmd_open_storage"),
    (" о системе", "cmd_open_about_pc"), (" о компьютере", "cmd_open_about_pc"), (" ночной свет", "cmd_open_night_light"), (" эмодзи", "cmd_open_emoji_panel"),
    (" параметры", "cmd_open_settings"), (" настройки виндовс", "cmd_open_settings"), (" настройки windows", "cmd_open_settings"),
    (" камер", "cmd_open_camera"), (" календар", "cmd_open_calendar"), (" карты", "cmd_open_maps"),
    (" загрузки", "cmd_open_downloads"), (" документы", "cmd_open_documents"), (" рабочий стол", "cmd_open_desktop_folder"),
    (" корзин", "cmd_open_recycle_bin"), (" скриншот", "cmd_open_screenshots_folder"),
    (" авито", "cmd_open_avito"), (" озон", "cmd_open_ozon"), (" вайлдберриз", "cmd_open_wildberries"), (" wildberries", "cmd_open_wildberries"),
    (" кинопоиск", "cmd_open_kinopoisk"), (" иви ", "cmd_open_ivi"), (" нетфликс", "cmd_open_netflix"), (" netflix", "cmd_open_netflix"),
    (" рутуб", "cmd_open_rutube"), (" пинтерест", "cmd_open_pinterest"), (" зум ", "cmd_open_zoom"), (" zoom", "cmd_open_zoom"),
    (" тимс", "cmd_open_teams"), (" teams", "cmd_open_teams"), (" ноушен", "cmd_open_notion"), (" notion", "cmd_open_notion"),
    (" новости", "cmd_open_news"), (" переводчик", "cmd_open_translate"),
];

/// Who plays a clip: the commands of the table that use it (by id), and the phrases of «открой …» that choose it. For the list of the clips of a pack on the page.
pub fn users_of(clip: &str) -> Vec<String> {
    let mut v: Vec<String> = FIXED.iter().filter(|(_, c)| c.contains(&clip)).map(|(id, _)| id.to_string()).collect();
    for (w, c) in WORDS {
        if *c == clip {
            v.push(format!("«открой{}»", w.trim_end()));
        }
    }
    v
}

// the phrase as one lower-case line with single spaces, with a space before and after (so that « вк » and the beginnings of words can be told)
fn normalize(said: &str) -> String {
    let mut s = String::from(" ");
    let mut gap = true;
    for ch in said.to_lowercase().chars() {
        let ch = if ch == 'ё' { 'е' } else { ch };
        if ch.is_alphanumeric() {
            s.push(ch);
            gap = false;
        } else if !gap {
            s.push(' ');
            gap = true;
        }
    }
    if !gap {
        s.push(' ');
    }
    s
}

/// The clips that belong to a command (chosen by its id and, for «open …» commands, by what was named). None = the command has no clip of its own.
pub fn clips_for(command_id: &str, said: &str) -> Option<Vec<&'static str>> {
    if let Some((_, c)) = FIXED.iter().find(|(id, _)| *id == command_id) {
        return Some(c.to_vec());
    }
    if BY_WORDS.contains(&command_id) {
        let p = normalize(said);
        if let Some((_, clip)) = WORDS.iter().find(|(w, _)| p.contains(w)) {
            return Some(vec![clip]);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../resources")
    }

    // every clip of the table exists in the packs that were recorded for the commands (Luna 2.0 and Jarvis New), so that a typo in a name cannot stay silent
    #[test]
    fn every_clip_of_the_table_exists_in_the_recorded_packs() {
        let mut names: Vec<&str> = FIXED.iter().flat_map(|(_, c)| c.iter().copied()).collect();
        names.extend(WORDS.iter().map(|(_, c)| *c));
        for pack in ["luna-2", "jarvis-remaster"] {
            let dir = root().join("sound/voices").join(pack).join("ru");
            for n in &names {
                // the jokes are recorded for Jarvis New only (a pack without a clip keeps its general reaction)
                if n.starts_with("joke") && pack == "luna-2" {
                    continue;
                }
                assert!(dir.join(format!("{}.mp3", n)).exists(), "{} has no clip {}", pack, n);
            }
        }
    }

    // every command id of the table is a real command (a renamed command must not lose its clip unnoticed)
    #[test]
    fn every_command_of_the_table_exists() {
        let mut ids = HashSet::new();
        for entry in std::fs::read_dir(root().join("commands")).unwrap().flatten() {
            if let Ok(t) = std::fs::read_to_string(entry.path().join("command.toml")) {
                for l in t.lines() {
                    if let Some(rest) = l.trim().strip_prefix("id") {
                        if let Some(v) = rest.trim_start().strip_prefix('=') {
                            ids.insert(v.trim().trim_matches('"').to_string());
                        }
                    }
                }
            }
        }
        assert!(ids.len() > 100, "the commands were not read: {}", ids.len());
        for (id, _) in FIXED {
            assert!(ids.contains(*id), "the command {} of the table does not exist", id);
        }
        for id in BY_WORDS {
            assert!(ids.contains(*id), "the command {} of the table does not exist", id);
        }
    }

    #[test]
    fn a_fixed_command_gets_its_clip() {
        assert_eq!(clips_for("assistant_volume_up", ""), Some(vec!["cmd_volume_up"]));
        assert_eq!(clips_for("windows_4", "сделай скриншот"), Some(vec!["cmd_screenshot"]));
        assert_eq!(clips_for("assistant_greet", "привет").map(|c| c.len()), Some(2));
        assert_eq!(clips_for("jarvis_4", "расскажи анекдот").map(|c| c.len()), Some(8));
    }

    #[test]
    fn an_open_command_gets_the_clip_of_what_was_named() {
        assert_eq!(clips_for("assistant_program_open", "Открой Телеграм"), Some(vec!["cmd_open_telegram"]));
        assert_eq!(clips_for("assistant_website", "открой сайт ютуб"), Some(vec!["cmd_open_youtube"]));
        assert_eq!(clips_for("assistant_website", "открой ютуб музыку"), Some(vec!["cmd_open_youtube_music"]));
        assert_eq!(clips_for("assistant_website", "открой яндекс музыку"), Some(vec!["cmd_open_yandex_music"]));
        assert_eq!(clips_for("assistant_website", "открой яндекс"), Some(vec!["cmd_open_yandex"]));
        assert_eq!(clips_for("assistant_place_open", "открой загрузки"), Some(vec!["cmd_open_downloads"]));
        assert_eq!(clips_for("assistant_program_open", "открой настройки звука"), Some(vec!["cmd_open_sound_settings"]));
        assert_eq!(clips_for("assistant_program_open", "открой вк"), Some(vec!["cmd_open_vk"]));
    }

    #[test]
    fn an_unknown_thing_or_command_has_no_clip() {
        assert_eq!(clips_for("assistant_program_open", "открой неизвестную программу"), None);
        assert_eq!(clips_for("assistant_sys_cpu", "загрузка процессора"), None);
        // a word inside another word is not a name («вкус» is not «вк»)
        assert_eq!(clips_for("assistant_program_open", "открой вкусную еду"), None);
    }

    #[test]
    fn the_users_of_a_clip_are_listed() {
        assert_eq!(users_of("cmd_volume_up"), vec!["assistant_volume_up".to_string()]);
        let u = users_of("cmd_open_telegram");
        assert!(u.iter().any(|x| x.contains("телеграм")) && u.iter().any(|x| x.contains("telegram")), "{:?}", u);
        assert!(users_of("ok1").is_empty());
    }
}
