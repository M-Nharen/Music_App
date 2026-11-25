"""Source separation with Demucs."""

from __future__ import annotations

import numpy as np

from .progress import log

VERSION = 1
DEFAULT_MODEL = "htdemucs"      # drums, bass, other, vocals
SIX_STEM_MODEL = "htdemucs_6s"  # + guitar, piano


def load_model(name: str):
    """Kept as a module-level function so tests can swap in an untrained model."""
    from demucs.pretrained import get_model
    return get_model(name)


def separate(mix: np.ndarray, sr: int, model_name: str, device: str, *, shifts: int = 0,
             segment: float | None = None) -> dict[str, np.ndarray]:
    """mix: (2, samples) float at `sr`. Returns {stem name: (2, samples)} at `sr`."""
    import torch
    from demucs.apply import apply_model

    from .audio_io import resample

    model = load_model(model_name)
    model.eval()
    msr = int(getattr(model, "samplerate", 44100))
    x = resample(mix, sr, msr) if msr != sr else mix
    wav = torch.from_numpy(np.ascontiguousarray(x, dtype=np.float32))
    ref = wav.mean(0)
    mean, std = ref.mean(), ref.std() + 1e-8
    wav = (wav - mean) / std
    threads = torch.get_num_threads()
    log("stems", f"separating with {model_name} on {device} ({threads} CPU threads)" if device == "cpu"
        else f"separating with {model_name} on {device}")
    kwargs = dict(shifts=shifts, split=True, overlap=0.25, progress=False, device=device, num_workers=0)
    if segment:
        kwargs["segment"] = segment
    with torch.no_grad():
        out = apply_model(model, wav[None], **kwargs)[0]
    out = (out * std + mean).cpu().numpy()
    stems = {}
    for i, name in enumerate(model.sources):
        s = out[i]
        stems[name] = resample(s, msr, sr) if msr != sr else s
        n = mix.shape[1]
        stems[name] = stems[name][:, :n] if stems[name].shape[1] >= n else np.pad(stems[name], ((0, 0), (0, n - stems[name].shape[1])))
    return stems
