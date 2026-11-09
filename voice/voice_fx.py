"""Streaming "robot voice" effect for Niva's synthesized speech.

The owner picked a cute little robot for the mascot (variant "6 - Tierno" of the
throwaway prototype). The prototype processed a whole decoded clip; this module
does the same chain on audio that arrives in arbitrary chunks, so playback can
start at the first decoded block:

1. pitch up by ``PITCH_FACTOR`` (linear-interpolation resampler; also makes the
   speech 20 % faster, accepted by the owner),
2. "robotize": zero-phase STFT overlap-add with one frame every ``1/ROBOT_NOTE_HZ``
   seconds, which turns the voice into a flat buzzing note,
3. mix 70 % robot / 30 % pitched voice,
4. ring modulation (``RING_HZ``, ``RING_MIX``),
5. short comb filter (``COMB_MS``, ``COMB_FEEDBACK``),
6. fixed output gain and a soft limiter (the prototype peak-normalized the whole
   clip, which is impossible while streaming).

Everything is defined in Hz / ms, so any sample rate works. State (resampler
position, overlap-add buffers, ring phase, comb delay line) is carried between
``process`` calls, so the output does not depend on how the input is chunked
(up to float rounding) and has no clicks at chunk boundaries. ``flush`` emits the
tail at the end of an utterance and resets the state.

Added latency: the STFT needs one frame before the first sample is final, so the
output trails the input by ``n_fft - hop`` resampled samples (about 20 ms at
24 kHz). Nothing waits for the end of the utterance.

No per-sample Python loops: the resampler is a vectorized interpolation, the STFT
works on batches of frames, and the comb filter recurses over blocks of one
delay length.
"""

from __future__ import annotations

import math

import numpy as np

PITCH_FACTOR = 1.20
ROBOT_NOTE_HZ = 280.0
ROBOT_MIX = 0.70
RING_HZ = 90.0
RING_MIX = 0.12
COMB_MS = 4.0
COMB_FEEDBACK = 0.20
# STFT frame length. The approved prototype used 1024 samples on a clip that was
# decoded at 44.1 kHz, i.e. 23.2 ms; keeping the duration (not the sample count)
# keeps the timbre the owner chose at any sample rate.
FRAME_MS = 1024 / 44.1

# Gain constants, measured on a 5.9 s es-US-PalomaNeural clip decoded at 24 kHz
# (dry peak 0.656, dry RMS 0.1007):
# - ROBOT_PEAK_RATIO: peak of the robotized signal divided by the peak of the
#   pitched dry voice. The prototype normalized each branch by its own peak
#   before mixing; dividing the robot branch by this fixed ratio puts both
#   branches at the same level without looking at the whole clip.
ROBOT_PEAK_RATIO = 0.8102
# - OUTPUT_GAIN: scale applied after the comb filter so that the RMS of the
#   output equals the RMS of the dry voice on the same clip (1.74 on 0.0578 RMS
#   with unit gain gives 0.1006). The output peak lands at about 0.86, right at
#   the limiter knee.
OUTPUT_GAIN = 1.74
# Samples below this magnitude pass the limiter untouched; above it they are
# compressed smoothly towards 1.0.
LIMIT_KNEE = 0.85

_WINDOW_FLOOR = 1e-3  # same floor the prototype used when normalizing the overlap-add


def soft_limit(x: np.ndarray, knee: float = LIMIT_KNEE) -> np.ndarray:
    """Pointwise limiter: identity up to ``knee``, then a tanh curve that never exceeds 1."""
    mag = np.abs(x)
    over = mag > knee
    if not over.any():
        return x
    out = x.copy()
    room = 1.0 - knee
    out[over] = np.sign(x[over]) * (knee + room * np.tanh((mag[over] - knee) / room))
    return out


def _frame_size(sample_rate: int) -> int:
    return 2 * round(sample_rate * FRAME_MS / 2000.0)  # even, for irfft


class RobotVoice:
    """Stateful, chunk-size independent robot effect for one utterance at one sample rate."""

    def __init__(
        self,
        sample_rate: int,
        *,
        robot_peak_ratio: float = ROBOT_PEAK_RATIO,
        output_gain: float = OUTPUT_GAIN,
        robot_mix: float = ROBOT_MIX,
        ring_mix: float = RING_MIX,
        comb_feedback: float = COMB_FEEDBACK,
    ) -> None:
        self.sample_rate = sample_rate
        self._robot_gain = robot_mix / robot_peak_ratio
        self._voice_gain = 1.0 - robot_mix
        self._output_gain = output_gain
        self._ring_mix = ring_mix
        self._comb_fb = comb_feedback
        self._n_fft = _frame_size(sample_rate)
        self._hop = max(1, round(sample_rate / ROBOT_NOTE_HZ))
        self._window = np.hanning(self._n_fft).astype(np.float32)
        self._window_sq = self._window**2
        self._comb_delay = max(1, int(sample_rate * COMB_MS / 1000))
        self._ring_step = 2.0 * math.pi * RING_HZ / sample_rate
        self.reset()

    @property
    def latency_samples(self) -> int:
        """Samples the output trails the (resampled) input by."""
        return self._n_fft - self._hop

    def reset(self) -> None:
        """Forget everything: the next audio starts a new utterance."""
        # resampler: pending input from global index _rs_base, next output index _rs_k
        self._rs_hold = np.zeros(0, dtype=np.float32)
        self._rs_base = 0
        self._rs_k = 0
        # robotizer: resampled signal from global index _fr_base (= next frame start)
        self._fr_in = np.zeros(0, dtype=np.float32)
        self._fr_base = 0
        self._ola = np.zeros(self._n_fft, dtype=np.float32)
        self._ola_w = np.zeros(self._n_fft, dtype=np.float32)
        # post chain
        self._out_n = 0  # global index of the next output sample (ring phase)
        self._comb_hist = np.zeros(self._comb_delay, dtype=np.float32)

    # ------------------------------------------------------------ public API

    def process(self, chunk: np.ndarray) -> np.ndarray:
        """Feed float samples in [-1, 1]; returns the samples that became final (maybe none)."""
        x = np.asarray(chunk, dtype=np.float32)
        if x.size == 0:
            return np.zeros(0, dtype=np.float32)
        return self._finish(self._robotize(self._resample(x)))

    def flush(self) -> np.ndarray:
        """End of the utterance: emit the remaining tail and reset."""
        tail = self._resample_tail()
        out = self._robotize(tail, final=True)
        result = self._finish(out)
        self.reset()
        return result

    # ------------------------------------------------------------ stage 1

    def _interp(self, ext: np.ndarray, base: int, ks: np.ndarray) -> np.ndarray:
        pos = ks * PITCH_FACTOR
        i0g = np.floor(pos).astype(np.int64)
        frac = (pos - i0g).astype(np.float32)
        i0 = i0g - base
        return ext[i0] * (1.0 - frac) + ext[i0 + 1] * frac

    def _resample(self, x: np.ndarray) -> np.ndarray:
        ext = np.concatenate([self._rs_hold, x])
        base, length = self._rs_base, len(ext)
        # Output k needs inputs floor(k*f) and floor(k*f)+1, both already received.
        limit = base + length - 1
        k_end = int(math.ceil(limit / PITCH_FACTOR))
        ks = np.arange(self._rs_k, k_end, dtype=np.float64)
        ks = ks[ks * PITCH_FACTOR < limit]
        out = self._interp(ext, base, ks) if ks.size else np.zeros(0, dtype=np.float32)
        if ks.size:
            self._rs_k = int(ks[-1]) + 1
        keep = min(int(math.floor(self._rs_k * PITCH_FACTOR)) - base, length - 1)
        keep = max(keep, 0)
        self._rs_hold = ext[keep:]
        self._rs_base = base + keep
        return out

    def _resample_tail(self) -> np.ndarray:
        hold = self._rs_hold
        if hold.size == 0:
            return np.zeros(0, dtype=np.float32)
        total = self._rs_base + hold.size
        ext = np.concatenate([hold, hold[-1:]])  # clamp at the last sample, like np.interp
        k_end = int(math.ceil(total / PITCH_FACTOR))
        ks = np.arange(self._rs_k, k_end, dtype=np.float64)
        ks = ks[ks * PITCH_FACTOR < total]
        out = self._interp(ext, self._rs_base, ks) if ks.size else np.zeros(0, dtype=np.float32)
        self._rs_hold = np.zeros(0, dtype=np.float32)
        return out

    # ------------------------------------------------------------ stage 2

    def _robotize(self, pitched: np.ndarray, final: bool = False) -> np.ndarray:
        """Overlap-add robot frames; returns the mix of robot and pitched voice for
        every sample that is final (all frames covering it were added)."""
        n_fft, hop = self._n_fft, self._hop
        buf = np.concatenate([self._fr_in, pitched]) if pitched.size else self._fr_in
        total = self._fr_base + buf.size  # samples of pitched signal seen so far
        if final:
            n_frames = max(0, -(-(total - self._fr_base) // hop))  # frames starting before `total`
            buf = np.concatenate([buf, np.zeros(n_fft, dtype=np.float32)])
        else:
            n_frames = (buf.size - n_fft) // hop + 1 if buf.size >= n_fft else 0
        if n_frames <= 0:
            self._fr_in = buf if not final else buf[:0]
            return np.zeros(0, dtype=np.float32)

        starts = np.arange(n_frames) * hop
        frames = buf[starts[:, None] + np.arange(n_fft)[None, :]] * self._window
        mags = np.abs(np.fft.rfft(frames, axis=1))
        pulses = np.fft.fftshift(np.fft.irfft(mags, n_fft, axis=1), axes=1).astype(np.float32) * self._window

        robot = np.empty(n_frames * hop, dtype=np.float32)
        ola, ola_w = self._ola, self._ola_w
        for j in range(n_frames):
            ola += pulses[j]
            ola_w += self._window_sq
            robot[j * hop : (j + 1) * hop] = ola[:hop] / np.maximum(ola_w[:hop], _WINDOW_FLOOR)
            ola = np.concatenate([ola[hop:], np.zeros(hop, dtype=np.float32)])
            ola_w = np.concatenate([ola_w[hop:], np.zeros(hop, dtype=np.float32)])
        self._ola, self._ola_w = ola, ola_w

        voice = buf[: n_frames * hop]
        mixed = self._robot_gain * robot + self._voice_gain * voice
        if final:
            mixed = mixed[: total - self._fr_base]
            self._fr_in = buf[:0]
        else:
            self._fr_in = buf[n_frames * hop :]
        self._fr_base += n_frames * hop
        return mixed

    # ------------------------------------------------------------ stage 3

    def _finish(self, mixed: np.ndarray) -> np.ndarray:
        """Ring modulation, comb filter, gain, limiter."""
        if mixed.size == 0:
            return mixed
        n = self._out_n + np.arange(mixed.size, dtype=np.float64)
        carrier = np.sin(self._ring_step * n).astype(np.float32)
        self._out_n += mixed.size
        ringed = (1.0 - self._ring_mix) * mixed + self._ring_mix * mixed * carrier
        combed = self._comb(ringed)
        return soft_limit(combed * self._output_gain).astype(np.float32, copy=False)

    def _comb(self, x: np.ndarray) -> np.ndarray:
        """y[i] = x[i] + fb * y[i - d], evaluated block by block (d samples at a time)."""
        d, fb = self._comb_delay, self._comb_fb
        out = np.empty_like(x)
        hist = self._comb_hist
        for start in range(0, x.size, d):
            seg = x[start : start + d]
            y = seg + fb * hist[: seg.size]
            out[start : start + seg.size] = y
            hist = np.concatenate([hist[seg.size :], y])
        self._comb_hist = hist
        return out


# ---------------------------------------------------------------- PCM16 helpers


def pcm16_to_float(pcm: bytes) -> np.ndarray:
    return np.frombuffer(pcm, dtype=np.int16).astype(np.float32) / 32768.0


def float_to_pcm16(x: np.ndarray) -> bytes:
    return (np.clip(x, -1.0, 1.0) * 32767.0).astype(np.int16).tobytes()
