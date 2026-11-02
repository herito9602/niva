# /// script
# requires-python = ">=3.11,<3.13"
# dependencies = [
#   "openwakeword==0.6.0",
#   "onnxruntime",
#   "sounddevice",
#   "numpy<2",
#   "faster-whisper",
#   "edge-tts",
#   "miniaudio",
# ]
# ///
"""Throwaway spike: validate the voice loop for Niva.

Pipeline: wake word -> record command until silence -> STT -> TTS reply.
Measures false activations and end-of-speech -> reply latency.

The wake word uses openWakeWord's pre-trained "hey_jarvis" model as a
placeholder; a custom "hey niva" model is trained later.

Usage:
  uv run spike_voice.py --selftest       # no microphone, synthetic audio
  uv run spike_voice.py                  # live microphone loop
  uv run spike_voice.py --idle 600       # count false activations for 10 min
"""

import argparse
import asyncio
import time

import edge_tts
import miniaudio
import numpy as np
import sounddevice as sd

SAMPLE_RATE = 16000
FRAME = 1280  # 80 ms, what openWakeWord expects
WAKE_MODEL = "hey_jarvis"
REPLY_VOICE = "es-AR-ElenaNeural"


def load_wake_model():
    import openwakeword
    from openwakeword.model import Model

    openwakeword.utils.download_models(model_names=[WAKE_MODEL])
    return Model(wakeword_models=[WAKE_MODEL], inference_framework="onnx")


def load_stt(name):
    from faster_whisper import WhisperModel

    return WhisperModel(name, device="cpu", compute_type="int8")


def tts_bytes(text, voice=REPLY_VOICE):
    async def run():
        data = b""
        async for chunk in edge_tts.Communicate(text, voice).stream():
            if chunk["type"] == "audio":
                data += chunk["data"]
        return data

    return asyncio.run(run())


def decode_mp3(data, sample_rate=SAMPLE_RATE):
    decoded = miniaudio.decode(
        data,
        output_format=miniaudio.SampleFormat.SIGNED16,
        nchannels=1,
        sample_rate=sample_rate,
    )
    return np.frombuffer(decoded.samples, dtype=np.int16)


def wake_score(model, frame):
    return max(model.predict(frame).values())


def transcribe(stt, audio_int16):
    audio = audio_int16.astype(np.float32) / 32768.0
    segments, _ = stt.transcribe(audio, language="es", beam_size=1, vad_filter=True)
    return " ".join(s.text.strip() for s in segments)


def speak(text):
    t0 = time.perf_counter()
    audio = decode_mp3(tts_bytes(text), sample_rate=24000)
    synth = time.perf_counter() - t0
    sd.play(audio, 24000)
    sd.wait()
    return synth


def record_command(stream, noise_rms, max_s=8.0, start_timeout_s=4.0, silence_s=0.8):
    """Record until the user stops talking. Returns (audio, end_of_speech_time)."""
    threshold = max(noise_rms * 3, 300)
    frames, started, silent, waited = [], False, 0.0, 0.0
    end_of_speech = time.perf_counter()
    frame_s = FRAME / SAMPLE_RATE
    while len(frames) * frame_s < max_s:
        frame, _ = stream.read(FRAME)
        frame = frame[:, 0]
        frames.append(frame)
        rms = np.sqrt(np.mean(frame.astype(np.float32) ** 2))
        if rms > threshold:
            started, silent = True, 0.0
            end_of_speech = time.perf_counter()
        elif started:
            silent += frame_s
            if silent >= silence_s:
                break
        else:
            waited += frame_s
            if waited >= start_timeout_s:
                return None, None
    return np.concatenate(frames), end_of_speech


def calibrate_noise(stream, seconds=1.5):
    frames = [stream.read(FRAME)[0][:, 0] for _ in range(int(seconds * SAMPLE_RATE / FRAME))]
    audio = np.concatenate(frames).astype(np.float32)
    return float(np.sqrt(np.mean(audio**2)))


def live(args):
    print("Loading models...")
    wake = load_wake_model()
    stt = load_stt(args.stt_model)
    activations, latencies = 0, []
    started_at = time.perf_counter()

    with sd.InputStream(samplerate=SAMPLE_RATE, channels=1, dtype="int16", blocksize=FRAME) as stream:
        print("Calibrating ambient noise, stay quiet...")
        noise = calibrate_noise(stream)
        print(f"Noise RMS: {noise:.0f}")
        mode = "IDLE (do not say the wake word)" if args.idle else "say 'Hey Jarvis' + a command"
        print(f"Listening - {mode}. Ctrl+C to stop.\n")
        try:
            while True:
                if args.idle and time.perf_counter() - started_at >= args.idle:
                    break
                frame, _ = stream.read(FRAME)
                score = wake_score(wake, frame[:, 0])
                if score < args.threshold:
                    continue
                activations += 1
                print(f"[wake] score={score:.2f}")
                wake.reset()
                if args.idle:
                    continue

                audio, end_of_speech = record_command(stream, noise)
                if audio is None:
                    print("  (no command heard)\n")
                    continue
                stream.stop()
                text = transcribe(stt, audio)
                stt_done = time.perf_counter()
                print(f"  heard: {text!r}  stt={stt_done - end_of_speech:.2f}s")
                synth = speak(f"Escuché: {text}" if text else "No te entendí")
                total = stt_done - end_of_speech + synth
                latencies.append(total)
                print(f"  tts={synth:.2f}s  end-of-speech -> reply={total:.2f}s\n")
                stream.start()
                wake.reset()
        except KeyboardInterrupt:
            pass

    minutes = (time.perf_counter() - started_at) / 60
    print(f"\nActivations: {activations} in {minutes:.1f} min")
    if latencies:
        print(f"Latency avg={np.mean(latencies):.2f}s max={np.max(latencies):.2f}s")


def selftest(args):
    print("Loading models...")
    wake = load_wake_model()
    stt = load_stt(args.stt_model)

    for phrase, voice in [("Hey Jarvis", "en-US-GuyNeural"), ("Hola, ¿cómo va?", REPLY_VOICE)]:
        audio = decode_mp3(tts_bytes(phrase, voice))
        audio = np.concatenate([np.zeros(SAMPLE_RATE, np.int16), audio, np.zeros(SAMPLE_RATE, np.int16)])
        wake.reset()
        best = max(wake_score(wake, audio[i : i + FRAME]) for i in range(0, len(audio) - FRAME, FRAME))
        print(f"wake score for {phrase!r}: {best:.2f} (threshold {args.threshold})")

    command = decode_mp3(tts_bytes("Abrí Spotify y subí el volumen al cincuenta por ciento"))
    t0 = time.perf_counter()
    text = transcribe(stt, command)
    print(f"stt ({args.stt_model}): {text!r} in {time.perf_counter() - t0:.2f}s for {len(command) / SAMPLE_RATE:.1f}s of audio")

    t0 = time.perf_counter()
    tts_bytes(f"Escuché: {text}")
    print(f"tts synth: {time.perf_counter() - t0:.2f}s")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--selftest", action="store_true")
    parser.add_argument("--idle", type=float, default=0, help="seconds to count false activations")
    parser.add_argument("--threshold", type=float, default=0.5)
    parser.add_argument("--stt-model", default="small")
    args = parser.parse_args()
    selftest(args) if args.selftest else live(args)


if __name__ == "__main__":
    main()
