import shutil
import subprocess

import numpy as np
import pytest

from conftest import load_song, run
from qwertone_analyzer.audio_io import SR, load_audio

needs_ffmpeg = pytest.mark.skipif(shutil.which("ffmpeg") is None, reason="ffmpeg not installed")


@pytest.fixture(scope="module")
def encoded(clip, tmp_path_factory):
    d = tmp_path_factory.mktemp("enc")
    out = {"flac": clip["dir"] / "mix.flac"}
    if shutil.which("ffmpeg"):
        for ext, codec in (("mp3", ["-c:a", "libmp3lame", "-b:a", "192k"]), ("m4a", ["-c:a", "aac", "-b:a", "192k"])):
            p = d / f"mix.{ext}"
            subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", str(clip["mix"]), *codec, str(p)], check=True)
            out[ext] = p
    return out


@pytest.mark.parametrize("ext", ["flac", pytest.param("mp3", marks=needs_ffmpeg), pytest.param("m4a", marks=needs_ffmpeg)])
def test_decodes_every_format(encoded, clip, ext):
    y = load_audio(encoded[ext])
    ref = load_audio(clip["mix"])
    assert y.shape[0] == 2 and y.dtype == np.float32
    assert abs(y.shape[1] - ref.shape[1]) < 0.1 * SR  # mp3/aac add a little padding


@needs_ffmpeg
def test_mp3_gives_same_chords(encoded, clip, workdirs):
    song = load_song(run(encoded["mp3"], workdirs, subdir="mp3", mode="chords", separate=False))
    symbols = [c["symbol"] for c in song["chords"] if c["symbol"] != "N"]
    assert symbols == [c["symbol"] for c in clip["truth"]["chords"]]
