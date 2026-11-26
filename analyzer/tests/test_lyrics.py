"""Lyrics step (Part 10a-lite): .lrc parsing, Tanglish, LRCLIB lookup + cache,
lyrics.json against schema/lyrics.schema.json, and the lyrics / import-lrc commands.
LRCLIB is never contacted: `lrclib.http_get` is replaced by a fake server."""

import json
import shutil
import subprocess
import urllib.parse
from pathlib import Path

import pytest

from qwertone_analyzer import cli, lrc, lrclib, lyrics, pipeline, tanglish
from qwertone_analyzer.cache import file_sha256
from qwertone_analyzer.progress import configure

DATA = Path(__file__).parent / "data"
SAMPLE = DATA / "sample_ta.lrc"
SCHEMA = Path(__file__).parent.parent / "schema" / "lyrics.schema.json"


@pytest.fixture(autouse=True)
def _online_allowed(monkeypatch):
    # conftest turns lyrics lookups off for the whole suite; these tests use a fake LRCLIB instead
    monkeypatch.delenv("QWERTONE_OFFLINE", raising=False)
    monkeypatch.setattr(lrclib, "http_get", _no_network)


def _no_network(url, timeout=10.0):
    raise AssertionError(f"unexpected network call: {url}")


def validate(doc):
    jsonschema = pytest.importorskip("jsonschema")
    with open(SCHEMA) as f:
        jsonschema.validate(doc, json.load(f))


def make_song(tmp_path, name="Kaalai Thendral.mp3", title=None, artist=None, duration=30.0) -> Path:
    """A minimal analysed song: fake audio file + song folder with a 120 BPM beat grid from 0.5 s."""
    audio = tmp_path / "music" / name
    audio.parent.mkdir(parents=True, exist_ok=True)
    audio.write_bytes(b"not really audio " + name.encode())
    sha = file_sha256(audio)
    d = tmp_path / "songs" / f"song-{sha[:8]}"
    d.mkdir(parents=True)
    beats = [{"time": 0.5 + 0.5 * i, "bar": 1 + i // 4, "beat": 1 + i % 4, "downbeat": i % 4 == 0} for i in range(59)]
    song = {"format": "qwertone-song", "format_version": 1, "duration": duration,
            "source": {"kind": "audio", "file": audio.name, "path": str(audio), "sha256": sha, "duration": duration},
            "metadata": {"title": title or audio.stem, "artist": artist, "album": None},
            "tempo": {"bpm": 120.0, "beats_per_bar": 4}, "beats": beats}
    (d / "song.json").write_text(json.dumps(song))
    return d


def opts(tmp_path, **kw):
    return lyrics.LyricsOptions(cache_dir=str(tmp_path / "cache"), **kw)


def read(d):
    with open(d / "lyrics.json") as f:
        return json.load(f)


class FakeLrclib:
    """Answers /api/get and /api/search from a list of records."""

    def __init__(self, records):
        self.records = records
        self.calls = []

    def __call__(self, url, timeout=10.0):
        self.calls.append(url)
        u = urllib.parse.urlparse(url)
        q = dict(urllib.parse.parse_qsl(u.query))
        if u.path.endswith("/get"):
            for r in self.records:
                if lrclib.norm(r["trackName"]) == lrclib.norm(q["track_name"]) and \
                        lrclib.norm(r["artistName"]) == lrclib.norm(q["artist_name"]) and \
                        abs(r["duration"] - float(q["duration"])) <= 2:
                    return 200, json.dumps(r).encode()
            return 404, b'{"code":404,"name":"TrackNotFound"}'
        if u.path.endswith("/search"):
            needle = lrclib.norm(q.get("track_name") or q.get("q"))
            hits = [r for r in self.records if lrclib.norm(r["trackName"]) in needle
                    or needle in lrclib.norm(r["trackName"] + " " + r["artistName"])]
            return 200, json.dumps(hits).encode()
        return 404, b""


def record(id=4242, synced=None, plain=None, title="Kaalai Thendral", artist="Qwertone Test Band", duration=30.0,
           instrumental=False):
    return {"id": id, "trackName": title, "artistName": artist, "albumName": "Test Songs", "duration": duration,
            "instrumental": instrumental, "plainLyrics": plain, "syncedLyrics": synced}


# ============================================================================ .lrc parsing

def test_parse_sample_lrc():
    doc = lrc.parse(SAMPLE.read_text())
    assert doc.synced and doc.has_words and doc.language == "ta"
    assert doc.tags["ar"] == "Qwertone Test Band" and doc.offset == pytest.approx(0.2)
    starts = [l.start for l in doc.lines]
    assert starts == [2.0, 5.5, 9.0, 12.5, 17.0, 20.0]                 # offset applied, chorus repeated, sorted
    assert doc.lines[2].text == doc.lines[5].text == "வா வா என் அன்பே"
    morning = doc.lines[3]
    assert [w.text for w in morning.words] == ["Morning", "breeze", "is", "here"]
    assert [w.start for w in morning.words] == [12.5, 13.0, 13.5, 13.8]
    assert morning.words[-1].end == 14.5 and morning.end == 14.5        # trailing word stamp ends the line


def test_parse_variants():
    doc = lrc.parse("﻿[00:01.5]one\r\n[00:03:25]two\n[0:05]three\n[01:02.345]four\n")
    assert [l.start for l in doc.lines] == [1.5, 3.25, 5.0, 62.345]
    gap = lrc.parse("[00:01.00]a\n[00:04.00]\n[00:09.00]b\n")
    assert gap.lines[0].end == 4.0 and gap.lines[1].end is None
    plain = lrc.parse("[ar:Someone]\nfirst line\n\nsecond line\n")
    assert not plain.synced and [l.text for l in plain.lines] == ["first line", "second line"]
    neg = lrc.parse("[offset:-500]\n[00:01.00]late\n")
    assert neg.lines[0].start == 1.5


# ============================================================================ Tanglish

@pytest.mark.parametrize("tamil,expected", [
    ("வணக்கம்", "vanakkam"), ("தமிழ்", "thamizh"), ("நன்றி", "nandri"), ("அன்பு", "anbu"),
    ("கொஞ்சம்", "konjam"), ("எங்கே", "enge"), ("காதல்", "kaadhal"), ("வந்தது", "vandhadhu"),
    ("பச்சை", "pacchai"), ("காற்று", "kaatru"), ("மகன்", "magan"), ("சங்கம்", "sangam"),
    ("முத்தம்", "muttham"), ("கண்ணில்", "kannil"), ("பாட்டு", "paattu"), ("நெஞ்சில்", "nenjil"),
    ("வாழ்க", "vaazhga"), ("கட்சி", "katchi"), ("பயிற்சி", "payirchi"), ("ஞானம்", "gnaanam"),
    ("ஸ்ரீ", "sri"), ("ஏன்", "aen"), ("உயிரே", "uyire"), ("அஜித்", "ajith"), ("௨௦௨௬", "2026"),
])
def test_tanglish_words(tamil, expected):
    assert tanglish.transliterate(tamil) == expected


def test_tanglish_lines_and_mixed_text():
    assert tanglish.transliterate("வா வா என் அன்பே") == "vaa vaa en anbe"
    assert tanglish.transliterate("Hello தமிழ், rock-ல் ஆடு!") == "Hello thamizh, rock-l aadu!"
    import unicodedata
    assert tanglish.transliterate(unicodedata.normalize("NFD", "கொஞ்சம் போ")) == "konjam po"


def test_line_language():
    assert lyrics.line_language("காலை தென்றல்") == "ta"
    assert lyrics.line_language("Morning breeze is here") == "en"
    assert lyrics.line_language("nee enna solla vandha") == "ta-Latn"
    assert lyrics.line_language("नमस्ते दुनिया") == "hi"
    assert lyrics.line_language("Morning breeze", song_hint="ta-Latn") == "ta-Latn"


# ============================================================================ lyrics.json from a local .lrc

def test_sidecar_lrc_to_lyrics_json(tmp_path):
    d = make_song(tmp_path)
    audio = Path(json.loads((d / "song.json").read_text())["source"]["path"])
    shutil.copy(SAMPLE, audio.with_suffix(".lrc"))
    res = lyrics.run_lyrics(d, opts(tmp_path))
    doc = read(d)
    validate(doc)
    assert res.status == doc["status"] == "synced" and not res.skipped and res.lrclib_requests == 0
    assert doc["language"] == "ta" and doc["has_word_timing"] and doc["offset"] == 0.2
    lines = doc["lines"]
    assert len(lines) == 6
    first = lines[0]
    assert (first["start"], first["end"]) == (2.0, 5.5)
    assert (first["start_beat"], first["end_beat"]) == (3.0, 10.0)      # beats every 0.5 s from 0.5 s
    assert first["tanglish"] == "kaalai thendral vandhadhu" and first["translation"] is None
    assert lines[3]["language"] == "en" and lines[3]["tanglish"] is None and lines[3]["end"] == 14.5
    assert lines[4]["end"] == 20.0                                       # until the next line
    assert lines[5]["end"] == 30.0                                       # last line: until the song ends
    w = lines[4]["words"]
    assert [x["tanglish"] for x in w] == ["kannil", "un", "mugam"]
    assert w[-1]["end"] == 20.0 and w[0]["start_beat"] == 33.0
    src = doc["sources"]
    assert src["words"] == src["timing"]
    assert src["words"]["kind"] == "file" and src["words"]["file"].endswith(".lrc") and src["words"]["fetched"]
    assert src["words"]["by"] == "qwertone tests"
    assert src["translation"] is None and src["transliteration"]["kind"] == "generated"


def test_up_to_date_is_skipped_and_changes_rebuild(tmp_path):
    d = make_song(tmp_path)
    side = Path(json.loads((d / "song.json").read_text())["source"]["path"]).with_suffix(".lrc")
    side.write_text("[00:01.00]ஒன்று\n[00:03.00]இரண்டு\n")
    assert not lyrics.run_lyrics(d, opts(tmp_path)).skipped
    assert lyrics.run_lyrics(d, opts(tmp_path)).skipped
    side.write_text("[00:01.00]ஒன்று\n[00:03.00]மூன்று\n")
    res = lyrics.run_lyrics(d, opts(tmp_path))
    assert not res.skipped and read(d)["lines"][1]["tanglish"] == "moondru"
    assert not lyrics.run_lyrics(d, opts(tmp_path, force=True)).skipped


def test_language_override(tmp_path):
    d = make_song(tmp_path)
    Path(json.loads((d / "song.json").read_text())["source"]["path"]).with_suffix(".lrc").write_text(
        "[00:01.00]nee varuvaai ena\n[00:04.00]naan irundhen\n")
    lyrics.run_lyrics(d, opts(tmp_path, online=False, language="ta-Latn"))
    doc = read(d)
    assert doc["language"] == "ta-Latn"
    assert all(l["language"] == "ta-Latn" and l["tanglish"] == l["text"] for l in doc["lines"])
    assert doc["sources"]["transliteration"] is None                     # nothing was transliterated


# ============================================================================ LRCLIB

def test_lrclib_synced_lookup_and_cache(tmp_path, monkeypatch):
    fake = FakeLrclib([record(synced=SAMPLE.read_text().split("\n", 8)[-1])])   # without the ID tags
    monkeypatch.setattr(lrclib, "http_get", fake)
    d = make_song(tmp_path, name="Qwertone Test Band - Kaalai Thendral (Official Video).mp3")
    res = lyrics.run_lyrics(d, opts(tmp_path))
    doc = read(d)
    validate(doc)
    assert res.status == "synced" and res.lrclib_requests == 1
    assert doc["song"]["artist"] == "Qwertone Test Band"                # split from "Artist - Title" file name
    src = doc["sources"]
    assert src["words"]["kind"] == "lrclib" and src["words"]["id"] == 4242
    assert src["words"]["url"] == "https://lrclib.net/api/get/4242" and src["words"]["fetched"]
    assert doc["lines"][0]["start"] == 2.2                                # no [offset] tag in the LRCLIB text
    n = len(fake.calls)
    # rebuilding uses the cached answer, no new request
    res = lyrics.run_lyrics(d, opts(tmp_path, force=True))
    assert res.lrclib_requests == 0 and len(fake.calls) == n
    # --refresh asks again
    assert lyrics.run_lyrics(d, opts(tmp_path, refresh=True)).lrclib_requests == 1


def test_lrclib_get_endpoint_with_tags(tmp_path, monkeypatch):
    fake = FakeLrclib([record(synced="[00:01.00]ஒன்று\n[00:02.00]இரண்டு\n", duration=31.0)])
    monkeypatch.setattr(lrclib, "http_get", fake)
    d = make_song(tmp_path, title="Kaalai Thendral", artist="Qwertone Test Band")
    lyrics.run_lyrics(d, opts(tmp_path))
    assert "/api/get?" in fake.calls[0] and "duration=30" in fake.calls[0] and len(fake.calls) == 1
    assert read(d)["lines"][1]["tanglish"] == "irandu"


def test_lrclib_wrong_duration_is_rejected(tmp_path, monkeypatch):
    monkeypatch.setattr(lrclib, "http_get", FakeLrclib([record(synced="[00:01.00]x\n", duration=95.0)]))
    d = make_song(tmp_path)
    assert lyrics.run_lyrics(d, opts(tmp_path)).status == "not_found"
    validate(read(d))


def test_not_found_is_cached_and_offline_makes_no_calls(tmp_path, monkeypatch):
    fake = FakeLrclib([])
    monkeypatch.setattr(lrclib, "http_get", fake)
    d = make_song(tmp_path, title="Nothing Here", artist="Nobody")
    assert lyrics.run_lyrics(d, opts(tmp_path)).status == "not_found"
    n = len(fake.calls)
    assert lyrics.run_lyrics(d, opts(tmp_path, force=True)).lrclib_requests == 0 and len(fake.calls) == n
    d2 = make_song(tmp_path / "other", name="Different.mp3")
    assert lyrics.run_lyrics(d2, opts(tmp_path, online=False)).status == "not_found"
    assert len(fake.calls) == n


def test_network_error_is_not_cached(tmp_path, monkeypatch):
    def down(url, timeout=10.0):
        raise lrclib.LrclibError("LRCLIB not reachable: test")
    monkeypatch.setattr(lrclib, "http_get", down)
    d = make_song(tmp_path)
    res = lyrics.run_lyrics(d, opts(tmp_path))
    assert res.status == "not_found" and res.notes
    fake = FakeLrclib([record(synced="[00:01.00]ஒன்று\n")])
    monkeypatch.setattr(lrclib, "http_get", fake)
    assert lyrics.run_lyrics(d, opts(tmp_path, force=True)).status == "synced"


def test_instrumental(tmp_path, monkeypatch):
    monkeypatch.setattr(lrclib, "http_get", FakeLrclib([record(instrumental=True)]))
    d = make_song(tmp_path, title="Kaalai Thendral", artist="Qwertone Test Band")
    assert lyrics.run_lyrics(d, opts(tmp_path)).status == "instrumental"
    doc = read(d)
    validate(doc)
    assert doc["lines"] == [] and doc["sources"]["words"]["kind"] == "lrclib"


def test_local_plain_words_with_lrclib_timing(tmp_path, monkeypatch):
    synced = "[00:01.00]காலை தென்றல் வந்தது\n[00:04.00]நெஞ்சில் ஒரு பாட்டு\n"
    monkeypatch.setattr(lrclib, "http_get", FakeLrclib([record(synced=synced)]))
    d = make_song(tmp_path, title="Kaalai Thendral", artist="Qwertone Test Band")
    side = Path(json.loads((d / "song.json").read_text())["source"]["path"]).with_suffix(".lrc")
    side.write_text("காலை தென்றல் வந்தது!\nநெஞ்சில் ஒரு பாட்டு\n")      # plain, slightly different punctuation
    lyrics.run_lyrics(d, opts(tmp_path))
    doc = read(d)
    validate(doc)
    assert doc["status"] == "synced"
    assert doc["sources"]["words"]["kind"] == "file" and doc["sources"]["timing"]["kind"] == "lrclib"
    assert doc["lines"][0]["text"] == "காலை தென்றல் வந்தது!" and doc["lines"][1]["start"] == 4.0


def test_local_plain_kept_when_lrclib_text_differs(tmp_path, monkeypatch):
    monkeypatch.setattr(lrclib, "http_get", FakeLrclib([record(synced="[00:01.00]something else entirely\n")]))
    d = make_song(tmp_path, title="Kaalai Thendral", artist="Qwertone Test Band")
    Path(json.loads((d / "song.json").read_text())["source"]["path"]).with_suffix(".lrc").write_text("ஒன்று\nஇரண்டு\n")
    res = lyrics.run_lyrics(d, opts(tmp_path))
    doc = read(d)
    validate(doc)
    assert res.status == "plain" and res.notes and doc["sources"]["timing"] is None
    assert doc["lines"][0]["start"] is None and doc["lines"][0]["start_beat"] is None


# ============================================================================ embedded tags

@pytest.mark.skipif(shutil.which("ffmpeg") is None or shutil.which("ffprobe") is None, reason="needs ffmpeg")
def test_embedded_lyrics_tag(tmp_path):
    audio = tmp_path / "music" / "tagged.flac"
    audio.parent.mkdir(parents=True)
    subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "anullsrc=r=22050:cl=mono", "-t", "1",
                    "-metadata", "LYRICS=[00:00.50]ஒன்று\n[00:00.80]இரண்டு", str(audio)], check=True)
    sha = file_sha256(audio)
    d = tmp_path / "songs" / f"tagged-{sha[:8]}"
    d.mkdir(parents=True)
    (d / "song.json").write_text(json.dumps({"duration": 1.0, "metadata": {"title": "tagged"},
                                             "source": {"file": audio.name, "path": str(audio), "sha256": sha},
                                             "beats": []}))
    lyrics.run_lyrics(d, opts(tmp_path, online=False))
    doc = read(d)
    validate(doc)
    assert doc["status"] == "synced" and doc["sources"]["words"]["kind"] == "embedded"
    assert doc["sources"]["words"]["tag"].lower() == "lyrics"
    assert doc["lines"][0]["start_beat"] is None and doc["lines"][1]["end"] == 1.0


# ============================================================================ commands

def test_import_lrc_command_wins_over_sidecar(tmp_path, capsys):
    d = make_song(tmp_path)
    audio = Path(json.loads((d / "song.json").read_text())["source"]["path"])
    audio.with_suffix(".lrc").write_text("[00:01.00]sidecar line\n")
    common = ["-o", str(tmp_path / "songs"), "--cache-dir", str(tmp_path / "cache"), "-q", "--offline"]
    assert cli.main(["lyrics", str(audio), *common]) == 0                # found by the audio file
    assert read(d)["lines"][0]["text"] == "sidecar line"
    assert cli.main(["import-lrc", str(SAMPLE), str(d), *common, "--show"]) == 0
    out = capsys.readouterr().out
    assert "synced, 6 lines" in out and "kaalai thendral vandhadhu" in out
    doc = read(d)
    assert doc["sources"]["words"]["name"] == "imported .lrc"
    assert doc["sources"]["words"]["file"] == str(SAMPLE.resolve())
    assert doc["sources"]["words"]["copy"] == "lyrics/imported.lrc" and (d / "lyrics" / "imported.lrc").exists()
    # re-running just the lyrics step keeps the import
    assert cli.main(["lyrics", str(d / "song.json"), *common]) == 0
    assert "[up to date]" in capsys.readouterr().out
    assert cli.main(["lyrics", "--all", *common, "--force"]) == 0
    assert read(d)["sources"]["words"]["name"] == "imported .lrc"
    configure(quiet=True)


def test_lyrics_command_errors(tmp_path, capsys):
    assert cli.main(["lyrics", str(tmp_path / "nope"), "-q"]) == 2
    stray = tmp_path / "x.mp3"
    stray.write_bytes(b"x")
    assert cli.main(["lyrics", str(stray), "-o", str(tmp_path), "-q"]) == 2
    assert "hasn't been analysed" in capsys.readouterr().err
    configure(quiet=True)


# ============================================================================ as part of analyze

def test_analyze_writes_lyrics_json(clip, tmp_path):
    song = tmp_path / "in" / "clip.mid"
    song.parent.mkdir()
    shutil.copy(clip["midi"], song)
    song.with_suffix(".lrc").write_text("[00:02.00]வா வா என் அன்பே\n[00:06.00]second line\n")
    o = pipeline.Options(mode="chords", out_dir=str(tmp_path / "songs"), cache_dir=str(tmp_path / "c"),
                         lyrics_online=False)
    res = pipeline.analyze(song, o)
    assert res.lyrics is not None and res.lyrics.status == "synced"
    doc = read(res.song_dir)
    validate(doc)
    with open(res.song_json) as f:
        beats = [b["time"] for b in json.load(f)["beats"]]
    from qwertone_analyzer.model import Timeline
    assert doc["lines"][0]["start_beat"] == pytest.approx(Timeline(beats).beat_at(2.0), abs=1e-3)
    # lyrics options don't change the song fingerprint: a second run skips analysis but still checks lyrics
    again = pipeline.analyze(song, pipeline.Options(mode="chords", out_dir=str(tmp_path / "songs"),
                                                    cache_dir=str(tmp_path / "c"), lyrics_online=True))
    assert again.skipped and again.lyrics.status == "synced"
    off = pipeline.analyze(song, pipeline.Options(mode="chords", out_dir=str(tmp_path / "s2"),
                                                  cache_dir=str(tmp_path / "c"), lyrics=False))
    assert off.lyrics is None and not (off.song_dir / "lyrics.json").exists()


@pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="needs ffmpeg")
def test_embedded_sylt_synced_frame(tmp_path):
    id3 = pytest.importorskip("mutagen.id3")
    audio = tmp_path / "music" / "sylt.mp3"
    audio.parent.mkdir(parents=True)
    r = subprocess.run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i", "anullsrc=r=22050:cl=mono", "-t", "2",
                        str(audio)], capture_output=True)
    if r.returncode != 0:
        pytest.skip("ffmpeg without an mp3 encoder")
    tags = id3.ID3()
    tags.add(id3.USLT(encoding=3, lang="tam", desc="", text="plain words"))
    tags.add(id3.SYLT(encoding=3, lang="tam", format=2, type=1, text=[("ஒன்று", 500), ("இரண்டு", 1200)]))
    tags.save(str(audio))
    sha = file_sha256(audio)
    d = tmp_path / "songs" / f"sylt-{sha[:8]}"
    d.mkdir(parents=True)
    (d / "song.json").write_text(json.dumps({"duration": 2.0, "metadata": {"title": "sylt"},
                                             "source": {"file": audio.name, "path": str(audio), "sha256": sha},
                                             "beats": []}))
    lyrics.run_lyrics(d, opts(tmp_path, online=False))
    doc = read(d)
    validate(doc)
    assert doc["status"] == "synced" and doc["sources"]["words"]["tag"].startswith("SYLT")
    assert [(l["start"], l["tanglish"]) for l in doc["lines"]] == [(0.5, "ondru"), (1.2, "irandu")]
