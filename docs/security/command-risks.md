# Риски команд Jarvis

Файл создан по `resources/commands/*/command.toml` и скриптам. Поле `risk` — это **потолок**: операция опаснее, чем объявлена командой, отклоняется (`crates/jarvis-core/src/policy.rs`). Команда без `risk` считается `low`.

«Что может скрипт» — это функции, которые скрипт содержит (общий скрипт обслуживает несколько команд, поэтому у соседних команд список шире, чем реально делает конкретная ветка). Реальный потолок задаёт `risk`.

| риск | пакет | команда | тип | скрипт | что может скрипт |
|---|---|---|---|---|---|
| high | assistant | `assistant_game_off` | lua | game.lua | запуск программы |
| high | assistant | `assistant_power_hibernate` | lua | power.lua | запуск программы |
| high | assistant | `assistant_power_logoff` | lua | power.lua | запуск программы |
| high | assistant | `assistant_power_off` | lua | power.lua | запуск программы |
| high | assistant | `assistant_power_restart` | lua | power.lua | запуск программы |
| high | assistant | `assistant_program_force_close` | lua | program.lua | запуск программы |
| high | assistant | `assistant_remote_on` | lua | remote.lua | — |
| high | assistant | `assistant_session` | lua | office.lua | PowerShell (известные скрипты), PowerShell -File (свой .ps1), ввод текста, запись файла, запуск программы, окна |
| high | assistant | `assistant_task_clear` | lua | tasks.lua | запись файла, открытие |
| medium | browser | `browser_close` | lua | actions.lua | запуск программы, открытие |
| high | calculator | `calculator_2` | cli |  | запуск программы: taskkill /f /im CalculatorApp.exe |
| high | steam | `steam_2` | lua | actions.lua | запуск программы, открытие |
| high | windows | `windows_2` | lua | actions.lua | PowerShell (известные скрипты), запуск программы, нажатие клавиш, открытие |
| high | windows | `windows_6` | lua | actions.lua | PowerShell (известные скрипты), запуск программы, нажатие клавиш, открытие |
| medium | assistant | `assistant_apps_rescan` | lua | program.lua | запуск программы |
| medium | assistant | `assistant_program_close` | lua | program.lua | запуск программы |
| medium | assistant | `assistant_brightness_down` | lua | brightness.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_brightness_set` | lua | brightness.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_brightness_up` | lua | brightness.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_browser_set` | lua | extra.lua | копирование выделенного, открытие |
| medium | assistant | `assistant_disk_open` | lua | places.lua | запуск программы, открытие |
| medium | assistant | `assistant_file_cancel` | lua | files.lua | — |
| medium | assistant | `assistant_file_folder` | lua | files.lua | — |
| medium | assistant | `assistant_file_more` | lua | files.lua | — |
| medium | assistant | `assistant_file_next` | lua | files.lua | — |
| medium | assistant | `assistant_file_open` | lua | files.lua | — |
| medium | assistant | `assistant_file_search` | lua | files.lua | — |
| medium | assistant | `assistant_file_widen` | lua | files.lua | — |
| medium | assistant | `assistant_info_battery` | lua | info.lua | PowerShell (известные скрипты) |
| medium | assistant | `assistant_maps` | lua | web2.lua | открытие |
| medium | assistant | `assistant_media_next` | lua | media.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_media_playpause` | lua | media.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_media_prev` | lua | media.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_mic_on` | lua | mic.lua | — |
| medium | assistant | `assistant_music_search` | lua | web2.lua | открытие |
| medium | assistant | `assistant_note_add` | lua | notes.lua | запись файла, открытие |
| medium | assistant | `assistant_note_open` | lua | notes.lua | запись файла, открытие |
| medium | assistant | `assistant_note_read` | lua | notes.lua | запись файла, открытие |
| medium | assistant | `assistant_object_start` | lua | office.lua | PowerShell (известные скрипты), PowerShell -File (свой .ps1), ввод текста, запись файла, запуск программы, окна |
| medium | assistant | `assistant_office_cell` | lua | office.lua | PowerShell (известные скрипты), PowerShell -File (свой .ps1), ввод текста, запись файла, запуск программы, окна |
| medium | assistant | `assistant_office_dictate` | lua | office.lua | PowerShell (известные скрипты), PowerShell -File (свой .ps1), ввод текста, запись файла, запуск программы, окна |
| medium | assistant | `assistant_office_new` | lua | office.lua | PowerShell (известные скрипты), PowerShell -File (свой .ps1), ввод текста, запись файла, запуск программы, окна |
| medium | assistant | `assistant_place_open` | lua | places.lua | запуск программы, открытие |
| medium | assistant | `assistant_power_cancel` | lua | power.lua | запуск программы |
| medium | assistant | `assistant_program_open` | lua | program.lua | запуск программы |
| medium | assistant | `assistant_read_clipboard` | lua | speech.lua | запись в буфер обмена, копирование выделенного |
| medium | assistant | `assistant_read_selection` | lua | speech.lua | запись в буфер обмена, копирование выделенного |
| medium | assistant | `assistant_screen` | lua | screen.lua | — |
| medium | assistant | `assistant_selftest` | lua | selftest.lua | PowerShell (известные скрипты), PowerShell -File (свой .ps1), запись файла |
| medium | assistant | `assistant_speedtest` | lua | extra.lua | копирование выделенного, открытие |
| medium | assistant | `assistant_sys_cpu` | lua | sysinfo.lua | PowerShell (известные скрипты) |
| medium | assistant | `assistant_sys_disk` | lua | sysinfo.lua | PowerShell (известные скрипты) |
| medium | assistant | `assistant_sys_ip` | lua | sysinfo.lua | PowerShell (известные скрипты) |
| medium | assistant | `assistant_sys_ram` | lua | sysinfo.lua | PowerShell (известные скрипты) |
| medium | assistant | `assistant_sys_uptime` | lua | sysinfo.lua | PowerShell (известные скрипты) |
| medium | assistant | `assistant_task_add` | lua | tasks.lua | запись файла, открытие |
| medium | assistant | `assistant_task_done` | lua | tasks.lua | запись файла, открытие |
| medium | assistant | `assistant_task_open` | lua | tasks.lua | запись файла, открытие |
| medium | assistant | `assistant_task_read` | lua | tasks.lua | запись файла, открытие |
| medium | assistant | `assistant_translate` | lua | web2.lua | открытие |
| medium | assistant | `assistant_translate_clip` | lua | extra.lua | копирование выделенного, открытие |
| medium | assistant | `assistant_volume_down` | lua | volume.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_volume_mute` | lua | volume.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_volume_set` | lua | volume.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_volume_unmute` | lua | volume.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_volume_up` | lua | volume.lua | PowerShell -File (свой .ps1) |
| medium | assistant | `assistant_website` | lua | website.lua | открытие |
| medium | assistant | `assistant_wiki` | lua | web2.lua | открытие |
| medium | browser | `browser_open` | lua | actions.lua | запуск программы, открытие |
| medium | browser | `open_google` | lua | actions.lua | запуск программы, открытие |
| medium | calculator | `calculator_1` | cli |  | запуск программы: calc  |
| medium | input | `key_clipboard` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_copy` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_cut` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_enter` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_escape` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_page_end` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_page_top` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_paste` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_print` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_redo` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_save` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_select_all` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `key_undo` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_click` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_double` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_down` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_left` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_right` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_right_move` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_scroll_down` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_scroll_up` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `mouse_up` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_address` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_back` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_bookmark` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_downloads` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_find` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_forward` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_history` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_incognito` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_refresh` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_tab_close` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_tab_new` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_tab_next` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_tab_prev` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_tab_reopen` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `web_window_new` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `win_maximize` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `win_minimize` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `win_restore` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | input | `win_switch` | lua | input.lua | мышь, нажатие клавиш, окна |
| medium | steam | `steam_1` | lua | actions.lua | запуск программы, открытие |
| medium | terminate | `terminate_1` | terminate |  | завершает Jarvis |
| medium | windows | `windows_1` | lua | actions.lua | PowerShell (известные скрипты), запуск программы, нажатие клавиш, открытие |
| medium | windows | `windows_3` | lua | actions.lua | PowerShell (известные скрипты), запуск программы, нажатие клавиш, открытие |
| medium | windows | `windows_4` | lua | actions.lua | PowerShell (известные скрипты), запуск программы, нажатие клавиш, открытие |
| medium | windows | `windows_5` | lua | actions.lua | PowerShell (известные скрипты), запуск программы, нажатие клавиш, открытие |
| medium | windows | `windows_8` | lua | actions.lua | PowerShell (известные скрипты), запуск программы, нажатие клавиш, открытие |
| medium | windows | `windows_9` | lua | actions.lua | PowerShell (известные скрипты), запуск программы, нажатие клавиш, открытие |
| low | assistant | `assistant_chat_end` | lua | chat.lua | — |
| low | assistant | `assistant_chat_start` | lua | chat.lua | — |
| low | assistant | `assistant_game_on` | lua | game.lua | запуск программы |
| low | assistant | `assistant_greet` | lua | smalltalk.lua | — |
| low | assistant | `assistant_help` | lua | smalltalk.lua | — |
| low | assistant | `assistant_howareyou` | lua | smalltalk.lua | — |
| low | assistant | `assistant_info_date` | lua | info.lua | PowerShell (известные скрипты) |
| low | assistant | `assistant_info_time` | lua | info.lua | PowerShell (известные скрипты) |
| low | assistant | `assistant_mic_off` | lua | mic.lua | — |
| low | assistant | `assistant_phone_find` | lua | phone.lua | — |
| low | assistant | `assistant_phone_link` | lua | phone.lua | — |
| low | assistant | `assistant_phone_stop` | lua | phone.lua | — |
| low | assistant | `assistant_phone_text` | lua | phone.lua | — |
| low | assistant | `assistant_reminder` | lua | reminder.lua | — |
| low | assistant | `assistant_remote_off` | lua | remote.lua | — |
| low | assistant | `assistant_timer_cancel` | lua | timer.lua | — |
| low | assistant | `assistant_timer_set` | lua | timer.lua | — |
| low | assistant | `assistant_timer_status` | lua | timer.lua | — |
| low | assistant | `assistant_tts_off` | lua | speech.lua | запись в буфер обмена, копирование выделенного |
| low | assistant | `assistant_tts_on` | lua | speech.lua | запись в буфер обмена, копирование выделенного |
| low | assistant | `assistant_tts_stop` | lua | speech.lua | запись в буфер обмена, копирование выделенного |
| low | assistant | `assistant_tts_test` | lua | speech.lua | запись в буфер обмена, копирование выделенного |
| low | assistant | `assistant_unlock` | lua | unlock.lua | — |
| low | assistant | `assistant_who` | lua | smalltalk.lua | — |
| low | counter | `counter` | lua | script.lua | — |
| low | jarvis | `jarvis_2` | voice |  | — |
| low | jarvis | `jarvis_3` | voice |  | — |
| low | jarvis | `jarvis_4` | voice |  | — |
| low | jarvis | `jarvis_5` | voice |  | — |
| low | stop | `stop_1` | stop_chaining |  | — |
| low | weather | `set_city` | lua | set_city.lua | — |
| low | weather | `weather` | lua | script.lua | — |
