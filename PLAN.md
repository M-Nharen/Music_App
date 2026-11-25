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
| 2 | Chords + Melody layout: diatonic chord keys (I–vii°, borrowed chords), modifiers (7th/sus4/add9/major↔minor on Shift/Alt/Ctrl and 1–4), scale lock, smart voice-leading, strum, arpeggiator, auto-accompaniment patterns (text files), Isomorphic + Drum layouts, layout switcher (F1) | **done** (branch `part-2-chords`) |
| 3 | Learn mode: theory overlay, lessons (notes → intervals → scales → chords → progressions → circle of fifths → modes), ear-training games | next |
| 4 | Recording + looper, MIDI export, virtual MIDI out (ALSA seq / PipeWire MIDI), more instruments, metronome | |
| 5 | Song mode from MIDI files: falling notes labelled with keyboard keys, chord sheet, wait mode, loop + slow down, scoring, one-key mode, auto song layout, simplify/transpose | |
| 6 | Python pipeline: Demucs stems → extraction modes (chords only / melody only via CREPE / chords+melody / full via Basic Pitch / single instrument incl. piano model + drums / simplified) + confidence highlighting + note editor. Live listen mode | |
| 7 | Jam mode: stem mixer with your part muted, time-stretch, Bluetooth delay compensation of backing + visuals | |
| 8 | Spotify: PKCE login, search, remote control, now-playing → open practice version, sync mode | |
| 9 | Polish: themes, stats, streaks, more lessons | |

## Notes for the next session

### From Part 2

- **Layouts are just keymaps.** Piano / Chords + Melody / Isomorphic / Drums are built-in keymap files (`Keymap::builtin`, `BUILTIN_LAYOUTS`), written to `~/.config/qwertone/keymaps/` if missing. The Play tab decides what panels to show from the actions a keymap contains (`Keymap::has_kind`), so a user's custom layout that mixes chords and drums just works.
- **Keys are `KeyId`** (`Key(egui::Key)` | `Shift` | `Ctrl` | `Alt`). egui reports modifiers only as state, so `raw_input_hook` turns `Event::ModifiersChanged` (and each key event's `modifiers`) into press/release of those KeyIds via `sync_modifiers`. Left/right can't be told apart. Ctrl/Alt + key is passed to egui as a shortcut only if the layout doesn't bind Ctrl/Alt.
- **New actions:** `melody n`, `chord <1-7 | roman numeral>`, `mod 7th|sus4|add9|swap`, `drum <name|num>`, `rhythm`, `style±`, `tempo±`, `scale+`, `scale-lock`, `layout+`. Each needs `parse`, `Display`, `describe`, a `kind()`, and (controls) an entry in `CONTROLS` + an icon in `widgets::control_icon`.
- **Theory** (`theory.rs`): `Scale` (8 scales; pentatonic/blues borrow chords from their parent major/minor), `ChordRef::{Diatonic(1..7), Fixed{root, quality}}`, `build_chord(ref, mods, tonic_pc, scale)` → name + roman numeral + intervals, `voice_chord` (tries every inversion near the centre, minimises movement from the previous voicing, with a pull back to the centre so it never drifts). The key's tonic is `cfg.transpose mod 12`. Chords are voiced around G3 regardless of key; bass is the root in octave 2; melody tonic = base note + 12 (C4 at octave 0).
- **Audio channels:** 0 melody (pedal logic in `Renderer`), 1 chords, 2 bass, 9 drums (GM percussion; rustysynth treats channel 9 as drums). The built-in synth now has a patch per channel and a small synthesised drum kit, so everything works without a SoundFont.
- **Sequencer** (`audio/sequencer.rs`) runs in the audio thread: block / strum / arpeggio / pattern styles, its own sample clock, scheduled note-offs with per-note generation counters (a re-struck note isn't cut by the old note's off). `Renderer::render_block` splits each buffer at the sequencer's next event, so timing is sample-accurate. Commands: `Chord{chord, retrigger, velocity}` (`None` = key released, `retrigger=false` = only a modifier/key/scale changed → only changed notes move), `Style`, `Pattern(Arc)`, `Tempo`, `Rhythm(bool)`, `Drum`, `ChannelProgram`. Stats expose `seq_running/seq_step/...` for the beat dots.
- **Bluetooth "grace" window:** a chord change pressed shortly after a step replays the most recent step that had chord notes, with the new chord. Window = 80 ms + 0.8 × the device's measured latency (max 320 ms), so people reacting to delayed sound still land on the beat. The same idea can line up Part 5's scoring.
- **Patterns** (`audio/patterns.rs`) are TOML grids (one line per part, one char per step). Built-ins are in code; `~/.config/qwertone/patterns/*.toml` add to / override them (first run writes `shuffle.toml` as an example). The UI keeps every `Arc<Pattern>` alive (including replaced ones) so the audio thread never frees memory.
- Latch ("keep playing after I let go", default on) applies to arpeggio/patterns; for block/strum the pedal does the same. A modifier/key change while latched sends `Chord{Some, retrigger:false}` then `Chord{None}`.
- New config keys (all `#[serde(default)]`): `scale`, `scale_lock` (off, so Part 1's piano is unchanged), `show_scale`, `voice_leading`, `chord_bass`, `style`, `tempo`, `strum_ms`, `arp_mode`, `arp_rate`, `arp_octaves`, `drums`, `latch`, `chord_instrument`, `bass_instrument`. Instruments appended to `INSTRUMENTS` (indices of old ones unchanged); `BASS_SOUNDS` is separate.
- Part 1 keymaps get `F1 = layout+` added once (migration in `config::load_keymap`, only for files with no `layout+` text at all).
- Tested headless under Xvfb with an ALSA null device (`pcm.!default { type null }` in `~/.asoundrc`) and xdotool; handy for screenshots in later sessions.
- Ideas not done: count-in, fills / variations per pattern, swing amount, tap tempo, per-pattern default tempo, chord inversion keys, recording the chord progression (Part 4 looper can reuse the `Chord` commands as events).

### From Part 1

- Velocity values live in `ui/app.rs` (`VELOCITIES`). The note base is C3 (`BASE_NOTE = 48`) plus octave × 12 plus transpose.
- `Action` in `keymap.rs` is the place to add new actions (see the Part 2 notes for the full checklist).
- egui's `Key` has no Shift/Ctrl/Alt/CapsLock; solved in Part 2 with `KeyId` (CapsLock still isn't visible to egui).
- The audio thread gets commands through a crossbeam channel. `Renderer::render_block` can be tested offline (see the tests in `audio/mod.rs`).
- The latency test clicks are scheduled in the audio thread; their Instants come back over `Engine::clicks`.
- end4 palette path: `~/.local/state/quickshell/user/generated/colors.json` (snake_case Material keys). Older AGS: `~/.local/state/ags/scss/_material.scss`.
