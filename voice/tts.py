"""Text to speech: edge-tts by default, Piper as the fallback (design D7).

The chain is tried per utterance. Playback starts at the first decoded chunk, and
``speak_started`` is sent only after the first non-silent chunk has been written
to the output device (speech-io, "Truthful speech start"). Nothing audible
reaching the device is ``TTS_FAILED`` for that speak id.

Network, decoder, voices and audio device are seams (``mp3_stream``,
``TtsEngine``, ``make_output``), so tests run without any of them.
"""

from __future__ import annotations

import asyncio
import logging
import queue
import re
import threading
from collections.abc import AsyncIterator, Callable, Iterable, Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

import numpy as np

import protocol
from audio_io import AudioOutput, OutputError
from config import Tunables
from protocol import ErrorCode

log = logging.getLogger("niva.voice.tts")

EDGE_VOICES = {"es": "es-AR-ElenaNeural", "en": "en-US-AriaNeural"}
PIPER_VOICE_FILES = {"es": "es_AR-daniela-high.onnx", "en": "en_US-lessac-medium.onnx"}
EDGE_SAMPLE_RATE = 24_000

# ---------------------------------------------------------------- clean text

_FENCE = re.compile(r"```.*?(?:```|\Z)", re.DOTALL)
_INLINE_CODE = re.compile(r"`[^`\n]*`")
_IMAGE = re.compile(r"!\[([^\]]*)\]\([^)]*\)")
_LINK = re.compile(r"\[([^\]]+)\]\([^)]*\)")
_URL = re.compile(r"(?:https?://|www\.)\S+", re.IGNORECASE)
_HTML = re.compile(r"<[^>\n]+>")
_HEADING = re.compile(r"^[ \t]{0,3}#{1,6}[ \t]*", re.MULTILINE)
_QUOTE = re.compile(r"^[ \t]*>[ \t]?", re.MULTILINE)
_BULLET = re.compile(r"^[ \t]*(?:[-*+•]|\d{1,3}[.)])[ \t]+", re.MULTILINE)
_RULE = re.compile(r"^[ \t]*(?:[-*_=]{3,})[ \t]*$", re.MULTILINE)
_BOLD = re.compile(r"(\*\*|__|~~)")
_ITALIC_UNDERSCORE = re.compile(r"(?<!\w)_(?=\S)(.+?)(?<=\S)_(?!\w)")
_EMOJI = re.compile(
    "[\U0001f000-\U0001faff\U00002600-\U000027bf\U00002300-\U000023ff\U00002b00-\U00002bff"
    "\U0000fe0f\U0000200d\U000020e3\U0001f1e6-\U0001f1ff]+"
)
_SENTENCE_END = ".!?:;,…"


def clean_for_speech(text: str) -> str:
    """Strip markdown, URLs, emoji and code, so the voice does not read symbols."""
    text = _FENCE.sub(" ", text)
    text = _INLINE_CODE.sub(" ", text)
    text = _IMAGE.sub(r"\1", text)
    text = _LINK.sub(r"\1", text)
    text = _URL.sub(" ", text)
    text = _HTML.sub(" ", text)
    text = _RULE.sub("", text)
    text = _HEADING.sub("", text)
    text = _QUOTE.sub("", text)
    text = _BULLET.sub("", text)
    text = _BOLD.sub("", text)
    text = _ITALIC_UNDERSCORE.sub(r"\1", text)
    text = text.replace("*", "").replace("|", ", ").replace("#", " ")
    text = _EMOJI.sub(" ", text)
    lines = [re.sub(r"[ \t]+", " ", line).strip() for line in text.splitlines()]
    lines = [line for line in lines if line]
    if not lines:
        return ""
    spoken = []
    for index, line in enumerate(lines):
        if index < len(lines) - 1 and line[-1] not in _SENTENCE_END:
            line += "."
        spoken.append(line)
    return " ".join(spoken)


# ---------------------------------------------------------------- engines


@dataclass(frozen=True)
class PcmChunk:
    data: bytes  # int16 mono little-endian
    sample_rate: int


class TtsEngine(Protocol):
    name: str

    def chunks(self, text: str, lang: str, cancel: threading.Event) -> Iterator[PcmChunk]: ...


class TtsTimeout(Exception):
    """The engine did not deliver in time (first chunk, or between chunks)."""


Mp3Stream = Callable[[str, str], AsyncIterator[bytes]]
_DONE = object()


async def edge_mp3_stream(text: str, voice: str) -> AsyncIterator[bytes]:
    """The real network seam: MP3 chunks from the edge-tts endpoint."""
    import edge_tts

    async for chunk in edge_tts.Communicate(text, voice).stream():
        if chunk["type"] == "audio":
            yield chunk["data"]


def threaded_chunks(
    make_stream: Callable[[], AsyncIterator[bytes]],
    first_timeout_s: float,
    idle_timeout_s: float,
    cancel: threading.Event,
) -> Iterator[bytes]:
    """Run an async byte stream on a helper thread and hand its chunks over with
    a timeout on the first one (design D7: first chunk > 2 s falls back)."""
    handoff: queue.Queue[Any] = queue.Queue()
    abandon = threading.Event()

    def pump() -> None:
        async def run() -> None:
            try:
                async for chunk in make_stream():
                    if abandon.is_set() or cancel.is_set():
                        return
                    handoff.put(chunk)
                handoff.put(_DONE)
            except Exception as exc:  # noqa: BLE001 - handed to the consumer
                handoff.put(exc)

        asyncio.run(run())

    threading.Thread(target=pump, name="tts-network", daemon=True).start()
    timeout = first_timeout_s
    try:
        while True:
            try:
                item = handoff.get(timeout=timeout)
            except queue.Empty:
                raise TtsTimeout(f"no audio from the network for {timeout:g} s") from None
            timeout = idle_timeout_s
            if item is _DONE:
                return
            if isinstance(item, Exception):
                raise item
            yield item
    finally:
        abandon.set()


class _IterSource:
    """Adapts an iterator of byte chunks to miniaudio's streaming source."""

    def __init__(self, chunks: Iterable[bytes]):
        self._it = iter(chunks)
        self._buf = b""
        self.error: Exception | None = None

    def read(self, num_bytes: int) -> bytes:
        while not self._buf:
            try:
                self._buf = next(self._it)
            except StopIteration:
                return b""
            except Exception as exc:  # noqa: BLE001 - re-raised after the decoder stops
                self.error = exc
                return b""
        out, self._buf = self._buf[:num_bytes], self._buf[num_bytes:]
        return out

    def seek(self, offset: int, origin: Any) -> bool:
        return False

    def close(self) -> None:
        pass


def decode_mp3_stream(chunks: Iterable[bytes], sample_rate: int = EDGE_SAMPLE_RATE) -> Iterator[PcmChunk]:
    """Decode MP3 bytes to mono int16 PCM as they arrive (miniaudio)."""
    import miniaudio

    class Source(_IterSource, miniaudio.StreamableSource):
        pass

    source = Source(chunks)
    stream = None
    try:
        stream = miniaudio.stream_any(
            source,
            source_format=miniaudio.FileFormat.MP3,
            output_format=miniaudio.SampleFormat.SIGNED16,
            nchannels=1,
            sample_rate=sample_rate,
            frames_to_read=1024,
        )
        for block in stream:
            if len(block):
                yield PcmChunk(block.tobytes(), sample_rate)
    except miniaudio.DecodeError:
        if source.error is not None:  # the decoder only saw the network failure as "no data"
            raise source.error from None
        raise
    finally:
        if stream is not None:
            stream.close()
    if source.error is not None:
        raise source.error


class EdgeTtsEngine:
    name = "edge-tts"

    def __init__(self, tunables: Tunables, mp3_stream: Mp3Stream = edge_mp3_stream, idle_timeout_s: float = 10.0):
        self._first = tunables.tts_first_chunk_s
        self._idle = idle_timeout_s
        self._mp3_stream = mp3_stream

    def chunks(self, text: str, lang: str, cancel: threading.Event) -> Iterator[PcmChunk]:
        voice = EDGE_VOICES[lang]
        mp3 = threaded_chunks(lambda: self._mp3_stream(text, voice), self._first, self._idle, cancel)
        yield from decode_mp3_stream(mp3)


class PiperEngine:
    name = "piper"

    def __init__(self, model_dir: str | Path):
        self._dir = Path(model_dir)
        self._voices: dict[str, Any] = {}
        self._load_lock = threading.Lock()

    def available(self) -> bool:
        return all((self._dir / name).is_file() for name in PIPER_VOICE_FILES.values())

    def preload(self) -> None:
        """Load both voices now, so the fallback does not pay seconds on first use."""
        for lang in PIPER_VOICE_FILES:
            self._voice(lang)

    def _voice(self, lang: str) -> Any:
        with self._load_lock:
            voice = self._voices.get(lang)
            if voice is None:
                from piper import PiperVoice

                voice = self._voices[lang] = PiperVoice.load(str(self._dir / PIPER_VOICE_FILES[lang]))
            return voice

    def chunks(self, text: str, lang: str, cancel: threading.Event) -> Iterator[PcmChunk]:
        voice = self._voice(lang)
        for chunk in voice.synthesize(text):
            if cancel.is_set():
                return
            yield PcmChunk(chunk.audio_int16_bytes, chunk.sample_rate)


# ---------------------------------------------------------------- speaker


def is_silent(pcm: bytes, silent_peak: int) -> bool:
    samples = np.frombuffer(pcm, dtype=np.int16)
    return samples.size == 0 or int(np.abs(samples.astype(np.int32)).max()) <= silent_peak


class _DeviceFailure(Exception):
    pass


class _Cancelled(Exception):
    pass


class _Run:
    """State of one utterance: its own cancel flag and its own output device."""

    def __init__(self, emit: Callable[[dict[str, Any]], None]) -> None:
        self.cancel = threading.Event()
        self.output: AudioOutput | None = None
        self.lock = threading.Lock()
        self._emit = emit

    def say(self, message: dict[str, Any]) -> None:
        """Send a protocol line unless this utterance was stopped. Checking and
        sending are atomic with `abort`, so a stop can never be followed by a line."""
        with self.lock:
            if not self.cancel.is_set():
                self._emit(message)

    def abort(self) -> None:
        with self.lock:
            self.cancel.set()
            output = self.output
        if output is not None:
            output.abort()

    def adopt(self, output: AudioOutput) -> None:
        with self.lock:
            if self.cancel.is_set():
                output.close()
                raise _Cancelled
            self.output = output

    def close_output(self) -> None:
        """Close the device (this waits for the buffer to play out). It stays
        reachable by `abort` while it drains."""
        with self.lock:
            output = self.output
        if output is not None:
            output.close()
        with self.lock:
            self.output = None


class Speaker:
    """Speaks one utterance at a time on a worker thread."""

    def __init__(
        self,
        engines: Sequence[TtsEngine],
        make_output: Callable[[int], AudioOutput],
        emit: Callable[[dict[str, Any]], None],
        on_ended: Callable[[int], None],
        tunables: Tunables,
    ):
        self._engines = list(engines)
        self._make_output = make_output
        self._emit = emit
        self._on_ended = on_ended
        self._t = tunables
        self._current: _Run | None = None
        self._thread: threading.Thread | None = None

    def speak(self, speak_id: int, text: str, lang: str) -> None:
        self.stop()
        run = _Run(self._emit)
        self._current = run
        self._thread = threading.Thread(
            target=self._work, args=(speak_id, text, lang, run), name=f"speak-{speak_id}", daemon=True
        )
        self._thread.start()

    def stop(self) -> None:
        run = self._current
        if run is not None:
            run.abort()

    def join(self, timeout: float | None = None) -> None:
        thread = self._thread
        if thread is not None:
            thread.join(timeout)

    # -- worker

    def _work(self, speak_id: int, text: str, lang: str, run: _Run) -> None:
        completed = False
        try:
            completed = self._speak(speak_id, text, lang, run)
        except _Cancelled:
            pass
        except Exception as exc:  # noqa: BLE001 - never leave the host waiting for this id
            log.exception("speaking failed")
            run.say(protocol.error(ErrorCode.TTS_FAILED, f"speech failed: {exc}", speak_id))
        finally:
            run.close_output()  # drains: the sound has really finished after this
            if completed:
                run.say(protocol.speak_done(speak_id))
            self._on_ended(speak_id)

    def _speak(self, speak_id: int, text: str, lang: str, run: _Run) -> bool:
        """Play the reply. True when it played and `speak_done` is due (after the
        device drained); errors are reported here."""
        cancel = run.cancel
        spoken = clean_for_speech(text)
        if not spoken:
            run.say(protocol.error(ErrorCode.TTS_FAILED, "nothing speakable in the reply", speak_id))
            return False
        started = False
        failures: list[str] = []
        for engine in self._engines:
            try:
                for chunk in engine.chunks(spoken, lang, cancel):
                    if cancel.is_set():
                        return False
                    self._write(run, chunk)
                    if not started and not is_silent(chunk.data, self._t.silent_peak):
                        started = True
                        run.say(protocol.speak_started(speak_id))
            except _DeviceFailure as exc:
                run.say(protocol.error(ErrorCode.TTS_FAILED, str(exc), speak_id))
                return False
            except _Cancelled:
                return False
            except Exception as exc:  # noqa: BLE001 - this engine failed; maybe the next one works
                if cancel.is_set():
                    return False
                if started:
                    log.warning("%s stopped mid-utterance: %s", engine.name, exc)
                    break
                failures.append(f"{engine.name}: {exc}")
                log.warning("%s failed before any audio: %s", engine.name, exc)
                continue
            if started:
                break
            failures.append(f"{engine.name}: produced no audible audio")
        if cancel.is_set():
            return False
        if started:
            return True
        detail = " | ".join(failures) or "no speech engine available"
        run.say(protocol.error(ErrorCode.TTS_FAILED, detail, speak_id))
        return False

    def _write(self, run: _Run, chunk: PcmChunk) -> None:
        try:
            if run.output is None or run.output.sample_rate != chunk.sample_rate:
                run.close_output()
                run.adopt(self._make_output(chunk.sample_rate))
            run.output.write(chunk.data)
        except OutputError as exc:
            raise _DeviceFailure(str(exc)) from exc
