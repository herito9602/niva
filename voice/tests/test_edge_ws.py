"""The persistent edge-tts websocket, against a fake transport: no network."""

import asyncio
import threading
import time

import pytest

from config import Tunables
from edge_ws import (
    EdgeWsClient,
    EdgeWsTimeout,
    UnsupportedText,
    WsProtocolError,
    config_message,
    parse_frame,
    ssml_message,
)
from tts import EdgeTtsEngine, Speaker, TtsTimeout

VOICE = "es-US-PalomaNeural"

# ------------------------------------------------------------------ wire format


def audio_frame(payload: bytes):
    header = b"X-RequestId:abc\r\nContent-Type:audio/mpeg\r\nX-StreamId:1\r\nPath:audio\r\n"
    return "binary", len(header).to_bytes(2, "big") + header + payload


END = ("text", "X-RequestId:abc\r\nPath:turn.end\r\n\r\n{}")
TURN_START = ("text", "X-RequestId:abc\r\nPath:turn.start\r\n\r\n{}")
METADATA = ("text", "X-RequestId:abc\r\nPath:audio.metadata\r\n\r\n{}")


def test_frames_are_classified_like_edge_tts_does():
    assert parse_frame(*audio_frame(b"mp3")) == ("audio", b"mp3")
    assert parse_frame(*END) == ("end", b"")
    assert parse_frame(*TURN_START) == ("skip", b"") and parse_frame(*METADATA) == ("skip", b"")
    terminator = b"X-RequestId:abc\r\nPath:audio\r\n"
    assert parse_frame("binary", len(terminator).to_bytes(2, "big") + terminator) == ("skip", b"")


@pytest.mark.parametrize(
    ("kind", "payload"),
    [
        ("text", "Path:something.new\r\n\r\n{}"),
        ("binary", b"\x00"),
        ("binary", (500).to_bytes(2, "big") + b"short"),
        ("binary", (9).to_bytes(2, "big") + b"Path:text" + b"data"),
        ("binary", (10).to_bytes(2, "big") + b"Path:audio" + b"data"),  # data without a content type
        ("other", b""),
    ],
)
def test_unexpected_frames_are_protocol_errors_so_the_caller_falls_back(kind, payload):
    with pytest.raises(WsProtocolError):
        parse_frame(kind, payload)


def test_the_request_is_what_communicate_sends_by_default():
    from edge_tts import Communicate
    from edge_tts.communicate import mkssml

    text = "Hola & chau <3"
    communicate = Communicate(text, VOICE)
    expected = mkssml(communicate.tts_config, list(communicate.texts)[0])
    message = ssml_message(text, VOICE)
    assert message.endswith(expected) and "Path:ssml" in message and "Content-Type:application/ssml+xml" in message
    for fragment in ("rate='+0%'", "pitch='+0Hz'", "volume='+0%'", "Paloma"):
        assert fragment in message


def test_the_connection_config_asks_for_the_same_24khz_mp3():
    message = config_message()
    assert "Path:speech.config" in message and "audio-24khz-48kbitrate-mono-mp3" in message
    assert '"sentenceBoundaryEnabled":"true"' in message


def test_a_text_that_needs_several_requests_is_left_to_the_one_off_path():
    with pytest.raises(UnsupportedText):
        ssml_message("palabra " * 600, VOICE)


# ------------------------------------------------------------------ fake service


class FakeTransport:
    """A connection whose replies are scripted per request: ok / die / hang / partial_die / partial_hang."""

    def __init__(self, service, plan):
        self.service, self.plan = service, list(plan)
        self.inbox: asyncio.Queue = asyncio.Queue()
        self.sent: list[str] = []
        self.closed = False
        self.late_done = threading.Event()  # the "still in flight" frames of partial_unfinished arrived

    async def send(self, text):
        if self.closed:
            raise ConnectionError("socket is closed")
        self.sent.append(text)
        if "Path:ssml" not in text:
            return
        action = self.plan.pop(0) if self.plan else "ok"
        if action == "ok":
            for item in (TURN_START, audio_frame(b"AA"), METADATA, audio_frame(b"BB"), END):
                self.inbox.put_nowait(item)
        elif action == "die":
            self.inbox.put_nowait(ConnectionError("server closed the socket"))
        elif action in ("partial_die", "partial_hang"):
            self.inbox.put_nowait(audio_frame(b"AA"))
            if action == "partial_die":
                self.inbox.put_nowait(ConnectionError("lost mid-stream"))
        elif action == "partial_unfinished":  # the rest of the turn is still in flight
            self.inbox.put_nowait(audio_frame(b"AA"))
            asyncio.get_running_loop().call_later(0.2, self._late_frames)
        # "hang": nothing comes back

    def _late_frames(self):
        self.inbox.put_nowait(audio_frame(b"STALE"))
        self.inbox.put_nowait(END)
        self.late_done.set()

    async def receive(self):
        item = await self.inbox.get()
        if isinstance(item, Exception):
            raise item
        return item

    async def close(self):
        self.closed = True


class FakeService:
    def __init__(self, *plans):
        self.plans = list(plans)  # one list of per-request behaviours per connection
        self.connections: list[FakeTransport] = []

    async def connect(self):
        plan = self.plans.pop(0) if self.plans else []
        transport = FakeTransport(self, plan)
        self.connections.append(transport)
        return transport

    @property
    def ssml_requests(self):
        return [sent for c in self.connections for sent in c.sent if "Path:ssml" in sent]


class Clock:
    def __init__(self):
        self.now = 1000.0

    def __call__(self):
        return self.now


def make(*plans, **kwargs):
    service, clock = FakeService(*plans), Clock()
    kwargs.setdefault("reuse_first_frame_s", 0.2)
    kwargs.setdefault("frame_timeout_s", 0.5)
    client = EdgeWsClient(service.connect, clock=clock, **kwargs)
    return client, service, clock


def speak(client, text="Hola.", first=1.0, idle=1.0, cancel=None):
    return b"".join(client.stream(text, VOICE, cancel or threading.Event(), first, idle))


def wait_for(predicate, seconds=2.0):
    deadline = time.time() + seconds
    while not predicate() and time.time() < deadline:
        time.sleep(0.01)
    return predicate()


@pytest.fixture(autouse=True)
def _close_clients():
    clients = []
    original = EdgeWsClient.__init__

    def tracking(self, *args, **kwargs):
        original(self, *args, **kwargs)
        clients.append(self)

    EdgeWsClient.__init__ = tracking
    yield
    EdgeWsClient.__init__ = original
    for client in clients:
        client.close()


# ------------------------------------------------------------------ the connection


def test_one_connection_serves_several_utterances_and_configures_once():
    client, service, _ = make()
    assert speak(client) == b"AABB"
    assert speak(client, "Otra vez.") == b"AABB"
    assert len(service.connections) == 1
    sent = service.connections[0].sent
    assert sum("Path:speech.config" in s for s in sent) == 1 and len(service.ssml_requests) == 2


def test_prewarm_connects_before_the_utterance_and_the_utterance_reuses_it():
    client, service, _ = make()
    client.prewarm()
    assert wait_for(lambda: len(service.connections) == 1)
    client.prewarm()  # a fresh connection is not replaced
    assert speak(client) == b"AABB"
    assert len(service.connections) == 1


def test_an_idle_connection_is_replaced_before_use():
    client, service, clock = make()
    speak(client)
    clock.now += 31
    assert speak(client) == b"AABB"
    assert len(service.connections) == 2 and wait_for(lambda: service.connections[0].closed)
    clock.now += 10  # still within the limit: reused
    speak(client)
    assert len(service.connections) == 2


def test_prewarm_replaces_a_stale_connection_early():
    client, service, clock = make()
    speak(client)
    clock.now += 45
    client.prewarm()
    assert wait_for(lambda: len(service.connections) == 2)


def test_a_dead_reused_connection_is_retried_once_on_a_fresh_one():
    client, service, _ = make(["ok", "die"], [])
    assert speak(client) == b"AABB"
    assert speak(client, "Segunda.") == b"AABB"  # the reused socket died on this request
    assert len(service.connections) == 2 and wait_for(lambda: service.connections[0].closed)
    assert len(service.ssml_requests) == 3


def test_a_reused_connection_that_never_answers_is_retried_on_a_fresh_one():
    client, service, _ = make(["ok", "hang"], [])
    speak(client)
    assert speak(client, "Segunda.", first=3.0) == b"AABB"
    assert len(service.connections) == 2


def test_a_failure_on_a_fresh_connection_is_raised_not_retried():
    client, service, _ = make(["die"], ["ok"])
    with pytest.raises(ConnectionError):
        speak(client)
    assert len(service.connections) == 1 and wait_for(lambda: service.connections[0].closed)


def test_a_failure_after_audio_started_is_raised_and_never_retried():
    client, service, _ = make(["ok", "partial_die"], ["ok"])
    speak(client)
    got = []
    with pytest.raises(ConnectionError):
        for chunk in client.stream("Segunda.", VOICE, threading.Event(), 1.0, 1.0):
            got.append(chunk)
    assert got == [b"AA"] and len(service.connections) == 1  # no fresh connection was opened for a retry
    assert wait_for(lambda: service.connections[0].closed)


def test_no_audio_in_time_is_a_timeout_and_the_connection_is_dropped():
    client, service, _ = make(["hang"])
    with pytest.raises(EdgeWsTimeout):
        speak(client, first=0.15)
    assert wait_for(lambda: service.connections[0].closed)


def test_an_abandoned_utterance_drops_the_connection_so_its_frames_cannot_leak_into_the_next():
    client, service, _ = make(["partial_unfinished"], ["ok"])
    stream = client.stream("Primera.", VOICE, threading.Event(), 1.0, 1.0)
    assert next(stream) == b"AA"
    stream.close()  # barge-in: the speaker stops reading mid-turn
    assert wait_for(lambda: service.connections[0].closed)
    assert service.connections[0].late_done.wait(2)  # the first turn's remaining frames reach the dead socket only
    assert speak(client, "Segunda.") == b"AABB"  # nothing of the first turn (b"STALE") is heard
    assert len(service.connections) == 2


def test_a_set_cancel_flag_ends_the_stream_and_drops_the_connection():
    client, service, _ = make(["partial_hang"])
    cancel = threading.Event()
    got = []
    for chunk in client.stream("Hola.", VOICE, cancel, 1.0, 1.0):
        got.append(chunk)
        cancel.set()
    assert got == [b"AA"]
    assert wait_for(lambda: service.connections[0].closed)


def test_a_text_that_needs_splitting_raises_before_touching_the_connection():
    client, service, _ = make()
    with pytest.raises(UnsupportedText):
        speak(client, "palabra " * 600)
    assert service.connections == []


def test_a_connect_failure_reaches_the_consumer():
    async def refuse():
        raise OSError("network is unreachable")

    client = EdgeWsClient(refuse)
    with pytest.raises(OSError):
        speak(client)


def test_utterances_queue_behind_each_other_on_the_one_connection():
    client, service, _ = make()
    results = []

    def run(text):
        results.append(speak(client, text))

    threads = [threading.Thread(target=run, args=(f"Texto {i}.",)) for i in range(3)]
    for t in threads:
        t.start()
    for t in threads:
        t.join(5)
    assert results == [b"AABB"] * 3 and len(service.connections) == 1


# ------------------------------------------------------------------ the engine around it


async def one_off_stream(text, voice):
    yield b"one-off:" + text.encode()


def collect(engine, text="Hola."):
    return b"".join(engine.mp3_chunks(text, VOICE, threading.Event()))


def test_the_engine_prefers_the_persistent_connection():
    client, service, _ = make()
    engine = EdgeTtsEngine(Tunables(), one_off_stream, client=client)
    assert collect(engine) == b"AABB"


def test_the_engine_falls_back_to_the_one_off_path_when_the_persistent_one_fails_before_audio():
    client, service, _ = make(["die"])
    engine = EdgeTtsEngine(Tunables(), one_off_stream, client=client)
    assert collect(engine, "Hola.") == b"one-off:Hola."


def test_the_engine_falls_back_to_the_one_off_path_for_a_long_text():
    client, service, _ = make()
    engine = EdgeTtsEngine(Tunables(), one_off_stream, client=client)
    long_text = "palabra " * 600
    assert collect(engine, long_text) == b"one-off:" + long_text.encode()
    assert service.connections == []


def test_a_persistent_timeout_falls_through_to_the_one_off_path_not_straight_to_piper():
    client, _, _ = make(["hang"])
    engine = EdgeTtsEngine(Tunables(tts_first_chunk_s=0.1), one_off_stream, client=client)
    assert collect(engine) == b"one-off:Hola."


def test_when_the_one_off_path_also_times_out_the_engine_gives_up_so_piper_can_answer():
    async def never(text, voice):
        await asyncio.Event().wait()
        yield b""

    client, _, _ = make(["hang"])
    engine = EdgeTtsEngine(Tunables(tts_first_chunk_s=0.1), never, client=client)
    with pytest.raises(TtsTimeout):
        collect(engine)


def test_the_one_off_attempt_after_a_persistent_timeout_gets_the_full_first_audio_budget(monkeypatch):
    import tts

    seen = []
    real = tts.threaded_chunks

    def recording(make, first, idle, cancel):
        seen.append(first)
        return real(make, first, idle, cancel)

    monkeypatch.setattr(tts, "threaded_chunks", recording)
    client, _, _ = make(["hang"])
    engine = EdgeTtsEngine(Tunables(tts_first_chunk_s=0.2), one_off_stream, client=client)
    assert collect(engine) == b"one-off:Hola."
    assert seen == [0.2]  # the persistent attempt's wait is not deducted


def test_a_half_open_reused_socket_is_retried_invisibly_inside_the_first_chunk_budget():
    """The persistent client's own retry (short reuse wait + reconnect) must finish before the
    consumer's first-chunk timeout, or the engine would abandon a retry that was about to work."""
    client, service, _ = make(["ok", "hang"], [])
    engine = EdgeTtsEngine(Tunables(tts_first_chunk_s=1.0), one_off_stream, client=client)
    assert collect(engine, "Primera.") == b"AABB"
    assert collect(engine, "Segunda.") == b"AABB"  # served by the retry on a fresh socket, not by the one-off path
    assert len(service.connections) == 2


def test_a_reused_socket_that_goes_silent_says_how_long_it_idled_in_the_log(caplog):
    import logging

    client, _, clock = make(["ok", "hang"], [])
    engine = EdgeTtsEngine(Tunables(tts_first_chunk_s=1.0), one_off_stream, client=client)
    collect(engine, "Primera.")
    clock.now += 12
    with caplog.at_level(logging.INFO, logger="niva.voice.edge_ws"):
        collect(engine, "Segunda.")
    assert any("reused socket, 12.0 s idle, 1 turns done" in r.getMessage() for r in caplog.records)


def test_the_default_reuse_wait_is_longer_than_a_slow_healthy_first_frame_but_inside_the_first_chunk_budget():
    reuse = EdgeWsClient(lambda: None)._reuse_first_frame
    assert 1.0 <= reuse < Tunables().tts_first_chunk_s  # measured healthy first frames reach ~0.8 s; 0.8 was too tight


def test_a_failure_after_audio_is_not_replayed_on_the_one_off_path():
    client, _, _ = make(["partial_die"])
    engine = EdgeTtsEngine(Tunables(), one_off_stream, client=client)
    got = []
    with pytest.raises(ConnectionError):
        for piece in engine.mp3_chunks("Hola.", VOICE, threading.Event()):
            got.append(piece)
    assert got == [b"AA"]


def test_without_a_client_the_engine_is_the_one_off_path_only():
    engine = EdgeTtsEngine(Tunables(), one_off_stream)
    assert collect(engine) == b"one-off:Hola."
    engine.prepare()  # nothing to prepare, and no error


def test_prepare_connects_the_persistent_client_early():
    client, service, _ = make()
    EdgeTtsEngine(Tunables(), one_off_stream, client=client).prepare()
    assert wait_for(lambda: len(service.connections) == 1)


def test_the_speaker_prepares_every_engine_that_can_and_ignores_the_rest():
    calls = []

    class Preparable:
        name = "a"

        def prepare(self):
            calls.append("a")

    class Plain:
        name = "b"

    class Broken:
        name = "c"

        def prepare(self):
            raise RuntimeError("nope")

    speaker = Speaker([Preparable(), Plain(), Broken()], lambda rate: None, lambda m: None, lambda i: None, Tunables())
    speaker.prepare()
    assert calls == ["a"]


# ------------------------------------------------------------------ hung connects, prewarm, close


def test_a_prewarmed_connection_that_never_carried_an_utterance_counts_as_reused_for_the_retry():
    client, service, _ = make(["hang"], [])
    client.prewarm()
    assert wait_for(lambda: len(service.connections) == 1)
    assert speak(client, first=3.0) == b"AABB"  # the idle prewarmed socket was half-open: retried once
    assert len(service.connections) == 2


def test_a_hung_connect_gives_up_on_its_own_and_a_turn_waiting_behind_it_does_not_wait_longer_than_its_budget():
    calls = []
    service = FakeService()

    async def connect():
        calls.append(1)
        if len(calls) == 1:
            await asyncio.Event().wait()  # the prewarm's connect never completes
        return await service.connect()

    client = EdgeWsClient(connect, connect_timeout_s=0.4, reuse_first_frame_s=0.2, frame_timeout_s=0.5)
    client.prewarm()
    assert wait_for(lambda: len(calls) == 1)
    started = time.monotonic()
    with pytest.raises(EdgeWsTimeout):
        speak(client, first=0.15)  # queued behind the stalled prewarm: its own budget, not the connect's
    assert time.monotonic() - started < 0.35
    # The prewarm's connect times out by itself, which frees the turn lock for the next utterance.
    assert speak(client, "Otra.", first=3.0) == b"AABB"
    assert len(calls) == 2


def test_a_connect_that_hangs_in_a_turn_is_a_timeout_the_engine_can_fall_back_from():
    async def hang():
        await asyncio.Event().wait()

    client = EdgeWsClient(hang, connect_timeout_s=0.2)
    with pytest.raises(EdgeWsTimeout):
        speak(client, first=3.0)
    engine = EdgeTtsEngine(Tunables(tts_first_chunk_s=1.0), one_off_stream, client=client)
    assert collect(engine) == b"one-off:Hola."


def test_close_stops_the_loop_thread_and_closes_the_loop():
    client, service, _ = make()
    speak(client)
    loop, thread = client._loop, client._loop_thread
    client.close()
    assert not thread.is_alive() and loop.is_closed()
    assert wait_for(lambda: service.connections[0].closed)
    client.close()  # idempotent


def test_a_prewarm_after_close_is_a_no_op():
    client, service, _ = make()
    speak(client)
    client.close()
    client.prewarm()
    assert len(service.connections) == 1 and client._loop is None
    with pytest.raises(RuntimeError):
        speak(client)  # the engine treats this like any persistent failure and uses the one-off path


def test_closing_a_client_that_never_connected_is_fine():
    client, service, _ = make()
    client.close()
    client.prewarm()
    assert service.connections == []


# ------------------------------------------------------------------ edge-tts internals


def test_the_installed_edge_tts_still_has_the_internals_the_persistent_client_uses():
    """A guard for upgrades: if this fails, the persistent path is falling back to the one-off one."""
    pytest.importorskip("edge_tts")
    from edge_tts import communicate, constants, data_classes, drm

    for name in (
        "connect_id",
        "date_to_string",
        "escape",
        "mkssml",
        "remove_incompatible_characters",
        "ssml_headers_plus_data",
    ):
        assert callable(getattr(communicate, name)), name
    for name in ("SEC_MS_GEC_VERSION", "WSS_HEADERS", "WSS_URL"):
        assert getattr(constants, name), name
    for name in ("generate_sec_ms_gec", "headers_with_muid", "handle_client_response_error"):
        assert callable(getattr(drm.DRM, name)), name
    data_classes.TTSConfig(VOICE, "+0%", "+0%", "+0Hz", "SentenceBoundary")  # the 5-field shape
    assert "Path:speech.config" in config_message() and "Path:ssml" in ssml_message("Hola.", VOICE)
    import aiohttp  # noqa: F401 - connect_edge needs these two
    import certifi  # noqa: F401
