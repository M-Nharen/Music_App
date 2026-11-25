"""Music-theory helpers. Names match the Rust app (`src/music.rs`): sharps only,
C4 = MIDI 60."""

from __future__ import annotations

import numpy as np

NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
BLACK_PCS = {1, 3, 6, 8, 10}

# quality -> (intervals from the root, symbol suffix, template weights)
CHORD_QUALITIES: dict[str, tuple[tuple[int, ...], str]] = {
    "maj": ((0, 4, 7), ""),
    "min": ((0, 3, 7), "m"),
    "7": ((0, 4, 7, 10), "7"),
    "maj7": ((0, 4, 7, 11), "maj7"),
    "m7": ((0, 3, 7, 10), "m7"),
    "dim": ((0, 3, 6), "dim"),
    "aug": ((0, 4, 8), "aug"),
    "sus2": ((0, 2, 7), "sus2"),
    "sus4": ((0, 5, 7), "sus4"),
}
TRIAD_OF = {
    "maj": "maj", "min": "min", "7": "maj", "maj7": "maj", "m7": "min",
    "dim": "dim", "aug": "maj", "sus2": "maj", "sus4": "maj",
}
MAJOR_SCALE = (0, 2, 4, 5, 7, 9, 11)
MINOR_SCALE = (0, 2, 3, 5, 7, 8, 10)

# Krumhansl–Kessler key profiles.
KK_MAJOR = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
KK_MINOR = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])


def note_name(midi: int) -> str:
    """60 -> "C4"."""
    midi = int(midi)
    return f"{NOTE_NAMES[midi % 12]}{midi // 12 - 1}"


def pc_name(pc: int) -> str:
    return NOTE_NAMES[int(pc) % 12]


def is_black(midi: int) -> bool:
    return int(midi) % 12 in BLACK_PCS


def hz_to_midi(hz):
    return 69.0 + 12.0 * np.log2(np.asarray(hz, dtype=float) / 440.0)


def midi_to_hz(midi):
    return 440.0 * 2.0 ** ((np.asarray(midi, dtype=float) - 69.0) / 12.0)


def chord_symbol(root: int | None, quality: str | None, bass: int | None = None) -> str:
    """(9, "min", None) -> "Am";  (0, "maj", 4) -> "C/E";  None -> "N" (no chord)."""
    if root is None or quality is None:
        return "N"
    s = pc_name(root) + CHORD_QUALITIES[quality][1]
    if bass is not None and bass % 12 != root % 12:
        s += "/" + pc_name(bass)
    return s


def chord_pcs(root: int, quality: str) -> list[int]:
    return [(root + i) % 12 for i in CHORD_QUALITIES[quality][0]]


_MAJOR_NUMERALS = ["I", "bII", "II", "bIII", "III", "IV", "#IV", "V", "bVI", "VI", "bVII", "VII"]
_MINOR_NUMERALS = ["I", "bII", "II", "III", "#III", "IV", "#IV", "V", "VI", "#VI", "VII", "#VII"]


def roman_numeral(root: int | None, quality: str | None, tonic: int, mode: str) -> str | None:
    """Roman numeral of a chord in a key. Minor-key numerals are relative to the
    natural minor scale (so in A minor, C is "III" and G is "VII")."""
    if root is None or quality is None:
        return None
    table = _MAJOR_NUMERALS if mode == "major" else _MINOR_NUMERALS
    num = table[(root - tonic) % 12]
    triad = TRIAD_OF[quality]
    if triad in ("min", "dim"):
        # lowercase the letters, keep the accidental prefix
        num = num[0] + num[1:].lower() if num[0] in "b#" else num.lower()
    suffix = {"dim": "°", "aug": "+", "7": "7", "maj7": "maj7", "m7": "7",
              "sus2": "sus2", "sus4": "sus4"}.get(quality, "")
    return num + suffix


def is_diatonic(root: int, quality: str, tonic: int, mode: str) -> bool:
    scale = MAJOR_SCALE if mode == "major" else MINOR_SCALE
    pcs = {(tonic + s) % 12 for s in scale}
    return all(p in pcs for p in chord_pcs(root, quality))


def key_name(tonic: int, mode: str) -> str:
    return f"{pc_name(tonic)} {mode}"


def estimate_key(pc_weights: np.ndarray) -> list[tuple[float, int, str]]:
    """Correlate a 12-bin pitch-class histogram against all 24 key profiles.
    Returns [(correlation, tonic, mode), ...] best first."""
    w = np.asarray(pc_weights, dtype=float)
    if w.sum() <= 0 or np.allclose(w, w[0]):
        return [(0.0, 0, "major")]
    out = []
    for tonic in range(12):
        for mode, prof in (("major", KK_MAJOR), ("minor", KK_MINOR)):
            r = np.corrcoef(w, np.roll(prof, tonic))[0, 1]
            out.append((float(r), tonic, mode))
    out.sort(key=lambda x: -x[0])
    return out
