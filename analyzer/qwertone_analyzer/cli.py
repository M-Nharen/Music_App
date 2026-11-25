"""Command line: `qwertone-analyze song.mp3 --mode chords+melody`."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

from . import __version__, backends
from .cache import cache_size, clear_cache, default_cache_root
from .pipeline import INSTRUMENTS, Options, analyze
from .progress import configure, event

SUBCOMMANDS = {"analyze", "doctor", "cache", "download-models", "-h", "--help", "--version"}

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

    g = p.add_argument_group("output")
    g.add_argument("-q", "--quiet", action="store_true")
    g.add_argument("--json-progress", action="store_true", help="progress as JSON lines on stderr (for the app)")
    return p


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    if argv and argv[0] not in SUBCOMMANDS:
        argv = ["analyze", *argv]  # `qwertone-analyze song.mp3` works too

    ap = argparse.ArgumentParser(prog="qwertone-analyze",
                                 description="Turn a song (audio or MIDI) into a Qwertone song folder.")
    ap.add_argument("--version", action="version", version=f"%(prog)s {__version__}")
    sub = ap.add_subparsers(dest="cmd", required=True)
    _analyze_parser(sub)
    sub.add_parser("doctor", help="show which engines are installed and whether a GPU is used")
    c = sub.add_parser("cache", help="show or clear the analysis cache")
    c.add_argument("action", choices=("info", "clear"))
    c.add_argument("--cache-dir")
    dm = sub.add_parser("download-models", help="fetch model weights now instead of on first use")
    dm.add_argument("--six-stems", action="store_true", help="also htdemucs_6s (piano/guitar stems)")
    dm.add_argument("--no-piano", action="store_true")
    args = ap.parse_args(argv)

    if args.cmd == "doctor":
        for k, v in backends.status().items():
            print(f"{k:<22} {v}")
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
    print("CREPE and Basic Pitch weights ship inside their packages.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
