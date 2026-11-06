"""Voice activity detection: Silero as a streaming model plus an endpointer.

``SileroStream`` turns one 512-sample frame into a speech probability and keeps
the LSTM state between calls. ``Endpointer`` turns those probabilities into an
onset and an end of speech with the rules faster-whisper's offline
``get_speech_timestamps`` uses (the ones the VAD spike measured), so the live
behaviour matches ``spikes/vad/RESULTS.md``.
"""

from __future__ import annotations

import os
from dataclasses import dataclass
from typing import Protocol

import numpy as np

from config import FRAME_MS, FRAME_SAMPLES, Tunables

CONTEXT_SAMPLES = 64


class SpeechModel(Protocol):
    """Anything that scores a frame (float32 in [-1, 1], FRAME_SAMPLES long)."""

    def reset(self) -> None: ...

    def __call__(self, frame: np.ndarray) -> float: ...


class SileroStream:
    """Streaming wrapper over the Silero VAD ONNX model bundled with faster-whisper."""

    def __init__(self, model_path: str | None = None):
        import onnxruntime

        if model_path is None:
            from faster_whisper.utils import get_assets_path

            model_path = os.path.join(get_assets_path(), "silero_vad_v6.onnx")
        options = onnxruntime.SessionOptions()
        options.inter_op_num_threads = 1
        options.intra_op_num_threads = 1
        options.log_severity_level = 4
        self._session = onnxruntime.InferenceSession(
            model_path, providers=["CPUExecutionProvider"], sess_options=options
        )
        self.reset()

    def reset(self) -> None:
        self._h = np.zeros((1, 1, 128), dtype=np.float32)
        self._c = np.zeros((1, 1, 128), dtype=np.float32)
        self._context = np.zeros(CONTEXT_SAMPLES, dtype=np.float32)

    def __call__(self, frame: np.ndarray) -> float:
        if frame.shape != (FRAME_SAMPLES,):
            raise ValueError(f"expected {FRAME_SAMPLES} samples, got {frame.shape}")
        x = np.concatenate([self._context, frame.astype(np.float32)])[None, :]
        out, self._h, self._c = self._session.run(None, {"input": x, "h": self._h, "c": self._c})
        self._context = frame[-CONTEXT_SAMPLES:].astype(np.float32)
        return float(out[0])


@dataclass(frozen=True)
class Onset:
    """Speech confirmed. ``frames_ago`` is how many frames back it really began
    (the current frame is 0)."""

    frames_ago: int


@dataclass(frozen=True)
class End:
    """End of speech. ``silence_frames_ago`` counts back from the current frame
    to the first frame of the closing silence (the current frame is 0)."""

    silence_frames_ago: int


class Endpointer:
    """Probabilities in, ``Onset`` / ``End`` out. One instance per listen."""

    def __init__(self, tunables: Tunables):
        self._t = tunables
        self._end_frames = -(-tunables.end_silence_ms // FRAME_MS)  # ceil
        self.reset()

    def reset(self) -> None:
        self._index = 0
        self._speech_run = 0
        self._in_speech = False
        self._silence_start: int | None = None

    @property
    def in_speech(self) -> bool:
        return self._in_speech

    def push(self, prob: float) -> Onset | End | None:
        i = self._index
        self._index += 1
        t = self._t
        if not self._in_speech:
            self._speech_run = self._speech_run + 1 if prob >= t.vad_threshold else 0
            if self._speech_run >= t.onset_frames:
                self._in_speech = True
                self._silence_start = None
                return Onset(frames_ago=self._speech_run - 1)
            return None
        if prob >= t.vad_threshold:
            self._silence_start = None
            return None
        if prob < t.vad_neg_threshold and self._silence_start is None:
            self._silence_start = i
        if self._silence_start is not None and i - self._silence_start >= self._end_frames:
            ago = i - self._silence_start
            self._in_speech = False
            self._speech_run = 0
            self._silence_start = None
            return End(silence_frames_ago=ago)
        return None
