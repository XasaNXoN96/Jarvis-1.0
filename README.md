# JARVIS Voice Assistant

Offline voice assistant (Rust). Target platform: **Windows**.

## Two modes

| Mode | When | How you talk to it |
|---|---|---|
| **Voice** | microphone + Vosk speech model + wake-word model are available | say "Джарвис, ..." |
| **Text** | no microphone / no speech model, or started with `--text` | type a command in the console and press Enter (`exit` quits) |

The mode is chosen automatically at start-up: if anything needed for voice is missing, Jarvis logs a warning and continues in text mode instead of exiting. Commands can also be sent as text while in voice mode.

If the intent model files are missing, commands are matched by plain text similarity.

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
