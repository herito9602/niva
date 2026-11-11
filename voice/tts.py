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
import time
from collections.abc import AsyncIterator, Callable, Iterable, Iterator, Sequence
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Protocol

import numpy as np

import protocol
from audio_io import AudioOutput, OutputError
from config import Tunables
from edge_ws import EdgeWsClient, EdgeWsTimeout, UnsupportedText
from level_pacer import LevelPacer, level_offset_ms
from protocol import ErrorCode
from tts_cache import TtsCache
from voice_fx import RobotVoice, float_to_pcm16, pcm16_to_float

log = logging.getLogger("niva.voice.tts")

EDGE_VOICES = {"es": "es-CU-BelkysNeural", "en": "en-US-AriaNeural"}
PIPER_VOICE_FILES = {"es": "es_AR-daniela-high.onnx", "en": "en_US-lessac-medium.onnx"}
EDGE_SAMPLE_RATE = 24_000
# An output device that has not opened after this long is a device failure, not a wait.
OUTPUT_OPEN_TIMEOUT_S = 10.0

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
    output_rate = EDGE_SAMPLE_RATE  # lets the speaker open the device before the first chunk

    def __init__(
        self,
        tunables: Tunables,
        mp3_stream: Mp3Stream = edge_mp3_stream,
        idle_timeout_s: float = 10.0,
        client: EdgeWsClient | None = None,
        cache: TtsCache | None = None,
    ):
        self._first = tunables.tts_first_chunk_s
        self._idle = idle_timeout_s
        self._mp3_stream = mp3_stream
        self._client = client  # the persistent connection; None = a new connection per reply
        self._cache = cache  # raw MP3 of short replies; None = always synthesize
        self._writers: list[threading.Thread] = []  # cache writes in flight (they run off the speaker thread)

    def prepare(self) -> None:
        """A reply is likely soon: have the connection ready."""
        if self._client is not None:
            self._client.prewarm()

    def chunks(self, text: str, lang: str, cancel: threading.Event) -> Iterator[PcmChunk]:
        voice = EDGE_VOICES[lang]
        cache, key = self._cache, None
        if cache is not None and cache.cacheable(text):
            key = cache.key(self.name, voice, text)
            cached = cache.get(key)
            if cached is not None:
                produced = False
                try:
                    for pcm in decode_mp3_stream([cached]):  # the same MP3 the network would deliver
                        produced = True
                        yield pcm
                    return
                except Exception as exc:  # noqa: BLE001
                    if produced:
                        raise
                    log.warning("cached reply did not decode (%s); synthesizing it again", exc)
                    cache.delete(key)
        if cache is None or key is None:
            yield from decode_mp3_stream(self.mp3_chunks(text, voice, cancel))
            return
        received: list[bytes] = []

        def tap() -> Iterator[bytes]:
            for piece in self.mp3_chunks(text, voice, cancel):
                received.append(piece)
                yield piece

        yield from decode_mp3_stream(tap())
        # Reached only when the synthesis ran to its end: a stop closes this generator before here.
        if not cancel.is_set():
            # Off the speaker's thread: this code runs before the effect's tail is flushed, and a
            # disk write plus an eviction scan must not delay the last milliseconds of the reply.
            writer = threading.Thread(
                target=cache.put, args=(key, b"".join(received)), name="tts-cache-put", daemon=True
            )
            self._writers = [w for w in self._writers if w.is_alive()] + [writer]
            writer.start()

    def wait_for_cache(self, timeout_s: float = 5.0) -> None:
        """Block until the cache writes started so far are done (shutdown and tests)."""
        for writer in list(self._writers):
            writer.join(timeout_s)

    def mp3_chunks(self, text: str, voice: str, cancel: threading.Event) -> Iterator[bytes]:
        """MP3 chunks from the network: the persistent connection first, then the one-off
        edge-tts connection when that failed before any audio, a timeout included (the
        one-off attempt gets the full first-audio budget: a slow network must not lose to
        Piper just because the persistent attempt used some of it). A gap in the middle of
        the audio goes straight to the next engine."""
        client = self._client
        first = self._first
        if client is not None:
            started = False
            try:
                for piece in client.stream(text, voice, cancel, self._first, self._idle):
                    started = True
                    yield piece
                return
            except EdgeWsTimeout as exc:
                if started:
                    raise TtsTimeout(str(exc)) from exc
                log.warning("persistent edge-tts connection timed out (%s); using a one-off connection", exc)
            except UnsupportedText:
                pass  # a long text: edge-tts splits it into several requests itself
            except Exception as exc:  # noqa: BLE001
                if started:
                    raise
                log.warning("persistent edge-tts connection failed (%s); using a one-off connection", exc)
        yield from threaded_chunks(lambda: self._mp3_stream(text, voice), first, self._idle, cancel)


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


class OutputPrefetch:
    """Opens the output device on a helper thread while the reply is still being
    synthesized (opening a stream costs ~300 ms on MME). Exactly one of two things
    happens to it: the speaker `take`s it for the first write, or `discard` closes
    it (stop, unused, wrong sample rate). Open failures surface from `take`, at the
    moment the first audio would be written, like an open done at that moment."""

    def __init__(self, make_output: Callable[[int], AudioOutput], sample_rate: int) -> None:
        self.sample_rate = sample_rate
        self._make_output = make_output
        self._lock = threading.Lock()
        self._opened = threading.Event()
        self._output: AudioOutput | None = None
        self._error: BaseException | None = None
        self._finished = False  # taken or discarded
        threading.Thread(target=self._open, name="output-open", daemon=True).start()

    def _open(self) -> None:
        output: AudioOutput | None = None
        error: BaseException | None = None
        try:
            output = self._make_output(self.sample_rate)
        except BaseException as exc:  # noqa: BLE001 - re-raised by `take`
            error = exc
        with self._lock:
            late = self._finished  # discarded while the device was opening
            if not late:
                self._output, self._error = output, error
        if late and output is not None:
            output.abort()
            output.close()
        self._opened.set()

    def take(self) -> AudioOutput:
        """Wait for the device and hand it over. Raises `_Cancelled` when it was discarded
        meanwhile (a stop), whatever opening it raised, or `OutputError` when it has not
        opened after `OUTPUT_OPEN_TIMEOUT_S` (a hung driver: reported like a device failure)."""
        deadline = time.monotonic() + OUTPUT_OPEN_TIMEOUT_S
        while not self._opened.wait(0.05):
            with self._lock:
                if self._finished:  # discarded by a stop while the device was still opening
                    raise _Cancelled
                if time.monotonic() >= deadline:
                    self._finished = True  # `_open` closes the device if it ever arrives
                    raise OutputError(f"the audio output did not open within {OUTPUT_OPEN_TIMEOUT_S:g} s")
        with self._lock:
            if self._finished:
                raise _Cancelled
            self._finished = True
            output, error = self._output, self._error
            self._output = None
        if error is not None:
            raise error
        assert output is not None
        return output

    def discard(self) -> None:
        with self._lock:
            if self._finished:
                return
            self._finished = True
            output, self._output = self._output, None
        if output is not None:  # (a device still opening is closed by `_open` when it arrives)
            output.abort()  # never wait for a buffer: nothing was played, and a stop must not block
            output.close()


class _Run:
    """State of one utterance: its own cancel flag and its own output device."""

    def __init__(self, emit: Callable[[dict[str, Any]], None], level_offset_ms: float = 0.0) -> None:
        self.cancel = threading.Event()
        self.output: AudioOutput | None = None
        self.prefetch: OutputPrefetch | None = None  # device being opened ahead of the first audio
        self.fx: RobotVoice | None = None  # robot effect state of the current engine's audio
        self.odd = b""  # a lone trailing byte of an odd-sized chunk, completed by the next one
        self.lock = threading.Lock()
        self._emit = emit
        # Levels of the voice for the notch, paced to what is audible. They flow only
        # between `speak_started` and the end of the speech: nobody speaking, no levels.
        self.audible = threading.Event()
        self.pacer = LevelPacer(self._level, offset_ms=level_offset_ms)

    def _level(self, bands: list[int]) -> None:
        if self.audible.is_set():
            self.say(protocol.level(bands))

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
            prefetch = self.prefetch
        self.pacer.stop()  # a barge-in: the bars stop with the sound, not a period later
        if prefetch is not None:
            prefetch.discard()
        if output is not None:
            output.abort()

    def start_prefetch(self, make_output: Callable[[int], AudioOutput], sample_rate: int) -> None:
        with self.lock:
            if not self.cancel.is_set() and self.prefetch is None:
                self.prefetch = OutputPrefetch(make_output, sample_rate)

    def take_prefetch(self) -> OutputPrefetch | None:
        with self.lock:
            prefetch, self.prefetch = self.prefetch, None
        return prefetch

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


def _latency_of(output: AudioOutput) -> float:
    """The output's latency in seconds; an output that does not report one counts as immediate."""
    try:
        return max(0.0, float(getattr(output, "latency", 0.0) or 0.0))
    except (TypeError, ValueError):
        return 0.0


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
        self._level_offset_ms = level_offset_ms()  # NIVA_TTS_LEVEL_OFFSET_MS, read once at start
        self._current: _Run | None = None
        self._thread: threading.Thread | None = None

    def speak(self, speak_id: int, text: str, lang: str) -> None:
        self.stop()
        run = _Run(self._emit, self._level_offset_ms)
        self._current = run
        self._thread = threading.Thread(
            target=self._work, args=(speak_id, text, lang, run), name=f"speak-{speak_id}", daemon=True
        )
        self._thread.start()

    def stop(self) -> None:
        run = self._current
        if run is not None:
            run.abort()

    def prepare(self) -> None:
        """A reply is likely soon (a listen started or a transcript is about to go out):
        let the engines get ready, e.g. connect."""
        for engine in self._engines:
            prepare = getattr(engine, "prepare", None)
            if prepare is not None:
                try:
                    prepare()
                except Exception as exc:  # noqa: BLE001 - preparing is an optimization only
                    log.debug("%s could not prepare: %s", engine.name, exc)

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
            unused = run.take_prefetch()
            if unused is not None:  # no audio ever needed the device that was opened ahead
                unused.discard()
            run.close_output()  # drains: the sound has really finished after this
            run.pacer.finish()  # what is due goes out, the rest of the levels never will
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
        output_rate = getattr(self._engines[0], "output_rate", None) if self._engines else None
        if self._t.preopen_output and output_rate:
            run.start_prefetch(self._make_output, output_rate)  # opens while the first audio is synthesized

        def heard(pcm: bytes) -> None:
            """`speak_started` goes out only once audible samples really reached the device."""
            nonlocal started
            if not started and pcm and not is_silent(pcm, self._t.silent_peak):
                started = True
                run.say(protocol.speak_started(speak_id))
                run.audible.set()

        for engine in self._engines:
            run.fx, run.odd = None, b""  # a new engine is a new signal: no effect state carries over
            try:
                for chunk in engine.chunks(spoken, lang, cancel):
                    if cancel.is_set():
                        return False
                    self._play(run, chunk, heard)
                if not cancel.is_set():
                    heard(self._finish_effect(run))  # tail held back by the effect
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
                    try:
                        heard(self._finish_effect(run))
                    except _DeviceFailure as device_exc:
                        run.say(protocol.error(ErrorCode.TTS_FAILED, str(device_exc), speak_id))
                        return False
                    except _Cancelled:
                        return False
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

    def _play(self, run: _Run, chunk: PcmChunk, heard: Callable[[bytes], None]) -> None:
        """Apply the robot effect (when enabled), write the result and report what
        reached the device to `heard`. The effect holds back a short tail, so the
        first chunks can write nothing."""
        if not self._t.robot_voice:
            self._write(run, chunk)
            heard(chunk.data)
            return
        if run.fx is not None and run.fx.sample_rate != chunk.sample_rate:
            heard(self._finish_effect(run))  # the rate changed mid-stream: close the old signal
        pcm = run.odd + chunk.data
        run.odd = pcm[-1:] if len(pcm) % 2 else b""
        pcm = pcm[: len(pcm) - len(run.odd)]
        if run.fx is None:
            run.fx = RobotVoice(chunk.sample_rate)
        data = float_to_pcm16(run.fx.process(pcm16_to_float(pcm)))
        if data:
            self._write(run, PcmChunk(data, chunk.sample_rate))
        heard(data)

    def _finish_effect(self, run: _Run) -> bytes:
        """Write what the effect still holds at the end of an engine's audio, and reset it."""
        fx, run.fx, run.odd = run.fx, None, b""
        if fx is None or run.cancel.is_set():
            return b""
        data = float_to_pcm16(fx.flush())
        if data:
            self._write(run, PcmChunk(data, fx.sample_rate))
        return data

    def _write(self, run: _Run, chunk: PcmChunk) -> None:
        try:
            if run.output is None or run.output.sample_rate != chunk.sample_rate:
                run.close_output()
                run.adopt(self._open_output(run, chunk.sample_rate))
            handed = time.perf_counter()  # the pacer's clock: monotonic() ticks every 15.6 ms on Windows
            run.output.write(chunk.data)
            returned = time.perf_counter()
            run.pacer.on_write(chunk.data, chunk.sample_rate, returned, _latency_of(run.output), handed)
        except OutputError as exc:
            raise _DeviceFailure(str(exc)) from exc

    def _open_output(self, run: _Run, sample_rate: int) -> AudioOutput:
        """The device for this audio: the one opened ahead when its rate fits."""
        prefetch = run.take_prefetch()
        if prefetch is not None:
            if prefetch.sample_rate == sample_rate:
                return prefetch.take()
            prefetch.discard()  # the engine delivers another rate than it announced
        return self._make_output(sample_rate)
