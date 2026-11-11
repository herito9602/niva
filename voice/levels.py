"""Band levels of a slice of Niva's voice, for the notch's waveform (design L12).

Pure: samples in, seven integers out. A Hann window, a real FFT, then the RMS
of seven log-spaced bands between 80 Hz and 8 kHz, in dBFS, mapped from
[-60, -10] dBFS to 0-100. Only Niva's own voice is ever analysed; nothing
that came from the microphone goes through here.
"""

from __future__ import annotations

import math

import numpy as np

BAND_EDGES_HZ = (80, 160, 320, 640, 1280, 2560, 4000, 8000)
BAND_COUNT = len(BAND_EDGES_HZ) - 1
DB_FLOOR = -60.0
DB_TOP = -10.0

_SILENT = [0] * BAND_COUNT


def level_from_db(db: float) -> int:
    """-60 dBFS and below is 0, -10 dBFS and above is 100, linear in between."""
    if not math.isfinite(db) and db < 0:
        return 0
    scaled = (db - DB_FLOOR) / (DB_TOP - DB_FLOOR) * 100.0
    return int(min(100, max(0, round(scaled))))


def bands(pcm: bytes | np.ndarray, rate: int) -> list[int]:
    """Seven 0-100 levels for one slice of int16 mono PCM. Never raises: a slice
    too short to analyse (under two samples) is all zeros."""
    samples = np.frombuffer(pcm[: len(pcm) // 2 * 2], dtype=np.int16) if isinstance(pcm, bytes) else np.asarray(pcm)
    if samples.size < 2 or rate <= 0:
        return list(_SILENT)
    x = samples.astype(np.float64) / 32768.0
    window = np.hanning(x.size)
    spectrum = np.fft.rfft(x * window)
    # A sine of amplitude A shows 2|X|/sum(w) in its bin; its power is A^2 / 2.
    power = (2.0 * np.abs(spectrum) / window.sum()) ** 2 / 2.0
    freqs = np.fft.rfftfreq(x.size, 1.0 / rate)
    out = []
    for low, high in zip(BAND_EDGES_HZ, BAND_EDGES_HZ[1:], strict=False):
        band = power[(freqs >= low) & (freqs < high)]
        rms = math.sqrt(float(band.sum()))
        out.append(level_from_db(20.0 * math.log10(rms) if rms > 0 else float("-inf")))
    return out
