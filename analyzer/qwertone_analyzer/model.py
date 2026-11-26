"""Plain data types shared by the pipeline. Everything serialises to the
song.json layout described in SONG_FORMAT.md."""

from __future__ import annotations

from dataclasses import asdict, dataclass, field
from typing import Any

import numpy as np

from . import theory


@dataclass
class Note:
    pitch: int            # MIDI note number
    start: float          # seconds
    end: float            # seconds
    velocity: int = 90    # 1..127
    confidence: float = 1.0  # 0..1

    @property
    def dur(self) -> float:
        return self.end - self.start

    def to_dict(self, timeline: "Timeline", threshold: float) -> dict[str, Any]:
        return {
            "pitch": int(self.pitch),
            "name": theory.note_name(self.pitch),
            "start": round(float(self.start), 4),
            "end": round(float(self.end), 4),
            "start_beat": round(float(timeline.beat_at(self.start)), 4),
            "end_beat": round(float(timeline.beat_at(self.end)), 4),
            "velocity": int(np.clip(self.velocity, 1, 127)),
            "confidence": round(float(self.confidence), 3),
            "low_confidence": bool(self.confidence < threshold),
        }

    @staticmethod
    def from_dict(d: dict) -> "Note":
        return Note(int(d["pitch"]), float(d["start"]), float(d["end"]),
                    int(d.get("velocity", 90)), float(d.get("confidence", 1.0)))


@dataclass
class Chord:
    start: float
    end: float
    root: int | None       # pitch class 0..11, None = no chord
    quality: str | None    # key of theory.CHORD_QUALITIES
    bass: int | None = None
    confidence: float = 1.0

    @property
    def symbol(self) -> str:
        return theory.chord_symbol(self.root, self.quality, self.bass)

    def to_dict(self, timeline: "Timeline", key: "KeyInfo | None", threshold: float) -> dict:
        roman = None
        diatonic = None
        if key is not None and self.root is not None:
            roman = theory.roman_numeral(self.root, self.quality, key.tonic, key.mode)
            diatonic = theory.is_diatonic(self.root, self.quality, key.tonic, key.mode)
        return {
            "start": round(float(self.start), 4),
            "end": round(float(self.end), 4),
            "start_beat": round(float(timeline.beat_at(self.start)), 4),
            "end_beat": round(float(timeline.beat_at(self.end)), 4),
            "symbol": self.symbol,
            "root": None if self.root is None else theory.pc_name(self.root),
            "quality": self.quality,
            "bass": None if self.bass is None else theory.pc_name(self.bass),
            "pitch_classes": [] if self.root is None else theory.chord_pcs(self.root, self.quality),
            "roman": roman,
            "diatonic": diatonic,
            "confidence": round(float(self.confidence), 3),
            "low_confidence": bool(self.confidence < threshold),
        }

    def to_cache(self) -> dict:
        return asdict(self)

    @staticmethod
    def from_cache(d: dict) -> "Chord":
        return Chord(**d)


@dataclass
class KeyInfo:
    tonic: int
    mode: str  # "major" | "minor"
    confidence: float
    alternatives: list[tuple[int, str, float]] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "tonic": theory.pc_name(self.tonic),
            "tonic_pc": self.tonic,
            "mode": self.mode,
            "name": theory.key_name(self.tonic, self.mode),
            "confidence": round(float(self.confidence), 3),
            "alternatives": [
                {"name": theory.key_name(t, m), "score": round(float(s), 3)}
                for t, m, s in self.alternatives
            ],
        }

    def to_cache(self) -> dict:
        return {"tonic": self.tonic, "mode": self.mode, "confidence": self.confidence,
                "alternatives": [list(a) for a in self.alternatives]}

    @staticmethod
    def from_cache(d: dict) -> "KeyInfo":
        return KeyInfo(d["tonic"], d["mode"], d["confidence"],
                       [tuple(a) for a in d.get("alternatives", [])])


@dataclass
class Section:
    start: float
    end: float
    label: str               # "A", "B", ... same letter = same material
    name: str                # best guess: intro / verse / chorus / bridge / outro / section
    confidence: float = 0.5

    def to_dict(self, timeline: "Timeline") -> dict:
        return {
            "start": round(float(self.start), 4),
            "end": round(float(self.end), 4),
            "start_bar": timeline.bar_at(self.start),
            "end_bar": timeline.bar_at(self.end - 1e-3),
            "label": self.label,
            "name": self.name,
            "confidence": round(float(self.confidence), 3),
        }


@dataclass
class Part:
    id: str
    name: str
    role: str                  # melody | chords | bass | piano | drums | vocals | other | guitar | arrangement
    notes: list[Note]
    method: str                # how the notes were made (model name)
    stem: str | None = None    # which stem the notes came from
    is_drum: bool = False
    program: int = 0           # General MIDI program for playback
    transpose: int = 0         # semitones relative to the recording
    simplified: dict | None = None


class Timeline:
    """Beat grid. Maps seconds <-> beat positions (beat 0 = first detected beat)
    and knows where bars start."""

    def __init__(self, beats, downbeat_flags=None, beats_per_bar: int = 4):
        self.beats = np.asarray(beats, dtype=float)
        if len(self.beats) < 2:
            self.beats = np.array([0.0, 0.5])
        self.beats_per_bar = beats_per_bar
        if downbeat_flags is None:
            downbeat_flags = [(i % beats_per_bar) == 0 for i in range(len(self.beats))]
        self.downbeat = np.asarray(downbeat_flags, dtype=bool)
        self._first_ibi = float(self.beats[1] - self.beats[0])
        self._last_ibi = float(self.beats[-1] - self.beats[-2])

    def beat_at(self, t: float) -> float:
        b = self.beats
        if t <= b[0]:
            return (t - b[0]) / self._first_ibi
        if t >= b[-1]:
            return len(b) - 1 + (t - b[-1]) / self._last_ibi
        return float(np.interp(t, b, np.arange(len(b))))

    def time_at(self, beat: float) -> float:
        b = self.beats
        if beat <= 0:
            return float(b[0] + beat * self._first_ibi)
        if beat >= len(b) - 1:
            return float(b[-1] + (beat - (len(b) - 1)) * self._last_ibi)
        return float(np.interp(beat, np.arange(len(b)), b))

    def bar_numbers(self) -> list[int]:
        """Bar number (1-based) of every beat. Beats before the first downbeat
        are a pickup, bar 0."""
        bars, bar = [], 0
        for flag in self.downbeat:
            if flag:
                bar += 1
            bars.append(bar)
        return bars

    def bar_at(self, t: float) -> int:
        idx = int(np.clip(np.floor(self.beat_at(t)), 0, len(self.beats) - 1))
        return self.bar_numbers()[idx]

    def downbeat_times(self) -> np.ndarray:
        return self.beats[self.downbeat]
