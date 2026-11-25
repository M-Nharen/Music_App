# Qwertone song analyzer (Part 6)

Turns a song into something Qwertone can teach you: give it an audio file
(mp3, flac, wav, m4a, ogg…) or a MIDI file, and it writes a **song folder**
with separated instrument tracks (stems), tempo and beats, key, chords,
sections and the notes to play. The Rust app reads that folder (format in
[SONG_FORMAT.md](SONG_FORMAT.md)).

It's a separate Python tool on purpose: the machine-learning models it uses
only exist for Python, and the app itself stays fast and small.

## Install (Arch)

```bash
cd analyzer
./setup.sh            # CPU only. Safe default, ~1.5 GB
./setup.sh --gpu      # NVIDIA GPU: separation goes from minutes to seconds (~5 GB)
./setup.sh --light    # no AI models at all, ~200 MB (see "What runs without models")
```

The script installs `ffmpeg` and `uv` with pacman, makes a private Python 3.12
environment in `analyzer/.venv` (uv downloads that Python for you, so Arch's
newer system Python is fine), installs everything into it, downloads the model
weights once, and links `qwertone-analyze` into `~/.local/bin`. To remove it,
delete `analyzer/.venv` and that link.

Check what's installed and whether your GPU is used:

```bash
qwertone-analyze doctor
```

AMD GPU: pass a ROCm PyTorch index from pytorch.org, e.g.
`./setup.sh --torch-index https://download.pytorch.org/whl/rocmX.Y`.

## Use

```bash
qwertone-analyze ~/Music/song.mp3                        # chords + melody (default)
qwertone-analyze song.flac --mode chords
qwertone-analyze song.m4a  --mode simplified -d 2        # easy version, difficulty 2
qwertone-analyze song.mp3  --mode piano
qwertone-analyze song.mid  --mode full
```

It prints the song folder at the end, by default
`~/.local/share/qwertone/songs/<title>-<id>/`.

### Modes

| Mode | What you get | How |
|---|---|---|
| `chords` | chord names with Roman numerals + a playable chords part | chroma → chord templates → HMM smoothing |
| `melody` | the sung melody | CREPE on the vocal stem |
| `chords+melody` | both (default) | |
| `full` | every instrument | Basic Pitch on each stem + drum transcriber |
| `bass` | the bass line | CREPE on the bass stem |
| `piano` | piano notes | ByteDance piano transcription model on the piano stem (6-stem Demucs) |
| `drums` | kick / snare / hi-hats | band-split onset detector on the drum stem |
| `simplified` | an easy arrangement of chords + melody | see below |

Tempo, beats, bars, key, chords and sections are always included.

### Simplified

`--mode simplified` (or `--simplify` with any other mode) adds easy parts:

- notes snapped to a beat grid,
- the melody one note at a time in C4–G5 (the top letter row),
- a simple left hand from the chords in C3–B3 (the bottom row),
- never more than 1–3 keys held at once,
- moved to the key with the fewest black keys (often C major / A minor).

| `-d` | grid | keys at once | left hand |
|---|---|---|---|
| 1 | quarter notes | 1 | none (melody only) |
| 2 (default) | eighths | 2 | root note when the chord changes |
| 3 | eighths | 2 | root note every bar |
| 4 | sixteenths | 3 | root + fifth |
| 5 | sixteenths | 3 | root + third, 7th chords kept |

`--transpose none` keeps the original key (so it matches the recording),
`--transpose -2` picks a shift yourself, `--max-keys 2` overrides the limit.

### Confidence

Every note and chord has a confidence from 0 to 1. Anything under 0.5 is
marked `low_confidence` (change with `--low-confidence 0.4`). The app shows
these differently so you know which notes to double-check by ear.

### Caching

Results are cached per song in `~/.cache/qwertone/analyzer/`. Running the same
file again with the same settings does nothing. Changing the mode only runs
what's new (stems are separated once per file). `--force` redoes everything;
`qwertone-analyze cache info` / `cache clear` show or free the space.

### Other options

```
--no-separate          skip Demucs and analyse the whole mix (fast, less accurate)
--stems-model NAME     htdemucs (default), htdemucs_ft (better, 4x slower), htdemucs_6s
--device cpu|cuda      default: GPU if available
--crepe-model tiny|full   default: full on GPU, tiny on CPU
--chords basic         major/minor chords only
--json-progress        one JSON object per line on stderr, for the app's progress bar
-o DIR                 put song folders somewhere else
```

`qwertone-analyze analyze --help` lists everything.

## What runs without models

With `./setup.sh --light` (or when a model is missing) it falls back instead
of failing:

| Missing | Fallback |
|---|---|
| Demucs | the whole mix is analysed; melody/bass confidence is lowered |
| torchcrepe | librosa pYIN pitch tracking (slower, a bit less accurate) |
| basic-pitch | monophonic pitch tracker per stem |
| piano model | Basic Pitch |

Tempo, beats, key, chords and sections need no models at all.

## How fast

On a laptop CPU a 4-minute song takes roughly: a few minutes for stems
(once), then under a minute for the rest. Without stems
(`--no-separate`) it's about 25 seconds. With an NVIDIA GPU, stems take
seconds.

## Tests

```bash
./setup.sh --dev
.venv/bin/python -m pytest
```

The tests build a 27-second song (`tests/synth_clip.py`: 120 BPM, G major,
sections A B A, melody, pad, bass, drums, plus the same thing as MIDI) and
check that the analyzer gets it right: tempo, beats, downbeats, key, every
chord, the A B A sections, melody notes, drums, bass, simplification limits,
the JSON schema, and that the MIDI tempo map matches the JSON times.
Model tests are marked `models` and skip themselves when a model isn't
installed (`pytest -m "not models"` skips them on purpose).

Model weights can't be fetched in some CI sandboxes, so the Demucs and piano
model tests run the real code path with randomly initialised networks of the
same shape; with real weights you get real results.

`python tests/synth_clip.py /tmp/clip` writes the test song if you want to
listen to it.

## Code map

```
qwertone_analyzer/
  cli.py          command line
  pipeline.py     runs the steps, picks stems, caching per step
  audio_io.py     ffmpeg decoding, FLAC writing
  stems.py        Demucs
  features.py     chroma, onsets, timbre (shared by the next four)
  rhythm.py       tempo, beats, meter, downbeats
  chords.py       chord recognition + voicing the chords part
  keydetect.py    key (Krumhansl–Kessler + chord evidence)
  sections.py     self-similarity → boundaries → A/B labels → names
  melody.py       CREPE / pYIN pitch tracking → notes
  transcribe.py   Basic Pitch, piano model, drums
  midi_input.py   MIDI files as input
  simplify.py     easy arrangements
  midi_writer.py  song.mid with a beat-following tempo map
  writer.py       song.json + folder
  cache.py        per-song cache
```

## For the next parts

- **Part 5** reads `song.json` (see the Rust sketch in SONG_FORMAT.md). Draw
  `low_confidence` notes dimmed, and use `start_beat`/`end_beat` for the
  falling-notes grid, loops and slow-down.
- **Part 6 still to do (in Rust):** the note editor (load parts, fix notes,
  save back with confidence 1.0) and live listen mode. A good way to start the
  analyzer from the app: spawn `qwertone-analyze FILE --json-progress` and read
  stderr line by line; the last event is `{"event": "done", "song_dir": …}`.
- **Part 7** jam mode: the stems are in `stems/`; mute the stem of the part the
  user plays. If a simplified part has `transpose != 0`, the backing needs
  pitch-shifting too.
