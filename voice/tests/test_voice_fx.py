import numpy as np
import pytest

from voice_fx import (
    LIMIT_KNEE,
    PITCH_FACTOR,
    RobotVoice,
    float_to_pcm16,
    pcm16_to_float,
    soft_limit,
)

RATE = 24_000


def speechlike(seconds=0.6, rate=RATE, seed=3):
    """Voiced buzz with a slow amplitude envelope plus a little noise."""
    rng = np.random.default_rng(seed)
    t = np.arange(int(seconds * rate)) / rate
    envelope = 0.5 + 0.5 * np.sin(2 * np.pi * 3 * t)
    voice = 0.5 * np.sin(2 * np.pi * 150 * t) + 0.25 * np.sin(2 * np.pi * 450 * t)
    return (envelope * voice + 0.02 * rng.standard_normal(t.size)).astype(np.float32)


def run(signal, chunk, rate=RATE):
    fx = RobotVoice(rate)
    parts = [fx.process(signal[i : i + chunk]) for i in range(0, len(signal), chunk)]
    parts.append(fx.flush())
    return np.concatenate(parts)


@pytest.mark.parametrize("chunk", [1, 37, 480, 4096])
def test_chunked_processing_matches_one_shot(chunk):
    signal = speechlike(0.25 if chunk == 1 else 0.6)  # one-sample chunks are slow
    expected = run(signal, len(signal))
    got = run(signal, chunk)
    assert got.shape == expected.shape
    assert np.max(np.abs(got - expected)) < 1e-4


def test_no_discontinuity_at_chunk_boundaries():
    signal = speechlike()
    got = run(signal, 480)
    expected = run(signal, len(signal))
    jumps, reference_jumps = np.abs(np.diff(got)), np.abs(np.diff(expected))
    assert jumps.shape == reference_jumps.shape
    # every step, chunk boundaries included, is the step the uninterrupted signal takes
    assert np.max(np.abs(jumps - reference_jumps)) < 1e-4
    assert jumps.max() < 0.5  # no click anywhere


def test_output_length_is_input_over_pitch_factor():
    signal = speechlike(1.0)
    out = run(signal, 480)
    assert abs(len(out) - len(signal) / PITCH_FACTOR) <= 2


@pytest.mark.parametrize("rate", [16_000, 22_050, 24_000, 44_100])
def test_any_sample_rate_works_and_keeps_duration(rate):
    signal = speechlike(0.5, rate=rate)
    out = run(signal, 512, rate)
    assert abs(len(out) - len(signal) / PITCH_FACTOR) <= 2
    assert np.abs(out).max() > 0.05


def test_output_stays_inside_full_scale_even_for_loud_input():
    loud = np.clip(speechlike() * 8.0, -1.0, 1.0)
    out = run(loud, 480)
    assert np.abs(out).max() <= 1.0


def test_soft_limit_is_identity_below_the_knee_and_bounded_above():
    x = np.array([-5.0, -1.0, -LIMIT_KNEE, -0.1, 0.0, 0.3, LIMIT_KNEE, 1.0, 5.0], dtype=np.float32)
    y = soft_limit(x)
    inside = np.abs(x) <= LIMIT_KNEE
    assert np.array_equal(y[inside], x[inside])
    assert np.abs(y).max() <= 1.0
    assert np.all(np.diff(y) > 0)  # monotonic


def test_level_is_close_to_the_dry_voice():
    signal = speechlike(1.0)
    out = run(signal, 480)
    rms = lambda a: float(np.sqrt(np.mean(a**2)))  # noqa: E731
    assert 0.5 < rms(out) / rms(signal) < 2.0


def test_state_resets_between_utterances():
    signal = speechlike(0.4)
    fx = RobotVoice(RATE)

    def utterance():
        out = [fx.process(signal[i : i + 480]) for i in range(0, len(signal), 480)]
        out.append(fx.flush())
        return np.concatenate(out)

    first, second = utterance(), utterance()
    assert np.array_equal(first, second)


def test_flush_without_audio_is_empty():
    fx = RobotVoice(RATE)
    assert fx.flush().size == 0
    assert fx.process(np.zeros(0, dtype=np.float32)).size == 0


def test_first_audio_does_not_wait_for_the_whole_utterance():
    fx = RobotVoice(RATE)
    signal = speechlike(0.6)
    fed = 0
    first = None
    for i in range(0, len(signal), 480):
        out = fx.process(signal[i : i + 480])
        fed += 480
        if out.size and first is None:
            first = fed
    assert first is not None and first / RATE < 0.08  # within about 80 ms of input
    assert fx.latency_samples / RATE < 0.03  # held-back samples: about 20 ms


def test_reset_discards_pending_state():
    fx = RobotVoice(RATE)
    fx.process(speechlike(0.3))
    fx.reset()
    assert fx.flush().size == 0


def test_pcm16_roundtrip_helpers():
    pcm = np.array([0, 1000, -1000, 32767, -32768], dtype=np.int16).tobytes()
    back = float_to_pcm16(pcm16_to_float(pcm))
    assert np.abs(np.frombuffer(back, np.int16).astype(int) - np.frombuffer(pcm, np.int16).astype(int)).max() <= 1
    assert float_to_pcm16(np.array([2.0, -2.0], dtype=np.float32)) == np.array([32767, -32767], dtype=np.int16).tobytes()
