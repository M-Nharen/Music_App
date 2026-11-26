#!/usr/bin/env python3
"""Make the tiny Jam-mode test song: 4 s at 120 BPM in G major, four stems
of plain tones (vocals = melody, bass, other = chords, drums = noise ticks)
and a song.json that matches. Needs numpy and ffmpeg.

    cd tests/fixtures/jam-song && python3 make.py
"""
import json, subprocess, wave
from pathlib import Path
import numpy as np

SR, DUR, BEAT = 44100, 4.0, 0.5
here = Path(__file__).parent
(here / "stems").mkdir(exist_ok=True)
t = np.arange(int(SR * DUR)) / SR
hz = lambda p: 440.0 * 2 ** ((p - 69) / 12)

def tone(notes, amp, harmonics=1):
    y = np.zeros_like(t)
    for p, a, b in notes:
        m = (t >= a) & (t < b)
        env = np.minimum(1, (t[m] - a) / 0.01) * np.minimum(1, (b - t[m]) / 0.03)
        for h in range(1, harmonics + 1):
            y[m] += np.sin(2 * np.pi * hz(p) * h * (t[m] - a)) * env / h
    return y * amp

melody = [(71, 0.0, 0.45), (74, 0.5, 0.95), (79, 1.0, 1.45), (78, 1.5, 1.95),
          (76, 2.0, 2.45), (72, 2.5, 2.95), (74, 3.0, 3.45), (71, 3.5, 3.95)]
bass = [(43, 0.0, 1.9), (48, 2.0, 3.9)]
chords = [(p, 0.0, 1.95) for p in (55, 59, 62)] + [(p, 2.0, 3.95) for p in (55, 60, 64)]
rng = np.random.default_rng(7)
drums = np.zeros_like(t)
for k in range(8):
    s = int(k * BEAT * SR)
    n = int(0.04 * SR)
    drums[s:s + n] += rng.standard_normal(n) * np.exp(-np.arange(n) / (0.008 * SR)) * 0.5

stems = {"vocals": tone(melody, 0.4, 2), "drums": drums, "bass": tone(bass, 0.4), "other": tone(chords, 0.15, 2)}
for name, y in stems.items():
    pcm = (np.clip(np.stack([y, y], 1), -1, 1) * 32767).astype("<i2")
    wav = here / "stems" / f"{name}.wav"
    with wave.open(str(wav), "wb") as w:
        w.setnchannels(2); w.setsampwidth(2); w.setframerate(SR); w.writeframes(pcm.tobytes())
    subprocess.run(["ffmpeg", "-loglevel", "error", "-y", "-i", str(wav), "-c:a", "flac",
                    "-compression_level", "12", str(here / "stems" / f"{name}.flac")], check=True)
    wav.unlink()

names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
def notes(lst, shift=0):
    return [{"pitch": p + shift, "name": f"{names[(p + shift) % 12]}{(p + shift) // 12 - 1}", "start": a, "end": b,
             "start_beat": a / BEAT, "end_beat": b / BEAT, "velocity": 96, "confidence": 0.9,
             "low_confidence": False} for p, a, b in lst]
def chord(sym, root, pcs, a, b, roman):
    return {"start": a, "end": b, "start_beat": a / BEAT, "end_beat": b / BEAT, "symbol": sym, "root": root,
            "quality": "maj", "bass": None, "pitch_classes": pcs, "roman": roman, "diatonic": True,
            "confidence": 0.9, "low_confidence": False}
def part(id, name, role, stem, ns, transpose=0, is_drum=False, program=0):
    return {"id": id, "name": name, "role": role, "method": "test", "stem": stem, "is_drum": is_drum,
            "program": program, "transpose": transpose, "notes": ns}

drum_notes = [{"pitch": 42, "name": "F#2", "start": k * BEAT, "end": k * BEAT + 0.05, "start_beat": k,
               "end_beat": k + 0.1, "velocity": 90, "confidence": 0.9, "low_confidence": False} for k in range(8)]
song = {
    "format": "qwertone-song", "format_version": 1,
    "analyzer": {"version": "test", "mode": "simplified", "fingerprint": "jamtest00001"},
    "source": {"kind": "audio", "file": "jam.wav", "path": "/tmp/jam.wav", "sha256": "0" * 64,
               "duration": DUR, "sample_rate": SR},
    "metadata": {"title": "Jam test", "artist": "Qwertone", "album": None},
    "duration": DUR, "low_confidence_threshold": 0.5,
    "stems": [{"name": n, "file": f"stems/{n}.flac"} for n in stems],
    "tempo": {"bpm": 120.0, "beats_per_bar": 4, "time_signature": [4, 4], "confidence": 1.0},
    "beats": [{"time": k * BEAT, "bar": k // 4 + 1, "beat": k % 4 + 1, "downbeat": k % 4 == 0} for k in range(8)],
    "key": {"tonic": "G", "tonic_pc": 7, "mode": "major", "name": "G major", "confidence": 0.9},
    "chords": [chord("G", "G", [7, 11, 2], 0.0, 2.0, "I"), chord("C", "C", [0, 4, 7], 2.0, 4.0, "IV")],
    "sections": [{"start": 0.0, "end": 4.0, "start_bar": 1, "end_bar": 2, "label": "A", "name": "verse",
                  "confidence": 1.0}],
    "parts": [part("melody", "Melody", "melody", "vocals", notes(melody)),
              part("chords", "Chords", "chords", None, notes(chords)),
              part("bass", "Bass", "bass", "bass", notes(bass), program=33),
              part("drums", "Drums", "drums", "drums", drum_notes, is_drum=True),
              part("melody-easy", "Melody (easy)", "melody", "vocals", notes(melody, 5), transpose=5)],
    "simplified": {"difficulty": 2, "transpose": 5, "max_keys": 2,
                   "key": {"tonic": "C", "tonic_pc": 0, "mode": "major", "name": "C major", "confidence": 0.9},
                   "chords": [chord("C", "C", [0, 4, 7], 0.0, 2.0, "I"), chord("F", "F", [5, 9, 0], 2.0, 4.0, "IV")],
                   "parts": ["melody-easy"]},
}
(here / "song.json").write_text(json.dumps(song, indent=1) + "\n")
