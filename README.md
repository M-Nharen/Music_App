# Qwertone

Play music on your laptop keyboard. Built for Arch + Hyprland (end4 rice), works over Bluetooth earbuds, and lets you remap every key.

> **Status: complete (Parts 1–10).** Low-latency engine, four layouts (Piano, Chords + Melody, Isomorphic, Drums), auto-accompaniment, Learn mode (theory overlay, 30 lessons, ear-training games), metronome, recording and a looper, MIDI out, Song mode (falling notes, chord sheet, wait mode, loops, scoring, one-key mode, auto song layout), Jam mode (play along with the real recording's separated parts), a note editor, live listen mode, lyrics (in several languages, with an editor), Spotify (now playing, search, sync mode), practice stats and streaks, themes, the song analyzer (`analyzer/`), keybind editor, latency tools and a keyboard test. [PLAN.md](PLAN.md) has the design notes for every part.

**Tabs at a glance:** [Play](#playing) · [Learn](#learn) · [Song](#songs) · [Listen](#listen) · [Spotify](#spotify) · [Progress](#progress) · [Settings](#settings) (General, [Keybinds](#keybinds), Audio & latency, Keyboard test)

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

Colours follow end4's Material You palette automatically (from `~/.local/state/quickshell/user/generated/colors.json`, or the older AGS `_material.scss`) and change when your wallpaper does. Without end4 (or with *Match end4* unticked in **Settings → General**) you pick a built-in theme: Violet, Ocean, Forest (all dark), Light or High contrast.

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

The octave and velocity can also be changed with the − / + and *Velocity* buttons in the toolbar. The big readout names whatever chord you're holding (try `Z C B` = C major, `N Q E` = A minor).

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

**Bluetooth:** the rhythm keeps perfect time inside the app, but you hear it ~200 ms late, so you'll naturally press chord changes a little late. Qwertone uses your measured latency (Settings → Audio & latency) to let a chord pressed just after the beat still land on that beat.

### Isomorphic

Every key to the right is a whole step, up-left is a fourth, up-right is a fifth. Every chord and scale has the same shape in every key: a major chord is always "this key, two keys right, and the key up-right" (`Z C S` = C E G).

### Drums

Bottom row kicks, snares, claps; home row hi-hats, ride, congas; Q-row toms and crashes; Space is a kick. `Enter` plays the current pattern's beat so you can drum along.

### The picture under the keyboard

It follows your instrument and lights up whatever is playing (melody in one colour, chord in another):

| Instrument | Picture |
|---|---|
| Pianos, organ, electric piano, mallets, harp | Keyboard |
| Guitars, sitar | Guitar fretboard (standard tuning), with a playable hand position for each chord |
| Strings | Violin or cello fingerboard (cello when the music is low), with beginner finger-tape lines |
| Flute | 6-hole fingering diagram (● covered, ◐ half, ○ open) for the note you're playing |
| Drums layout | Drum pads that flash when hit |
| Pads, synths, choir, brass, sax, clarinet, shehnai | A glowing waveform of the notes |

The **View** menu above it lets you pick any picture instead. Dots on the fretboards mark the notes of the current scale.

### Key and scale

The **Key** and **Scale** pickers on the Play tab apply to every layout. Keys get a • if their note is in the scale and a ★ on the key's home note. **Scale lock** makes piano/isomorphic keys play the nearest scale note instead, so nothing sounds wrong.

### Instruments

**PageUp / PageDown** (or the picker) step through 36 sounds: pianos, harpsichord, celesta, music box, mallets (vibraphone, marimba, xylophone, kalimba, steel drums), organ, accordion, guitars (nylon, steel, clean electric), sitar, harp, strings (ensemble, violin, cello, pizzicato), choir, brass (trumpet, brass section), reeds (alto sax, clarinet, shehnai), flutes (flute, pan flute), synth leads and pads. With a SoundFont they use its General MIDI sounds; the built-in synth has a rough version of each family.

### Metronome, recording and the looper

The two bars under the keyboard on the Play tab:

- **Metronome** (F5), **tempo** (F7/F8, or drag), **Tap** (F6: tap in time to set the tempo), **Record** (F9): clicks in for 4 beats, records everything you play and saves `~/Music/qwertone/take-001.mid` when you press it again.
- **Looper** (F10): clicks in, then records a loop of the length you picked (1–16 bars, or *Free*: press F10 again when you're done and it closes at the nearest bar line). The loop then plays round and round in time with the metronome.
  - **F10** again starts an **overdub**: play on top while it loops, press F10 to keep that layer. Up to 8 layers.
  - **F11** stops / plays the loop, **F12** undoes the last layer, **Delete** twice clears everything. Click a layer's chip to mute it.
  - Each layer keeps the instrument it was recorded with: record a piano layer, switch to strings (PageUp) and overdub.
  - Changing the tempo speeds the whole loop up or slows it down. Escape stops the loop; switching windows doesn't.
  - Press Record (F9) while a loop plays and the take starts at the top of the loop, with the loop in it.

All of these keys can be changed in Settings → Keybinds (`loop`, `loop-play`, `loop-undo`, `loop-clear`, `loop-mute 1`…).

### MIDI out to other apps

**Settings → Audio & latency → Virtual MIDI out → Send MIDI out** opens a MIDI port called **Qwertone**. Everything you play (notes, chords, bass, drums, loop layers; the song backing too if you tick it) goes out there, timed like the sound, so you can record it in a DAW or play it through another synth. It's an ALSA sequencer port, so ALSA apps see it directly and PipeWire shows it through its MIDI bridge (connect it in qpwgraph or Helvum, or `aconnect Qwertone:0 <synth>`). Untick *Also play the sound here* to hear only the other app.

Channels: 1 melody, 2 chords, 3 bass, 10 drums; loop layers get their own channels from 16 downwards.

## Learn

The **Learn** tab teaches music from zero, on your own keyboard and keybinds.

**What you're hearing.** Whatever you play gets explained: the chord's name, its roman numeral in the key (V7 in C major), its job (home, away, tension), why it sounds happy, sad or tense, a Rest ↔ Tension meter, and how the move from the last chord sounds ("V → I: tension → release"). Two notes are named as an interval with songs that start with it. The same panel shows on the Play tab (untick **Explain** to hide it).

**Lessons.** 30 short, hands-on lessons in 9 chapters:

1. Notes & octaves
2. Intervals (each with famous songs that start with it)
3. Major, minor and pentatonic scales
4. Building chords: triads, major vs minor, dim/aug/sus, the 7 chords of a key, 7ths
5. Progressions: home/away/tension, cadences, I–V–vi–IV, ii–V–I, borrowed chords
6. The circle of fifths (drawn next to the lesson and following your chords)
7. Modes
8. Rhythm & first tunes: beats and tempo (with the metronome), Ode to Joy / Twinkle Twinkle / Frère Jacques, inversions, melody over chords
9. Indian music: Sa Re Ga Ma (sargam and the Sa–Pa drone), Mayamalavagowla, Mohanam

Lessons light up the keys to press in your current layout, numbered in order, and tick them off as you play. If your layout can't play a step (say, a black key in Chords + Melody), the lesson offers to switch. Lessons are written in C major; if you're in another key there's a one-click reset. Progress is saved.

**Ear training.** Four games, three levels each: *Find the note*, *Intervals*, *Happy or sad?* and *Name that chord*. You answer by playing: the reference key ("home", "1st") and, on easier levels, the possible answers are lit. Best streaks are saved.

On the Learn tab **F2** plays the example / question again and **F3** goes to the next step (unless your layout uses those keys).

## Songs

The **Song** tab teaches you real songs on your own keys.

**Getting a song.** Two ways:

- **Analyse a recording** (mp3, flac, wav, m4a…): pick the file (Browse…, or drag it onto the window), choose what you want (chords + melody is the best start; *Easy version* adds simplified parts) and press **Analyze**. Qwertone runs the analyzer (`qwertone-analyze`, see [analyzer/README.md](analyzer/README.md); install it once with `cd analyzer && ./setup.sh`) and shows a progress bar. Finished songs appear under **Your songs** (they live in `~/.local/share/qwertone/songs/`).
- **Open a MIDI file**: any `.mid` works. Chords are guessed from the notes; MIDI files in `~/Music` and `~/Downloads` are listed.

**Practising.** Notes fall towards a line; the letter on each note is the key to press when it reaches the line, and the strip under the line shows which key plays which note. Chords fall in their own lane on the left with their key ("G · Z").

- **I play**: pick the parts you play (melody, easy melody, bass…) and/or the chords.
- **Song layout** (on by default): switches to a layout made for the song. Each different chord of the song gets one key on the bottom rows (home chord on Z), and the song's notes get one key each on the top rows, low to high. The fewest keys possible. Untick it to go back to your own layout (the song's notes then use whatever keys your layout has; "Any octave counts" helps).
- **Wait for me**: the song stops at each note until you press it. No rush; nothing is missed.
- **One-key mode**: press any letter key and the next note(s) play. Good for feeling the rhythm first.
- **Speed** 25–150 % (F7 / F8). **Loop** a section (click its name), *This bar ×2*, or drag across the timeline. *Speed up when I get it* goes 5 % faster after each loop pass of 90 % or more.
- **Chord sheet** on the right: bars in rows of four, the current bar lit, roman numerals and the key for each chord. Click a bar (or the timeline) to jump.
- **Scoring**: Perfect / Good / Missed, wrong keys, streaks, a result card at the end and your best score per song. In wait mode only right or wrong counts.
- Notes the analyzer wasn't sure about are drawn dimmed and dashed with a "?". Trust your ears.
- **F5** play/pause · **F6** restart · **F7/F8** slower/faster · **F9** wait mode on/off (when your layout doesn't use those keys).

**Jam with the recording.** Songs analysed with stem separation (the default) come with the real recording split into parts: vocals, drums, bass and the other instruments (piano and guitar too with the 6-stem model). With *Play the real recording* on (the **Jam with the recording** card, on by default) you hear the actual band instead of the MIDI notes, and the part you play is muted so you take its place (playing the melody mutes the singer; chords mute the other instruments). The mixer has On/Muted, Solo and a volume for each part: turn the singer back on to follow along, or solo the bass to hear it clearly.

- **Speed** works on the recording too: it's time-stretched, so a slowed-down song keeps its pitch.
- **Easy parts in an easier key** (*Melody (easy)* moved to C, say): the recording is pitch-shifted to match, so you still play along in tune. The drums aren't shifted.
- **Loops and wait mode** work as before. While the song waits for your note, the band holds its last sound and fades out.
- The stems are loaded into memory when you open the song (about 40 MB per part for a 4-minute song; the card shows how much).

**Bluetooth.** *Line up with my output delay* (on by default) shifts the backing and the falling notes by your earbuds' delay: press a key when its note reaches the line and your note comes out of the earbuds together with the band. (Without it the notes follow what you hear, and what you play sounds a little late against the backing.) The delay is the one you measured (Settings → Audio & latency → latency test); the scoring still forgives a slightly late press (the same grace window as the chord keys). If it still feels off, *Sound & sync → Nudge* moves things by a few ms.

**Edit notes.** *Edit notes* opens the part with the most unsure notes in a piano roll. **Tab** jumps to the next unsure note (it plays so you can check it by ear), ↑↓ change its pitch, drag to move, drag the right edge to resize, double-click to add, Delete removes, **Enter** marks it "looks right", Ctrl+Z undoes. **Save** (Ctrl+S) writes `song.json` (every note you touched gets confidence 1.0; a `song.json.bak` is kept) and refreshes the song's MIDI files. For a plain MIDI file it saves `<name>-edited.mid` next to it.

**Lyrics.** *♪ Lyrics* shows the words big, following the song (click a line to jump there), and the current line also shows inside the falling notes (top or bottom, from the *Lyrics* section; it can show the next line too). Songs can be in any language, and mixed ones (Hindi + English, say) are handled line by line:

- Lines are shown in English letters (Tanglish / Tenglish / Hinglish …) by default; the original script (Tamil, Telugu, Devanagari … with the Noto fonts) and the English machine translation can be switched on next to them.
- **Where they come from**: a synced `.lrc` next to your audio file or inside its tags, [LRCLIB](https://lrclib.net), or transcribed from the singing (Whisper). The badge on the Lyrics screen says which, and warns when they were generated and may be wrong. *Wrong language?* re-runs it in the right one.
- **No lyrics yet?** *Find lyrics* runs the analyzer's lyrics step; *Quick (no models)* skips the speech models (your files and LRCLIB only); or import an `.lrc` / `.txt`, or paste the words.
- **Fix them** in the lyrics editor: edit the text, nudge lines, or *tap along* (press a key at the start of each line while the song plays) to time plain lyrics. Ctrl+S saves; your edits are kept even if the lyrics are looked up again (*Forget my edits* undoes that).
- **Nudge** moves the lyrics earlier / later if they're off. They use the same Bluetooth line-up as the notes.

## Spotify

The **Spotify** tab connects to your Spotify (Premium) account through your own personal developer app: set it up once with [docs/SPOTIFY_SETUP.md](docs/SPOTIFY_SETUP.md) (create the app, add the redirect URI `http://127.0.0.1:8898/callback`, paste the Client ID, log in).

- **Now playing** with play / pause / seek, and **search** (results are remembered).
- **Learn this** opens the practice version of the song. If you haven't analysed it yet, Qwertone finds your copy in `~/Music` / `~/Downloads` (or you pick it) and analyses it; Spotify's own audio is never used.
- **Sync mode**: Learn this on the song that's playing makes the Song tab follow Spotify: falling notes, chord sheet, lyrics and scoring move with Spotify's position, and play / pause / seek control Spotify. *Notes − later / + earlier* lines your file up with Spotify's version (saved per song).
- **Lyrics** for whatever plays, synced: from your analysed song, else LRCLIB by Spotify's title / artist / length.
- Spotify's limits for personal apps are small, so Qwertone only checks what's playing while the Spotify tab or sync mode is on screen (every 12 s, plus right after you press play / pause / seek) and estimates the position in between. If Spotify says "too many requests", everything waits as long as it asks, the header shows *Spotify paused for N min*, and the app carries on with what it knows.

## Listen

The **Listen** tab hears any audio input and shows the chord, the notes, the melody note (with a little tuner), the key so far and the chords heard, and lights the keys of your layout that play along. Pick the input that says *what the computer plays* to follow Spotify or a video without a microphone. Don't pick the earbuds' own microphone: Bluetooth switches to call quality while it's in use.

## Progress

The **Progress** tab shows how your practice is going:

- **Streak**: days in a row you've played. A day counts after 1 minute of playing, and today not having counted *yet* doesn't break it. The header shows the streak too (click it to open Progress; hide it in Settings → General).
- **Today** against your daily goal (10 minutes by default, set in Settings → General), **this week** compared with last week, and **all time**.
- A bar per day of this week with the goal line, and a calendar of the last 20 weeks (brighter = more practice; hover a day for the details).
- Where the time went (free play, lessons and ear training, songs), lessons done, ear-training answers, and **your songs**: runs, best and last score.

Practice time only counts while you're actually playing: the time between notes, chords and drum hits, as long as you don't stop for more than 30 seconds. Leaving the app open doesn't add anything. The numbers are kept in `~/.local/share/qwertone/stats.json`, on your computer only; *Reset stats* (press twice) clears them.

## Settings

The **Settings** button holds four pages:

- **General**: *Match end4 / wallpaper colours*, the built-in themes, **window size** (80–160 %; Ctrl + / Ctrl − / Ctrl 0 work too when your layout doesn't use Ctrl), the daily practice goal and the streak chip, and **where things are kept**, with an *Open* button for each folder.
- **Keybinds**: see [below](#keybinds).
- **Audio & latency**: the output device and Bluetooth codec, the audio engine and buffer size, the tap-along latency test, the SoundFont, instruments per device, virtual MIDI out and your saved devices. See [Bluetooth earbuds](#bluetooth-earbuds-galaxy-buds-live-sony-xm4).
- **Keyboard test**: hold keys to see how many your laptop keyboard can sense at once (sound is off on this page).

Spotify's own settings (Client ID, the login store, how often to check, the sync offset) are at the bottom of the Spotify tab.

### Where things are kept

| What | Where |
|---|---|
| Settings | `~/.config/qwertone/config.toml` (edit by hand if you like: the app reloads it; a file with a mistake is kept as `config.toml.broken` and the defaults are used) |
| Keyboard layouts | `~/.config/qwertone/keymaps/*.toml` |
| Accompaniment patterns | `~/.config/qwertone/patterns/*.toml` |
| Analysed songs | `~/.local/share/qwertone/songs/<song>/` (`song.json`, MIDI, stems, `lyrics.json`) |
| Practice stats | `~/.local/share/qwertone/stats.json` |
| Recordings | `~/Music/qwertone/take-NNN.mid` |
| Spotify login | the desktop keyring (`secret-tool`), else `~/.local/share/qwertone/spotify-login.json` (only you can read it) |
| Spotify cache, lyrics for any track | `~/.cache/qwertone/` |
| The analyzer | `analyzer/.venv`, linked as `~/.local/bin/qwertone-analyze`; its models in `~/.cache` (Hugging Face, torch) |

To remove everything: uninstall the package (or delete the folder you cloned), then delete `~/.config/qwertone`, `~/.local/share/qwertone`, `~/.cache/qwertone` and `~/.local/bin/qwertone-analyze`.

## Keybinds

Open **Settings → Keybinds**:

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
| `metronome`, `tap-tempo`, `record` | Metronome on/off, tap tempo, record / stop + save |
| `loop`, `loop-play`, `loop-undo`, `loop-clear`, `loop-mute <1-8>` | Looper: record / overdub, play / stop, undo layer, clear (press twice), mute a layer |

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

1. **Settings → Audio & latency → Output.** If a lower-delay codec is available, click **Switch to a2dp-sink-sbc**. SBC has less delay than AAC or LDAC (LDAC on the XM4 has the most).
2. **Run the latency test** with each pair and press **Save**. Song and Jam mode use this number to line up the falling notes, the backing and your own notes.
   Every saved pair appears under **Saved devices** (name, Bluetooth or wired, delay, instrument, and whether it's connected now), where you can rename, re-test or remove it.
3. **Choose an instrument per device.** Sounds marked ✦ (pads, strings, organ, choir) have soft attacks that hide the delay. Qwertone switches to it automatically when those earbuds connect.
4. Wired earphones (any cheap pair) bring the delay down to about 5 ms with no setup.

## Troubleshooting

- **Crackles or pops:** Settings → Audio & latency → Buffer size → 256.
- **No sound:** check the Audio engine card for an error, then click **Restart audio**. Make sure PipeWire is running: `systemctl --user status pipewire`.
- **Output card says it can't ask PipeWire:** install `libpulse` (it provides `pactl`).
- **Some keys don't light up when held together:** that's your laptop keyboard (ghosting). Check it in **Settings → Keyboard test**.
- **Jam mode stutters or uses a lot of CPU:** run the release build (`cargo run --release`); a debug build is 20–50 × slower at time-stretching.
- **Lyrics show boxes instead of letters:** install the Noto fonts (`sudo pacman -S noto-fonts`); the Lyrics screen lists the missing ones.
- **The window is too small / too big:** Settings → General → Window size.
- **Spotify says "paused for N min":** Spotify asked the app to slow down; it waits by itself. Keep using the Spotify app meanwhile. See [docs/SPOTIFY_SETUP.md](docs/SPOTIFY_SETUP.md).
- **MIDI out says it can't open the ALSA sequencer:** load the module once with `sudo modprobe snd-seq` (to keep it, add `snd-seq` to `/etc/modules-load.d/snd-seq.conf`).
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
  theme.rs      end4 / Material You colours, built-in themes, fonts
  stats.rs      practice time, streaks, song records (stats.json)
  music.rs      note names, chord naming, instruments
  learn/        Learn mode logic (no UI): analysis.rs (the theory overlay),
                intervals.rs, lessons.rs (all lesson content), ear.rs (games)
  song/         Song mode logic (no UI): song.json + MIDI reading/writing, chord
                and key guessing, practice (targets, timing, wait, scoring),
                the auto song layout, the note editor, running the analyzer
  audio/songplay.rs   song playback in the audio thread (speed, loops, wait)
  audio/stems.rs      Jam mode: the analyzer's stems as the backing track
  audio/stretch.rs    time-stretch / pitch-shift (phase vocoder) + resampling
  audio/looper.rs     the looper (layers, overdub, mute) in the audio thread
  record.rs     recording takes to MIDI files, tap tempo
  midiout.rs    virtual MIDI out (ALSA sequencer port + timing thread)
  listen/       live listen: audio input + chord / note / pitch detection
  spotify/      Spotify (no UI): PKCE login, token store (keyring / private
                file), rate limiter, polling + position estimate, API worker,
                caches, matching tracks to songs, lyrics lookup
  ui/           egui app (app.rs, app/play.rs, app/learn.rs, app/song.rs,
                app/song/jam.rs, app/song/lyrics.rs, app/listen.rs,
                app/keybinds.rs, app/record.rs, app/looper.rs,
                app/spotify_tab.rs, app/progress.rs, app/settings.rs),
                custom widgets, circle.rs,
                song_view.rs (falling notes, chord sheet, timeline, piano roll),
                visuals.rs (keyboard / fretboard / fingerboard / flute / pads / waveform)
analyzer/       the Python song analyzer (see analyzer/README.md)
```

## Tests

```bash
cargo test                                   # the app: ~200 tests (audio engine, theory, lessons, songs, lyrics, Spotify, stats…)
cargo clippy --all-targets                   # lints
cargo test --release -- --ignored --nocapture cost   # Jam mode's CPU cost, in release
cd analyzer && ./setup.sh --dev && .venv/bin/python -m pytest   # the analyzer (the ML models are faked)
```

Most logic lives outside the UI (`learn/`, `song/`, `spotify/`, `stats.rs`, the audio thread), so it's covered by those tests; the screens were checked under Xvfb. Things that need real hardware (how Bluetooth sync feels, the real Spotify login, live listen on a real microphone) are listed in PLAN.md.
