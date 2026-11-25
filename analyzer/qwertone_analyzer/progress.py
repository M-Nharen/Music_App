"""Progress messages. Plain text on stderr by default; with --json-progress each
line is a JSON object so the Rust app can show a progress bar:

    {"event": "stage", "stage": "stems", "message": "separating with htdemucs"}
    {"event": "done", "song_dir": "/home/me/.local/share/qwertone/songs/x-1234"}
    {"event": "error", "message": "..."}
"""

from __future__ import annotations

import json
import sys
import time

_state = {"json": False, "quiet": False, "t0": time.monotonic()}


def configure(json_lines: bool = False, quiet: bool = False) -> None:
    _state["json"] = json_lines
    _state["quiet"] = quiet
    _state["t0"] = time.monotonic()


def log(stage: str, message: str) -> None:
    if _state["json"]:
        print(json.dumps({"event": "stage", "stage": stage, "message": message}), file=sys.stderr, flush=True)
    elif not _state["quiet"]:
        dt = time.monotonic() - _state["t0"]
        print(f"[{dt:6.1f}s] {stage:<10} {message}", file=sys.stderr, flush=True)


def event(name: str, **fields) -> None:
    if _state["json"]:
        print(json.dumps({"event": name, **fields}), file=sys.stderr, flush=True)
