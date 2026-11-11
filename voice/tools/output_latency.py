"""Prints the latency of the default output device and how its blocking writes behave.

Run it from voice/ (it plays silence): .venv/Scripts/python.exe tools/output_latency.py
The level pacer (level_pacer.py) schedules the notch waveform with this latency; a value the
device under-reports shows up as bars that move before the sound (use NIVA_TTS_LEVEL_OFFSET_MS).
"""

from __future__ import annotations

import statistics
import time

import sounddevice as sd

RATE = 24_000
CHUNK = 1024


def main() -> None:
    device = sd.query_devices(sd.default.device[1])
    api = sd.query_hostapis(device["hostapi"])["name"]
    stream = sd.RawOutputStream(samplerate=RATE, channels=1, dtype="int16")
    stream.start()
    print(f"device: {device['name']} ({api})")
    print(f"stream.latency: {stream.latency * 1000:.0f} ms")
    silence = bytes(CHUNK * 2)
    blocked = []
    for _ in range(60):
        started = time.perf_counter()
        stream.write(silence)
        blocked.append((time.perf_counter() - started) * 1000)
    steady = blocked[20:]
    print(f"chunk length: {CHUNK / RATE * 1000:.1f} ms")
    print(f"write blocks for (steady state): mean {statistics.mean(steady):.1f} ms, max {max(steady):.1f} ms")
    stream.stop()
    stream.close()


if __name__ == "__main__":
    main()
