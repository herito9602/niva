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
    # Speech heard this long after a wake detection is provisional, like the
    # replayed pre-roll: the detector fires before the word ends, so "...vis" is
    # still being said after it. If the user pauses (to watch the island) the tail
    # alone must not end the turn; a one-breath command keeps talking past the
    # window and so counts. MEASURED on live runs (2026-11-07): tails of 200-400
    # ms. A command that ends entirely inside this window would be mistaken for
    # the tail; real commands are longer than the tail plus this.
    wake_tail_ms: int = 400
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
    # Hallucination gate: a Whisper segment whose no_speech_prob is ABOVE this is
    # dropped, in every mode. Measured on the owner's recordings (2026-11-07,
    # Whisper small, fp16): the 0.32 s tail of Niva's own voice came out as "Thank
    # you." with no_speech_prob 0.59-0.60; real words had 0.02-0.14. 0.5 sits in
    # the gap with margin on both sides. Tune it with real use.
    no_speech_prob_max: float = 0.5
    # Confirm answers are one or two words ("Si.", "Cancelar"): 12 tokens is
    # enough and keeps a 1 s clip from returning hundreds of characters (446 chars
    # were seen in the live run).
    confirm_max_new_tokens: int = 12
    # Commands: the bound grows with the clip (tokens per second of audio, a
    # generous speaking rate), between a floor and the hard cap below. INVENTED
    # numbers, loose on purpose: they only stop runaway text, not real commands.
    command_tokens_per_s: float = 10.0
    command_min_new_tokens: int = 24
    command_max_new_tokens: int = 224
    # Niva's voice is a cute little robot (owner's choice, 2026-11-09): every
    # synthesized reply goes through the effect in voice_fx.py. False speaks the
    # plain voices (`--no-robot-voice`).
    robot_voice: bool = True
    # Open the output device while the reply is still being synthesized, so the
    # first audio does not wait for the device (MME open measured at ~294 ms per
    # utterance, 2026-11-09). False opens it at the first audio, as before
    # (`--no-preopen-output`).
    preopen_output: bool = True

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
