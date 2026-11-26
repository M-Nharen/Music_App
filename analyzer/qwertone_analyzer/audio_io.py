"""Decoding and writing audio. ffmpeg handles mp3/m4a/anything; WAV and FLAC
also work without it through libsndfile."""

from __future__ import annotations

import json
import shutil
import subprocess
from pathlib import Path

import numpy as np

SR = 44100  # working sample rate for the mix and stems

AUDIO_EXTS = {".mp3", ".flac", ".wav", ".m4a", ".aac", ".ogg", ".opus", ".aiff", ".aif", ".wma"}
MIDI_EXTS = {".mid", ".midi", ".kar"}


def have_ffmpeg() -> bool:
    return shutil.which("ffmpeg") is not None


def load_audio(path: str | Path, sr: int = SR) -> np.ndarray:
    """Decode to float32, shape (2, samples), at `sr`."""
    path = Path(path)
    if have_ffmpeg():
        cmd = ["ffmpeg", "-nostdin", "-v", "error", "-i", str(path), "-map", "0:a:0",
               "-f", "f32le", "-acodec", "pcm_f32le", "-ac", "2", "-ar", str(sr), "-"]
        res = subprocess.run(cmd, capture_output=True)
        if res.returncode != 0:
            raise RuntimeError(f"ffmpeg could not decode {path.name}: {res.stderr.decode(errors='replace').strip()}")
        y = np.frombuffer(res.stdout, dtype=np.float32).reshape(-1, 2).T.copy()
    else:
        if path.suffix.lower() not in {".wav", ".flac", ".ogg", ".aiff", ".aif"}:
            raise RuntimeError(f"Decoding {path.suffix} needs ffmpeg (sudo pacman -S ffmpeg)")
        import soundfile as sf
        data, file_sr = sf.read(str(path), dtype="float32", always_2d=True)
        y = data.T
        if y.shape[0] == 1:
            y = np.vstack([y, y])
        y = y[:2]
        if file_sr != sr:
            import librosa
            y = librosa.resample(y, orig_sr=file_sr, target_sr=sr)
    if y.size == 0:
        raise RuntimeError(f"{path.name} has no audio")
    return y.astype(np.float32)


def write_audio(path: str | Path, y: np.ndarray, sr: int = SR) -> None:
    """Write (channels, samples) float audio as FLAC/WAV (16-bit)."""
    import soundfile as sf
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    data = np.clip(np.asarray(y, dtype=np.float32), -1.0, 1.0)
    if data.ndim == 2:
        data = data.T
    fmt = "FLAC" if path.suffix.lower() == ".flac" else "WAV"
    sf.write(str(path), data, sr, format=fmt, subtype="PCM_16")


def read_audio(path: str | Path) -> tuple[np.ndarray, int]:
    import soundfile as sf
    data, sr = sf.read(str(path), dtype="float32", always_2d=True)
    return data.T, sr


def to_mono(y: np.ndarray) -> np.ndarray:
    return y.mean(axis=0) if y.ndim == 2 else y


def resample(y: np.ndarray, sr: int, target: int) -> np.ndarray:
    if sr == target:
        return y
    import librosa
    return librosa.resample(y, orig_sr=sr, target_sr=target)


def probe_tags(path: str | Path) -> dict:
    """Title/artist/album from the file tags (needs ffprobe)."""
    if shutil.which("ffprobe") is None:
        return {}
    res = subprocess.run(["ffprobe", "-v", "error", "-print_format", "json", "-show_format", str(path)],
                         capture_output=True)
    if res.returncode != 0:
        return {}
    try:
        tags = json.loads(res.stdout).get("format", {}).get("tags", {})
    except json.JSONDecodeError:
        return {}
    tags = {k.lower(): v for k, v in tags.items()}
    return {k: tags[k] for k in ("title", "artist", "album") if tags.get(k)}
