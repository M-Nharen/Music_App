"""Indian scripts -> Latin letters, the way people type them in chats and
lyric videos: Tanglish (`kaadhal`, `nenjam`), Tenglish (`nuvvu naa praanam`),
Hinglish (`tum hi ho`, `main tera pyaar hoon`).

One library does every script: Aksharamukha (`RomanReadable` target) for
Tamil, Telugu, Devanagari (Hindi), Kannada, Malayalam, Bengali, Gujarati,
Gurmukhi and Odia. On top of it a few tidy rules per language make the
result look like what people actually write:

- all: Aksharamukha's `'` marks (retroflex / short e, o) are dropped;
- Tamil: its `TamilTranscribe` option picks the sound of க/ச/ட/த/ப/ற
  from the letters around it (kaadhal, endru, anbu); `chch` -> `cch`,
  `thth` -> `tth`, ஃப -> f, ஸ்ரீ -> sri;
- Hindi (Devanagari): its `RemoveSchwaHindi` option drops the silent a
  (dil, not dila); nasal `m'` -> n (main, hoon, hain); a word-final long
  ee / aa is written short (kabhi, tera), as in Hinglish;
- Telugu, Kannada, Malayalam ...: just the marks.

Only runs of an Indic script are converted; Latin, digits and punctuation
are kept, so mixed lines ("ஹே baby நீ") work. Without Aksharamukha
installed, `romanize()` returns None.
"""

from __future__ import annotations

import re
import unicodedata
import warnings
from functools import lru_cache

from .languages import SCRIPTS, base, script_of

VERSION = 2          # 1 was the Tamil-only rule table (tanglish.py)
LIBRARY = "aksharamukha"

PRE_OPTIONS = {"Tamil": ["TamilTranscribe"], "Devanagari": ["RemoveSchwaHindi"]}
DIGITS = {chr(z + i): str(i) for z in (0x0BE6, 0x0C66, 0x0966, 0x0CE6, 0x0D66, 0x09E6, 0x0AE6, 0x0A66, 0x0B66)
          for i in range(10)}
_INDIC_RUN = re.compile("[" + "".join(f"{lo}-{hi}" for lo, hi, _ in SCRIPTS.values()) + "‌‍]+")


@lru_cache(maxsize=1)
def _engine():
    try:
        with warnings.catch_warnings():          # it has old-style regex escapes (SyntaxWarning on 3.12+)
            warnings.simplefilter("ignore")
            from aksharamukha import transliterate
        return transliterate
    except ImportError:
        return None


def available() -> bool:
    return _engine() is not None


def library_version() -> str | None:
    try:
        from importlib.metadata import version
        return version("aksharamukha")
    except Exception:  # noqa: BLE001
        return None


def _tidy(word: str, script: str, lang: str | None) -> str:
    w = word
    if script == "Tamil":
        w = w.replace("shree", "sri").replace("chch", "cch").replace("thth", "tth").replace("hp", "f")
        w = re.sub(r"^ny", "gn", w)                   # ஞானம் -> gnaanam
    if script == "Devanagari" and base(lang) in (None, "hi", "ur"):
        w = re.sub(r"m'(?=[pbm])", "m", w)
        w = w.replace("m'", "n")                      # anusvara / chandrabindu: main, hoon, hain
        w = re.sub(r"ee$", "i", w)                    # kabhi, zindagi
        w = re.sub(r"(?<=\w\w)aa$", "a", w)           # tera, mera (but: aa, jaa stay)
    w = w.replace("'", "")
    return w


@lru_cache(maxsize=20000)
def _word(word: str, script: str, lang: str | None) -> str:
    eng = _engine()
    with warnings.catch_warnings():
        warnings.simplefilter("ignore")
        out = eng.process(script, "RomanReadable", word, pre_options=PRE_OPTIONS.get(script, []))
    return _tidy(out, script, lang)


def _run(run: str, lang: str | None) -> str:
    out = []
    for piece in re.split(r"([‌‍]+)", run):
        if not piece or piece[0] in "‌‍":
            continue
        if all(c in DIGITS for c in piece):
            out.append("".join(DIGITS[c] for c in piece))
            continue
        counts: dict[str, int] = {}
        for c in piece:
            s = script_of(c)
            if s:
                counts[s] = counts.get(s, 0) + 1
        script = max(counts, key=counts.get)
        out.append(_word(piece, script, lang))
    return "".join(out)


def romanize(text: str, lang: str | None = None) -> str | None:
    """Latin letters for every Indic-script run in `text`; other text kept. None when the
    library is missing. `lang` (the line language) picks the tidy rules (hi vs mr ...)."""
    if _engine() is None:
        return None
    text = unicodedata.normalize("NFC", text)
    out, pos = [], 0
    for m in _INDIC_RUN.finditer(text):
        out.append(text[pos:m.start()])
        out.append(_run(m.group(), lang))
        pos = m.end()
    out.append(text[pos:])
    return "".join(out)


def can_romanize(text: str) -> bool:
    return bool(_INDIC_RUN.search(text)) and available()


def ascii_key(text: str, lang: str | None = None) -> str:
    """A rough sound-alike key for matching sung words to lyric words across spellings and
    scripts (used by the aligner): romanised, lowercase a-z only, voicing and length dropped."""
    r = romanize(text, lang) if _INDIC_RUN.search(text) else text
    r = unicodedata.normalize("NFKD", r or text).lower()
    r = "".join(c for c in r if "a" <= c <= "z")
    r = re.sub(r"(.)\1+", r"\1", r)
    for a, b in (("zh", "l"), ("sh", "s"), ("ch", "s"), ("th", "t"), ("dh", "t"), ("kh", "k"), ("gh", "k"),
                 ("ph", "f"), ("bh", "b"), ("w", "v"), ("z", "s"), ("j", "s"), ("q", "k"), ("x", "ks"), ("ee", "i"),
                 ("oo", "u"), ("y", "i")):
        r = r.replace(a, b)
    r = r.translate(str.maketrans("gdbc", "ktpk"))
    r = re.sub(r"(.)\1+", r"\1", r)           # doubled letters / long vowels
    r = r.replace("h", "")
    return r
