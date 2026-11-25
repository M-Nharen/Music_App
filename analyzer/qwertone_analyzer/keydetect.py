"""Key detection: Krumhansl–Kessler profiles on the pitch-class histogram, with
the chords used to settle relative major/minor."""

from __future__ import annotations

import numpy as np

from .model import Chord, KeyInfo
from .theory import estimate_key

VERSION = 3


def detect(pc_hist: np.ndarray, chords: list[Chord]) -> KeyInfo:
    ranked = estimate_key(pc_hist)
    best_r, tonic, mode = ranked[0]

    def tonic_evidence(t: int, m: str) -> float:
        want = "maj" if m == "major" else "min"
        total = sum(c.end - c.start for c in chords) or 1.0
        ev = sum(c.end - c.start for c in chords
                 if c.root == t and c.quality is not None
                 and (c.quality in ("maj", "7", "maj7") if want == "maj" else c.quality in ("min", "m7")))
        ev /= total
        real = [c for c in chords if c.root is not None]
        if real:
            for c in (real[0], real[-1]):
                if c.root == t:
                    ev += 0.15
        return ev

    # relative major/minor share notes; the chords decide which is home
    rel = (tonic + 3) % 12 if mode == "minor" else (tonic + 9) % 12
    rel_mode = "major" if mode == "minor" else "minor"
    rel_r = next((r for r, t, m in ranked if t == rel and m == rel_mode), -1.0)
    if best_r - rel_r < 0.15 and tonic_evidence(rel, rel_mode) > tonic_evidence(tonic, mode) + 0.05:
        best_r, tonic, mode = rel_r, rel, rel_mode

    others = [(r, t, m) for r, t, m in ranked if not (t == tonic and m == mode)]
    # Relative major/minor always score close (same notes), so measure the
    # margin against the best *other* key, and let the chord evidence say how
    # sure we are between the two relatives.
    rel = (tonic + 3) % 12 if mode == "minor" else (tonic + 9) % 12
    rel_mode = "major" if mode == "minor" else "minor"
    non_rel = [o for o in others if not (o[1] == rel and o[2] == rel_mode)]
    margin = best_r - (non_rel[0][0] if non_rel else 0.0)
    rel_sure = float(np.clip(0.6 + abs(tonic_evidence(tonic, mode) - tonic_evidence(rel, rel_mode)) / 0.3, 0, 1))
    conf = float(np.clip((0.5 * np.clip(best_r, 0, 1) + 0.5 * np.clip(margin / 0.08, 0, 1)) * rel_sure, 0, 1))
    alts = [(t, m, r) for r, t, m in others[:3]]
    return KeyInfo(tonic, mode, conf, alts)


def histogram_from_chroma(chroma: np.ndarray, bass_chroma: np.ndarray | None = None) -> np.ndarray:
    c = chroma / (chroma.max() + 1e-9)
    h = c.sum(axis=1)
    if bass_chroma is not None:
        b = bass_chroma / (bass_chroma.max() + 1e-9)
        h = h + 0.5 * b.sum(axis=1)
    return h
