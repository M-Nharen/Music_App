"""Frame-level features shared by rhythm, chord, key and section analysis.
All features use the same 22.05 kHz / 512-sample hop grid (~23 ms)."""

from __future__ import annotations

import numpy as np

FEAT_SR = 22050
HOP = 512
VERSION = 1


def compute(harmonic: np.ndarray, full: np.ndarray, bass: np.ndarray | None = None,
            drums: np.ndarray | None = None) -> dict[str, np.ndarray]:
    """All inputs mono at FEAT_SR. `harmonic` should be the pitched accompaniment
    (no drums, ideally no vocals); `bass`/`drums` are the stems if available."""
    import librosa

    sr = FEAT_SR
    chroma = librosa.feature.chroma_cqt(y=harmonic, sr=sr, hop_length=HOP, norm=None,
                                        bins_per_octave=36, n_octaves=7)
    bass_src = bass if bass is not None else harmonic
    bass_chroma = librosa.feature.chroma_cqt(y=bass_src, sr=sr, hop_length=HOP, norm=None,
                                             fmin=librosa.note_to_hz("C1"), n_octaves=3,
                                             bins_per_octave=36)
    perc = drums if drums is not None else full
    onset = librosa.onset.onset_strength(y=perc, sr=sr, hop_length=HOP, aggregate=np.median)
    full_onset = onset if drums is None else librosa.onset.onset_strength(y=full, sr=sr, hop_length=HOP)
    low_mel = librosa.feature.melspectrogram(y=perc, sr=sr, hop_length=HOP, n_mels=12, fmax=250)
    low_onset = librosa.onset.onset_strength(S=librosa.power_to_db(low_mel), sr=sr, hop_length=HOP)
    mfcc = librosa.feature.mfcc(y=full, sr=sr, hop_length=HOP, n_mfcc=13)
    rms = librosa.feature.rms(y=full, hop_length=HOP)[0]

    n = min(chroma.shape[1], bass_chroma.shape[1], len(onset), len(full_onset), len(low_onset),
            mfcc.shape[1], len(rms))
    return {
        "chroma": chroma[:, :n].astype(np.float32),
        "bass_chroma": bass_chroma[:, :n].astype(np.float32),
        "onset": onset[:n].astype(np.float32),
        "full_onset": full_onset[:n].astype(np.float32),
        "low_onset": low_onset[:n].astype(np.float32),
        "mfcc": mfcc[:, :n].astype(np.float32),
        "rms": rms[:n].astype(np.float32),
        "times": (np.arange(n) * HOP / sr).astype(np.float64),
    }


def sync(feature: np.ndarray, frame_times: np.ndarray, bounds: np.ndarray, agg=np.mean) -> np.ndarray:
    """Aggregate a (d, T) or (T,) feature over the time spans between `bounds`."""
    f = feature if feature.ndim == 2 else feature[None, :]
    idx = np.searchsorted(frame_times, bounds)
    out = np.zeros((f.shape[0], len(bounds) - 1), dtype=np.float64)
    for i in range(len(bounds) - 1):
        a, b = idx[i], max(idx[i + 1], idx[i] + 1)
        seg = f[:, a:b]
        if seg.shape[1]:
            out[:, i] = agg(seg, axis=1)
    return out if feature.ndim == 2 else out[0]


def normalize_chroma(c: np.ndarray) -> np.ndarray:
    """Log-compress and L2-normalise each column."""
    c = np.log1p(10.0 * np.maximum(c, 0) / (c.max() + 1e-9))
    return c / (np.linalg.norm(c, axis=0, keepdims=True) + 1e-9)
