# JARVIS Voice Assistant

Offline voice assistant (Rust). Target platform: **Windows**.

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

Timers and reminders live in memory: they are lost if Jarvis is closed. A scheduled shutdown is handled by Windows itself.

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

- Vosk models (~419 MB) are not stored in git, see `resources/vosk/README.md`.
- Commands live in `resources/commands/*/command.toml`.
- `ahk` commands need the `.ahk` files in `resources/commands/*/ahk/` compiled to `.exe` with AutoHotkey.

## License

GPL-3.0, see LICENSE.txt. Based on https://github.com/Priler/jarvis
