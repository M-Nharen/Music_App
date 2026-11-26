#!/usr/bin/env bash
# Set up the Qwertone song analyzer on Arch Linux.
#
#   ./setup.sh              CPU-only (works everywhere, ~1.5 GB)
#   ./setup.sh --gpu        NVIDIA GPU build of PyTorch (~5 GB, much faster stems)
#   ./setup.sh --light      no machine-learning models (chords/key/tempo only,
#                           pYIN melody, no stems) - ~200 MB
#
# Other flags:
#   --torch-index URL   install PyTorch from this index (e.g. a ROCm one for AMD GPUs)
#   --dev               also install pytest + jsonschema to run the tests
#   --no-download       don't fetch model weights now (they download on first use)
#   --no-pacman         skip the system packages (not on Arch, or already installed)
#   --no-link           don't put qwertone-analyze in ~/.local/bin
#
# Everything goes into analyzer/.venv; nothing is installed system-wide except
# the pacman packages. Delete .venv to undo.
set -euo pipefail

say() { printf '\n\033[1;36m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

cd "$(dirname "$0")"
HERE="$(pwd)"
VENV="$HERE/.venv"
PY_VERSION="3.12"   # PyTorch, Demucs and friends are safest here
TORCH_INDEX="https://download.pytorch.org/whl/cpu"
MODELS=1 DEV=0 DOWNLOAD=1 PACMAN=1 LINK=1

while [[ $# -gt 0 ]]; do
    case "$1" in
        --gpu) TORCH_INDEX="" ;;                       # default PyPI wheels = CUDA on Linux
        --torch-index) TORCH_INDEX="${2:?}"; shift ;;
        --light) MODELS=0 ;;
        --dev) DEV=1 ;;
        --no-download) DOWNLOAD=0 ;;
        --no-pacman) PACMAN=0 ;;
        --no-link) LINK=0 ;;
        -h|--help) sed -n '2,19p' "$0"; exit 0 ;;
        *) die "unknown option $1 (see --help)" ;;
    esac
    shift
done

if [[ $PACMAN -eq 1 ]]; then
    if command -v pacman >/dev/null; then
        say "Installing system packages (needs sudo): ffmpeg uv"
        sudo pacman -S --needed ffmpeg uv
    else
        echo "pacman not found: make sure ffmpeg and uv (https://docs.astral.sh/uv/) are installed."
    fi
fi
command -v uv >/dev/null || die "uv is missing (sudo pacman -S uv)"
command -v ffmpeg >/dev/null || echo "warning: ffmpeg missing - only WAV/FLAC input will work"

say "Creating $VENV with Python $PY_VERSION (uv downloads it if needed)"
[[ -d "$VENV" ]] || uv venv --python "$PY_VERSION" "$VENV"
export VIRTUAL_ENV="$VENV"

if [[ $MODELS -eq 1 ]]; then
    if [[ -n "$TORCH_INDEX" ]]; then
        say "Installing PyTorch from $TORCH_INDEX"
        uv pip install torch torchaudio --index-url "$TORCH_INDEX"
    else
        say "Installing PyTorch (CUDA build from PyPI)"
        uv pip install torch torchaudio
    fi
    EXTRAS="models"
    [[ $DEV -eq 1 ]] && EXTRAS="models,dev"
    say "Installing the analyzer and its engines"
    uv pip install -e ".[$EXTRAS]"
    # basic-pitch's metadata pins TensorFlow (no wheels for new Pythons);
    # we run its bundled ONNX model with onnxruntime instead.
    uv pip install --no-deps "basic-pitch==0.4.0"
else
    say "Installing the analyzer (light: no ML engines)"
    if [[ $DEV -eq 1 ]]; then uv pip install -e ".[dev]"; else uv pip install -e .; fi
fi

if [[ $LINK -eq 1 ]]; then
    mkdir -p "$HOME/.local/bin"
    ln -sf "$VENV/bin/qwertone-analyze" "$HOME/.local/bin/qwertone-analyze"
    echo "Linked ~/.local/bin/qwertone-analyze"
    case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "(add ~/.local/bin to your PATH to use it from anywhere)";; esac
fi

if [[ $MODELS -eq 1 && $DOWNLOAD -eq 1 ]]; then
    say "Downloading model weights (Demucs ~80 MB, piano model ~165 MB)"
    "$VENV/bin/qwertone-analyze" download-models || echo "warning: download failed; weights will download on first use"
fi

say "Done. Engines:"
"$VENV/bin/qwertone-analyze" doctor
echo
echo "Try:  qwertone-analyze ~/Music/song.mp3"
[[ $DEV -eq 1 ]] && echo "Tests: cd analyzer && .venv/bin/python -m pytest"
true
