"""Offline English translation of lyric lines (machine translation, labelled so).

Model: Meta's NLLB-200 distilled 600M, converted to CTranslate2 int8
(`JustFrederik/nllb-200-distilled-600M-ct2-int8`, ~650 MB, downloaded on
first use into ~/.cache/qwertone/models/translate). It translates from
~200 languages, so every language Whisper can detect (Tamil, Telugu, Hindi,
Kannada, Malayalam, Bengali ... and the non-Indian ones) goes to English
with one model, on the CPU, no PyTorch. The weights are CC BY-NC 4.0
(fine for personal use, not for selling).

Each line is translated on its own from its own language (a Hindi + English
song: the Hindi lines are translated, the English ones are left alone).
Lines written in Latin letters (Tanglish, Hinglish) have no NLLB code and
are left untranslated. Results are cached per song by model and text.
"""

from __future__ import annotations

import json
import os
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path

from . import backends
from .languages import base, nllb_code
from .progress import log

VERSION = 1
DEFAULT_MODEL = "JustFrederik/nllb-200-distilled-600M-ct2-int8"
MODEL_NAMES = {DEFAULT_MODEL: "NLLB-200 distilled 600M"}
TARGET = "en"
BATCH = 16


class TranslatorUnavailable(RuntimeError):
    pass


@dataclass
class TranslateOptions:
    model: str = DEFAULT_MODEL
    device: str = "auto"
    offline: bool = False
    cache_dir: Path | None = None
    use_cache: bool = True


def available() -> bool:
    return backends.has("ctranslate2") and backends.has("sentencepiece") and backends.has("huggingface_hub")


def translate_dir() -> Path:
    d = backends.models_dir() / "translate"
    d.mkdir(parents=True, exist_ok=True)
    return d


def model_label(model: str) -> str:
    return MODEL_NAMES.get(model, model)


def model_url(model: str) -> str | None:
    if Path(model).exists():
        return None
    return f"https://huggingface.co/{model}"


@lru_cache(maxsize=1)
def load(model: str, device: str = "auto", offline: bool = False):
    """(ctranslate2.Translator, sentencepiece processor)."""
    if not available():
        raise TranslatorUnavailable("ctranslate2 / sentencepiece not installed (run setup.sh without --light)")
    import ctranslate2
    import sentencepiece as spm
    path = Path(model)
    if not path.exists():
        from huggingface_hub import snapshot_download
        log("lyrics", f"loading translator {model_label(model)}" + ("" if offline else " (~650 MB download, once)"))
        try:
            path = Path(snapshot_download(model, cache_dir=str(translate_dir()), local_files_only=offline))
        except Exception as e:  # noqa: BLE001
            if offline:
                raise TranslatorUnavailable("the translation model isn't downloaded yet and we're offline "
                                            "(run: qwertone-analyze download-models)") from e
            raise TranslatorUnavailable(f"translation model {model} could not be loaded: {e}") from e
    dev = "cuda" if device in ("auto", "cuda") and ctranslate2.get_cuda_device_count() > 0 else "cpu"
    tr = ctranslate2.Translator(str(path), device=dev, compute_type="int8_float16" if dev == "cuda" else "int8")
    sp = spm.SentencePieceProcessor(model_file=str(path / "sentencepiece.bpe.model"))
    return tr, sp


def run_translation(texts: list[str], src_codes: list[str], *, model: str, device: str, offline: bool) -> list[str]:
    """The model call (tests replace this). NLLB codes in, English out."""
    tr, sp = load(model, device, offline)
    out: list[str] = []
    for i in range(0, len(texts), BATCH):
        chunk, codes = texts[i:i + BATCH], src_codes[i:i + BATCH]
        src = [[c] + sp.encode(t, out_type=str) + ["</s>"] for t, c in zip(chunk, codes)]
        res = tr.translate_batch(src, target_prefix=[["eng_Latn"]] * len(src), beam_size=4,
                                 max_decoding_length=200, repetition_penalty=1.2)
        for r in res:
            toks = [t for t in r.hypotheses[0] if t not in ("eng_Latn", "</s>")]
            out.append(sp.decode(toks).strip())
    return out


def needs_translation(lang: str | None) -> bool:
    return bool(lang) and base(lang) != TARGET and nllb_code(lang) is not None


def translate_lines(lines: list[dict], song_sha: str, opts: TranslateOptions) -> int:
    """Fills `translation` of every line that isn't English. Returns how many lines got one."""
    todo = [(l["text"], l["language"]) for l in lines if needs_translation(l["language"]) and l["text"].strip()]
    if not todo:
        return 0
    cpath = None
    cache: dict[str, str] = {}
    if opts.use_cache and opts.cache_dir is not None:
        from .cache import params_hash
        cpath = Path(opts.cache_dir) / song_sha[:20] / \
            f"lyrics-translate-{params_hash({'v': VERSION, 'model': opts.model})}.json"
        try:
            with open(cpath) as f:
                cache = json.load(f)
        except (OSError, json.JSONDecodeError):
            cache = {}
    key = lambda t, l: f"{nllb_code(l)}\t{t}"   # noqa: E731
    missing = list(dict.fromkeys((t, l) for t, l in todo if key(t, l) not in cache))
    if missing:
        if not available():
            raise TranslatorUnavailable("ctranslate2 / sentencepiece not installed (run setup.sh without --light)")
        log("lyrics", f"translating {len(missing)} lines to English ({model_label(opts.model)})")
        res = run_translation([t for t, _ in missing], [nllb_code(l) for _, l in missing],
                              model=opts.model, device=opts.device, offline=opts.offline)
        for (t, l), r in zip(missing, res):
            cache[key(t, l)] = r
        if cpath is not None:
            cpath.parent.mkdir(parents=True, exist_ok=True)
            tmp = cpath.with_suffix(".tmp")
            with open(tmp, "w") as f:
                json.dump(cache, f, ensure_ascii=False)
            os.replace(tmp, cpath)
    n = 0
    for l in lines:
        if needs_translation(l["language"]) and l["text"].strip():
            l["translation"] = cache.get(key(l["text"], l["language"])) or None
            n += l["translation"] is not None
    return n
