"""The sidecar around the engine: threads, the single microphone stream, failures."""

import io
import json
import queue
import threading
import time

import pytest
from conftest import SILENCE, SPEECH, Harness, frames

from audio_io import MicError
from config import FRAME_SAMPLES, Tunables
from main import EXIT_LOAD, EXIT_OK, Emitter, Sidecar, read_commands
from protocol import ErrorCode, Listen, Shutdown
from stt import SttError


class FakeMic:
    """A microphone the test feeds by hand. `read` times out like the real one."""

    def __init__(self, stall_s=2.0, open_failures=()):
        self.feed_queue: queue.Queue = queue.Queue()
        self.stall_s = stall_s
        self.open_failures = list(open_failures)
        self.opens = 0
        self.closes = 0

    def open(self):
        self.opens += 1
        if self.open_failures:
            raise self.open_failures.pop(0)

    def read(self):
        try:
            item = self.feed_queue.get(timeout=self.stall_s)
        except queue.Empty:
            raise MicError(ErrorCode.MIC_MISSING, "stalled") from None
        if isinstance(item, Exception):
            raise item
        return item

    def close(self):
        self.closes += 1

    def feed(self, items):
        for item in items:
            self.feed_queue.put(item)


class Running:
    """A Sidecar running on its own thread over a FakeMic."""

    def __init__(self, mic, tunables=None, **stt_args):
        self.h = Harness(tunables or Tunables(mic_retry_s=0.02), **stt_args)
        self.mic = mic
        self.sidecar = Sidecar(self.h.engine, mic, self.h.events.append, self.h.tunables)
        self.code = None
        self.thread = threading.Thread(target=self._run, daemon=True)
        self.thread.start()

    def _run(self):
        self.code = self.sidecar.run()

    def wait_for(self, predicate, timeout=3.0):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return True
            time.sleep(0.005)
        return False

    def settle(self):
        """Everything fed so far has been processed by the main loop."""
        assert self.wait_for(lambda: self.mic.feed_queue.empty() and self.sidecar.inbox.empty())
        time.sleep(0.03)

    def command(self, command):
        self.sidecar.inbox.put(("cmd", command))

    def finish(self):
        self.command(Shutdown())
        self.thread.join(3)
        assert not self.thread.is_alive()
        return self.code


def test_ready_is_sent_once_the_stream_is_open():
    mic = FakeMic()
    run = Running(mic)
    mic.feed(frames(SILENCE, 1))
    assert run.wait_for(lambda: run.h.types() == ["ready"])
    assert run.finish() == EXIT_OK


def test_one_stream_serves_wake_turn_and_second_turn_without_reopening():
    mic = FakeMic()
    run = Running(mic, text="abre spotify", lang="es")
    mic.feed(frames(SILENCE, 1))
    assert run.wait_for(lambda: run.h.types() == ["ready"])
    for turn_id in (1, 2):
        mic.feed(frames(SILENCE, 10))
        run.settle()
        run.command(Listen(turn_id, "command"))
        mic.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
        assert run.wait_for(
            lambda n=turn_id: [e["id"] for e in run.h.events if e["type"] == "transcript"] == list(range(1, n + 1))
        )
        run.settle()
    assert mic.opens == 1 and mic.closes == 0, "the microphone was reopened between turns"
    run.finish()
    assert mic.closes >= 1  # released on shutdown


def test_a_stalled_stream_reports_mic_error_and_never_stays_silently_deaf():
    mic = FakeMic(stall_s=0.05)
    run = Running(mic)
    assert run.wait_for(lambda: any(e["type"] == "error" for e in run.h.events))
    error = next(e for e in run.h.events if e["type"] == "error")
    assert error["code"] in ("MIC_MISSING", "MIC_BUSY")
    run.finish()


def test_a_stream_failure_during_a_listen_names_the_listen_and_recovers():
    mic = FakeMic()
    run = Running(mic)
    mic.feed(frames(SILENCE, 1))
    assert run.wait_for(lambda: run.h.types() == ["ready"])
    run.command(Listen(30, "command"))
    mic.feed(frames(SPEECH, 3))
    run.settle()
    mic.feed([MicError(ErrorCode.MIC_MISSING, "device unplugged")])
    assert run.wait_for(lambda: any(e["type"] == "error" for e in run.h.events))
    error = next(e for e in run.h.events if e["type"] == "error")
    assert (error["code"], error["id"]) == ("MIC_MISSING", 30)
    mic.feed(frames(SILENCE, 1))  # the reopened stream delivers audio
    # the stream is reopened by itself and the host hears `ready` again
    assert run.wait_for(lambda: run.h.types().count("ready") == 2)
    assert mic.opens == 2 and mic.closes == 1
    run.finish()


def test_open_failure_is_reported_once_then_retried_until_it_works():
    busy = MicError(ErrorCode.MIC_BUSY, "in use by another app")
    mic = FakeMic(open_failures=[busy, busy, busy])
    mic.feed(frames(SILENCE, 1))
    run = Running(mic)
    assert run.wait_for(lambda: "ready" in run.h.types())
    assert run.h.types() == ["error", "ready"]  # three failed tries, one report
    assert run.h.events[0]["code"] == "MIC_BUSY"
    assert mic.opens == 4
    run.finish()


def test_unreadable_host_line_is_a_protocol_error_and_the_sidecar_keeps_serving():
    mic = FakeMic()
    run = Running(mic)
    mic.feed(frames(SILENCE, 1))
    assert run.wait_for(lambda: run.h.types() == ["ready"])
    run.sidecar.inbox.put(("proto", "unknown message type: 'dance'"))
    assert run.wait_for(lambda: "error" in run.h.types())
    assert run.h.events[-1]["code"] == "PROTOCOL"
    run.command(Listen(1, "command"))
    mic.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert run.wait_for(lambda: "transcript" in run.h.types())
    run.finish()


def test_stt_out_of_memory_stops_the_sidecar_with_the_listen_id_and_a_load_exit_code():
    mic = FakeMic()
    run = Running(mic)
    run.h.stt.fail = SttError(ErrorCode.CUDA_OOM, "out of memory")
    mic.feed(frames(SILENCE, 1))
    assert run.wait_for(lambda: run.h.types() == ["ready"])
    run.command(Listen(9, "command"))
    mic.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    run.thread.join(3)
    assert run.code == EXIT_LOAD
    error = run.h.events[-1]
    assert (error["type"], error["code"], error["id"]) == ("error", "CUDA_OOM", 9)


def test_end_of_input_stops_the_sidecar_cleanly():
    mic = FakeMic()
    run = Running(mic)
    mic.feed(frames(SILENCE, 1))
    assert run.wait_for(lambda: run.h.types() == ["ready"])
    run.sidecar.inbox.put(("eof",))
    run.thread.join(3)
    assert run.code == EXIT_OK


# ------------------------------------------------------------ stdin and stdout


def test_read_commands_parses_lines_and_flags_bad_ones():
    lines = io.BytesIO(
        b'{"type":"listen","id":4,"mode":"confirm"}\n\n{"oops"\n'
        b'{"type":"speak","id":5,"text":"Hola \xc2\xbf","lang":"es"}\n'
    )
    inbox: queue.Queue = queue.Queue()
    gone = threading.Event()
    read_commands(lines, inbox, gone)
    items = [inbox.get_nowait() for _ in range(inbox.qsize())]
    assert [i[0] for i in items] == ["cmd", "proto", "cmd", "eof"]
    assert items[0][1] == Listen(4, "confirm")
    assert items[2][1].text == "Hola ¿"
    assert gone.is_set()


def test_read_commands_stops_at_shutdown():
    lines = io.BytesIO(b'{"type":"shutdown"}\n{"type":"listen","id":1,"mode":"command"}\n')
    inbox: queue.Queue = queue.Queue()
    gone = threading.Event()
    read_commands(lines, inbox, gone)
    assert [i[0] for i in list(inbox.queue)] == ["cmd"]
    assert isinstance(inbox.get_nowait()[1], Shutdown)
    assert gone.is_set()


def test_emitter_writes_one_json_object_per_line_and_nothing_else():
    sink = io.BytesIO()
    emit = Emitter(sink)
    emit({"type": "wake", "score": 0.7})
    emit({"type": "transcript", "id": 1, "text": "ñ", "lang": "es", "lang_prob": 1.0, "end_ms": 0})
    lines = sink.getvalue().decode("utf-8").splitlines()
    assert [json.loads(line)["type"] for line in lines] == ["wake", "transcript"]
    assert sink.getvalue().endswith(b"\n")


def test_emitter_survives_a_closed_host_pipe():
    class Closed:
        def write(self, data):
            raise BrokenPipeError

        def flush(self):
            raise BrokenPipeError

    Emitter(Closed())({"type": "wake", "score": 0.5})  # must not raise


def test_frames_have_the_size_the_models_expect():
    assert frames(SILENCE, 1)[0].shape == (FRAME_SAMPLES,)


@pytest.mark.parametrize("flag", ["--no-edge", "--allow-cpu"])
def test_cli_flags_parse(flag):
    from main import parse_args, tunables_from

    args = parse_args([flag, "--pre-speech-ms", "450", "--wake-threshold", "0.6"])
    tunables = tunables_from(args)
    assert tunables.pre_speech_ms == 450 and tunables.wake_threshold == 0.6
    assert tunables.end_silence_ms == 500  # the owner's VAD decision stays the default


def test_the_robot_voice_is_on_by_default_and_the_flag_turns_it_off():
    from main import parse_args, tunables_from

    assert tunables_from(parse_args([])).robot_voice is True
    assert tunables_from(parse_args(["--no-robot-voice"])).robot_voice is False


# ------------------------------------------------- review regressions (W1-W4, S3, S4)


def test_an_unexpected_exception_in_the_capture_thread_is_reported_not_swallowed():
    """W1: the thread used to die silently and the sidecar stayed deaf."""
    mic = FakeMic()
    run = Running(mic)
    mic.feed(frames(SILENCE, 1))
    assert run.wait_for(lambda: run.h.types() == ["ready"])
    mic.feed([RuntimeError("PortAudio exploded")])
    assert run.wait_for(lambda: "error" in run.h.types())
    assert run.h.events[-1]["code"] in ("MIC_MISSING", "MIC_BUSY")
    assert run.wait_for(lambda: mic.opens == 2)  # and it tries again
    run.finish()


def test_a_non_micerror_failure_when_opening_is_reported_too():
    class Boom(FakeMic):
        def open(self):
            self.opens += 1
            if self.opens == 1:
                raise ImportError("no sounddevice")

    mic = Boom()
    run = Running(mic)
    assert run.wait_for(lambda: "error" in run.h.types())
    mic.feed(frames(SILENCE, 1))
    assert run.wait_for(lambda: "ready" in run.h.types())
    run.finish()


def test_a_pathological_line_does_not_kill_the_reader_and_eof_is_still_seen():
    """W2: RecursionError used to escape the reader thread; EOF was never queued."""
    inbox: queue.Queue = queue.Queue()
    gone = threading.Event()
    read_commands(io.BytesIO(b"[" * 200_000 + b"\n"), inbox, gone)
    assert [i[0] for i in list(inbox.queue)] == ["proto", "eof"]
    assert gone.is_set()


def test_a_failing_input_stream_still_ends_with_eof():
    def broken():
        yield b'{"type":"stop_speaking"}\n'
        raise OSError("pipe broke")

    inbox: queue.Queue = queue.Queue()
    gone = threading.Event()
    with pytest.raises(OSError):
        read_commands(broken(), inbox, gone)
    assert [i[0] for i in list(inbox.queue)] == ["cmd", "eof"]
    assert gone.is_set()


def test_a_mic_that_opens_but_gives_no_audio_does_not_flap_ready_and_error():
    """W3: used to send ready/error about every 7 s forever."""
    mic = FakeMic(stall_s=0.03)
    run = Running(mic)
    time.sleep(0.5)
    assert run.h.types() == ["error"]  # one report, never a ready without audio
    assert mic.opens >= 3
    run.finish()


def test_frames_are_stamped_in_the_capture_thread():
    """W4: the engine must see when the audio arrived, not when it got to it."""
    seen = []
    mic = FakeMic()
    h = Harness()
    h.engine.on_frame = lambda frame, now_ms=None: seen.append(now_ms)
    ticks = iter(range(1000, 100000, 7))
    sidecar = Sidecar(h.engine, mic, h.events.append, h.tunables, clock_ms=lambda: next(ticks))
    thread = threading.Thread(target=sidecar.run, daemon=True)
    thread.start()
    mic.feed(frames(SILENCE, 3))
    deadline = time.monotonic() + 2
    while len(seen) < 3 and time.monotonic() < deadline:
        time.sleep(0.005)
    sidecar.inbox.put(("cmd", Shutdown()))
    thread.join(3)
    assert len(seen) >= 3 and all(isinstance(t, int) for t in seen)
    assert seen == sorted(seen) and len(set(seen)) == len(seen)


def test_the_dll_folders_are_added_before_any_model_is_loaded(monkeypatch):
    """S3: ctranslate2 must not be imported before the nvidia folders are on the DLL path."""
    import main
    import stt
    import wake

    order = []
    monkeypatch.setattr(stt, "add_nvidia_dll_dirs", lambda: order.append("dll") or [])

    class Boom:
        def __init__(self, *args, **kwargs):
            order.append("wake")
            raise RuntimeError("stop here")

    monkeypatch.setattr(wake, "OpenWakeWordDetector", Boom)
    monkeypatch.setattr(main, "claim_stdout", lambda: io.BytesIO())
    monkeypatch.setattr(main, "setup_logging", lambda args: None)
    assert main.main([]) == EXIT_LOAD
    assert order == ["dll", "wake"]


def test_the_emitter_finishes_a_line_the_pipe_accepted_only_in_part():
    """S4: a raw pipe may take fewer bytes than offered."""

    class Trickle:
        def __init__(self):
            self.data = b""

        def write(self, view):
            chunk = bytes(view[:5])
            self.data += chunk
            return len(chunk)

        def flush(self):
            pass

    pipe = Trickle()
    Emitter(pipe)(
        {"type": "transcript", "id": 1, "text": "a long enough sentence", "lang": "en", "lang_prob": 1.0, "end_ms": 0}
    )
    assert json.loads(pipe.data.decode("utf-8"))["text"] == "a long enough sentence"


@pytest.mark.parametrize(
    "failures",
    [
        [MicError(ErrorCode.MIC_BUSY, "in use"), MicError(ErrorCode.MIC_MISSING, "unplugged")],
        [MicError(ErrorCode.MIC_MISSING, "unplugged"), MicError(ErrorCode.MIC_BUSY, "in use")],
    ],
)
def test_a_different_mic_failure_is_reported_even_after_one_was(failures):
    mic = FakeMic(open_failures=list(failures))
    mic.feed(frames(SILENCE, 1))
    run = Running(mic)
    assert run.wait_for(lambda: "ready" in run.h.types())
    codes = [e["code"] for e in run.h.events if e["type"] == "error"]
    assert codes == [f.code.value for f in failures]
    run.finish()


@pytest.mark.parametrize(
    ("flag", "field"),
    [
        ("--no-preopen-output", "preopen_output"),
        ("--no-single-pass-stt", "single_pass_stt"),
    ],
)
def test_each_latency_optimization_is_on_by_default_and_has_its_own_switch(flag, field):
    from main import parse_args, tunables_from

    assert getattr(tunables_from(parse_args([])), field) is True
    off = tunables_from(parse_args([flag]))
    assert getattr(off, field) is False
    switches = ("preopen_output", "single_pass_stt")
    assert all(getattr(off, other) is True for other in switches if other != field), "one switch, one optimization"
