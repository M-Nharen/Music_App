"""MIDI files as input: the notes are already known, so we only need to pick
which track is which part and derive chords, key and sections."""

from __future__ import annotations

import numpy as np

from . import chords as chordmod
from . import keydetect, rhythm, sections
from .model import KeyInfo, Note, Part, Timeline
from .theory import NOTE_NAMES

VERSION = 1

_FLAT_TO_SHARP = {"Db": "C#", "Eb": "D#", "Gb": "F#", "Ab": "G#", "Bb": "A#", "Cb": "B", "Fb": "E"}


class MidiSong:
    def __init__(self, path):
        import pretty_midi
        self.pm = pretty_midi.PrettyMIDI(str(path))
        self.duration = float(self.pm.get_end_time())
        self.instruments = [i for i in self.pm.instruments if i.notes]

    # ---------------------------------------------------------------- grid
    def rhythm(self) -> dict:
        pm = self.pm
        beats = list(pm.get_beats())
        downs = set(np.round(pm.get_downbeats(), 4))
        flags = [round(b, 4) in downs for b in beats]
        bpb = self._main_meter()
        _, tempi = pm.get_tempo_changes()
        bpm = float(tempi[0]) if len(tempi) else 120.0
        if len(beats) > 2:
            bpm = 60.0 / float(np.median(np.diff(beats)))
        return rhythm.from_grid(beats, flags, bpb, bpm)

    def _main_meter(self) -> int:
        """Numerator of the time signature in force for the longest time
        (a pickup bar written as 1/4 shouldn't make the whole song 1/4)."""
        ts = self.pm.time_signature_changes
        if not ts:
            return 4
        spans = {}
        for i, t in enumerate(ts):
            end = ts[i + 1].time if i + 1 < len(ts) else max(self.duration, t.time)
            spans[t.numerator] = spans.get(t.numerator, 0.0) + (end - t.time)
        return int(max(spans, key=spans.get))

    # ---------------------------------------------------------------- notes
    def _notes(self, inst) -> list[Note]:
        return [Note(int(n.pitch), float(n.start), float(n.end), int(n.velocity), 1.0) for n in inst.notes]

    def pitched(self):
        return [i for i in self.instruments if not i.is_drum]

    def all_pitched_notes(self) -> list[Note]:
        return [n for i in self.pitched() for n in self._notes(i)]

    def pick(self, role: str):
        """Best instrument for a role."""
        pitched = self.pitched()
        name = lambda i: (i.name or "").lower()
        if role == "drums":
            return next((i for i in self.instruments if i.is_drum), None)
        if not pitched:
            return None
        if role == "bass":
            named = [i for i in pitched if "bass" in name(i) or 32 <= i.program <= 39]
            return named[0] if named else min(pitched, key=lambda i: np.mean([n.pitch for n in i.notes]))
        if role == "piano":
            named = [i for i in pitched if "piano" in name(i) or i.program <= 7]
            return max(named or pitched, key=lambda i: len(i.notes))
        if role == "melody":
            for kw in ("melody", "vocal", "voice", "lead", "sing"):
                for i in pitched:
                    if kw in name(i):
                        return i

            def score(i):
                p = np.array([n.pitch for n in i.notes])
                starts = np.array([n.start for n in i.notes])
                mono = len(np.unique(np.round(starts, 2))) / max(len(starts), 1)  # 1.0 = one note at a time
                return float(np.mean(p)) / 12 + 2 * mono - (3 if 32 <= i.program <= 39 else 0)
            return max(pitched, key=score)
        return None

    def part_for(self, inst, role: str, pid: str | None = None, mono: bool | None = None) -> Part:
        from .simplify import monophonic
        notes = self._notes(inst)
        if mono if mono is not None else role == "melody":
            notes = monophonic(notes)
        label = inst.name or ("Drums" if inst.is_drum else f"Program {inst.program}")
        return Part(pid or role, label, role, notes, "midi", None, is_drum=inst.is_drum,
                    program=int(inst.program))

    # ---------------------------------------------------------------- analysis
    def key_signature(self) -> KeyInfo | None:
        ks = self.pm.key_signature_changes
        if not ks:
            return None
        import pretty_midi
        name = pretty_midi.key_number_to_key_name(ks[0].key_number)  # e.g. "Bb Major"
        tonic, mode = name.split()
        tonic = _FLAT_TO_SHARP.get(tonic, tonic)
        return KeyInfo(NOTE_NAMES.index(tonic), mode.lower(), 0.9, [])

    def analyze(self, vocab: str = "extended"):
        rh = self.rhythm()
        tl = Timeline(rh["beats"], rh["downbeats"], rh["beats_per_bar"])
        notes = self.all_pitched_notes()
        dur = max(self.duration, 1.0)
        chroma, bass, times = chordmod.chroma_from_notes(notes, dur)
        chords = chordmod.detect(chroma, bass, times, rh["beats"], dur, vocab=vocab, sharp=True)
        for c in chords:
            c.confidence = max(c.confidence, 0.6)  # symbolic input: notes are certain, naming may not be
        key = self.key_signature() or keydetect.detect(keydetect.histogram_from_chroma(chroma, bass), chords)
        density = np.zeros_like(times)
        for n in notes:
            density[int(n.start * 43): int(n.end * 43) + 1] += 1
        feats = {"chroma": chroma, "times": times, "rms": density[: len(times)]}
        secs = sections.segment(feats, tl, dur)
        return rh, tl, chords, key, secs
