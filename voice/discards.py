"""Discard reasons: why a wake detection or an utterance did not become a request.

One local log line per discard, a reason code and at most a wake score. Never
audio, never transcript text (speech-io, "Discard reasons").
"""

from __future__ import annotations

import logging
from collections import Counter
from enum import StrEnum

log = logging.getLogger("niva.voice.discard")


class Discard(StrEnum):
    WAKE_WHILE_SPEAKING = "wake_while_speaking"
    WAKE_WHILE_LISTENING = "wake_while_listening"
    WAKE_UNCLAIMED = "wake_unclaimed"  # woke, but the host never sent a listen
    WAKE_MIC_DOWN = "wake_mic_down"
    WAKE_NEAR_MISS = "wake_near_miss"  # a score just under the threshold
    NO_SPEECH = "no_speech"
    EMPTY_TRANSCRIPT = "empty_transcript"
    STALE_LISTEN = "stale_listen"  # a listen replaced or cancelled with audio in hand
    LISTEN_WHILE_MIC_DOWN = "listen_mic_down"
    STALE_CANCEL = "stale_cancel"  # cancel/force_end for an id that is not the open listen


class DiscardLog:
    def __init__(self) -> None:
        self.counts: Counter[str] = Counter()

    def record(self, reason: Discard, score: float | None = None) -> None:
        self.counts[reason.value] += 1
        if score is None:
            log.info("discard reason=%s", reason.value)
        else:
            log.info("discard reason=%s score=%.2f", reason.value, score)
