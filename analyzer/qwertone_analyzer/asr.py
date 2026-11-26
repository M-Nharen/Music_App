"""Whisper (faster-whisper, CTranslate2) on the vocal stem: lyrics when no timed
lyrics exist, and the word timings the aligner (align.py) matches plain
lyrics against.

- Runs on the CPU (int8) or an NVIDIA GPU (float16), no PyTorch needed.
- Model weights (~1.6 GB for large-v3-turbo) download on first use into
  ~/.cache/qwertone/models/whisper; `--offline` only uses what's there.
- The language is detected per song from the voiced parts (Silero VAD),
  unless `--language` says it. If the two most likely languages are both
  likely (a Hindi + English song), every 30-second window is detected on its
  own. Sung Hindi is often detected as Urdu; that is transcribed as Hindi
  (Devanagari) unless you pass `--language ur`.
- Results are cached per song, keyed by the audio, model, language and prompt.
"""

from __future__ import annotations

import json
import os
import re
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path

from . import backends, lrc
from .cache import params_hash
from .languages import WHISPER_TO_FAMILY
from .progress import log

VERSION = 1
DEFAULT_MODEL = "large-v3-turbo"
MODELS = ("tiny", "base", "small", "medium", "large-v2", "large-v3", "large-v3-turbo", "turbo")
SR = 16000
MIXED_LANGUAGE_PROB = 0.25         # 2nd language at least this likely -> detect per window
MAX_LINE_CHARS = 48                # split Whisper segments into lyric-sized lines
LINE_GAP = 0.6                     # ... and at pauses this long between words
HALLUCINATIONS = re.compile(
    r"(thank(s| you) for watching|subscribe|subtitles? by|amara\.org|transcribed by|♪+$|^\W*$"
    r"|copyright|all rights reserved)", re.I)


class AsrUnavailable(RuntimeError):
    pass


@dataclass
class AsrOptions:
    model: str = DEFAULT_MODEL
    device: str = "auto"
    offline: bool = False              # never download weights
    cache_dir: Path | None = None      # per-song cache root (cache.default_cache_root())
    use_cache: bool = True


def available() -> bool:
    return backends.has("faster_whisper")


def model_name(name: str | None) -> str:
    name = name or "auto"
    if name in ("auto", "turbo"):
        return DEFAULT_MODEL
    return name


def whisper_dir() -> Path:
    d = backends.models_dir() / "whisper"
    d.mkdir(parents=True, exist_ok=True)
    return d


def device_and_type(device: str) -> tuple[str, str]:
    if device in ("auto", "cuda"):
        try:
            import ctranslate2
            if ctranslate2.get_cuda_device_count() > 0:
                return "cuda", "float16"
        except Exception:  # noqa: BLE001
            pass
    return "cpu", "int8"


@lru_cache(maxsize=2)
def load_model(name: str, device: str = "auto", offline: bool = False):
    if not available():
        raise AsrUnavailable("faster-whisper is not installed (run setup.sh without --light)")
    from faster_whisper import WhisperModel
    dev, ctype = device_and_type(device)
    log("lyrics", f"loading Whisper {name} on {dev} ({ctype})" + ("" if offline else "; downloads once if missing"))
    try:
        return WhisperModel(name, device=dev, compute_type=ctype, download_root=str(whisper_dir()),
                            local_files_only=offline)
    except Exception as e:  # noqa: BLE001 - missing weights offline, network errors ...
        if offline:
            raise AsrUnavailable(f"Whisper {name} isn't downloaded yet and we're offline "
                                 "(run: qwertone-analyze download-models)") from e
        raise AsrUnavailable(f"Whisper {name} could not be loaded: {e}") from e


def whisper_language(lang: str | None) -> str | None:
    return WHISPER_TO_FAMILY.get(lang, lang) if lang else lang


def run_whisper(audio_path: Path, *, model: str, device: str, language: str | None, prompt: str | None,
                offline: bool) -> dict:
    """Transcribe one file. Returns plain data (cached as JSON):
    {"language", "language_probability", "detected", "top_languages", "multilingual", "duration",
     "segments": [{"start", "end", "text", "avg_logprob", "no_speech_prob", "compression_ratio",
                   "words": [{"start", "end", "word", "probability"}]}]}"""
    m = load_model(model, device, offline)
    from faster_whisper import decode_audio
    audio = decode_audio(str(audio_path), sampling_rate=SR)
    duration = len(audio) / SR
    detected, prob, top, multilingual = None, None, [], False
    lang = language
    if lang is None:
        detected, prob, probs = m.detect_language(audio=audio, vad_filter=True, language_detection_segments=4,
                                                  language_detection_threshold=0.5)
        top = [(l, round(float(p), 3)) for l, p in (probs or [])[:5]]
        lang = whisper_language(detected)
        others = [l for l, p in top[1:] if p >= MIXED_LANGUAGE_PROB and whisper_language(l) != lang]
        multilingual = bool(others)
        log("lyrics", f"Whisper hears {detected} ({prob:.0%})" + (f", also {others[0]}: per-window languages"
                                                                   if multilingual else ""))
    segments, info = m.transcribe(
        audio, language=lang, multilingual=multilingual, word_timestamps=True, beam_size=5,
        vad_filter=True, vad_parameters={"min_silence_duration_ms": 500},
        condition_on_previous_text=False, initial_prompt=prompt or None,
        hallucination_silence_threshold=2.0)
    out = []
    last = -30.0
    for s in segments:
        if s.end - last >= 30:
            log("lyrics", f"transcribing {s.end:5.0f} / {duration:.0f} s")
            last = s.end
        out.append({"start": round(s.start, 3), "end": round(s.end, 3), "text": s.text.strip(),
                    "avg_logprob": round(s.avg_logprob, 3), "no_speech_prob": round(s.no_speech_prob, 3),
                    "compression_ratio": round(s.compression_ratio, 3),
                    "words": [{"start": round(w.start, 3), "end": round(w.end, 3), "word": w.word,
                               "probability": round(w.probability, 3)} for w in (s.words or [])]})
    return {"language": lang, "language_probability": None if language else (round(float(prob), 3) if prob else None),
            "detected": detected, "top_languages": top, "multilingual": multilingual,
            "duration": round(duration, 3), "segments": out}


def _audio_id(path: Path) -> dict:
    st = path.stat()
    return {"file": path.name, "size": st.st_size, "mtime": int(st.st_mtime)}


def transcribe(audio_path: Path, song_sha: str, opts: AsrOptions, language: str | None = None,
               prompt: str | None = None) -> dict:
    """run_whisper with the per-song cache."""
    model = model_name(opts.model)
    key = {"v": VERSION, "model": model, "language": language, "prompt": prompt, "audio": _audio_id(audio_path)}
    cpath = None
    if opts.use_cache and opts.cache_dir is not None:
        cpath = Path(opts.cache_dir) / song_sha[:20] / f"lyrics-whisper-{params_hash(key)}.json"
        if cpath.exists():
            try:
                with open(cpath) as f:
                    res = json.load(f)
                log("lyrics", f"Whisper result cached ({cpath.name})")
                return res
            except (OSError, json.JSONDecodeError):
                pass
    if not available():
        raise AsrUnavailable("faster-whisper is not installed (run setup.sh without --light)")
    res = run_whisper(audio_path, model=model, device=opts.device, language=language, prompt=prompt,
                      offline=opts.offline)
    res["model"] = model
    if cpath is not None:
        cpath.parent.mkdir(parents=True, exist_ok=True)
        tmp = cpath.with_suffix(".tmp")
        with open(tmp, "w") as f:
            json.dump(res, f, ensure_ascii=False)
        os.replace(tmp, cpath)
    return res


# ============================================================================ Whisper output -> lyric lines

def good_segments(res: dict) -> list[dict]:
    """Drop what Whisper typically makes up over music: silence, loops, credits."""
    out, prev = [], None
    repeats = 0
    for s in res.get("segments", []):
        text = s["text"].strip()
        if not text or HALLUCINATIONS.search(text):
            continue
        if s.get("no_speech_prob", 0) > 0.6 and s.get("avg_logprob", 0) < -1.0:
            continue
        if s.get("compression_ratio", 0) > 2.4:          # "la la la la la ..." loops
            continue
        repeats = repeats + 1 if text == prev else 0
        prev = text
        if repeats >= 3:                                 # the same line over and over = stuck decoder
            continue
        out.append(s)
    return out


def words(res: dict) -> list[dict]:
    """Every kept word: {"text", "start", "end", "probability"} in time order."""
    ws = []
    for s in good_segments(res):
        for w in s.get("words") or []:
            t = w["word"].strip()
            if t:
                ws.append({"text": t, "start": float(w["start"]), "end": max(float(w["end"]), float(w["start"])),
                           "probability": float(w.get("probability", 0.0))})
    ws.sort(key=lambda w: w["start"])
    return ws


def to_doc(res: dict) -> tuple[lrc.LrcDoc, list[float]]:
    """Whisper output as an LrcDoc with word timing (one line per sung phrase),
    and the mean word probability of each line (its confidence)."""
    doc = lrc.LrcDoc(synced=True, has_words=True)
    conf = []
    for s in good_segments(res):
        ws = [w for w in (s.get("words") or []) if w["word"].strip()]
        if not ws:
            doc.lines.append(lrc.Line(s["text"], float(s["start"]), None, float(s["end"])))
            conf.append(round(min(1.0, max(0.0, 1.0 + s.get("avg_logprob", -1.0))), 3))
            continue
        cur: list[dict] = []

        def flush():
            if not cur:
                return
            text = "".join(w["word"] for w in cur).strip()
            text = re.sub(r"\s+", " ", text)
            lw = [lrc.Word(w["word"].strip(), float(w["start"]), float(w["end"])) for w in cur]
            doc.lines.append(lrc.Line(text, lw[0].start, lw, max(lw[-1].end or lw[-1].start, lw[0].start)))
            conf.append(round(sum(w["probability"] for w in cur) / len(cur), 3))
            cur.clear()

        for w in ws:
            if cur:
                gap = w["start"] - cur[-1]["end"]
                length = len("".join(x["word"] for x in cur))
                if gap >= LINE_GAP or length + len(w["word"]) > MAX_LINE_CHARS:
                    flush()
            cur.append(w)
        flush()
    order = sorted(range(len(doc.lines)), key=lambda i: doc.lines[i].start)
    doc.lines = [doc.lines[i] for i in order]
    conf = [conf[i] for i in order]
    return doc, conf
