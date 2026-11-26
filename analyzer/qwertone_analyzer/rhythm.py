"""Tempo, beats, meter and downbeats."""

from __future__ import annotations

import numpy as np

from .features import FEAT_SR, HOP, normalize_chroma, sync

VERSION = 1


def _z(x: np.ndarray) -> np.ndarray:
    s = x.std()
    return (x - x.mean()) / s if s > 1e-9 else np.zeros_like(x)


def _extend(beats: np.ndarray, onset: np.ndarray, times: np.ndarray, duration: float) -> np.ndarray:
    """The beat tracker drops weak beats at the edges. Walk outwards one beat at
    a time while there is still a clear onset where the next beat should be."""
    def strength(t):
        f = int(np.searchsorted(times, t))
        return onset[max(f - 2, 0): f + 3].max() if 0 <= f < len(onset) else 0.0

    ref = float(np.median([strength(b) for b in beats]))
    ibi = float(np.median(np.diff(beats)))
    out = list(beats)
    while out[0] - ibi > -0.03 and strength(out[0] - ibi) > 0.3 * ref:
        out.insert(0, max(out[0] - ibi, 0.0))
    while out[-1] + ibi < duration - 0.05 and strength(out[-1] + ibi) > 0.3 * ref:
        out.append(out[-1] + ibi)
    return np.array(out)


def analyze(feats: dict[str, np.ndarray], duration: float) -> dict:
    import librosa

    onset = feats["onset"].astype(float)
    times = feats["times"]
    _, frames = librosa.beat.beat_track(onset_envelope=onset, sr=FEAT_SR, hop_length=HOP,
                                        units="frames", start_bpm=110.0)
    beats = librosa.frames_to_time(np.asarray(frames, dtype=int), sr=FEAT_SR, hop_length=HOP)

    if len(beats) < 4:
        # Nothing rhythmic (rubato piano, silence...). Lay a plain 120 BPM grid.
        beats = np.arange(0.0, max(duration, 2.0), 0.5)
        return {"bpm": 120.0, "beats": beats.tolist(), "downbeats": [i % 4 == 0 for i in range(len(beats))],
                "beats_per_bar": 4, "tempo_confidence": 0.0, "downbeat_confidence": 0.0}

    beats = _extend(beats, onset, times, duration)
    ibi = np.diff(beats)
    # tempo from a straight-line fit of beat time vs beat number: beat times are
    # quantised to 23 ms frames, so single gaps are too coarse
    good = np.abs(ibi - np.median(ibi)) < 0.15 * np.median(ibi)
    idx = np.arange(len(beats))
    if good.sum() >= 3:
        keep = np.r_[good, False] | np.r_[False, good]
        slope = np.polyfit(idx[keep], beats[keep], 1)[0]
    else:
        slope = float(np.median(ibi))
    bpm = 60.0 / float(slope)
    cv = float(np.std(ibi) / np.mean(ibi))
    beat_frames = np.clip(np.searchsorted(times, beats), 0, len(onset) - 1)
    pulse = float(onset[beat_frames].mean() / (onset.mean() + 1e-9))
    tempo_conf = float(np.clip(1.0 - 4.0 * cv, 0, 1) * np.clip((pulse - 1.0) / 1.0, 0, 1))

    # --- meter + downbeat phase -------------------------------------------------
    def peak_at(x):
        return np.array([x[max(f - 2, 0): f + 3].max() for f in beat_frames])

    bounds = np.concatenate([beats, [min(duration, beats[-1] + ibi[-1])]])
    bc = normalize_chroma(sync(feats["chroma"], times, bounds))
    novelty = np.r_[0.0, 1.0 - np.sum(bc[:, 1:] * bc[:, :-1], axis=0)]
    bass_bc = normalize_chroma(sync(feats["bass_chroma"], times, bounds))
    bass_nov = np.r_[0.0, 1.0 - np.sum(bass_bc[:, 1:] * bass_bc[:, :-1], axis=0)]
    accent = _z(peak_at(onset)) + _z(peak_at(feats["low_onset"])) + 1.5 * _z(novelty) + _z(bass_nov)

    best = (-np.inf, 4, 0)
    scores = {}
    for m in (4, 3):
        for phase in range(m):
            mask = np.zeros(len(beats), bool)
            mask[phase::m] = True
            if mask.sum() < 2 or (~mask).sum() < 2:
                continue
            s = float(accent[mask].mean() - accent[~mask].mean())
            if m == 3:
                s *= 0.8  # most pop music is in 4; ask 3/4 to be clearly better
            scores[(m, phase)] = s
            if s > best[0]:
                best = (s, m, phase)
    score, m, phase = best
    downbeats = [(i - phase) % m == 0 for i in range(len(beats))]
    db_conf = float(np.clip(score / 2.0, 0, 1))
    return {"bpm": round(bpm, 2), "beats": [round(float(b), 4) for b in beats], "downbeats": downbeats,
            "beats_per_bar": m, "tempo_confidence": round(tempo_conf, 3),
            "downbeat_confidence": round(db_conf, 3)}


def from_grid(beats: list[float], downbeats: list[bool], beats_per_bar: int, bpm: float) -> dict:
    """Rhythm block for sources that already know their grid (MIDI input)."""
    return {"bpm": round(float(bpm), 2), "beats": [round(float(b), 4) for b in beats],
            "downbeats": [bool(d) for d in downbeats], "beats_per_bar": int(beats_per_bar),
            "tempo_confidence": 1.0, "downbeat_confidence": 1.0}
