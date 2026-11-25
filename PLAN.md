# Qwertone plan

This file is the shared memory between build sessions. Each session: read it, build the next part, update the status and notes.

## About the user

- Arch Linux, Hyprland with the end4 (illogical-impulse) rice. Wayland.
- Total beginner at music, so explain things plainly in the app.
- Galaxy Buds Live (SBC/AAC on Linux) and Sony WH-1000XM4 (SBC/AAC/LDAC). Wired earphones planned later. Bluetooth must stay a first-class case.
- Spotify Premium, and the only user (a personal Spotify developer app is fine).
- Wants every key/keybind customisable and the app easy to play.

## Decisions

- **Rust** for the app and audio engine (no GC pauses). **egui/eframe 0.37** for the UI.
- **cpal 0.18 with the native PipeWire backend** (feature `pipewire`, on by default), with ALSA as the fallback. Buffer is a fixed 128 frames by default (2.7 ms at 48 kHz); `node.latency` gets set from it.
- Sound: **rustysynth** SoundFont player (FluidR3 from `soundfont-fluid`), plus a small built-in additive synth as the fallback.
- Input: egui key events, read in `raw_input_hook` before widgets see them. Uses physical keys, ignores key repeat, and releases all notes when the window loses focus. (Option for later: an evdev thread for even lower input latency.)
- Config: plain TOML in `~/.config/qwertone/`, reloaded when the file changes.
- Python is only used for the offline song analysis in Part 6 (Demucs, Basic Pitch, CREPE, chord detection), as a separate tool that writes song files.
- Spotify can't provide audio for splitting into parts (against its terms). Spotify is used for search, remote control, now-playing and sync. Splitting and note extraction run on the user's own audio files or on MIDI.

## Parts

| # | Part | Status |
|---|---|---|
| 1 | Engine, Piano layout, keybind editor + TOML, latency test, Bluetooth device profiles + codec switch, keyboard test, end4 theme, PKGBUILD (`packaging/arch`) | **done** |
| 2 | Chords + Melody layout: diatonic chord keys (I–vii°, borrowed chords), modifiers (7th/sus/add9/minor swap; needs modifier keys in the keymap), scale lock, smart voice-leading, strum, arpeggiator, auto-accompaniment patterns, Isomorphic + Drum layouts | next |
| 3 | Learn mode: theory overlay, lessons (notes → intervals → scales → chords → progressions → circle of fifths → modes), ear-training games | |
| 4 | Recording + looper, MIDI export, virtual MIDI out (ALSA seq / PipeWire MIDI), more instruments, metronome | |
| 5 | Song mode from MIDI files: falling notes labelled with keyboard keys, chord sheet, wait mode, loop + slow down, scoring, one-key mode, auto song layout, simplify/transpose | |
| 6 | Python pipeline: Demucs stems → extraction modes (chords only / melody only via CREPE / chords+melody / full via Basic Pitch / single instrument incl. piano model + drums / simplified) + confidence highlighting + note editor. Live listen mode | |
| 7 | Jam mode: stem mixer with your part muted, time-stretch, Bluetooth delay compensation of backing + visuals | |
| 8 | Spotify: PKCE login, search, remote control, now-playing → open practice version, sync mode | |
| 9 | Polish: themes, stats, streaks, more lessons | |

## Notes for the next session

- Velocity values live in `ui/app.rs` (`VELOCITIES`). The note base is C3 (`BASE_NOTE = 48`) plus octave × 12 plus transpose.
- `Action` in `keymap.rs` is the place to add new actions. Each needs `parse`, `Display`, `describe` and an icon in `widgets::control_icon`.
- egui's `Key` has no Shift/Ctrl/Alt/CapsLock. Part 2's chord modifiers need either the `modifiers` field on key events or a raw-key path. Plan: read `modifiers` from the event and let the chord keymap say which modifier means what.
- The audio thread gets commands through a crossbeam channel. `Renderer::render_block` can be tested offline (see the tests in `audio/mod.rs`).
- The latency test clicks are scheduled in the audio thread; their Instants come back over `Engine::clicks`.
- end4 palette path: `~/.local/state/quickshell/user/generated/colors.json` (snake_case Material keys). Older AGS: `~/.local/state/ags/scss/_material.scss`.
