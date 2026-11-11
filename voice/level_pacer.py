"""Paces the band levels of Niva's voice to the moment the sound is AUDIBLE (design L13).

Writing PCM to the output device is not hearing it: the device holds `latency`
seconds of audio ahead of the loudspeaker. A level message sent when the PCM is
written would make the waveform move before the sound, so the pacer keeps a
model of the playback clock and emits each 32 ms slice when it is heard.

The clock model, per written chunk (``write_return`` is when the blocking
write came back, ``latency`` is the stream's output latency):

* The first chunk anchors the clock: its first sample is audible at
  ``write_return + latency``.
* While the device keeps playing, the next chunk simply follows the previous
  one (``prev_end``). This is the design's ``prev_due + slice_ms``.
* A write that blocked was waiting for room, which means the buffer was full:
  the chunk's END is audible about ``latency`` after the write returned, so its
  start is ``write_return + latency - blocked`` (``blocked`` is how long the
  write took, at most the chunk's duration). A write that did not block went to
  an empty buffer and is audible ``latency`` after it returned.
* When that estimate is later than ``prev_end`` by more than ``TOLERANCE_S``,
  the device ran dry (an underrun: the network stalled): the clock is
  re-anchored on the estimate. A smaller gap is ignored, which bounds how early
  a bar can show to ``TOLERANCE_S`` instead of drifting.

Time is ``time.perf_counter``: on Windows ``time.monotonic`` ticks every ~15.6 ms,
which would put the bars up to a frame off before anything else did.

``NIVA_TTS_LEVEL_OFFSET_MS`` shifts every due time, for latency a device does
not report (Bluetooth). A negative value makes the bars earlier.
"""

from __future__ import annotations

import logging
import math
import os
import threading
import time
from collections.abc import Callable, Mapping
from dataclasses import dataclass

import levels

log = logging.getLogger("niva.voice.levels")

SLICE_MS = 32
# At most one emit per this long, whatever the slices say (the host and the
# webview should never see a flood).
MIN_EMIT_GAP_S = 0.025
# A chunk estimated later than the continuous schedule by more than this means the
# device ran dry: the clock is anchored again. Also the most a bar can lead.
TOLERANCE_S = 0.025
# `time.sleep` is precise (high resolution timer); a condition wait is coarse. The thread
# waits on the condition until this long before the due time, then sleeps the rest.
_FINE_S = 0.016
OFFSET_ENV = "NIVA_TTS_LEVEL_OFFSET_MS"
_MAX_OFFSET_MS = 500.0


def level_offset_ms(environ: Mapping[str, str] | None = None) -> float:
    """The owner's correction for latency the device does not report. Anything
    unreadable or beyond +-500 ms is ignored (0)."""
    raw = (os.environ if environ is None else environ).get(OFFSET_ENV, "")
    try:
        value = float(raw)
    except ValueError:
        return 0.0
    return value if math.isfinite(value) and abs(value) <= _MAX_OFFSET_MS else 0.0


@dataclass(frozen=True)
class Slice:
    due: float  # perf_counter seconds at which this slice is audible (offset included)
    bands: list[int]
    samples: bytes


class LevelPacer:
    """Cuts written chunks into 32 ms slices and emits their levels on time.

    ``emit(bands)`` is called from the pacer's thread. With ``threaded=False``
    nothing runs by itself and tests call :meth:`pump` with a fake clock."""

    def __init__(
        self,
        emit: Callable[[list[int]], None],
        clock: Callable[[], float] = time.perf_counter,
        offset_ms: float = 0.0,
        threaded: bool = True,
    ) -> None:
        self._emit = emit
        self._clock = clock
        self._offset_s = offset_ms / 1000.0
        self._threaded = threaded
        self._cond = threading.Condition()
        self._pending: list[Slice] = []
        self._stopped = False
        self._thread: threading.Thread | None = None
        self._end: float | None = None  # model time at which everything written so far has been heard
        self._carry = b""  # less than one slice, waiting for the next chunk
        self._last_emit: float | None = None

    # -- input

    @property
    def pending(self) -> list[Slice]:
        with self._cond:
            return list(self._pending)

    @property
    def running(self) -> bool:
        thread = self._thread
        return thread is not None and thread.is_alive()

    def on_write(
        self, pcm: bytes, rate: int, write_return: float, latency: float, write_start: float | None = None
    ) -> None:
        """A chunk of int16 mono PCM was handed to the device (the write has returned)."""
        pcm = pcm[: len(pcm) // 2 * 2]
        if not pcm or rate <= 0:
            return
        duration = len(pcm) // 2 / rate
        blocked = 0.0 if write_start is None else min(max(write_return - write_start, 0.0), duration)
        slice_bytes = rate * SLICE_MS // 1000 * 2
        slice_s = slice_bytes / 2 / rate
        with self._cond:
            if self._stopped:
                return
            if self._end is None:
                start, self._carry = write_return + latency, b""  # the first write anchors the clock
            else:
                estimate = write_return + latency - blocked
                if estimate > self._end + TOLERANCE_S:  # the device ran dry: anchor again
                    start, self._carry = estimate, b""
                else:
                    start = self._end
            carry_start = start - len(self._carry) / 2 / rate
            data = self._carry + pcm
            index = 0
            while len(data) - index * slice_bytes >= slice_bytes:
                chunk = data[index * slice_bytes : (index + 1) * slice_bytes]
                self._pending.append(
                    Slice(carry_start + index * slice_s + self._offset_s, levels.bands(chunk, rate), chunk)
                )
                index += 1
            self._carry = data[index * slice_bytes :]
            self._end = start + duration
            self._start_thread()
            self._cond.notify_all()

    # -- output

    def pump(self) -> None:
        """Emit what is due now (the thread does this by itself)."""
        with self._cond:
            item, _ = self._next(self._clock())
        if item is not None:
            self._send(item)

    def finish(self) -> None:
        """The speech ended: what is due goes out, what is not never will."""
        self.pump()
        self.stop()

    def stop(self) -> None:
        """Barge-in or shutdown: drop everything and stop now."""
        with self._cond:
            self._stopped = True
            self._pending.clear()
            self._carry = b""
            self._cond.notify_all()
            thread = self._thread
        if thread is not None and thread is not threading.current_thread():
            thread.join(0.2)

    # -- internals

    def _next(self, now: float) -> tuple[Slice | None, float | None]:
        """The slice to send now (the newest of those due: an older one is stale),
        or how long to wait. Call with the lock held."""
        if self._stopped:
            return None, None
        due = 0
        while due < len(self._pending) and self._pending[due].due <= now:
            due += 1
        if due:
            wait = None if self._last_emit is None else self._last_emit + MIN_EMIT_GAP_S - now
            if wait is not None and wait > 0:
                return None, wait
            item = self._pending[due - 1]
            del self._pending[:due]
            self._last_emit = now
            return item, None
        if self._pending:
            return None, self._pending[0].due - now
        return None, None

    def _send(self, item: Slice) -> None:
        try:
            self._emit(item.bands)
        except Exception:  # noqa: BLE001 - a level is decoration; it must never break the speech
            log.debug("a level could not be sent", exc_info=True)

    def _start_thread(self) -> None:
        if self._threaded and self._thread is None:
            self._thread = threading.Thread(target=self._run, name="level-pacer", daemon=True)
            self._thread.start()

    def _run(self) -> None:
        while True:
            pause = 0.0
            item = None
            with self._cond:
                if self._stopped:
                    return
                item, wait = self._next(self._clock())
                if item is None:
                    if wait is not None and wait <= _FINE_S:
                        pause = wait  # close to the due time: sleep it out with the precise timer
                    else:
                        # A condition wait on Windows can overshoot by one timer tick (~16 ms),
                        # which would show the bars late: leave the last stretch to `sleep`.
                        self._cond.wait(None if wait is None else wait - _FINE_S)
            if item is not None:
                self._send(item)
            elif pause > 0:
                time.sleep(pause)  # at most _FINE_S: a stop waits for this, never for a whole period
