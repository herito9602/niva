"""Audio constants and named tunables of the sidecar.

Every number that is a guess or a measurement lives here with its origin, so it
can be tuned in one place.
"""

from __future__ import annotations

from dataclasses import dataclass

SAMPLE_RATE = 16_000
# One capture frame is one Silero chunk (512 samples, 32 ms). The wake word
# detector buffers these into its own 80 ms windows.
FRAME_SAMPLES = 512
FRAME_MS = FRAME_SAMPLES * 1000 // SAMPLE_RATE  # 32


@dataclass(frozen=True)
class Tunables:
    # Rolling in-memory audio kept before the speech onset (speech-io, "Audio
    # before speech onset"). INVENTED default (300 ms), to tune with the owner's
    # recordings; see voice/README.md for the start-margin comparison.
    pre_speech_ms: int = 300
    # Pre-roll kept from the wake detection, so a command said in one breath
    # keeps its first syllables. Same value and same caveat as above; it can
    # also carry the tail of the wake word itself.
    wake_preroll_ms: int = 300
    # Stock hey_jarvis placeholder threshold (spikes/voice/RESULTS.md: live
    # scores 0.61-0.85, 0 false activations in 10 idle minutes at 0.5).
    wake_threshold: float = 0.5
    # Wake scores from this value up to the threshold are logged as near misses.
    wake_near_miss_floor: float = 0.3
    # Silero thresholds as faster-whisper uses them (neg = threshold - 0.15).
    vad_threshold: float = 0.5
    vad_neg_threshold: float = 0.35
    # Owner decision (spikes/vad/RESULTS.md): wait 500 ms of silence to end speech.
    end_silence_ms: int = 500
    # Consecutive speech time needed to confirm an onset (filters a single click).
    onset_ms: int = 64
    # Silence kept after the last speech frame in the audio sent to STT.
    trailing_pad_ms: int = 160
    # Listen with no speech at all ends with `no_speech` (matches the host's 6 s).
    no_speech_s: float = 6.0
    # A wake detection the host never claims with a `listen` is dropped after this.
    wake_claim_s: float = 3.0
    # Backstop only: the host forces the end at 15 s (`force_end`).
    max_utterance_s: float = 30.0
    # No frame from the microphone for this long is a failed stream.
    mic_stall_s: float = 2.0
    # While the microphone is down, try to reopen it this often.
    mic_retry_s: float = 5.0
    # edge-tts must deliver its first chunk within this, else Piper (design D7).
    tts_first_chunk_s: float = 2.0
    # Peak amplitude (int16) above which a chunk counts as non-silent.
    silent_peak: int = 24

    @property
    def pre_speech_frames(self) -> int:
        return max(0, -(-self.pre_speech_ms // FRAME_MS))

    @property
    def wake_preroll_frames(self) -> int:
        return max(0, -(-self.wake_preroll_ms // FRAME_MS))

    @property
    def trailing_pad_frames(self) -> int:
        return max(0, -(-self.trailing_pad_ms // FRAME_MS))

    @property
    def onset_frames(self) -> int:
        return max(1, -(-self.onset_ms // FRAME_MS))
