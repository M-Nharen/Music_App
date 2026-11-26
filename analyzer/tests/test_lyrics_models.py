"""Lyrics engine, the model parts (Part 10a-full): Whisper transcription, aligning
plain lyrics, English translation, language detection per song and per line,
AcoustID. Whisper, the translator, fpcalc and the AcoustID server are fakes
here, so this runs without models or network. `QWERTONE_TEST_LYRICS_MODELS=1`
also runs the real models at the end (downloads ~2 GB once)."""

import json
import os
import shutil
from pathlib import Path

import pytest

from qwertone_analyzer import acoustid, align, asr, cli, lrc, lrclib, lyrics, pipeline, translate

from test_lyrics import FakeLrclib, make_song, opts, read, record, validate

DATA = Path(__file__).parent / "data"


@pytest.fixture(autouse=True)
def _models_allowed(monkeypatch, tmp_path):
    monkeypatch.delenv("QWERTONE_LIGHT", raising=False)
    monkeypatch.delenv("QWERTONE_OFFLINE", raising=False)
    monkeypatch.delenv("ACOUSTID_API_KEY", raising=False)
    monkeypatch.setenv("XDG_CONFIG_HOME", str(tmp_path / "config"))
    monkeypatch.setattr(lrclib, "http_get", FakeLrclib([]))
    monkeypatch.setattr(asr, "available", lambda: True)
    monkeypatch.setattr(translate, "available", lambda: True)
    monkeypatch.setattr(acoustid, "http_post", _no_post)
    monkeypatch.setattr(translate, "run_translation", FakeTranslator())


def _no_post(url, data, timeout=10.0):
    raise AssertionError("unexpected AcoustID call")


def seg(start, end, words, text=None, **kw):
    """A Whisper segment from [(word, start, end, prob)]."""
    ws = [{"word": (" " if i else "") + w, "start": s, "end": e, "probability": p}
          for i, (w, s, e, p) in enumerate(words)]
    return {"start": start, "end": end, "text": text if text is not None else " ".join(w[0] for w in words),
            "avg_logprob": kw.get("avg_logprob", -0.3), "no_speech_prob": kw.get("no_speech_prob", 0.05),
            "compression_ratio": kw.get("compression_ratio", 1.2), "words": ws}


class FakeWhisper:
    def __init__(self, segments, language="ta", detected=None, prob=0.9):
        self.segments, self.language, self.detected, self.prob = segments, language, detected or language, prob
        self.calls = []

    def __call__(self, audio_path, *, model, device, language, prompt, offline):
        self.calls.append({"audio": Path(audio_path), "model": model, "language": language, "prompt": prompt,
                           "offline": offline})
        lang = language or self.language
        return {"language": lang, "language_probability": None if language else self.prob,
                "detected": None if language else self.detected, "top_languages": [[self.detected, self.prob]],
                "multilingual": False, "duration": 30.0, "segments": self.segments}


class FakeTranslator:
    """English = "EN(<nllb code>):<text>"."""

    def __init__(self):
        self.calls = []

    def __call__(self, texts, src_codes, *, model, device, offline):
        self.calls.append(list(zip(texts, src_codes)))
        return [f"EN({c}):{t}" for t, c in zip(texts, src_codes)]


def with_vocals(d: Path) -> Path:
    """Give the song folder a vocal stem like the analyzer does."""
    (d / "stems").mkdir(exist_ok=True)
    v = d / "stems" / "vocals.flac"
    v.write_bytes(b"fake flac")
    song = json.loads((d / "song.json").read_text())
    song["stems"] = [{"name": "vocals", "file": "stems/vocals.flac"}]
    (d / "song.json").write_text(json.dumps(song))
    return v


TAMIL_SEGMENTS = [
    seg(0.0, 2.0, [("Thank", 0.2, 0.5, 0.3), ("you", 0.5, 0.7, 0.3), ("for", 0.7, 0.9, 0.3),
                   ("watching", 0.9, 1.5, 0.3)]),                                   # a typical hallucination
    seg(2.0, 9.4, [("காலை", 2.0, 2.6, 0.9), ("தென்றல்", 2.6, 3.4, 0.8), ("வந்தது", 3.4, 4.6, 0.9),
                   ("கண்ணில்", 5.5, 6.2, 0.7), ("உன்", 6.2, 6.6, 0.9), ("முகம்", 6.6, 7.4, 0.6)]),
    seg(12.5, 14.5, [("Morning", 12.5, 13.0, 0.95), ("breeze", 13.0, 13.5, 0.9), ("is", 13.5, 13.8, 0.9),
                     ("here", 13.8, 14.5, 0.9)]),
]


# ============================================================================ transcription

def test_whisper_transcribes_when_nothing_is_found(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    with_vocals(d)
    fake = FakeWhisper(TAMIL_SEGMENTS, language="ta", prob=0.83)
    monkeypatch.setattr(asr, "run_whisper", fake)
    res = lyrics.run_lyrics(d, opts(tmp_path, online=False))
    doc = read(d)
    validate(doc)
    assert res.status == doc["status"] == "synced"
    assert doc["auto_generated"] == {"words": True, "timing": True, "translation": True}
    w = doc["sources"]["words"]
    assert w["kind"] == "whisper" and "auto-generated" in w["name"] and w["file"] == "stems/vocals.flac"
    assert w["model"] == "large-v3-turbo" and doc["sources"]["timing"] == w
    assert fake.calls[0]["audio"] == d / "stems" / "vocals.flac" and fake.calls[0]["language"] is None
    assert fake.calls[0]["offline"] is True
    # the "Thank you for watching" segment is dropped; the 0.9 s pause splits the Tamil segment in two
    texts = [l["text"] for l in doc["lines"]]
    assert texts == ["காலை தென்றல் வந்தது", "கண்ணில் உன் முகம்", "Morning breeze is here"]
    first = doc["lines"][0]
    assert first["start"] == 2.0 and first["end"] == 4.6 and first["start_beat"] == 3.0
    assert first["romanized"] == first["tanglish"] == "kaalai thendral vandhadhu"
    assert [x["romanized"] for x in first["words"]] == ["kaalai", "thendral", "vandhadhu"]
    assert first["confidence"] == pytest.approx(0.867, abs=0.001)
    assert doc["language"] == "ta" and doc["language_source"] == "whisper" and doc["languages"] == ["ta", "en"]
    assert doc["whisper"]["language_probability"] == 0.83
    # English translation of the Tamil lines only, labelled as machine translation
    assert first["translation"] == "EN(tam_Taml):காலை தென்றல் வந்தது"
    assert doc["lines"][2]["translation"] is None and doc["lines"][2]["romanized"] is None
    t = doc["sources"]["translation"]
    assert t["kind"] == "translator" and t["machine_translation"] is True and "machine translation" in t["name"]
    assert doc["sources"]["transliteration"]["library"].startswith("aksharamukha")
    assert res.translated == 2

    # forced rebuild: Whisper and the translator come from the cache
    lyrics.run_lyrics(d, opts(tmp_path, online=False, force=True))
    assert len(fake.calls) == 1 and len(translate.run_translation.calls) == 1
    # nothing changed: up to date
    assert lyrics.run_lyrics(d, opts(tmp_path, online=False)).skipped


def test_light_skips_every_model(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    with_vocals(d)
    fake = FakeWhisper(TAMIL_SEGMENTS)
    monkeypatch.setattr(asr, "run_whisper", fake)
    res = lyrics.run_lyrics(d, opts(tmp_path, online=False, models=False))
    assert res.status == "not_found" and not fake.calls
    assert any("--light" in n for n in res.notes)
    doc = read(d)
    assert doc["analyzer"]["models"] is False and doc["auto_generated"]["words"] is False
    # turning models on later rebuilds (different fingerprint)
    assert lyrics.run_lyrics(d, opts(tmp_path, online=False)).status == "synced"


def test_light_keeps_local_lyrics_untranslated(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    audio = Path(json.loads((d / "song.json").read_text())["source"]["path"])
    shutil.copy(DATA / "sample_ta.lrc", audio.with_suffix(".lrc"))
    res = lyrics.run_lyrics(d, opts(tmp_path, online=False, models=False))
    doc = read(d)
    assert res.status == "synced" and all(l["translation"] is None for l in doc["lines"])
    assert doc["sources"]["translation"] is None and doc["lines"][0]["romanized"] == "kaalai thendral vandhadhu"
    assert not translate.run_translation.calls


def test_language_override_goes_to_whisper(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    with_vocals(d)
    fake = FakeWhisper([seg(1.0, 4.0, [("నువ్వు", 1.0, 1.6, 0.9), ("నా", 1.6, 2.0, 0.9), ("ప్రాణం", 2.0, 3.0, 0.9)])])
    monkeypatch.setattr(asr, "run_whisper", fake)
    lyrics.run_lyrics(d, opts(tmp_path, online=False, language="te"))
    doc = read(d)
    assert fake.calls[0]["language"] == "te"
    assert doc["language"] == "te" and doc["language_source"] == "user"
    line = doc["lines"][0]
    assert line["language"] == "te" and line["romanized"] == "nuvvu naa praanam" and line["tanglish"] is None
    assert line["translation"] == "EN(tel_Telu):నువ్వు నా ప్రాణం"
    # Tanglish override: Whisper still listens for Tamil
    lyrics.run_lyrics(d, opts(tmp_path, online=False, language="ta-Latn", force=True))
    assert fake.calls[-1]["language"] == "ta"


def test_mixed_hindi_english_song(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    with_vocals(d)
    fake = FakeWhisper([
        seg(1.0, 3.0, [("तुम", 1.0, 1.4, 0.9), ("ही", 1.4, 1.8, 0.9), ("हो", 1.8, 2.6, 0.9)]),
        seg(4.0, 6.0, [("I", 4.0, 4.2, 0.9), ("need", 4.2, 4.6, 0.9), ("you", 4.6, 5.5, 0.9)]),
        seg(7.0, 9.0, [("मैं", 7.0, 7.3, 0.9), ("तेरा", 7.3, 7.8, 0.9), ("प्यार", 7.8, 8.4, 0.9),
                       ("हूँ", 8.4, 9.0, 0.9)]),
    ], language="hi")
    monkeypatch.setattr(asr, "run_whisper", fake)
    lyrics.run_lyrics(d, opts(tmp_path, online=False))
    doc = read(d)
    validate(doc)
    assert [l["language"] for l in doc["lines"]] == ["hi", "en", "hi"]
    assert doc["language"] == "hi" and doc["languages"] == ["hi", "en"]
    assert [l["romanized"] for l in doc["lines"]] == ["tum hi ho", None, "main tera pyaar hoon"]
    assert [l["translation"] for l in doc["lines"]] == ["EN(hin_Deva):तुम ही हो", None,
                                                         "EN(hin_Deva):मैं तेरा प्यार हूँ"]


def test_whisper_failure_is_retried_next_time(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    with_vocals(d)

    def broken(*a, **kw):
        raise asr.AsrUnavailable("Whisper large-v3-turbo could not be loaded: no weights (offline)")
    monkeypatch.setattr(asr, "run_whisper", broken)
    res = lyrics.run_lyrics(d, opts(tmp_path, online=False))
    assert res.status == "not_found" and any("could not be loaded" in n for n in res.notes)
    assert read(d)["analyzer"]["incomplete"] is True
    monkeypatch.setattr(asr, "run_whisper", FakeWhisper(TAMIL_SEGMENTS))
    res = lyrics.run_lyrics(d, opts(tmp_path, online=False))
    assert not res.skipped and res.status == "synced"


def test_no_vocal_stem_uses_the_mix(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    fake = FakeWhisper(TAMIL_SEGMENTS)
    monkeypatch.setattr(asr, "run_whisper", fake)
    res = lyrics.run_lyrics(d, opts(tmp_path, online=False))
    audio = Path(json.loads((d / "song.json").read_text())["source"]["path"])
    assert fake.calls[0]["audio"] == audio and any("whole mix" in n for n in res.notes)


def test_instrumental_is_not_transcribed(tmp_path, monkeypatch):
    d = make_song(tmp_path, artist="Qwertone Test Band")
    with_vocals(d)
    monkeypatch.setattr(lrclib, "http_get", FakeLrclib([record(instrumental=True)]))
    fake = FakeWhisper(TAMIL_SEGMENTS)
    monkeypatch.setattr(asr, "run_whisper", fake)
    assert lyrics.run_lyrics(d, opts(tmp_path)).status == "instrumental" and not fake.calls


# ============================================================================ Whisper details

def test_hallucination_filter_and_line_splitting():
    loop = [seg(10.0 + i, 11.0 + i, [("na", 10.0 + i, 10.5 + i, 0.5)]) for i in range(5)]
    res = {"segments": [
        seg(0, 1, [("♪", 0, 1, 0.1)]),
        seg(1, 2, [("la", 1, 2, 0.2)], compression_ratio=3.1),
        seg(2, 3, [("hmm", 2, 3, 0.1)], no_speech_prob=0.9, avg_logprob=-1.5),
        *loop,
        seg(20, 30, [(f"word{i}", 20 + i * 0.5, 20.4 + i * 0.5, 0.9) for i in range(14)]),
    ]}
    doc, conf = asr.to_doc(res)
    texts = [l.text for l in doc.lines]
    assert texts[:3] == ["na", "na", "na"]                       # 4th and 5th repeat dropped
    long_lines = texts[3:]
    assert len(long_lines) == 2 and all(len(t) <= asr.MAX_LINE_CHARS for t in long_lines)
    assert " ".join(long_lines) == " ".join(f"word{i}" for i in range(14))
    assert len(conf) == len(doc.lines)


class _Seg:
    def __init__(self, d):
        self.__dict__.update(d)
        self.words = [type("W", (), w)() for w in d["words"]]


class FakeModel:
    def __init__(self, probs):
        self.probs = probs
        self.kw = None

    def detect_language(self, **kw):
        return self.probs[0][0], self.probs[0][1], self.probs

    def transcribe(self, audio, **kw):
        self.kw = kw
        return iter([_Seg(seg(1.0, 2.0, [("x", 1.0, 2.0, 0.9)]))]), None


@pytest.mark.parametrize("probs,lang,multi", [
    ([("ur", 0.6), ("hi", 0.3)], "hi", False),        # sung Hindi heard as Urdu -> Hindi
    ([("hi", 0.55), ("en", 0.35)], "hi", True),       # Hindi + English -> language per window
    ([("ta", 0.9), ("ml", 0.05)], "ta", False),
])
def test_run_whisper_language_detection(monkeypatch, probs, lang, multi):
    import numpy as np
    pytest.importorskip("faster_whisper")
    import faster_whisper
    m = FakeModel(probs)
    monkeypatch.setattr(asr, "load_model", lambda *a, **k: m)
    monkeypatch.setattr(faster_whisper, "decode_audio", lambda p, sampling_rate: np.zeros(16000 * 3, "float32"))
    res = asr.run_whisper(Path("v.flac"), model="tiny", device="cpu", language=None, prompt=None, offline=True)
    assert res["language"] == lang and res["detected"] == probs[0][0] and res["multilingual"] is multi
    assert m.kw["language"] == lang and m.kw["word_timestamps"] and m.kw["vad_filter"]
    res = asr.run_whisper(Path("v.flac"), model="tiny", device="cpu", language="te", prompt="p", offline=True)
    assert m.kw["language"] == "te" and m.kw["initial_prompt"] == "p" and res["detected"] is None


# ============================================================================ aligning plain lyrics

PLAIN_TA = "காலை தென்றல் வந்தது\nகண்ணில் உன் முகம்\nவா வா என் அன்பே\n"


def heard(*words):
    return [{"text": w, "start": s, "end": e, "probability": 0.9} for w, s, e in words]


def test_align_unit():
    plain = lrc.parse(PLAIN_TA)
    h = heard(("ஆ", 0.5, 1.0), ("ஆ", 1.0, 1.5),                     # an intro the lyrics don't have
              ("காலை", 2.0, 2.6), ("தென்றல்", 2.6, 3.4), ("வந்தது", 3.4, 4.6),
              ("கன்னில்", 5.5, 6.2), ("முகம்", 6.6, 7.4),             # misheard + one word missing
              ("vaa", 9.0, 9.4), ("vaa", 9.4, 9.8), ("en", 9.8, 10.1), ("anbe", 10.1, 11.0))   # heard as Tanglish
    res = align.align(plain, h, "ta", 30.0)
    assert res.doc is not None and res.matched >= 0.9
    starts = [l.start for l in res.doc.lines]
    assert starts == pytest.approx([2.0, 5.5, 9.0])
    second = res.doc.lines[1]
    assert [w.text for w in second.words] == ["கண்ணில்", "உன்", "முகம்"]
    assert 6.2 <= second.words[1].start <= 6.6                     # the missing word sits in the gap
    assert res.confidence[0] > res.confidence[1] > 0


def test_align_gives_up_on_unrelated_audio():
    res = align.align(lrc.parse(PLAIN_TA), heard(("hello", 1, 2), ("world", 2, 3), ("okay", 3, 4)), "ta")
    assert res.doc is None and res.matched < align.MIN_MATCHED


def test_plain_lyrics_are_aligned_to_the_vocals(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    with_vocals(d)
    audio = Path(json.loads((d / "song.json").read_text())["source"]["path"])
    audio.with_suffix(".lrc").write_text(PLAIN_TA)
    fake = FakeWhisper([
        seg(2.0, 7.4, [("காலை", 2.0, 2.6, 0.9), ("தென்றல்", 2.6, 3.4, 0.9), ("வந்தது", 3.4, 4.6, 0.9),
                       ("கண்ணில்", 5.5, 6.2, 0.9), ("உன்", 6.2, 6.6, 0.9), ("முகம்", 6.6, 7.4, 0.9)]),
        seg(9.0, 11.0, [("வா", 9.0, 9.4, 0.9), ("வா", 9.4, 9.8, 0.9), ("என்", 9.8, 10.1, 0.9),
                        ("அன்பே", 10.1, 11.0, 0.9)]),
    ])
    monkeypatch.setattr(asr, "run_whisper", fake)
    res = lyrics.run_lyrics(d, opts(tmp_path, online=False))
    doc = read(d)
    validate(doc)
    assert res.status == "synced"
    assert fake.calls[0]["language"] == "ta" and fake.calls[0]["prompt"].startswith("காலை தென்றல் வந்தது")
    assert doc["sources"]["words"]["kind"] == "file"                         # the words stay the user's
    t = doc["sources"]["timing"]
    assert t["kind"] == "aligner" and t["file"] == "stems/vocals.flac" and t["matched"] == 1.0
    assert doc["auto_generated"] == {"words": False, "timing": True, "translation": True}
    assert [l["start"] for l in doc["lines"]] == [2.0, 5.5, 9.0]
    assert [l["text"] for l in doc["lines"]] == PLAIN_TA.strip().split("\n")
    assert doc["lines"][0]["words"][2]["start"] == 3.4 and doc["lines"][0]["confidence"] == 1.0


def test_alignment_failure_keeps_plain(tmp_path, monkeypatch):
    d = make_song(tmp_path)
    with_vocals(d)
    audio = Path(json.loads((d / "song.json").read_text())["source"]["path"])
    audio.with_suffix(".lrc").write_text(PLAIN_TA)
    monkeypatch.setattr(asr, "run_whisper", FakeWhisper([seg(1, 3, [("totally", 1, 2, 0.9), ("else", 2, 3, 0.9)])]))
    res = lyrics.run_lyrics(d, opts(tmp_path, online=False))
    assert res.status == "plain" and any("alignment failed" in n for n in res.notes)
    doc = read(d)
    assert doc["sources"]["timing"] is None and doc["lines"][0]["translation"].startswith("EN(tam_Taml)")


# ============================================================================ AcoustID

ACOUSTID_ANSWER = {"status": "ok", "results": [
    {"id": "aid-low", "score": 0.3, "recordings": [{"id": "r0", "title": "Wrong", "artists": [{"name": "X"}]}]},
    {"id": "aid-1", "score": 0.94, "recordings": [
        {"id": "r1", "title": "Kaalai Thendral", "duration": 30,
         "artists": [{"name": "Qwertone Test Band"}], "releasegroups": [{"title": "Test Songs", "type": "Album"}]},
        {"id": "r2", "title": "Kaalai Thendral (Remix)", "duration": 212,
         "artists": [{"name": "Someone", "joinphrase": " & "}, {"name": "Else"}]}]}]}


def test_acoustid_identifies_untagged_file(tmp_path, monkeypatch):
    d = make_song(tmp_path, name="track01.mp3")          # no artist, title = file name
    posts = []

    def post(url, data, timeout=10.0):
        posts.append(data)
        return 200, json.dumps(ACOUSTID_ANSWER).encode()
    monkeypatch.setattr(acoustid, "http_post", post)
    monkeypatch.setattr(acoustid, "have_fpcalc", lambda: True)
    monkeypatch.setattr(acoustid, "fingerprint", lambda p: (30.2, "AQAAfake"))
    synced = (DATA / "sample_ta.lrc").read_text()
    monkeypatch.setattr(lrclib, "http_get", FakeLrclib([record(synced=synced)]))
    res = lyrics.run_lyrics(d, opts(tmp_path, acoustid_key="k3y", models=False))
    doc = read(d)
    validate(doc)
    assert posts[0]["client"] == "k3y" and posts[0]["duration"] == "30" and posts[0]["fingerprint"] == "AQAAfake"
    assert res.status == "synced" and doc["song"]["title"] == "Kaalai Thendral"
    ib = doc["song"]["identified_by"]
    assert ib["kind"] == "acoustid" and ib["artist"] == "Qwertone Test Band" and ib["album"] == "Test Songs"
    assert ib["url"] == "https://acoustid.org/track/aid-1"
    # cached: no second request, even offline
    lyrics.run_lyrics(d, opts(tmp_path, acoustid_key="k3y", models=False, force=True))
    lyrics.run_lyrics(d, opts(tmp_path, acoustid_key="k3y", models=False, force=True, online=False))
    assert len(posts) == 1


def test_acoustid_only_for_untagged_files_and_with_a_key(tmp_path, monkeypatch):
    monkeypatch.setattr(acoustid, "have_fpcalc", lambda: True)
    monkeypatch.setattr(acoustid, "fingerprint", lambda p: (30.0, "AQAA"))
    tagged = make_song(tmp_path, name="01.mp3", title="Kaalai Thendral", artist="Qwertone Test Band")
    lyrics.run_lyrics(tagged, opts(tmp_path, acoustid_key="k3y", models=False))   # _no_post would fail
    untagged = make_song(tmp_path / "b", name="track02.mp3")
    lyrics.run_lyrics(untagged, opts(tmp_path, models=False))                     # no key: skipped quietly
    res = lyrics.run_lyrics(untagged, opts(tmp_path, models=False, acoustid="always", force=True))
    assert any("API key" in n for n in res.notes)
    (tmp_path / "config" / "qwertone").mkdir(parents=True)
    acoustid.key_file().write_text("from-file\n")
    assert acoustid.api_key() == "from-file"
    monkeypatch.setenv("ACOUSTID_API_KEY", "from-env")
    assert acoustid.api_key() == "from-env" and acoustid.api_key("given") == "given"


def test_acoustid_best_match():
    m = acoustid.best_match(ACOUSTID_ANSWER["results"], 211.0)
    assert m["title"] == "Kaalai Thendral (Remix)" and m["artist"] == "Someone & Else"   # duration decides
    assert acoustid.best_match([{"score": 0.2, "recordings": [{"title": "x", "artists": [{"name": "y"}]}]}], 1) is None


# ============================================================================ command line

def test_cli_light_and_language(tmp_path, monkeypatch, capsys):
    d = make_song(tmp_path)
    with_vocals(d)
    fake = FakeWhisper(TAMIL_SEGMENTS)
    monkeypatch.setattr(asr, "run_whisper", fake)
    cache = str(tmp_path / "cache")
    assert cli.main(["lyrics", str(d), "--light", "--offline", "--cache-dir", cache, "-q"]) == 0
    assert "not_found" in capsys.readouterr().out and not fake.calls
    assert cli.main(["lyrics", str(d), "--offline", "--language", "ta", "--whisper-model", "small",
                     "--cache-dir", cache, "--show", "-q"]) == 0
    out = capsys.readouterr().out
    assert "synced, 3 lines, ta" in out and "kaalai thendral vandhadhu" in out and "= EN(tam_Taml)" in out
    assert "machine translation" in out and "auto-generated" in out
    assert fake.calls[0]["model"] == "small" and fake.calls[0]["language"] == "ta"


def test_analyze_lyrics_options_reach_the_lyrics_step(tmp_path, monkeypatch):
    seen = {}
    monkeypatch.setattr(lyrics, "run_lyrics", lambda d, o: seen.setdefault("o", o))
    o = pipeline.Options(lyrics_models=False, lyrics_language="hi", whisper_model="medium", translate=False,
                         acoustid="never", device="cpu")
    pipeline._lyrics_step(tmp_path, o)
    lo = seen["o"]
    assert (lo.models, lo.language, lo.whisper_model, lo.translate, lo.acoustid, lo.device) == \
        (False, "hi", "medium", False, "never", "cpu")
    # none of them change song.json's fingerprint
    assert o.fingerprint() == pipeline.Options(device="cpu").fingerprint()


# ============================================================================ real models (opt-in)

real = pytest.mark.skipif(os.environ.get("QWERTONE_TEST_LYRICS_MODELS") != "1",
                          reason="set QWERTONE_TEST_LYRICS_MODELS=1 to download and run the real models")


@real
@pytest.mark.lyrics_models
def test_real_translation(monkeypatch):
    monkeypatch.undo()
    out = translate.run_translation(["நன்றி", "धन्यवाद", "ధన్యవాదాలు"], ["tam_Taml", "hin_Deva", "tel_Telu"],
                                    model=translate.DEFAULT_MODEL, device="cpu", offline=False)
    assert all("thank" in o.lower() for o in out), out


@real
@pytest.mark.lyrics_models
def test_real_whisper_loads(monkeypatch):
    monkeypatch.undo()
    assert asr.load_model("tiny", "cpu", False) is not None
