"""The capture loop: one microphone stream feeds wake word, VAD and STT.

``Engine`` is a plain, single-threaded state machine. The caller (main.py) feeds
it microphone frames, host commands and speech-end notices from one queue, so the
engine needs no locks and tests drive it frame by frame with fakes.

What it does, in the order audio flows (design.md, "Sidecar capture loop"):

* Every frame goes into a rolling in-memory buffer (``pre_speech_ms``). It is
  never written to disk and is cleared when a turn ends or Niva stops speaking.
* The wake detector reads the same frames. A detection while Niva speaks or while
  a listen is open is ignored and logged. Otherwise ``wake`` is sent and the engine
  keeps recording from ``wake_preroll_ms`` before the detection, so "Hey Niva, open
  Spotify" in one breath reaches STT whole when the host's ``listen`` arrives.
* A ``listen`` starts from that recording (or from the rolling buffer for a
  shortcut) and replays it through the VAD, then follows live frames. When speech
  is confirmed, the audio sent to STT starts ``pre_speech_ms`` before the onset.
"""

from __future__ import annotations

import logging
from collections import deque
from collections.abc import Callable
from dataclasses import dataclass, field
from typing import Any, Protocol

import numpy as np

import protocol
from config import FRAME_MS, Tunables
from discards import Discard, DiscardLog
from protocol import (
    CancelListen,
    Command,
    ErrorCode,
    ForceEnd,
    Listen,
    Shutdown,
    Speak,
    StopSpeaking,
)
from stt import SttError, SttResult, classify_runtime_error
from vad import End, Endpointer, Onset, SpeechModel
from wake import WakeDetector

log = logging.getLogger("niva.voice.capture")

Emit = Callable[[dict[str, Any]], None]


class Transcriber(Protocol):
    def transcribe(
        self, audio: np.ndarray, *, mode: str = "command", lang: str | None = None, context: str | None = None
    ) -> SttResult: ...


class SpeakerPort(Protocol):
    def speak(self, speak_id: int, text: str, lang: str) -> None: ...

    def stop(self) -> None: ...


class SidecarFatal(Exception):
    """An error after which the sidecar cannot keep serving (the host restarts it)."""

    def __init__(self, code: ErrorCode, msg: str, message_id: int | None = None):
        super().__init__(msg)
        self.code = code
        self.msg = msg
        self.message_id = message_id


# A frame with the time the capture thread got it, so a replay keeps the real times.
_Stamped = tuple[np.ndarray, int]


@dataclass
class _Listen:
    id: int
    mode: str
    lead: deque[np.ndarray]
    lang: str | None = None  # confirm only: the language of the question the host spoke
    context: str | None = None  # confirm only: its text. Never logged.
    utterance: list[np.ndarray] = field(default_factory=list)
    in_speech: bool = False
    waited_frames: int = 0  # live frames since the listen opened with no live speech yet
    speech_frames: int = 0
    # Speech found only in the replayed pre-roll (often the wake word's own tail) is
    # provisional: it counts as a turn only once a live frame is speech too.
    live_speech: bool = False
    announced: bool = False


class Engine:
    def __init__(
        self,
        *,
        wake: WakeDetector,
        vad: SpeechModel,
        stt: Transcriber,
        speaker: SpeakerPort,
        emit: Emit,
        discards: DiscardLog,
        tunables: Tunables,
        clock_ms: Callable[[], int],
    ):
        self._wake = wake
        self._vad = vad
        self._stt = stt
        self._speaker = speaker
        self._emit = emit
        self._discards = discards
        self._t = tunables
        self._clock = clock_ms
        self._endpointer = Endpointer(tunables)
        self._ring: deque[_Stamped] = deque(maxlen=max(1, tunables.pre_speech_frames, tunables.wake_preroll_frames))
        self._pending: list[_Stamped] | None = None  # audio recorded since a wake detection
        self._pending_pre = 0  # how many of those are pre-roll (up to the detection itself)
        self._listen: _Listen | None = None
        self._speaking_id: int | None = None
        self._near_peak = 0.0
        self._mic_down: tuple[ErrorCode, str] | None = None
        self._awaiting_first_frame = False
        self._frame_ms: int | None = None  # capture time of the newest frame

    # ------------------------------------------------------------ introspection

    @property
    def listening(self) -> bool:
        return self._listen is not None

    @property
    def speaking(self) -> bool:
        return self._speaking_id is not None

    # ------------------------------------------------------------ microphone life

    def on_mic_ok(self) -> None:
        """The stream is open (first time, or reopened). `ready` waits for the first
        real frame: a device that opens but delivers nothing is not ready."""
        self._ring.clear()
        self._pending = None
        self._listen = None
        self._vad.reset()
        self._wake.reset()
        self._awaiting_first_frame = True

    def on_mic_error(self, code: ErrorCode, msg: str) -> None:
        """The stream failed. Report it once, with the id of the listen it cut short.
        The same failure again, with no audio in between, is not news."""
        listen_id = self._listen.id if self._listen else None
        self._listen = None
        self._pending = None
        self._awaiting_first_frame = False
        repeated = self._mic_down == (code, msg)
        self._mic_down = (code, msg)
        if not repeated:
            self._emit(protocol.error(code, msg, listen_id))

    # ------------------------------------------------------------ host commands

    def on_command(self, command: Command) -> None:
        if isinstance(command, Listen):
            self._start_listen(command)
        elif isinstance(command, CancelListen):
            if self._listen and self._listen.id == command.id:
                self._discards.record(Discard.STALE_LISTEN)
                self._end_listen()
            else:
                self._discards.record(Discard.STALE_CANCEL)
        elif isinstance(command, ForceEnd):
            if self._listen and self._listen.id == command.id:
                self._force_end()
            else:
                self._discards.record(Discard.STALE_CANCEL)
        elif isinstance(command, Speak):
            if self._speaking_id is not None:
                self._speaker.stop()
            self._speaking_id = command.id
            self._speaker.speak(command.id, command.text, command.lang)
        elif isinstance(command, StopSpeaking):
            if self._speaking_id is not None:
                self._speaker.stop()
                self._speech_over()
        elif isinstance(command, Shutdown):  # handled by the caller; harmless here
            pass

    def on_speak_ended(self, speak_id: int) -> None:
        """The speaker thread finished (done, failed or cut off) an utterance."""
        if speak_id == self._speaking_id:
            self._speech_over()

    def _speech_over(self) -> None:
        # What the microphone heard while Niva spoke is her own voice: drop it so it
        # can neither trigger the wake word nor lead the next utterance.
        self._speaking_id = None
        self._ring.clear()
        self._wake.reset()

    # ------------------------------------------------------------ frames

    def on_frame(self, frame: np.ndarray, now_ms: int | None = None) -> None:
        """``now_ms`` is when the capture thread got the frame (defaults to now)."""
        self._frame_ms = now_ms if now_ms is not None else self._clock()
        if self._awaiting_first_frame:
            self._awaiting_first_frame = False
            self._mic_down = None
            self._emit(protocol.ready(self._frame_ms))
        self._ring.append((frame, self._frame_ms))
        recording = self._pending is not None
        self._watch_wake(frame)
        if self._listen is not None:
            self._feed_listen(frame, replay=False)
        elif recording and self._pending is not None:
            # (a pending list created by this very frame already contains it)
            self._pending.append((frame, self._frame_ms))
            if len(self._pending) * FRAME_MS >= self._t.wake_claim_s * 1000:
                self._discards.record(Discard.WAKE_UNCLAIMED)
                self._pending = None

    def _watch_wake(self, frame: np.ndarray) -> None:
        t = self._t
        score = self._wake.score(frame)
        if score >= t.wake_threshold:
            self._wake.reset()
            self._near_peak = 0.0
            if self._speaking_id is not None:
                self._discards.record(Discard.WAKE_WHILE_SPEAKING, score)
            elif self._listen is not None or self._pending is not None:
                self._discards.record(Discard.WAKE_WHILE_LISTENING, score)
            else:
                preroll = self._t.wake_preroll_frames
                self._pending = list(self._ring)[-preroll:] if preroll else [(frame, self._frame_ms)]
                self._pending_pre = len(self._pending)
                self._emit(protocol.wake(score))
        elif score >= t.wake_near_miss_floor:
            self._near_peak = max(self._near_peak, score)
        elif self._near_peak:
            self._discards.record(Discard.WAKE_NEAR_MISS, self._near_peak)
            self._near_peak = 0.0

    # ------------------------------------------------------------ listening

    def _start_listen(self, command: Listen) -> None:
        if self._mic_down is not None:
            code, msg = self._mic_down
            self._discards.record(Discard.LISTEN_WHILE_MIC_DOWN)
            self._emit(protocol.error(code, msg, command.id))
            return
        t = self._t
        history: list[_Stamped]
        preroll = None  # frames of `history` that are old audio (the rest was heard after the wake)
        if self._speaking_id is not None:
            # The reply's tail is still in the buffer (or the speech-end notice is late):
            # she must not hear herself. Forget the speech and start from live audio.
            self._speech_over()
            history = []
        elif self._listen is not None:
            # A replacing listen: the buffer holds the old turn's speech, not new audio.
            self._discards.record(Discard.STALE_LISTEN)
            history = []
        elif self._pending is not None:
            history = self._pending
            preroll = self._pending_pre
        else:
            history = list(self._ring)[-t.pre_speech_frames :] if t.pre_speech_frames else []
        self._pending = None
        self._vad.reset()
        self._endpointer.reset()
        self._listen = _Listen(
            id=command.id,
            mode=command.mode,
            lead=deque(maxlen=t.pre_speech_frames + t.onset_frames),
            lang=command.lang if command.mode == "confirm" else None,
            context=command.context if command.mode == "confirm" else None,
        )
        newest = self._frame_ms
        for index, (old, stamp) in enumerate(history):
            if self._listen is None:
                break
            self._frame_ms = stamp  # end_ms must come from the replayed frame itself
            # Only audio from before the wake detection is provisional; what the user
            # said after it (a command in one breath) is a real turn.
            self._feed_listen(old, replay=preroll is None or index < preroll)
        self._frame_ms = newest

    def _feed_listen(self, frame: np.ndarray, *, replay: bool) -> None:
        listen = self._listen
        assert listen is not None
        t = self._t
        prob = self._vad(frame.astype(np.float32) / 32768.0)
        signal = self._endpointer.push(prob)

        if not listen.in_speech:
            listen.lead.append(frame)
            if isinstance(signal, Onset):
                listen.in_speech = True
                listen.utterance = list(listen.lead)
                listen.speech_frames = len(listen.utterance)
        else:
            listen.utterance.append(frame)
            listen.speech_frames += 1

        # Live speech means a live frame while the turn is in speech (one loud frame
        # before the onset is confirmed does not stop the no-speech clock).
        if not replay and listen.in_speech and prob >= t.vad_threshold:
            listen.live_speech = True

        if listen.in_speech and listen.live_speech and not listen.announced:
            listen.announced = True
            self._emit(protocol.speech_start(listen.id))

        if isinstance(signal, End) and not listen.live_speech:
            # The "speech" was only the pre-roll (the wake word's tail): not a turn.
            listen.in_speech = False
            listen.utterance = []
            listen.lead.clear()
            listen.speech_frames = 0
        elif isinstance(signal, End):
            self._complete(signal.silence_frames_ago)
            return
        elif listen.in_speech and listen.live_speech and listen.speech_frames * FRAME_MS >= t.max_utterance_s * 1000:
            self._force_end()
            return

        if not listen.live_speech and not replay:
            listen.waited_frames += 1
            if listen.waited_frames * FRAME_MS >= t.no_speech_s * 1000:
                self._discards.record(Discard.NO_SPEECH)
                self._emit(protocol.no_speech(listen.id))
                self._end_listen()

    def _force_end(self) -> None:
        listen = self._listen
        assert listen is not None
        if listen.in_speech and listen.live_speech:
            self._complete(None)
        else:
            self._discards.record(Discard.NO_SPEECH)
            self._emit(protocol.no_speech(listen.id))
            self._end_listen()

    def _end_listen(self) -> None:
        self._listen = None
        self._pending = None
        self._ring.clear()  # audio already consumed must not lead the next turn

    def _complete(self, silence_frames_ago: int | None) -> None:
        listen = self._listen
        assert listen is not None
        t = self._t
        now = self._frame_ms if self._frame_ms is not None else self._clock()
        frames = listen.utterance
        if silence_frames_ago is None:
            end_ms = now
        else:
            end_ms = now - (silence_frames_ago + 1) * FRAME_MS
            keep = len(frames) - (silence_frames_ago + 1) + t.trailing_pad_frames
            frames = frames[: max(1, min(len(frames), keep))]
        self._end_listen()

        audio = np.concatenate(frames).astype(np.float32) / 32768.0
        try:
            result = self._stt.transcribe(audio, mode=listen.mode, lang=listen.lang, context=listen.context)
        except SttError as exc:
            raise SidecarFatal(exc.code, exc.msg, listen.id) from exc
        except Exception as exc:  # noqa: BLE001 - classify whatever the runtime raised
            raise SidecarFatal(classify_runtime_error(exc), f"speech-to-text failed: {exc}", listen.id) from exc

        text = result.text.strip()
        if result.dropped_segments:
            self._discards.record(Discard.HALLUCINATION_GATE)
        if not text:
            self._discards.record(Discard.EMPTY_TRANSCRIPT)
        self._emit(protocol.transcript(listen.id, text, result.lang, result.lang_prob, end_ms))
