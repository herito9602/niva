"""Audio devices behind thin seams: the microphone and the speaker output.

Only this module touches ``sounddevice``. Everything else talks to the
``Microphone`` and ``AudioOutput`` protocols, so tests use fakes and never open a
device.
"""

from __future__ import annotations

import queue
from typing import Protocol

import numpy as np

from config import FRAME_SAMPLES, SAMPLE_RATE
from protocol import ErrorCode


class MicError(Exception):
    """The microphone could not be opened or stopped delivering audio."""

    def __init__(self, code: ErrorCode, msg: str):
        super().__init__(msg)
        self.code = code
        self.msg = msg


class OutputError(Exception):
    """Writing to the output device failed."""


class Microphone(Protocol):
    def open(self) -> None:
        """Open the one stream. Raises :class:`MicError`."""
        ...

    def read(self) -> np.ndarray:
        """Block for the next FRAME_SAMPLES int16 frame. Raises :class:`MicError`
        when the stream fails or stalls."""
        ...

    def close(self) -> None: ...


class AudioOutput(Protocol):
    sample_rate: int
    # Seconds between handing samples to the device and hearing them (the stream's
    # output latency). The level pacer needs it to show the waveform when the voice
    # is audible, not when it is written.
    latency: float

    def write(self, pcm: bytes) -> None:
        """Write int16 mono PCM; blocks until the device accepted it. Raises
        :class:`OutputError`."""
        ...

    def abort(self) -> None: ...

    def close(self) -> None: ...


_BUSY_HINTS = ("unavailable", "-9985", "in use", "busy", "exclusive")
_MISSING_HINTS = ("no default input", "invalid device", "-9996", "querying device", "no device", "-9998")


def classify_mic_error(exc: BaseException) -> MicError:
    """Map a PortAudio failure to the taxonomy. A device that is held by another
    app is MIC_BUSY; anything else that stops capture (unplugged, no device,
    unknown) is MIC_MISSING, because the stream is gone either way."""
    text = str(exc)
    lowered = text.lower()
    if any(hint in lowered for hint in _BUSY_HINTS):
        return MicError(ErrorCode.MIC_BUSY, f"microphone in use: {text}")
    if any(hint in lowered for hint in _MISSING_HINTS):
        return MicError(ErrorCode.MIC_MISSING, f"no microphone: {text}")
    return MicError(ErrorCode.MIC_MISSING, f"microphone stream failed: {text}")


class SoundDeviceMic:
    """One input stream, callback-fed into a bounded queue, read with a stall timeout."""

    def __init__(self, stall_s: float, device: int | str | None = None):
        self._stall_s = stall_s
        self._device = device
        self._frames: queue.Queue[np.ndarray] = queue.Queue(maxsize=512)
        self._stream = None
        self._failure: str | None = None

    def open(self) -> None:
        self._frames = queue.Queue(maxsize=512)
        self._failure = None
        try:
            import sounddevice as sd

            self._stream = sd.InputStream(
                samplerate=SAMPLE_RATE,
                channels=1,
                dtype="int16",
                blocksize=FRAME_SAMPLES,
                device=self._device,
                callback=self._on_audio,
                finished_callback=self._on_finished,
            )
            self._stream.start()
        except Exception as exc:  # noqa: BLE001 - PortAudio raises several types
            self.close()
            raise classify_mic_error(exc) from exc

    def _on_audio(self, indata, frames, time_info, status) -> None:
        try:
            self._frames.put_nowait(indata[:, 0].copy())
        except queue.Full:
            pass  # the main loop is far behind; dropping is better than blocking PortAudio

    def _on_finished(self) -> None:
        self._failure = "stream stopped"

    def read(self) -> np.ndarray:
        try:
            return self._frames.get(timeout=self._stall_s)
        except queue.Empty:
            reason = self._failure or f"no audio for {self._stall_s:g} s"
            raise MicError(ErrorCode.MIC_MISSING, f"microphone stream stalled: {reason}") from None

    def close(self) -> None:
        stream, self._stream = self._stream, None
        if stream is not None:
            try:
                stream.abort()
                stream.close()
            except Exception:  # noqa: BLE001, S110 - closing a dead stream may fail
                pass


class SoundDeviceOutput:
    """One playback stream for one utterance."""

    def __init__(self, sample_rate: int):
        import sounddevice as sd

        self.sample_rate = sample_rate
        try:
            self._stream = sd.RawOutputStream(samplerate=sample_rate, channels=1, dtype="int16")
            self._stream.start()
        except Exception as exc:  # noqa: BLE001
            raise OutputError(f"cannot open output device: {exc}") from exc

    @property
    def latency(self) -> float:
        try:
            return float(self._stream.latency)
        except Exception:  # noqa: BLE001 - a stream without a latency is read as an immediate one
            return 0.0

    def write(self, pcm: bytes) -> None:
        try:
            self._stream.write(pcm)
        except Exception as exc:  # noqa: BLE001
            raise OutputError(f"output write failed: {exc}") from exc

    def abort(self) -> None:
        try:
            self._stream.abort()
        except Exception:  # noqa: BLE001, S110
            pass

    def close(self) -> None:
        try:
            self._stream.stop()
            self._stream.close()
        except Exception:  # noqa: BLE001, S110
            pass
