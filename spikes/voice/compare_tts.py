# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = [
#   "piper-tts",
#   "edge-tts",
# ]
# ///
"""Throwaway spike S2: compare edge-tts and Piper by ear and by latency.

Writes the same phrases with both engines to spikes/voice/samples/ and prints
time to first audio (edge-tts) and total synthesis time (both).

Usage:
  uv run compare_tts.py
"""

import asyncio
import time
import urllib.request
import wave
from pathlib import Path

import edge_tts

HERE = Path(__file__).parent
MODELS = HERE / "models"
SAMPLES = HERE / "samples"
VOICES_URL = "https://huggingface.co/rhasspy/piper-voices/resolve/main/"

PHRASES = {
    "es": "Listo, abrí Spotify y puse el volumen al cincuenta por ciento. ¿Necesitás algo más?",
    "en": "Done, I opened Spotify and set the volume to fifty percent. Anything else?",
}
EDGE_VOICES = {"es": "es-AR-ElenaNeural", "en": "en-US-AriaNeural"}
PIPER_VOICES = {
    "es": "es/es_AR/daniela/high/es_AR-daniela-high",
    "en": "en/en_US/lessac/medium/en_US-lessac-medium",
}


def download_piper_voice(remote):
    name = remote.rsplit("/", 1)[-1]
    onnx = MODELS / f"{name}.onnx"
    for suffix in (".onnx", ".onnx.json"):
        target = MODELS / f"{name}{suffix}"
        if not target.exists():
            print(f"downloading {name}{suffix} ...")
            urllib.request.urlretrieve(VOICES_URL + remote + suffix, target)
    return onnx


async def edge_sample(lang, out):
    t0 = time.perf_counter()
    first = None
    data = b""
    async for chunk in edge_tts.Communicate(PHRASES[lang], EDGE_VOICES[lang]).stream():
        if chunk["type"] == "audio":
            first = first or time.perf_counter() - t0
            data += chunk["data"]
    out.write_bytes(data)
    return first, time.perf_counter() - t0


def piper_sample(lang, out):
    from piper import PiperVoice

    voice = PiperVoice.load(str(download_piper_voice(PIPER_VOICES[lang])))
    t0 = time.perf_counter()
    with wave.open(str(out), "wb") as wav:
        voice.synthesize_wav(PHRASES[lang], wav)
    return time.perf_counter() - t0


def main():
    MODELS.mkdir(exist_ok=True)
    SAMPLES.mkdir(exist_ok=True)
    for lang in PHRASES:
        first, total = asyncio.run(edge_sample(lang, SAMPLES / f"{lang}-edge.mp3"))
        print(f"[{lang}] edge-tts: first audio {first:.2f}s, total {total:.2f}s")
        total = piper_sample(lang, SAMPLES / f"{lang}-piper.wav")
        print(f"[{lang}] piper:    total {total:.2f}s (first audio = total, no streaming here)")
    print(f"\nSamples in {SAMPLES}")


if __name__ == "__main__":
    main()
