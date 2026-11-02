# Voice spike results (2026-11-01)

Setup: openWakeWord `hey_jarvis` (placeholder), faster-whisper on CPU (int8), edge-tts `es-AR-ElenaNeural`, energy-based endpointing.

| Check | Result |
|---|---|
| Wake word, live (7 tries) | 7/7 detected, scores 0.61-0.85 (threshold 0.5) |
| False activations, 10 min idle | 0 |
| Endpointing | 2 commands missed, 2 cut off mid-phrase |
| End-of-speech -> reply latency | avg 3.75 s, max 4.11 s (target < 2 s) |
| Whisper `small` vs `base` (4.2 s audio) | 1.98 s accurate vs 0.61 s with errors |

## Decisions
- Wake word stays local (openWakeWord). Train a custom "hey niva" model.
- STT moves to a cloud streaming API; local CPU Whisper is too slow on typical PCs.
- Replace energy endpointing with a real VAD (Silero).
- Stream TTS playback from the first audio chunk.

## S2 TTS comparison (2026-11-02, `compare_tts.py`)

| Voice | First audio | Total |
|---|---|---|
| edge-tts es-AR-ElenaNeural | 0.98 s | 1.30 s |
| Piper es_AR-daniela-high (CPU, first run) | — | 1.10 s |
| edge-tts en-US-AriaNeural | 0.89 s | 1.55 s |
| Piper en_US-lessac-medium (CPU) | — | 0.18 s |

Owner listened to both and chose **edge-tts as the default voice**; Piper stays the automatic fallback (offline, errors).
Note: edge-tts first-audio (~0.9 s) is above the 400 ms TTS slot in the latency budget.
