"""Shared fixtures. The test clip is generated once per session (tests/synth_clip.py)
and the slow analyses are run once and shared."""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

sys.path.insert(0, str(Path(__file__).parent))
import synth_clip  # noqa: E402

from qwertone_analyzer import backends, pipeline, stems  # noqa: E402
from qwertone_analyzer.progress import configure  # noqa: E402

configure(quiet=True)
# never ask LRCLIB from the test suite (test_lyrics.py uses a fake server instead)
os.environ["QWERTONE_OFFLINE"] = "1"
# and never load Whisper / the translator (test_lyrics_models.py uses fakes instead)
os.environ["QWERTONE_LIGHT"] = "1"
REAL_SEPARATE = stems.separate  # the fake_stems fixture swaps this out

needs_crepe = pytest.mark.skipif(not backends.has_crepe(), reason="torchcrepe not installed")
needs_basic_pitch = pytest.mark.skipif(not backends.has_basic_pitch(), reason="basic-pitch/onnxruntime not installed")
needs_demucs = pytest.mark.skipif(not backends.has_demucs(), reason="demucs not installed")
needs_piano = pytest.mark.skipif(not backends.has_piano_model(), reason="piano_transcription_inference not installed")


@pytest.fixture(scope="session")
def clip(tmp_path_factory) -> dict:
    d = tmp_path_factory.mktemp("clip")
    truth = synth_clip.write_all(d)
    return {"dir": d, "truth": truth, "mix": d / "mix.wav", "midi": d / "song.mid"}


@pytest.fixture(scope="session")
def workdirs(tmp_path_factory) -> dict:
    return {"out": tmp_path_factory.mktemp("songs"), "cache": tmp_path_factory.mktemp("cache")}


def load_song(result) -> dict:
    with open(result.song_json) as f:
        return json.load(f)


def run(path, workdirs, **kw):
    opts = pipeline.Options(out_dir=str(workdirs["out"] / kw.pop("subdir", "default")),
                            cache_dir=str(workdirs["cache"]), **kw)
    return pipeline.analyze(path, opts)


class FakeSeparator:
    """Stands in for Demucs: returns the clip's true stems."""

    def __init__(self, clip_dir: Path):
        self.dir = clip_dir
        self.calls = 0

    def __call__(self, mix, sr, model, device, **kw):
        self.calls += 1
        out = {}
        for name in ("drums", "bass", "other", "vocals"):
            y, _ = sf.read(self.dir / "stems" / f"{name}.wav", dtype="float32")
            out[name] = y.T[:, : mix.shape[1]]
        return out


@pytest.fixture(scope="session")
def fake_stems(clip):
    """Patch the pipeline so 'separation' returns the true stems."""
    sep = FakeSeparator(clip["dir"])
    mp = pytest.MonkeyPatch()
    mp.setattr(stems, "separate", sep)
    mp.setattr(backends, "has_demucs", lambda: True)
    yield sep
    mp.undo()


@pytest.fixture(scope="session")
def song_mix(clip, workdirs):
    """chords+melody on the plain mix (no separation)."""
    res = run(clip["mix"], workdirs, subdir="mix", separate=False)
    return load_song(res), res


@pytest.fixture(scope="session")
def song_stems(clip, workdirs, fake_stems):
    """chords+melody with (fake) stems: melody comes from the vocal stem."""
    res = run(clip["mix"], workdirs, subdir="stems", mode="chords+melody")
    return load_song(res), res


def melody_recall(notes: list[dict], truth: list[dict], tol: float = 0.08) -> float:
    hit = sum(any(n["pitch"] == m["pitch"] and abs(n["start"] - m["start"]) < tol for n in notes) for m in truth)
    return hit / len(truth)


def chord_accuracy(chords: list[dict], truth: list[dict]) -> float:
    """Fraction of bars whose middle falls inside a chord with the right symbol."""
    ok = 0
    for t in truth:
        mid = 0.5 * (t["start"] + t["end"])
        c = next((c for c in chords if c["start"] <= mid < c["end"]), None)
        ok += c is not None and c["symbol"] == t["symbol"]
    return ok / len(truth)


def max_simultaneous(notes) -> int:
    ev = sorted([(n["start"], 1) for n in notes] + [(n["end"], -1) for n in notes], key=lambda x: (x[0], x[1]))
    cur = best = 0
    for _, d in ev:
        cur += d
        best = max(best, cur)
    return best


__all__ = ["REAL_SEPARATE", "np", "run", "load_song", "melody_recall", "chord_accuracy", "max_simultaneous",
           "needs_crepe", "needs_basic_pitch", "needs_demucs", "needs_piano"]
