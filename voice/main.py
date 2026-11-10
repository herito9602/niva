"""Niva speech sidecar: JSON lines over stdio (see protocol.py).

    uv run --offline --no-project python main.py [options]

stdout carries protocol lines only. Anything a library prints to stdout is moved
to stderr, and logs go to stderr (and to --log-file when given).

Exit codes: 0 clean shutdown, 3 the models did not load (error sent first),
1 an unrecoverable runtime error (error sent first).
"""

from __future__ import annotations

import argparse
import dataclasses
import logging
import os
import queue
import sys
import threading
import time
from collections.abc import Callable
from pathlib import Path
from typing import IO, Any

import protocol
from audio_io import MicError, Microphone, SoundDeviceMic, SoundDeviceOutput, classify_mic_error
from capture import Engine, SidecarFatal
from config import Tunables
from discards import DiscardLog
from protocol import ErrorCode, ProtocolError, Shutdown, parse_command

log = logging.getLogger("niva.voice")

EXIT_OK = 0
EXIT_RUNTIME = 1
EXIT_LOAD = 3


class Emitter:
    """Writes protocol lines to the real stdout, one at a time."""

    def __init__(self, stream: IO[bytes]):
        self._stream = stream
        self._lock = threading.Lock()

    def __call__(self, message: dict[str, Any]) -> None:
        line = protocol.encode(message).encode("utf-8") + b"\n"
        with self._lock:
            try:
                view = memoryview(line)
                while view:  # a raw pipe may take only part of a line
                    written = self._stream.write(view)
                    if not written:
                        raise OSError("the host pipe accepted no bytes")
                    view = view[written:]
                self._stream.flush()
            except (BrokenPipeError, ValueError, OSError):
                log.error("host pipe closed; dropping %s", message.get("type"))


def claim_stdout() -> IO[bytes]:
    """Keep the real stdout for the protocol; point fd 1 and ``sys.stdout`` at stderr."""
    sys.stdout.flush()
    protocol_fd = os.dup(1)
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    return os.fdopen(protocol_fd, "wb", buffering=0)


def monotonic_ms() -> int:
    return int(time.monotonic() * 1000)


Inbox = queue.Queue[tuple[Any, ...]]


def read_commands(lines: IO[bytes], inbox: Inbox, gone: threading.Event) -> None:
    """Host lines -> inbox. Runs on its own thread; ends at EOF or `shutdown`.
    However it ends, ``gone`` is set (so a sidecar still loading models can stop)
    and, unless the host said `shutdown`, `eof` is queued."""
    shutdown = False
    try:
        for raw in lines:
            text = raw.decode("utf-8", errors="replace").strip()
            if not text:
                continue
            try:
                command = parse_command(text)
            except ProtocolError as exc:
                inbox.put(("proto", str(exc)))
                continue
            except Exception as exc:  # noqa: BLE001 - e.g. RecursionError on a hostile line
                inbox.put(("proto", f"unreadable line: {type(exc).__name__}"))
                continue
            inbox.put(("cmd", command))
            if isinstance(command, Shutdown):
                shutdown = True
                return
    finally:
        if not shutdown:
            inbox.put(("eof",))
        gone.set()


def _as_mic_error(exc: BaseException) -> MicError:
    return exc if isinstance(exc, MicError) else classify_mic_error(exc)


class Sidecar:
    """The threads and the queue around an :class:`Engine`."""

    def __init__(
        self,
        engine: Engine,
        mic: Microphone,
        emit: Callable[[dict[str, Any]], None],
        tunables: Tunables,
        inbox: Inbox | None = None,
        clock_ms: Callable[[], int] = monotonic_ms,
    ):
        self._clock = clock_ms
        self.engine = engine
        self._mic = mic
        self._emit = emit
        self._t = tunables
        self.inbox: Inbox = inbox if inbox is not None else queue.Queue()
        self.stop = threading.Event()

    # -- producers

    def speak_ended(self, speak_id: int) -> None:
        self.inbox.put(("speak_ended", speak_id))

    def capture_loop(self) -> None:
        """One stream for the sidecar's life. If it fails: report, then retry opening
        it until it works, so the sidecar never stays silently deaf. Every failure is
        sent; the engine drops exact repeats, a different one is news. Any exception
        ends up here as a MIC_* error; the thread never dies quietly."""
        while not self.stop.is_set():
            try:
                self._mic.open()
            except Exception as exc:  # noqa: BLE001
                error = _as_mic_error(exc)
                self.inbox.put(("mic_error", error))
                if self.stop.wait(self._t.mic_retry_s):
                    return
                continue
            self.inbox.put(("mic_ok",))
            try:
                while not self.stop.is_set():
                    frame = self._mic.read()
                    self.inbox.put(("frame", frame, self._clock()))  # stamped when it arrived
            except Exception as exc:  # noqa: BLE001
                self.inbox.put(("mic_error", _as_mic_error(exc)))
            finally:
                self._mic.close()
            if self.stop.wait(self._t.mic_retry_s):
                return

    # -- consumer

    def run(self) -> int:
        capture = threading.Thread(target=self.capture_loop, name="capture", daemon=True)
        capture.start()
        code = EXIT_OK
        try:
            while True:
                item = self.inbox.get()
                try:
                    if not self._handle(item):
                        break
                except SidecarFatal as fatal:
                    self._emit(protocol.error(fatal.code, fatal.msg, fatal.message_id))
                    code = EXIT_LOAD if fatal.code in (ErrorCode.CUDA_OOM, ErrorCode.MODEL_LOAD) else EXIT_RUNTIME
                    break
                except Exception as exc:  # noqa: BLE001 - last line of defence
                    log.exception("unexpected failure")
                    self._emit(protocol.error(ErrorCode.INTERNAL, f"unexpected failure: {exc}"))
                    code = EXIT_RUNTIME
                    break
        finally:
            self.stop.set()
            self._mic.close()  # the capture thread may still be blocked in a read
        return code

    def _handle(self, item: tuple[Any, ...]) -> bool:
        kind = item[0]
        if kind == "frame":
            self.engine.on_frame(item[1], item[2])
        elif kind == "cmd":
            if isinstance(item[1], Shutdown):
                return False
            self.engine.on_command(item[1])
        elif kind == "proto":
            self._emit(protocol.error(ErrorCode.PROTOCOL, item[1]))
        elif kind == "mic_ok":
            self.engine.on_mic_ok()
        elif kind == "mic_error":
            self.engine.on_mic_error(item[1].code, item[1].msg)
        elif kind == "speak_ended":
            self.engine.on_speak_ended(item[1])
        elif kind == "eof":
            return False
        return True


# ---------------------------------------------------------------- startup


def parse_args(argv: list[str] | None = None) -> argparse.Namespace:
    env = os.environ.get
    p = argparse.ArgumentParser(description="Niva speech sidecar")
    p.add_argument("--whisper-model", default=env("NIVA_WHISPER_MODEL", "small"), help="name or path (default: small)")
    p.add_argument("--stt-wait-s", type=float, default=0.0, help="wait before loading Whisper (LLM warm-up first)")
    p.add_argument("--allow-cpu", action="store_true", help="run Whisper on the CPU when CUDA is missing")
    p.add_argument("--allow-download", action="store_true", help="let faster-whisper download a missing model")
    p.add_argument(
        "--wake-model-dir",
        default=env("NIVA_WAKE_MODEL_DIR"),
        help="folder with the openWakeWord .onnx files",
    )
    p.add_argument("--piper-dir", default=env("NIVA_PIPER_DIR"), help="folder with the Piper voices (fallback TTS)")
    p.add_argument("--no-edge", action="store_true", help="speak with Piper only (nothing leaves the machine)")
    p.add_argument("--no-robot-voice", action="store_true", help="speak with the plain voices, without the robot effect")
    p.add_argument(
        "--no-preopen-output",
        action="store_true",
        help="open the output device at the first audio, not in parallel with synthesis",
    )
    p.add_argument(
        "--no-single-pass-stt",
        action="store_true",
        help="transcribe through faster-whisper's public API (two encoder passes)",
    )
    p.add_argument(
        "--no-persistent-edge", action="store_true", help="open a new edge-tts connection for every reply"
    )
    p.add_argument("--no-tts-cache", action="store_true", help="do not cache short synthesized replies on disk")
    p.add_argument(
        "--tts-cache-dir",
        default=None,
        help="folder for the reply cache (default: NIVA_TTS_CACHE_DIR, else %%LOCALAPPDATA%%\\Niva\\tts-cache)",
    )
    p.add_argument("--mic-device", default=None, help="sounddevice input device index or name")
    p.add_argument(
        "--dev-host", action="store_true", help="act as a tiny host: listen after wake, speak the transcript"
    )
    p.add_argument("--log-file", default=None)
    p.add_argument("--verbose", action="store_true")
    p.add_argument(
        "--no-speculative-stt", action="store_true", help="transcribe only after the end of speech is confirmed"
    )
    for field in ("pre_speech_ms", "wake_preroll_ms", "end_silence_ms", "speculative_stt_ms"):
        p.add_argument(f"--{field.replace('_', '-')}", type=int, default=None)
    p.add_argument("--wake-threshold", type=float, default=None)
    return p.parse_args(argv)


def tunables_from(args: argparse.Namespace) -> Tunables:
    overrides = {
        name: getattr(args, name)
        for name in ("pre_speech_ms", "wake_preroll_ms", "end_silence_ms", "speculative_stt_ms", "wake_threshold")
        if getattr(args, name) is not None
    }
    if args.no_robot_voice:
        overrides["robot_voice"] = False
    if args.no_preopen_output:
        overrides["preopen_output"] = False
    if args.no_single_pass_stt:
        overrides["single_pass_stt"] = False
    if args.no_persistent_edge:
        overrides["persistent_edge_ws"] = False
    if args.no_tts_cache:
        overrides["tts_cache"] = False
    if args.no_speculative_stt:
        overrides["speculative_stt"] = False
    return dataclasses.replace(Tunables(), **overrides)


def setup_logging(args: argparse.Namespace) -> None:
    handlers: list[logging.Handler] = [logging.StreamHandler(sys.stderr)]
    if args.log_file:
        handlers.append(logging.FileHandler(args.log_file, encoding="utf-8"))
    logging.basicConfig(
        level=logging.DEBUG if args.verbose else logging.INFO,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        handlers=handlers,
        force=True,
    )


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    setup_logging(args)
    emit = Emitter(claim_stdout())
    tunables = tunables_from(args)

    import stt as stt_module

    # Before anything can import ctranslate2 (the VAD or a wake model import may pull it in).
    stt_module.add_nvidia_dll_dirs()
    from stt import SttError, WhisperTranscriber
    from tts import EdgeTtsEngine, PiperEngine, Speaker
    from vad import SileroStream
    from wake import OpenWakeWordDetector

    try:
        wake = OpenWakeWordDetector(args.wake_model_dir)
        vad = SileroStream()
    except Exception as exc:  # noqa: BLE001 - WakeModelError, ONNX, missing files
        log.exception("wake/VAD models did not load")
        emit(protocol.error(ErrorCode.MODEL_LOAD, f"wake word or VAD model did not load: {exc}"))
        return EXIT_LOAD

    # Read the host from now on, so a `shutdown` sent while models load is not lost.
    inbox: Inbox = queue.Queue()
    gone = threading.Event()
    threading.Thread(target=read_commands, args=(sys.stdin.buffer, inbox, gone), name="stdin", daemon=True).start()

    if args.stt_wait_s > 0:
        log.info("waiting up to %.0f s before loading Whisper", args.stt_wait_s)
        gone.wait(args.stt_wait_s)
    if gone.is_set():
        return EXIT_OK

    try:
        stt = WhisperTranscriber.load(
            args.whisper_model,
            allow_cpu=args.allow_cpu,
            local_files_only=not args.allow_download,
            tunables=tunables,
        )
    except SttError as exc:
        log.error("Whisper did not load: %s", exc.msg)
        emit(protocol.error(exc.code, exc.msg))
        return EXIT_LOAD

    if args.dev_host:
        from devhost import wrap_emit

        emit = wrap_emit(emit, inbox)
    mic = SoundDeviceMic(tunables.mic_stall_s, _device(args.mic_device))
    discards = DiscardLog()
    engines: list[Any] = []
    edge_client = None
    if not args.no_edge:
        if tunables.persistent_edge_ws:
            from edge_ws import EdgeWsClient

            edge_client = EdgeWsClient(stale_after_s=tunables.edge_ws_stale_s)
        engines.append(EdgeTtsEngine(tunables, client=edge_client, cache=_reply_cache(args, tunables)))
    if args.piper_dir:
        piper = PiperEngine(args.piper_dir)
        if piper.available():
            engines.append(piper)
            threading.Thread(target=_preload, args=(piper,), name="piper-preload", daemon=True).start()
        else:
            log.warning("Piper voices not found in %s; no offline fallback", args.piper_dir)
    holder: list[Sidecar] = []
    speaker = Speaker(engines, SoundDeviceOutput, emit, lambda i: holder[0].speak_ended(i), tunables)
    engine = Engine(
        wake=wake,
        vad=vad,
        stt=stt,
        speaker=speaker,
        emit=emit,
        discards=discards,
        tunables=tunables,
        clock_ms=monotonic_ms,
    )
    sidecar = Sidecar(engine, mic, emit, tunables, inbox)
    holder.append(sidecar)
    try:
        return sidecar.run()
    finally:
        speaker.stop()
        if edge_client is not None:
            edge_client.close()
        log.info("discard counts: %s", dict(discards.counts))


def _reply_cache(args: argparse.Namespace, tunables: Tunables) -> Any:
    """The on-disk cache of short synthesized replies, or None when it is off or has no folder."""
    if not tunables.tts_cache:
        return None
    from tts_cache import TtsCache, default_cache_dir

    directory = Path(args.tts_cache_dir) if args.tts_cache_dir else default_cache_dir()
    if directory is None:
        log.warning("no folder for the reply cache (LOCALAPPDATA is not set); caching is off")
        return None
    log.info("reply cache: %s", directory)
    return TtsCache(directory, tunables.tts_cache_max_mb * 1024 * 1024, tunables.tts_cache_max_chars)


def _preload(piper: Any) -> None:
    try:
        piper.preload()
    except Exception as exc:  # noqa: BLE001 - the fallback then loads on first use
        log.warning("Piper preload failed: %s", exc)


def _device(value: str | None) -> int | str | None:
    if value is None:
        return None
    return int(value) if value.isdigit() else value


if __name__ == "__main__":
    sys.exit(main())
