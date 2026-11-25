"""Song sections: bar-level self-similarity -> checkerboard novelty for the
boundaries -> repeated material gets the same letter -> guessed names."""

from __future__ import annotations

import string

import numpy as np

from .features import normalize_chroma, sync
from .model import Section, Timeline

VERSION = 1


def _bars(timeline: Timeline, duration: float) -> np.ndarray:
    db = timeline.downbeat_times()
    db = db[db < duration - 0.5]
    if len(db) == 0:
        return np.array([0.0, duration])
    bounds = list(db)
    if bounds[0] > 0.5:
        bounds = [0.0] + bounds  # pickup bar
    else:
        bounds[0] = 0.0
    return np.array(bounds + [duration])


def segment(feats: dict[str, np.ndarray], timeline: Timeline, duration: float,
            min_bars: int = 4) -> list[Section]:
    times = feats["times"]
    bounds = _bars(timeline, duration)
    nb = len(bounds) - 1
    if nb < 2 * min_bars:
        return [Section(0.0, duration, "A", "section", 1.0)]

    chroma = normalize_chroma(sync(feats["chroma"], times, bounds))
    parts = [chroma.T]
    if "mfcc" in feats:
        mf = sync(feats["mfcc"], times, bounds)
        mf = (mf - mf.mean(axis=1, keepdims=True)) / (mf.std(axis=1, keepdims=True) + 1e-9)
        parts.append(0.6 * mf.T / np.sqrt(mf.shape[0]) * np.sqrt(12) / 3)
    energy = sync(feats["rms"], times, bounds) if "rms" in feats else np.ones(nb)
    X = np.hstack(parts)
    X = X - X.mean(axis=0, keepdims=True)
    X /= np.linalg.norm(X, axis=1, keepdims=True) + 1e-9
    # two-bar time-delay embedding so progressions, not single bars, are compared
    Xe = np.hstack([X, np.vstack([X[1:], X[-1:]])])
    Xe /= np.linalg.norm(Xe, axis=1, keepdims=True) + 1e-9
    S = Xe @ Xe.T

    # 1) homogeneity: checkerboard kernel on the self-similarity matrix
    L = min_bars
    g = np.exp(-0.5 * (np.linspace(-1.5, 1.5, 2 * L)) ** 2)
    K = np.outer(g, g) * np.outer(np.r_[-np.ones(L), np.ones(L)], np.r_[-np.ones(L), np.ones(L)])
    Sp = np.pad(S, L, mode="edge")
    homog = np.array([np.sum(K * Sp[i: i + 2 * L, i: i + 2 * L]) for i in range(nb)])
    homog = np.maximum(homog, 0)

    # 2) repetition: "structure features" (Serra et al.) - rows of the lag
    #    matrix of a diagonally-smoothed recurrence; they change where a
    #    repeated block starts or ends
    S1 = X @ X.T
    Sd = S1.copy()
    for k in (-1, 1):
        sh = np.full_like(S1, np.nan)
        if k > 0:
            sh[:-k, :-k] = S1[k:, k:]
        else:
            sh[-k:, -k:] = S1[:k, :k]
        Sd = np.nansum(np.stack([Sd, sh]), axis=0)
    Sd /= 3.0
    thr_r = np.percentile(Sd[~np.eye(nb, dtype=bool)], 75)
    R = (Sd >= thr_r).astype(float) * np.clip(Sd, 0, 1)
    lag = np.stack([np.roll(R[i], -i) for i in range(nb)])
    from scipy.ndimage import gaussian_filter1d
    lag = gaussian_filter1d(lag, sigma=0.6, axis=0)
    struct = np.r_[0.0, np.linalg.norm(np.diff(lag, axis=0), axis=1)]

    def unit(x):
        return x / (x.max() + 1e-9)

    nov = unit(homog) + unit(struct)
    # gentle preference for 4-bar phrase lines counted from the first full bar
    first_full = 1 if bounds[1] - bounds[0] < 0.75 * (bounds[2] - bounds[1]) else 0
    for i in range(nb):
        if (i - first_full) % 4 == 0:
            nov[i] *= 1.15
    nov[:min_bars] = 0
    nov[nb - min_bars + 1:] = 0
    segment.last_novelty = nov  # kept for debugging / tests
    thr = nov.mean() + 0.3 * nov.std()
    cands = [i for i in range(1, nb) if nov[i] > thr and nov[i] >= nov[max(i - 1, 0)] and nov[i] >= nov[min(i + 1, nb - 1)]]
    cands.sort(key=lambda i: -nov[i])
    chosen: list[int] = []
    for i in cands:
        if all(abs(i - j) >= min_bars for j in chosen):
            chosen.append(i)
    cuts = [0] + sorted(chosen) + [nb]
    peak = nov.max() or 1.0

    # label: same letter for repeated material
    segs = [(cuts[k], cuts[k + 1]) for k in range(len(cuts) - 1)]

    def sim(a, b):
        (a0, a1), (b0, b1) = a, b
        n = min(a1 - a0, b1 - b0)
        diag = np.mean([S[a0 + k, b0 + k] for k in range(n)])
        mean = float(X[a0:a1].mean(0) @ X[b0:b1].mean(0) /
                     (np.linalg.norm(X[a0:a1].mean(0)) * np.linalg.norm(X[b0:b1].mean(0)) + 1e-9))
        return max(diag, mean)

    labels: list[str] = []
    reps: list[tuple[int, int]] = []
    for s in segs:
        scores = [sim(s, r) for r in reps]
        if scores and max(scores) > 0.6:
            labels.append(string.ascii_uppercase[int(np.argmax(scores))])
        else:
            reps.append(s)
            labels.append(string.ascii_uppercase[min(len(reps) - 1, 25)])

    names = _guess_names(segs, labels, energy)
    out = []
    for k, ((a, b), lab, name) in enumerate(zip(segs, labels, names)):
        conf = 1.0 if k == 0 else float(np.clip(nov[a] / peak, 0, 1))
        out.append(Section(float(bounds[a]), float(bounds[b]), lab, name, conf))
    return out


def _guess_names(segs, labels, energy) -> list[str]:
    n = len(segs)
    seg_energy = [float(np.mean(energy[a:b])) for a, b in segs]
    counts = {lab: labels.count(lab) for lab in labels}
    names = ["section"] * n
    repeated = [lab for lab in counts if counts[lab] > 1]
    chorus = None
    if repeated:
        chorus = max(repeated, key=lambda lab: (np.mean([e for e, l in zip(seg_energy, labels) if l == lab]), counts[lab]))
        verse_cands = [lab for lab in repeated if lab != chorus]
        verse = max(verse_cands, key=lambda lab: counts[lab]) if verse_cands else None
        for i, lab in enumerate(labels):
            if lab == chorus:
                names[i] = "chorus"
            elif lab == verse:
                names[i] = "verse"
    mean_e = float(np.mean(seg_energy))
    if n > 2 and counts[labels[0]] == 1 and seg_energy[0] < mean_e:
        names[0] = "intro"
    if n > 2 and counts[labels[-1]] == 1 and seg_energy[-1] < mean_e:
        names[-1] = "outro"
    for i in range(1, n - 1):
        if names[i] == "section" and counts[labels[i]] == 1 and chorus is not None:
            names[i] = "bridge"
    return names


def to_cache(sections: list[Section]) -> list[dict]:
    return [s.__dict__ for s in sections]


def from_cache(d: list[dict]) -> list[Section]:
    return [Section(**s) for s in d]
