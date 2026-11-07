"""JSON-lines protocol between the Niva host and the speech sidecar.

One JSON object per line, UTF-8, on stdio. Every message is a flat object whose
``type`` field names it (the host side maps this to a serde tagged enum). The
sidecar never writes anything else to stdout; logs go to stderr.

Host -> sidecar (design.md, "Sidecar protocol")::

    {"type": "listen", "id": 7, "mode": "command" | "confirm"}
    {"type": "listen", "id": 8, "mode": "confirm", "lang": "es", "context": "..."}
                                            # lang and context are optional (added); only a
                                            # confirm listen uses them: the language of the
                                            # question the host just spoke and its text. The
                                            # sidecar forces that language and gives the text
                                            # to Whisper as its prompt. Never logged.
    {"type": "cancel_listen", "id": 7}
    {"type": "force_end", "id": 7}          # added: carries Effect::ForceEndOfSpeech
    {"type": "speak", "id": 8, "text": "...", "lang": "es" | "en"}
    {"type": "stop_speaking"}
    {"type": "shutdown"}

Sidecar -> host::

    {"type": "ready", "now_ms": 123456}      # sent once the microphone delivered its first frame;
                                             # now_ms added: the clock `end_ms` is on
    {"type": "wake", "score": 0.71}
    {"type": "speech_start", "id": 7}
    {"type": "transcript", "id": 7, "text": "...", "lang": "es", "lang_prob": 0.98, "end_ms": 123999}
    {"type": "no_speech", "id": 7}
    {"type": "speak_started", "id": 8}
    {"type": "speak_done", "id": 8}
    {"type": "error", "code": "MIC_BUSY", "msg": "...", "id": 7}   # id optional (added)

Ids are the host's unsigned integers and are echoed unchanged.
"""

from __future__ import annotations

import json
from dataclasses import dataclass
from enum import StrEnum
from typing import Any

LANGS = ("es", "en")
LISTEN_MODES = ("command", "confirm")
# Whisper keeps the END of a prompt when it is too long; so does this cut.
MAX_CONTEXT_CHARS = 200


class ErrorCode(StrEnum):
    """The sidecar error taxonomy. The first six make voice unusable (reducer
    ``SidecarError``); ``TTS_FAILED`` is per speak id (reducer ``TtsFailed``)."""

    MIC_MISSING = "MIC_MISSING"
    MIC_BUSY = "MIC_BUSY"
    CUDA_OOM = "CUDA_OOM"
    MODEL_LOAD = "MODEL_LOAD"
    TTS_FAILED = "TTS_FAILED"
    PROTOCOL = "PROTOCOL"
    INTERNAL = "INTERNAL"


class ProtocolError(ValueError):
    """A line from the host that cannot be understood."""


# ---------------------------------------------------------------- host -> sidecar


@dataclass(frozen=True)
class Listen:
    id: int
    mode: str
    lang: str | None = None  # confirm only: the language of the question just spoken
    context: str | None = None  # confirm only: the question's text (never logged)


@dataclass(frozen=True)
class CancelListen:
    id: int


@dataclass(frozen=True)
class ForceEnd:
    id: int


@dataclass(frozen=True)
class Speak:
    id: int
    text: str
    lang: str


@dataclass(frozen=True)
class StopSpeaking:
    pass


@dataclass(frozen=True)
class Shutdown:
    pass


Command = Listen | CancelListen | ForceEnd | Speak | StopSpeaking | Shutdown


def _uint(obj: dict[str, Any], key: str) -> int:
    value = obj.get(key)
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ProtocolError(f"field '{key}' must be an unsigned integer")
    return value


def _choice(obj: dict[str, Any], key: str, allowed: tuple[str, ...]) -> str:
    value = obj.get(key)
    if value not in allowed:
        raise ProtocolError(f"field '{key}' must be one of {list(allowed)}")
    return value


def _optional_choice(obj: dict[str, Any], key: str, allowed: tuple[str, ...]) -> str | None:
    if obj.get(key) is None:
        return None
    return _choice(obj, key, allowed)


def _optional_text(obj: dict[str, Any], key: str, limit: int) -> str | None:
    value = obj.get(key)
    if value is None:
        return None
    if not isinstance(value, str):
        raise ProtocolError(f"field '{key}' must be a string")
    return value[-limit:]


def parse_command(line: str) -> Command:
    """Parse one host line. Raises :class:`ProtocolError` on anything unreadable."""
    try:
        obj = json.loads(line)
    except json.JSONDecodeError as exc:
        raise ProtocolError(f"not JSON: {exc.msg}") from exc
    except RecursionError as exc:
        raise ProtocolError("nested too deeply") from exc
    if not isinstance(obj, dict):
        raise ProtocolError("message must be a JSON object")
    kind = obj.get("type")
    if kind == "listen":
        return Listen(
            _uint(obj, "id"),
            _choice(obj, "mode", LISTEN_MODES),
            _optional_choice(obj, "lang", LANGS),
            _optional_text(obj, "context", MAX_CONTEXT_CHARS),
        )
    if kind == "cancel_listen":
        return CancelListen(_uint(obj, "id"))
    if kind == "force_end":
        return ForceEnd(_uint(obj, "id"))
    if kind == "speak":
        text = obj.get("text")
        if not isinstance(text, str):
            raise ProtocolError("field 'text' must be a string")
        return Speak(_uint(obj, "id"), text, _choice(obj, "lang", LANGS))
    if kind == "stop_speaking":
        return StopSpeaking()
    if kind == "shutdown":
        return Shutdown()
    raise ProtocolError(f"unknown message type: {kind!r}")


# ---------------------------------------------------------------- sidecar -> host


def encode(message: dict[str, Any]) -> str:
    """One line of JSON, no trailing newline."""
    return json.dumps(message, ensure_ascii=False, separators=(",", ":"))


def ready(now_ms: int) -> dict[str, Any]:
    return {"type": "ready", "now_ms": now_ms}


def wake(score: float) -> dict[str, Any]:
    return {"type": "wake", "score": round(float(score), 3)}


def speech_start(listen_id: int) -> dict[str, Any]:
    return {"type": "speech_start", "id": listen_id}


def transcript(listen_id: int, text: str, lang: str, lang_prob: float, end_ms: int) -> dict[str, Any]:
    """``lang_prob`` is the combined es+en probability, reported for the logs only
    (no gate uses it any more, design D1); ``end_ms`` is the sidecar monotonic
    time (ms) of the last speech frame."""
    return {
        "type": "transcript",
        "id": listen_id,
        "text": text,
        "lang": lang,
        "lang_prob": round(float(lang_prob), 3),
        "end_ms": int(end_ms),
    }


def no_speech(listen_id: int) -> dict[str, Any]:
    return {"type": "no_speech", "id": listen_id}


def speak_started(speak_id: int) -> dict[str, Any]:
    return {"type": "speak_started", "id": speak_id}


def speak_done(speak_id: int) -> dict[str, Any]:
    return {"type": "speak_done", "id": speak_id}


def error(code: ErrorCode, msg: str, message_id: int | None = None) -> dict[str, Any]:
    out: dict[str, Any] = {"type": "error", "code": code.value, "msg": msg}
    if message_id is not None:
        out["id"] = message_id
    return out
