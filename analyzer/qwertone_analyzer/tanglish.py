"""Tamil script -> Tanglish (Tamil written the way people type it in chats
and lyric videos: "kaadhal", "nenjam", "vandhaen" -> "vandhen").

Rule-based, no model. Tamil has one letter for k/g, t/d, p/b, ch/s/j... and
how it sounds depends on where it is, so each consonant looks at what comes
before it:

| letter | word start | between vowels / after r, l, y, zh, v | after a nasal (n, m, ng) | doubled |
|---|---|---|---|---|
| க | k | g | g (ங்க = ng) | kk |
| ச | s | s | j (ஞ்ச = nj) | cch |
| ட | d | d | d (ண்ட = nd) | tt |
| த | th | dh | dh (ந்த = ndh) | tth |
| ப | p | p | b (ம்ப = mb) | pp |
| ற | r | r | dr (ன்ற = ndr) | tr (ற்ற) |

Long vowels are doubled (aa, ee, oo); ே/ெ both give "e", ொ/ோ both "o",
independent ஏ gives "ae". Anything that isn't Tamil passes through as is,
so mixed Tamil/English lines work.
"""

from __future__ import annotations

import re
import unicodedata

VERSION = 1

VOWELS = {"அ": "a", "ஆ": "aa", "இ": "i", "ஈ": "ee", "உ": "u", "ஊ": "oo", "எ": "e", "ஏ": "ae",
          "ஐ": "ai", "ஒ": "o", "ஓ": "o", "ஔ": "au"}
SIGNS = {"ா": "aa", "ி": "i", "ீ": "ee", "ு": "u", "ூ": "oo", "ெ": "e",
         "ே": "e", "ை": "ai", "ொ": "o", "ோ": "o", "ௌ": "au", "ௗ": "au"}
PULLI = "்"
AYTHAM = "ஃ"

# plain sound of each consonant (word start / after a hard consonant)
BASE = {"க": "k", "ங": "ng", "ச": "s", "ஜ": "j", "ஞ": "nj", "ட": "d", "ண": "n", "த": "th", "ந": "n",
        "ன": "n", "ப": "p", "ம": "m", "ய": "y", "ர": "r", "ற": "r", "ல": "l", "ள": "l", "ழ": "zh",
        "வ": "v", "ஶ": "sh", "ஷ": "sh", "ஸ": "s", "ஹ": "h"}
SOFT = {"க": "g", "ச": "s", "ட": "d", "த": "dh", "ப": "p", "ற": "r"}           # between vowels, after liquids
NASAL_NEXT = {"க": "g", "ச": "j", "ட": "d", "த": "dh", "ப": "b", "ற": "dr"}    # after a nasal
HARD_NEXT = {"க": "k", "ச": "ch", "ட": "t", "த": "th", "ப": "p", "ற": "r"}     # after k, ch, t, th, p, r
GEMINATE = {"க": "kk", "ச": "cch", "ட": "tt", "த": "tth", "ப": "pp", "ற": "tr"}
NASALS = set("ஙஞணநனம")
LIQUIDS = set("யரலவழள")
HARD = set("கசடதபற")
SPECIAL = {"ஸ்ரீ": "sri"}

TAMIL_LETTER = re.compile(r"[஀-௿]")
DIGITS = {chr(0x0BE6 + i): str(i) for i in range(10)}


def _units(word: str):
    """Split a run of Tamil characters into (kind, consonant/vowel, vowel sound or None)."""
    i, n = 0, len(word)
    while i < n:
        for k, v in SPECIAL.items():
            if word.startswith(k, i):
                yield ("special", k, v)
                i += len(k)
                break
        else:
            c = word[i]
            if c in VOWELS:
                yield ("vowel", c, VOWELS[c])
                i += 1
            elif c in BASE:
                nxt = word[i + 1] if i + 1 < n else ""
                if nxt == PULLI:
                    yield ("cons", c, None)
                    i += 2
                elif nxt in SIGNS:
                    yield ("cons", c, SIGNS[nxt])
                    i += 2
                else:
                    yield ("cons", c, "a")
                    i += 1
            elif c == AYTHAM:
                yield ("aytham", c, None)
                i += 1
            elif c in DIGITS:
                yield ("other", DIGITS[c], None)
                i += 1
            else:  # stray sign, other Tamil symbol
                yield ("other", "" if c in SIGNS or c == PULLI else c, None)
                i += 1


def _word(word: str) -> str:
    units = list(_units(word))
    out = []
    for i, (kind, ch, vowel) in enumerate(units):
        prev = units[i - 1] if i else None
        nxt = units[i + 1] if i + 1 < len(units) else None
        if kind in ("vowel", "special"):
            out.append(vowel)
            continue
        if kind == "other":
            out.append(ch)
            continue
        if kind == "aytham":
            out.append("" if nxt and nxt[1] == "ப" else "h")   # ஃப = f
            continue

        dead = vowel is None                                    # consonant with pulli
        prev_dead = prev is not None and prev[0] == "cons" and prev[2] is None
        if prev_dead and prev[1] == ch:
            sound = ""                                          # second half of a doubled letter
        elif dead and nxt and nxt[0] == "cons" and nxt[1] == ch:
            b = BASE[ch]
            sound = GEMINATE.get(ch) or (b if len(b) > 1 else b * 2)   # kk, tth, nn, ll...
        elif prev and prev[0] == "aytham" and ch == "ப":
            sound = "f"
        elif ch in HARD:
            if dead and nxt is not None:
                sound = HARD_NEXT[ch]                           # கட்சி -> katchi
            elif prev is None or prev[0] == "other":
                sound = BASE[ch]                                # word start
            elif prev[0] == "aytham":
                sound = HARD_NEXT[ch]
            elif prev_dead:
                p = prev[1]
                sound = NASAL_NEXT[ch] if p in NASALS else SOFT[ch] if p in LIQUIDS else HARD_NEXT[ch]
            else:
                sound = SOFT[ch]                                # after a vowel
        elif dead and nxt and nxt[0] == "cons" and ch in NASALS:
            sound = "m" if ch == "ம" else "n"                    # ங்க -> ng, ஞ்ச -> nj, ந்த -> ndh
        elif ch == "ஞ":
            sound = "gn" if prev is None else "nj"
        else:
            sound = BASE[ch]
        if dead and nxt is None and ch in HARD:
            sound = HARD_NEXT[ch]                               # word-final: கார்த்திக் -> ...ik
        out.append(sound + (vowel or ""))
    return "".join(out)


def transliterate(text: str) -> str:
    """Tamil script -> Tanglish. Non-Tamil text is kept."""
    text = unicodedata.normalize("NFC", text)
    out, pos = [], 0
    for m in re.finditer(r"[஀-௿]+", text):
        out.append(text[pos: m.start()])
        out.append(_word(m.group()))
        pos = m.end()
    out.append(text[pos:])
    return "".join(out)


def has_tamil(text: str) -> bool:
    return bool(TAMIL_LETTER.search(text))
