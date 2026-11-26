# Qwertone song format (version 1)

This is the contract between the Python analyzer (`analyzer/`) and the Rust
app (Part 5 song mode, Part 7 jam mode). The analyzer writes it; the app only
reads it. A machine-checkable version is in
[`schema/song.schema.json`](schema/song.schema.json) (JSON Schema 2020-12).

## Folder layout

One folder per song, by default under `~/.local/share/qwertone/songs/`
(`$XDG_DATA_HOME/qwertone/songs`):

```
~/.local/share/qwertone/songs/<title-slug>-<first 8 hex of sha256>/
├── song.json          everything (source of truth)
├── lyrics.json        lyrics, if the lyrics step ran (see "lyrics.json" below)
├── lyrics/imported.lrc  an .lrc the user imported (`qwertone-analyze import-lrc`)
├── song.mid           all parts as one type-1 MIDI file
├── midi/<part id>.mid each part on its own (handy for DAWs)
└── stems/             separated audio, only for audio input with stems on
    ├── vocals.flac
    ├── drums.flac
    ├── bass.flac
    ├── other.flac
    └── (piano.flac, guitar.flac with the 6-stem model)
```

To list songs, the app scans `songs/*/song.json`. The folder name is stable for
the same input file, so analysing again overwrites in place.

Paths inside `song.json` are relative to the song folder and always use `/`.

## Conventions

- **Times** are seconds from the start of the original recording (`float`).
  Stems start at the same moment, so `t` in JSON is `t` in every stem.
- **Beat positions** (`start_beat`, `end_beat`) are fractional indices into
  `beats`: `0.0` is the first entry of `beats`, `2.5` is halfway between
  `beats[2]` and `beats[3]`. Anything before the first beat is negative.
  Past the last beat, positions continue at the last beat's tempo.
  Use these to draw notes on a beat grid and for loop/slow-down maths.
- **Pitch** is a MIDI note number, C4 = 60, the same as `src/music.rs`.
  Note and chord names use sharps only (`C#`, never `Db`).
- **Confidence** is `0.0–1.0` everywhere. `low_confidence` is
  `confidence < low_confidence_threshold` (top level of `song.json`, default
  0.5). The app should draw these notes differently (dimmed, dashed,
  question mark) and the note editor should jump between them. Notes from
  MIDI input have confidence 1.0.
- Unknown keys must be ignored (newer analyzers may add fields). A change that
  would break old readers bumps `format_version`.

## song.json

```jsonc
{
  "format": "qwertone-song",
  "format_version": 1,
  "analyzer": {
    "version": "0.6.0",
    "created": "2026-11-26T04:35:00+00:00",
    "mode": "chords+melody",      // chords | melody | chords+melody | full | instrument | simplified
    "instrument": null,           // bass | piano | drums when mode == "instrument"
    "options": { ... },           // every option used (see `qwertone-analyze analyze --help`)
    "fingerprint": "69a931e339b8",// same input + same options + same analyzer => same value
    "models": { "stems": "htdemucs", "melody": "torchcrepe-tiny" },
    "device": "cpu"
  },
  "source": {
    "kind": "audio",              // audio | midi
    "file": "Song.mp3",
    "path": "/home/me/Music/Song.mp3",  // absolute, at analysis time (may move later)
    "sha256": "…",
    "duration": 215.3,
    "sample_rate": 44100          // of the stems; null for MIDI
  },
  "metadata": { "title": "Song", "artist": "…", "album": null },  // from file tags, else title = file name
  "duration": 215.3,
  "low_confidence_threshold": 0.5,

  "stems": [ { "name": "vocals", "file": "stems/vocals.flac" }, … ],  // [] for MIDI or --no-separate
  "midi": "song.mid",

  "tempo": {
    "bpm": 119.99,                // average; the real tempo map is `beats`
    "beats_per_bar": 4,
    "time_signature": [4, 4],
    "confidence": 0.91,
    "downbeat_confidence": 1.0
  },
  "beats": [
    { "time": 0.5108, "bar": 1, "beat": 1, "downbeat": true },
    { "time": 1.0217, "bar": 1, "beat": 2, "downbeat": false }
    // bar is 1-based; beats before the first downbeat are a pickup, bar 0
  ],

  "key": {
    "tonic": "G", "tonic_pc": 7, "mode": "major", "name": "G major",
    "confidence": 0.82,
    "alternatives": [ { "name": "E minor", "score": 0.81 }, … ]
  },

  "chords": [                      // back to back, cover 0 … duration
    {
      "start": 0.51, "end": 2.53, "start_beat": 0.0, "end_beat": 4.0,
      "symbol": "G",               // "N" = no chord; slash chords like "C/E"
      "root": "G", "quality": "maj", "bass": null,
      "pitch_classes": [7, 11, 2], // chord tones, root first
      "roman": "I",                // relative to `key`; null for "N"
      "diatonic": true,
      "confidence": 0.90, "low_confidence": false
    }
  ],

  "sections": [
    {
      "start": 0.0, "end": 8.52, "start_bar": 1, "end_bar": 4,
      "label": "A",                // same letter = same music repeating
      "name": "verse",             // guess: intro | verse | chorus | bridge | outro | section
      "confidence": 1.0            // how clear the boundary at `start` is
    }
  ],

  "parts": [
    {
      "id": "melody",              // unique, safe as a file name
      "name": "Melody",            // for display
      "role": "melody",            // melody | chords | bass | piano | drums | vocals | other | guitar | arrangement
      "method": "torchcrepe-tiny", // what made the notes
      "stem": "vocals",            // stem the notes came from; "mix" or null otherwise
      "is_drum": false,
      "program": 0,                // General MIDI program for playback
      "midi_track": 1,             // track index in song.mid (0 = conductor)
      "midi_channel": 0,           // drums always 9
      "midi_file": "midi/melody.mid",
      "transpose": 0,              // semitones relative to the recording (simplified parts)
      "simplified": null,          // copy of the top-level "simplified" settings for easy parts
      "note_count": 48,
      "low_confidence_count": 3,
      "pitch_range": [67, 79],     // [null, null] when empty
      "notes": [
        {
          "pitch": 71, "name": "B4",
          "start": 0.51, "end": 0.90,
          "start_beat": 0.0, "end_beat": 0.76,
          "velocity": 98,          // 1..127
          "confidence": 0.94, "low_confidence": false
        }
      ]
    }
  ],

  "simplified": null               // or the block below
}
```

### Parts by mode

| mode | parts |
|---|---|
| `chords` | `chords` |
| `melody` | `melody` |
| `chords+melody` | `melody`, `chords` |
| `full` | one per stem (`vocals`, `bass`, `other`, `piano`, `guitar`; or `mix` without stems) via Basic Pitch, plus `drums`. MIDI input: one per track, ids from track names |
| `instrument` | `bass`, `piano` or `drums` |
| `simplified` | `melody`, `chords`, then `melody-easy`, `chords-easy`, `arrangement` |

`--simplify` adds the three easy parts to any other mode too.

The `chords` part is a playable block-chord voicing of `chords` (voice-led,
roughly E3–D5; slash chords add their bass note an octave lower).

Drum parts use General MIDI drum notes on channel 9: 36 kick, 38 snare,
42 closed hi-hat, 46 open hi-hat, 49 crash. Drum notes are 0.1 s long.

### The `simplified` block

Present when the song has easy parts:

```jsonc
"simplified": {
  "difficulty": 3,                 // 1 (easiest) … 5
  "transpose": 5,                  // semitones applied to easy parts and chords
  "max_keys": 2,                   // never more keys held at once in "arrangement"
  "grid_beats": 0.5,               // notes start and end on this grid
  "melody_range": ["C4", "F#5"],   // where melody-easy lives (top keyboard row)
  "accompaniment": "root_each_bar",// none | root_on_change | root_each_bar | root_fifth | root_third
  "accompaniment_range": ["C3", "F#4"],
  "key": { … },                    // key after transposing
  "chords": [ … ],                 // chords after transposing and reducing to triads (difficulty < 5)
  "parts": ["melody-easy", "chords-easy", "arrangement"]
}
```

`arrangement` is what to play: easy melody on top, easy left hand below,
merged so `max_keys` is never exceeded (melody wins).

**Transposed parts don't match the original audio.** If `transpose != 0`, the
app should either pitch-shift the backing (Part 7) or play the easy part with
its own sound only, and say so to the user.

## song.mid

- Type 1, 480 ticks per beat.
- Track 0 (`conductor`): tempo map, time signature, key signature, section
  markers (`marker` meta, text `"A:verse"`) and chord symbols (`text` meta,
  text `"chord:Am"`).
- Then one track per entry in `parts`, in the same order, named by part `id`
  (`track_name` meta), with a `program_change` first (not for drums).
- **The tempo map follows the detected beats.** Every detected beat is exactly
  480 ticks, so bar lines in a DAW line up with the recording, and converting
  ticks back to seconds with the tempo map gives the JSON times to within
  a couple of milliseconds.
  Before the first downbeat there is a pickup of whole beats with its own
  time signature (e.g. 1/4), then the song's real meter.
- Confidence is not stored in MIDI. Use `song.json` for that.

## Stems

- FLAC, 16-bit, stereo, 44.1 kHz, same length as the decoded input.
- Names from Demucs: `vocals`, `drums`, `bass`, `other` (`htdemucs`,
  default); `htdemucs_6s` adds `piano` and `guitar` (used automatically for
  piano mode).
- Summing all stems gives back (almost exactly) the original mix, so Part 7
  can mute "your" part and play the rest.

## lyrics.json

Written by the lyrics step, right after `song.json` (and on its own with
`qwertone-analyze lyrics SONG`). Schema:
[`schema/lyrics.schema.json`](schema/lyrics.schema.json). It is a separate
file with its own `format_version`, so re-fetching lyrics never touches
`song.json`, and the app just checks whether `lyrics.json` exists next to it.

Same conventions as `song.json`: times in seconds from the start of the
recording, `start_beat`/`end_beat` as fractional indices into `song.json`'s
`beats`, unknown keys ignored. Text is UTF-8, in whatever script it was sung
in (Tamil, Telugu, Devanagari, Latin ...).

Songs are in many languages (mostly Tamil, also Telugu and Hindi, sometimes
others), so everything is per language: the song has one `language` plus the
list of every line language in `languages`, and **each line has its own
`language`** (a Hindi song with English lines has both). `romanized` and
`translation` follow the line's language.

```jsonc
{
  "format": "qwertone-lyrics",
  "format_version": 1,
  "analyzer": { "version": "0.6.0", "lyrics_version": 2, "created": "2026-11-26T10:59:27+00:00",
                "fingerprint": "…", "online": true,
                "models": true,          // false with --light (no Whisper / translation)
                "incomplete": false },   // a model step failed (e.g. no weights offline): the next run retries
  "song": { "title": "Kaalai Thendral", "artist": "…", "album": null,   // what was looked up
            "duration": 30.0, "sha256": "…",                            // = song.json source.sha256
            "identified_by": null },     // or { "kind": "acoustid", "name": "AcoustID", "url": "https://acoustid.org/track/…",
                                         //      "title", "artist", "album", "score", "recording_id", "fetched" }
  "status": "synced",          // synced | plain (no timing) | instrumental | not_found
  "language": "ta",            // the song's language, BCP 47 (see "Languages")
  "languages": ["ta", "en"],   // every line language, most lines first
  "language_source": "whisper",// user (--language) | whisper (heard) | text (script of the lyrics) | tag ([la:]) | none
  "has_word_timing": true,     // some lines have "words"
  "auto_generated": {          // made by a model, not written by a person: show it in the app
    "words": false,            //   Whisper transcribed the vocals
    "timing": false,           //   Whisper transcribed them, or plain lyrics were aligned to them
    "translation": true        //   always machine translation when present
  },
  "offset": 0.2,               // the source's [offset:] in seconds, ALREADY applied to every time
  "sources": {                 // where each layer came from; null = not available
    "words":  { "kind": "lrclib", "name": "LRCLIB", "url": "https://lrclib.net/api/get/4242",
                "file": null, "fetched": "2026-11-26T10:59:27+00:00", "id": 4242,
                "track": "…", "artist": "…", "album": "…", "duration": 30 },
    "timing": { … },           // often the same object as words, can differ (local words + LRCLIB / aligner timing)
    "translation": { "kind": "translator", "name": "NLLB-200 distilled 600M (machine translation)",
                     "url": "https://huggingface.co/JustFrederik/nllb-200-distilled-600M-ct2-int8", "file": null,
                     "fetched": "…", "model": "…", "target": "en", "version": 1,
                     "machine_translation": true, "auto_generated": true },
    "transliteration": { "kind": "generated", "name": "Aksharamukha romanisation",
                         "url": "https://aksharamukha.appspot.com", "file": null, "fetched": "…",
                         "version": 2, "library": "aksharamukha 2.3",
                         "style": "RomanReadable + Qwertone rules (Tanglish / Tenglish / Hinglish)" }
                       // null when no line is in an Indian script
  },
  "whisper": { "model": "large-v3-turbo", "language": "ta", "detected": "ta",   // only when Whisper ran
               "language_probability": 0.83, "top_languages": [["ta", 0.83], ["ml", 0.09]],
               "per_window_language": false },
  "lines": [
    {
      "start": 2.0, "end": 5.5,            // null for status "plain"
      "start_beat": 3.0, "end_beat": 10.0, // null when plain or song.json has no beats
      "text": "காலை தென்றல் வந்தது",
      "language": "ta",                    // per line
      "romanized": "kaalai thendral vandhadhu",   // Latin letters; = text for ta-Latn/hi-Latn/te-Latn lines; else null
      "tanglish": "kaalai thendral vandhadhu",    // DEPRECATED (10a-lite name): = romanized on Tamil lines, else null
      "translation": "The morning breeze came",   // English machine translation; null for English / romanised lines
      "confidence": 0.87,                  // only when words or timing are auto-generated (0-1)
      "words": [                           // null unless the source has word timing
        { "text": "காலை", "start": 2.0, "end": 2.6, "start_beat": 3.0, "end_beat": 4.2,
          "romanized": "kaalai", "tanglish": "kaalai" }
      ]
      // "sources": { "timing": { "kind": "user", … } }   optional per-line override, see below
    }
  ]
}
```

### Languages

| `language` | means | `romanized` | `translation` |
|---|---|---|---|
| `ta`, `te`, `hi`, `kn`, `ml`, `bn`, `gu`, `pa`, `or`, `mr` … | that language in its own script | Tanglish / Tenglish / Hinglish … | English |
| `ta-Latn`, `te-Latn`, `hi-Latn` | written in Latin letters (Tanglish, Tenglish, Hinglish) | = `text` | null (no model reads romanised text) |
| `en` | English | null | null (show `text`) |
| `ar`, `ru`, `ko`, `ja`, `zh` … | other scripts Whisper writes | null | English |
| `und` | unknown (no letters) | null | null |

How a line's language is decided: its script (Tamil, Telugu, Devanagari …);
Latin text is English unless enough common Tanglish / Hinglish / Tenglish
words appear in it. The song's language decides between languages sharing a
script (Devanagari is Hindi unless the song is Marathi …). The song
`language` is `--language` if given, else the lines (a song with a few
English lines is still a Tamil/Hindi/Telugu song), else what Whisper heard,
else the LRC `[la:]` tag.

The app: hide the native-script toggle for `*-Latn` lines (`romanized ==
text`), the romanised toggle when `romanized` is null, and the English
toggle while every `translation` is null. Label translations "machine
translation" (`sources.translation.machine_translation`). Load Noto Sans
Tamil / Telugu / Devanagari as fallback fonts.

### Sources

Every source object has `kind`, `name`, `url` (or null), `file` (or null) and
`fetched` (ISO 8601, when Qwertone got it; for an import, when it was
imported). Extra keys by kind:

| kind | what | extra keys |
|---|---|---|
| `file` | an .lrc next to the audio (`Song.lrc`), or one imported into the song folder | `sha256`, `modified`, `by` (LRC `[by:]`), `copy` (`lyrics/imported.lrc` for imports; `file` is then the original path) |
| `embedded` | the audio file's own tags (USLT / LYRICS / ©lyr, ID3 SYLT) | `tag`, `sha256` |
| `lrclib` | [LRCLIB](https://lrclib.net) | `id`, `track`, `artist`, `album`, `duration` |
| `whisper` | transcribed from the vocal stem (auto-generated words + timing) | `model`, `language`, `detected`, `language_probability`, `auto_generated`; `file` = `stems/vocals.flac` (or the audio file when there was no vocal stem) |
| `aligner` | plain lyrics aligned to the vocal stem with Whisper (auto-generated timing) | `model`, `version`, `matched` (share of lyric words heard), `auto_generated`; `file` as above |
| `translator` | offline machine translation | `model`, `target`, `version`, `machine_translation`, `auto_generated` |
| `generated` | made by the analyzer (romanisation) | `version`, `library`, `style` |
| `user` | edited in the app (Part 10b) | anything useful, e.g. `edited` |

Top-level `sources` describe the whole song. A line may carry its own
`sources` object with any of the same four keys; those override the top level
for that line only (e.g. after the user retimes one line in the editor,
that line gets `"sources": {"timing": {"kind": "user", …}}`). Keys a line
doesn't have fall back to the top level.

### Which source wins

1. `lyrics/imported.lrc` in the song folder,
2. an .lrc next to the audio file, same name (`Song.mp3` → `Song.lrc`),
3. lyrics in the audio file's tags,
4. LRCLIB, by title, artist, album and duration (±3 s). If song.json has no
   artist and the file is named `Artist - Title`, that is used. A file with
   no title/artist tags can be identified with AcoustID first (needs `fpcalc`
   and a free API key; see the analyzer README).
5. Whisper on the vocal stem (`auto_generated.words`), when none of the above
   has lyrics and LRCLIB doesn't say the song is instrumental.

The first one with **synced** lyrics wins. If 1–4 only have plain text,
LRCLIB is still asked: when its synced lyrics are the same text (≥ 80 %
similar line by line, same line count), `words` stays the local source and
`timing` becomes LRCLIB. Otherwise the plain lyrics are **aligned** to the
vocal stem (`timing.kind` = `aligner`, `auto_generated.timing`); if under
30 % of their words are heard, they stay `plain`. With `--light` (no models)
steps 5 and the alignment are skipped.

### Timing rules

- A line ends where the next line starts, or at a timestamped empty line
  (LRC's "instrumental gap" marker), or at its last word's end stamp; the
  last line ends with the song. `end >= start` always.
- A line with several timestamps (a repeated chorus) appears once per
  timestamp, sorted by time; word times move with it.
- Words without an end stamp end where the next word starts (the last one:
  at the line end).
- Whisper lines are sung phrases: a segment is split at pauses of 0.6 s or
  more and at 48 characters, and every line has word timing.
- Aligned lines have word timing when at least half their words were heard;
  words between heard ones are spread evenly.
- The app adds the same Bluetooth latency offset as Song mode; nothing in
  lyrics.json accounts for it.

### Romanisation

One library for every Indian script: [Aksharamukha](https://github.com/virtualvinodh/aksharamukha)
(`RomanReadable`), with a few rules on top so it reads the way people type:

- Tamil (Tanglish): long vowels doubled (`aa`, `ee`, `oo`), `zh` for ழ, and
  the sound of க/ச/ட/த/ப/ற picked from the letters around it (`kaadhal`,
  `nenjam`, `endru`, `vanakkam`, `pacchai`).
- Hindi (Hinglish): the silent a is dropped (`dil`, not `dila`), nasals are
  `n` (`main`, `hoon`, `hain`), word-final long ī/ā are short (`kabhi`, `tera`).
- Telugu (Tenglish), Kannada, Malayalam, Bengali, Gujarati, Punjabi, Odia:
  `RomanReadable` as is (`nuvvu naa praanam`).

Text in other scripts in the same line (English words) is kept as is.
`version` in the transliteration source changes when the rules change.

### Translation

Offline, per line, from the line's own language to English, with NLLB-200
(about 200 languages, so any language Whisper detects). English lines and
romanised lines (`*-Latn`) get `null`. It is machine translation of song
lyrics: literal, sometimes wrong; the app must label it so.

## Reading it from Rust (sketch for Part 5)

```rust
#[derive(serde::Deserialize)]
struct Song { format_version: u32, duration: f64, low_confidence_threshold: f32,
              tempo: Tempo, beats: Vec<Beat>, key: Option<Key>, chords: Vec<Chord>,
              sections: Vec<Section>, parts: Vec<Part>, stems: Vec<Stem>,
              simplified: Option<Simplified> }
#[derive(serde::Deserialize)]
struct Note { pitch: u8, start: f64, end: f64, start_beat: f64, end_beat: f64,
              velocity: u8, confidence: f32, low_confidence: bool }
// serde ignores unknown fields by default, which is what we want.
```

Reject files whose `format` isn't `"qwertone-song"` or whose
`format_version` is newer than the app knows.

## Caching (analyzer side, for reference)

The analyzer keeps intermediate results in `~/.cache/qwertone/analyzer/`
keyed by the input's sha256, so analysing the same file in another mode only
runs the missing steps (stems are separated once per file). If the song folder
already holds a `song.json` with the same `source.sha256` and
`analyzer.fingerprint`, nothing runs at all. The app can use the same check to
show "already analysed".
