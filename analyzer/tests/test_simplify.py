import pytest

from conftest import load_song, max_simultaneous, run
from qwertone_analyzer import simplify, theory
from qwertone_analyzer.model import Chord, Note, Part, Timeline


def _tl():
    return Timeline([i * 0.5 for i in range(64)], beats_per_bar=4)  # 120 BPM, bars every 2 s


def test_quantize_snaps_to_grid():
    tl = _tl()
    out = simplify.quantize([Note(60, 0.27, 0.71), Note(62, 1.02, 1.04)], tl, 0.5)
    assert [(round(n.start, 3), round(n.end, 3)) for n in out] == [(0.25, 0.75), (1.0, 1.25)]


def test_monophonic_keeps_top_line():
    notes = [Note(60, 0.0, 1.0), Note(72, 0.01, 0.5), Note(64, 0.5, 1.0)]
    out = simplify.monophonic(notes)
    assert [n.pitch for n in out] == [72, 64]
    assert out[0].end <= out[1].start


def test_transpose_prefers_white_keys():
    # E-flat major scale notes -> should move to C (down 3) or nearby white key
    notes = [Note(p, i * 0.5, i * 0.5 + 0.4) for i, p in enumerate([63, 65, 67, 68, 70, 72, 74, 75])]
    s = simplify.choose_transpose(notes, [])
    assert sum(theory.is_black(n.pitch + s) for n in notes) == 0
    assert -6 <= s <= 5


def test_fit_range_folds_into_window():
    notes = [Note(p, 0, 1) for p in (40, 60, 67, 90)]
    out = simplify.fit_range(notes, 60, 72)
    assert all(60 <= n.pitch <= 72 for n in out)
    assert sorted({n.pitch % 12 for n in out}) == sorted({p % 12 for p in (40, 60, 67, 90)})


@pytest.mark.parametrize("difficulty", [1, 2, 3, 4, 5])
def test_difficulty_limits(difficulty):
    tl = _tl()
    mel = Part("melody", "Melody", "melody",
               [Note(66 + (i % 5), i * 0.25, i * 0.25 + 0.3, 90, 0.4 + 0.1 * (i % 6)) for i in range(64)], "test")
    chords = [Chord(i * 2.0, i * 2.0 + 2.0, r, q) for i, (r, q) in enumerate([(2, "maj"), (11, "min"), (7, "maj7"), (9, "7")] * 2)]
    res = simplify.simplify(mel, chords, None, tl, difficulty=difficulty)
    st = simplify.DIFFICULTY[difficulty]
    arr = next(p for p in res["parts"] if p.id == "arrangement")
    notes = [{"start": n.start, "end": n.end} for n in arr.notes]
    assert max_simultaneous(notes) <= st["max_keys"]
    for n in arr.notes:
        b = tl.beat_at(n.start) / st["grid"]
        assert abs(b - round(b)) < 1e-6
    mel_easy = next(p for p in res["parts"] if p.id == "melody-easy")
    assert all(simplify.MELODY_LO <= n.pitch <= simplify.MELODY_HI for n in mel_easy.notes)
    if difficulty < 5:
        assert all(c.quality in (None, "maj", "min", "dim") for c in res["chords"])


def test_simplified_mode_end_to_end(clip, workdirs, fake_stems):
    song = load_song(run(clip["mix"], workdirs, subdir="simple", mode="simplified", difficulty=3))
    sp = song["simplified"]
    assert sp["difficulty"] == 3 and sp["max_keys"] == 2
    assert sp["transpose"] == 5 and sp["key"]["name"] == "C major"  # G major -> C major, no black keys
    ids = [p["id"] for p in song["parts"]]
    assert ids[:2] == ["melody", "chords"] and {"melody-easy", "arrangement"} <= set(ids)
    arr = next(p for p in song["parts"] if p["id"] == "arrangement")
    assert max_simultaneous(arr["notes"]) <= 2
    assert all(n["name"][1] != "#" for n in arr["notes"])
    assert arr["transpose"] == 5 and arr["simplified"]["difficulty"] == 3
    assert [c["symbol"] for c in sp["chords"] if c["symbol"] != "N"][:4] == ["C", "Am", "F", "G"]


def test_max_keys_override(clip, workdirs):
    song = load_song(run(clip["midi"], workdirs, subdir="simple3", mode="simplified", difficulty=4, max_keys=2))
    arr = next(p for p in song["parts"] if p["id"] == "arrangement")
    assert song["simplified"]["max_keys"] == 2 and max_simultaneous(arr["notes"]) <= 2
