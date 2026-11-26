"""Chord recognition: beat-synchronous chroma -> chord templates -> HMM
(Viterbi) smoothing. Works on audio chroma and on chroma built from MIDI notes."""

from __future__ import annotations

import numpy as np

from .features import normalize_chroma, sync
from .model import Chord
from .theory import CHORD_QUALITIES

VERSION = 1

VOCABS = {
    "basic": ["maj", "min"],
    "extended": ["maj", "min", "7", "maj7", "m7", "dim", "aug", "sus2", "sus4"],
}
# Small score penalties so plain triads win unless the extra note is really there.
PRIOR = {"maj": 0.0, "min": 0.0, "7": 0.03, "maj7": 0.04, "m7": 0.03, "dim": 0.05,
         "aug": 0.08, "sus2": 0.06, "sus4": 0.05}


def _states(vocab: str) -> list[tuple[int | None, str | None, np.ndarray]]:
    states = []
    for q in VOCABS[vocab]:
        intervals = CHORD_QUALITIES[q][0]
        for root in range(12):
            v = np.zeros(12)
            for i in intervals:
                v[(root + i) % 12] = 1.0
            v[root] += 0.2  # the root is usually loudest
            states.append((root, q, v / np.linalg.norm(v)))
    states.append((None, None, np.ones(12) / np.sqrt(12)))  # N = no chord
    return states


def _viterbi(log_emit: np.ndarray, p_stay: float) -> np.ndarray:
    k, n = log_emit.shape
    ls, lc = np.log(p_stay), np.log((1 - p_stay) / (k - 1))
    delta = log_emit[:, 0].copy()
    back = np.zeros((k, n), dtype=int)
    for t in range(1, n):
        best_prev = int(np.argmax(delta))
        stay = delta + ls
        change = delta[best_prev] + lc
        use_change = change > stay
        back[:, t] = np.where(use_change, best_prev, np.arange(k))
        delta = np.where(use_change, change, stay) + log_emit[:, t]
    path = np.zeros(n, dtype=int)
    path[-1] = int(np.argmax(delta))
    for t in range(n - 1, 0, -1):
        path[t - 1] = back[path[t], t]
    return path


def detect(chroma: np.ndarray, bass_chroma: np.ndarray | None, frame_times: np.ndarray,
           beats: list[float], duration: float, energy: np.ndarray | None = None,
           vocab: str = "extended", p_stay: float = 0.8, beta: float = 25.0,
           sharp: bool = False) -> list[Chord]:
    """`chroma`: (12, T) un-normalised. `energy`: (T,) loudness for silence
    detection. `sharp=True` for clean symbolic input (MIDI)."""
    beats = np.asarray(beats, dtype=float)
    bounds = np.unique(np.concatenate([[0.0], beats[(beats > 0.05) & (beats < duration - 0.05)], [duration]]))
    c = sync(chroma, frame_times, bounds)
    raw_energy = c.sum(axis=0)
    cn = normalize_chroma(c)
    bn = normalize_chroma(sync(bass_chroma, frame_times, bounds)) if bass_chroma is not None else None
    if energy is not None:
        seg_energy = sync(energy, frame_times, bounds)
        silent = seg_energy < 0.08 * np.percentile(seg_energy, 90)
    else:
        silent = raw_energy < 0.05 * (np.percentile(raw_energy, 90) + 1e-9)

    states = _states(vocab)
    T = np.stack([s[2] for s in states])           # (K, 12)
    scores = T @ cn                                  # cosine similarity (K, S)
    for i, (root, q, _) in enumerate(states):
        if q is None:
            scores[i] = 0.8 * scores[i] + 0.6 * silent
        else:
            scores[i] -= PRIOR[q]
            if bn is not None:
                scores[i] += 0.08 * bn[root]
            scores[i] -= 0.6 * silent
    if sharp:
        beta *= 2
    log_emit = beta * scores - np.log(np.exp(beta * scores).sum(axis=0, keepdims=True))
    path = _viterbi(log_emit, p_stay)

    # confidence: how far the chosen chord beats the best chord with a different triad
    triad_key = [None if q is None else (r, "min" if q in ("min", "m7") else q) for r, q, _ in states]
    conf = np.zeros(len(path))
    for t, s in enumerate(path):
        others = [scores[j, t] for j in range(len(states)) if triad_key[j] != triad_key[s]]
        margin = scores[s, t] - max(others)
        conf[t] = np.clip(0.5 + margin / 0.12, 0, 1)

    chords: list[Chord] = []
    for t, s in enumerate(path):
        root, q, _ = states[s]
        dur = bounds[t + 1] - bounds[t]
        if chords and chords[-1].root == root and chords[-1].quality == q:
            prev = chords[-1]
            w = prev.end - prev.start
            prev.confidence = (prev.confidence * w + conf[t] * dur) / (w + dur)
            prev.end = float(bounds[t + 1])
        else:
            chords.append(Chord(float(bounds[t]), float(bounds[t + 1]), root, q, None, float(conf[t])))

    # inversions from the bass line
    if bass_chroma is not None:
        for ch in chords:
            if ch.root is None:
                continue
            m = (frame_times >= ch.start) & (frame_times < ch.end)
            if not m.any():
                continue
            b = bass_chroma[:, m].mean(axis=1)
            low = int(np.argmax(b))
            tones = [(ch.root + i) % 12 for i in CHORD_QUALITIES[ch.quality][0]]
            if low != ch.root and low in tones and b[low] > 1.5 * b[ch.root]:
                ch.bass = low
    return chords


def chroma_from_notes(notes, duration: float, fps: float = 43.0) -> tuple[np.ndarray, np.ndarray, np.ndarray]:
    """Pitch-class 'chroma' and bass chroma from symbolic notes (MIDI input)."""
    n = int(np.ceil(duration * fps)) + 1
    chroma = np.zeros((12, n))
    bass = np.zeros((12, n))
    lowest = np.full(n, 999)
    times = np.arange(n) / fps
    for nt in notes:
        a, b = int(nt.start * fps), max(int(nt.end * fps), int(nt.start * fps) + 1)
        w = nt.velocity / 127.0
        chroma[nt.pitch % 12, a:b] += w
        seg = lowest[a:b]
        lowest[a:b] = np.minimum(seg, nt.pitch)
    for i in range(n):
        if lowest[i] < 999:
            bass[lowest[i] % 12, i] = 1.0
    return chroma, bass, times


def voice_chords(chords: list[Chord], lo: int = 52, hi: int = 74, velocity: int = 72) -> list:
    """Playable block chords with smooth voice leading: each chord takes the
    inversion (inside lo..hi) closest to the previous one. Slash chords put
    their bass note an octave below."""
    from itertools import product

    from .model import Note

    notes = []
    prev: list[int] | None = None
    for c in chords:
        if c.root is None:
            continue
        pcs = [(c.root + i) % 12 for i in CHORD_QUALITIES[c.quality][0]]
        options = []
        for octs in product(*[[p + 12 * o for o in range(3, 7) if lo <= p + 12 * o <= hi] for p in pcs]):
            v = sorted(octs)
            if v and v[-1] - v[0] <= 14:
                options.append(v)
        if not options:
            continue
        if prev is None:
            voicing = min(options, key=lambda v: abs(np.mean(v) - 62))
        else:
            voicing = min(options, key=lambda v: sum(min(abs(a - b) for b in prev) for a in v) + 0.1 * abs(np.mean(v) - 62))
        prev = voicing
        for p in voicing:
            notes.append(Note(int(p), c.start, c.end, velocity, c.confidence))
        if c.bass is not None:
            b = 36 + (c.bass - 36) % 12
            notes.append(Note(int(b), c.start, c.end, velocity, c.confidence))
    return notes
