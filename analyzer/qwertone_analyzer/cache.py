"""Per-song cache so no stage runs twice for the same input and settings.

Layout: <cache_root>/<sha256 of the input file, 20 chars>/
            <stage>-<params hash>.json   results of analysis stages
            <stage>-<params hash>.npz    arrays (decoded audio, pitch tracks)
            stems-<params hash>/         separated stems as FLAC

The key of each entry is the stage name + its version + every parameter that
changes the result, so changing an option only recomputes what depends on it.
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
from pathlib import Path
from typing import Any, Callable

import numpy as np

from .progress import log


def default_cache_root() -> Path:
    base = os.environ.get("XDG_CACHE_HOME") or os.path.join(Path.home(), ".cache")
    return Path(base) / "qwertone" / "analyzer"


def file_sha256(path: str | Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1 << 20), b""):
            h.update(block)
    return h.hexdigest()


def params_hash(params: dict[str, Any]) -> str:
    blob = json.dumps(params, sort_keys=True, default=str).encode()
    return hashlib.sha256(blob).hexdigest()[:12]


class Cache:
    def __init__(self, root: Path | None, song_hash: str, enabled: bool = True, refresh: bool = False):
        self.enabled = enabled and root is not None
        self.refresh = refresh  # recompute and overwrite
        self.dir = (root or default_cache_root()) / song_hash[:20]
        self.hits: list[str] = []
        self.misses: list[str] = []
        if self.enabled:
            self.dir.mkdir(parents=True, exist_ok=True)

    def _path(self, stage: str, params: dict, ext: str) -> Path:
        return self.dir / f"{stage}-{params_hash(params)}{ext}"

    def json(self, stage: str, params: dict, compute: Callable[[], Any]) -> Any:
        p = self._path(stage, params, ".json")
        if self.enabled and not self.refresh and p.exists():
            try:
                with open(p) as f:
                    value = json.load(f)
                self.hits.append(stage)
                log(stage, f"cached ({p.name})")
                return value
            except (OSError, json.JSONDecodeError):
                pass
        self.misses.append(stage)
        value = compute()
        if self.enabled:
            tmp = p.with_suffix(".tmp")
            with open(tmp, "w") as f:
                json.dump(value, f)
            os.replace(tmp, p)
        return value

    def arrays(self, stage: str, params: dict, compute: Callable[[], dict[str, np.ndarray]]) -> dict[str, np.ndarray]:
        p = self._path(stage, params, ".npz")
        if self.enabled and not self.refresh and p.exists():
            try:
                with np.load(p) as z:
                    value = {k: z[k] for k in z.files}
                self.hits.append(stage)
                log(stage, f"cached ({p.name})")
                return value
            except (OSError, ValueError):
                pass
        self.misses.append(stage)
        value = compute()
        if self.enabled:
            tmp = p.with_name(p.stem + ".tmp.npz")
            np.savez(tmp, **value)
            os.replace(tmp, p)
        return value

    def directory(self, stage: str, params: dict) -> tuple[Path, bool]:
        """A cache folder for file outputs. Returns (path, already_complete).
        Call mark_complete(path) once everything is written."""
        d = self.dir / f"{stage}-{params_hash(params)}"
        done = d / ".complete"
        if self.enabled and not self.refresh and done.exists():
            self.hits.append(stage)
            log(stage, f"cached ({d.name}/)")
            return d, True
        self.misses.append(stage)
        if d.exists():
            shutil.rmtree(d)
        d.mkdir(parents=True, exist_ok=True)
        return d, False

    @staticmethod
    def mark_complete(d: Path) -> None:
        (d / ".complete").write_text("ok\n")


def cache_size(root: Path | None = None) -> tuple[int, int]:
    root = root or default_cache_root()
    songs, total = 0, 0
    if root.exists():
        for song in root.iterdir():
            if song.is_dir():
                songs += 1
                for f in song.rglob("*"):
                    if f.is_file():
                        total += f.stat().st_size
    return songs, total


def clear_cache(root: Path | None = None) -> None:
    root = root or default_cache_root()
    if root.exists():
        shutil.rmtree(root)
