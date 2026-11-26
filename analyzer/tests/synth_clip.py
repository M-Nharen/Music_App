"""Generate a short test song with known answers.

27 s, 120 BPM, 4/4, G major, sections A B A (4 bars each) after a one-beat
lead-in. Separate "stems" (melody, chords, bass, drums) are written too, plus
a MIDI file with the same notes.

    python tests/synth_clip.py /tmp/clip     # writes mix.wav, stems, song.mid, truth.json
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np

SR = 44100
BPM = 120.0
BEAT = 60.0 / BPM
LEAD_IN = BEAT          # one silent beat before bar 1
BAR = 4 * BEAT

# chord per bar: (symbol, root pc, quality, pitches of the pad, bass note)
A_CHORDS = [("G", 7, "maj", [55, 59, 62], 43), ("Em", 4, "min", [52, 55, 59], 40),
            ("C", 0, "maj", [48, 52, 55], 36), ("D", 2, "maj", [50, 54, 57], 38)]
B_CHORDS = [("Em", 4, "min", [52, 55, 59], 40), ("Bm", 11, "min", [47, 50, 54], 35),
            ("C", 0, "maj", [48, 52, 55], 36), ("Am", 9, "min", [45, 48, 52], 33)]
A_MELODY = [[71, 74, 71, 67], [67, 71, 76, 71], [72, 76, 72, 67], [74, 78, 74, 69]]
B_MELODY = [[76, 74, 71, 67], [74, 71, 74, 78], [72, 76, 79, 76], [76, 72, 69, 72]]
SECTIONS = [("A", A_CHORDS, A_MELODY), ("B", B_CHORDS, B_MELODY), ("A", A_CHORDS, A_MELODY)]


def hz(m):
    return 440.0 * 2 ** ((np.asarray(m, float) - 69) / 12)


def _env(n, attack=0.01, release=0.05):
    t = np.arange(n) / SR
    e = np.minimum(1.0, t / attack)
    rel = int(release * SR)
    if n > rel:
        e[-rel:] *= np.linspace(1, 0, rel)
    return e


def _tone(pitch, dur, harmonics=(1.0, 0.5, 0.25), vibrato=0.0):
    n = int(dur * SR)
    t = np.arange(n) / SR
    f = hz(pitch) * (1 + vibrato * np.sin(2 * np.pi * 5.0 * t))
    phase = 2 * np.pi * np.cumsum(f) / SR
    y = sum(a * np.sin((k + 1) * phase) for k, a in enumerate(harmonics))
    return y * _env(n)


def _add(buf, start, y):
    a = int(start * SR)
    b = min(a + len(y), len(buf))
    buf[a:b] += y[: b - a]


def build():
    total = LEAD_IN + 12 * BAR + 1.0
    n = int(total * SR)
    melody, chords, bass, drums = (np.zeros(n) for _ in range(4))
    rng = np.random.default_rng(7)
    truth = {"bpm": BPM, "beats_per_bar": 4, "key": "G major", "lead_in": LEAD_IN,
             "chords": [], "melody": [], "sections": [], "beats": []}
    midi_notes = {"melody": [], "chords": [], "bass": [], "drums": []}

    bar_idx = 0
    for label, chs, mel in SECTIONS:
        sec_start = LEAD_IN + bar_idx * BAR
        loud = 1.4 if label == "B" else 1.0
        for (sym, root, q, pad, bnote), bar_mel in zip(chs, mel):
            t0 = LEAD_IN + bar_idx * BAR
            truth["chords"].append({"start": t0, "end": t0 + BAR, "symbol": sym})
            for p in pad:
                _add(chords, t0, 0.12 * _tone(p, BAR * 0.98, (1.0, 0.4, 0.2, 0.1)))
                midi_notes["chords"].append((p, t0, t0 + BAR * 0.98, 70))
            for half in (0, 2):
                _add(bass, t0 + half * BEAT, 0.35 * _tone(bnote, 2 * BEAT * 0.9, (1.0, 0.6, 0.3)))
                midi_notes["bass"].append((bnote, t0 + half * BEAT, t0 + half * BEAT + 2 * BEAT * 0.9, 90))
            for k, p in enumerate(bar_mel):
                s = t0 + k * BEAT
                _add(melody, s, 0.22 * _tone(p, BEAT * 0.85, (1.0, 0.6, 0.4, 0.25, 0.15), vibrato=0.004))
                truth["melody"].append({"pitch": p, "start": s, "end": s + BEAT * 0.85})
                midi_notes["melody"].append((p, s, s + BEAT * 0.85, 100))
            for k in range(4):
                s = t0 + k * BEAT
                truth["beats"].append(s)
                if k in (0, 2):  # kick
                    m = int(0.25 * SR)
                    t = np.arange(m) / SR
                    f = 50 + 90 * np.exp(-t * 30)
                    _add(drums, s, 0.9 * loud * np.sin(2 * np.pi * np.cumsum(f) / SR) * np.exp(-t * 14))
                    midi_notes["drums"].append((36, s, s + 0.1, 110))
                else:  # snare
                    m = int(0.18 * SR)
                    t = np.arange(m) / SR
                    noise = rng.standard_normal(m)
                    _add(drums, s, 0.35 * loud * (noise * np.exp(-t * 25) + 0.6 * np.sin(2 * np.pi * 190 * t) * np.exp(-t * 30)))
                    midi_notes["drums"].append((38, s, s + 0.1, 100))
                subdiv = 4 if label == "B" else 2
                for h in range(subdiv):  # hi-hats
                    hs = s + h * BEAT / subdiv
                    m = int(0.04 * SR)
                    noise = np.diff(rng.standard_normal(m + 1)) * np.exp(-np.arange(m) / SR * 120)
                    _add(drums, hs, 0.12 * loud * noise)
                    midi_notes["drums"].append((42, hs, hs + 0.05, 70))
            bar_idx += 1
        truth["sections"].append({"label": label, "start": sec_start, "end": LEAD_IN + bar_idx * BAR})

    stems = {"vocals": melody, "other": chords, "bass": bass, "drums": drums}
    mix = sum(stems.values())
    peak = np.abs(mix).max()
    mix = 0.9 * mix / peak
    stems = {k: 0.9 * v / peak for k, v in stems.items()}
    truth["duration"] = total
    return mix, stems, truth, midi_notes


def write_midi(path, midi_notes):
    import mido
    tpb = 480
    mid = mido.MidiFile(type=1, ticks_per_beat=tpb)
    conductor = mido.MidiTrack()
    conductor.append(mido.MetaMessage("set_tempo", tempo=mido.bpm2tempo(BPM)))
    conductor.append(mido.MetaMessage("time_signature", numerator=1, denominator=4))
    conductor.append(mido.MetaMessage("time_signature", numerator=4, denominator=4, time=tpb))
    conductor.append(mido.MetaMessage("key_signature", key="G"))
    mid.tracks.append(conductor)
    programs = {"melody": (0, 0), "chords": (48, 1), "bass": (33, 2), "drums": (0, 9)}
    names = {"melody": "Melody", "chords": "Strings", "bass": "Bass", "drums": "Drums"}
    for part, notes in midi_notes.items():
        prog, ch = programs[part]
        tr = mido.MidiTrack()
        tr.append(mido.MetaMessage("track_name", name=names[part]))
        if ch != 9:
            tr.append(mido.Message("program_change", program=prog, channel=ch))
        ev = []
        for p, s, e, v in notes:
            ev.append((round(s / BEAT * tpb), 1, mido.Message("note_on", note=p, velocity=v, channel=ch)))
            ev.append((round(e / BEAT * tpb), 0, mido.Message("note_off", note=p, velocity=0, channel=ch)))
        ev.sort(key=lambda x: (x[0], x[1]))
        now = 0
        for tick, _, msg in ev:
            tr.append(msg.copy(time=tick - now))
            now = tick
        mid.tracks.append(tr)
    mid.save(str(path))


def write_all(out: Path) -> dict:
    import soundfile as sf
    out.mkdir(parents=True, exist_ok=True)
    mix, stems, truth, midi_notes = build()
    sf.write(out / "mix.wav", np.stack([mix, mix], 1), SR, subtype="PCM_16")
    sf.write(out / "mix.flac", np.stack([mix, mix], 1), SR, subtype="PCM_16")
    (out / "stems").mkdir(exist_ok=True)
    for k, v in stems.items():
        sf.write(out / "stems" / f"{k}.wav", np.stack([v, v], 1), SR, subtype="PCM_16")
    write_midi(out / "song.mid", midi_notes)
    with open(out / "truth.json", "w") as f:
        json.dump(truth, f, indent=1)
    return truth


if __name__ == "__main__":
    write_all(Path(sys.argv[1] if len(sys.argv) > 1 else "test-clip"))
    print("written")
