"""Make a part easy to play on a laptop keyboard.

Steps: drop shaky notes -> quantise to a beat grid -> one note at a time for
the melody -> transpose to a key with few black keys -> fold into a playable
range -> add a simple left-hand accompaniment from the chords -> never more
than `max_keys` keys down at once.

Qwertone's default Piano layout: bottom row Z.. = C3–E4, top row Q.. = C4–G5.
So the melody goes in C4–G5 and the accompaniment in C3–B3.
"""

from __future__ import annotations

from dataclasses import replace

import numpy as np

from . import theory
from .model import Chord, KeyInfo, Note, Part, Timeline

VERSION = 1

DIFFICULTY = {
    #    grid (beats), keys at once, melody range, drop notes below conf, accompaniment
    1: dict(grid=1.0, max_keys=1, melody_range=12, drop_conf=0.6, accomp="none", keep_sevenths=False),
    2: dict(grid=0.5, max_keys=2, melody_range=15, drop_conf=0.5, accomp="root_on_change", keep_sevenths=False),
    3: dict(grid=0.5, max_keys=2, melody_range=19, drop_conf=0.4, accomp="root_each_bar", keep_sevenths=False),
    4: dict(grid=0.25, max_keys=3, melody_range=20, drop_conf=0.3, accomp="root_fifth", keep_sevenths=False),
    5: dict(grid=0.25, max_keys=3, melody_range=20, drop_conf=0.2, accomp="root_third", keep_sevenths=True),
}
MELODY_LO = 60   # C4
MELODY_HI = 79   # G5
ACCOMP_LO = 48   # C3
ACCOMP_HI = 59   # B3


def settings(difficulty: int, max_keys: int | None = None) -> dict:
    d = dict(DIFFICULTY[int(np.clip(difficulty, 1, 5))])
    if max_keys is not None:
        d["max_keys"] = int(np.clip(max_keys, 1, 3))
    return d


# --------------------------------------------------------------------------- steps

def quantize(notes: list[Note], tl: Timeline, grid: float) -> list[Note]:
    out = []
    for n in notes:
        b0 = round(tl.beat_at(n.start) / grid) * grid
        b1 = round(tl.beat_at(n.end) / grid) * grid
        if b1 <= b0:
            b1 = b0 + grid
        out.append(replace(n, start=tl.time_at(b0), end=tl.time_at(b1)))
    # identical (pitch, start) after snapping -> keep the more confident one
    best: dict[tuple[int, float], Note] = {}
    for n in out:
        k = (n.pitch, round(n.start, 4))
        if k not in best or n.confidence > best[k].confidence:
            best[k] = n
    return sorted(best.values(), key=lambda n: (n.start, -n.pitch))


def monophonic(notes: list[Note], onset_tol: float = 0.05, min_dur: float = 0.05,
               merge_gap: float | None = None) -> list[Note]:
    """Skyline: among notes starting within `onset_tol` of each other keep the
    highest; cut overlaps; drop slivers. With `merge_gap`, same-pitch fragments
    closer than that are joined (for split notes from a transcriber)."""
    seq: list[Note] = []
    for n in sorted(notes, key=lambda n: n.start):
        if seq and n.start - seq[-1].start < onset_tol:
            if n.pitch > seq[-1].pitch:
                seq[-1] = replace(n, start=seq[-1].start)
            continue
        seq.append(n)
    out: list[Note] = []
    for i, n in enumerate(seq):
        end = min(n.end, seq[i + 1].start) if i + 1 < len(seq) else n.end
        if end - n.start < min_dur:
            continue
        if merge_gap is not None and out and out[-1].pitch == n.pitch and n.start - out[-1].end < merge_gap:
            prev = out[-1]
            out[-1] = replace(prev, end=end, confidence=max(prev.confidence, n.confidence))
            continue
        out.append(replace(n, end=end))
    return out


def black_key_cost(notes: list[Note], chords: list[Chord], shift: int) -> float:
    total = black = 0.0
    for n in notes:
        w = n.dur
        total += w
        black += w * theory.is_black(n.pitch + shift)
    for c in chords:
        if c.root is None:
            continue
        w = c.end - c.start
        for pc in theory.chord_pcs(c.root, c.quality)[:3]:
            total += 0.5 * w
            black += 0.5 * w * theory.is_black(pc + shift)
    return black / total if total else 0.0


def choose_transpose(notes: list[Note], chords: list[Chord]) -> int:
    """Shift in -6..+5 semitones that leaves the fewest black keys (ties -> smaller shift)."""
    best = min(range(-6, 6), key=lambda s: (round(black_key_cost(notes, chords, s), 3) + 0.004 * abs(s), abs(s)))
    return int(best)


def fit_range(notes: list[Note], lo: int, hi: int) -> list[Note]:
    """Move the whole part by octaves to fit as many notes as possible, then
    fold the rest in by octaves."""
    if not notes:
        return notes
    pitches = np.array([n.pitch for n in notes])
    best = max(range(-4, 5), key=lambda k: (np.sum((pitches + 12 * k >= lo) & (pitches + 12 * k <= hi)), -abs(k)))
    out = []
    for n in notes:
        p = n.pitch + 12 * best
        while p < lo:
            p += 12
        while p > hi:
            p -= 12
        if p < lo:  # range narrower than an octave: clamp to the nearest edge
            p = lo
        out.append(replace(n, pitch=int(p)))
    return out


def reduce_chord(c: Chord, keep_sevenths: bool) -> Chord:
    if c.root is None:
        return c
    q = c.quality if keep_sevenths and c.quality in ("7", "maj7", "m7", "min", "maj", "dim") else theory.TRIAD_OF[c.quality]
    return replace(c, quality=q, bass=c.bass if keep_sevenths else None)


def transpose_chord(c: Chord, s: int) -> Chord:
    if c.root is None:
        return c
    return replace(c, root=(c.root + s) % 12, bass=None if c.bass is None else (c.bass + s) % 12)


def accompaniment(chords: list[Chord], tl: Timeline, style: str, grid: float) -> list[Note]:
    if style == "none":
        return []
    notes: list[Note] = []
    bar_times = list(tl.downbeat_times())
    for c in chords:
        if c.root is None:
            continue
        b0 = max(round(tl.beat_at(c.start) / grid) * grid, 0)
        b1 = round(tl.beat_at(c.end) / grid) * grid
        if b1 - b0 < grid:
            continue
        start, end = tl.time_at(b0), tl.time_at(b1)
        root = ACCOMP_LO + c.root  # C3..B3
        third = root + theory.CHORD_QUALITIES[c.quality][0][1]
        fifth = root + theory.CHORD_QUALITIES[c.quality][0][2]
        if style in ("root_on_change",):
            pitches = [root]
        elif style == "root_each_bar":
            pitches = [root]
        elif style == "root_fifth":
            pitches = [root, fifth]
        else:
            pitches = [root, third]
        # re-strike at every bar line inside the chord (except "on change")
        cuts = [start]
        if style != "root_on_change":
            cuts += [t for t in bar_times if start + 0.05 < t < end - 0.05]
        cuts.append(end)
        for a, b in zip(cuts[:-1], cuts[1:]):
            for p in pitches:
                notes.append(Note(int(p), float(a), float(b), 70, c.confidence))
    return notes


def limit_keys(melody: list[Note], accomp: list[Note], max_keys: int) -> list[Note]:
    """Merge melody + accompaniment so that at no moment more than `max_keys`
    keys are held. Melody wins; then lower accompaniment notes (the root)."""
    allowed_acc = max(max_keys - 1, 0) if melody else max_keys
    kept: list[Note] = []
    acc_sorted = sorted(accomp, key=lambda n: (n.start, n.pitch))
    for n in acc_sorted:
        held = [k for k in kept if k.start < n.end - 1e-3 and k.end > n.start + 1e-3]
        mel_now = any(m.start < n.end - 1e-3 and m.end > n.start + 1e-3 for m in melody)
        budget = allowed_acc if mel_now else max_keys
        if len(held) < budget:
            kept.append(n)
    return sorted(list(melody) + kept, key=lambda n: (n.start, n.pitch))


# --------------------------------------------------------------------------- main entry

def simplify(melody: Part | None, chords: list[Chord], key: KeyInfo | None, tl: Timeline, *,
             difficulty: int = 2, transpose: str | int = "auto", max_keys: int | None = None,
             low_conf: float = 0.5) -> dict:
    st = settings(difficulty, max_keys)
    grid = st["grid"]
    mel_notes = list(melody.notes) if melody else []

    if mel_notes:
        confident = [n for n in mel_notes if n.confidence >= st["drop_conf"]]
        if len(confident) >= 0.3 * len(mel_notes):
            mel_notes = confident
        mel_notes = monophonic(quantize(mel_notes, tl, grid))

    red_chords = [reduce_chord(c, st["keep_sevenths"]) for c in chords]

    if transpose == "auto":
        shift = choose_transpose(mel_notes, red_chords)
    elif transpose in (None, "none"):
        shift = 0
    else:
        shift = int(transpose)

    mel_notes = [replace(n, pitch=n.pitch + shift) for n in mel_notes]
    mel_hi = min(MELODY_LO + st["melody_range"] - 1, MELODY_HI)
    mel_notes = fit_range(mel_notes, MELODY_LO, mel_hi)
    red_chords = [transpose_chord(c, shift) for c in red_chords]
    acc = accompaniment(red_chords, tl, st["accomp"], max(grid, 1.0))
    arrangement = limit_keys(mel_notes, acc, st["max_keys"])

    info = {
        "difficulty": int(difficulty),
        "transpose": int(shift),
        "max_keys": int(st["max_keys"]),
        "grid_beats": grid,
        "melody_range": [theory.note_name(MELODY_LO), theory.note_name(mel_hi)],
        "accompaniment": st["accomp"],
        "accompaniment_range": [theory.note_name(ACCOMP_LO), theory.note_name(ACCOMP_HI + 7)],
    }
    new_key = None
    if key is not None:
        new_key = KeyInfo((key.tonic + shift) % 12, key.mode, key.confidence, [])
    parts = []
    if mel_notes:
        parts.append(Part("melody-easy", "Melody (easy)", "melody", mel_notes,
                          (melody.method if melody else "") + "+simplify", melody.stem if melody else None,
                          program=0, transpose=shift, simplified=info))
    if acc:
        parts.append(Part("chords-easy", "Left hand (easy)", "chords", acc, "chords+simplify", None,
                          program=0, transpose=shift, simplified=info))
    parts.append(Part("arrangement", "Easy arrangement", "arrangement", arrangement,
                      "simplify", None, program=0, transpose=shift, simplified=info))
    return {"info": info, "key": new_key, "chords": red_chords, "parts": parts}
