"""AcoustID: find title / artist of an untagged file from its sound.

`fpcalc` (Chromaprint; `sudo pacman -S chromaprint`) fingerprints the first
two minutes, the AcoustID web service returns the MusicBrainz recordings it
matches. Used only to look up lyrics (LRCLIB needs a title and artist), and
by default only when the file has no artist tag or its title is just the
file name. Needs a free application API key from
https://acoustid.org/new-application, given as `--acoustid-key`, the
ACOUSTID_API_KEY environment variable, or the file
~/.config/qwertone/acoustid_key. Answers are cached per song.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

from . import __version__
from .progress import log

API = "https://api.acoustid.org/v2/lookup"
MIN_SCORE = 0.5
USER_AGENT = f"Qwertone/{__version__} (https://github.com/M-Nharen/Music_App)"


class AcoustidError(RuntimeError):
    pass


def key_file() -> Path:
    base = os.environ.get("XDG_CONFIG_HOME") or os.path.join(Path.home(), ".config")
    return Path(base) / "qwertone" / "acoustid_key"


def api_key(given: str | None = None) -> str | None:
    if given:
        return given.strip()
    if os.environ.get("ACOUSTID_API_KEY"):
        return os.environ["ACOUSTID_API_KEY"].strip()
    try:
        return key_file().read_text().strip() or None
    except OSError:
        return None


def have_fpcalc() -> bool:
    return shutil.which("fpcalc") is not None


def fingerprint(path: Path) -> tuple[float, str]:
    """(duration, fingerprint) from fpcalc."""
    if not have_fpcalc():
        raise AcoustidError("fpcalc not found (sudo pacman -S chromaprint)")
    res = subprocess.run(["fpcalc", "-json", str(path)], capture_output=True, text=True)
    if res.returncode != 0:
        raise AcoustidError(f"fpcalc failed: {res.stderr.strip()[:200]}")
    data = json.loads(res.stdout)
    return float(data["duration"]), data["fingerprint"]


def http_post(url: str, data: dict, timeout: float = 10.0) -> tuple[int, bytes]:
    body = urllib.parse.urlencode(data).encode()
    req = urllib.request.Request(url, data=body, headers={"User-Agent": USER_AGENT,
                                                          "Content-Type": "application/x-www-form-urlencoded"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, r.read()
    except urllib.error.HTTPError as e:
        return e.code, e.read()
    except (urllib.error.URLError, OSError, TimeoutError) as e:
        raise AcoustidError(f"AcoustID unreachable: {e}") from e


def lookup(path: Path, key: str, timeout: float = 10.0) -> dict | None:
    """Best match as {"title", "artist", "album", "duration", "score", "recording_id", "acoustid"} or None."""
    duration, fp = fingerprint(path)
    status, body = http_post(API, {"client": key, "meta": "recordings releasegroups compress",
                                   "duration": str(int(round(duration))), "fingerprint": fp}, timeout)
    try:
        data = json.loads(body or b"{}")
    except json.JSONDecodeError as e:
        raise AcoustidError(f"AcoustID: bad answer (HTTP {status})") from e
    if status != 200 or data.get("status") != "ok":
        msg = (data.get("error") or {}).get("message") or f"HTTP {status}"
        raise AcoustidError(f"AcoustID: {msg}")
    return best_match(data.get("results") or [], duration)


def best_match(results: list[dict], duration: float | None) -> dict | None:
    best, best_key = None, None
    for r in results:
        score = float(r.get("score") or 0)
        if score < MIN_SCORE:
            continue
        for rec in r.get("recordings") or []:
            if not rec.get("title") or not rec.get("artists"):
                continue
            d = rec.get("duration")
            off = abs(d - duration) if d and duration else 5.0
            k = (score, -min(off, 30.0), len(rec.get("releasegroups") or []))
            if best_key is None or k > best_key:
                groups = rec.get("releasegroups") or []
                album = next((g.get("title") for g in groups if g.get("type") == "Album"), None) or \
                    (groups[0].get("title") if groups else None)
                artist = "".join(a["name"] + (a.get("joinphrase") or "") for a in rec["artists"]).strip()
                best_key = k
                best = {"title": rec["title"], "artist": artist, "album": album, "duration": d,
                        "score": round(score, 3), "recording_id": rec.get("id"), "acoustid": r.get("id")}
    if best:
        log("lyrics", f"AcoustID: {best['title']!r} by {best['artist']} ({best['score']:.0%})")
    return best
