import numpy as np
import pytest
import soundfile as sf

from conftest import melody_recall, needs_crepe
from qwertone_analyzer import melody


def _vocal(clip):
    y, sr = sf.read(clip["dir"] / "stems" / "vocals.wav", dtype="float32")
    return y.mean(axis=1), sr


def test_contour_to_notes_segments_and_flags_shaky_notes():
    hop = 0.01
    times = np.arange(200) * hop
    midi = np.full(200, np.nan)
    conf = np.zeros(200)
    midi[10:60], conf[10:60] = 60.02, 0.95          # steady C4
    midi[60:110], conf[60:110] = 64.0, 0.9          # E4 right after
    midi[130:170] = 67 + 0.45 * np.sin(np.arange(40))  # G4, wobbly
    conf[130:170] = 0.35                             # and barely voiced
    notes = melody.contour_to_notes(times, midi, conf)
    assert [n.pitch for n in notes] == [60, 64, 67]
    assert abs(notes[0].start - 0.10) < 0.02 and abs(notes[1].start - 0.60) < 0.02
    assert notes[0].confidence > 0.8
    assert notes[2].confidence < 0.5  # would be flagged low_confidence


def test_pyin_fallback_on_vocal_stem(clip):
    y, sr = _vocal(clip)
    notes, method, _ = melody.notes_from_audio(y, sr, fmin=65, fmax=1400, method="pyin")
    assert method == "pyin"
    found = [{"pitch": n.pitch, "start": n.start} for n in notes]
    assert melody_recall(found, clip["truth"]["melody"]) >= 0.85


@needs_crepe
@pytest.mark.models
def test_crepe_on_vocal_stem(clip):
    y, sr = _vocal(clip)
    notes, method, _ = melody.notes_from_audio(y, sr, fmin=65, fmax=1400, method="crepe", crepe_model="tiny")
    assert method == "torchcrepe-tiny"
    found = [{"pitch": n.pitch, "start": n.start} for n in notes]
    assert melody_recall(found, clip["truth"]["melody"]) >= 0.95
    assert len(notes) <= len(clip["truth"]["melody"]) + 3
    assert np.mean([n.confidence for n in notes]) > 0.6


@needs_crepe
@pytest.mark.models
def test_pipeline_melody_from_vocal_stem(song_stems, clip):
    song, _ = song_stems
    mel = next(p for p in song["parts"] if p["id"] == "melody")
    assert mel["stem"] == "vocals"
    assert melody_recall(mel["notes"], clip["truth"]["melody"]) >= 0.95
    assert mel["low_confidence_count"] <= 3
