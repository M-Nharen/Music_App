"""Command line: `qwertone-analyze song.mp3 --mode chords+melody`."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from . import __version__, backends
from .cache import cache_size, clear_cache, default_cache_root
from .pipeline import INSTRUMENTS, Options, analyze
from .progress import configure, event

SUBCOMMANDS = {"analyze", "lyrics", "import-lrc", "doctor", "cache", "download-models", "-h", "--help", "--version"}

MODE_HELP = """\
modes:
  chords          chord symbols + a playable chords part
  melody          the sung melody (CREPE on the vocal stem)
  chords+melody   both (default)
  full            every stem transcribed with Basic Pitch, plus drums
  bass | piano | drums
                  one instrument (same as --mode instrument --instrument X)
  simplified      chords+melody made easy: quantised, fitted to the keyboard,
                  at most 2-3 keys at once, moved to an easy key
"""


def _analyze_parser(sub) -> argparse.ArgumentParser:
    p = sub.add_parser("analyze", help="analyse an audio or MIDI file", epilog=MODE_HELP,
                       formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("input", type=Path, help="mp3 / flac / wav / m4a / ogg, or .mid")
    p.add_argument("-m", "--mode", default="chords+melody",
                   help="chords | melody | chords+melody | full | instrument | bass | piano | drums | simplified")
    p.add_argument("-i", "--instrument", choices=INSTRUMENTS, help="for --mode instrument")
    p.add_argument("-o", "--out", dest="out_dir", help="where song folders go (default ~/.local/share/qwertone/songs)")

    g = p.add_argument_group("simplify")
    g.add_argument("--simplify", action="store_true", help="also add easy versions (any mode)")
    g.add_argument("-d", "--difficulty", type=int, default=2, choices=range(1, 6), metavar="1-5",
                   help="1 = one key at a time on quarter notes ... 5 = close to the original (default 2)")
    g.add_argument("-t", "--transpose", default="auto",
                   help="'auto' (easiest key), 'none', or semitones like -2 (default auto)")
    g.add_argument("--max-keys", type=int, choices=(1, 2, 3), help="most keys held at once (default from difficulty)")

    g = p.add_argument_group("engines")
    g.add_argument("--no-separate", dest="separate", action="store_false", help="skip Demucs; analyse the mix")
    g.add_argument("--stems-model", help="Demucs model (htdemucs, htdemucs_ft, htdemucs_6s, mdx_extra...)")
    g.add_argument("--shifts", type=int, default=0, help="Demucs random shifts: better stems, N times slower")
    g.add_argument("--device", default="auto", help="auto | cpu | cuda (default auto)")
    g.add_argument("--crepe-model", default="auto", help="auto | tiny | small | medium | large | full")
    g.add_argument("--pitch", dest="pitch_method", default="auto", choices=("auto", "crepe", "pyin"))
    g.add_argument("--chords", dest="chord_vocab", default="extended", choices=("basic", "extended"),
                   help="basic = major/minor only")
    g.add_argument("--low-confidence", type=float, default=0.5, help="notes below this are flagged (default 0.5)")

    g = p.add_argument_group("cache")
    g.add_argument("--cache-dir", help=f"default {default_cache_root()}")
    g.add_argument("--no-cache", dest="use_cache", action="store_false")
    g.add_argument("--force", action="store_true", help="redo everything even if cached")

    g = p.add_argument_group("lyrics")
    g.add_argument("--no-lyrics", dest="lyrics", action="store_false", help="skip the lyrics step")
    g.add_argument("--offline", dest="lyrics_online", action="store_false",
                   help="don't go online: no LRCLIB / AcoustID, models only if already downloaded")
    g.add_argument("--light", dest="lyrics_models", action="store_false",
                   help="lyrics without models: no Whisper transcription/alignment, no translation")
    g.add_argument("--lyrics-language", help="the song's language if detection gets it wrong: ta, te, hi, en, "
                                             "ta-Latn (Tanglish) ... (default: Whisper / the lyrics decide)")
    g.add_argument("--whisper-model", default="auto",
                   help="auto (= large-v3-turbo) | small | medium | large-v3 ... (smaller = faster, rougher)")
    g.add_argument("--no-translate", dest="translate", action="store_false", help="no English translation")
    g.add_argument("--acoustid", default="auto", choices=("auto", "always", "never"),
                   help="identify the song by its sound: auto = only files without title/artist tags")
    g.add_argument("--acoustid-key", help="AcoustID API key (or ACOUSTID_API_KEY, or ~/.config/qwertone/acoustid_key)")

    g = p.add_argument_group("output")
    g.add_argument("-q", "--quiet", action="store_true")
    g.add_argument("--json-progress", action="store_true", help="progress as JSON lines on stderr (for the app)")
    return p


def _lyrics_common(p: argparse.ArgumentParser) -> None:
    p.add_argument("--offline", action="store_true",
                   help="don't go online: no LRCLIB / AcoustID / model downloads (cached answers are still used)")
    p.add_argument("--refresh", action="store_true", help="ask LRCLIB / AcoustID again instead of using cached answers")
    p.add_argument("--language", help="the song's language if detection gets it wrong (also tells Whisper): "
                                      "ta, te, hi, en, ta-Latn (Tanglish), hi-Latn (Hinglish) ...")
    p.add_argument("--light", action="store_true",
                   help="no models: skip Whisper transcription / alignment and translation")
    p.add_argument("--whisper-model", default="auto",
                   help="auto (= large-v3-turbo) | small | medium | large-v3 ... (smaller = faster, rougher)")
    p.add_argument("--device", default="auto", help="auto | cpu | cuda (Whisper and the translator)")
    p.add_argument("--no-translate", dest="translate", action="store_false", help="no English translation")
    p.add_argument("--acoustid", default="auto", choices=("auto", "always", "never"),
                   help="identify the song by its sound: auto = only files without title/artist tags")
    p.add_argument("--acoustid-key", help="AcoustID API key (or ACOUSTID_API_KEY, or ~/.config/qwertone/acoustid_key)")
    p.add_argument("--title", help="look up this title instead of the one in song.json")
    p.add_argument("--artist")
    p.add_argument("--album")
    p.add_argument("-o", "--out", dest="out_dir", help="where song folders are (to find SONG by its audio file)")
    p.add_argument("--cache-dir", help=f"default {default_cache_root()}")
    p.add_argument("--no-cache", dest="use_cache", action="store_false")
    p.add_argument("--show", action="store_true", help="print the lines (romanised + English) afterwards")
    p.add_argument("-q", "--quiet", action="store_true")
    p.add_argument("--json-progress", action="store_true")


def _lyrics_parsers(sub) -> None:
    p = sub.add_parser("lyrics", help="(re)make lyrics.json for analysed songs, without re-analysing",
                       description="Re-run only the lyrics step. SONG is a song folder, its song.json, "
                                   "or the audio/MIDI file you analysed.")
    p.add_argument("songs", nargs="*", metavar="SONG")
    p.add_argument("--all", action="store_true", help="every song folder in the songs directory")
    p.add_argument("--force", action="store_true", help="rebuild even if lyrics.json is up to date")
    _lyrics_common(p)
    p = sub.add_parser("import-lrc", help="use your own .lrc for a song (wins over every other source)")
    p.add_argument("lrc", type=Path, help="the .lrc (or plain .txt) file")
    p.add_argument("song", metavar="SONG", help="song folder, song.json, or the analysed audio/MIDI file")
    _lyrics_common(p)


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] not in SUBCOMMANDS:
        argv = ["analyze", *argv]  # `qwertone-analyze song.mp3` works too

    ap = argparse.ArgumentParser(prog="qwertone-analyze",
                                 description="Turn a song (audio or MIDI) into a Qwertone song folder.")
    ap.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    sub = ap.add_subparsers(dest="cmd", required=True)
    _analyze_parser(sub)
    _lyrics_parsers(sub)
    sub.add_parser("doctor", help="show which engines are installed and whether a GPU is used")
    c = sub.add_parser("cache", help="show or clear the analysis cache")
    c.add_argument("action", choices=("info", "clear"))
    c.add_argument("--cache-dir")
    dm = sub.add_parser("download-models", help="fetch model weights now instead of on first use")
    dm.add_argument("--six-stems", action="store_true", help="also htdemucs_6s (piano/guitar stems)")
    dm.add_argument("--no-piano", action="store_true")
    dm.add_argument("--no-whisper", action="store_true", help="skip the Whisper lyrics model")
    dm.add_argument("--whisper-model", default="auto", help="which Whisper model (default large-v3-turbo)")
    dm.add_argument("--no-translate", action="store_true", help="skip the translation model")
    args = ap.parse_args(argv)

    if args.cmd == "doctor":
        for k, v in backends.status().items():
            print(f"{k:<25} {v}")
        return 0

    if args.cmd == "cache":
        root = Path(args.cache_dir).expanduser() if args.cache_dir else default_cache_root()
        if args.action == "clear":
            clear_cache(root)
            print(f"cleared {root}")
        else:
            songs, size = cache_size(root)
            print(f"{root}: {songs} songs, {size / 1e6:.1f} MB")
        return 0

    if args.cmd == "download-models":
        return _download(args)

    if args.cmd in ("lyrics", "import-lrc"):
        return _lyrics_cmd(args)

    configure(json_lines=args.json_progress, quiet=args.quiet)
    opts = Options(**{k: v for k, v in vars(args).items() if k in Options.__dataclass_fields__})
    try:
        res = analyze(args.input, opts)
    except (ValueError, FileNotFoundError, RuntimeError) as e:
        event("error", message=str(e))
        print(f"error: {e}", file=sys.stderr)
        return 2
    print(res.song_dir)
    return 0


def _lyrics_cmd(args) -> int:
    from . import lyrics
    from .writer import default_songs_dir

    configure(json_lines=args.json_progress, quiet=args.quiet)
    songs_dir = Path(args.out_dir).expanduser() if args.out_dir else default_songs_dir()

    def options(force: bool = False) -> "lyrics.LyricsOptions":
        return lyrics.LyricsOptions(online=not args.offline, refresh=args.refresh, force=force,
                                    language=args.language, title=args.title, artist=args.artist,
                                    album=args.album, cache_dir=args.cache_dir, use_cache=args.use_cache,
                                    models=not args.light, whisper_model=args.whisper_model, device=args.device,
                                    translate=args.translate, acoustid=args.acoustid, acoustid_key=args.acoustid_key)
    try:
        if args.cmd == "import-lrc":
            if not args.lrc.exists():
                raise FileNotFoundError(args.lrc)
            d = lyrics.find_song_dir(args.song, songs_dir)
            results = [(d, lyrics.import_lrc(args.lrc, d, options()))]
        else:
            targets = [Path(s) for s in args.songs]
            if args.all:
                targets += sorted(p.parent for p in songs_dir.glob("*/song.json"))
            if not targets:
                raise ValueError("give a SONG (folder, song.json or audio file) or --all")
            if args.title and len(targets) > 1:
                raise ValueError("--title/--artist/--album only make sense for one song")
            results = []
            for t in targets:
                d = lyrics.find_song_dir(t, songs_dir)
                results.append((d, lyrics.run_lyrics(d, options(args.force))))
    except (ValueError, FileNotFoundError, OSError) as e:
        event("error", message=str(e))
        print(f"error: {e}", file=sys.stderr)
        return 2
    for d, r in results:
        extra = f"  ({r.words_source} words" + (f", {r.timing_source} timing)" if r.timing_source else ")") \
            if r.words_source else ""
        lang = f", {r.language}" if r.language and r.lines else ""
        print(f"{r.path}  {r.status}, {r.lines} lines{lang}{extra}{'  [up to date]' if r.skipped else ''}")
        for n in r.notes:
            print(f"  note: {n}")
        if args.show:
            _show_lyrics(r.path)
    return 0


def _show_lyrics(path: Path) -> None:
    import json
    with open(path) as f:
        doc = json.load(f)
    for l in doc["lines"]:
        t = f"{l['start']:7.2f}" if l["start"] is not None else "      -"
        print(f"  {t}  {l['text']}   [{l.get('language')}]")
        if l.get("romanized") and l["romanized"] != l["text"]:
            print(f"           {l['romanized']}")
        if l.get("translation"):
            print(f"           = {l['translation']}")
    if any(l.get("translation") for l in doc["lines"]):
        print(f"  (English: {doc['sources']['translation']['name']})")
    if any((doc.get("auto_generated") or {}).values()):
        gen = [k for k, v in doc["auto_generated"].items() if v]
        print(f"  (auto-generated: {', '.join(gen)})")


def _download(args) -> int:
    configure()
    if backends.has_demucs():
        from . import stems
        names = [stems.DEFAULT_MODEL] + ([stems.SIX_STEM_MODEL] if args.six_stems else [])
        for n in names:
            print(f"demucs {n} ...", flush=True)
            stems.load_model(n)
    else:
        print("demucs not installed, skipping")
    if backends.has_piano_model() and not args.no_piano:
        print("piano transcription model ...", flush=True)
        backends.ensure_piano_checkpoint()
    from . import asr, translate
    if asr.available() and not args.no_whisper:
        name = asr.model_name(args.whisper_model)
        print(f"whisper {name} ...", flush=True)
        try:
            asr.load_model(name, "cpu")
        except asr.AsrUnavailable as e:
            print(f"  failed: {e}")
    elif not args.no_whisper:
        print("faster-whisper not installed, skipping")
    if translate.available() and not args.no_translate:
        print(f"translator {translate.model_label(translate.DEFAULT_MODEL)} ...", flush=True)
        try:
            translate.load(translate.DEFAULT_MODEL, "cpu")
        except translate.TranslatorUnavailable as e:
            print(f"  failed: {e}")
    elif not args.no_translate:
        print("ctranslate2/sentencepiece not installed, skipping the translator")
    print("CREPE and Basic Pitch weights ship inside their packages.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
