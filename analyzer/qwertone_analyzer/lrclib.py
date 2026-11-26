"""LRCLIB (https://lrclib.net): free, open database of synced lyrics.

`lookup()` tries `/api/get` (exact title + artist + duration, LRCLIB allows
±2 s), then `/api/search`, and picks the closest match by title, artist and
duration, preferring synced lyrics. Only stdlib networking, so no new
dependency. Tests replace `http_get`.
"""

from __future__ import annotations

import difflib
import json
import re
import urllib.error
import urllib.parse
import urllib.request

from . import __version__

API = "https://lrclib.net/api"
USER_AGENT = f"qwertone-analyzer/{__version__} (https://github.com/M-Nharen/Music_App)"
DURATION_TOLERANCE = 3.0  # seconds, for search results


class LrclibError(RuntimeError):
    """Network or server trouble (not 'not found'). Not cached, so the next run retries."""


def http_get(url: str, timeout: float = 10.0) -> tuple[int, bytes]:
    req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, "Accept": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read() if e.fp else b""
    except (urllib.error.URLError, TimeoutError, OSError) as e:
        raise LrclibError(f"LRCLIB not reachable: {getattr(e, 'reason', e)}") from e


def _get_json(path: str, params: dict, timeout: float):
    url = f"{API}/{path}?" + urllib.parse.urlencode({k: v for k, v in params.items() if v not in (None, "")})
    status, body = http_get(url, timeout)
    if status == 404:
        return None
    if status != 200:
        raise LrclibError(f"LRCLIB answered HTTP {status} for {url}")
    try:
        return json.loads(body.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as e:
        raise LrclibError(f"LRCLIB sent something that isn't JSON ({e})") from e


def norm(s: str | None) -> str:
    s = (s or "").lower()
    s = re.sub(r"[\(\[].*?[\)\]]", " ", s)               # (From "Movie"), [Official Video]
    s = re.sub(r"\b(feat|ft)\.?\s.*$", " ", s)
    s = re.sub(r"[^\w\s]", " ", s, flags=re.UNICODE)
    return re.sub(r"\s+", " ", s).strip()


def clean_title(title: str) -> str:
    """Drop video-site junk; keeps the real name for searching."""
    t = re.sub(r"[\(\[][^\)\]]*(official|lyric|video|audio|hd|4k|full song|remaster)[^\)\]]*[\)\]]", "",
               title, flags=re.I)
    return re.sub(r"\s+", " ", t).strip(" -_") or title


def _similar(a: str | None, b: str | None) -> float:
    a, b = norm(a), norm(b)
    if not a or not b:
        return 0.0
    if a == b or a in b or b in a:
        return 1.0
    return difflib.SequenceMatcher(None, a, b).ratio()


def score(rec: dict, title: str, artist: str | None, duration: float | None) -> float | None:
    """Higher is better; None = not this song."""
    ts = _similar(rec.get("trackName") or rec.get("name"), title)
    if ts < 0.6:
        return None
    s = 2.0 * ts
    if artist:
        s += _similar(rec.get("artistName"), artist)
    if duration and rec.get("duration"):
        dd = abs(float(rec["duration"]) - duration)
        if dd > DURATION_TOLERANCE:
            return None
        s -= dd / DURATION_TOLERANCE
    if rec.get("syncedLyrics"):
        s += 1.0
    elif rec.get("plainLyrics"):
        s += 0.3
    return s


def lookup(title: str, artist: str | None = None, album: str | None = None, duration: float | None = None,
           timeout: float = 10.0) -> dict | None:
    """Best LRCLIB record for the song, or None. Raises LrclibError on network trouble."""
    if not title:
        return None
    if artist and duration:
        rec = _get_json("get", {"track_name": title, "artist_name": artist, "album_name": album,
                                "duration": int(round(duration))}, timeout)
        if rec and (rec.get("syncedLyrics") or rec.get("instrumental")):
            return rec
        best_plain = rec
    else:
        best_plain = None

    queries = []
    if artist:
        queries.append({"track_name": clean_title(title), "artist_name": artist})
    queries.append({"q": f"{clean_title(title)} {artist or ''}".strip()})
    if norm(clean_title(title)) != norm(title) or artist:
        queries.append({"track_name": clean_title(title)})
    seen: dict[int, dict] = {}
    for q in queries:
        res = _get_json("search", q, timeout) or []
        for r in res if isinstance(res, list) else []:
            if isinstance(r, dict) and "id" in r:
                seen.setdefault(r["id"], r)
        ranked = sorted(((score(r, title, artist, duration), r) for r in seen.values()),
                        key=lambda x: -(x[0] if x[0] is not None else -1e9))
        ranked = [(s, r) for s, r in ranked if s is not None]
        if ranked and ranked[0][1].get("syncedLyrics"):
            return ranked[0][1]
    ranked = [(s, r) for s, r in ((score(r, title, artist, duration), r) for r in seen.values()) if s is not None]
    if ranked:
        return max(ranked, key=lambda x: x[0])[1]
    return best_plain


def record_url(rec: dict) -> str:
    return f"{API}/get/{rec['id']}"
