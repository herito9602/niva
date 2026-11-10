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

Voices: edge-tts speaks `es-US-PalomaNeural` (Spanish) and `en-US-AriaNeural` (English); the
Piper fallback keeps `es_AR-daniela-high` and `en_US-lessac-medium`. Every reply, from either
engine, goes through a streaming robot effect (`voice_fx.py`: pitch up 1.2x, a 280 Hz robot
buzz mixed with the voice, light ring modulation and comb filter). It is chunk-by-chunk, adds
about 20 ms and makes speech 20 % faster. `--no-robot-voice` plays the plain voices.

Start margin and endpointer check on the recorded phrases (no microphone):

```powershell
uv run --offline --no-project python margin.py ..\spikes\vad\audio
```
