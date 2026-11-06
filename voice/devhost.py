"""A stand-in for the Niva host, for trying the sidecar alone (``--dev-host``).

After every ``wake`` it sends ``listen{command}``, and it speaks every non-empty
transcript back ("I heard: ..."). It is a test aid, not part of the protocol; the
real host is task 3.4.
"""

from __future__ import annotations

import itertools
import queue
from collections.abc import Callable
from typing import Any

from protocol import Listen, Speak

REPLIES = {"es": "Escuché: {text}", "en": "I heard: {text}"}


def wrap_emit(emit: Callable[[dict[str, Any]], None], inbox: queue.Queue[tuple[Any, ...]]) -> Callable[[dict], None]:
    ids = itertools.count(1)

    def wrapped(message: dict[str, Any]) -> None:
        emit(message)
        kind = message.get("type")
        if kind == "wake":
            inbox.put(("cmd", Listen(next(ids), "command")))
        elif kind == "transcript" and message.get("text"):
            template = REPLIES.get(message["lang"], REPLIES["en"])
            inbox.put(("cmd", Speak(next(ids), template.format(text=message["text"]), message["lang"])))

    return wrapped
