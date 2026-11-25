from conftest import chord_accuracy, load_song, melody_recall, run


def test_midi_chords_melody(clip, workdirs):
    song = load_song(run(clip["midi"], workdirs, subdir="midi"))
    assert song["source"]["kind"] == "midi" and song["stems"] == []
    assert song["tempo"]["bpm"] == 120.0 and song["tempo"]["time_signature"] == [4, 4]
    assert song["key"]["name"] == "G major"
    assert chord_accuracy(song["chords"], clip["truth"]["chords"]) == 1.0
    mel = next(p for p in song["parts"] if p["id"] == "melody")
    assert melody_recall(mel["notes"], clip["truth"]["melody"], tol=0.01) == 1.0
    assert mel["low_confidence_count"] == 0
    assert [s["label"] for s in song["sections"]] == ["A", "B", "A"]


def test_midi_full_keeps_every_track(clip, workdirs):
    song = load_song(run(clip["midi"], workdirs, subdir="midi-full", mode="full"))
    by_id = {p["id"]: p for p in song["parts"]}
    assert set(by_id) == {"melody", "strings", "bass", "drums"}
    assert by_id["melody"]["role"] == "melody"
    assert by_id["bass"]["role"] == "bass" and by_id["bass"]["note_count"] == 24
    assert by_id["drums"]["is_drum"] and by_id["drums"]["note_count"] == 176


def test_midi_single_instrument(clip, workdirs):
    song = load_song(run(clip["midi"], workdirs, subdir="midi-bass", mode="bass"))
    (part,) = song["parts"]
    assert part["role"] == "bass" and {n["pitch"] for n in part["notes"]} == {43, 40, 36, 38, 35, 33}
