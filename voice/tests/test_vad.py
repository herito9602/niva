"""Endpointer rules, and the streaming Silero model on the owner's recordings."""

import dataclasses
from pathlib import Path

import numpy as np
import pytest

import margin
from config import FRAME_MS, FRAME_SAMPLES, Tunables
from vad import End, Endpointer, Onset, SileroStream

AUDIO = Path(__file__).resolve().parents[2] / "spikes" / "vad" / "audio"
OWNER_WAVS = sorted(AUDIO.glob("*.wav"))


def run(probs, tunables=None):
    endpointer = Endpointer(tunables or Tunables())
    return [(i, endpointer.push(p)) for i, p in enumerate(probs)]


def signals(probs, tunables=None):
    return [(i, s) for i, s in run(probs, tunables) if s is not None]


def test_onset_needs_two_speech_frames_in_a_row():
    assert signals([0.1, 0.9, 0.1, 0.9, 0.1]) == []
    got = signals([0.1, 0.1, 0.9, 0.9, 0.9])
    assert got == [(3, Onset(frames_ago=1))]


def test_end_waits_500_ms_of_silence_like_the_vad_spike():
    t = Tunables()
    frames_needed = -(-t.end_silence_ms // FRAME_MS)  # 16 chunks, about 532 ms with the chunk itself
    probs = [0.9] * 5 + [0.05] * 40
    got = signals(probs, t)
    end = [(i, s) for i, s in got if isinstance(s, End)]
    assert len(end) == 1
    index, signal = end[0]
    assert signal.silence_frames_ago == frames_needed
    assert index == 5 + frames_needed  # first silent frame is index 5


def test_the_running_silence_is_visible_until_it_reaches_the_end_value():
    endpointer = Endpointer(Tunables())
    seen = []
    for prob in [0.9] * 4 + [0.05] * 17:
        signal = endpointer.push(prob)
        seen.append((endpointer.silence_frames, signal))
    assert [s for s, _ in seen[:4]] == [None] * 4  # speech: no silence running
    assert [s for s, _ in seen[4:20]] == list(range(16))  # 0 at the first silent frame, 15 just before the end
    assert isinstance(seen[20][1], End) and seen[20][0] is None, "the end resets it"
    assert endpointer.push(0.9) is None and endpointer.silence_frames is None


def test_a_short_pause_does_not_end_the_utterance():
    pause = int(400 / FRAME_MS)
    probs = [0.9] * 5 + [0.05] * pause + [0.9] * 5 + [0.05] * 30
    ends = [s for _, s in signals(probs) if isinstance(s, End)]
    assert len(ends) == 1
    assert [i for i, s in signals(probs) if isinstance(s, End)][0] > 5 + pause + 5


def test_in_between_probabilities_neither_start_nor_cancel_the_silence():
    # 0.4 is between the negative (0.35) and positive (0.5) thresholds
    probs = [0.9] * 3 + [0.05] + [0.4] * 20
    ends = [s for _, s in signals(probs) if isinstance(s, End)]
    assert len(ends) == 1  # the wait runs through the in-between frames, as in faster-whisper


def test_speech_resets_the_silence_wait():
    probs = [0.9] * 3 + [0.05] * 10 + [0.9] + [0.05] * 10
    assert [s for _, s in signals(probs) if isinstance(s, End)] == []


def test_end_silence_is_a_tunable():
    t = dataclasses.replace(Tunables(), end_silence_ms=200)
    probs = [0.9] * 3 + [0.05] * 20
    ((index, signal),) = [(i, s) for i, s in signals(probs, t) if isinstance(s, End)]
    assert signal.silence_frames_ago == 7


def test_silero_stream_scores_silence_low_and_keeps_state():
    vad = SileroStream()
    quiet = np.zeros(FRAME_SAMPLES, dtype=np.float32)
    probs = [vad(quiet) for _ in range(5)]
    assert all(0.0 <= p < 0.2 for p in probs)
    with pytest.raises(ValueError):
        vad(np.zeros(100, dtype=np.float32))


def test_silero_stream_hears_a_loud_voiced_tone_as_more_speech_than_silence():
    vad = SileroStream()
    t = np.arange(FRAME_SAMPLES * 20) / 16_000
    voiced = (0.4 * np.sin(2 * np.pi * 180 * t) * (1 + 0.5 * np.sin(2 * np.pi * 4 * t))).astype(np.float32)
    probs = [vad(voiced[i : i + FRAME_SAMPLES]) for i in range(0, len(voiced), FRAME_SAMPLES)]
    vad.reset()
    silent = [vad(np.zeros(FRAME_SAMPLES, dtype=np.float32)) for _ in range(20)]
    assert max(probs) >= max(silent)


@pytest.mark.skipif(not OWNER_WAVS, reason="owner recordings (spikes/vad/audio) are local-only")
def test_streaming_vad_matches_the_offline_reference_on_the_owner_recordings():
    """The live path must agree with the offline path the VAD spike measured."""
    rows = margin.measure(OWNER_WAVS)
    assert len(rows) >= 1
    for row in rows:
        assert row.offline_end_ms is not None, row.name
        assert row.end_ms == row.offline_end_ms, f"{row.name}: live end differs from the 500 ms offline reference"
        if row.offline_start_ms is not None and row.onset_ms is not None:
            # the live onset waits for two speech frames, so it can only be later, never earlier
            assert row.onset_ms >= row.offline_start_ms, row.name


@pytest.mark.skipif(not OWNER_WAVS, reason="owner recordings (spikes/vad/audio) are local-only")
def test_the_pre_speech_buffer_covers_the_start_margin_on_the_owner_recordings():
    """Speech-io: the first syllable must not be cut by the detection delay."""
    t = Tunables()
    rows = margin.measure(OWNER_WAVS, t)
    needs = [row.need_ms for row in rows if row.need_ms is not None]
    assert needs
    assert max(needs) <= t.pre_speech_ms
    # and the confirmation delay itself is small next to the buffer
    delays = [row.confirmed_ms - row.onset_ms for row in rows if row.onset_ms is not None]
    assert max(delays) <= t.onset_ms + FRAME_MS
