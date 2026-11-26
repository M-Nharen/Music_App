"""Polyphonic transcription: Basic Pitch (any stem), the ByteDance piano
transcription model, and a light-weight drum transcriber."""

from __future__ import annotations

import contextlib
import io
import os
import tempfile

import numpy as np

from . import backends
from .model import Note
from .progress import log

VERSION = 3

# Frequency limits per stem, so Basic Pitch doesn't hallucinate out of range.
STEM_RANGES = {
    "vocals": (70.0, 1400.0),
    "bass": (30.0, 400.0),
    "guitar": (70.0, 1500.0),
    "piano": (27.0, 4200.0),
    "other": (40.0, 4200.0),
    "mix": (27.0, 4200.0),
}

# --------------------------------------------------------------------------- Basic Pitch

_bp_model = None


def basic_pitch_notes(y: np.ndarray, sr: int, stem: str = "mix", *, onset_threshold: float = 0.5,
                      frame_threshold: float = 0.3, min_note_ms: float = 80.0) -> list[Note]:
    global _bp_model
    import soundfile as sf
    with contextlib.redirect_stdout(io.StringIO()):
        from basic_pitch import FilenameSuffix, build_icassp_2022_model_path
        from basic_pitch.inference import Model, predict
        if _bp_model is None:
            _bp_model = Model(build_icassp_2022_model_path(FilenameSuffix.onnx))
    fmin, fmax = STEM_RANGES.get(stem, STEM_RANGES["mix"])
    with tempfile.TemporaryDirectory() as td:
        path = os.path.join(td, f"{stem}.wav")
        sf.write(path, np.asarray(y, dtype=np.float32), sr)
        with contextlib.redirect_stdout(io.StringIO()):
            _, _, events = predict(path, _bp_model, onset_threshold=onset_threshold,
                                   frame_threshold=frame_threshold, minimum_note_length=min_note_ms,
                                   minimum_frequency=fmin, maximum_frequency=fmax,
                                   melodia_trick=True)
    notes = []
    for start, end, pitch, amp, _bends in events:
        amp = float(amp)
        # amp is the mean note activation (0..1); ~0.45 is a coin flip in practice
        conf = float(np.clip((amp - 0.25) / 0.4, 0, 1))
        vel = int(np.clip(30 + 97 * amp, 1, 127))
        notes.append(Note(int(pitch), float(start), float(end), vel, conf))
    notes.sort(key=lambda n: (n.start, n.pitch))
    return notes


# --------------------------------------------------------------------------- piano

_piano = None


def _load_piano(device: str, checkpoint: str | None = None):
    """Build the ByteDance model ourselves (its own loader shells out to wget)."""
    import torch
    with contextlib.redirect_stdout(io.StringIO()):
        from piano_transcription_inference import config
        from piano_transcription_inference.inference import PianoTranscription
        from piano_transcription_inference.models import Note_pedal

    path = checkpoint or str(backends.ensure_piano_checkpoint())
    model = Note_pedal(frames_per_second=config.frames_per_second, classes_num=config.classes_num)
    try:
        ck = torch.load(path, map_location=device, weights_only=True)
    except Exception:  # older checkpoints pickle extra metadata
        ck = torch.load(path, map_location=device, weights_only=False)
    model.load_state_dict(ck["model"] if "model" in ck else ck, strict=False)
    model.to(device).eval()

    pt = PianoTranscription.__new__(PianoTranscription)
    pt.segment_samples = 16000 * 10
    pt.frames_per_second = config.frames_per_second
    pt.classes_num = config.classes_num
    pt.onset_threshold = 0.3
    pt.offset_threshod = 0.3  # (sic) name used by the library
    pt.frame_threshold = 0.1
    pt.pedal_offset_threshold = 0.2
    pt.model = model
    return pt


def piano_notes(y: np.ndarray, sr: int, device: str = "cpu", checkpoint: str | None = None) -> list[Note]:
    global _piano
    from .audio_io import resample

    if _piano is None:
        _piano = _load_piano(device, checkpoint)
    audio = resample(np.asarray(y, dtype=np.float32), sr, 16000).astype(np.float64)
    log("piano", f"piano transcription model on {device}")
    with contextlib.redirect_stdout(io.StringIO()):
        out = _piano.transcribe(audio, None)
    frame = out["output_dict"].get("frame_output")
    onset = out["output_dict"].get("reg_onset_output")
    fps = _piano.frames_per_second
    notes = []
    for ev in out["est_note_events"]:
        s, e, p = float(ev["onset_time"]), float(ev["offset_time"]), int(ev["midi_note"])
        if not (np.isfinite(s) and np.isfinite(e)) or e <= s:
            continue
        k = p - 21
        conf = 0.8
        if frame is not None and 0 <= k < frame.shape[1]:
            a, b = int(s * fps), max(int(e * fps), int(s * fps) + 1)
            fm = float(frame[a:b, k].mean()) if a < len(frame) else 0.0
            op = float(onset[a:a + 3, k].max()) if onset is not None and a < len(onset) else fm
            conf = float(np.clip(0.5 * fm + 0.5 * op, 0, 1))
        vel = int(np.clip(round(float(ev.get("velocity", 90))), 1, 127))  # library scales to 0..128
        notes.append(Note(p, s, max(e, s + 0.03), vel, conf))
    notes.sort(key=lambda n: (n.start, n.pitch))
    return notes


# --------------------------------------------------------------------------- drums

KICK, SNARE, CLOSED_HAT, OPEN_HAT, CRASH = 36, 38, 42, 46, 49
DRUM_BANDS = {KICK: (30.0, 120.0), SNARE: (150.0, 1200.0), CLOSED_HAT: (7000.0, 16000.0)}


def drum_notes(y: np.ndarray, sr: int, from_mix: bool = False) -> list[Note]:
    """Band-wise onset detection: low band = kick, mid = snare, high = hi-hat.
    Hi-hats that ring > 150 ms become open hats; loud long broadband hits
    become crashes. Heuristic, so confidences are modest."""
    import librosa
    from .audio_io import resample

    sr2, hop = 22050, 256
    x = resample(np.asarray(y, dtype=np.float32), sr, sr2)
    S = np.abs(librosa.stft(x, n_fft=1024, hop_length=hop)) ** 2
    freqs = librosa.fft_frequencies(sr=sr2, n_fft=1024)
    fps = sr2 / hop
    cands: list[tuple[float, int, int, float]] = []
    for drum, (lo, hi) in DRUM_BANDS.items():
        band = (freqs >= lo) & (freqs < hi)
        raw = S[band].sum(axis=0)
        # each band against its own level, so quiet hi-hats aren't drowned by the kick
        e = np.log1p(1000.0 * raw / (raw.max() + 1e-12))
        flux = np.maximum(np.diff(e, prepend=e[0]), 0)
        if flux.max() <= 0:
            continue
        flux /= flux.max()
        peaks = librosa.util.peak_pick(flux, pre_max=3, post_max=3, pre_avg=10, post_avg=10,
                                       delta=0.1, wait=int(0.06 * fps))
        if len(peaks) == 0:
            continue
        ref = np.percentile(flux[peaks], 90) + 1e-9
        for p in peaks:
            strength = float(min(flux[p] / ref, 1.5))
            pitch = drum
            if drum == CLOSED_HAT:
                tail = e[p: p + int(0.4 * fps)]
                below = np.flatnonzero(tail < e[p] * 0.5)
                ring = (below[0] if len(below) else len(tail)) / fps
                if ring > 0.15:
                    pitch = OPEN_HAT
                if ring > 0.35 and strength > 0.9:
                    pitch = CRASH
            cands.append((p / fps, drum, pitch, strength))

    # A kick leaks a little into the snare's body band (150-1200 Hz) but its
    # energy stays below 120 Hz; a real snare puts far more energy in the body.
    lo_b = (freqs >= 30) & (freqs < 120)
    body_b = (freqs >= 150) & (freqs < 1200)

    def body_ratio(t):
        f = int(round(t * fps))
        return float(S[body_b, f: f + 4].sum() / (S[lo_b, f: f + 4].sum() + 1e-12))

    # Bleed: a snare also shows up in the kick band, a hi-hat in the snare
    # band. A weak kick/snare at the same moment as a stronger hit in another
    # band is that bleed. Isolated weak hits (ghost notes) are kept, flagged.
    cands.sort()
    ct = np.array([c[0] for c in cands])

    def stronger_elsewhere(t, band, strength):
        a, b = np.searchsorted(ct, t - 0.03), np.searchsorted(ct, t + 0.03)
        return any(cands[j][1] != band and cands[j][3] > strength for j in range(a, b))

    cleaned: list[Note] = []
    for t, band, pitch, strength in cands:
        if band == SNARE and body_ratio(t) < 0.3:
            continue  # that "snare" is the kick's upper end
        if band in (KICK, SNARE) and strength < 0.55 and stronger_elsewhere(t, band, strength):
            continue
        conf = float(np.clip(0.3 + 0.6 * strength, 0, 0.95)) * (0.7 if from_mix else 1.0)
        vel = int(np.clip(50 + 70 * min(strength, 1.0), 1, 127))
        cleaned.append(Note(pitch, t, t + 0.1, vel, conf))
    cleaned.sort(key=lambda n: (n.start, n.pitch))
    return cleaned
