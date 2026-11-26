"""Which optional ML backends are installed, which device to use, and where
model weights live."""

from __future__ import annotations

import importlib.util
import os
import urllib.request
from pathlib import Path

from .progress import log

PIANO_CHECKPOINT_URL = (
    "https://zenodo.org/record/4034264/files/"
    "CRNN_note_F1%3D0.9677_pedal_F1%3D0.9186.pth?download=1"
)
PIANO_CHECKPOINT_MIN_BYTES = 160_000_000


def has(module: str) -> bool:
    try:
        return importlib.util.find_spec(module) is not None
    except (ImportError, ValueError):
        return False


def has_torch() -> bool:
    return has("torch")


def has_demucs() -> bool:
    return has_torch() and has("demucs")


def has_crepe() -> bool:
    return has_torch() and has("torchcrepe")


def has_basic_pitch() -> bool:
    return has("basic_pitch") and has("onnxruntime")


def has_piano_model() -> bool:
    return has_torch() and has("piano_transcription_inference")


def resolve_device(device: str) -> str:
    """"auto" -> "cuda" when an NVIDIA (or ROCm) GPU is usable, else "cpu"."""
    if device != "auto":
        return device
    if not has_torch():
        return "cpu"
    import torch
    return "cuda" if torch.cuda.is_available() else "cpu"


def models_dir() -> Path:
    base = os.environ.get("XDG_CACHE_HOME") or os.path.join(Path.home(), ".cache")
    d = Path(base) / "qwertone" / "models"
    d.mkdir(parents=True, exist_ok=True)
    return d


def piano_checkpoint_path() -> Path:
    return models_dir() / "piano_transcription_note_F1=0.9677_pedal_F1=0.9186.pth"


def ensure_piano_checkpoint() -> Path:
    """Download the ByteDance piano transcription weights (~165 MB) once."""
    p = piano_checkpoint_path()
    if p.exists() and p.stat().st_size >= PIANO_CHECKPOINT_MIN_BYTES:
        return p
    log("models", "downloading piano transcription model (~165 MB, once)")
    tmp = p.with_suffix(".part")
    with urllib.request.urlopen(PIANO_CHECKPOINT_URL) as r, open(tmp, "wb") as f:
        total = int(r.headers.get("Content-Length") or 0)
        done, last = 0, -10
        while chunk := r.read(1 << 20):
            f.write(chunk)
            done += len(chunk)
            if total and (pct := done * 100 // total) >= last + 10:
                last = pct
                log("models", f"piano model {pct}%")
    os.replace(tmp, p)
    return p


def status() -> dict[str, str]:
    """Human-readable status of every backend, for `qwertone-analyze doctor`."""
    import shutil
    out = {}
    out["ffmpeg"] = "ok" if shutil.which("ffmpeg") else "missing (needed for mp3/m4a: sudo pacman -S ffmpeg)"
    if has_torch():
        import torch
        gpu = "cuda: " + torch.cuda.get_device_name(0) if torch.cuda.is_available() else "CPU only"
        out["torch"] = f"{torch.__version__} ({gpu})"
    else:
        out["torch"] = "missing (stems, CREPE and the piano model are off)"
    out["demucs (stems)"] = "ok" if has_demucs() else "missing -> whole mix is analysed as one stem"
    out["torchcrepe (melody)"] = "ok" if has_crepe() else "missing -> librosa pYIN fallback (slower, less accurate)"
    out["basic-pitch (full)"] = "ok (onnx)" if has_basic_pitch() else "missing -> pitch-tracker fallback, monophonic only"
    if has_piano_model():
        ck = piano_checkpoint_path()
        out["piano model"] = "ok" if ck.exists() else "installed, weights download on first use (~165 MB)"
    else:
        out["piano model"] = "missing -> piano mode uses Basic Pitch instead"
    import importlib.util
    out["lyrics"] = "local .lrc, file tags, LRCLIB (online), Tanglish built in"
    out["mutagen (SYLT)"] = "ok" if importlib.util.find_spec("mutagen") else \
        "missing -> synced ID3 SYLT lyrics are skipped (plain tag lyrics still work)"
    return out
