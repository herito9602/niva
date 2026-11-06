"""Fakes shared by the tests: no microphone, GPU, network or sound card."""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import pytest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from capture import Engine  # noqa: E402
from config import FRAME_SAMPLES, Tunables  # noqa: E402
from discards import DiscardLog  # noqa: E402
from stt import SttResult  # noqa: E402

# Frame levels: the fakes read the loudness of a frame to decide what it "is".
SILENCE = 0
SOFT = 200  # a soft syllable: below the fake VAD's speech level, above silence
SPEECH = 1000
WAKE_WORD = 3000  # the wake phrase; the fake detector fires on the frame that ends it
WAKE_END = 3001


def frame(level: int) -> np.ndarray:
    return np.full(FRAME_SAMPLES, level, dtype=np.int16)


def frames(level: int, count: int) -> list[np.ndarray]:
    return [frame(level) for _ in range(count)]


def levels(audio: np.ndarray) -> list[int]:
    """The per-frame levels of float audio handed to STT (inverse of `frame`)."""
    ints = np.rint(audio * 32768.0).astype(np.int32)
    return [int(ints[i]) for i in range(0, len(ints), FRAME_SAMPLES)]


class FakeVad:
    def __init__(self) -> None:
        self.resets = 0

    def reset(self) -> None:
        self.resets += 1

    def __call__(self, audio: np.ndarray) -> float:
        peak = abs(int(round(float(audio[0]) * 32768.0)))
        if peak >= SPEECH:
            return 0.9
        if peak >= SOFT:
            return 0.2
        return 0.02


class FakeWake:
    def __init__(self) -> None:
        self.resets = 0
        self.forced: float | None = None

    def reset(self) -> None:
        self.resets += 1

    def score(self, audio: np.ndarray) -> float:
        if self.forced is not None:
            return self.forced
        return 0.9 if int(audio[0]) == WAKE_END else 0.0


class FakeStt:
    def __init__(self, text: str = "open spotify", lang: str = "en", lang_prob: float = 0.99) -> None:
        self.text, self.lang, self.lang_prob = text, lang, lang_prob
        self.heard: list[np.ndarray] = []
        self.fail: Exception | None = None

    def transcribe(self, audio: np.ndarray) -> SttResult:
        self.heard.append(audio)
        if self.fail:
            raise self.fail
        return SttResult(self.text, self.lang, self.lang_prob)


class FakeSpeaker:
    def __init__(self) -> None:
        self.spoken: list[tuple[int, str, str]] = []
        self.stops = 0

    def speak(self, speak_id: int, text: str, lang: str) -> None:
        self.spoken.append((speak_id, text, lang))

    def stop(self) -> None:
        self.stops += 1


class Harness:
    def __init__(self, tunables: Tunables | None = None, **stt_args) -> None:
        self.events: list[dict] = []
        self.wake = FakeWake()
        self.vad = FakeVad()
        self.stt = FakeStt(**stt_args)
        self.speaker = FakeSpeaker()
        self.discards = DiscardLog()
        self.now = 1_000_000
        self.tunables = tunables or Tunables()
        self.engine = Engine(
            wake=self.wake,
            vad=self.vad,
            stt=self.stt,
            speaker=self.speaker,
            emit=self.events.append,
            discards=self.discards,
            tunables=self.tunables,
            clock_ms=lambda: self.now,
        )

    def feed(self, items: list[np.ndarray]) -> None:
        for item in items:
            self.now += 32
            self.engine.on_frame(item, self.now)

    def start(self) -> Harness:
        """Microphone open and the first frame delivered: `ready` sent, then forgotten."""
        self.engine.on_mic_ok()
        self.feed([frame(SILENCE)])
        self.events.clear()
        return self

    def types(self) -> list[str]:
        return [e["type"] for e in self.events]


@pytest.fixture
def h() -> Harness:
    return Harness().start()
