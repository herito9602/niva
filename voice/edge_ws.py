"""A persistent edge-tts websocket (latency: the per-utterance connect costs ~450 ms).

``edge_tts.Communicate`` opens a new ``ClientSession`` and websocket for every
utterance. This client keeps ONE connection open on a private event-loop thread,
connects it early (``prewarm``: the sidecar calls it when a listen starts and
when a transcript arrives) and reuses it for the next utterance. The wire
protocol is edge-tts 7.3.1's: the URL with its ``Sec-MS-GEC`` token, one
``speech.config`` message per connection, one SSML message per utterance, audio
frames until ``turn.end``. Voice, rate, pitch and volume are what
``Communicate(text, voice)`` sends by default.

Failure model (every one ends in "the caller uses the one-off edge-tts path, then
Piper", never in a silent reply):

* A connection idle for more than ``stale_after_s`` is replaced before use. The
  service was seen dropping an idle socket after 60 s and keeping it at 30 s.
* An error on a REUSED connection (one that was already connected when the turn
  started, prewarmed or not), before any audio, is retried once on a fresh one.
  An error on a fresh connection, or after audio started, is raised.
* Connecting is bounded (``connect_timeout_s``): a connect that hangs cannot hold
  the turn lock, so neither a prewarm nor a turn blocks the ones behind it forever.
* An utterance that is abandoned (barge-in, stop, device failure, timeout) drops
  the connection instead of draining it, so a half-read turn can never leak its
  frames into the next utterance.

Only this module and the ``EdgeTtsEngine`` wiring in tts.py use edge-tts
internals (``edge_tts.communicate``, ``drm``, ``constants``). The network is a
seam (``connect``), so tests drive a fake transport.
"""

from __future__ import annotations

import asyncio
import concurrent.futures
import logging
import queue
import threading
import time
from collections.abc import Awaitable, Callable, Iterator
from dataclasses import dataclass
from typing import Any, Protocol

log = logging.getLogger("niva.voice.edge_ws")

# edge-tts splits longer texts into several requests; the persistent path leaves those to it.
MAX_SSML_TEXT_BYTES = 4096

_DONE = object()


class EdgeWsTimeout(TimeoutError):
    """No audio frame arrived in time (the persistent path)."""


class UnsupportedText(ValueError):
    """The text needs edge-tts' multi-request splitting; the caller uses the one-off path."""


class WsProtocolError(Exception):
    """The service sent something this client does not understand."""


# ---------------------------------------------------------------- wire protocol


def config_message() -> str:
    """The ``speech.config`` message edge-tts sends first on a connection (sentence boundaries)."""
    from edge_tts.communicate import date_to_string

    return (
        f"X-Timestamp:{date_to_string()}\r\n"
        "Content-Type:application/json; charset=utf-8\r\n"
        "Path:speech.config\r\n\r\n"
        '{"context":{"synthesis":{"audio":{"metadataoptions":{'
        '"sentenceBoundaryEnabled":"true","wordBoundaryEnabled":"false"'
        "},"
        '"outputFormat":"audio-24khz-48kbitrate-mono-mp3"'
        "}}}}\r\n"
    )


def ssml_message(text: str, voice: str) -> str:
    """The SSML request edge-tts sends for ``Communicate(text, voice)`` with default prosody."""
    from edge_tts.communicate import (
        connect_id,
        date_to_string,
        escape,
        mkssml,
        remove_incompatible_characters,
        ssml_headers_plus_data,
    )
    from edge_tts.data_classes import TTSConfig

    escaped = escape(remove_incompatible_characters(text))
    if len(escaped.encode("utf-8")) > MAX_SSML_TEXT_BYTES:
        raise UnsupportedText(f"{len(escaped)} characters need more than one request")
    config = TTSConfig(voice, "+0%", "+0%", "+0Hz", "SentenceBoundary")
    return ssml_headers_plus_data(connect_id(), date_to_string(), mkssml(config, escaped))


def parse_frame(kind: str, payload: Any) -> tuple[str, bytes]:
    """Classify one websocket message: ``("audio", mp3)``, ``("end", b"")`` or ``("skip", b"")``."""
    if kind == "text":
        data = payload.encode("utf-8")
        head = data.split(b"\r\n\r\n", 1)[0]
        path = None
        for line in head.split(b"\r\n"):
            key, _, value = line.partition(b":")
            if key == b"Path":
                path = value
        if path == b"turn.end":
            return "end", b""
        if path in (b"audio.metadata", b"response", b"turn.start"):
            return "skip", b""
        raise WsProtocolError(f"unknown text message path {path!r}")
    if kind == "binary":
        if len(payload) < 2:
            raise WsProtocolError("binary frame without a header length")
        header_length = int.from_bytes(payload[:2], "big")
        if header_length > len(payload):
            raise WsProtocolError("binary frame header longer than the frame")
        headers = {}
        for line in payload[2 : 2 + header_length].split(b"\r\n"):
            key, _, value = line.partition(b":")
            headers[key] = value
        data = payload[2 + header_length :]
        if headers.get(b"Path") != b"audio":
            raise WsProtocolError("binary frame whose path is not audio")
        content_type = headers.get(b"Content-Type")
        if content_type is None:
            if data:
                raise WsProtocolError("audio data without a content type")
            return "skip", b""  # the empty terminator frame
        if content_type != b"audio/mpeg":
            raise WsProtocolError(f"unexpected audio content type {content_type!r}")
        if not data:
            raise WsProtocolError("audio frame without data")
        return "audio", data
    raise WsProtocolError(f"unexpected websocket message type {kind!r}")


# ---------------------------------------------------------------- transport


class Transport(Protocol):
    """One open websocket. ``receive`` returns ``("text", str)`` or ``("binary", bytes)``
    and raises ``ConnectionError`` when the socket is closed or failed."""

    async def send(self, text: str) -> None: ...

    async def receive(self) -> tuple[str, Any]: ...

    async def close(self) -> None: ...


Connect = Callable[[], Awaitable[Transport]]


class AiohttpTransport:
    def __init__(self, session: Any, ws: Any):
        self._session, self._ws = session, ws

    async def send(self, text: str) -> None:
        await self._ws.send_str(text)

    async def receive(self) -> tuple[str, Any]:
        import aiohttp

        message = await self._ws.receive()
        if message.type == aiohttp.WSMsgType.TEXT:
            return "text", message.data
        if message.type == aiohttp.WSMsgType.BINARY:
            return "binary", message.data
        if message.type == aiohttp.WSMsgType.ERROR:
            raise ConnectionError(f"websocket error: {message.data}")
        raise ConnectionError("websocket closed")

    async def close(self) -> None:
        for closer in (self._ws.close, self._session.close):
            try:
                await asyncio.wait_for(closer(), 3)
            except Exception:  # noqa: BLE001, S110 - closing a dead socket may fail or hang
                pass


async def connect_edge() -> Transport:
    """Open the edge-tts websocket exactly as ``Communicate`` does (including its
    one retry after a 403, which corrects the clock used by the token)."""
    import ssl

    import aiohttp
    import certifi
    from edge_tts.communicate import connect_id
    from edge_tts.constants import SEC_MS_GEC_VERSION, WSS_HEADERS, WSS_URL
    from edge_tts.drm import DRM

    context = ssl.create_default_context(cafile=certifi.where())
    timeout = aiohttp.ClientTimeout(total=None, connect=None, sock_connect=10, sock_read=60)
    for attempt in (0, 1):
        session = aiohttp.ClientSession(trust_env=True, timeout=timeout)
        try:
            ws = await session.ws_connect(
                f"{WSS_URL}&ConnectionId={connect_id()}"
                f"&Sec-MS-GEC={DRM.generate_sec_ms_gec()}"
                f"&Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}",
                compress=15,
                headers=DRM.headers_with_muid(WSS_HEADERS),
                ssl=context,
            )
            return AiohttpTransport(session, ws)
        except aiohttp.ClientResponseError as exc:
            await session.close()
            if exc.status != 403 or attempt == 1:
                raise
            DRM.handle_client_response_error(exc)
        except BaseException:
            await session.close()
            raise
    raise AssertionError("unreachable")


# ---------------------------------------------------------------- client


@dataclass
class _Connection:
    transport: Transport
    last_used: float
    configured: bool = False
    turns: int = 0


class _Turn:
    """One utterance handed to the loop thread; its frames come back through `frames`."""

    def __init__(self) -> None:
        self.frames: queue.Queue[Any] = queue.Queue()
        self.future: concurrent.futures.Future[None] | None = None
        self.audio_sent = False  # once true, a failure cannot be retried invisibly


class EdgeWsClient:
    def __init__(
        self,
        connect: Connect = connect_edge,
        *,
        stale_after_s: float = 30.0,
        # How long a REUSED socket gets to answer before it counts as half-open. Measured on
        # healthy sockets with a ~230-char reply: first frame 0.13 to 0.83 s, with outliers up
        # to ~2.1 s, so 0.8 s declared healthy sockets dead. 1.5 s keeps the dead-socket wait
        # below the consumer's first-chunk budget (`tts_first_chunk_s`, 2.0 s).
        reuse_first_frame_s: float = 1.5,
        frame_timeout_s: float = 10.0,
        connect_timeout_s: float = 8.0,
        clock: Callable[[], float] = time.monotonic,
    ):
        self._connect = connect
        self._stale_after = stale_after_s
        self._reuse_first_frame = reuse_first_frame_s
        self._frame_timeout = frame_timeout_s
        self._connect_timeout = connect_timeout_s
        self._clock = clock
        self._loop: asyncio.AbstractEventLoop | None = None
        self._loop_thread: threading.Thread | None = None
        self._closed = False
        self._loop_lock = threading.Lock()
        # Everything below is touched on the loop thread only.
        self._conn: _Connection | None = None
        self._turn_lock = asyncio.Lock()
        self._closing: set[asyncio.Future[None]] = set()

    # -- public, any thread

    def prewarm(self) -> None:
        """Connect (or replace a stale connection) in the background. Never blocks, never raises."""
        try:
            loop = self._ensure_loop()
            asyncio.run_coroutine_threadsafe(self._prewarm(), loop)
        except Exception as exc:  # noqa: BLE001 - a missed prewarm only costs the connect time later
            log.debug("edge-tts prewarm not scheduled: %s", exc)

    def stream(
        self, text: str, voice: str, cancel: threading.Event, first_timeout_s: float, idle_timeout_s: float
    ) -> Iterator[bytes]:
        """MP3 chunks for one utterance. Raises `EdgeWsTimeout` when the first chunk (or a later
        gap) takes too long, `UnsupportedText` for texts the one-off path must split, and
        whatever the connection raised once the retry is spent."""
        ssml_message(text, voice)  # validates now: UnsupportedText must not cost a loop round trip
        turn = _Turn()
        loop = self._ensure_loop()
        turn.future = asyncio.run_coroutine_threadsafe(self._run_turn(turn, text, voice), loop)
        finished = False
        timeout = first_timeout_s
        try:
            while True:
                if cancel.is_set():
                    return
                try:
                    item = turn.frames.get(timeout=timeout)
                except queue.Empty:
                    raise EdgeWsTimeout(f"no audio from the persistent connection for {timeout:g} s") from None
                timeout = idle_timeout_s
                if item is _DONE:
                    finished = True
                    return
                if isinstance(item, BaseException):
                    finished = True
                    raise item
                yield item
        finally:
            if not finished and turn.future is not None:
                turn.future.cancel()  # abandoned: the loop side drops the connection

    def close(self) -> None:
        """Close the connection, stop the loop thread and close the loop. Later calls are no-ops
        (a `prewarm` after `close` schedules nothing; a `stream` raises and the caller falls back)."""
        with self._loop_lock:
            self._closed = True
            loop, thread = self._loop, self._loop_thread
            self._loop = self._loop_thread = None
        if loop is None:
            return
        try:
            asyncio.run_coroutine_threadsafe(self._shutdown(), loop).result(5)
        except Exception:  # noqa: BLE001, S110 - shutting down
            pass
        loop.call_soon_threadsafe(loop.stop)
        if thread is not None:
            thread.join(5)
        if not loop.is_running():
            loop.close()

    # -- loop thread

    def _ensure_loop(self) -> asyncio.AbstractEventLoop:
        with self._loop_lock:
            if self._closed:
                raise RuntimeError("the edge-tts client is closed")
            if self._loop is None:
                loop = asyncio.new_event_loop()
                self._loop_thread = threading.Thread(target=loop.run_forever, name="edge-ws", daemon=True)
                self._loop_thread.start()
                self._loop = loop
            return self._loop

    def _fresh(self, conn: _Connection) -> bool:
        return self._clock() - conn.last_used <= self._stale_after

    async def _shutdown(self) -> None:
        await self._drop()
        if self._closing:
            await asyncio.wait(list(self._closing), timeout=3)

    async def _prewarm(self) -> None:
        if self._turn_lock.locked():
            return  # an utterance is using (or opening) the connection
        async with self._turn_lock:
            try:
                await self._connection()
            except Exception as exc:  # noqa: BLE001
                log.debug("edge-tts prewarm failed: %s", exc)

    async def _connection(self) -> _Connection:
        """The live connection, replacing a stale one. Call with the turn lock held."""
        conn = self._conn
        if conn is not None and not self._fresh(conn):
            log.debug("edge-tts connection idle for more than %g s: reconnecting", self._stale_after)
            await self._drop()
            conn = None
        if conn is None:
            try:
                transport = await asyncio.wait_for(self._connect(), self._connect_timeout)
            except TimeoutError:
                raise EdgeWsTimeout(f"no connection within {self._connect_timeout:g} s") from None
            conn = self._conn = _Connection(transport, self._clock())
        return conn

    async def _drop(self) -> None:
        """Forget the connection and close it in the background (a close handshake with a
        dead peer can take seconds, and the next utterance must not wait for it)."""
        conn, self._conn = self._conn, None
        if conn is not None:
            closing = asyncio.ensure_future(conn.transport.close())
            self._closing.add(closing)
            closing.add_done_callback(self._closing.discard)

    async def _run_turn(self, turn: _Turn, text: str, voice: str) -> None:
        try:
            async with self._turn_lock:
                await self._turn_with_retry(turn, text, voice)
        except Exception as exc:  # noqa: BLE001 - handed to the consumer (a cancel is not an Exception)
            turn.frames.put(exc)

    async def _turn_with_retry(self, turn: _Turn, text: str, voice: str) -> None:
        for attempt in (0, 1):
            # "Reused" = connected before this turn started (a prewarmed socket that never carried
            # an utterance can be just as half-open as a used one).
            reused = self._conn is not None and self._fresh(self._conn)
            try:
                conn = await self._connection()
                await self._exchange(conn, turn, text, voice, reused)
                turn.frames.put(_DONE)
                return
            except asyncio.CancelledError:
                await self._drop()  # abandoned mid-turn: never reuse a socket with frames in flight
                raise
            except Exception as exc:
                await self._drop()
                # Only a reused connection can be merely stale, and only before any audio went out.
                if attempt == 1 or not reused or turn.audio_sent or isinstance(exc, UnsupportedText):
                    raise
                log.info("edge-tts connection failed (%s: %s); retrying on a fresh one", type(exc).__name__, exc)

    async def _exchange(self, conn: _Connection, turn: _Turn, text: str, voice: str, reused: bool) -> None:
        """One request and its audio, up to ``turn.end``."""
        if not conn.configured:
            await conn.transport.send(config_message())
            conn.configured = True
        await conn.transport.send(ssml_message(text, voice))
        first = True
        while True:
            limit = self._reuse_first_frame if first and reused else self._frame_timeout
            try:
                kind, payload = await asyncio.wait_for(conn.transport.receive(), limit)
            except TimeoutError:
                state = (
                    f"reused socket, {self._clock() - conn.last_used:.1f} s idle, {conn.turns} turns done"
                    if reused
                    else "new socket"
                )
                raise EdgeWsTimeout(f"no frame within {limit:g} s ({state})") from None
            event, data = parse_frame(kind, payload)
            if event == "audio":
                first = False
                turn.audio_sent = True
                turn.frames.put(data)
            elif event == "end":
                conn.turns += 1
                conn.last_used = self._clock()
                return
