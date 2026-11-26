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

### Lyrics

After writing the song folder the analyzer also makes `lyrics.json` (format in
[SONG_FORMAT.md](SONG_FORMAT.md#lyricsjson)): line by line with times and
beats, word timing when the source has it, the **language of the song and of
every line**, a romanised form of every line in an Indian script, and an
English machine translation of every line that isn't English.

Where the lyrics come from, in order: an imported .lrc, an .lrc next to the
audio (`Song.mp3` + `Song.lrc`), the file's own lyrics tags,
[LRCLIB](https://lrclib.net) (free synced lyrics, looked up by title, artist
and length), and finally **Whisper** listening to the vocal stem. Where each
part came from (file or URL, and the date) is stored for the words, the
timing and the translation separately, and anything a model made is marked
`auto_generated`.

- **No lyrics anywhere:** faster-whisper transcribes the vocal stem (the whole
  mix if the song was analysed without stems). The words and timing are
  marked auto-generated, with a confidence per line.
- **Only plain lyrics (no timing):** they are aligned to the vocal stem: Whisper
  transcribes it with word times and the lyric words are matched to what it
  heard (sound-alike matching across scripts and spellings). Your words are
  kept; only the timing is generated. If too little matches, they stay plain.
- **Language:** Whisper detects it per song from the singing (Tamil, Telugu,
  Hindi, …); if two languages are both likely (Hindi + English) it detects
  every 30 s window on its own, and each line gets the language of its own
  text. Sung Hindi is often detected as Urdu; that is written as Hindi
  (Devanagari) unless you say `--language ur`. `--language` overrides it all.
- **Romanised lines:** one library (Aksharamukha) for every Indian script:
  Tanglish, Tenglish, Hinglish, and Kannada, Malayalam, Bengali, … too.
- **English translation:** offline with NLLB-200 (any of ~200 languages to
  English), one line at a time from its own language; labelled machine
  translation. Romanised lines (Tanglish typed in Latin letters) aren't
  translated.
- **Untagged files:** AcoustID identifies the song from its sound so LRCLIB can
  be asked (needs `fpcalc` from `chromaprint` and a free key from
  <https://acoustid.org/new-application>: `--acoustid-key KEY`, or
  `ACOUSTID_API_KEY`, or the file `~/.config/qwertone/acoustid_key`).

```bash
qwertone-analyze lyrics ~/Music/song.mp3          # redo only the lyrics step (song folder or song.json works too)
qwertone-analyze lyrics --all                      # every analysed song
qwertone-analyze lyrics song.mp3 --show            # ...and print the lines, romanised + English
qwertone-analyze lyrics song.mp3 --language te     # it's a Telugu song (also tells Whisper)
qwertone-analyze lyrics song.mp3 --language hi-Latn   # the lyrics are Hinglish in Latin letters
qwertone-analyze lyrics song.mp3 --light           # no models: no Whisper, no translation
qwertone-analyze lyrics song.mp3 --whisper-model small   # faster, rougher (default large-v3-turbo)
qwertone-analyze lyrics song.mp3 --artist "Anirudh Ravichander" --title "Hukum"   # wrong tags
qwertone-analyze import-lrc my-timing.lrc ~/Music/song.mp3   # your own .lrc wins from now on
qwertone-analyze song.mp3 --lyrics-language ta --light       # the same options on analyze
```

Models download on first use into `~/.cache/qwertone/models/` (Whisper
large-v3-turbo ~1.6 GB, the translator ~650 MB; `qwertone-analyze
download-models` fetches them now). Both run on the CPU (int8) or an NVIDIA
GPU, without PyTorch. On a laptop CPU, Whisper takes roughly 1–3 minutes for
a 4-minute song; the translation a few seconds. Results are cached per song,
so `--force` doesn't run them again.

`--light` (or `QWERTONE_LIGHT=1`, or installing with `./setup.sh --light`)
skips every model: only local files, tags and LRCLIB, still romanised.
`--offline` never goes online (no LRCLIB, no AcoustID, no model downloads;
models already downloaded still run; `QWERTONE_OFFLINE=1` does the same),
`--no-translate` skips the translation, `--no-lyrics` on `analyze` skips the
whole step. LRCLIB and AcoustID answers are cached in the per-song cache;
"not found" is asked again after 7 days or with `--refresh`. If `lyrics.json`
is up to date the step does nothing; `--force` rebuilds it. To undo an
import, delete `lyrics/` in the song folder and run `qwertone-analyze lyrics`
again.

The translation model (NLLB-200) is licensed CC BY-NC 4.0: fine for your own
use, not for selling.

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
| faster-whisper | lyrics only from .lrc / tags / LRCLIB; plain lyrics stay untimed |
| ctranslate2 + sentencepiece | no English translation |

Tempo, beats, key, chords, sections and romanised lyrics need no models at all.

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
`tests/test_lyrics.py` uses `tests/data/sample_ta.lrc` (Tamil + English lines,
an offset, a repeated chorus, word timing) and a fake LRCLIB server;
`tests/test_lyrics_models.py` covers Whisper transcription, alignment,
translation, language detection (Tamil, Telugu, Hindi + English) and
AcoustID with fake engines. The suite never goes online and never loads a
lyrics model (`QWERTONE_LIGHT=1` is set for it); `QWERTONE_TEST_LYRICS_MODELS=1`
additionally runs the real Whisper and translator (downloads ~2 GB).
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
  lyrics.py       lyrics step: picks a source, lyrics.json, import-lrc
  lrc.py          .lrc parser (offsets, repeated lines, word timing)
  lrclib.py       LRCLIB lookup (stdlib HTTP, fuzzy match on title/artist/length)
  languages.py    language of a line / a song, romanised-language word lists, NLLB codes
  romanize.py     Indic scripts -> Latin (Aksharamukha + Tanglish/Hinglish rules)
  asr.py          Whisper (faster-whisper) on the vocal stem, hallucination filter, lines
  align.py        plain lyrics -> timing, matched against Whisper's words
  translate.py    offline English translation (NLLB-200, CTranslate2)
  acoustid.py     identify untagged files (fpcalc + AcoustID web service)
```

## For the next parts

The handoff notes for the app (Song mode, the note editor, live listen,
jam mode) now live in [PLAN.md](../PLAN.md) at the top of the repo, with the
rest of the project's notes.

## Lyrics notes (replace PLAN.md's "Lyrics notes" when merging)

Written here because Parts 10a-lite and 10a-full only touch `analyzer/`
(another branch edits PLAN.md); move this section into PLAN.md when merging.

- Spotify's lyrics can't be used (no public API; licensed to Spotify). Lyrics
  come from local files, LRCLIB, or are generated (Whisper). Spotify (Part 8)
  only supplies title/artist/duration/position for lookup and sync.
- **Songs are in several languages**: mostly Tamil, also Telugu and Hindi,
  sometimes others. Nothing is Tamil-only: the language is stored per song
  (`language`, `languages`) and per line (`lines[].language`, for songs that
  mix e.g. Hindi + English). Whisper detects the song language from the
  singing; `--language` (`--lyrics-language` on `analyze`) overrides it.
- Three toggles per line: **native script** (`text`), **romanised**
  (`romanized`: Tanglish / Tenglish / Hinglish …, one library — Aksharamukha
  — for every Indian script), **English** (`translation`, offline machine
  translation from any language, labelled as such).
- Fonts: egui 0.37 shapes text with harfrust, so Indic scripts render when
  the font is loaded: add Noto Sans Tamil, Noto Sans Telugu and Noto Sans
  Devanagari (all in `noto-fonts`) as fallback fonts; more scripts as needed.
- Source info is kept separately for words / timing / translation /
  transliteration, per song and per line where it differs (e.g. user edits),
  with the URL or file and fetch date, so the app can show a "where these
  lyrics came from" badge. Model output is flagged in `auto_generated`.
- Lyric timing uses seconds and beats (like notes) and the same Bluetooth
  latency offset as Song mode.

## Notes for later parts (from Parts 10a-lite and 10a-full)

**For 10b (lyrics in the app)**
- Read `lyrics.json` next to `song.json`; missing file = lyrics step never ran
  (offer "Find lyrics", which runs `qwertone-analyze lyrics <song dir>
  --json-progress`; it prints a `{"event": "lyrics", "status": …, "language":
  …, "auto_generated": {…}}` line). `status` `not_found` / `instrumental` →
  show that instead of an empty screen. Whisper can take minutes on a CPU:
  show the `{"event": "stage", "stage": "lyrics", …}` lines ("transcribing 60 / 240 s"); `--light` makes it
  quick (no models).
- Toggles map to `text`, `romanized`, `translation` (**not** `tanglish`: that
  10a-lite key is still written for Tamil lines but is deprecated). Hide the
  native-script toggle for `*-Latn` lines (`romanized == text`), the romanised
  toggle when `romanized` is null (English lines), and the English toggle while
  every `translation` is null. English lines have `translation: null`: show
  `text`. Label the English "machine translation".
- Badge: `sources.words.name` (+ `timing` if different, + transliteration,
  + translation), link to `url` or show `file`, and `fetched`. If
  `auto_generated.words` / `.timing`, say so ("Auto-generated from the vocals
  — may be wrong") and offer the editor / tap-along. `lines[].confidence`
  (0–1, only on generated lines) can grey out doubtful lines.
- Language: show `language` (and `languages` for mixed songs) and offer
  "Wrong language?" → `qwertone-analyze lyrics <song> --language te --force`.
- Current line: last line with `start <= t + bluetooth_offset`; use
  `start_beat`/`end_beat` in the falling-notes view so slow-down/loops line up
  with notes. Plain lyrics (`start == null`) can only be shown as a list.
- Editor / tap-along: write edits back into `lyrics.json` with a per-line
  `"sources": {"timing": {"kind": "user", "name": "edited in Qwertone",
  "url": null, "file": null, "fetched": <now>}}` (or `words`). Re-running the
  lyrics step **overwrites** `lyrics.json`, so either export the edit as
  `lyrics/imported.lrc` (it then wins forever) or keep user edits in a
  separate file and merge on load — decide in 10b. The simplest is: the
  editor saves `lyrics/imported.lrc` + runs `qwertone-analyze lyrics`
  (an imported synced .lrc is never re-aligned or re-transcribed; it is still
  romanised and translated).
- Word timing exists for enhanced-LRC sources, Whisper transcripts and well
  aligned lines; LRCLIB is line-level.

**From 10a-full (what was built, and what's left)**
- Whisper is faster-whisper (CTranslate2), default `large-v3-turbo`, int8 on
  the CPU. Transcripts are split into lines at 0.6 s pauses / 48 characters;
  typical music hallucinations ("Thank you for watching", loops, high
  compression ratio, silence) are dropped (`asr.good_segments`).
- Alignment (`align.py`) is word-level: Whisper's words vs the lyric words by
  a sound-alike key on romanised text, Needleman–Wunsch with free end gaps.
  It is not a phoneme forced aligner: lines Whisper hears badly get spread-out
  times and a low `confidence`. If that's not good enough on real songs, the
  next step is CTC forced alignment (MMS / wav2vec2 on romanised text, which
  `romanize.ascii_key` already produces) — heavier (torch, ~1.2 GB).
- Translation: NLLB-200 distilled 600M (CT2 int8, CC BY-NC). Romanised lines
  (Tanglish typed in Latin letters) aren't translated; transliterating them
  back to script first would be the fix.
- Whisper's "Urdu" for sung Hindi is mapped to Hindi (`languages.WHISPER_TO_FAMILY`).
- Not run against the real models in CI (no model downloads there): the
  engines are faked in `tests/test_lyrics_models.py`. Run
  `QWERTONE_TEST_LYRICS_MODELS=1 .venv/bin/python -m pytest tests/test_lyrics_models.py`
  once on a real machine, and try a few real Tamil / Telugu / Hindi songs.
- Romanisation rules are in `romanize._tidy`; word-initial ச is "s"
  (sinna, not chinna), as most Tanglish typists write it — make it a setting
  if wanted.

**For merging**
- Only `analyzer/` changed. `song.json` is unchanged and its fingerprint
  ignores every lyrics option, so existing song folders are not re-analysed.
  `lyrics.json` files from 10a-lite rebuild on the next `lyrics` / `analyze`
  run (`lyrics_version` 2); `format_version` is still 1 (only keys were added).
- New dependencies: `mutagen` and `aksharamukha` (pure Python, always
  installed); `faster-whisper`, `ctranslate2`, `sentencepiece`,
  `huggingface_hub` in the `models` extra (and a `lyrics` extra on their own).
  `setup.sh` also installs `chromaprint` (fpcalc) with pacman.
- `tanglish.py` is gone (replaced by `romanize.py`); `lyrics.line_language` /
  `song_language` / `TANGLISH_WORDS` still exist as aliases into `languages.py`.
