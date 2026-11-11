# Niva speech sidecar

Wake word, VAD, STT and TTS in one process, talking to the Rust host in JSON lines
over stdio. The protocol is documented in `protocol.py`; the design is in
`openspec/changes/niva-mvp/design.md` ("Sidecar protocol", "Sidecar capture loop").

| Module | Job |
|---|---|
| `protocol.py` | message shapes, parsing, the error taxonomy |
| `capture.py` | the capture loop: one stream, rolling pre-speech buffer, wake/listen/speak rules |
| `wake.py`, `vad.py`, `stt.py`, `tts.py` | openWakeWord, Silero, faster-whisper, edge-tts + Piper |
| `audio_io.py` | the only code that touches `sounddevice` |
| `discards.py` | discard reason codes (no audio, no text) |
| `config.py` | every tunable, with its origin |
| `margin.py` | start-margin / endpointer check on recorded WAVs |
| `devhost.py` | `--dev-host`: a stand-in host for trying the sidecar alone |

## Setup

Online, once: `uv sync` (needs the dev group for the tests: `uv sync --group dev`).

Offline (everything already in the uv cache): create `.venv` once and call tools with
`--no-project`; `uv run --offline` without it cannot lock the project offline.

```powershell
cd voice
uv venv --python 3.12
uv pip install --offline "numpy<2" onnxruntime sounddevice "openwakeword==0.6.0" faster-whisper `
    nvidia-cublas-cu12 nvidia-cudnn-cu12 edge-tts "miniaudio==1.71" piper-tts
```

## Tests (no microphone, GPU or network)

```powershell
uv run --offline --no-project pytest
uv run --offline --no-project ruff check .
```

Four tests use local-only files and skip themselves when missing: the owner's
`spikes/vad/audio` recordings and an edge-tts sample in `spikes/voice/s1_audio`.

## Models (paths come from configuration, nothing is downloaded)

| What | Setting | Default |
|---|---|---|
| openWakeWord `hey_jarvis_v0.1.onnx`, `melspectrogram.onnx`, `embedding_model.onnx` | `NIVA_WAKE_MODEL_DIR` or `--wake-model-dir` | the `openwakeword` package folder |
| Whisper `small` | `NIVA_WHISPER_MODEL` or `--whisper-model` (name or path) | `small`, from the Hugging Face cache, never downloaded unless `--allow-download` |
| Piper voices `es_AR-daniela-high.onnx`, `en_US-lessac-medium.onnx` | `NIVA_PIPER_DIR` or `--piper-dir` | none (no offline fallback) |
| Silero VAD | bundled with faster-whisper | |

## Short answers and hallucinations (2026-11-07)

Whisper gets one-word answers wrong ("Si" detected as English and written "C", "Cancel"
as "Council."), and invents text over silence or over the tail of Niva's own voice
("Thank you."). Two measures, both in `stt.py`, tunables in `config.py`:

- **Confirm listens know their question.** The host sends `lang` and `context` (the
  text of the confirmation it just spoke) with `listen{confirm}`. The sidecar forces
  that language, gives the question as `initial_prompt`, limits the answer to
  `confirm_max_new_tokens` (12) and turns timestamps off. The prompt is the question,
  never answer words ("Yes. No. Okay." turned an English tail into "Okay."). A listen
  without them (or a command) detects the language as before. `context` is never logged.
- **No-speech gate, every mode.** A segment with `no_speech_prob` above
  `no_speech_prob_max` (0.5) is dropped; if nothing remains the transcript is empty
  and the host reissues the confirm listen or shows the empty-transcript error. The
  output length is bounded too (`command_*_new_tokens` by clip length).

Offline check on the owner's recordings (`spikes/vad/audio` 11-13, 20; Whisper small,
fp16): old settings "C" / "No" / "cancelar" / "Council."; new settings "Si." / "No." /
"Cancelar" / "Cancel", and every 0.32 s tail empty.

## Run it with the Niva host (task 3.4)

The Rust host starts, feeds and supervises this process; there is nothing to launch by
hand. It speaks the protocol of `protocol.py` over stdio, logs the sidecar's stderr with
the prefix `[niva-voice:py]`, restarts a crash up to 3 times (the counter resets after 5
stable minutes), restarts once with the next smaller Whisper model after `CUDA_OOM`
(`small`, then `base`), and falls back to text mode when it gives up. The tray entry
"Restart voice" resets all of that and starts again.

Closing the sidecar's stdin ends it (checked through `uv`: 0.4 s); the host sends
`shutdown`, closes stdin, waits 3 s and only then kills the whole process tree. Every
sidecar also sits in a Windows job object that dies with the app, so it cannot outlive
Niva and keep the microphone.

How the host finds things (all optional in a dev checkout):

| Variable | Meaning | Default |
|---|---|---|
| `NIVA_VOICE` | `off` (also `0`, `false`, `no`) keeps Niva in text mode | on |
| `NIVA_VOICE_DIR` | folder with `main.py` | `voice\` next to the app; in a debug build, this repository's `voice\` |
| `NIVA_VOICE_PYTHON` | a Python executable, run as `python main.py ...` (for example `voice\.venv\Scripts\python.exe`, which avoids the `uv` launcher) | `uv run --offline --no-project python main.py ...` |
| `NIVA_WAKE_MODEL_DIR` | openWakeWord `.onnx` folder | the package folder, which has none: set it |
| `NIVA_PIPER_DIR` | Piper voices | none (no offline voice) |
| `NIVA_WHISPER_MODEL` | first Whisper model of the ladder (name or path) | `small`, then `base` |
| `NIVA_MIC_DEVICE` | microphone index or name (`--mic-device`; one value, so a name with spaces works) | the system default |
| `NIVA_VOICE_ARGS` | extra options for `main.py`, split on spaces (`--no-edge`, `--allow-cpu`, `--verbose`) | none |
| `NIVA_TTS_LEVEL_OFFSET_MS` | milliseconds added to the moment the notch waveform reacts to Niva's voice (negative = earlier); for latency the audio device does not report, such as Bluetooth. Values beyond +-500 or unreadable ones are ignored | `0` |

Try Niva by voice (this opens the microphone and plays sound; `uv` must be on `PATH` and
`voice\.venv` must exist, see Setup):

```powershell
cd D:\trabajo\ProyectoAsistente\app
$env:NIVA_WAKE_MODEL_DIR = "$env:LOCALAPPDATA\uv\cache\environments-v2\spike-voice-32308f98e9d78df1\Lib\site-packages\openwakeword\resources\models"
$env:NIVA_PIPER_DIR = "D:\trabajo\ProyectoAsistente\spikes\voice\models"
npm run tauri dev
```

Wait for the tray tooltip to say "Niva: voice ready" (about 20 s for the models, and up to
30 s more if Ollama is still warming up the LLM), then say "Hey Jarvis, open Spotify" or
press Ctrl+Shift+Space.

## Try it with the real microphone

This opens the microphone and plays sound. It acts as a tiny host (`--dev-host`): say
"Hey Jarvis, open Spotify", it prints the events and speaks back what it heard.

```powershell
cd voice
$env:NIVA_WAKE_MODEL_DIR = "$env:LOCALAPPDATA\uv\cache\environments-v2\spike-voice-32308f98e9d78df1\Lib\site-packages\openwakeword\resources\models"
uv run --offline --no-project python main.py --dev-host --piper-dir ..\spikes\voice\models --verbose
```

Without `--dev-host` it waits for protocol lines on stdin; type, for example,
`{"type":"speak","id":1,"text":"Hola, soy Niva.","lang":"es"}` or
`{"type":"listen","id":2,"mode":"command"}` (Ctrl+C or `{"type":"shutdown"}` to stop).
`--no-edge` speaks with Piper only (nothing leaves the machine).

Voices: edge-tts speaks `es-CU-BelkysNeural` (Spanish) and `en-US-AriaNeural` (English); the
Piper fallback keeps `es_AR-daniela-high` and `en_US-lessac-medium`. `--robot-voice` sends every
reply, from either engine, through a streaming robot effect (`voice_fx.py`: pitch up 1.2x, a
280 Hz robot buzz mixed with the voice, light ring modulation and comb filter). It is
chunk-by-chunk, adds about 20 ms and makes speech 20 % faster. It is off by default.

### Latency switches (task 3.5, branch `perf/3f-voice-latency`)

Five optimizations, each on by default and each with its own switch, so any one can be turned
off if it misbehaves (add the option to `NIVA_VOICE_ARGS`). Savings are measured locally on
2026-11-09, not yet over a full session; the real-microphone measurement is still the owner's.

| Switch (off) | What it does | Measured |
|---|---|---|
| `--no-preopen-output` | Opens the sound device while the reply is still being synthesized, not at the first audio. | ~220 ms (MME open) |
| `--no-single-pass-stt` | Runs the Whisper encoder once and uses it both to choose es/en and to decode (faster-whisper 1.2.1 internals, isolated in `stt.single_pass_decode`; falls back to the public API and logs once if they change). | ~115-190 ms |
| `--no-persistent-edge` | Keeps one edge-tts websocket open (`edge_ws.py`), connects it when a listen starts and when a transcript is about to go out, replaces it after 30 s idle, retries once on a fresh connection (a reused socket gets 1.5 s to answer; healthy first frames measured 0.13 to 2.1 s), then falls back to the one-connection-per-reply path with the full 2 s first-chunk budget, then Piper (preloaded in the background at startup). | ~310 ms |
| `--no-tts-cache` | Caches the raw MP3 of short replies (120 characters or fewer) and plays repeats without the network (`tts_cache.py`). | a hit skips the network round trip: ~140 ms mean audible saving (the first PCM chunk is ready in ~10 ms, but the sound still waits for the output device) |
| `--no-speculative-stt` | Starts Whisper in the background after 250 ms of the 500 ms end-of-speech silence (`--speculative-stt-ms`); the result is used only if the end is confirmed with the same audio, and nothing is sent before the end. | ~270 ms (synthetic clips: 903 to 629 ms) |

Connection warm-up and privacy: with the persistent connection, the sidecar opens the TTS service
connection (a websocket to Microsoft's edge-tts endpoint) as soon as a wake word is heard or a
listen starts, so the voice is ready when the reply comes. That happens even if no reply follows
(a wake word that leads nowhere, a cancelled listen, an empty transcript); no text is sent over it
until Niva speaks. `--no-persistent-edge` avoids the early connection (a connection is then opened
only when a reply is spoken), and `--no-edge` never contacts the service at all.

Speculative Whisper and mid-sentence pauses: when the user pauses and then keeps talking, the job
started during the pause is thrown away and the whole utterance is transcribed at the real end. The
cost was measured (real Silero + Whisper `small` on CUDA, 8 utterances per cell, 2026-11-09, with
Niva running on the same GPU, so absolute numbers are noisy). Median / p95 ms from the end of
speech to the transcript:

| Case | off | 250 ms (default) | 350 ms |
|---|---|---|---|
| No pause | 968 / 4174 (outlier) | 649 / 909 | 789 / 1055 |
| 300 ms pause, then speech | 1110 / 1402 | 700 / 1036 | 835 / 1118 |
| 450 ms pause, then speech | 1099 / 1317 | 672 / 1078 | 908 / 1174 |
| 380 ms pause, then 0.4 s of speech | 992 / 1190 | 553 / 873 | 759 / 1054 |

The abandoned job costs nothing measurable: it runs while the user is still talking, and the real
end waits for the GPU only if that job is somehow still running (it is not, by then). A higher
threshold only gives up savings (350 ms is ~140 ms slower than 250 ms in every case), so 250 ms
stays the default. `--no-speculative-stt` remains the switch if the extra Whisper runs during pauses
are unwanted (they use the GPU while the user talks).

Reply cache: `%LOCALAPPDATA%\Niva\tts-cache` (`NIVA_TTS_CACHE_DIR` or `--tts-cache-dir` change
it), at most 50 MB, oldest files evicted first. It holds Niva's own replies only, never the
user's speech, as MP3 audio in files named by a hash of the text. A reply can still say what was
asked ("Abrí Calculadora."), so delete the folder or use `--no-tts-cache` if that matters. A
damaged file is deleted and the reply is synthesized again.

Start margin and endpointer check on the recorded phrases (no microphone):

```powershell
uv run --offline --no-project python margin.py ..\spikes\vad\audio
```

## Waveform levels (living-pc-notch, slice 5)

While Niva speaks, the sidecar sends `{"type":"level","src":"tts","bands":[7 integers 0-100]}`
about 31 times a second (one per 32 ms of audio). Only Niva's own voice is analysed; there is no
microphone level, ever. `levels.py` computes the bands (Hann window, FFT, 7 log bands between
80 Hz and 8 kHz, -60..-10 dBFS mapped to 0-100).

The levels are paced to the moment the sound is audible, not to the moment it is written:
the output device holds `stream.latency` seconds of audio ahead of the loudspeaker, so
`level_pacer.py` keeps a model of the playback clock (the first write anchors it, a stalled
network re-anchors it) and emits each slice when it is heard. The pacer stops with the
speech and at once on a barge-in. `python tools/output_latency.py` prints the latency and the
write timing of the default output device.

If the bars still move before or after the sound on a device that under- or over-reports its
latency (typically Bluetooth), set `NIVA_TTS_LEVEL_OFFSET_MS`: a negative number makes the bars
earlier, a positive one later. Film the debug window's "level test" at 240 fps to measure it: the
rise of the bars should follow the sound by 50 ms on average and never by more than 100 ms.
