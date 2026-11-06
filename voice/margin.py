"""Compare the live VAD and the start margin on recorded phrases (no microphone).

    uv run --offline --no-project python margin.py ../spikes/vad/audio

For every WAV (16 kHz mono, the owner's ``spikes/vad/audio`` recordings) it
runs the streaming Silero path the sidecar uses and reports:

* ``onset``   first frame the VAD calls speech at the real threshold (0.5);
* ``soft``    first frame of three in a row above a low threshold (0.3), a proxy
              for a soft first syllable that the real threshold misses;
* ``need``    ``onset - soft``: how much audio before the confirmed speech the
              buffer must keep so that syllable is not cut;
* ``end``     where the live endpointer ends the first utterance, next to the
              offline reference of the VAD spike (500 ms wait).

The audio never leaves this process and nothing is written to disk.
"""

from __future__ import annotations

import dataclasses
import sys
import wave
from dataclasses import dataclass
from pathlib import Path

import numpy as np

from config import FRAME_MS, FRAME_SAMPLES, SAMPLE_RATE, Tunables
from vad import End, Endpointer, Onset, SileroStream


@dataclass(frozen=True)
class Margin:
    name: str
    onset_ms: int | None  # first speech frame at the real threshold
    soft_ms: int | None  # first speech frame at a very low threshold
    confirmed_ms: int | None  # when the live VAD confirms the onset
    end_ms: int | None  # end of the first utterance, live endpointer
    offline_start_ms: int | None
    offline_end_ms: int | None

    @property
    def need_ms(self) -> int | None:
        if self.onset_ms is None or self.soft_ms is None:
            return None
        return self.onset_ms - self.soft_ms


def load_wav(path: Path) -> np.ndarray:
    with wave.open(str(path), "rb") as source:
        if source.getframerate() != SAMPLE_RATE or source.getnchannels() != 1:
            raise ValueError(f"{path.name}: expected 16 kHz mono")
        raw = source.readframes(source.getnframes())
    return np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0


def probabilities(audio: np.ndarray, vad: SileroStream) -> list[float]:
    vad.reset()
    usable = len(audio) // FRAME_SAMPLES * FRAME_SAMPLES
    return [float(vad(audio[i : i + FRAME_SAMPLES])) for i in range(0, usable, FRAME_SAMPLES)]


def run_endpointer(probs: list[float], tunables: Tunables) -> tuple[int | None, int | None, int | None]:
    """``(first speech frame, confirmation frame, end-of-speech frame)`` as ms."""
    endpointer = Endpointer(tunables)
    first = confirmed = end = None
    for i, prob in enumerate(probs):
        signal = endpointer.push(prob)
        if isinstance(signal, Onset) and first is None:
            first = (i - signal.frames_ago) * FRAME_MS
            confirmed = (i + 1) * FRAME_MS
        elif isinstance(signal, End):
            end = (i - signal.silence_frames_ago) * FRAME_MS
            break
    return first, confirmed, end


def offline_reference(audio: np.ndarray, silence_ms: int) -> tuple[int | None, int | None]:
    from faster_whisper.vad import VadOptions, get_speech_timestamps

    found = get_speech_timestamps(audio, VadOptions(min_silence_duration_ms=silence_ms, speech_pad_ms=0))
    if not found:
        return None, None
    return found[0]["start"] * 1000 // SAMPLE_RATE, found[0]["end"] * 1000 // SAMPLE_RATE


def measure(paths: list[Path], tunables: Tunables | None = None) -> list[Margin]:
    tunables = tunables or Tunables()
    soft = dataclasses.replace(tunables, vad_threshold=0.3, vad_neg_threshold=0.2, onset_ms=3 * FRAME_MS)
    vad = SileroStream()
    rows = []
    for path in paths:
        audio = load_wav(path)
        probs = probabilities(audio, vad)
        onset, confirmed, end = run_endpointer(probs, tunables)
        soft_onset, _, _ = run_endpointer(probs, soft)
        off_start, off_end = offline_reference(audio, tunables.end_silence_ms)
        rows.append(Margin(path.stem, onset, soft_onset, confirmed, end, off_start, off_end))
    return rows


def report(rows: list[Margin], tunables: Tunables) -> str:
    def cell(value: int | None) -> str:
        return "-" if value is None else str(value)

    lines = [
        f"pre_speech_ms={tunables.pre_speech_ms} end_silence_ms={tunables.end_silence_ms}",
        "| file | onset | soft | need | confirmed | end | offline start | offline end |",
        "|---|---|---|---|---|---|---|---|",
    ]
    for r in rows:
        lines.append(
            f"| {r.name} | {cell(r.onset_ms)} | {cell(r.soft_ms)} | {cell(r.need_ms)} | {cell(r.confirmed_ms)} "
            f"| {cell(r.end_ms)} | {cell(r.offline_start_ms)} | {cell(r.offline_end_ms)} |"
        )
    needs = sorted(r.need_ms for r in rows if r.need_ms is not None)
    if needs:
        lines.append(
            f"need ms: median {needs[len(needs) // 2]}, max {needs[-1]}; "
            f"covered by the {tunables.pre_speech_ms} ms buffer: "
            f"{sum(n <= tunables.pre_speech_ms for n in needs)}/{len(needs)}"
        )
    return "\n".join(lines)


def main(argv: list[str]) -> int:
    folder = Path(argv[1]) if len(argv) > 1 else Path("../spikes/vad/audio")
    paths = sorted(folder.glob("*.wav"))
    if not paths:
        print(f"no WAV files in {folder}", file=sys.stderr)
        return 1
    tunables = Tunables()
    print(report(measure(paths, tunables), tunables))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
