"""The song folder matches SONG_FORMAT.md / schema/song.schema.json."""

import json
from pathlib import Path

import mido
import numpy as np
import pytest
import soundfile as sf

from conftest import load_song, run

SCHEMA = Path(__file__).parent.parent / "schema" / "song.schema.json"


def _validate(song):
    jsonschema = pytest.importorskip("jsonschema")
    with open(SCHEMA) as f:
        jsonschema.validate(song, json.load(f))


def test_schema_mix(song_mix):
    _validate(song_mix[0])


def test_schema_stems(song_stems):
    _validate(song_stems[0])


def test_schema_midi_simplified(clip, workdirs):
    _validate(load_song(run(clip["midi"], workdirs, subdir="fmt-midi", mode="simplified")))


def test_folder_layout_with_stems(song_stems):
    song, res = song_stems
    d = res.song_dir
    assert d.name.startswith("mix-") and len(d.name.split("-")[-1]) == 8
    assert (d / "song.mid").exists()
    names = {s["name"] for s in song["stems"]}
    assert names == {"vocals", "drums", "bass", "other"}
    lengths = set()
    for s in song["stems"]:
        info = sf.info(d / s["file"])
        assert info.format == "FLAC" and info.channels == 2 and info.samplerate == 44100
        lengths.add(info.frames)
    assert len(lengths) == 1 and abs(lengths.pop() / 44100 - song["duration"]) < 0.01
    for p in song["parts"]:
        assert (d / p["midi_file"]).exists()


def test_low_confidence_flags_match_threshold(song_mix):
    song, _ = song_mix
    thr = song["low_confidence_threshold"]
    for p in song["parts"]:
        assert p["low_confidence_count"] == sum(n["low_confidence"] for n in p["notes"])
        for n in p["notes"]:
            assert n["low_confidence"] == (n["confidence"] < thr)


def test_beat_positions_consistent(song_mix):
    song, _ = song_mix
    beats = [b["time"] for b in song["beats"]]
    for p in song["parts"]:
        for n in p["notes"][:20]:
            i = int(np.floor(n["start_beat"]))
            if 0 <= i < len(beats) - 1:
                t = beats[i] + (n["start_beat"] - i) * (beats[i + 1] - beats[i])
                assert abs(t - n["start"]) < 0.002


def _midi_note_times(path):
    """Seconds of every note_on per track name, using the file's own tempo map."""
    mid = mido.MidiFile(path)
    tempo_changes = []
    tick = 0
    for msg in mid.tracks[0]:
        tick += msg.time
        if msg.type == "set_tempo":
            tempo_changes.append((tick, msg.tempo))

    def secs(t):
        s, last_tick, tempo = 0.0, 0, 500000
        for ct, tp in tempo_changes:
            if ct >= t:
                break
            s += (ct - last_tick) * tempo / 1e6 / mid.ticks_per_beat
            last_tick, tempo = ct, tp
        return s + (t - last_tick) * tempo / 1e6 / mid.ticks_per_beat

    out = {}
    for tr in mid.tracks[1:]:
        name, tick, ons = None, 0, []
        for msg in tr:
            tick += msg.time
            if msg.type == "track_name":
                name = msg.name
            if msg.type == "note_on" and msg.velocity > 0:
                ons.append((secs(tick), msg.note))
        out[name] = ons
    return mid, out


def test_midi_tempo_map_matches_json_times(song_mix):
    song, res = song_mix
    mid, notes = _midi_note_times(res.song_dir / "song.mid")
    assert mid.type == 1 and mid.ticks_per_beat == 480
    assert [t.name for t in mid.tracks][0] == "conductor"
    for p in song["parts"]:
        got = sorted(notes[p["id"]])
        want = sorted((n["start"], n["pitch"]) for n in p["notes"])
        assert len(got) == len(want)
        assert max(abs(a[0] - b[0]) for a, b in zip(got, want)) < 0.003
        assert all(a[1] == b[1] for a, b in zip(got, want))
        assert p["midi_track"] == [t.name for t in mid.tracks].index(p["id"])


def test_midi_conductor_has_meta(song_mix):
    _, res = song_mix
    mid = mido.MidiFile(res.song_dir / "song.mid")
    types = [m.type for m in mid.tracks[0]]
    texts = [m.text for m in mid.tracks[0] if m.type in ("text", "marker")]
    assert "set_tempo" in types and "time_signature" in types and "key_signature" in types
    assert any(t.startswith("chord:") for t in texts) and any(t.startswith("A:") for t in texts)
    # pickup: the clip starts with one silent beat, so 1/4 first, then 4/4
    sigs = [(m.numerator, m.denominator) for m in mid.tracks[0] if m.type == "time_signature"]
    assert sigs[-1] == (4, 4)
