# S1 results: faster-whisper on GPU together with the product LLM

Date: 2026-11-02. Hardware: RTX 3060 Laptop, 6144 MiB VRAM, driver 610.62. Script: `spikes/voice/s1_gpu_stt.py`
(`uv run spikes/voice/s1_gpu_stt.py --model <small|large-v3-turbo> --mode <solo|co>`). Raw data: `spikes/voice/s1_raw_*.json`.

**CAVEAT: all audio is SYNTHETIC** (edge-tts: es-AR-Elena/Tomas, en-US-Aria/Guy, 20 clips, 53 s total, decoded to 16 kHz mono).
Clean TTS audio is the easy case. Real mic audio (noise, accents, far field, clipped starts) is still a gap; the latency numbers
are unaffected (Whisper pads to a 30 s window) but language-ID and WER numbers are optimistic.

## Setup notes
- Deps resolved cleanly: `nvidia-cublas-cu12` + `nvidia-cudnn-cu12` (cuBLAS/cuDNN 12 wheels) work with the installed ctranslate2. No version mismatch.
- DLL folders (inside the uv script env's site-packages): `nvidia/cublas/bin`, `nvidia/cudnn/bin`, `nvidia/cuda_nvrtc/bin`. The script adds all `nvidia/*/bin` via `os.add_dll_directory` and PATH.
- Language ID: `WhisperModel.detect_language(audio)` -> `all_language_probs`, pick the higher of `es` vs `en`, then `transcribe(language=...)`, `beam_size=1`, no VAD. Latency below = detection + transcription (two encoder passes).
- Compute type: `float16` loaded fine for both models (no int8_float16 fallback needed).
- Models downloaded: `small`, `large-v3-turbo` (mobiuslabsgmbh CT2 conversion), into the HF cache.

## Results (20 clips, after 3 warm-up passes)

| Model | Mode | STT mean | p50 | p95 | max | Lang-ID | Detect pass (mean) | Whisper VRAM | Mean word-hit |
|---|---|---|---|---|---|---|---|---|---|
| small (fp16) | solo | 183 ms | 179 | 203 | 207 | 20/20 | 65 ms | ~755 MiB | 0.97 |
| small (fp16) | co-resident with LLM | 177 ms | 176 | 190 | 204 | 20/20 | 64 ms | ~755 MiB (4522 total) | 0.97 |
| large-v3-turbo (fp16) | solo | 563 ms | 562 | 573 | 581 | 20/20 | 267 ms | ~2259 MiB | 0.99 |
| large-v3-turbo (fp16) | co-resident with LLM | 664 ms | 668 | 710 | 750 | 20/20 | 316 ms | ~2085 MiB (5852 total) | 0.99 |

Lowest es/en winning probability: small 0.78 (es07 "Apaga la PC"), large-v3-turbo 0.97. Every clip was classified correctly by both models.
Transcripts: small has 2 minor errors ("bloc" -> "blog", "No apagues" -> "No apáis la PC"; the latter would be a safety-relevant miss, see below);
large-v3-turbo only drops voseo accents ("Apagá" -> "Apaga") and writes "30%". Full per-clip transcripts are in the raw JSON (`clips.rows[].text` vs `ref`).

## Co-residency (qwen3.5:4b-q4_K_M, 3.1 GB, Ollama, num_ctx 4096)

| Stage | VRAM used / 6144 MiB | `ollama ps` PROCESSOR |
|---|---|---|
| Empty GPU | 0 MiB | - |
| LLM only (warm, keep_alive 30m) | 3767 MiB | 100% GPU |
| + small loaded | 4522 MiB | 100% GPU |
| + small, after E1 eval | 4536 MiB | 100% GPU |
| + large-v3-turbo loaded | 5852 MiB | 100% GPU |
| + large-v3-turbo, after E1 eval | 5910 MiB | 100% GPU |

Neither combination makes Ollama spill layers to CPU at `ollama ps` level. large-v3-turbo leaves only ~230-290 MiB headroom, which is
too tight for the 2 other things in the plan (TTS/other GPU use, desktop compositor, a longer context); the WDDM driver can silently page
GPU memory to system RAM when near the limit, which is consistent with the slowdowns below.

### LLM latency (E1, `run_e1.py`, 51 cases) with Whisper resident

| Condition | tool % | mean | p50 | p95 | p95 action path |
|---|---|---|---|---|---|
| Solo (reference, from earlier run) | 96.1 | 776 ms | - | 1232 ms | - |
| + small resident | 94.1 | 760 ms | 707 | 1110 | 1110 |
| + large-v3-turbo resident | 96.1 | 1122 ms | 980 | 1764 | 1748 |

Whisper `small` residency costs the LLM nothing measurable (differences are within run-to-run noise; the 94.1 vs 96.1 tool % is one
case of a non-deterministic adversarial item). `large-v3-turbo` residency slows the LLM by roughly +45% mean / +43% p95, and Whisper
itself slows from 563 -> 664 ms (and to 956 ms mean when re-run right after the E1 load): the two compete for a nearly full GPU.
Note the solo LLM p95 (1.1-1.2 s) already misses the E1 700 ms action-path gate, independent of Whisper.

## Gates

| Gate | small | large-v3-turbo |
|---|---|---|
| STT p95 <= 400 ms (incl. es/en detection) | PASS (190-203 ms) | FAIL (573 ms solo, 710 ms co-resident) |
| Lang-ID >= 19/20 (synthetic) | PASS 20/20 | PASS 20/20 |
| Fits with LLM, no CPU spill | PASS (4.5 GB of 6) | PASS technically (5.9 GB of 6) but ~0 headroom and LLM +45% slower |

## Interpretation and caveats
- `small` is the viable S1 configuration: ~180-200 ms, 20/20, ~1.5 GB headroom, no LLM penalty.
- `large-v3-turbo` fails the 400 ms gate even solo. About 270-320 ms of that is the separate language-detection pass (a second full encoder run); a single-pass design (detect + decode off one encoder output, or detect only on a shortened prefix) was not tried and could bring it down, but it would still be tight and still leaves no VRAM headroom.
- `small` mis-heard "No apagues la PC" as "No apáis la PC" (negation about a risky power action). On real audio this matters; keep SafetyGate/confirmation as designed and consider this a data point for the accuracy question on real mic input.
- Not measured: real microphone audio, noisy rooms, 1-2 word clips with silence padding, VAD/endpointing latency, concurrent STT while the LLM is generating (clips ran sequentially, not overlapped with generation), TTS on GPU, Whisper int8 variants.
- The first run spent ~23 min downloading CUDA wheels (about 1.3 GB, uv cache); runs after that start in seconds.
