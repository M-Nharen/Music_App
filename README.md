# Qwertone

Play music on your laptop keyboard. Built for Arch + Hyprland (end4 rice), works over Bluetooth earbuds, and lets you remap every key.

> **Status: Part 3 of 9.** Low-latency engine, four layouts (Piano, Chords + Melody, Isomorphic, Drums), auto-accompaniment, Learn mode (theory overlay, 23 lessons, ear-training games), keybind editor, latency tools, keyboard test, end4 theme. See [PLAN.md](PLAN.md) for what's next.

## Install (Arch)

```bash
git clone https://github.com/M-Nharen/Music_App.git qwertone
cd qwertone
./scripts/setup-arch.sh        # packages, realtime group, PipeWire low-latency config
cargo run --release            # first build takes a few minutes
```

Or install it like a normal package (adds a launcher entry too):

```bash
cd packaging/arch
makepkg -si
qwertone
```

The piano sound comes from `soundfont-fluid` (the setup script installs it). Without it you get a simple built-in synth.

### Hyprland

Add to `~/.config/hypr/hyprland.conf` (or your end4 custom rules file):

```ini
# float it at a nice size, slightly see-through so end4's blur shows
windowrulev2 = float, class:^(qwertone)$
windowrulev2 = size 1180 820, class:^(qwertone)$
windowrulev2 = opacity 0.96 0.92, class:^(qwertone)$
```

Colours follow end4's Material You palette automatically (from `~/.local/state/quickshell/user/generated/colors.json`, or the older AGS `_material.scss`) and change when your wallpaper does.

## Playing

Pick a layout at the top of the **Play** tab, or press `F1` to cycle through them.

### Piano

| Keys | What they do |
|---|---|
| `Z X C V B N M , . /` | White keys, lower octave (starts at C3) |
| `S D G H J L ;` | Black keys, lower octave |
| `Q W E R T Y U I O P [ ]` | White keys, upper octave (starts at C4) |
| `2 3 5 6 7 9 0 =` | Black keys, upper octave |
| `Space` | Sustain pedal (hold) |
| `↑ / ↓` | Octave up / down |
| `← / →` | Change key (transpose by a semitone) |
| `PgUp / PgDn` | Next / previous instrument |
| `Tab` | Soft / normal / loud |
| `Esc` | Stop all sound |

The big readout names whatever chord you're holding (try `Z C B` = C major, `N Q E` = A minor).

### Chords + Melody (easiest)

Left hand plays a whole chord with one key, right hand plays a melody that can't hit a wrong note.

| Keys | What they do |
|---|---|
| `Z X C V B N M` | The 7 chords of the key: I ii iii IV V vi vii°. In C major: C Dm Em F G Am Bdim |
| `A S D F G H J` | "Borrowed" chords for extra colour: i II bIII iv III bVI bVII (in C: Cm D Eb Fm E Ab Bb) |
| hold `Shift` or `1` | add the 7th (G → G7) |
| hold `Alt` or `2` | sus4 (C → Csus4) |
| hold `3` | add9 |
| hold `Ctrl` or `4` | swap major ↔ minor (Am → A) |
| `Q W E R T Y U I O P [ ]` | Melody, locked to the scale |
| `Enter` | Start / stop the rhythm |
| `Backspace` | Next style (block → strum → arpeggio → patterns) |
| `-` / `=` | Slower / faster |
| `\` | Next scale (major, minor, dorian, blues…) |
| `← / →` | Change key |
| `Space` | Pedal: holds the chord |

The key caps show the actual chord names and numerals, and they change live while you hold a modifier. Try `Z B N V` (C G Am F), the chords of hundreds of pop songs.

**Styles** (Style menu or `Backspace`):

- **Block chords**: the chord sounds while you hold the key.
- **Strum**: notes roll in low to high like a guitar (speed adjustable).
- **Arpeggio**: the chord's notes one at a time, up / down / up-and-down / random, in 8ths, triplets or 16ths, over 1–3 octaves.
- **Patterns**: a backing band. Pop, Ballad, Country, Rock, Waltz, Reggae, Bossa nova, Dance, Alberti and Shuffle, with bass and drums. Tap a chord once and it keeps playing ("Keep playing after I let go"); press another chord to change, `Enter` to stop.

**Smooth chords** (on by default) picks the inversion that moves your hand least, like a pianist does: C → F plays C-F-A instead of jumping up to F-A-C.

**Bluetooth:** the rhythm keeps perfect time inside the app, but you hear it ~200 ms late, so you'll naturally press chord changes a little late. Qwertone uses your measured latency (Audio tab) to let a chord pressed just after the beat still land on that beat.

### Isomorphic

Every key to the right is a whole step, up-left is a fourth, up-right is a fifth. Every chord and scale has the same shape in every key: a major chord is always "this key, two keys right, and the key up-right" (`Z C S` = C E G).

### Drums

Bottom row kicks, snares, claps; home row hi-hats, ride, congas; Q-row toms and crashes; Space is a kick. `Enter` plays the current pattern's beat so you can drum along.

### The picture under the keyboard

It follows your instrument and lights up whatever is playing (melody in one colour, chord in another):

| Instrument | Picture |
|---|---|
| Pianos, organ, electric piano, mallets, harp | Keyboard |
| Guitars | Guitar fretboard (standard tuning), with a playable hand position for each chord |
| Strings | Violin or cello fingerboard (cello when the music is low), with beginner finger-tape lines |
| Flute | 6-hole fingering diagram (● covered, ◐ half, ○ open) for the note you're playing |
| Drums layout | Drum pads that flash when hit |
| Pads, synths, choir | A glowing waveform of the notes |

The **View** menu above it lets you pick any picture instead. Dots on the fretboards mark the notes of the current scale.

### Key and scale

The **Key** and **Scale** pickers on the Play tab apply to every layout. Keys get a • if their note is in the scale and a ★ on the key's home note. **Scale lock** makes piano/isomorphic keys play the nearest scale note instead, so nothing sounds wrong.

## Learn

The **Learn** tab teaches music from zero, on your own keyboard and keybinds.

**What you're hearing.** Whatever you play gets explained: the chord's name, its roman numeral in the key (V7 in C major), its job (home, away, tension), why it sounds happy, sad or tense, a Rest ↔ Tension meter, and how the move from the last chord sounds ("V → I: tension → release"). Two notes are named as an interval with songs that start with it. The same panel shows on the Play tab (untick **Explain** to hide it).

**Lessons.** 23 short, hands-on lessons in 7 chapters:

1. Notes & octaves
2. Intervals (each with famous songs that start with it)
3. Major, minor and pentatonic scales
4. Building chords: triads, major vs minor, dim/aug/sus, the 7 chords of a key, 7ths
5. Progressions: home/away/tension, cadences, I–V–vi–IV, ii–V–I, borrowed chords
6. The circle of fifths (drawn next to the lesson and following your chords)
7. Modes

Lessons light up the keys to press in your current layout, numbered in order, and tick them off as you play. If your layout can't play a step (say, a black key in Chords + Melody), the lesson offers to switch. Lessons are written in C major; if you're in another key there's a one-click reset. Progress is saved.

**Ear training.** Four games, three levels each: *Find the note*, *Intervals*, *Happy or sad?* and *Name that chord*. You answer by playing: the reference key ("home", "1st") and, on easier levels, the possible answers are lit. Best streaks are saved.

On the Learn tab **F2** plays the example / question again and **F3** goes to the next step (unless your layout uses those keys).

## Keybinds

Open the **Keybinds** tab:

- Click any key on the drawn keyboard to set what it does (a note, a control, or nothing).
- Or press **Rebind** next to an action and then press the new key. If that key was already in use, the two keys swap, so nothing loses its key without you noticing.
- **Save as new layout** makes a copy you can change freely. **Reset to Piano** restores the defaults.

Layouts are plain text files in `~/.config/qwertone/keymaps/`. Edit them in any editor; the app reloads them as soon as you save:

```toml
name = "My layout"
[bindings]
Z = "note 0"      # semitones above the lowest C (C3 at octave 0)
S = "note 1"
Space = "sustain"
Up = "octave+"
```

Shift, Ctrl and Alt can be bound too (`Shift = "mod 7th"`). Click them on the drawn keyboard like any other key.

Actions:

| Action | Meaning |
|---|---|
| `note <n>` | A fixed note, `n` semitones above the layout's lowest C |
| `melody <n>` | Scale step `n` above the key's home note (always in key) |
| `chord <1-7>` | The chord on that scale step |
| `chord <numeral>` | A fixed chord: `bVII`, `bVI`, `bIII`, `iv`, `II`, `III`, `v`, `vii°`, `III+` … (upper case = major, lower = minor) |
| `mod 7th` / `mod sus4` / `mod add9` / `mod swap` | Hold to change the chord |
| `drum <name>` | `kick`, `snare`, `clap`, `rim`, `hat`, `pedalhat`, `openhat`, `ride`, `bell`, `crash`, `splash`, `china`, `lowtom`, `midtom`, `hitom`, `floortom`, `tamb`, `shaker`, `cowbell`, `claves`, `conga`, `bongo` … or a General MIDI number |
| `sustain`, `octave+`, `octave-`, `transpose+`, `transpose-`, `instrument+`, `instrument-`, `velocity`, `panic` | As in Part 1 |
| `rhythm`, `style+`, `style-`, `tempo+`, `tempo-`, `scale+`, `scale-lock`, `layout+` | Accompaniment and layout controls |

**Reset to…** restores any layout to one of the four built-in ones.

### Your own accompaniment patterns

Patterns are text files in `~/.config/qwertone/patterns/` (there's an example, `shuffle.toml`). One line per part, one character per step, like a drum machine:

```toml
name = "My groove"
beats = 4
steps_per_beat = 4          # 16th notes
[tracks]
bass  = "x-------x---5---"  # x root, 5 fifth, 8 octave, - hold
chord = "..x...x...x...x."  # x whole chord, 1-9 single chord notes
kick  = "x.......x.x....."
snare = "....x.......x..."
hat   = "x.x.x.x.x.x.x.x."  # X loud, o soft
```

Then **Play → Sounds & patterns → Reload patterns**.

Settings live in `~/.config/qwertone/config.toml` and reload on save too (key, scale, style, tempo, chord and bass sounds are all in there).

## Bluetooth earbuds (Galaxy Buds Live, Sony XM4…)

Bluetooth adds roughly 150–250 ms of delay. That comes from the earbuds and the Bluetooth codec, not from Qwertone. To get the most out of it:

1. **Audio & latency → Output.** If a lower-delay codec is available, click **Switch to a2dp-sink-sbc**. SBC has less delay than AAC or LDAC (LDAC on the XM4 has the most).
2. **Run the latency test** with each pair and press **Save**. Later parts use this number to line up songs and falling notes with what you actually hear.
   Every saved pair appears under **Saved devices** (name, Bluetooth or wired, delay, instrument, and whether it's connected now), where you can rename, re-test or remove it.
3. **Choose an instrument per device.** Sounds marked ✦ (pads, strings, organ, choir) have soft attacks that hide the delay. Qwertone switches to it automatically when those earbuds connect.
4. Wired earphones (any cheap pair) bring the delay down to about 5 ms with no setup.

## Troubleshooting

- **Crackles or pops:** Audio & latency → Buffer size → 256.
- **No sound:** check the Audio engine card for an error, then click **Restart audio**. Make sure PipeWire is running: `systemctl --user status pipewire`.
- **Output card says it can't ask PipeWire:** install `libpulse` (it provides `pactl`).
- **Some keys don't light up when held together:** that's your laptop keyboard (ghosting). Check it in the **Keyboard test** tab.
- **Build fails on `pipewire`/`bindgen`:** install `clang`, or build without the native PipeWire backend: `cargo run --release --no-default-features`.

## Project layout

```
src/
  audio/        real-time engine (PipeWire via cpal), SoundFont + built-in synth
  audio/sequencer.rs  strum, arpeggiator, accompaniment (runs in the audio thread)
  audio/patterns.rs   accompaniment pattern format + built-in patterns
  keymap.rs     keybind model, TOML format, the four built-in layouts
  theory.rs     scales, chords, roman numerals, voice-leading
  drums.rs      General MIDI drum names
  config.rs     settings, keymap files, SoundFont search, file watching
  devices.rs    current output / Bluetooth codec detection via pactl
  latency.rs    tap-along latency measurement
  theme.rs      end4 / Material You colours and fonts
  music.rs      note names, chord naming, instruments
  learn/        Learn mode logic (no UI): analysis.rs (the theory overlay),
                intervals.rs, lessons.rs (all lesson content), ear.rs (games)
  ui/           egui app (app.rs, app/play.rs, app/learn.rs, app/keybinds.rs),
                custom widgets, circle.rs (circle of fifths),
                visuals.rs (keyboard / fretboard / fingerboard / flute / pads / waveform)
```

Run the tests with `cargo test`.
