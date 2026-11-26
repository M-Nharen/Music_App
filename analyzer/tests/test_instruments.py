"""Drums, bass, full transcription, piano."""

import numpy as np
import pytest
import soundfile as sf

from conftest import (REAL_SEPARATE, load_song, needs_basic_pitch, needs_crepe, needs_demucs, needs_piano, run)
from qwertone_analyzer import stems, transcribe


def _truth_drums(truth):
    beats = np.array(truth["beats"])
    hats = []
    for i, b in enumerate(beats):
        sub = 4 if 4 <= i // 4 < 8 else 2  # B section has 16th hats
        hats += [b + k * 0.5 / sub for k in range(sub)]
    return {36: beats[0::2], 38: beats[1::2], 42: np.array(hats)}


def _pr(detected, truth, tol=0.035):
    detected = np.asarray(detected)
    if len(detected) == 0:
        return 0.0, 0.0
    recall = np.mean([np.min(np.abs(detected - t)) < tol for t in truth])
    precision = np.mean([np.min(np.abs(truth - d)) < tol for d in detected])
    return recall, precision


def test_drums_from_drum_stem(clip):
    y, sr = sf.read(clip["dir"] / "stems" / "drums.wav", dtype="float32")
    notes = transcribe.drum_notes(y.mean(axis=1), sr)
    for pitch, truth in _truth_drums(clip["truth"]).items():
        got = [n.start for n in notes if n.pitch == pitch or (pitch == 42 and n.pitch in (46, 49))]
        r, p = _pr(got, truth)
        assert r >= 0.9 and p >= 0.9, (pitch, r, p)


def test_drums_mode_on_mix(clip, workdirs):
    song = load_song(run(clip["mix"], workdirs, subdir="drums-mix", mode="drums", separate=False))
    (part,) = song["parts"]
    assert part["is_drum"] and part["midi_channel"] == 9
    kicks = [n["start"] for n in part["notes"] if n["pitch"] == 36]
    r, _ = _pr(kicks, _truth_drums(clip["truth"])[36])
    assert r >= 0.85
    assert all(n["confidence"] <= 0.7 for n in part["notes"])  # from the mix: never fully trusted


@needs_crepe
@pytest.mark.models
def test_bass_mode_with_stems(clip, workdirs, fake_stems):
    song = load_song(run(clip["mix"], workdirs, subdir="bass", mode="bass"))
    (part,) = song["parts"]
    assert part["stem"] == "bass" and part["program"] == 33
    roots = {"G": 43, "Em": 40, "C": 36, "D": 38, "Bm": 35, "Am": 33}
    ok = 0
    for c in clip["truth"]["chords"]:
        ok += any(n["pitch"] == roots[c["symbol"]] and c["start"] - 0.05 <= n["start"] < c["end"] for n in part["notes"])
    assert ok >= 0.75 * len(clip["truth"]["chords"])


@needs_basic_pitch
@pytest.mark.models
def test_full_mode_with_stems(clip, workdirs, fake_stems):
    song = load_song(run(clip["mix"], workdirs, subdir="full", mode="full"))
    ids = {p["id"] for p in song["parts"]}
    assert {"vocals", "bass", "other", "drums"} <= ids
    vocals = next(p for p in song["parts"] if p["id"] == "vocals")
    truth = clip["truth"]["melody"]
    hit = sum(any(n["pitch"] == m["pitch"] and abs(n["start"] - m["start"]) < 0.08 for n in vocals["notes"]) for m in truth)
    assert hit / len(truth) >= 0.9
    other = next(p for p in song["parts"] if p["id"] == "other")
    pad = [n for n in other["notes"] if 45 <= n["pitch"] <= 62]  # the pad's chord tones
    assert other["note_count"] > 0 and len(pad) >= 0.6 * other["note_count"]


@needs_demucs
@pytest.mark.models
def test_demucs_wrapper_with_untrained_model(clip, monkeypatch):
    """Weights can't be downloaded in CI; an untrained HTDemucs exercises the
    same code path (normalisation, chunking, stem naming, lengths)."""
    from demucs.htdemucs import HTDemucs
    monkeypatch.setattr(stems, "load_model",
                        lambda name: HTDemucs(sources=["drums", "bass", "other", "vocals"], samplerate=44100, segment=4))
    y, sr = sf.read(clip["mix"], dtype="float32")
    mix = y.T[:, : sr * 5]
    out = REAL_SEPARATE(mix, sr, "htdemucs", "cpu")
    assert set(out) == {"drums", "bass", "other", "vocals"}
    assert all(v.shape == mix.shape and np.isfinite(v).all() for v in out.values())


@needs_piano
@pytest.mark.models
def test_piano_wrapper_with_random_checkpoint(clip, tmp_path):
    """Same idea for the piano model: random weights in the real checkpoint layout."""
    import torch
    from piano_transcription_inference import config
    from piano_transcription_inference.models import Note_pedal

    m = Note_pedal(frames_per_second=config.frames_per_second, classes_num=config.classes_num)
    ck = tmp_path / "piano.pth"
    torch.save({"model": {"note_model": m.note_model.state_dict(), "pedal_model": m.pedal_model.state_dict()}}, ck)
    transcribe._piano = None
    y, sr = sf.read(clip["mix"], dtype="float32")
    try:
        notes = transcribe.piano_notes(y.mean(axis=1)[: sr * 4], sr, "cpu", checkpoint=str(ck))
    finally:
        transcribe._piano = None
    assert all(0 <= n.confidence <= 1 and n.end > n.start and 21 <= n.pitch <= 108 for n in notes)
