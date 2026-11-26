"""The lyrics step: song folder -> lyrics.json (see SONG_FORMAT.md, "lyrics.json").

Where lyrics come from, first match wins:

1. an .lrc imported into the song folder (`qwertone-analyze import-lrc`),
2. an .lrc next to the audio file (`Song.mp3` -> `Song.lrc`),
3. lyrics stored in the audio file's tags (USLT / LYRICS / ©lyr, and ID3 SYLT
   synced frames when mutagen is installed),
4. LRCLIB, looked up by title, artist, album and duration (for a file
   without tags, AcoustID can find the title and artist first),
5. Whisper on the vocal stem (auto-generated words and timing).

Synced lyrics beat plain ones: if 1-3 only have plain text, LRCLIB is still
asked, and when its synced text is the same song text, the words keep their
local source and only the timing comes from LRCLIB. Plain lyrics that stay
plain are aligned to the vocal stem with Whisper (align.py). Every line that
isn't English gets an offline machine translation (translate.py) and every
line in an Indian script a romanised form (romanize.py). Each of words /
timing / translation / transliteration records its own source.

`light` (--light) never loads a model: no Whisper, no alignment, no
translation; everything else works the same.

Runs after `analyze` (needs song.json for the beat grid) and on its own with
`qwertone-analyze lyrics SONG`.
"""

from __future__ import annotations

import datetime as _dt
import difflib
import hashlib
import json
import os
import shutil
import subprocess
from dataclasses import dataclass, field
from pathlib import Path

from . import __version__, acoustid, align, asr, languages, lrc, lrclib, romanize, translate
from .audio_io import AUDIO_EXTS, MIDI_EXTS
from .cache import default_cache_root, file_sha256, params_hash
from .languages import base, has_indic, is_latin_form
from .model import Timeline
from .translate import DEFAULT_MODEL as TRANSLATION_MODEL
from .progress import event, log

VERSION = 2                # 2: Whisper, alignment, translation, romanisation for every Indic script
FORMAT_VERSION = 1
MISS_TTL_DAYS = 7          # "LRCLIB / AcoustID has nothing" is re-asked after this long
LAST_LINE_SECONDS = 6.0    # end of the last line when the song length is unknown
IMPORT_DIR = "lyrics"
IMPORT_LRC = "imported.lrc"
IMPORT_META = "imported.json"
PROMPT_CHARS = 200         # lyrics given to Whisper as a hint when aligning


@dataclass
class LyricsOptions:
    online: bool = True                 # ask LRCLIB / AcoustID, download models
    refresh: bool = False               # ignore cached LRCLIB / AcoustID answers
    force: bool = False                 # rebuild lyrics.json even if up to date
    language: str | None = None         # override the song language (BCP 47: ta, te, hi, en, ta-Latn ...)
    title: str | None = None            # override what is looked up
    artist: str | None = None
    album: str | None = None
    cache_dir: str | None = None
    use_cache: bool = True
    timeout: float = 10.0
    models: bool = True                 # False = --light: no Whisper, alignment or translation
    whisper_model: str = "auto"         # auto = large-v3-turbo; tiny ... large-v3
    device: str = "auto"                # auto | cpu | cuda
    translate: bool = True              # English machine translation of non-English lines
    translation_model: str = TRANSLATION_MODEL
    acoustid: str = "auto"              # auto (untagged files) | always | never
    acoustid_key: str | None = None


@dataclass
class LyricsResult:
    path: Path
    status: str
    skipped: bool = False
    lines: int = 0
    words_source: str | None = None
    timing_source: str | None = None
    lrclib_requests: int = 0
    notes: list[str] = field(default_factory=list)
    language: str | None = None
    translated: int = 0


def now_iso() -> str:
    return _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds")


def offline_by_env() -> bool:
    return os.environ.get("QWERTONE_OFFLINE", "").lower() in ("1", "true", "yes")


# ============================================================================ finding the song folder

def find_song_dir(target: str | Path, songs_dir: Path) -> Path:
    """A song folder, its song.json, or the audio/MIDI file that was analysed."""
    p = Path(target).expanduser()
    if p.is_dir() and (p / "song.json").exists():
        return p.resolve()
    if p.is_file() and p.name == "song.json":
        return p.parent.resolve()
    if p.is_file() and p.suffix.lower() in AUDIO_EXTS | MIDI_EXTS:
        sha = file_sha256(p)
        for d in sorted(songs_dir.glob(f"*-{sha[:8]}")):
            try:
                with open(d / "song.json") as f:
                    if json.load(f).get("source", {}).get("sha256") == sha:
                        return d.resolve()
            except (OSError, json.JSONDecodeError):
                continue
        raise FileNotFoundError(f"{p.name} hasn't been analysed yet (no song folder in {songs_dir}); "
                                f"run: qwertone-analyze {p}")
    raise FileNotFoundError(f"{target}: not a song folder, song.json, or analysed audio/MIDI file")


# ============================================================================ local sources

@dataclass
class Candidate:
    source: dict                         # goes into lyrics.json "sources"
    doc: lrc.LrcDoc
    content_hash: str


def _hash_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def _read_text(path: Path) -> str:
    data = path.read_bytes()
    for enc in ("utf-8-sig", "utf-16", "cp1252"):
        try:
            return data.decode(enc)
        except UnicodeDecodeError:
            continue
    return data.decode("utf-8", errors="replace")


def _mtime_iso(path: Path) -> str:
    return _dt.datetime.fromtimestamp(path.stat().st_mtime, _dt.timezone.utc).isoformat(timespec="seconds")


def _file_candidate(path: Path, kind: str, name: str, fetched: str | None = None, original: str | None = None) -> Candidate | None:
    try:
        text = _read_text(path)
    except OSError:
        return None
    doc = lrc.parse(text)
    if not doc.lines:
        return None
    src = {"kind": kind, "name": name, "url": None, "file": original or str(path), "fetched": fetched or now_iso(),
           "modified": _mtime_iso(path), "sha256": _hash_text(text)}
    if doc.tags.get("by"):
        src["by"] = doc.tags["by"]
    if doc.tags.get("re"):
        src["tool"] = doc.tags["re"]
    if original:
        src["copy"] = f"{IMPORT_DIR}/{IMPORT_LRC}"
    return Candidate(src, doc, src["sha256"])


def imported_candidate(song_dir: Path) -> Candidate | None:
    p = song_dir / IMPORT_DIR / IMPORT_LRC
    if not p.exists():
        return None
    meta = {}
    try:
        with open(song_dir / IMPORT_DIR / IMPORT_META) as f:
            meta = json.load(f)
    except (OSError, json.JSONDecodeError):
        pass
    if meta.get("editor") == "qwertone":
        # Saved by the app's lyrics editor (Part 10b): a user edit, based on
        # whatever the lyrics came from before.
        c = _file_candidate(p, "user", "edited in Qwertone", fetched=meta.get("imported"))
        if c is not None:
            c.source["file"] = f"{IMPORT_DIR}/{IMPORT_LRC}"
            if meta.get("based_on"):
                c.source["based_on"] = meta["based_on"]
        return c
    return _file_candidate(p, "file", "imported .lrc", fetched=meta.get("imported"),
                           original=meta.get("original") or str(p))


def sidecar_candidate(audio: Path | None) -> Candidate | None:
    if audio is None:
        return None
    for ext in (".lrc", ".LRC", ".Lrc"):
        p = audio.with_suffix(ext)
        if p.exists():
            return _file_candidate(p, "file", ".lrc next to the song")
    return None


def _ffprobe_lyrics(audio: Path) -> list[tuple[str, str]]:
    if shutil.which("ffprobe") is None:
        return []
    res = subprocess.run(["ffprobe", "-v", "error", "-print_format", "json", "-show_format", "-show_streams",
                          str(audio)], capture_output=True)
    if res.returncode != 0:
        return []
    try:
        info = json.loads(res.stdout)
    except json.JSONDecodeError:
        return []
    found = []
    for block in [info.get("format", {})] + info.get("streams", []):
        for k, v in (block.get("tags") or {}).items():
            if "lyric" in k.lower() and isinstance(v, str) and v.strip():
                found.append((k, v))
    return found


def _sylt_lyrics(audio: Path) -> list[tuple[str, str]]:
    """ID3 SYLT (synced) frames as LRC text. Needs mutagen (optional)."""
    try:
        from mutagen.id3 import ID3, ID3NoHeaderError
    except ImportError:
        return []
    try:
        tags = ID3(str(audio))
    except (ID3NoHeaderError, Exception):  # noqa: BLE001 - any unreadable tag means "no SYLT"
        return []
    out = []
    for frame in tags.getall("SYLT"):
        if getattr(frame, "format", 2) != 2:   # 2 = milliseconds; 1 = MPEG frames (rare, skipped)
            continue
        lines = []
        for text, ms in frame.text:
            m, s = divmod(ms / 1000.0, 60)
            lines.append(f"[{int(m):02d}:{s:05.2f}]{text.strip()}")
        if lines:
            out.append((f"SYLT-{getattr(frame, 'lang', 'xxx')}", "\n".join(lines)))
    return out


def embedded_candidate(audio: Path | None) -> Candidate | None:
    if audio is None or audio.suffix.lower() not in AUDIO_EXTS:
        return None
    best = None
    for tag, text in _sylt_lyrics(audio) + _ffprobe_lyrics(audio):
        doc = lrc.parse(text)
        if not doc.lines:
            continue
        cand = Candidate({"kind": "embedded", "name": f"tags in {audio.name}", "url": None, "file": str(audio),
                          "tag": tag, "fetched": now_iso(), "sha256": _hash_text(text)}, doc, _hash_text(text))
        if doc.synced:
            return cand
        best = best or cand
    return best


# ============================================================================ LRCLIB with cache

def _lrclib_cache_path(cache_root: Path, song_sha: str, query: dict) -> Path:
    return cache_root / song_sha[:20] / f"lyrics-lrclib-{params_hash({'v': VERSION, **query})}.json"


def lrclib_candidate(query: dict, opts: LyricsOptions, song_sha: str, result: LyricsResult) -> Candidate | None:
    cache_root = Path(opts.cache_dir).expanduser() if opts.cache_dir else default_cache_root()
    cpath = _lrclib_cache_path(cache_root, song_sha, query)
    entry = None
    if opts.use_cache and not opts.refresh and cpath.exists():
        try:
            with open(cpath) as f:
                entry = json.load(f)
            age = _dt.datetime.now(_dt.timezone.utc) - _dt.datetime.fromisoformat(entry["fetched"])
            if entry.get("record") is None and age.days >= MISS_TTL_DAYS:
                entry = None
            else:
                log("lyrics", f"LRCLIB answer cached ({cpath.name})")
        except (OSError, json.JSONDecodeError, KeyError, ValueError):
            entry = None
    if entry is None:
        if not opts.online:
            return None
        log("lyrics", f"asking LRCLIB for {query['title']!r}" + (f" by {query['artist']}" if query.get("artist") else ""))
        result.lrclib_requests += 1
        try:
            rec = lrclib.lookup(query["title"], query.get("artist"), query.get("album"), query.get("duration"),
                                timeout=opts.timeout)
        except lrclib.LrclibError as e:
            log("lyrics", f"{e} (will retry next time)")
            result.notes.append(str(e))
            return None
        entry = {"fetched": now_iso(), "query": query, "record": rec}
        if opts.use_cache:
            cpath.parent.mkdir(parents=True, exist_ok=True)
            tmp = cpath.with_suffix(".tmp")
            with open(tmp, "w") as f:
                json.dump(entry, f, ensure_ascii=False)
            os.replace(tmp, cpath)
    rec = entry.get("record")
    if not rec:
        return None
    text = rec.get("syncedLyrics") or rec.get("plainLyrics") or ""
    src = {"kind": "lrclib", "name": "LRCLIB", "url": lrclib.record_url(rec), "file": None,
           "fetched": entry["fetched"], "id": rec.get("id"), "track": rec.get("trackName"),
           "artist": rec.get("artistName"), "album": rec.get("albumName"), "duration": rec.get("duration")}
    doc = lrc.parse(text) if text else lrc.LrcDoc()
    cand = Candidate(src, doc, _hash_text(text))
    cand.source["instrumental"] = bool(rec.get("instrumental")) and not text.strip()
    return cand


def untagged(song: dict) -> bool:
    meta = song.get("metadata", {}) or {}
    stem = Path(song.get("source", {}).get("file", "")).stem
    return not meta.get("artist") or not meta.get("title") or meta.get("title") == stem


def lookup_query(song: dict, opts: LyricsOptions, identified: dict | None = None) -> dict:
    meta = song.get("metadata", {}) or {}
    ident = identified or {}
    title = opts.title or ident.get("title") or meta.get("title") or \
        Path(song.get("source", {}).get("file", "song")).stem
    artist = opts.artist or ident.get("artist") or meta.get("artist")
    album = opts.album or ident.get("album") or meta.get("album")
    src_file = Path(song.get("source", {}).get("file", "")).stem
    if not artist and not opts.title and title == src_file and " - " in title:
        artist, title = (s.strip() for s in title.split(" - ", 1))    # "Artist - Title.mp3"
    return {"title": title, "artist": artist, "album": album, "duration": song.get("duration")}


# ============================================================================ language

# kept here for callers of 10a-lite; the logic lives in languages.py
line_language = languages.line_language
song_language = languages.song_language
TANGLISH_WORDS = languages.LATIN_WORDS["ta-Latn"]


# ============================================================================ AcoustID (untagged files)

def identify(song: dict, audio: Path | None, opts: LyricsOptions, sha: str, result: LyricsResult) -> dict | None:
    """Title/artist from AcoustID for files without tags (or always, with acoustid="always"). Cached."""
    if opts.acoustid == "never" or audio is None or audio.suffix.lower() not in AUDIO_EXTS:
        return None
    if opts.acoustid == "auto" and (not untagged(song) or opts.title):
        return None
    cache_root = Path(opts.cache_dir).expanduser() if opts.cache_dir else default_cache_root()
    cpath = cache_root / sha[:20] / "lyrics-acoustid.json"
    if opts.use_cache and not opts.refresh and cpath.exists():
        try:
            with open(cpath) as f:
                entry = json.load(f)
            age = _dt.datetime.now(_dt.timezone.utc) - _dt.datetime.fromisoformat(entry["fetched"])
            if entry.get("match") is not None or age.days < MISS_TTL_DAYS or not opts.online:
                return entry.get("match")
        except (OSError, json.JSONDecodeError, KeyError, ValueError):
            pass
    if not opts.online:
        return None
    key = acoustid.api_key(opts.acoustid_key)
    if not key:
        if opts.acoustid == "always":
            result.notes.append("AcoustID needs an API key (--acoustid-key, ACOUSTID_API_KEY, "
                                f"or {acoustid.key_file()}); free at https://acoustid.org/new-application")
        return None
    if not acoustid.have_fpcalc():
        result.notes.append("AcoustID skipped: fpcalc not found (sudo pacman -S chromaprint)")
        return None
    log("lyrics", "no title/artist tags: asking AcoustID")
    try:
        match = acoustid.lookup(audio, key, opts.timeout)
    except acoustid.AcoustidError as e:
        result.notes.append(str(e))
        return None
    entry = {"fetched": now_iso(), "match": match}
    if match:
        match["fetched"] = entry["fetched"]
    if opts.use_cache:
        cpath.parent.mkdir(parents=True, exist_ok=True)
        with open(cpath, "w") as f:
            json.dump(entry, f, ensure_ascii=False)
    return match


# ============================================================================ building lyrics.json

def _r(x: float | None, nd: int = 3) -> float | None:
    return None if x is None else round(float(x), nd)


def _romanized(text: str, lang: str) -> str | None:
    if has_indic(text):
        return romanize.romanize(text, lang)
    return text if is_latin_form(lang) else None


def build_lines(doc: lrc.LrcDoc, tl: Timeline | None, duration: float | None, lang_hint: str | None,
                words_doc: lrc.LrcDoc | None = None, confidence: list[float] | None = None) -> list[dict]:
    """Lines for lyrics.json. `words_doc` replaces the text line by line (same line count).
    `confidence` (generated lyrics/timing only) is copied onto each line."""
    lines = doc.lines
    out = []
    for i, ln in enumerate(lines):
        text = words_doc.lines[i].text if words_doc is not None else ln.text
        lang = languages.line_language(text, lang_hint)
        rom = _romanized(text, lang)
        start = end = None
        if doc.synced and ln.start is not None:
            start = ln.start
            nxt = next((l.start for l in lines[i + 1:] if l.start is not None and l.start > start), None)
            end = ln.end if ln.end is not None else nxt
            if end is None:
                end = duration if duration and duration > start else start + LAST_LINE_SECONDS
            if nxt is not None:
                end = min(end, nxt)
            end = max(end, start)
        entry = {
            "start": _r(start), "end": _r(end),
            "start_beat": _r(tl.beat_at(start)) if tl is not None and start is not None else None,
            "end_beat": _r(tl.beat_at(end)) if tl is not None and end is not None else None,
            "text": text,
            "language": lang,
            "romanized": rom,
            "tanglish": rom if base(lang) == "ta" else None,
            "translation": None,
            "words": None,
        }
        if confidence is not None and i < len(confidence):
            entry["confidence"] = confidence[i]
        if ln.words and words_doc is None and start is not None:
            ws = []
            for j, w in enumerate(ln.words):
                ws_end = w.end if w.end is not None else (ln.words[j + 1].start if j + 1 < len(ln.words) else end)
                ws_end = max(min(ws_end, end) if end is not None else ws_end, w.start)
                wr = _romanized(w.text, lang) if has_indic(w.text) else None
                ws.append({"text": w.text, "start": _r(w.start), "end": _r(ws_end),
                           "start_beat": _r(tl.beat_at(w.start)) if tl is not None else None,
                           "end_beat": _r(tl.beat_at(ws_end)) if tl is not None else None,
                           "romanized": wr, "tanglish": wr if base(lang) == "ta" else None})
            entry["words"] = ws
        out.append(entry)
    return out


def _texts_match(a: lrc.LrcDoc, b: lrc.LrcDoc) -> bool:
    if len(a.lines) != len(b.lines) or not a.lines:
        return False
    sims = [difflib.SequenceMatcher(None, lrclib.norm(x.text), lrclib.norm(y.text)).ratio()
            for x, y in zip(a.lines, b.lines)]
    return sum(sims) / len(sims) >= 0.8


def _timeline(song: dict) -> Timeline | None:
    beats = [b["time"] for b in song.get("beats", []) if "time" in b]
    if len(beats) < 2:
        return None
    flags = [bool(b.get("downbeat")) for b in song["beats"]]
    return Timeline(beats, flags, song.get("tempo", {}).get("beats_per_bar", 4))


def _source_path(song: dict) -> Path | None:
    p = song.get("source", {}).get("path")
    if p and Path(p).exists():
        return Path(p)
    return None


def vocal_audio(song_dir: Path, song: dict, audio: Path | None) -> tuple[Path | None, str | None]:
    """(file Whisper should hear, its song-folder name). The vocal stem, else the mix."""
    for st in song.get("stems", []) or []:
        if st.get("name") == "vocals" and (song_dir / st.get("file", "")).is_file():
            return song_dir / st["file"], st["file"]
    for p in sorted((song_dir / "stems").glob("vocals.*")) if (song_dir / "stems").is_dir() else []:
        return p, f"stems/{p.name}"
    if audio is not None and audio.suffix.lower() in AUDIO_EXTS:
        return audio, None
    return None, None


def _models_on(opts: LyricsOptions) -> bool:
    return opts.models and not light_by_env()


def light_by_env() -> bool:
    return os.environ.get("QWERTONE_LIGHT", "").lower() in ("1", "true", "yes")


def _whisper_lang(lang: str | None) -> str | None:
    """--language ta-Latn still means Whisper should listen for Tamil."""
    b = base(lang)
    return None if not b or b == "und" else b


def _asr_opts(opts: LyricsOptions) -> asr.AsrOptions:
    return asr.AsrOptions(model=opts.whisper_model, device=opts.device, offline=not opts.online,
                          cache_dir=Path(opts.cache_dir).expanduser() if opts.cache_dir else default_cache_root(),
                          use_cache=opts.use_cache)


def _generated_source(kind: str, name: str, file: str | None, **extra) -> dict:
    return {"kind": kind, "name": name, "url": None, "file": file, "fetched": now_iso(), **extra}


def _prompt(doc: lrc.LrcDoc) -> str:
    text = ""
    for l in doc.lines:
        if len(text) + len(l.text) > PROMPT_CHARS:
            break
        text += l.text + "\n"
    return text.strip()


def run_lyrics(song_dir: str | Path, opts: LyricsOptions | None = None) -> LyricsResult:
    opts = opts or LyricsOptions()
    if offline_by_env():
        opts.online = False
    song_dir = Path(song_dir)
    out_path = song_dir / "lyrics.json"
    with open(song_dir / "song.json") as f:
        song = json.load(f)
    sha = song.get("source", {}).get("sha256", "")
    audio = _source_path(song)
    tl = _timeline(song)
    duration = song.get("duration")
    result = LyricsResult(out_path, "not_found")
    identified = identify(song, audio, opts, sha, result)
    query = lookup_query(song, opts, identified)
    models = _models_on(opts)
    vocals, vocals_rel = vocal_audio(song_dir, song, audio)

    local = [c for c in (imported_candidate(song_dir), sidecar_candidate(audio), embedded_candidate(audio)) if c]
    beats_hash = params_hash({"beats": [b.get("time") for b in song.get("beats", [])], "duration": duration})
    fp = params_hash({
        "v": VERSION, "romanize": [romanize.VERSION, romanize.available()], "format": FORMAT_VERSION,
        "beats": beats_hash, "online": opts.online, "language": opts.language, "query": query,
        "local": [(c.source["kind"], c.source.get("file"), c.content_hash) for c in local],
        "models": models and [asr.VERSION, align.VERSION, asr.model_name(opts.whisper_model), asr.available(),
                              vocals_rel or (vocals.name if vocals else None)],
        "translate": models and opts.translate and [translate.VERSION, opts.translation_model,
                                                    translate.available()]})

    if out_path.exists() and not opts.force:
        try:
            with open(out_path) as f:
                old = json.load(f)
            same = old.get("analyzer", {}).get("fingerprint") == fp
            stale_miss = old.get("status") == "not_found" and opts.online and \
                (_dt.datetime.now(_dt.timezone.utc) - _dt.datetime.fromisoformat(old["analyzer"]["created"])).days >= MISS_TTL_DAYS
            incomplete = bool(old.get("analyzer", {}).get("incomplete"))
            if same and not stale_miss and not incomplete and not opts.refresh:
                log("lyrics", f"up to date ({old.get('status')}, {len(old.get('lines', []))} lines)")
                return LyricsResult(out_path, old.get("status", "not_found"), True, len(old.get("lines", [])),
                                    (old.get("sources", {}).get("words") or {}).get("kind"),
                                    (old.get("sources", {}).get("timing") or {}).get("kind"),
                                    language=old.get("language"))
        except (OSError, json.JSONDecodeError, KeyError, ValueError, TypeError):
            pass

    words_c = timing_c = None
    synced_local = next((c for c in local if c.doc.synced), None)
    plain_local = next((c for c in local if not c.doc.synced), None)
    instrumental_src = None
    if synced_local:
        words_c = timing_c = synced_local
    else:
        remote = lrclib_candidate(query, opts, sha, result) if (opts.online or opts.use_cache) else None
        if remote is not None and remote.source.get("instrumental"):
            instrumental_src = remote
        if plain_local:
            words_c = plain_local
            if remote is not None and remote.doc.synced and _texts_match(plain_local.doc, remote.doc):
                timing_c = remote
                log("lyrics", "local words, timing from LRCLIB (same text)")
            elif remote is not None and remote.doc.synced:
                result.notes.append("LRCLIB has synced lyrics but the text differs from the local file; "
                                    "kept the local words")
        elif remote is not None and remote.doc.lines:
            words_c = remote
            timing_c = remote if remote.doc.synced else None

    lang_tag = (words_c.doc.language if words_c else None) or None
    hint = opts.language or lang_tag
    incomplete = False
    whisper_res = None
    confidence = None
    language_source = "user" if opts.language else None

    def model_skip(what: str) -> None:
        nonlocal incomplete
        if not opts.models:
            result.notes.append(f"{what}: skipped (--light)")
        elif light_by_env():
            result.notes.append(f"{what}: skipped (QWERTONE_LIGHT)")
        elif not asr.available():
            result.notes.append(f"{what}: faster-whisper isn't installed (run setup.sh without --light)")
        elif vocals is None:
            result.notes.append(f"{what}: no audio to listen to (MIDI input)")
        else:
            incomplete = True

    # ---- plain lyrics: align them to the vocals
    if words_c is not None and timing_c is None:
        if models and asr.available() and vocals is not None:
            plain_lang = languages.song_language(
                [{"language": languages.line_language(l.text, hint)} for l in words_c.doc.lines], opts.language)
            try:
                whisper_res = asr.transcribe(vocals, sha, _asr_opts(opts), _whisper_lang(plain_lang),
                                             prompt=_prompt(words_c.doc))
                al = align.align(words_c.doc, asr.words(whisper_res), plain_lang, duration)
                if al.doc is not None:
                    name = f"Aligned to the {'vocal stem' if vocals_rel else 'mix'} with Whisper {whisper_res.get('model')}"
                    src = _generated_source("aligner", name, vocals_rel or str(vocals), model=whisper_res.get("model"),
                                            version=align.VERSION, matched=al.matched, auto_generated=True)
                    timing_c = Candidate(src, al.doc, "")
                    confidence = al.confidence
                    log("lyrics", f"aligned plain lyrics to the vocals ({al.matched:.0%} of words heard)")
                else:
                    result.notes.append(f"alignment failed: Whisper heard only {al.matched:.0%} of the lyric "
                                        "words (wrong lyrics, or no clear vocals); kept them plain")
            except (asr.AsrUnavailable, OSError, RuntimeError) as e:
                result.notes.append(f"alignment skipped: {e}")
                incomplete = True
        else:
            model_skip("alignment")

    # ---- nothing found: transcribe the vocals
    if words_c is None and instrumental_src is None:
        if models and asr.available() and vocals is not None:
            try:
                whisper_res = asr.transcribe(vocals, sha, _asr_opts(opts), _whisper_lang(opts.language))
                doc, confidence = asr.to_doc(whisper_res)
                if doc.lines:
                    model = whisper_res.get("model")
                    src = _generated_source(
                        "whisper", f"Whisper {model} (auto-generated)", vocals_rel or str(vocals), model=model,
                        language=whisper_res.get("language"), detected=whisper_res.get("detected"),
                        language_probability=whisper_res.get("language_probability"), auto_generated=True)
                    words_c = timing_c = Candidate(src, doc, "")
                    if not opts.language:
                        hint = whisper_res.get("language")
                        language_source = "whisper"
                    if not vocals_rel:
                        result.notes.append("no vocal stem (analysed with --no-separate or --light?): "
                                            "Whisper listened to the whole mix")
                else:
                    result.notes.append("Whisper heard no singing")
            except (asr.AsrUnavailable, OSError, RuntimeError) as e:
                result.notes.append(f"transcription skipped: {e}")
                incomplete = True
        else:
            model_skip("transcription")

    if words_c is None:
        lines, status = [], ("instrumental" if instrumental_src else "not_found")
    elif timing_c is not None:
        lines = build_lines(timing_c.doc, tl, duration, hint,
                            words_doc=words_c.doc if words_c is not timing_c and timing_c.source["kind"] != "aligner"
                            else None, confidence=confidence)
        status = "synced"
    else:
        lines = build_lines(words_c.doc, tl, duration, hint)
        status = "plain"

    # ---- English translation
    translation_src = None
    translated = 0
    if lines and opts.translate and any(translate.needs_translation(l["language"]) for l in lines):
        if models and translate.available():
            topts = translate.TranslateOptions(
                model=opts.translation_model, device=opts.device, offline=not opts.online,
                cache_dir=Path(opts.cache_dir).expanduser() if opts.cache_dir else default_cache_root(),
                use_cache=opts.use_cache)
            try:
                translated = translate.translate_lines(lines, sha, topts)
                if translated:
                    translation_src = {
                        "kind": "translator",
                        "name": f"{translate.model_label(opts.translation_model)} (machine translation)",
                        "url": translate.model_url(opts.translation_model), "file": None, "fetched": now_iso(),
                        "model": opts.translation_model, "target": translate.TARGET, "version": translate.VERSION,
                        "machine_translation": True, "auto_generated": True}
            except (translate.TranslatorUnavailable, OSError, RuntimeError) as e:
                result.notes.append(f"translation skipped: {e}")
                incomplete = True
        elif not models:
            result.notes.append("translation: skipped (--light)")
        else:
            result.notes.append("translation: ctranslate2/sentencepiece aren't installed (run setup.sh without --light)")

    romanized = any(l["romanized"] and has_indic(l["text"]) for l in lines)
    if romanized:
        rom_src = {"kind": "generated", "name": "Aksharamukha romanisation", "url": "https://aksharamukha.appspot.com",
                   "file": None, "fetched": now_iso(), "version": romanize.VERSION,
                   "library": f"{romanize.LIBRARY} {romanize.library_version() or ''}".strip(),
                   "style": "RomanReadable + Qwertone rules (Tanglish / Tenglish / Hinglish)"}
    else:
        rom_src = None
    words_src = words_c.source if words_c else (instrumental_src.source if instrumental_src else None)
    song_lang = languages.song_language(lines, opts.language, whisper_res.get("language") if whisper_res else None,
                                        lang_tag) if lines else (opts.language or lang_tag or "und")
    if language_source is None:
        language_source = "text" if lines else ("tag" if lang_tag else "none")
    doc = {
        "format": "qwertone-lyrics",
        "format_version": FORMAT_VERSION,
        "analyzer": {"version": __version__, "lyrics_version": VERSION, "created": now_iso(), "fingerprint": fp,
                     "online": opts.online, "models": models, "incomplete": incomplete},
        "song": {"title": query["title"], "artist": query["artist"], "album": query["album"],
                 "duration": duration, "sha256": sha,
                 "identified_by": ({"kind": "acoustid", "name": "AcoustID", "url": "https://acoustid.org/track/"
                                    + identified["acoustid"] if identified.get("acoustid") else None,
                                    **{k: identified.get(k) for k in ("title", "artist", "album", "score",
                                                                      "recording_id", "fetched")}}
                                   if identified else None)},
        "status": status,
        "language": song_lang,
        "languages": languages.song_languages(lines),
        "language_source": language_source,
        "has_word_timing": any(l["words"] for l in lines),
        "auto_generated": {
            "words": bool(words_c and words_c.source["kind"] == "whisper"),
            "timing": bool(timing_c and timing_c.source["kind"] in ("whisper", "aligner")),
            "translation": translation_src is not None,
        },
        "offset": _r(timing_c.doc.offset) if timing_c else 0.0,
        "sources": {
            "words": words_src,
            "timing": timing_c.source if timing_c else None,
            "translation": translation_src,
            "transliteration": rom_src,
        },
        "lines": lines,
    }
    if whisper_res is not None:
        doc["whisper"] = {"model": whisper_res.get("model"), "language": whisper_res.get("language"),
                          "detected": whisper_res.get("detected"),
                          "language_probability": whisper_res.get("language_probability"),
                          "top_languages": whisper_res.get("top_languages"),
                          "per_window_language": whisper_res.get("multilingual", False)}
    song_dir.mkdir(parents=True, exist_ok=True)
    tmp = out_path.with_suffix(".json.tmp")
    with open(tmp, "w") as f:
        json.dump(doc, f, indent=1, ensure_ascii=False)
    os.replace(tmp, out_path)

    result.status = status
    result.lines = len(lines)
    result.language = song_lang
    result.translated = translated
    result.words_source = words_src["kind"] if words_src else None
    result.timing_source = timing_c.source["kind"] if timing_c else None
    src_name = words_c.source["name"] if words_c else "nowhere"
    log("lyrics", f"{status}: {len(lines)} lines ({song_lang}) from {src_name}" +
        (f", timing from {timing_c.source['name']}" if timing_c and timing_c is not words_c else "") +
        (f", {translated} translated" if translated else ""))
    for n in result.notes:
        log("lyrics", n)
    event("lyrics", status=status, lines=len(lines), path=str(out_path), language=song_lang,
          auto_generated=doc["auto_generated"])
    return result


def import_lrc(lrc_path: str | Path, song_dir: str | Path, opts: LyricsOptions | None = None) -> LyricsResult:
    """Copy an .lrc into the song folder (it then wins over every other source) and rebuild lyrics.json."""
    lrc_path = Path(lrc_path).expanduser().resolve()
    song_dir = Path(song_dir)
    doc = lrc.parse(_read_text(lrc_path))
    if not doc.lines:
        raise ValueError(f"{lrc_path.name} has no lyric lines")
    d = song_dir / IMPORT_DIR
    d.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(lrc_path, d / IMPORT_LRC)
    with open(d / IMPORT_META, "w") as f:
        json.dump({"original": str(lrc_path), "imported": now_iso(), "sha256": file_sha256(lrc_path)}, f, indent=1)
    log("lyrics", f"imported {lrc_path.name} ({len(doc.lines)} lines, {'synced' if doc.synced else 'plain'})")
    opts = opts or LyricsOptions()
    opts.force = True
    return run_lyrics(song_dir, opts)
