import asyncio
import threading
import time
from pathlib import Path

import numpy as np
import pytest

from audio_io import OutputError
from config import Tunables
from tts import (
    EDGE_VOICES,
    EdgeTtsEngine,
    OutputPrefetch,
    PcmChunk,
    Speaker,
    TtsTimeout,
    _Cancelled,
    _Run,
    clean_for_speech,
    decode_mp3_stream,
    is_silent,
    threaded_chunks,
)
from voice_fx import RobotVoice, float_to_pcm16, pcm16_to_float

SAMPLE_MP3 = Path(__file__).resolve().parents[2] / "spikes" / "voice" / "s1_audio" / "en00.mp3"

# ------------------------------------------------------------------ clean text


@pytest.mark.parametrize(
    ("raw", "spoken"),
    [
        ("Opening Notepad", "Opening Notepad"),
        ("**Done**: volume at *30%*.", "Done: volume at 30%."),
        ("See [the docs](https://example.com/a?b=1) now", "See the docs now"),
        ("Visit https://example.com/path and www.test.org today", "Visit and today"),
        ("Run `rm -rf /tmp` carefully", "Run carefully"),
        ("Before\n```python\nprint('hi')\n```\nAfter", "Before. After"),
        ("Unclosed ```code never ends", "Unclosed"),
        ("# Title\n- one\n- two\n3. three", "Title. one. two. three"),
        ("Great job \U0001f389\U0001f680 ✅ done", "Great job done"),
        ("Flags \U0001f1e6\U0001f1f7 and keycap 1️⃣ gone", "Flags and keycap 1 gone"),
        ("snake_case_word stays, _this_ goes plain", "snake_case_word stays, this goes plain"),
        ("| a | b |", ", a , b ,"),
        ("<b>bold</b> text", "bold text"),
        ("![chart](http://x/y.png) shown", "chart shown"),
        ("   \n  ", ""),
        ("```only code```", ""),
    ],
)
def test_clean_for_speech(raw, spoken):
    assert clean_for_speech(raw) == spoken


def test_clean_keeps_accents_and_spanish_punctuation():
    assert clean_for_speech("¿Querés que abra Spotify? ¡Listo!") == "¿Querés que abra Spotify? ¡Listo!"


# ------------------------------------------------------------ network handoff


def fake_stream(chunks, delay_first=0.0):
    async def stream(text, voice):
        await asyncio.sleep(delay_first)
        for chunk in chunks:
            yield chunk

    return stream


def test_threaded_chunks_hands_over_in_order():
    stream = fake_stream([b"a", b"b", b"c"])
    got = list(threaded_chunks(lambda: stream("t", "v"), 1.0, 1.0, threading.Event()))
    assert got == [b"a", b"b", b"c"]


def test_first_chunk_timeout_raises_so_the_caller_can_fall_back():
    stream = fake_stream([b"a"], delay_first=0.5)
    with pytest.raises(TtsTimeout):
        list(threaded_chunks(lambda: stream("t", "v"), 0.05, 1.0, threading.Event()))


def test_network_errors_reach_the_consumer():
    async def broken(text, voice):
        yield b"a"
        raise ConnectionError("endpoint refused")

    with pytest.raises(ConnectionError):
        list(threaded_chunks(lambda: broken("t", "v"), 1.0, 1.0, threading.Event()))


@pytest.mark.skipif(not SAMPLE_MP3.exists(), reason="local TTS sample (spikes/voice/s1_audio) not present")
def test_mp3_is_decoded_to_pcm_as_it_arrives():
    data = SAMPLE_MP3.read_bytes()
    pieces = [data[i : i + 700] for i in range(0, len(data), 700)]  # network-sized pieces
    blocks = list(decode_mp3_stream(iter(pieces)))
    assert blocks and all(b.sample_rate == 24_000 for b in blocks)
    pcm = np.frombuffer(b"".join(b.data for b in blocks), dtype=np.int16)
    assert pcm.size > 24_000  # more than a second of speech
    assert np.abs(pcm).max() > 1000
    assert not is_silent(blocks[len(blocks) // 2].data, 24)


@pytest.mark.skipif(not SAMPLE_MP3.exists(), reason="local TTS sample (spikes/voice/s1_audio) not present")
def test_first_pcm_block_arrives_before_the_whole_stream_is_read():
    data = SAMPLE_MP3.read_bytes()
    consumed = []

    def source():
        for i in range(0, len(data), 700):
            consumed.append(i)
            yield data[i : i + 700]

    first = next(decode_mp3_stream(source()))
    assert first.data
    assert len(consumed) * 700 < len(data) / 2  # started playing after a fraction of the bytes


def test_one_voice_per_language():
    asked = []

    async def stream(text, voice):
        asked.append(voice)
        raise ConnectionError("stop after recording the voice")
        yield b""  # pragma: no cover

    engine = EdgeTtsEngine(Tunables(), stream)
    for lang in ("es", "en"):
        with pytest.raises(ConnectionError):
            list(engine.chunks("hola", lang, threading.Event()))
    assert asked == [EDGE_VOICES["es"], EDGE_VOICES["en"]]
    assert EDGE_VOICES["es"] != EDGE_VOICES["en"]
    assert EDGE_VOICES == {"es": "es-CU-BelkysNeural", "en": "en-US-AriaNeural"}


# -------------------------------------------------------------------- speaker


AUDIBLE = PcmChunk((np.full(480, 4000, dtype=np.int16)).tobytes(), 24_000)
QUIET = PcmChunk(bytes(960), 24_000)


class Timeline:
    """Output writes and protocol events in one ordered log."""

    def __init__(self):
        self.log = []

    def emit(self, message):
        self.log.append(("event", message["type"], message.get("code")))

    def count(self, kind):
        return sum(1 for entry in self.log if entry[:2] == ("event", kind))


class FakeOutput:
    def __init__(self, timeline, rate, fail_on_write=False):
        self.timeline, self.sample_rate, self.fail = timeline, rate, fail_on_write
        self.aborted = False
        self.closed = False
        self.written = []

    def write(self, pcm):
        if self.fail:
            raise OutputError("device went away")
        self.written.append(pcm)
        self.timeline.log.append(("write", is_silent(pcm, 24)))

    def abort(self):
        self.aborted = True

    def close(self):
        self.closed = True
        self.timeline.log.append(("close",))


class FakeEngine:
    def __init__(self, name, chunks=(), error=None, error_after=None, gate=None):
        self.name = name
        self._chunks, self._error, self._error_after, self._gate = chunks, error, error_after, gate
        self.calls = []

    def chunks(self, text, lang, cancel):
        self.calls.append((text, lang))
        if self._error is not None:
            raise self._error
        for chunk in self._chunks:
            yield chunk
            if self._gate is not None:
                self._gate.wait(2)
        if self._error_after is not None:
            raise self._error_after


PLAIN = Tunables(robot_voice=False)  # the plumbing tests care about order and ids, not about the effect
ROBOT = Tunables(robot_voice=True)  # the effect tests turn it on: it is off by default


def build(engines, fail_on_write=False, tunables=PLAIN):
    timeline = Timeline()
    ended = []
    outputs = []

    def make_output(rate):
        out = FakeOutput(timeline, rate, fail_on_write)
        outputs.append(out)
        return out

    speaker = Speaker(engines, make_output, timeline.emit, ended.append, tunables)
    return speaker, timeline, ended, outputs


def run_speak(speaker, speak_id=7, text="Hello there.", lang="en"):
    speaker.speak(speak_id, text, lang)
    speaker.join(3)


def test_speak_started_comes_only_after_the_first_audible_chunk_was_written():
    speaker, tl, ended, outputs = build([FakeEngine("edge", [QUIET, QUIET, AUDIBLE, AUDIBLE])])
    run_speak(speaker)
    kinds = [entry[0:2] for entry in tl.log]
    first_audible_write = kinds.index(("write", False))
    started = kinds.index(("event", "speak_started"))
    assert started == first_audible_write + 1  # right after the audible write, not before
    assert kinds[:first_audible_write] == [("write", True)] * 2  # silence was written, not announced
    assert tl.count("speak_done") == 1 and tl.count("error") == 0
    assert ended == [7] and outputs[0].closed


def test_nothing_audible_reaching_the_device_is_tts_failed_for_that_id():
    speaker, tl, ended, _ = build([FakeEngine("edge", [QUIET, QUIET])])
    run_speak(speaker, speak_id=11)
    assert tl.count("speak_started") == 0 and tl.count("speak_done") == 0
    assert ("event", "error", "TTS_FAILED") in tl.log
    assert ended == [11]


def test_device_write_failure_is_tts_failed_and_never_speak_started():
    speaker, tl, ended, _ = build([FakeEngine("edge", [AUDIBLE, AUDIBLE])], fail_on_write=True)
    run_speak(speaker, speak_id=12)
    assert tl.count("speak_started") == 0
    assert tl.count("error") == 1 and ("event", "error", "TTS_FAILED") in tl.log
    assert ended == [12]


def test_device_failure_does_not_try_the_next_engine():
    first = FakeEngine("edge", [AUDIBLE])
    second = FakeEngine("piper", [AUDIBLE])
    speaker, tl, _, _ = build([first, second], fail_on_write=True)
    run_speak(speaker)
    assert second.calls == []


def test_falls_back_to_piper_when_edge_fails_before_any_audio():
    edge = FakeEngine("edge", error=ConnectionError("offline"))
    piper = FakeEngine("piper", [AUDIBLE, AUDIBLE])
    speaker, tl, _, _ = build([edge, piper])
    run_speak(speaker, text="Hola.", lang="es")
    assert piper.calls == [("Hola.", "es")]
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 1 and tl.count("error") == 0


def test_piper_preload_loads_each_voice_once_and_a_later_fallback_does_not_reload(monkeypatch, tmp_path):
    import sys
    import types

    loads = []

    class FakeVoice:
        def synthesize(self, text):
            return [types.SimpleNamespace(audio_int16_bytes=b"\x01\x00", sample_rate=22050)]

    class FakePiperVoice:
        @staticmethod
        def load(path):
            loads.append(path)
            return FakeVoice()

    monkeypatch.setitem(sys.modules, "piper", types.SimpleNamespace(PiperVoice=FakePiperVoice))
    from tts import PIPER_VOICE_FILES, PiperEngine

    engine = PiperEngine(tmp_path)
    engine.preload()
    assert len(loads) == len(PIPER_VOICE_FILES)
    assert [c.data for c in engine.chunks("Hola.", "es", threading.Event())] == [b"\x01\x00"]
    engine.preload()
    assert len(loads) == len(PIPER_VOICE_FILES), "a fallback after the preload must not load again"


def test_falls_back_to_piper_on_a_first_chunk_timeout():
    edge = FakeEngine("edge", error=TtsTimeout("slow"))
    piper = FakeEngine("piper", [AUDIBLE])
    speaker, tl, _, _ = build([edge, piper])
    run_speak(speaker)
    assert piper.calls and tl.count("speak_done") == 1


def test_a_silent_engine_also_falls_back():
    speaker, tl, _, _ = build([FakeEngine("edge", [QUIET]), FakeEngine("piper", [AUDIBLE])])
    run_speak(speaker)
    assert tl.count("speak_started") == 1


def test_an_engine_dying_after_sound_started_ends_the_speech_without_repeating_it():
    edge = FakeEngine("edge", [AUDIBLE], error_after=ConnectionError("dropped"))
    piper = FakeEngine("piper", [AUDIBLE])
    speaker, tl, _, _ = build([edge, piper])
    run_speak(speaker)
    assert piper.calls == []
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 1 and tl.count("error") == 0


def test_all_engines_failing_is_tts_failed_naming_each():
    edge = FakeEngine("edge", error=ConnectionError("offline"))
    piper = FakeEngine("piper", error=RuntimeError("voice missing"))
    messages = []
    speaker, tl, _, _ = build([edge, piper])
    speaker._emit = lambda m: (messages.append(m), tl.emit(m))
    run_speak(speaker, speak_id=5)
    error = next(m for m in messages if m["type"] == "error")
    assert error["code"] == "TTS_FAILED" and error["id"] == 5
    assert "edge" in error["msg"] and "piper" in error["msg"]


def test_text_that_cleans_to_nothing_is_tts_failed():
    engine = FakeEngine("edge", [AUDIBLE])
    speaker, tl, _, _ = build([engine])
    run_speak(speaker, text="```code only```")
    assert engine.calls == []
    assert ("event", "error", "TTS_FAILED") in tl.log


def test_the_engine_gets_the_cleaned_text():
    engine = FakeEngine("edge", [AUDIBLE])
    speaker, _, _, _ = build([engine])
    run_speak(speaker, text="**Done** \U0001f389 see https://x.io")
    assert engine.calls == [("Done see", "en")]


def test_audio_starts_at_the_first_chunk_not_at_the_end_of_synthesis():
    gate = threading.Event()
    engine = FakeEngine("edge", [AUDIBLE, AUDIBLE], gate=gate)  # blocks after the first chunk
    speaker, tl, _, _ = build([engine])
    speaker.speak(3, "Hello.", "en")
    deadline = time.monotonic() + 2
    while tl.count("speak_started") == 0 and time.monotonic() < deadline:
        time.sleep(0.005)
    assert tl.count("speak_started") == 1
    assert tl.count("speak_done") == 0  # synthesis is still going
    gate.set()
    speaker.join(3)
    assert tl.count("speak_done") == 1


def test_stop_cuts_the_speech_without_done_or_error_and_aborts_the_device():
    gate = threading.Event()
    engine = FakeEngine("edge", [AUDIBLE, AUDIBLE, AUDIBLE], gate=gate)
    speaker, tl, ended, outputs = build([engine])
    speaker.speak(4, "Hello.", "en")
    deadline = time.monotonic() + 2
    while tl.count("speak_started") == 0 and time.monotonic() < deadline:
        time.sleep(0.005)
    speaker.stop()
    gate.set()
    speaker.join(3)
    assert tl.count("speak_done") == 0 and tl.count("error") == 0
    assert outputs[0].aborted and outputs[0].closed
    assert ended == [4]


def test_a_new_speak_replaces_the_old_one():
    gate = threading.Event()
    slow = FakeEngine("edge", [AUDIBLE, AUDIBLE], gate=gate)
    speaker, tl, ended, _ = build([slow])
    speaker.speak(1, "First.", "en")
    time.sleep(0.05)
    speaker._engines = [FakeEngine("edge", [AUDIBLE])]
    speaker.speak(2, "Second.", "en")
    gate.set()
    speaker.join(3)
    time.sleep(0.05)
    assert 1 in ended and 2 in ended
    assert tl.count("speak_done") == 1  # only the second one finished


# ------------------------------------------------- review regressions (C1, S2)


def test_speak_done_is_sent_only_after_the_output_drained_and_closed():
    """C1: closing the device waits for the buffer to play out; `speak_done` must follow it."""
    speaker, tl, ended, _ = build([FakeEngine("edge", [AUDIBLE, AUDIBLE])])
    run_speak(speaker)
    kinds = [entry[0:2] for entry in tl.log]
    assert kinds.index(("close",)) < kinds.index(("event", "speak_done"))
    assert ended == [7]


def test_stop_while_the_output_is_opening_writes_and_announces_nothing():
    """S2: a stop that lands during `make_output` must not still play or speak."""
    gate = threading.Event()
    tl = Timeline()
    outputs, ended = [], []

    def make_output(rate):
        gate.wait(2)
        out = FakeOutput(tl, rate)
        outputs.append(out)
        return out

    opening = threading.Event()
    real_make_output = make_output

    def make_output(rate):  # noqa: F811 - signals that the open is under way
        opening.set()
        return real_make_output(rate)

    speaker = Speaker([FakeEngine("edge", [AUDIBLE, AUDIBLE])], make_output, tl.emit, ended.append, PLAIN)
    speaker.speak(1, "Hello.", "en")
    assert opening.wait(2)
    speaker.stop()
    gate.set()
    speaker.join(3)
    assert [e for e in tl.log if e[0] == "write"] == []
    assert tl.count("speak_started") == 0 and tl.count("speak_done") == 0 and tl.count("error") == 0
    assert outputs[0].closed
    assert ended == [1]


# ---------------------------------------------------------------- robot voice


def voiced(seconds=0.5, rate=24_000, chunk=480):
    """A vowel-like buzz, cut into engine-sized chunks."""
    t = np.arange(int(seconds * rate)) / rate
    wave = 0.4 * np.sin(2 * np.pi * 180 * t) + 0.2 * np.sin(2 * np.pi * 540 * t)
    pcm = (wave * 32767).astype(np.int16)
    return [PcmChunk(pcm[i : i + chunk].tobytes(), rate) for i in range(0, len(pcm), chunk)]


def written_samples(outputs):
    return np.frombuffer(b"".join(b"".join(out.written) for out in outputs), dtype=np.int16)


def robot_reference(chunks):
    """What the effect alone makes of these chunks: the exact samples expected on the device."""
    fx = RobotVoice(chunks[0].sample_rate)
    parts = [float_to_pcm16(fx.process(pcm16_to_float(c.data))) for c in chunks]
    parts.append(float_to_pcm16(fx.flush()))
    return np.frombuffer(b"".join(parts), dtype=np.int16)


def test_robot_voice_changes_what_reaches_the_device_and_still_announces_the_speech():
    chunks = voiced()
    speaker, tl, ended, outputs = build([FakeEngine("edge", chunks)], tunables=ROBOT)
    run_speak(speaker)
    dry = np.frombuffer(b"".join(c.data for c in chunks), dtype=np.int16)
    played = written_samples(outputs)
    assert abs(len(played) - len(dry) / 1.2) <= 2  # pitched up: 20 % shorter, tail flushed
    assert np.array_equal(played, robot_reference(chunks))  # exactly the effect's output...
    n = min(len(played), len(dry))
    assert np.abs(played[:n].astype(int) - dry[:n].astype(int)).mean() > 1000  # ...which is not the dry voice
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 1 and tl.count("error") == 0
    assert ended == [7]


def test_robot_voice_off_plays_the_engine_audio_unchanged():
    chunks = voiced()
    speaker, tl, _, outputs = build([FakeEngine("edge", chunks)], tunables=Tunables(robot_voice=False))
    run_speak(speaker)
    assert b"".join(b"".join(out.written) for out in outputs) == b"".join(c.data for c in chunks)


def test_robot_voice_is_not_announced_before_audible_samples_reach_the_device():
    speaker, tl, _, _ = build([FakeEngine("edge", voiced(0.3))], tunables=ROBOT)
    run_speak(speaker)
    kinds = [entry[0:2] for entry in tl.log]
    first_audible_write = kinds.index(("write", False))
    assert kinds.index(("event", "speak_started")) == first_audible_write + 1


def test_robot_voice_effect_state_does_not_leak_between_utterances():
    chunks = voiced(0.3)
    speaker, _, _, outputs = build([FakeEngine("edge", chunks)], tunables=ROBOT)
    run_speak(speaker, speak_id=1)
    run_speak(speaker, speak_id=2)  # the same Speaker, a second utterance
    fresh, _, _, fresh_outputs = build([FakeEngine("edge", chunks)], tunables=ROBOT)
    run_speak(fresh)
    assert len(outputs) == 2
    assert b"".join(outputs[1].written) == b"".join(fresh_outputs[0].written)
    assert b"".join(outputs[0].written) == b"".join(outputs[1].written)


def test_robot_voice_does_not_carry_a_failed_engines_state_into_the_fallback():
    """Edge feeds the effect some (still inaudible) audio and then dies; Piper must sound
    exactly as if it had run alone, not continue the dead engine's signal."""
    partial = voiced(0.02)  # one chunk: inside the effect's start-up latency, nothing audible yet
    piper_chunks = voiced(0.3)
    edge = FakeEngine("edge", partial, error_after=ConnectionError("dropped"))
    piper = FakeEngine("piper", piper_chunks)
    speaker, tl, _, outputs = build([edge, piper], tunables=ROBOT)
    run_speak(speaker)
    assert len(outputs) == 1 and edge.calls and piper.calls  # edge wrote nothing; piper took over
    assert np.array_equal(written_samples(outputs), robot_reference(piper_chunks))
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 1 and tl.count("error") == 0


def test_robot_voice_applies_to_the_piper_fallback_too():
    edge = FakeEngine("edge", error=ConnectionError("offline"))
    piper = FakeEngine("piper", voiced(0.3, rate=22_050))
    speaker, tl, _, outputs = build([edge, piper], tunables=ROBOT)
    run_speak(speaker, lang="es")
    played = written_samples(outputs)
    assert outputs[0].sample_rate == 22_050
    assert np.array_equal(played, robot_reference(piper._chunks)) and tl.count("speak_done") == 1


def test_robot_voice_stop_discards_the_pending_tail():
    """The effect holds a tail back; a stop must drop it instead of writing it late."""
    gate = threading.Event()
    engine = FakeEngine("edge", voiced(0.2, chunk=4800), gate=gate)  # one chunk, then blocked
    speaker, tl, ended, outputs = build([engine], tunables=ROBOT)
    speaker.speak(3, "Hello.", "en")
    deadline = time.monotonic() + 2
    while tl.count("speak_started") == 0 and time.monotonic() < deadline:
        time.sleep(0.005)
    assert tl.count("speak_started") == 1
    speaker.stop()
    writes_at_stop = len(outputs[0].written)
    gate.set()  # the engine now ends: the tail would be flushed here
    speaker.join(3)
    assert len(outputs[0].written) == writes_at_stop  # nothing written after the stop
    assert tl.count("speak_done") == 0 and tl.count("error") == 0 and ended == [3]


def test_finishing_the_effect_of_a_stopped_utterance_writes_nothing():
    speaker, tl, _, outputs = build([FakeEngine("edge", [])], tunables=ROBOT)
    run = _Run(tl.emit)
    run.fx = RobotVoice(24_000)
    run.fx.process(pcm16_to_float(voiced(0.3, chunk=4800)[0].data))  # leaves a tail inside
    run.cancel.set()
    assert speaker._finish_effect(run) == b"" and outputs == [] and run.fx is None


def test_a_device_failure_while_flushing_after_a_mid_utterance_engine_death_is_reported_as_such():
    messages = []
    tl = Timeline()
    outputs = []

    class FailsOnSecondWrite(FakeOutput):
        def write(self, pcm):
            if self.written:
                raise OutputError("device went away")
            super().write(pcm)

    def make_output(rate):
        outputs.append(FailsOnSecondWrite(tl, rate))
        return outputs[-1]

    edge = FakeEngine("edge", voiced(0.2, chunk=4800), error_after=ConnectionError("dropped"))
    ended = []
    speaker = Speaker([edge], make_output, lambda m: (messages.append(m), tl.emit(m)), ended.append, ROBOT)
    run_speak(speaker, speak_id=9)
    errors = [m for m in messages if m["type"] == "error"]
    assert len(errors) == 1
    assert errors[0]["code"] == "TTS_FAILED" and errors[0]["id"] == 9 and errors[0]["msg"] == "device went away"
    assert tl.count("speak_done") == 0 and ended == [9]


def test_a_sample_rate_change_mid_stream_plays_both_rates_and_flushes_the_old_tail():
    first, second = voiced(0.2, rate=24_000), voiced(0.2, rate=22_050)
    speaker, tl, ended, outputs = build([FakeEngine("edge", first + second)], tunables=ROBOT)
    run_speak(speaker)
    assert [o.sample_rate for o in outputs] == [24_000, 22_050]
    assert np.array_equal(np.frombuffer(b"".join(outputs[0].written), np.int16), robot_reference(first))
    assert np.array_equal(np.frombuffer(b"".join(outputs[1].written), np.int16), robot_reference(second))
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 1 and tl.count("error") == 0
    assert ended == [7]


def test_the_flushed_old_rate_tail_counts_as_heard_audio():
    """Only that tail is audible: the new rate carries silence, so `speak_started` hinges on it."""
    short = voiced(0.02)  # inside the start-up latency: nothing is written until the flush
    silence = [PcmChunk(bytes(960), 22_050)]
    speaker, tl, _, outputs = build([FakeEngine("edge", short + silence)], tunables=ROBOT)
    run_speak(speaker)
    assert outputs[0].sample_rate == 24_000 and outputs[0].written  # the old tail did reach the device
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 1 and tl.count("error") == 0


def test_odd_sized_chunks_do_not_crash_the_effect_and_play_the_same_samples():
    chunks = voiced(0.3)
    raw = b"".join(c.data for c in chunks)
    odd = [PcmChunk(raw[i : i + 481], 24_000) for i in range(0, len(raw), 481)]  # 481 bytes: splits samples
    speaker, tl, _, outputs = build([FakeEngine("edge", odd)], tunables=ROBOT)
    run_speak(speaker)
    assert tl.count("error") == 0 and tl.count("speak_started") == 1 and tl.count("speak_done") == 1
    played, reference = written_samples(outputs), robot_reference(chunks)
    assert played.shape == reference.shape
    assert np.abs(played.astype(int) - reference.astype(int)).max() <= 2  # chunking noise only


# ------------------------------------------------------------ output pre-open


class RatedEngine(FakeEngine):
    """An engine that announces its output rate, like the edge-tts one."""

    output_rate = 24_000


def wait_until(predicate, seconds=2.0):
    deadline = time.time() + seconds
    while not predicate() and time.time() < deadline:
        time.sleep(0.005)
    return predicate()


def build_prefetch(engines, tunables=PLAIN, open_gate=None, fail_open=False):
    """Like `build`, but records when each device was opened and can delay or fail the open."""
    tl = Timeline()
    ended, outputs = [], []

    def make_output(rate):
        tl.log.append(("open", rate))
        if open_gate is not None:
            open_gate.wait(2)
        if fail_open:
            raise OutputError("no device")
        out = FakeOutput(tl, rate)
        outputs.append(out)
        return out

    return Speaker(engines, make_output, tl.emit, ended.append, tunables), tl, ended, outputs


def test_the_device_is_opened_before_the_first_chunk_exists():
    gate = threading.Event()

    class SlowFirst(RatedEngine):
        def chunks(self, text, lang, cancel):
            gate.wait(2)  # synthesis still running
            yield from super().chunks(text, lang, cancel)

    speaker, tl, _, outputs = build_prefetch([SlowFirst("edge", [AUDIBLE])])
    speaker.speak(1, "Hello.", "en")
    deadline = time.time() + 2
    while not any(e[0] == "open" for e in tl.log) and time.time() < deadline:
        time.sleep(0.01)
    assert ("open", 24_000) in tl.log and not [e for e in tl.log if e[0] == "write"]
    gate.set()
    speaker.join(3)
    assert len(outputs) == 1 and outputs[0].written  # the pre-opened device is the one that plays
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 1


def test_without_the_switch_the_device_opens_at_the_first_audio():
    lazy = Tunables(robot_voice=False, preopen_output=False)
    speaker, tl, _, outputs = build_prefetch([RatedEngine("edge", [AUDIBLE])], tunables=lazy)
    run_speak(speaker)
    assert [e for e in tl.log if e[0] == "open"] == [("open", 24_000)]
    assert tl.log.index(("open", 24_000)) < next(i for i, e in enumerate(tl.log) if e[0] == "write")
    assert len(outputs) == 1


def test_an_engine_without_a_rate_announcement_opens_lazily():
    speaker, tl, _, outputs = build_prefetch([FakeEngine("piper", [AUDIBLE])])
    run_speak(speaker)
    assert len(outputs) == 1 and tl.count("speak_done") == 1


def test_a_device_opened_ahead_with_the_wrong_rate_is_closed_and_replaced():
    wrong = PcmChunk(AUDIBLE.data, 22_050)
    speaker, tl, _, outputs = build_prefetch([RatedEngine("edge", [wrong])])
    run_speak(speaker)
    assert [o.sample_rate for o in outputs] == [24_000, 22_050]
    assert outputs[0].closed and not outputs[0].written
    assert outputs[1].written and tl.count("speak_done") == 1


def test_a_device_opened_ahead_is_closed_when_no_audio_ever_arrives():
    speaker, tl, ended, outputs = build_prefetch([RatedEngine("edge", error=RuntimeError("endpoint down"))])
    run_speak(speaker)
    assert len(outputs) == 1 and outputs[0].closed and not outputs[0].written
    assert tl.count("error") == 1 and tl.count("speak_started") == 0
    assert ended == [7]


def test_stop_while_the_device_is_opening_ahead_closes_it_and_plays_nothing():
    gate = threading.Event()
    speaker, tl, ended, outputs = build_prefetch([RatedEngine("edge", [AUDIBLE, AUDIBLE])], open_gate=gate)
    speaker.speak(1, "Hello.", "en")
    assert wait_until(lambda: ("open", 24_000) in tl.log)  # the device is opening (and blocked on the gate)
    speaker.stop()
    gate.set()
    speaker.join(3)
    assert wait_until(lambda: len(outputs) == 1 and outputs[0].closed)  # the opener closes a late device itself
    assert [e for e in tl.log if e[0] == "write"] == []
    assert tl.count("speak_started") == 0 and tl.count("speak_done") == 0 and tl.count("error") == 0
    assert ended == [1]


def test_a_device_that_fails_to_open_ahead_is_tts_failed_at_the_first_audio_and_never_started():
    engines = [RatedEngine("edge", [AUDIBLE]), RatedEngine("piper", [AUDIBLE])]
    speaker, tl, ended, _ = build_prefetch(engines, fail_open=True)
    run_speak(speaker)
    assert tl.count("error") == 1 and tl.count("speak_started") == 0 and tl.count("speak_done") == 0
    assert ended == [7]


def test_each_utterance_opens_its_own_device_ahead():
    speaker, tl, _, outputs = build_prefetch([RatedEngine("edge", [AUDIBLE])])
    run_speak(speaker, speak_id=1)
    run_speak(speaker, speak_id=2)
    assert len(outputs) == 2 and all(o.closed and o.written for o in outputs)


def test_a_stop_interrupts_the_wait_for_a_device_that_is_still_opening():
    gate = threading.Event()
    prefetch = OutputPrefetch(lambda rate: gate.wait(5) and FakeOutput(Timeline(), rate), 24_000)
    outcome = []

    def taker():
        try:
            prefetch.take()
        except BaseException as exc:  # noqa: BLE001
            outcome.append(type(exc))

    thread = threading.Thread(target=taker)
    thread.start()
    prefetch.discard()  # what a stop does while the first audio waits for the device
    thread.join(2)
    gate.set()
    assert not thread.is_alive() and outcome == [_Cancelled]


def test_a_device_that_never_opens_is_tts_failed_not_a_hang(monkeypatch):
    import tts

    monkeypatch.setattr(tts, "OUTPUT_OPEN_TIMEOUT_S", 0.2)
    gate = threading.Event()
    speaker, tl, ended, outputs = build_prefetch([RatedEngine("edge", [AUDIBLE])], open_gate=gate)
    run_speak(speaker)  # joins with a 3 s limit: a hang would leave the thread alive
    assert not speaker._thread.is_alive()
    assert tl.count("error") == 1 and tl.count("speak_started") == 0 and tl.count("speak_done") == 0
    assert ended == [7]
    gate.set()  # the driver finally answers: the late device is closed, never played
    assert wait_until(lambda: len(outputs) == 1 and outputs[0].closed) and not outputs[0].written


def test_discarding_an_unused_device_aborts_it_before_closing_it():
    calls = []

    class Recording(FakeOutput):
        def abort(self):
            calls.append("abort")

        def close(self):
            calls.append("close")

    prefetch = OutputPrefetch(lambda rate: Recording(Timeline(), rate), 24_000)
    assert prefetch._opened.wait(2)
    prefetch.discard()
    assert calls == ["abort", "close"]


# ------------------------------------------------------------------ level messages


TONE_1S = PcmChunk(
    np.rint(0.3 * 32767 * np.sin(2 * np.pi * 900 * np.arange(24_000) / 24_000)).astype(np.int16).tobytes(), 24_000
)


class DrainingOutput(FakeOutput):
    """A device that takes a while to play out its buffer on close, like the real one."""

    latency = 0.0

    def close(self):
        time.sleep(0.25)
        super().close()


def build_levels(engines, output_class=DrainingOutput):
    timeline, ended, outputs = Timeline(), [], []

    def make_output(rate):
        out = output_class(timeline, rate)
        outputs.append(out)
        return out

    return Speaker(engines, make_output, timeline.emit, ended.append, PLAIN), timeline, ended, outputs


def test_idle_sends_no_tts_level():
    speaker, tl, ended, outputs = build_levels([FakeEngine("edge", [TONE_1S])])
    time.sleep(0.2)
    assert tl.count("level") == 0  # nobody spoke
    speaker.stop()  # a stop with nothing playing is harmless and sends nothing either
    assert tl.count("level") == 0


def test_levels_flow_while_the_voice_is_audible_and_end_with_the_speech():
    speaker, tl, ended, outputs = build_levels([FakeEngine("edge", [TONE_1S])])
    run_speak(speaker)
    kinds = [entry[1] for entry in tl.log if entry[0] == "event"]
    assert kinds[0] == "speak_started" and kinds[-1] == "speak_done"
    levels = kinds.count("level")
    # Real-time smoke test: ~31 levels/s while the 250 ms drain lasted (~8). Bounds are wide on purpose so a
    # loaded machine does not flake; it checks that levels flow and stop, not their exact rate.
    assert 1 <= levels <= 20
    assert all(k == "level" for k in kinds[1:-1])  # nothing else in between
    sent = tl.count("level")
    time.sleep(0.2)
    assert tl.count("level") == sent  # the pacer is gone with the speech


def test_a_barge_in_stops_the_levels_at_once():
    gate = threading.Event()
    speaker, tl, ended, outputs = build_levels([FakeEngine("edge", [TONE_1S], gate=gate)])
    speaker.speak(7, "Hello there.", "en")
    assert wait_until(lambda: tl.count("level") >= 2)
    started = time.monotonic()
    speaker.stop()
    assert time.monotonic() - started < 0.1
    count = tl.count("level")
    gate.set()
    speaker.join(3)
    time.sleep(0.15)
    assert tl.count("level") == count  # not one level after the stop
    assert tl.count("speak_done") == 0


def test_a_level_write_stuck_on_a_stalled_host_never_blocks_a_barge_in():
    """Levels are disposable: a host that stops reading the pipe must not keep the voice playing."""
    stuck, release = threading.Event(), threading.Event()
    starts, tl = [], Timeline()

    def emit(message):
        if message["type"] == "level":
            starts.append(time.monotonic())
            stuck.set()
            release.wait(5)  # the blocking stdout write of a full pipe
        tl.emit(message)

    gate, outputs = threading.Event(), []

    def make_output(rate):
        outputs.append(DrainingOutput(tl, rate))
        return outputs[-1]

    speaker = Speaker([FakeEngine("edge", [TONE_1S], gate=gate)], make_output, emit, [].append, PLAIN)
    try:
        speaker.speak(7, "Hello there.", "en")
        assert stuck.wait(2)  # a level write is blocked right now
        stopper = threading.Thread(target=speaker.stop)
        began = time.monotonic()
        stopper.start()
        stopper.join(1.0)
        assert not stopper.is_alive(), "the barge-in waited for the blocked level write"
        assert time.monotonic() - began < 1.0
        assert outputs[0].aborted
        count = len(starts)
        release.set()
        gate.set()
        speaker.join(3)
        time.sleep(0.15)  # real-time smoke: give a stray level the chance to show up
        assert len(starts) == count  # no level write began after the stop returned
    finally:
        release.set()
        gate.set()


def _pacer_threads():
    return [t for t in threading.enumerate() if t.name == "level-pacer" and t.is_alive()]


def test_levels_continue_across_an_engine_fallback_at_another_sample_rate():
    quiet_edge = PcmChunk(bytes(960), 24_000)  # 20 ms of silence: the carry of a slice not yet complete
    tone_22k = PcmChunk(TONE_1S.data, 22_050)
    seen = []
    speaker, tl, ended, outputs = build_levels(
        [FakeEngine("edge", [quiet_edge]), FakeEngine("piper", [tone_22k])],
    )
    emit = speaker._emit

    def spy(message):
        if message["type"] == "level":
            seen.append(len(outputs))  # which device was open when the level went out
        emit(message)

    speaker._emit = spy
    run_speak(speaker)
    assert [o.sample_rate for o in outputs] == [24_000, 22_050]
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 1 and tl.count("error") == 0
    assert seen and set(seen) == {2}  # every level belongs to the new rate, none from the silent old one
    kinds = [entry[1] for entry in tl.log if entry[0] == "event"]
    assert kinds[0] == "speak_started" and kinds[-1] == "speak_done"
    assert "level" in kinds
    assert _pacer_threads() == []  # one pacer, gone with the speech
    sent = tl.count("level")
    time.sleep(0.15)
    assert tl.count("level") == sent  # no emit after the end


def test_a_device_failure_mid_utterance_stops_the_levels():
    class DiesOnSecondWrite(FakeOutput):
        latency = 0.0

        def write(self, pcm):
            if self.written:
                raise OutputError("device went away")
            super().write(pcm)

    tone = [PcmChunk(TONE_1S.data[i : i + 9600], 24_000) for i in range(0, len(TONE_1S.data), 9600)]
    speaker, tl, ended, outputs = build_levels([FakeEngine("edge", tone)], output_class=DiesOnSecondWrite)
    run_speak(speaker, speak_id=5)
    assert tl.count("speak_started") == 1 and tl.count("speak_done") == 0
    assert tl.count("error") == 1 and ended == [5]
    assert _pacer_threads() == []  # `finish` on the failure path
    sent = tl.count("level")
    time.sleep(0.15)
    assert tl.count("level") == sent
