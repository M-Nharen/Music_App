"""Write a type-1 MIDI file whose tempo map follows the detected beats, so a
beat in the MIDI file is a beat in the recording and bar lines line up."""

from __future__ import annotations

from pathlib import Path

import mido
import numpy as np

from .model import Chord, KeyInfo, Part, Section, Timeline
from .theory import pc_name

TPB = 480  # ticks per beat
_MAJOR_MIDI_NAMES = {"D#": "Eb", "G#": "Ab", "A#": "Bb"}


class TickMap:
    """Seconds -> ticks. Tick 0 is time 0. The stretch before the first downbeat
    is a pickup of a whole number of beats; after that each detected beat is
    exactly TPB ticks long."""

    def __init__(self, tl: Timeline):
        b = tl.beats
        db_idx = np.flatnonzero(tl.downbeat)
        first = int(db_idx[0]) if len(db_idx) else 0
        t_first = float(b[first])
        ibi0 = float(b[min(first + 1, len(b) - 1)] - b[first]) or 0.5
        # segments: (time_start, tick_start, seconds_per_tick)
        self.segments: list[tuple[float, int, float]] = []
        self.pickup_beats = 0
        tick = 0
        if t_first > 0.02:
            n_pre = max(1, int(round(t_first / ibi0)))
            self.pickup_beats = n_pre
            self.segments.append((0.0, 0, t_first / (n_pre * TPB)))
            tick = n_pre * TPB
        for i in range(first, len(b) - 1):
            self.segments.append((float(b[i]), tick, float(b[i + 1] - b[i]) / TPB))
            tick += TPB
        last_spt = self.segments[-1][2] if self.segments else 0.5 / TPB
        self.segments.append((float(b[-1]) if len(b) > first else t_first, tick, last_spt))
        self.beats_per_bar = tl.beats_per_bar
        self._starts = np.array([s[0] for s in self.segments])

    def tick(self, t: float) -> int:
        i = int(np.clip(np.searchsorted(self._starts, t, side="right") - 1, 0, len(self.segments) - 1))
        t0, k0, spt = self.segments[i]
        return max(0, int(round(k0 + (t - t0) / spt)))

    def tempo_events(self) -> list[tuple[int, int]]:
        out, last = [], None
        for _, tick, spt in self.segments:
            us = int(round(spt * TPB * 1e6))
            us = int(np.clip(us, 1, 0xFFFFFF))
            if us != last:
                out.append((tick, us))
                last = us
        return out


def _abs_to_track(events: list[tuple[int, int, mido.Message]], name: str) -> mido.MidiTrack:
    track = mido.MidiTrack()
    track.append(mido.MetaMessage("track_name", name=name, time=0))
    events.sort(key=lambda e: (e[0], e[1]))
    now = 0
    for tick, _order, msg in events:
        track.append(msg.copy(time=tick - now))
        now = tick
    track.append(mido.MetaMessage("end_of_track", time=0))
    return track


def write_midi(path: str | Path, parts: list[Part], tl: Timeline, *, key: KeyInfo | None = None,
               chords: list[Chord] | None = None, sections: list[Section] | None = None) -> dict[str, int]:
    """Returns {part id: track index}."""
    tm = TickMap(tl)
    mid = mido.MidiFile(type=1, ticks_per_beat=TPB)

    conductor: list[tuple[int, int, mido.Message]] = []
    for tick, us in tm.tempo_events():
        conductor.append((tick, 0, mido.MetaMessage("set_tempo", tempo=us)))
    if tm.pickup_beats:
        conductor.append((0, 1, mido.MetaMessage("time_signature", numerator=tm.pickup_beats, denominator=4)))
        conductor.append((tm.pickup_beats * TPB, 1, mido.MetaMessage("time_signature", numerator=tm.beats_per_bar, denominator=4)))
    else:
        conductor.append((0, 1, mido.MetaMessage("time_signature", numerator=tm.beats_per_bar, denominator=4)))
    if key is not None:
        name = pc_name(key.tonic)
        name = name + "m" if key.mode == "minor" else _MAJOR_MIDI_NAMES.get(name, name)
        try:
            conductor.append((0, 2, mido.MetaMessage("key_signature", key=name)))
        except (ValueError, KeyError):
            pass
    for s in sections or []:
        conductor.append((tm.tick(s.start), 3, mido.MetaMessage("marker", text=f"{s.label}:{s.name}")))
    for c in chords or []:
        if c.root is not None:
            conductor.append((tm.tick(c.start), 4, mido.MetaMessage("text", text=f"chord:{c.symbol}")))
    mid.tracks.append(_abs_to_track(conductor, "conductor"))

    index: dict[str, int] = {}
    channel = 0
    for part in parts:
        if part.is_drum:
            ch = 9
        else:
            ch = channel
            channel = channel + 1 if channel + 1 != 9 else 10
            ch = min(ch, 15)
        ev: list[tuple[int, int, mido.Message]] = []
        if not part.is_drum:
            ev.append((0, 0, mido.Message("program_change", program=int(part.program) & 127, channel=ch)))
        for n in part.notes:
            a = tm.tick(n.start)
            b = max(tm.tick(n.end), a + 1)
            v = int(np.clip(n.velocity, 1, 127))
            ev.append((a, 2, mido.Message("note_on", note=int(np.clip(n.pitch, 0, 127)), velocity=v, channel=ch)))
            ev.append((b, 1, mido.Message("note_off", note=int(np.clip(n.pitch, 0, 127)), velocity=0, channel=ch)))
        index[part.id] = len(mid.tracks)
        mid.tracks.append(_abs_to_track(ev, part.id))
    mid.save(str(path))
    return index
