"""Reading .lrc lyric files (and LRC-formatted text from tags or LRCLIB).

Handles:
- line timestamps `[mm:ss.xx]`, `[mm:ss:xx]`, `[mm:ss]`, several per line
  (a repeated chorus: the line is used once per timestamp);
- word timestamps from "enhanced" LRC: `[00:12.00]<00:12.00>Hello <00:12.40>world <00:13.10>`;
- ID tags `[ti:] [ar:] [al:] [by:] [length:] [la:]/[lang:]` and `[offset:+/-ms]`
  (positive offset = lyrics come earlier, as in the LRC spec; it is applied
  here, so returned times are final);
- timestamped empty lines: they end the previous line (instrumental gap);
- plain text with no timestamps at all (unsynced lyrics).
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

_TS = r"(\d{1,3}):(\d{1,2}(?:[.:]\d{1,3})?)"
LINE_TS = re.compile(r"\[" + _TS + r"\]")
WORD_TS = re.compile(r"<" + _TS + r">")
TAG = re.compile(r"^\[([a-zA-Z#]+)\s*:(.*)\]\s*$")


@dataclass
class Word:
    text: str
    start: float
    end: float | None = None


@dataclass
class Line:
    text: str
    start: float | None = None          # None for unsynced lyrics
    words: list[Word] | None = None
    end: float | None = None            # only set when the file says (gap marker / trailing word stamp)


@dataclass
class LrcDoc:
    lines: list[Line] = field(default_factory=list)
    tags: dict[str, str] = field(default_factory=dict)
    offset: float = 0.0                 # seconds that were subtracted from every stamp
    synced: bool = False
    has_words: bool = False

    @property
    def language(self) -> str | None:
        return self.tags.get("la") or self.tags.get("lang") or self.tags.get("language")


def _seconds(mm: str, ss: str) -> float:
    ss = ss.replace(":", ".")
    return int(mm) * 60 + float(ss)


def _parse_words(body: str, offset: float) -> tuple[str, list[Word] | None, float | None]:
    """Split `<ts>word <ts>word <ts>` into words. Returns (plain text, words, trailing end)."""
    marks = list(WORD_TS.finditer(body))
    if not marks:
        return body.strip(), None, None
    words: list[Word] = []
    lead = body[: marks[0].start()].strip()
    pieces = []
    for i, m in enumerate(marks):
        t = _seconds(m.group(1), m.group(2)) - offset
        nxt = marks[i + 1].start() if i + 1 < len(marks) else len(body)
        pieces.append((t, body[m.end(): nxt]))
    trailing_end = None
    for t, txt in pieces:
        if txt.strip():
            words.append(Word(txt.strip(), round(t, 3)))
        elif words and words[-1].end is None:
            words[-1].end = round(t, 3)
            trailing_end = round(t, 3)
    for a, b in zip(words, words[1:]):
        if a.end is None:
            a.end = b.start
    if lead and words:
        words[0].text = f"{lead} {words[0].text}"
    text = " ".join(w.text for w in words) if words else lead
    if words and words[-1].end is not None:
        trailing_end = words[-1].end
    return text, (words or None), trailing_end


def parse(text: str) -> LrcDoc:
    doc = LrcDoc()
    raw_lines = text.lstrip("﻿").replace("\r\n", "\n").replace("\r", "\n").split("\n")

    # tags first: the offset applies to every stamp in the file
    for raw in raw_lines:
        m = TAG.match(raw.strip())
        if m and not LINE_TS.match(raw.strip()):
            key, val = m.group(1).lower(), m.group(2).strip()
            doc.tags[key] = val
    try:
        doc.offset = int(doc.tags.get("offset", "0").replace("+", "")) / 1000.0
    except ValueError:
        doc.offset = 0.0

    timed: list[tuple[float, int, Line | None]] = []   # (time, order, line or None for a gap)
    plain: list[Line] = []
    for order, raw in enumerate(raw_lines):
        s = raw.strip()
        if not s:
            continue
        stamps = []
        pos = 0
        while True:
            m = LINE_TS.match(s, pos)
            if not m:
                break
            stamps.append(_seconds(m.group(1), m.group(2)) - doc.offset)
            pos = m.end()
        body = s[pos:]
        if not stamps:
            if TAG.match(s):
                continue
            plain.append(Line(_parse_words(body, doc.offset)[0]))
            continue
        txt, words, trailing = _parse_words(body, doc.offset)
        for t in stamps:
            if not txt:
                timed.append((t, order, None))
                continue
            ws = None
            if words is not None:
                shift = t - stamps[0]  # a repeated chorus keeps its word rhythm
                ws = [Word(w.text, round(w.start + shift, 3), None if w.end is None else round(w.end + shift, 3))
                      for w in words]
            end = None if trailing is None else round(trailing + (t - stamps[0]), 3)
            timed.append((t, order, Line(txt, round(max(t, 0.0), 3), ws, end)))

    if timed:
        doc.synced = True
        timed.sort(key=lambda x: (x[0], x[1]))
        for i, (t, _, line) in enumerate(timed):
            if line is None:
                # a gap marker: ends the line before it, if that line has no end yet
                prev = next((l for _, _, l in reversed(timed[:i]) if l is not None), None)
                if prev is not None and prev.end is None and t > prev.start:
                    prev.end = round(t, 3)
                continue
            doc.lines.append(line)
        doc.has_words = any(l.words for l in doc.lines)
    else:
        doc.lines = [l for l in plain if l.text]
    return doc


def looks_synced(text: str) -> bool:
    return any(LINE_TS.match(l.strip()) for l in text.splitlines())
