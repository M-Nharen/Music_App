"""Monophonic pitch tracking (CREPE via torchcrepe, or librosa pYIN when torch
isn't installed) and turning a pitch contour into notes. Used for the vocal
melody and for bass lines."""

from __future__ import annotations

import numpy as np

from . import backends
from .model import Note
from .progress import log
from .theory import hz_to_midi

VERSION = 2
CREPE_SR = 16000
HOP_S = 0.01  # 10 ms frames


def track_pitch(y: np.ndarray, sr: int, fmin: float, fmax: float, method: str = "auto",
                crepe_model: str = "auto", device: str = "cpu") -> dict[str, np.ndarray]:
    """Returns {"times", "midi" (nan when unvoiced), "conf"} at 10 ms frames,
    plus "method" as a 0-d string array."""
    if method == "auto":
        method = "crepe" if backends.has_crepe() else "pyin"
    if method == "crepe":
        import torch
        import torchcrepe
        from .audio_io import resample

        if crepe_model == "auto":
            crepe_model = "full" if device.startswith("cuda") else "tiny"
        audio = resample(y.astype(np.float32), sr, CREPE_SR)
        hop = int(CREPE_SR * HOP_S)
        log("pitch", f"CREPE ({crepe_model}) on {device}")
        x = torch.from_numpy(audio)[None]
        with torch.no_grad():
            pitch, periodicity = torchcrepe.predict(
                x, CREPE_SR, hop, max(fmin, 32.0), min(fmax, 1975.0), crepe_model,
                decoder=torchcrepe.decode.viterbi, return_periodicity=True,
                batch_size=1024, device=device, pad=True)
            periodicity = torchcrepe.filter.median(periodicity, 3)
            pitch = torchcrepe.filter.mean(pitch, 3)
        hz = pitch[0].cpu().numpy().astype(float)
        conf = np.nan_to_num(periodicity[0].cpu().numpy().astype(float))
        # Silence gate relative to this stem's own level. (torchcrepe's
        # Silence threshold is absolute and mutes quiet stems entirely.)
        frames = np.lib.stride_tricks.sliding_window_view(np.pad(audio, (512, 512)), 1024)[::hop][: len(conf)]
        db = 10 * np.log10(np.mean(frames.astype(np.float64) ** 2, axis=1) + 1e-12)
        conf[: len(db)][db < db.max() - 45.0] = 0.0
        midi = hz_to_midi(np.maximum(hz, 1e-3))
        midi[conf < 0.05] = np.nan
        name = f"torchcrepe-{crepe_model}"
    else:
        import librosa
        from .audio_io import resample

        sr2 = 22050
        audio = resample(y.astype(np.float32), sr, sr2)
        hop = int(sr2 * HOP_S)
        log("pitch", "librosa pYIN (install torchcrepe for better results)")
        f0, voiced, vprob = librosa.pyin(audio, fmin=fmin, fmax=fmax, sr=sr2, hop_length=hop,
                                         frame_length=2048 if fmin >= 50 else 4096)
        midi = hz_to_midi(np.where(voiced, f0, np.nan))
        conf = np.nan_to_num(vprob.astype(float))
        name = "pyin"
    times = np.arange(len(midi)) * HOP_S
    return {"times": times, "midi": midi.astype(float), "conf": conf, "method": np.array(name)}


def contour_to_notes(times: np.ndarray, midi: np.ndarray, conf: np.ndarray, *,
                     voicing_threshold: float = 0.3, min_dur: float = 0.08,
                     split_semitones: float = 0.6, onsets: np.ndarray | None = None,
                     loudness: np.ndarray | None = None) -> list[Note]:
    """Segment a pitch contour into notes.

    - a frame is voiced when it has a pitch and conf >= voicing_threshold
    - a new note starts when the pitch moves more than `split_semitones` from the
      running note for 3+ frames, or at a detected onset (repeated notes)
    - note pitch = median, confidence = mean voicing confidence, lowered when the
      pitch wobbles or sits between two semitones
    """
    hop = float(times[1] - times[0]) if len(times) > 1 else HOP_S
    voiced = np.isfinite(midi) & (conf >= voicing_threshold)
    m = midi.copy()
    # median-smooth inside voiced runs
    from scipy.ndimage import median_filter
    filled = np.where(voiced, m, 0.0)
    sm = median_filter(filled, size=5, mode="nearest")
    m = np.where(voiced, np.where(np.abs(sm - m) < 1.0, sm, m), np.nan)

    onset_frames = set()
    if onsets is not None:
        onset_frames = {int(round(t / hop)) + d for t in onsets for d in (-1, 0, 1)}

    segments: list[tuple[int, int]] = []
    i, n = 0, len(m)
    while i < n:
        if not voiced[i]:
            i += 1
            continue
        start = i
        ref = m[i]
        j = i + 1
        while j < n and voiced[j]:
            if j in onset_frames and j - start >= 3:
                break
            if abs(m[j] - ref) > split_semitones:
                # only a sustained move (3+ frames) starts a new note; shorter ones are glitches
                k = j
                while k < n and voiced[k] and abs(m[k] - ref) > split_semitones and k - j < 3:
                    k += 1
                if k - j >= 3:
                    break
            if j - start < 50:
                ref = float(np.median(m[start:j + 1]))
            j += 1
        segments.append((start, j))
        i = j

    notes: list[Note] = []
    loud_ref = np.percentile(loudness[np.isfinite(loudness)], 95) if loudness is not None and len(loudness) else None
    for a, b in segments:
        dur = (b - a) * hop
        if dur < min_dur:
            continue
        seg = m[a:b]
        med = float(np.median(seg))
        pitch = int(round(med))
        wobble = float(np.std(seg))
        detune = abs(med - pitch)
        c = float(np.mean(conf[a:b])) * (1 - 0.5 * min(1.0, wobble)) * (1 - detune)
        vel = 90
        if loud_ref:
            lv = float(np.mean(loudness[a:min(b, len(loudness))])) if a < len(loudness) else 0.0
            vel = int(np.clip(40 + 80 * lv / (loud_ref + 1e-9), 30, 120))
        start_t = float(times[a])
        end_t = float(times[b - 1] + hop)
        if notes and notes[-1].pitch == pitch and start_t - notes[-1].end < 0.03 and a not in onset_frames:
            prev = notes[-1]
            w1, w2 = prev.dur, end_t - start_t
            prev.confidence = (prev.confidence * w1 + c * w2) / (w1 + w2)
            prev.end = end_t
            continue
        notes.append(Note(pitch, start_t, end_t, vel, float(np.clip(c, 0, 1))))
    return notes


def notes_from_audio(y: np.ndarray, sr: int, *, fmin: float, fmax: float, method: str = "auto",
                     crepe_model: str = "auto", device: str = "cpu",
                     voicing_threshold: float = 0.3) -> tuple[list[Note], str, dict]:
    import librosa

    track = track_pitch(y, sr, fmin, fmax, method, crepe_model, device)
    y22 = librosa.resample(y.astype(np.float32), orig_sr=sr, target_sr=22050) if sr != 22050 else y
    onsets = librosa.onset.onset_detect(y=y22, sr=22050, units="time", backtrack=False)
    hop = int(22050 * HOP_S)
    rms = librosa.feature.rms(y=y22, frame_length=1024, hop_length=hop)[0]
    notes = contour_to_notes(track["times"], track["midi"], track["conf"],
                             voicing_threshold=voicing_threshold, onsets=onsets, loudness=rms)
    return notes, str(track["method"]), track
