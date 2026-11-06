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

Start margin and endpointer check on the recorded phrases (no microphone):

```powershell
uv run --offline --no-project python margin.py ..\spikes\vad\audio
```
