"""Assemble the song folder (see SONG_FORMAT.md)."""

from __future__ import annotations

import datetime as _dt
import json
import os
import re
import shutil
from pathlib import Path

from . import __version__
from .midi_writer import write_midi
from .model import Chord, KeyInfo, Part, Section, Timeline

FORMAT_VERSION = 1


def default_songs_dir() -> Path:
    base = os.environ.get("XDG_DATA_HOME") or os.path.join(Path.home(), ".local", "share")
    return Path(base) / "qwertone" / "songs"


def slugify(text: str) -> str:
    s = re.sub(r"[^\w\s-]", "", text.lower(), flags=re.UNICODE)
    s = re.sub(r"[\s_-]+", "-", s).strip("-")
    return s[:60] or "song"


def part_id(text: str) -> str:
    """ASCII-only id for parts (used as a file name in midi/)."""
    import unicodedata
    s = unicodedata.normalize("NFKD", text).encode("ascii", "ignore").decode()
    s = re.sub(r"[^a-z0-9]+", "-", s.lower()).strip("-")
    return s[:40] or "part"


def _link_or_copy(src: Path, dst: Path) -> None:
    dst.parent.mkdir(parents=True, exist_ok=True)
    if dst.exists():
        dst.unlink()
    try:
        os.link(src, dst)  # same filesystem: no extra disk space
    except OSError:
        shutil.copy2(src, dst)


def write_song(out_dir: Path, *, source: dict, metadata: dict, analyzer: dict, rhythm: dict,
               tl: Timeline, key: KeyInfo | None, chords: list[Chord], sections: list[Section],
               parts: list[Part], stems: dict[str, Path], low_conf: float,
               simplified: dict | None = None) -> Path:
    out_dir.mkdir(parents=True, exist_ok=True)

    # stems
    stem_entries = []
    for name, src in stems.items():
        rel = Path("stems") / f"{name}{src.suffix}"
        _link_or_copy(src, out_dir / rel)
        stem_entries.append({"name": name, "file": rel.as_posix()})

    # MIDI: everything in song.mid, plus one file per part
    track_of = write_midi(out_dir / "song.mid", parts, tl, key=key, chords=chords, sections=sections)
    midi_dir = out_dir / "midi"
    if midi_dir.exists():
        shutil.rmtree(midi_dir)
    midi_dir.mkdir()
    for p in parts:
        write_midi(midi_dir / f"{p.id}.mid", [p], tl, key=key)

    bars = tl.bar_numbers()
    beats_json = [
        {"time": round(float(t), 4), "bar": int(bars[i]),
         "beat": int(_beat_in_bar(i, tl)), "downbeat": bool(tl.downbeat[i])}
        for i, t in enumerate(tl.beats)
    ]
    parts_json = []
    channel_of = _channels(parts)
    for p in parts:
        notes = sorted(p.notes, key=lambda n: (n.start, n.pitch))
        parts_json.append({
            "id": p.id,
            "name": p.name,
            "role": p.role,
            "method": p.method,
            "stem": p.stem,
            "is_drum": p.is_drum,
            "program": int(p.program),
            "midi_track": track_of[p.id],
            "midi_channel": channel_of[p.id],
            "midi_file": f"midi/{p.id}.mid",
            "transpose": int(p.transpose),
            "simplified": p.simplified,
            "note_count": len(notes),
            "low_confidence_count": sum(1 for n in notes if n.confidence < low_conf),
            "pitch_range": [min((n.pitch for n in notes), default=None), max((n.pitch for n in notes), default=None)],
            "notes": [n.to_dict(tl, low_conf) for n in notes],
        })

    song = {
        "format": "qwertone-song",
        "format_version": FORMAT_VERSION,
        "analyzer": {**analyzer, "version": __version__,
                     "created": _dt.datetime.now(_dt.timezone.utc).isoformat(timespec="seconds")},
        "source": source,
        "metadata": metadata,
        "duration": round(float(source["duration"]), 4),
        "low_confidence_threshold": low_conf,
        "stems": stem_entries,
        "midi": "song.mid",
        "tempo": {
            "bpm": rhythm["bpm"],
            "beats_per_bar": rhythm["beats_per_bar"],
            "time_signature": [rhythm["beats_per_bar"], 4],
            "confidence": rhythm["tempo_confidence"],
            "downbeat_confidence": rhythm["downbeat_confidence"],
        },
        "beats": beats_json,
        "key": key.to_dict() if key else None,
        "chords": [c.to_dict(tl, key, low_conf) for c in chords],
        "sections": [s.to_dict(tl) for s in sections],
        "parts": parts_json,
        "simplified": None,
    }
    if simplified is not None:
        skey: KeyInfo | None = simplified["key"]
        song["simplified"] = {
            **simplified["info"],
            "key": skey.to_dict() if skey else None,
            "chords": [c.to_dict(tl, skey, low_conf) for c in simplified["chords"]],
            "parts": [p.id for p in simplified["parts"]],
        }

    tmp = out_dir / "song.json.tmp"
    with open(tmp, "w") as f:
        json.dump(song, f, indent=1, ensure_ascii=False)
    os.replace(tmp, out_dir / "song.json")
    return out_dir / "song.json"


def _beat_in_bar(i: int, tl: Timeline) -> int:
    """1-based position in the bar (pickup beats count up to the first downbeat)."""
    db = list(tl.downbeat[: i + 1])
    if True in db:
        last = len(db) - 1 - db[::-1].index(True)
        return i - last + 1
    first = list(tl.downbeat).index(True) if tl.downbeat.any() else len(tl.downbeat)
    return max(1, tl.beats_per_bar - (first - i) + 1)


def _channels(parts: list[Part]) -> dict[str, int]:
    out, ch = {}, 0
    for p in parts:
        if p.is_drum:
            out[p.id] = 9
        else:
            out[p.id] = min(ch, 15)
            ch = ch + 1 if ch + 1 != 9 else 10
    return out
