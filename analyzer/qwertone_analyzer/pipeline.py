"""The whole analysis, from an input file to a song folder."""

from __future__ import annotations

import json
from dataclasses import asdict, dataclass, field
from functools import lru_cache
from pathlib import Path

import numpy as np

from . import backends, features, keydetect, melody, rhythm, sections, simplify, stems, transcribe
from . import chords as chordmod
from .audio_io import AUDIO_EXTS, MIDI_EXTS, SR, load_audio, probe_tags, read_audio, resample, to_mono, write_audio
from .cache import Cache, default_cache_root, file_sha256, params_hash
from .model import Chord, KeyInfo, Note, Part, Timeline
from .progress import event, log
from .writer import default_songs_dir, part_id, slugify, write_song

MODES = ("chords", "melody", "chords+melody", "full", "instrument", "simplified")
INSTRUMENTS = ("bass", "piano", "drums")
# options that don't change song.json (the lyrics step has its own file and fingerprint)
NOT_IN_FINGERPRINT = ("out_dir", "cache_dir", "use_cache", "force", "lyrics", "lyrics_online")
MODE_ALIASES = {"chords-melody": "chords+melody", "chords_melody": "chords+melody", "simple": "simplified",
                "easy": "simplified", "transcribe": "full"}


@dataclass
class Options:
    mode: str = "chords+melody"
    instrument: str | None = None          # for mode "instrument"
    simplify: bool = False                 # simplify on top of any mode
    difficulty: int = 2                    # 1..5
    transpose: str = "auto"                # "auto" | "none" | "-3" ...
    max_keys: int | None = None            # override the difficulty's keys-at-once
    separate: bool = True                  # run Demucs
    stems_model: str | None = None         # default: htdemucs (htdemucs_6s for piano)
    device: str = "auto"                   # auto | cpu | cuda
    crepe_model: str = "auto"              # auto (full on GPU, tiny on CPU) | tiny | small | medium | large | full
    pitch_method: str = "auto"             # auto | crepe | pyin
    chord_vocab: str = "extended"          # basic | extended
    low_confidence: float = 0.5
    shifts: int = 0                        # Demucs test-time shifts (slower, slightly better)
    out_dir: str | None = None
    cache_dir: str | None = None
    use_cache: bool = True
    force: bool = False
    lyrics: bool = True                    # run the lyrics step after writing song.json
    lyrics_online: bool = True             # let the lyrics step ask LRCLIB

    def normalized(self) -> "Options":
        o = Options(**asdict(self))
        o.mode = MODE_ALIASES.get(o.mode, o.mode)
        if o.mode in INSTRUMENTS:
            o.instrument, o.mode = o.mode, "instrument"
        if o.mode not in MODES:
            raise ValueError(f"unknown mode {self.mode!r}; choose from {', '.join(MODES + INSTRUMENTS)}")
        if o.mode == "instrument" and o.instrument not in INSTRUMENTS:
            raise ValueError(f"mode 'instrument' needs --instrument {'/'.join(INSTRUMENTS)}")
        if o.mode == "simplified":
            o.simplify = True
        o.difficulty = int(np.clip(o.difficulty, 1, 5))
        return o

    def fingerprint(self) -> str:
        """Everything that changes the song folder (not cache/out/force)."""
        d = asdict(self)
        for k in NOT_IN_FINGERPRINT:
            d.pop(k)
        # a newer analyzer (or any changed stage) must redo old songs
        from . import __version__, midi_input, writer
        d["_versions"] = [__version__, stems.VERSION, features.VERSION, rhythm.VERSION, chordmod.VERSION,
                          keydetect.VERSION, sections.VERSION, melody.VERSION, transcribe.VERSION,
                          simplify.VERSION, midi_input.VERSION, writer.FORMAT_VERSION]
        return params_hash(d)


@dataclass
class Result:
    song_dir: Path
    song_json: Path
    skipped: bool = False
    cache_hits: list[str] = field(default_factory=list)
    cache_misses: list[str] = field(default_factory=list)
    lyrics: object | None = None           # lyrics.LyricsResult when the lyrics step ran


# ============================================================================ entry

def analyze(path: str | Path, opts: Options | None = None) -> Result:
    opts = (opts or Options()).normalized()
    path = Path(path).expanduser().resolve()
    if not path.exists():
        raise FileNotFoundError(path)
    ext = path.suffix.lower()
    if ext not in AUDIO_EXTS | MIDI_EXTS:
        raise ValueError(f"unsupported file type {ext} (audio: mp3/flac/wav/m4a/..., or .mid)")

    log("input", f"{path.name}")
    sha = file_sha256(path)
    tags = probe_tags(path) if ext in AUDIO_EXTS else {}
    title = tags.get("title") or path.stem
    out_root = Path(opts.out_dir).expanduser() if opts.out_dir else default_songs_dir()
    song_dir = out_root / f"{slugify(title)}-{sha[:8]}"
    fp = opts.fingerprint()

    existing = song_dir / "song.json"
    if existing.exists() and not opts.force:
        try:
            with open(existing) as f:
                old = json.load(f)
            if old.get("source", {}).get("sha256") == sha and old.get("analyzer", {}).get("fingerprint") == fp:
                log("done", f"already analysed with these settings: {song_dir}")
                res = Result(song_dir, existing, skipped=True, lyrics=_lyrics_step(song_dir, opts))
                event("done", song_dir=str(song_dir), skipped=True)
                return res
        except (OSError, json.JSONDecodeError):
            pass

    cache = Cache(Path(opts.cache_dir).expanduser() if opts.cache_dir else default_cache_root(),
                  sha, enabled=opts.use_cache, refresh=opts.force)
    analyzer_info = {"mode": opts.mode, "instrument": opts.instrument, "options": asdict(opts),
                     "fingerprint": fp, "models": {}}
    for k in NOT_IN_FINGERPRINT:
        analyzer_info["options"].pop(k)

    if ext in MIDI_EXTS:
        res = _analyze_midi(path, opts, analyzer_info)
        source_kind = "midi"
    else:
        res = _analyze_audio(path, opts, cache, analyzer_info)
        source_kind = "audio"

    source = {"kind": source_kind, "file": path.name, "path": str(path), "sha256": sha,
              "duration": res["duration"], "sample_rate": SR if source_kind == "audio" else None}
    metadata = {"title": title, "artist": tags.get("artist"), "album": tags.get("album")}

    parts: list[Part] = res["parts"]
    simplified = None
    if opts.simplify:
        mel = next((p for p in parts if p.role in ("melody", "vocals")), None) or \
              next((p for p in parts if p.role in ("piano", "bass", "other", "guitar")), None)
        simplified = simplify.simplify(mel, res["chords"], res["key"], res["timeline"],
                                       difficulty=opts.difficulty, transpose=opts.transpose,
                                       max_keys=opts.max_keys, low_conf=opts.low_confidence)
        parts = parts + simplified["parts"]

    log("write", f"song folder -> {song_dir}")
    song_json = write_song(song_dir, source=source, metadata=metadata, analyzer=analyzer_info,
                           rhythm=res["rhythm"], tl=res["timeline"], key=res["key"], chords=res["chords"],
                           sections=res["sections"], parts=parts, stems=res["stems"],
                           low_conf=opts.low_confidence, simplified=simplified)
    lyr = _lyrics_step(song_dir, opts)
    log("done", str(song_dir))
    event("done", song_dir=str(song_dir), skipped=False)
    return Result(song_dir, song_json, False, cache.hits, cache.misses, lyr)


def _lyrics_step(song_dir: Path, opts: Options):
    """lyrics.json next to song.json. Never fails the analysis."""
    if not opts.lyrics:
        return None
    from . import lyrics
    try:
        return lyrics.run_lyrics(song_dir, lyrics.LyricsOptions(
            online=opts.lyrics_online, cache_dir=opts.cache_dir, use_cache=opts.use_cache))
    except Exception as e:  # noqa: BLE001 - lyrics are a bonus; the song folder is already written
        log("lyrics", f"skipped: {e}")
        return None


# ============================================================================ audio

def _analyze_audio(path: Path, opts: Options, cache: Cache, info: dict) -> dict:
    device = backends.resolve_device(opts.device)
    info["device"] = device
    log("decode", "decoding audio")
    mix = load_audio(path, SR)
    duration = mix.shape[1] / SR
    mix_mono = to_mono(mix)

    # ---- stems
    stem_paths: dict[str, Path] = {}
    if opts.separate and backends.has_demucs():
        model = opts.stems_model or (stems.SIX_STEM_MODEL if opts.instrument == "piano" else stems.DEFAULT_MODEL)
        d, done = cache.directory("stems", {"v": stems.VERSION, "model": model, "shifts": opts.shifts})
        if not done:
            sep = stems.separate(mix, SR, model, device, shifts=opts.shifts)
            for name, y in sep.items():
                write_audio(d / f"{name}.flac", y, SR)
            Cache.mark_complete(d)
        stem_paths = {p.stem: p for p in sorted(d.glob("*.flac"))}
        info["models"]["stems"] = model
    else:
        if opts.separate:
            log("stems", "Demucs not installed: analysing the full mix (run setup.sh for stems)")
        info["models"]["stems"] = None
    stem_key = info["models"]["stems"] or "none"

    @lru_cache(maxsize=None)
    def stem_mono(name: str, sr: int) -> np.ndarray:
        if name == "mix" or name not in stem_paths:
            return resample(mix_mono, SR, sr) if sr != SR else mix_mono
        y, ysr = read_audio(stem_paths[name])
        return resample(to_mono(y), ysr, sr) if ysr != sr else to_mono(y)

    # ---- features
    def compute_features():
        log("features", "chroma, onsets, timbre")
        fs = features.FEAT_SR
        if stem_paths:
            harm_names = [n for n in stem_paths if n not in ("drums", "vocals")]
            harmonic = sum(stem_mono(n, fs) for n in harm_names)
            bass = stem_mono("bass", fs) if "bass" in stem_paths else None
            drums = stem_mono("drums", fs) if "drums" in stem_paths else None
        else:
            import librosa
            harmonic = librosa.effects.harmonic(stem_mono("mix", fs), margin=2.0)
            bass, drums = None, None
        return features.compute(harmonic, stem_mono("mix", fs), bass, drums)

    feat_params = {"v": features.VERSION, "stems": stem_key}
    feats = cache.arrays("features", feat_params, compute_features)

    rh = cache.json("rhythm", {"v": rhythm.VERSION, "features": feat_params}, lambda: _log_call("rhythm", "tempo, beats, bars", rhythm.analyze, feats, duration))
    tl = Timeline(rh["beats"], rh["downbeats"], rh["beats_per_bar"])
    log("rhythm", f"{rh['bpm']:.1f} BPM, {rh['beats_per_bar']}/4")

    def compute_chords():
        log("chords", f"chord recognition ({opts.chord_vocab})")
        cs = chordmod.detect(feats["chroma"], feats["bass_chroma"] if stem_paths else None, feats["times"],
                             rh["beats"], duration, energy=feats["rms"], vocab=opts.chord_vocab)
        return [c.to_cache() for c in cs]

    chords = [Chord.from_cache(c) for c in cache.json(
        "chords", {"v": chordmod.VERSION, "vocab": opts.chord_vocab, "rhythm": rhythm.VERSION, "features": feat_params}, compute_chords)]

    def compute_key():
        hist = keydetect.histogram_from_chroma(feats["chroma"], feats["bass_chroma"])
        return keydetect.detect(hist, chords).to_cache()

    key = KeyInfo.from_cache(cache.json("key", {"v": keydetect.VERSION, "chords": chordmod.VERSION, "rhythm": rhythm.VERSION,
                                                "vocab": opts.chord_vocab, "features": feat_params}, compute_key))
    log("key", f"{key.to_dict()['name']} (confidence {key.confidence:.2f})")

    secs = sections.from_cache(cache.json(
        "sections", {"v": sections.VERSION, "rhythm": rhythm.VERSION, "features": feat_params},
        lambda: sections.to_cache(sections.segment(feats, tl, duration))))
    log("sections", " ".join(f"{s.label}({s.name})" for s in secs))

    # ---- note parts
    parts = _audio_parts(opts, cache, stem_paths, stem_mono, chords, device, info, stem_key)

    return {"duration": duration, "rhythm": rh, "timeline": tl, "key": key, "chords": chords,
            "sections": secs, "parts": parts, "stems": stem_paths}


def _log_call(stage, msg, fn, *a):
    log(stage, msg)
    return fn(*a)


def _cached_notes(cache: Cache, stage: str, params: dict, compute) -> tuple[list[Note], str]:
    def run():
        notes, method = compute()
        return {"method": method, "notes": [[n.pitch, n.start, n.end, n.velocity, n.confidence] for n in notes]}
    d = cache.json(stage, params, run)
    return [Note(int(p), s, e, int(v), c) for p, s, e, v, c in d["notes"]], d["method"]


def _audio_parts(opts, cache, stem_paths, stem_mono, chords, device, info, stem_key) -> list[Part]:
    parts: list[Part] = []
    has = lambda n: n in stem_paths
    want_melody = opts.mode in ("melody", "chords+melody", "simplified")
    want_chords = opts.mode in ("chords", "chords+melody", "simplified")
    pitch_params = {"pitch_v": melody.VERSION, "method": opts.pitch_method, "crepe": opts.crepe_model,
                    "stems": stem_key, "crepe_available": backends.has_crepe()}

    if want_melody:
        src = "vocals" if has("vocals") else "mix"

        def run():
            if src == "mix" and backends.has_basic_pitch():
                # No vocal stem: a monophonic tracker hears everything at once.
                # Transcribe the whole mix and keep its top line instead.
                log("melody", "no vocal stem: Basic Pitch on the mix, keeping the top line")
                poly = transcribe.basic_pitch_notes(stem_mono("mix", 22050), 22050, "mix")
                notes = [n for n in simplify.monophonic(poly, merge_gap=0.03) if n.pitch >= 55]
                for n in notes:
                    n.confidence *= 0.7
                return notes, "basic-pitch-topline"
            y = stem_mono(src, SR)
            notes, method, _ = melody.notes_from_audio(y, SR, fmin=65.0, fmax=1400.0, method=opts.pitch_method,
                                                       crepe_model=opts.crepe_model, device=device,
                                                       voicing_threshold=0.3 if src != "mix" else 0.1)
            if src == "mix":
                for n in notes:
                    n.confidence *= 0.7
            return notes, method
        log("melody", f"melody from the {src} {'stem' if src != 'mix' else '(no stems)'}")
        notes, method = _cached_notes(cache, "notes-melody", {**pitch_params, "src": src,
                                                              "bp": backends.has_basic_pitch()}, run)
        info["models"]["melody"] = method
        parts.append(Part("melody", "Melody", "melody", notes, method, src, program=0))

    if want_chords:
        parts.append(Part("chords", "Chords", "chords", chordmod.voice_chords(chords), "chord-voicing", None, program=0))

    if opts.mode == "instrument":
        parts.append(_instrument_part(opts.instrument, opts, cache, stem_paths, stem_mono, device, info, stem_key, pitch_params))

    if opts.mode == "full":
        names = [n for n in stem_paths] or ["mix"]
        bp = backends.has_basic_pitch()
        if not bp:
            log("full", "basic-pitch not installed: using the monophonic pitch tracker per stem")
        programs = {"vocals": 54, "bass": 33, "other": 0, "piano": 0, "guitar": 25, "mix": 0}
        for name in names:
            if name == "drums":
                continue
            def run(name=name):
                y = stem_mono(name, 22050)
                if bp:
                    log("full", f"Basic Pitch on {name}")
                    return transcribe.basic_pitch_notes(y, 22050, name), "basic-pitch"
                lo, hi = transcribe.STEM_RANGES.get(name, (40.0, 2000.0))
                ns, m, _ = melody.notes_from_audio(y, 22050, fmin=max(lo, 32.0), fmax=min(hi, 1900.0),
                                                   method=opts.pitch_method, crepe_model=opts.crepe_model, device=device)
                return ns, m
            notes, method = _cached_notes(cache, f"notes-full-{name}", {"v": transcribe.VERSION, "bp": bp,
                                                                       **pitch_params, "stem": name}, run)
            role = name if name in ("vocals", "bass", "piano", "guitar", "other") else "other"
            parts.append(Part(name, name.capitalize() if name != "mix" else "Everything", role, notes, method,
                              name, program=programs.get(name, 0)))
            info["models"][f"full-{name}"] = method
        parts.append(_instrument_part("drums", opts, cache, stem_paths, stem_mono, device, info, stem_key, pitch_params))
    return parts


def _instrument_part(inst, opts, cache, stem_paths, stem_mono, device, info, stem_key, pitch_params) -> Part:
    has = lambda n: n in stem_paths
    if inst == "drums":
        src = "drums" if has("drums") else "mix"

        def run():
            y = stem_mono(src, 22050)
            if src == "mix":
                import librosa
                y = librosa.effects.percussive(y, margin=2.0)
            log("drums", f"drum hits from the {src}")
            return transcribe.drum_notes(y, 22050, from_mix=src == "mix"), "band-onsets"
        notes, method = _cached_notes(cache, "notes-drums", {"v": transcribe.VERSION, "stems": stem_key, "src": src}, run)
        info["models"]["drums"] = method
        return Part("drums", "Drums", "drums", notes, method, src, is_drum=True, program=0)

    if inst == "bass":
        src = "bass" if has("bass") else "mix"

        def run():
            y = stem_mono(src, SR)
            if src == "mix":
                from scipy.signal import butter, sosfiltfilt
                y = sosfiltfilt(butter(4, 300, "low", fs=SR, output="sos"), y).astype(np.float32)
            log("bass", f"bass line from the {src}")
            notes, method, _ = melody.notes_from_audio(y, SR, fmin=32.0, fmax=400.0, method=opts.pitch_method,
                                                       crepe_model=opts.crepe_model, device=device, voicing_threshold=0.25)
            if src == "mix":
                for n in notes:
                    n.confidence *= 0.7
            return notes, method
        notes, method = _cached_notes(cache, "notes-bass", {**pitch_params, "src": src}, run)
        info["models"]["bass"] = method
        return Part("bass", "Bass", "bass", notes, method, src, program=33)

    # piano
    src = "piano" if has("piano") else "mix"
    use_model = backends.has_piano_model()

    def run():
        y = stem_mono(src, SR)
        if use_model:
            return transcribe.piano_notes(y, SR, device), "bytedance-piano-transcription"
        if backends.has_basic_pitch():
            log("piano", "piano model not installed: using Basic Pitch")
            return transcribe.basic_pitch_notes(to_22k(y), 22050, "piano"), "basic-pitch"
        raise RuntimeError("piano mode needs piano_transcription_inference or basic-pitch (run setup.sh)")

    def to_22k(y):
        return resample(y, SR, 22050)

    if src == "mix" and stem_paths:
        log("piano", "no piano stem (use --stems-model htdemucs_6s): transcribing the full mix")
    notes, method = _cached_notes(cache, "notes-piano", {"v": transcribe.VERSION, "stems": stem_key, "src": src,
                                                        "model": use_model}, run)
    info["models"]["piano"] = method
    return Part("piano", "Piano", "piano", notes, method, src, program=0)


# ============================================================================ MIDI

def _analyze_midi(path: Path, opts: Options, info: dict) -> dict:
    from .midi_input import MidiSong

    log("midi", "reading MIDI file")
    song = MidiSong(path)
    rh, tl, chords, key, secs = song.analyze(opts.chord_vocab)
    info["models"] = {"stems": None, "notes": "midi"}
    log("rhythm", f"{rh['bpm']:.1f} BPM, {rh['beats_per_bar']}/4")
    log("key", key.to_dict()["name"])
    parts: list[Part] = []
    mode = opts.mode
    if mode in ("melody", "chords+melody", "simplified"):
        inst = song.pick("melody")
        if inst is not None:
            parts.append(song.part_for(inst, "melody"))
    if mode in ("chords", "chords+melody", "simplified"):
        parts.append(Part("chords", "Chords", "chords", chordmod.voice_chords(chords), "chord-voicing", None))
    if mode == "instrument":
        inst = song.pick(opts.instrument)
        if inst is None:
            raise ValueError(f"no {opts.instrument} track found in {path.name}")
        parts.append(song.part_for(inst, opts.instrument))
    if mode == "full":
        used = set()
        lead = song.pick("melody")
        for i, inst in enumerate(song.instruments):
            role = "drums" if inst.is_drum else ("melody" if inst is lead else
                                                 "bass" if 32 <= inst.program <= 39 else
                                                 "piano" if inst.program <= 7 else "other")
            pid = part_id(inst.name or role)
            while pid in used:
                pid = f"{pid}-{i}"
            used.add(pid)
            parts.append(song.part_for(inst, role, pid, mono=False))
    return {"duration": song.duration, "rhythm": rh, "timeline": tl, "key": key, "chords": chords,
            "sections": secs, "parts": parts, "stems": {}}
