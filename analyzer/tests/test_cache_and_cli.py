import json
import shutil

from conftest import run
from qwertone_analyzer import cli, pipeline
from qwertone_analyzer.progress import configure


def test_second_run_is_skipped(clip, workdirs):
    first = run(clip["mix"], workdirs, subdir="cache-a", separate=False, mode="chords")
    again = run(clip["mix"], workdirs, subdir="cache-a", separate=False, mode="chords")
    assert not first.skipped and again.skipped
    assert again.song_json == first.song_json


def test_other_mode_reuses_cached_stages(clip, workdirs):
    run(clip["mix"], workdirs, subdir="cache-b", separate=False, mode="chords")
    res = run(clip["mix"], workdirs, subdir="cache-c", separate=False, mode="drums")
    for stage in ("features", "rhythm", "chords", "key", "sections"):
        assert stage in res.cache_hits
    assert set(res.cache_misses) <= {"notes-drums"}  # only the new part is computed


def test_stems_are_separated_once(clip, workdirs, fake_stems, tmp_path):
    before = fake_stems.calls
    opts = dict(cache_dir=str(tmp_path / "c"), separate=True)
    pipeline.analyze(clip["mix"], pipeline.Options(mode="chords", out_dir=str(tmp_path / "o1"), **opts))
    pipeline.analyze(clip["mix"], pipeline.Options(mode="drums", out_dir=str(tmp_path / "o2"), **opts))
    assert fake_stems.calls - before == 1


def test_lost_song_folder_is_rebuilt_from_cache(clip, workdirs):
    res = run(clip["mix"], workdirs, subdir="cache-d", separate=False, mode="chords")
    shutil.rmtree(res.song_dir)
    again = run(clip["mix"], workdirs, subdir="cache-d", separate=False, mode="chords")
    assert not again.skipped and again.cache_misses == [] and again.song_json.exists()


def test_force_recomputes(clip, workdirs):
    run(clip["mix"], workdirs, subdir="cache-e", separate=False, mode="chords")
    res = run(clip["mix"], workdirs, subdir="cache-e", separate=False, mode="chords", force=True)
    assert not res.skipped and "rhythm" in res.cache_misses


def test_cli_analyze_and_json_progress(clip, tmp_path, capsys):
    code = cli.main([str(clip["midi"]), "--mode", "chords", "-o", str(tmp_path), "--cache-dir",
                     str(tmp_path / "c"), "--json-progress"])
    out, err = capsys.readouterr()
    assert code == 0
    song_dir = out.strip().splitlines()[-1]
    with open(f"{song_dir}/song.json") as f:
        assert json.load(f)["analyzer"]["mode"] == "chords"
    events = [json.loads(line) for line in err.splitlines() if line.startswith("{")]
    assert events[-1]["event"] == "done" and events[-1]["song_dir"] == song_dir
    assert any(e["event"] == "stage" for e in events)
    configure(quiet=True)


def test_cli_errors_are_clean(tmp_path, capsys):
    bad = tmp_path / "notes.txt"
    bad.write_text("hi")
    assert cli.main([str(bad), "-o", str(tmp_path)]) == 2
    assert "unsupported file type" in capsys.readouterr().err
    assert cli.main([str(tmp_path / "missing.mp3")]) == 2


def test_cli_doctor(capsys):
    assert cli.main(["doctor"]) == 0
    out = capsys.readouterr().out
    assert "demucs" in out and "torchcrepe" in out


def test_mode_aliases():
    assert pipeline.Options(mode="piano").normalized().instrument == "piano"
    assert pipeline.Options(mode="chords-melody").normalized().mode == "chords+melody"
    assert pipeline.Options(mode="simplified").normalized().simplify is True
