# Spike: VAD end-of-speech wait (2026-11-05)

Silero VAD (the model bundled with faster-whisper), 20 phrases in Spanish and
English recorded by the owner on the laptop microphone, replayed offline. The
audio stays local (`spikes/vad/audio/`, ignored by git).

| Wait | Estimated delay | Sweep count | Real cuts after inspection |
|---|---|---|---|
| 200 ms | 232 ms | 6/20 | 4/20 (04, 08, 15, 19) |
| 300 ms | 332 ms | 5/20 | 3/20 (04, 15, 19) |
| 400 ms | 432 ms | 4/20 | 2/20 (04, 15) |
| 500 ms | 532 ms | 3/20 | 1/20 (15) |
| 600 ms | 632 ms | 3/20 | 1/20 (15) |
| 800 ms | 832 ms | 3/20 | 1/20 (15) |

The sweep count overstates the cuts: phrases 01 and 07 were complete, and a later
noise in the 7 s window made them look cut. `inspect_segments.py` shows the spans.
Phrase 15 has a 0.92 s pause inside it: no setting up to 800 ms holds it.

The delay is the wait plus one 32 ms VAD chunk. It is computed, not measured live,
and it comes from one speaker, one microphone and one room.

## Other findings
- `device="cuda"` failed here with `cublas64_12.dll is not found`. The sidecar must
  solve how it finds the CUDA libraries, and fall back truthfully.
- The transcripts above came from CPU int8 with beam 1, not the product setup, so
  they do not measure STT quality. Phrase 04 came out badly and should be retried
  on the GPU with these same recordings.
