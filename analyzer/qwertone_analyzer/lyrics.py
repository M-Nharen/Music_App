"""The lyrics step: song folder -> lyrics.json (see SONG_FORMAT.md, "lyrics.json").

Where lyrics come from, first match wins:

1. an .lrc imported into the song folder (`qwertone-analyze import-lrc`),
2. an .lrc next to the audio file (`Song.mp3` -> `Song.lrc`),
3. lyrics stored in the audio file's tags (USLT / LYRICS / ©lyr, and ID3 SYLT
   synced frames when mutagen is installed),
4. LRCLIB, looked up by title, artist, album and duration.

Synced lyrics beat plain ones: if 1-3 only have plain text, LRCLIB is still
asked, and when its synced text is the same song text, the words keep their
local source and only the timing comes from LRCLIB. Each of words / timing /
translation / transliteration records its own source.

Runs after `analyze` (needs song.json for the beat grid) and on its own with
`qwertone-analyze lyrics SONG`.
"""

from __future__ import annotations

import datetime as _dt
import difflib
import hashlib
import json
import os
import re
import shutil
import subprocess
from dataclasses import dataclass, field
from pathlib import Path

from . import __version__, lrc, lrclib, tanglish
from .audio_io import AUDIO_EXTS, MIDI_EXTS
from .cache import default_cache_root, file_sha256, params_hash
from .model import Timeline
from .progress import event, log

VERSION = 1
FORMAT_VERSION = 1
MISS_TTL_DAYS = 7          # "LRCLIB has nothing" is re-asked after this long
LAST_LINE_SECONDS = 6.0    # end of the last line when the song length is unknown
IMPORT_DIR = "lyrics"
IMPORT_LRC = "imported.lrc"
IMPORT_META = "imported.json"


@dataclass
class LyricsOptions:
    online: bool = True                 # ask LRCLIB
    refresh: bool = False               # ignore cached LRCLIB answers
    force: bool = False                 # rebuild lyrics.json even if up to date
    language: str | None = None         # override the song language (BCP 47: ta, en, ta-Latn ...)
    title: str | None = None            # override what is looked up
    artist: str | None = None
    album: str | None = None
    cache_dir: str | None = None
    use_cache: bool = True
    timeout: float = 10.0


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


def lookup_query(song: dict, opts: LyricsOptions) -> dict:
    meta = song.get("metadata", {}) or {}
    title = opts.title or meta.get("title") or Path(song.get("source", {}).get("file", "song")).stem
    artist = opts.artist or meta.get("artist")
    album = opts.album or meta.get("album")
    src_file = Path(song.get("source", {}).get("file", "")).stem
    if not artist and not opts.title and title == src_file and " - " in title:
        artist, title = (s.strip() for s in title.split(" - ", 1))    # "Artist - Title.mp3"
    return {"title": title, "artist": artist, "album": album, "duration": song.get("duration")}


# ============================================================================ language

TANGLISH_WORDS = set("""
naan nee neeyum ennai unnai enna enakku unakku ennoda unnoda en un enge inge ange ippo eppo eppadi yaar
illa illai irukku iruku irundhu vaa vaadi vaada po podi poda di da thaan dhaan kooda mattum romba
kaadhal kadhal kaathal anbe anbu uyire uyir nenjam nenje kannu kanne kannil kangal penne ponnu paiyan
vazhkai vaazhkai pesu pesum paadu paattu paatu solla sollu sonna vandha vandhen pogum poren varen
manasu manasula manam nila vaanam mazhai kaatru kaathu thendral malar poo pookkal kanavu kanavil
azhagu azhagaa azhage ennamo yedho edho ellam ellaam onnu rendu konjam nalla ketta thirumba
""".split())


def line_language(text: str, song_hint: str | None = None) -> str:
    letters = [c for c in text if c.isalpha()]
    if not letters:
        return song_hint or "und"
    tamil = sum(1 for c in letters if "஀" <= c <= "௿")
    if tamil / len(letters) >= 0.3:
        return "ta"
    for lang, lo, hi in (("hi", "ऀ", "ॿ"), ("te", "ఀ", "౿"), ("kn", "ಀ", "೿"),
                         ("ml", "ഀ", "ൿ")):
        if sum(1 for c in letters if lo <= c <= hi) / len(letters) >= 0.3:
            return lang
    latin = sum(1 for c in letters if c.isascii())
    if latin / len(letters) >= 0.7:
        if song_hint == "ta-Latn":
            return "ta-Latn"
        words = re.findall(r"[a-z]+", text.lower())
        hits = sum(1 for w in words if w in TANGLISH_WORDS)
        if words and (hits >= 2 or hits / len(words) >= 0.25):
            return "ta-Latn"
        return "en"
    return "und"


def song_language(lines: list[dict], override: str | None, tag: str | None) -> str:
    if override:
        return override
    counts: dict[str, int] = {}
    for l in lines:
        counts[l["language"]] = counts.get(l["language"], 0) + 1
    counts.pop("und", None)
    if counts:
        # a Tamil song with a few English lines is still a Tamil song
        if counts.get("ta", 0) + counts.get("ta-Latn", 0) >= 0.3 * sum(counts.values()):
            return "ta" if counts.get("ta", 0) >= counts.get("ta-Latn", 0) else "ta-Latn"
        return max(counts, key=counts.get)
    return tag or "und"


# ============================================================================ building lyrics.json

def _r(x: float | None, nd: int = 3) -> float | None:
    return None if x is None else round(float(x), nd)


def build_lines(doc: lrc.LrcDoc, tl: Timeline | None, duration: float | None, lang_hint: str | None,
                words_doc: lrc.LrcDoc | None = None) -> list[dict]:
    """Lines for lyrics.json. `words_doc` replaces the text line by line (same line count)."""
    lines = doc.lines
    out = []
    for i, ln in enumerate(lines):
        text = words_doc.lines[i].text if words_doc is not None else ln.text
        lang = line_language(text, lang_hint)
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
            "tanglish": tanglish.transliterate(text) if lang == "ta" or tanglish.has_tamil(text)
            else (text if lang == "ta-Latn" else None),
            "translation": None,
            "words": None,
        }
        if ln.words and words_doc is None and start is not None:
            ws = []
            for j, w in enumerate(ln.words):
                ws_end = w.end if w.end is not None else (ln.words[j + 1].start if j + 1 < len(ln.words) else end)
                ws_end = max(min(ws_end, end) if end is not None else ws_end, w.start)
                ws.append({"text": w.text, "start": _r(w.start), "end": _r(ws_end),
                           "start_beat": _r(tl.beat_at(w.start)) if tl is not None else None,
                           "end_beat": _r(tl.beat_at(ws_end)) if tl is not None else None,
                           "tanglish": tanglish.transliterate(w.text) if tanglish.has_tamil(w.text) else None})
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
    query = lookup_query(song, opts)
    result = LyricsResult(out_path, "not_found")

    local = [c for c in (imported_candidate(song_dir), sidecar_candidate(audio), embedded_candidate(audio)) if c]
    beats_hash = params_hash({"beats": [b.get("time") for b in song.get("beats", [])], "duration": duration})
    fp = params_hash({"v": VERSION, "tanglish": tanglish.VERSION, "format": FORMAT_VERSION, "beats": beats_hash,
                      "online": opts.online, "language": opts.language, "query": query,
                      "local": [(c.source["kind"], c.source.get("file"), c.content_hash) for c in local]})

    if out_path.exists() and not opts.force:
        try:
            with open(out_path) as f:
                old = json.load(f)
            same = old.get("analyzer", {}).get("fingerprint") == fp
            stale_miss = old.get("status") == "not_found" and opts.online and \
                (_dt.datetime.now(_dt.timezone.utc) - _dt.datetime.fromisoformat(old["analyzer"]["created"])).days >= MISS_TTL_DAYS
            if same and not stale_miss and not opts.refresh:
                log("lyrics", f"up to date ({old.get('status')}, {len(old.get('lines', []))} lines)")
                return LyricsResult(out_path, old.get("status", "not_found"), True, len(old.get("lines", [])),
                                    (old.get("sources", {}).get("words") or {}).get("kind"),
                                    (old.get("sources", {}).get("timing") or {}).get("kind"))
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
                                    "kept the local (unsynced) lyrics")
        elif remote is not None and remote.doc.lines:
            words_c = remote
            timing_c = remote if remote.doc.synced else None

    lang_tag = (words_c.doc.language if words_c else None) or None
    hint = opts.language or lang_tag
    if words_c is None:
        lines, status = [], ("instrumental" if instrumental_src else "not_found")
    elif timing_c is not None:
        lines = build_lines(timing_c.doc, tl, duration, hint,
                            words_doc=words_c.doc if words_c is not timing_c else None)
        status = "synced"
    else:
        lines = build_lines(words_c.doc, tl, duration, hint)
        status = "plain"

    has_tamil = any(tanglish.has_tamil(l["text"]) for l in lines)
    doc = {
        "format": "qwertone-lyrics",
        "format_version": FORMAT_VERSION,
        "analyzer": {"version": __version__, "lyrics_version": VERSION, "created": now_iso(), "fingerprint": fp,
                     "online": opts.online},
        "song": {"title": query["title"], "artist": query["artist"], "album": query["album"],
                 "duration": duration, "sha256": sha},
        "status": status,
        "language": song_language(lines, opts.language, lang_tag) if lines else (opts.language or lang_tag or "und"),
        "has_word_timing": any(l["words"] for l in lines),
        "offset": _r(timing_c.doc.offset) if timing_c else 0.0,
        "sources": {
            "words": words_c.source if words_c else (instrumental_src.source if instrumental_src else None),
            "timing": timing_c.source if timing_c else None,
            "translation": None,
            "transliteration": {"kind": "generated", "name": "Qwertone Tanglish", "url": None, "file": None,
                                "fetched": now_iso(), "version": tanglish.VERSION} if has_tamil else None,
        },
        "lines": lines,
    }
    song_dir.mkdir(parents=True, exist_ok=True)
    tmp = out_path.with_suffix(".json.tmp")
    with open(tmp, "w") as f:
        json.dump(doc, f, indent=1, ensure_ascii=False)
    os.replace(tmp, out_path)

    result.status = status
    result.lines = len(lines)
    result.words_source = doc["sources"]["words"]["kind"] if doc["sources"]["words"] else None
    result.timing_source = timing_c.source["kind"] if timing_c else None
    src_name = words_c.source["name"] if words_c else "nowhere"
    log("lyrics", f"{status}: {len(lines)} lines from {src_name}" +
        (f", timing from {timing_c.source['name']}" if timing_c and timing_c is not words_c else ""))
    event("lyrics", status=status, lines=len(lines), path=str(out_path))
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
