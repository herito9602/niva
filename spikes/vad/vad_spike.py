# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = [
#   "onnxruntime",
#   "sounddevice",
#   "numpy<2",
#   "faster-whisper",
# ]
# ///
"""Spike before slice 3: how long must Silero VAD wait to end a command?

The latency budget assumes 400 ms, the only unmeasured line. Waiting less cuts a
sentence at a natural pause; waiting more adds latency to every command. So one
set of real recordings is replayed at several silence settings.

  uv run vad_spike.py record    # reads phrases, you say them, saves WAV files
  uv run vad_spike.py sweep     # no microphone: replays them and prints a table

The audio stays in spikes/vad/audio/, which git ignores, and leaves nothing else.
"""

import sys
import time
import wave
from pathlib import Path

import numpy as np

RATE = 16_000
SECONDS = 7
AUDIO = Path(__file__).parent / "audio"
SETTINGS_MS = [200, 300, 400, 500, 600, 800]
BASELINE_MS = 2000
# A chunk of 512 samples at 16 kHz: the VAD decides at most this late.
CHUNK_MS = 32
# The phrase counts as cut if detection ends this much before its real end.
CUT_TOLERANCE_MS = 250

PHRASES = [
    "Abrí Spotify",
    "Abrime la calculadora, por favor",
    "Poné el volumen al cincuenta",
    "Recordame en veinte minutos sacar la pizza del horno",
    "Cerrá Discord",
    "Bloqueá la compu",
    "Buscá quién ganó el mundial de dos mil veintidós",
    "Eh, abrí, a ver, el bloc de notas",
    "Subí el volumen, pero solo un poco",
    "Recordame, cuando sean las nueve, llamar a mamá",
    "Sí",
    "No",
    "Cancelar",
    "Open Spotify",
    "Set the volume to thirty percent",
    "Remind me in fifteen minutes to check the oven",
    "Close Chrome",
    "Search for the capital of Australia",
    "Well, um, turn the volume down a little",
    "Cancel",
]


def record() -> None:
    import sounddevice as sd

    AUDIO.mkdir(exist_ok=True)
    print(f"Micrófono: {sd.query_devices(kind='input')['name']}")
    print(f"{len(PHRASES)} frases. Hablá como le hablarías a Niva, con tus pausas normales.\n")
    for index, phrase in enumerate(PHRASES, 1):
        input(f"[{index}/{len(PHRASES)}] Enter y decí: «{phrase}» ")
        time.sleep(0.3)
        print(f"   ● grabando {SECONDS} s…")
        audio = sd.rec(SECONDS * RATE, samplerate=RATE, channels=1, dtype="int16")
        sd.wait()
        with wave.open(str(AUDIO / f"{index:02d}.wav"), "wb") as out:
            out.setnchannels(1)
            out.setsampwidth(2)
            out.setframerate(RATE)
            out.writeframes(audio.tobytes())
    print("\nListo. Ahora: uv run vad_spike.py sweep")


def load(path: Path) -> np.ndarray:
    with wave.open(str(path), "rb") as source:
        raw = source.readframes(source.getnframes())
    return np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0


def segments(audio: np.ndarray, silence_ms: int) -> list[dict]:
    from faster_whisper.vad import VadOptions, get_speech_timestamps

    options = VadOptions(min_silence_duration_ms=silence_ms, speech_pad_ms=0)
    return get_speech_timestamps(audio, options)


def sweep() -> None:
    files = sorted(AUDIO.glob("*.wav"))
    if not files:
        sys.exit("No hay audio: corré primero el modo record.")
    clips = [(f.stem, load(f)) for f in files]
    truth = {}
    for name, audio in clips:
        found = segments(audio, BASELINE_MS)
        truth[name] = (found[0]["start"], found[-1]["end"]) if found else None
    missing = [name for name, span in truth.items() if span is None]
    print(f"{len(clips)} frases, sin voz detectada: {missing or 'ninguna'}\n")
    print("| Espera | Demora estimada | Frases cortadas | Cuáles |")
    print("|---|---|---|---|")
    for silence in SETTINGS_MS:
        cut = []
        for name, audio in clips:
            if truth[name] is None:
                continue
            found = segments(audio, silence)
            end_of_first = found[0]["end"] if found else 0
            if truth[name][1] - end_of_first > CUT_TOLERANCE_MS * RATE / 1000:
                cut.append(name)
        print(f"| {silence} ms | {silence + CHUNK_MS} ms | {len(cut)}/{len(clips)} | {', '.join(cut) or '-'} |")


if __name__ == "__main__":
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    if mode == "record":
        record()
    elif mode == "sweep":
        sweep()
    else:
        sys.exit(__doc__)
