"""LevelPacer: band levels of Niva's voice, emitted when the sound is AUDIBLE.

The pacer is driven by a fake clock, so these tests need no sound card and no
sleeping (the barge-in test is the one exception: it checks a real thread)."""

from __future__ import annotations

import random
import threading
import time

import numpy as np
import pytest
from conftest import FakeAudioOutput

import audio_io
from audio_io import AudioOutput
from level_pacer import SLICE_MS, LevelPacer, level_offset_ms

RATE = 24_000
SLICE = RATE * SLICE_MS // 1000  # 768 samples
LAT = 0.18  # what MME reported on the dev machine


def tone(samples: int, amplitude: float = 0.3) -> bytes:
    t = np.arange(samples) / RATE
    return np.rint(amplitude * 32767 * np.sin(2 * np.pi * 900 * t)).astype(np.int16).tobytes()


class Clock:
    def __init__(self, now: float = 100.0) -> None:
        self.now = now

    def __call__(self) -> float:
        return self.now


def pacer(clock: Clock | None = None, offset_ms: float = 0.0, sink: list | None = None) -> LevelPacer:
    clock = clock or Clock()
    out = sink if sink is not None else []
    return LevelPacer(lambda bands: out.append((clock.now, bands)), clock=clock, offset_ms=offset_ms, threaded=False)


# -------------------------------------------------------------- the output protocol


def test_fake_output_exposes_latency():
    out = FakeAudioOutput(24_000, latency=0.25)
    assert out.latency == 0.25
    assert out.sample_rate == 24_000
    assert "latency" in AudioOutput.__annotations__  # part of the protocol, not just of the fake


def test_the_real_output_reports_the_stream_latency():
    assert isinstance(audio_io.SoundDeviceOutput.latency, property)


# ----------------------------------------------------------------------- slicing


def test_slices_are_32ms():
    p = pacer()
    p.on_write(tone(SLICE * 3), RATE, write_return=100.0, latency=LAT)
    assert len(p.pending) == 3
    assert all(len(s.samples) == SLICE * 2 for s in p.pending)
    gaps = [b.due - a.due for a, b in zip(p.pending, p.pending[1:], strict=False)]
    assert gaps == pytest.approx([0.032, 0.032])


def test_a_remainder_is_carried_to_the_next_chunk_not_sent_short():
    p = pacer()
    p.on_write(tone(1024), RATE, write_return=100.0, latency=LAT)  # 1.33 slices
    assert len(p.pending) == 1
    p.on_write(tone(1024), RATE, write_return=100.0427, latency=LAT)  # completes slice two (and part of three)
    assert len(p.pending) == 2
    assert all(len(s.samples) == SLICE * 2 for s in p.pending)


# ------------------------------------------------------------------- the clock


def test_first_write_anchors_clock():
    p = pacer()
    p.on_write(tone(SLICE), RATE, write_return=250.0, latency=LAT)
    assert p.pending[0].due == pytest.approx(250.0 + LAT)


def test_due_times_follow_playback_clock():
    """A fast producer: the first chunks are handed over at once, then every write
    blocks until the device has room. The audible time of sample n is
    anchor + n / rate, and so must every slice's due time be."""
    p = pacer()
    dur = 1024 / RATE
    anchor = 100.0 + LAT  # the first chunk is handed at t=100 and is audible LAT later
    started = 100.0
    for k in range(40):
        returned = max(100.0, 100.0 + (k + 1) * dur - LAT)  # blocked once the buffer holds LAT of audio
        p.on_write(tone(1024), RATE, write_return=returned, latency=LAT, write_start=started)
        started = returned
    assert len(p.pending) >= 40 * 1024 // SLICE - 1
    for index, s in enumerate(p.pending):
        assert s.due == pytest.approx(anchor + index * SLICE / RATE, abs=0.001), index


def test_a_gap_inside_the_tolerance_does_not_move_the_clock():
    p = pacer()
    p.on_write(tone(SLICE * 2), RATE, write_return=100.0, latency=LAT)
    # The next chunk is written 20 ms later than the audio needs: too small a gap to re-anchor for.
    p.on_write(tone(SLICE * 2), RATE, write_return=100.0 + 0.064 + 0.02, latency=LAT)
    assert [s.due for s in p.pending] == pytest.approx([100.0 + LAT + i * 0.032 for i in range(4)], abs=1e-6)


def test_underrun_reanchors():
    p = pacer()
    p.on_write(tone(SLICE * 2), RATE, write_return=100.0, latency=LAT)
    # The network stalled for a second: the device played silence, then this chunk.
    p.on_write(tone(SLICE * 2), RATE, write_return=101.5, latency=LAT)
    assert p.pending[2].due == pytest.approx(101.5 + LAT)
    assert p.pending[3].due == pytest.approx(101.5 + LAT + 0.032)


def test_level_offset_env_shifts_due(monkeypatch):
    monkeypatch.setenv("NIVA_TTS_LEVEL_OFFSET_MS", "-40")
    assert level_offset_ms() == -40.0
    p = pacer(offset_ms=level_offset_ms())
    p.on_write(tone(SLICE), RATE, write_return=100.0, latency=LAT)
    assert p.pending[0].due == pytest.approx(100.0 + LAT - 0.040)
    monkeypatch.delenv("NIVA_TTS_LEVEL_OFFSET_MS")
    assert level_offset_ms() == 0.0
    for junk in ("abc", "", "nan", "inf", "99999"):
        monkeypatch.setenv("NIVA_TTS_LEVEL_OFFSET_MS", junk)
        assert level_offset_ms() == 0.0, junk


# --------------------------------------------------------------------- emitting


def test_a_slice_is_emitted_when_it_is_due_and_not_before():
    clock, sink = Clock(100.0), []
    p = pacer(clock, sink=sink)
    p.on_write(tone(SLICE), RATE, write_return=100.0, latency=LAT)
    clock.now = 100.0 + LAT - 0.001
    p.pump()
    assert sink == []
    clock.now = 100.0 + LAT
    p.pump()
    assert len(sink) == 1
    bands = sink[0][1]
    assert len(bands) == 7 and bands.index(max(bands)) == 3  # 900 Hz


def test_tts_level_rate_20_to_40_hz():
    clock, sink = Clock(100.0), []
    p = pacer(clock, sink=sink)
    p.on_write(tone(RATE * 2), RATE, write_return=100.0, latency=0.0)  # 2 s of audio
    while clock.now < 101.0:
        p.pump()
        clock.now += 0.005
    assert 20 <= len(sink) <= 40
    gaps = [b[0] - a[0] for a, b in zip(sink, sink[1:], strict=False)]
    assert min(gaps) >= 0.025 - 1e-9  # never more than one emit per 25 ms


def test_after_a_stall_only_the_newest_due_slice_is_sent():
    clock, sink = Clock(100.0), []
    p = pacer(clock, sink=sink)
    p.on_write(tone(SLICE * 5), RATE, write_return=100.0, latency=0.0)
    clock.now = 100.5  # the thread was starved: five slices are overdue
    p.pump()
    assert len(sink) == 1
    assert p.pending == []


def test_pacer_flushed_on_speech_end():
    clock, sink = Clock(100.0), []
    p = pacer(clock, sink=sink)
    p.on_write(tone(SLICE * 4), RATE, write_return=100.0, latency=0.0)
    clock.now = 100.0
    p.finish()  # speech ended: what is due goes out, the rest never does
    sent = len(sink)
    assert sent == 1 and p.pending == []
    clock.now = 105.0
    p.pump()
    assert len(sink) == sent


def test_nothing_is_scheduled_after_stop():
    clock, sink = Clock(100.0), []
    p = pacer(clock, sink=sink)
    p.stop()
    p.on_write(tone(SLICE), RATE, write_return=100.0, latency=0.0)
    clock.now = 101.0
    p.pump()
    assert sink == [] and p.pending == []


def test_pacer_stops_within_one_period_on_barge_in():
    sink, got_first = [], threading.Event()

    def emit(bands):
        sink.append(bands)
        got_first.set()

    p = LevelPacer(emit)  # the real thread and the real clock
    p.on_write(tone(RATE * 3), RATE, write_return=time.perf_counter(), latency=0.0)
    assert got_first.wait(1.0)
    started = time.perf_counter()
    p.stop()
    assert time.perf_counter() - started < 0.033
    count = len(sink)
    time.sleep(0.15)
    assert len(sink) == count
    assert not p.running


# --------------------------------------------------------- onset lag (design L14)


def _onset_lags(producer_gap_s, ready_jitter_s, seed: int) -> list[float]:
    """Plays a click train (100 ms burst every 500 ms) through a simulated blocking
    output of latency LAT and returns, for every burst onset, how late (+) or early
    (-) the pacer would show it compared with when it is audible."""
    rng = random.Random(seed)
    clicks = RATE * 500 // 1000
    audio = b"".join(tone(RATE // 10, 0.5) + bytes((clicks - RATE // 10) * 2) for _ in range(8))
    chunk = 1024
    p = pacer()
    prev_end, ready, truth_start_of = None, 100.0, []
    for offset in range(0, len(audio), chunk * 2):
        piece = audio[offset : offset + chunk * 2]
        dur = len(piece) // 2 / RATE
        ready += producer_gap_s() + rng.uniform(0, ready_jitter_s)
        write_started = ready
        if prev_end is None or ready + LAT >= prev_end:  # the device was empty: handed at once
            start = ready + LAT
            returned = ready
        else:  # the device still had audio: audible right after it, and the write blocks until there is room
            start = prev_end
            returned = max(ready, start + dur - LAT)
        ready = returned  # the producer carries on from the moment the write returned
        prev_end = start + dur
        truth_start_of.append((offset // 2, start))
        p.on_write(piece, RATE, write_return=returned, latency=LAT, write_start=write_started)

    # Truth: the audible time of sample n, from the chunk it lives in.
    def audible(n: int) -> float:
        base_sample, start = max((c for c in truth_start_of if c[0] <= n), key=lambda c: c[0])
        return start + (n - base_sample) / RATE

    # (A re-anchor drops the sub-slice remainder, so a slice is located by its own bytes,
    # searching from where it should be: the same onset pattern recurs in later bursts.)
    lags = []
    for index, s in enumerate(p.pending):
        loud = max(s.bands) > 20
        was_quiet = index == 0 or max(p.pending[index - 1].bands) <= 20
        if loud and was_quiet:
            at = audio.find(s.samples, max(0, index - 2) * SLICE * 2)
            while at % 2:
                at = audio.find(s.samples, at + 1)
            lags.append(s.due - audible(at // 2))
    return lags


@pytest.mark.parametrize(
    ("name", "gap", "jitter"),
    [
        ("fast producer, writes block", lambda: 0.0, 0.0),
        ("real-time producer with decoder jitter", lambda: 1024 / RATE, 0.012),
    ],
)
def test_onset_lag_stays_inside_the_bound(name, gap, jitter):
    lags = _onset_lags(gap, jitter, seed=3)
    assert len(lags) >= 6, name
    # L14: the rise of a burst is shown, on average, within 50 ms of when it is heard,
    # and never more than 100 ms late. (A burst shown early is also wrong, so bound that too.)
    assert sum(lags) / len(lags) <= 0.050, (name, lags)
    assert max(lags) <= 0.100, (name, lags)
    assert min(lags) >= -0.080, (name, lags)
