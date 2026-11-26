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
| 3 | Learn mode: theory overlay, lessons (notes → intervals → scales → chords → progressions → circle of fifths → modes), ear-training games | **done** (branch `part-3-learn`) |
| 4 | Recording + looper, MIDI export, virtual MIDI out (ALSA seq / PipeWire MIDI), more instruments, metronome | next |
| 5 | Song mode from MIDI files: falling notes labelled with keyboard keys, chord sheet, wait mode, loop + slow down, scoring, one-key mode, auto song layout, simplify/transpose | |
| 6 | Python pipeline: Demucs stems → extraction modes (chords only / melody only via CREPE / chords+melody / full via Basic Pitch / single instrument incl. piano model + drums / simplified) + confidence highlighting + note editor. Live listen mode | |
| 7 | Jam mode: stem mixer with your part muted, time-stretch, Bluetooth delay compensation of backing + visuals | |
| 8 | Spotify: PKCE login, search, remote control, now-playing → open practice version, sync mode | |
| 9 | Polish: themes, stats, streaks, more lessons | |

## Notes for the next session

### From Part 3

- **Branching:** `part-3-learn` was built on `part-2-chords`. Part 6 (`analyzer/`, Python) was being built in parallel in another session; Part 3 doesn't touch `analyzer/`, so the two should merge cleanly.
- **Layout:** pure logic lives in `src/learn/` (tested, no egui): `analysis.rs` (overlay), `intervals.rs`, `lessons.rs` (all lesson content as `const` data), `ear.rs` (game questions + scoring), `Rng` (xorshift, no crate). The UI is `src/ui/app/learn.rs`; the circle of fifths is `src/ui/circle.rs`.
- **Overlay** (`analysis::explain(&Heard{melody, chord, tonic_pc, scale})`): uses the chord engine's `Chord` when a chord key sounds (so name/numeral match the key caps), otherwise identifies free-play notes with its own shape table (`SHAPES`: triads, sus, power, 6/7/9 chords; bass note decides C6 vs Am7). `role()` gives the chord's job per key (major vs minor-home scales), `motion()` names cadences between consecutive chords, `spell()` spells borrowed notes as b3/b6/b7. `update_overlay()` runs every frame; when the note count drops (letting go of a chord key by key) it waits 350 ms before replacing the explanation. It shows on the Play tab when `cfg.learn.explain` (toolbar "Explain").
- **Lesson targets are notes/chords, never keys.** `Target::Note(semitones above lesson C)`, `Together(&[..])` (held at once, any octave), `Chord("V")` (roman numeral in C). `resolve_targets()` maps them to keys of the current keymap: exact note first, else the same note name in the nearest octave; chords use a chord key with the same root + family, else fall back to the triad's notes. Unplayable → banner with "Switch to Piano/Chords". "Lesson C" = `BASE_NOTE + 12 × octave` (ignores transpose; there's a "Set key to C major" banner).
- **Lesson text placeholders:** `{n:4}` key for a note, `{c:V}` key for a chord, `{a:<action text>}` keys for any keymap action. `lessons::tests` checks every placeholder and numeral parses. To add a lesson, add a `Lesson` to `CHAPTERS`; ids are saved in config, so don't rename them.
- **Input:** `play_key` pushes `Played::Note` / `Played::Chord{root_pc, family}` into `learn.events` only while the Learn tab is open; the lesson / game consumes them each frame. "Hold together" targets are checked against what's held (`sounding` + chord voicing), and need a fresh key press between two targets (`armed`). F2 / F3 are Learn-tab shortcuts only when the keymap doesn't bind them.
- **Example playback** (`Player` in learn.rs) is timed from the UI thread (`tick_player` in `poll`) using plain NoteOn/NoteOff on channel 0. Good enough for examples; Part 4/5 could move it to the audio-thread sequencer for sample accuracy (and its own channel so it can't cut a note the user holds).
- **Keyboard hints:** `KeyboardView.hints: &HashMap<KeyId, Hint>` (`HintKind::Now` pulses; `Next`, `Option`, `Home`, `Correct`) with a small tag drawn inside the cap. Every other caller passes an empty map. Part 5 (falling notes / wait mode) can reuse this to light the next key. `widgets::key_position` gives a key's row/column.
- **Config:** `[learn]` table (`LearnProgress`: `explain`, `done`, `lesson`, `step`, `best`, `levels`), all `#[serde(default)]`.
- Headless testing as before (Xvfb + ALSA null + xdotool); `xdotool key` presses and releases within one frame, so use `keydown`/`sleep`/`keyup` to see the overlay react.
- Ideas not done: spaced repetition for the ear games (ask missed items more), rhythm lessons (needs Part 4's metronome), sight-reading / staff notation, per-lesson "free play" checks ("play any note of the scale"), a mode drone for the modes lessons, explaining the accompaniment's bass/drums.

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
- **Instrument pictures** (`ui/visuals.rs`): `Visual::for_program` picks keyboard / guitar / strings / flute / waveform from the GM program (drums layout → pads); `cfg.visual` = "auto" or a fixed one. Fretboards use `place_notes` (each note on its own string, smallest hand stretch; more notes than strings drops the lowest). The flute chart is a 6-hole simple-system flute/whistle in C (half-holes + one cross fingering), not a Boehm flute; same holes an octave up. Drum pads flash from `drum_hits` (only keys you hit, not the pattern's drums: those happen in the audio thread; exposing them would need a hit ring-buffer in `Stats`). The waveform is drawn from the note frequencies, not the real audio output.
- **Saved devices** (Audio tab): lists `cfg.devices` with rename (sets `DeviceProfile.label`, which now wins over PipeWire's description everywhere via `device_name`), re-test (only for the connected device) and remove (two clicks). Saving a test result no longer overwrites a custom name.
- Ideas not done: count-in, fills / variations per pattern, swing amount, tap tempo, per-pattern default tempo, chord inversion keys, recording the chord progression (Part 4 looper can reuse the `Chord` commands as events).

### From Part 1

- Velocity values live in `ui/app.rs` (`VELOCITIES`). The note base is C3 (`BASE_NOTE = 48`) plus octave × 12 plus transpose.
- `Action` in `keymap.rs` is the place to add new actions (see the Part 2 notes for the full checklist).
- egui's `Key` has no Shift/Ctrl/Alt/CapsLock; solved in Part 2 with `KeyId` (CapsLock still isn't visible to egui).
- The audio thread gets commands through a crossbeam channel. `Renderer::render_block` can be tested offline (see the tests in `audio/mod.rs`).
- The latency test clicks are scheduled in the audio thread; their Instants come back over `Engine::clicks`.
- end4 palette path: `~/.local/state/quickshell/user/generated/colors.json` (snake_case Material keys). Older AGS: `~/.local/state/ags/scss/_material.scss`.
