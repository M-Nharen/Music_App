"""Tempo, beats, key, chords and sections on the generated clip (no models needed)."""

import numpy as np

from conftest import chord_accuracy


def test_tempo_and_meter(song_mix, clip):
    song, _ = song_mix
    assert abs(song["tempo"]["bpm"] - 120.0) < 1.0
    assert song["tempo"]["time_signature"] == [4, 4]


def test_beats_line_up(song_mix, clip):
    song, _ = song_mix
    detected = np.array([b["time"] for b in song["beats"]])
    truth = np.array(clip["truth"]["beats"])
    errors = [np.min(np.abs(detected - t)) for t in truth]
    assert np.mean(np.array(errors) < 0.05) >= 0.95


def test_downbeats_are_bar_starts(song_mix, clip):
    song, _ = song_mix
    downs = [b["time"] for b in song["beats"] if b["downbeat"]]
    bar_starts = [c["start"] for c in clip["truth"]["chords"]]
    hits = sum(any(abs(d - b) < 0.06 for d in downs) for b in bar_starts)
    assert hits >= len(bar_starts) - 1


def test_key(song_mix):
    song, _ = song_mix
    assert song["key"]["name"] == "G major"


def test_chords(song_mix, clip):
    song, _ = song_mix
    assert chord_accuracy(song["chords"], clip["truth"]["chords"]) >= 0.9


def test_chords_cover_the_song_and_have_numerals(song_mix):
    song, _ = song_mix
    ch = song["chords"]
    assert ch[0]["start"] == 0.0
    assert abs(ch[-1]["end"] - song["duration"]) < 1e-3
    for a, b in zip(ch, ch[1:]):
        assert abs(a["end"] - b["start"]) < 1e-6
    romans = {c["symbol"]: c["roman"] for c in ch}
    assert romans["G"] == "I" and romans["D"] == "V" and romans["Em"] == "vi"


def test_sections_find_aba(song_mix, clip):
    song, _ = song_mix
    secs = song["sections"]
    labels = [s["label"] for s in secs]
    assert len(secs) == 3
    assert labels[0] == labels[2] != labels[1]
    for s, t in zip(secs[1:], clip["truth"]["sections"][1:]):
        assert abs(s["start"] - t["start"]) < 0.3
