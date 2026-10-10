# JARVIS Voice Assistant

Offline voice assistant (Rust). Target platform: **Windows**.

## The Jarvis window

On start Jarvis opens its own window (Edge/Chrome in app mode, no address bar): a glowing orb that reacts to listening/speaking, a chat with everything you say and Jarvis answers, a list of commands (click a phrase to insert it), settings buttons and **a text field at the bottom** for typing commands. Reopen it from the tray menu ("Open window"). Flags: `--background` (no window at start), `--no-ui` (no window at all), `--console` (keep the console window). The page is served on 127.0.0.1 only; the internal WebSocket accepts only the Jarvis window (other web pages are rejected).

## Two modes

| Mode | When | How you talk to it |
|---|---|---|
| **Voice** | microphone + Vosk speech model + wake-word model are available | say "Джарвис, ..." |
| **Text** | no microphone / no speech model, or started with `--text` | type a command in the console and press Enter (`exit` quits) |

The mode is chosen automatically at start-up: if anything needed for voice is missing, Jarvis logs a warning and continues in text mode instead of exiting. Commands can also be sent as text while in voice mode.

If the intent model files are missing, commands are matched by plain text similarity.

## What you can say

Examples (Russian; most have English equivalents). Numbers can be said as words or digits.

| Say | What happens |
|---|---|
| «открой сайт ютуб», «зайди на сайт example точка com» | opens the site in the default browser |
| «найди в гугле погоду», «найди в ютубе ...» | web search |
| «открой блокнот / дискорд / ворд», «закрой дискорд» | opens / closes a program |
| «громкость 50 процентов», «громче», «тише на 20», «выключи звук» | system volume |
| «яркость 70 процентов», «ярче», «темнее» | screen brightness (laptops / built-in screens) |
| «поставь таймер на 5 минут», «отмени таймер», «сколько осталось» | timer, notification + sound when it ends |
| «напомни через 10 минут позвонить маме» | reminder |
| «скажи мне через 5 минут чтобы я закончил игру», «остановись через 5 минут» | warns you when time is up |
| «выключи компьютер через 10 минут», «перезагрузи компьютер», «отмени выключение» | power (no time given = 1 minute, so you can cancel) |

| «разбуди меня завтра в 7:30», «напомни в 18:00 про тренировку» | alarm / reminder at a clock time |
| «я играю, остановись через час», «рабочий режим» | game mode: warns you 5 minutes before the end and when time is up |
| «сколько времени», «какое сегодня число», «сколько заряда» | time, date, battery (spoken) |
| «запиши идею купить молоко», «что я записывал», «открой заметки» | voice notes (`Documents\Jarvis-notes.txt`) |
| «пауза», «следующий трек», «предыдущий трек» | media keys (Spotify, browser, VLC ...) |
| select text with the mouse + «озвучь этот текст», «прочитай буфер обмена», «хватит читать» | reads the text aloud |
| «выключи озвучку» / «включи озвучку» | turns Jarvis' own voice off / on |
| «режим текста» / «режим голоса» (also: tray menu) | microphone off (text only) / on |
| «включи удалённое управление» | phone remote (see below) |

More:

| Say | What happens |
|---|---|
| «найди на моём компьютере файл 123», «найди файл отчёт» | searches the computer in the background (your folders first, then everything), lists the best matches |
| «открой первый результат», «открой второй файл», «покажи в папке» | opens a found file / shows it in Explorer |
| «открой загрузки / документы / рабочий стол / корзину», «открой диск D» | folders and drives |
| «открой настройки звука / экрана / сети / bluetooth / обновления …», «диспетчер устройств», «управление дисками», «службы» | Windows Settings pages and tools |
| «сколько оперативной памяти занято», «загрузка процессора», «свободное место на диске», «какой у меня IP», «сколько работает компьютер» | computer info (spoken) |
| «переведи на английский привет», «найди в википедии …», «покажи на карте Ташкент», «как добраться до вокзала», «включи песню …» | opens the right web page |
| «какая погода в Ташкенте», «установи город Самарканд» | weather (needs internet) |
| «привет», «как дела», «кто ты», «что ты умеешь» | small talk and the list of commands |

Jarvis answers aloud (female voice) and now and then adds a dry remark. Timers and reminders are saved to disk and survive a restart. A scheduled shutdown is handled by Windows itself.

### Voice

Jarvis speaks with the Piper neural voice if `piper\` is installed next to the program (full installer), otherwise with the best female Windows voice of the current language (install a Russian one in Windows: Settings -> Time & language -> Speech). Voices are chosen in `.github/workflows/build-windows.yml` (`PIPER_RU_VOICE`, `PIPER_EN_VOICE`); samples: https://rhasspy.github.io/piper-samples/

### Phone remote

Say «включи удалённое управление»: Jarvis copies a link like `http://192.168.x.x:9713/?t=<secret>` to the clipboard. Open it on a phone in the same Wi-Fi: a page with a text box and quick buttons sends commands to the PC. Off by default, protected by the secret token (do not share the link); Windows may ask once to allow Jarvis through the firewall. «выключи удалённое управление» stops it.

### Start with Windows

Tick "Start Jarvis with Windows" in the installer: it starts in the background (`--background`, no console window) with the tray icon.

### Adding your own commands

`resources/commands/<name>/command.toml`. Besides `phrases` (matched by the intent model) a command can have `triggers`: keyword stems, e.g. `triggers.ru = ["таймер"]`, `["выключ комп"]` (all stems must begin some word of the phrase; the longest match wins; checked before the intent model). Lua scripts get `jarvis.parse.number(text)` / `jarvis.parse.duration(text)`, `jarvis.timer.after(...)`, `jarvis.system.run(...)`, see `resources/commands/assistant/*.lua`.

## Run

```
cargo build --release -p jarvis-app
python post_build.py        # copies resources/ and DLLs next to the executable
target\release\jarvis-app.exe          # voice (falls back to text)
target\release\jarvis-app.exe --text   # text only
```

## Notes

- Vosk models (~419 MB) are not stored in git. The full installer already contains the Russian model (the CI takes it from the `v0.1` release archive, see `VOSK_MODELS` in the workflow); for the lite installer or other languages see `resources/vosk/README.md`.
- Commands live in `resources/commands/*/command.toml`.
- `ahk` commands need the `.ahk` files in `resources/commands/*/ahk/` compiled to `.exe` with AutoHotkey.

## License

GPL-3.0, see LICENSE.txt. Based on https://github.com/Priler/jarvis
