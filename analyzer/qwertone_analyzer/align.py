"""Timing for plain (unsynced) lyrics, from what Whisper heard on the vocal stem.

Whisper transcribes the vocal stem with word timestamps (asr.py; the first
lyric lines are given as its prompt so it leans towards the real words).
The lyric words and the heard words are then lined up with a global
sequence alignment (Needleman-Wunsch, free gaps at both ends so intros,
outros and extra chorus repeats cost nothing). Words are compared by a
sound-alike key (romanize.ascii_key): both sides romanised with the same
library, lowercased, voicing/length/aspiration dropped, so Tamil script
lyrics match a Tanglish-ish transcript and small mishearings still match.

Matched words give exact times; the words between them are spread evenly.
A line's confidence is the share of its words that matched times how well
they matched. If under 30 % of all words match, the alignment is thrown
away (wrong song, or Whisper heard nothing useful) and the lyrics stay plain.

This doesn't need any model besides Whisper, so it works on the CPU in the
same pass. It is not a phoneme-level forced aligner: lines Whisper hears
badly get interpolated times (and a low confidence), which the app can show
and the user can fix with tap-along timing.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from . import lrc
from .romanize import ascii_key

VERSION = 1
MATCH_MIN = 0.5            # bigram similarity for two words to count as the same
GAP = 0.2                  # cost of skipping a word on either side
MIN_MATCHED = 0.3          # share of lyric words that must match, else give up
SECONDS_PER_WORD = 0.4     # outside the matched range


@dataclass
class AlignResult:
    doc: lrc.LrcDoc | None                     # synced copy of the lyrics, None when it failed
    matched: float                             # share of lyric words matched
    confidence: list[float] = field(default_factory=list)
    word_timing: list[bool] = field(default_factory=list)   # per line: enough matches to keep word times


def _bigrams(s: str) -> frozenset:
    s = f"^{s}$"
    return frozenset(s[i:i + 2] for i in range(len(s) - 1))


def _sim(a: frozenset, b: frozenset) -> float:
    if not a or not b:
        return 0.0
    return 2 * len(a & b) / (len(a) + len(b))


def _nw(lyr: list[frozenset], heard: list[frozenset]) -> list[tuple[int, int, float]]:
    """Semi-global alignment. Returns matched (lyric index, heard index, similarity)."""
    n, m = len(lyr), len(heard)
    score = [[0.0] * (m + 1) for _ in range(n + 1)]
    move = [[0] * (m + 1) for _ in range(n + 1)]       # 1 diag, 2 up (skip lyric word), 3 left (skip heard)
    for i in range(1, n + 1):
        score[i][0] = score[i - 1][0] - GAP
        move[i][0] = 2
    for i in range(1, n + 1):
        li = lyr[i - 1]
        row, prev = score[i], score[i - 1]
        mrow = move[i]
        for j in range(1, m + 1):
            s = _sim(li, heard[j - 1])
            best, mv = prev[j - 1] + (s - MATCH_MIN), 1
            up = prev[j] - GAP
            if up > best:
                best, mv = up, 2
            left = row[j - 1] - (GAP if i < n else 0.0)   # free trailing heard words
            if left > best:
                best, mv = left, 3
            row[j] = best
            mrow[j] = mv
    j = max(range(m + 1), key=lambda k: score[n][k]) if m else 0
    i, pairs = n, []
    while i > 0 and j > 0:
        mv = move[i][j]
        if mv == 1:
            s = _sim(lyr[i - 1], heard[j - 1])
            if s >= MATCH_MIN:
                pairs.append((i - 1, j - 1, s))
            i, j = i - 1, j - 1
        elif mv == 2:
            i -= 1
        else:
            j -= 1
    return pairs[::-1]


def align(plain: lrc.LrcDoc, heard: list[dict], lang: str | None = None,
          duration: float | None = None) -> AlignResult:
    """`heard` = asr.words(...): [{"text", "start", "end", "probability"}]."""
    lines = plain.lines
    words: list[tuple[int, str]] = []          # (line index, word)
    for li, ln in enumerate(lines):
        for w in ln.text.split():
            if ascii_key(w, lang):
                words.append((li, w))
    if not words or not heard:
        return AlignResult(None, 0.0)
    lk = [_bigrams(ascii_key(w, lang)) for _, w in words]
    hk = [_bigrams(ascii_key(h["text"], lang)) for h in heard]
    pairs = _nw(lk, hk)
    matched = len(pairs) / len(words)
    if matched < MIN_MATCHED:
        return AlignResult(None, round(matched, 3))

    n = len(words)
    start: list[float | None] = [None] * n
    end: list[float | None] = [None] * n
    sim = [0.0] * n
    for wi, hi, s in pairs:
        start[wi], end[wi], sim[wi] = heard[hi]["start"], heard[hi]["end"], s
    anchors = [k for k in range(n) if start[k] is not None]
    # spread the unmatched words between anchors (by word count), extrapolate at the ends
    for k in range(n):
        if start[k] is not None:
            continue
        before = max((a for a in anchors if a < k), default=None)
        after = min((a for a in anchors if a > k), default=None)
        if before is not None and after is not None:
            t0, t1 = end[before], start[after]
            frac = (k - before) / (after - before)
            start[k] = t0 + (t1 - t0) * frac if t1 >= t0 else t0
        elif before is not None:
            start[k] = end[before] + SECONDS_PER_WORD * (k - before - 1)
        else:
            start[k] = max(0.0, start[after] - SECONDS_PER_WORD * (after - k))
    for k in range(n):
        if end[k] is None:
            nxt = start[k + 1] if k + 1 < n else start[k] + SECONDS_PER_WORD
            end[k] = max(start[k], min(start[k] + SECONDS_PER_WORD, nxt))
    if duration:
        start = [min(t, duration) for t in start]
        end = [min(t, duration) for t in end]

    out = lrc.LrcDoc(tags=dict(plain.tags), synced=True, has_words=False)
    conf, keep_words = [], []
    by_line: dict[int, list[int]] = {}
    for k, (li, _) in enumerate(words):
        by_line.setdefault(li, []).append(k)
    prev_start = 0.0
    for li, ln in enumerate(lines):
        ks = by_line.get(li)
        if not ks:
            # no comparable words (just punctuation): sits right after the previous line
            out.lines.append(lrc.Line(ln.text, prev_start, None, None))
            conf.append(0.0)
            keep_words.append(False)
            continue
        s0 = max(start[ks[0]], prev_start)
        e0 = max(end[ks[-1]], s0)
        hits = [sim[k] for k in ks if sim[k] > 0]
        c = (len(hits) / len(ks)) * (sum(hits) / len(hits)) if hits else 0.0
        good = len(hits) / len(ks) >= 0.5
        lw = None
        if good:
            lw = []
            for k in ks:
                ws = max(start[k], s0)
                lw.append(lrc.Word(words[k][1], ws, max(end[k], ws)))
        out.lines.append(lrc.Line(ln.text, s0, lw, e0))
        conf.append(round(c, 3))
        keep_words.append(good)
        prev_start = s0
    out.has_words = any(keep_words)
    return AlignResult(out, round(matched, 3), conf, keep_words)
