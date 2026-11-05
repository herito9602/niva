# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = ["onnxruntime", "numpy<2", "faster-whisper"]
# ///
"""Prints the speech segments and the transcript of each recording, to see why a
phrase counts as cut in the sweep."""

import sys

from faster_whisper import WhisperModel

from vad_spike import AUDIO, BASELINE_MS, PHRASES, RATE, load, segments


def spans(found: list[dict]) -> str:
    return " ".join(f"{s['start'] / RATE:.2f}-{s['end'] / RATE:.2f}" for s in found) or "-"


model = WhisperModel("small", device="cpu", compute_type="int8")
names = sys.argv[1:] or [f.stem for f in sorted(AUDIO.glob("*.wav"))]
for name in names:
    audio = load(AUDIO / f"{name}.wav")
    text = " ".join(s.text.strip() for s in model.transcribe(audio, beam_size=1)[0])
    print(f"{name} «{PHRASES[int(name) - 1]}»\n   heard: {text}")
    for silence in (300, 500, 800, BASELINE_MS):
        print(f"   {silence:>4} ms: {spans(segments(audio, silence))}")
