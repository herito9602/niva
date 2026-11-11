"""levels.bands: 7 log bands of one slice of Niva's voice, as 0-100 integers."""

from __future__ import annotations

import numpy as np
import pytest

from levels import BAND_EDGES_HZ, DB_FLOOR, DB_TOP, bands, level_from_db

RATE = 24_000


def sine(freq: float, amplitude: float = 0.3, rate: int = RATE, ms: int = 32) -> bytes:
    t = np.arange(rate * ms // 1000) / rate
    return np.rint(amplitude * 32767 * np.sin(2 * np.pi * freq * t)).astype(np.int16).tobytes()


def test_there_are_seven_bands_between_80_hz_and_8_khz():
    assert BAND_EDGES_HZ == (80, 160, 320, 640, 1280, 2560, 4000, 8000)


@pytest.mark.parametrize(
    ("freq", "band"),
    [(110, 0), (220, 1), (450, 2), (900, 3), (1800, 4), (3200, 5), (6000, 6)],
)
def test_sine_lands_in_expected_band(freq, band):
    got = bands(sine(freq), RATE)
    assert got.index(max(got)) == band
    assert got[band] > 50
    # The neighbours two bands away hear only leakage.
    for other in range(7):
        if abs(other - band) >= 2:
            assert got[other] < got[band] - 30


def test_silence_is_all_zero():
    assert bands(bytes(768 * 2), RATE) == [0] * 7


def test_returns_seven_ints_in_0_100():
    rng = np.random.default_rng(1)
    noise = rng.integers(-32768, 32767, 768, dtype=np.int16).tobytes()
    got = bands(noise, RATE)
    assert len(got) == 7
    assert all(type(v) is int and 0 <= v <= 100 for v in got)


def test_dbfs_mapping_endpoints():
    assert (DB_FLOOR, DB_TOP) == (-60.0, -10.0)
    assert level_from_db(-60.0) == 0
    assert level_from_db(-10.0) == 100
    assert level_from_db(-35.0) == 50
    assert level_from_db(-90.0) == 0  # clamped below
    assert level_from_db(0.0) == 100  # clamped above
    assert level_from_db(float("-inf")) == 0


def test_a_loud_sine_reaches_the_top_and_a_quiet_one_the_bottom():
    assert bands(sine(900, amplitude=0.9), RATE)[3] == 100
    assert bands(sine(900, amplitude=0.0005), RATE)[3] == 0


def test_short_frame_does_not_raise():
    assert bands(b"", RATE) == [0] * 7
    assert bands(b"\x01", RATE) == [0] * 7  # a lone byte is not a sample
    assert len(bands(np.array([5], dtype=np.int16), RATE)) == 7
    assert len(bands(sine(900, ms=1), RATE)) == 7


def test_accepts_an_int16_array_as_well_as_bytes():
    raw = sine(900)
    assert bands(np.frombuffer(raw, dtype=np.int16), RATE) == bands(raw, RATE)
