"""Speech to text: faster-whisper on CUDA, language restricted to es/en.

Setup follows spike S1 (spikes/voice/S1_RESULTS.md): ``small``, float16, beam 1,
no VAD filter, detect Spanish vs English first, then transcribe in the winner.
Compute types are probed and fall back truthfully: a model that loaded in a
different type than float16 says so in the log, and a GPU that cannot be used
is an error, not a silent CPU run.
"""

from __future__ import annotations

import logging
import os
import sys
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np

from protocol import ErrorCode

log = logging.getLogger("niva.voice.stt")

# Preference order per device; only types the device supports are tried.
CUDA_COMPUTE_ORDER = ("float16", "int8_float16", "int8", "float32")
CPU_COMPUTE_ORDER = ("int8", "int8_float32", "float32")

_OOM_HINTS = ("out of memory", "alloc_failed", "memoryallocation", "cudaerrormemory")


class SttError(Exception):
    """Loading or running Whisper failed in a way the taxonomy names."""

    def __init__(self, code: ErrorCode, msg: str):
        super().__init__(msg)
        self.code = code
        self.msg = msg


@dataclass(frozen=True)
class SttResult:
    text: str
    lang: str  # "es" or "en", the higher of the two (never another language)
    lang_prob: float  # combined es + en probability
    p_es: float = 0.0
    p_en: float = 0.0


def is_oom(exc: BaseException) -> bool:
    text = str(exc).lower()
    return any(hint in text for hint in _OOM_HINTS)


def classify_runtime_error(exc: BaseException) -> ErrorCode:
    return ErrorCode.CUDA_OOM if is_oom(exc) else ErrorCode.INTERNAL


def compute_type_order(supported: Iterable[str], device: str) -> list[str]:
    """The compute types to try, best first, limited to what the device supports."""
    order = CUDA_COMPUTE_ORDER if device == "cuda" else CPU_COMPUTE_ORDER
    available = set(supported)
    return [ct for ct in order if ct in available]


def add_nvidia_dll_dirs() -> list[str]:
    """ctranslate2 on Windows needs cuBLAS/cuDNN DLLs; the pip wheels ship them in
    ``nvidia/*/bin``. Must run before ctranslate2 is imported (s1_gpu_stt.py)."""
    if sys.platform != "win32":
        return []
    import importlib.util

    spec = importlib.util.find_spec("nvidia")
    if spec is None or not spec.submodule_search_locations:
        return []
    added = []
    for base in spec.submodule_search_locations:
        for bin_dir in sorted(Path(base).glob("*/bin")):
            os.add_dll_directory(str(bin_dir))
            os.environ["PATH"] = str(bin_dir) + os.pathsep + os.environ.get("PATH", "")
            added.append(str(bin_dir))
    return added


def load_model(
    make_model: Callable[[str], Any],
    supported: Iterable[str],
    device: str,
) -> tuple[Any, str]:
    """Try each supported compute type; return ``(model, compute_type)``.

    An out-of-memory error stops the search at once (``CUDA_OOM``): a smaller
    compute type would hide that the host should pick a smaller model. Any other
    failure moves to the next type; when none loads the error is ``MODEL_LOAD``.
    """
    order = compute_type_order(supported, device)
    if not order:
        raise SttError(ErrorCode.MODEL_LOAD, f"no usable compute type on {device}")
    failures: list[str] = []
    for compute_type in order:
        try:
            model = make_model(compute_type)
        except Exception as exc:  # noqa: BLE001 - ctranslate2 raises RuntimeError and others
            if is_oom(exc):
                raise SttError(
                    ErrorCode.CUDA_OOM, f"out of GPU memory loading Whisper ({compute_type}): {exc}"
                ) from exc
            failures.append(f"{compute_type}: {exc}")
            log.warning("whisper %s failed to load: %s", compute_type, exc)
            continue
        if compute_type != order[0]:
            log.warning("whisper loaded as %s, not the preferred %s", compute_type, order[0])
        return model, compute_type
    raise SttError(ErrorCode.MODEL_LOAD, "could not load Whisper: " + " | ".join(failures))


def detect_es_en(model: Any, audio: np.ndarray) -> tuple[str, float, float]:
    """Pick Spanish or English by comparing their probabilities (design D1)."""
    _lang, _prob, all_probs = model.detect_language(audio)
    probs = dict(all_probs)
    p_es, p_en = float(probs.get("es", 0.0)), float(probs.get("en", 0.0))
    return ("es" if p_es >= p_en else "en"), p_es, p_en


class WhisperTranscriber:
    def __init__(self, model: Any, compute_type: str, device: str):
        self._model = model
        self.compute_type = compute_type
        self.device = device

    @classmethod
    def load(
        cls,
        name_or_path: str = "small",
        *,
        allow_cpu: bool = False,
        local_files_only: bool = True,
    ) -> WhisperTranscriber:
        """Load Whisper. Never downloads unless ``local_files_only`` is False."""
        add_nvidia_dll_dirs()
        try:
            import ctranslate2
            from faster_whisper import WhisperModel
        except Exception as exc:  # noqa: BLE001 - missing DLLs surface as OSError/ImportError
            raise SttError(ErrorCode.MODEL_LOAD, f"cannot import the Whisper runtime: {exc}") from exc

        device = "cuda"
        try:
            supported = ctranslate2.get_supported_compute_types("cuda")
        except Exception as exc:  # noqa: BLE001 - no CUDA device or driver
            supported = set()
            log.warning("CUDA is not usable: %s", exc)
        if not supported:
            if not allow_cpu:
                raise SttError(
                    ErrorCode.MODEL_LOAD, "CUDA is not available (pass --allow-cpu to run Whisper on the CPU)"
                )
            device = "cpu"
            supported = ctranslate2.get_supported_compute_types("cpu")
            log.warning("running Whisper on the CPU: latency targets will not hold")

        def make(compute_type: str) -> Any:
            return WhisperModel(
                name_or_path, device=device, compute_type=compute_type, local_files_only=local_files_only
            )

        model, compute_type = load_model(make, supported, device)
        log.info("whisper %s on %s (%s)", name_or_path, device, compute_type)
        transcriber = cls(model, compute_type, device)
        transcriber.warm_up()
        return transcriber

    def warm_up(self) -> None:
        """A couple of passes so CUDA kernels are built before the first command."""
        silence = np.zeros(16_000, dtype=np.float32)
        try:
            for _ in range(2):
                self.transcribe(silence)
        except Exception as exc:  # noqa: BLE001
            if is_oom(exc):
                raise SttError(ErrorCode.CUDA_OOM, f"out of GPU memory warming up Whisper: {exc}") from exc
            raise SttError(ErrorCode.MODEL_LOAD, f"Whisper warm-up failed: {exc}") from exc

    def transcribe(self, audio: np.ndarray) -> SttResult:
        lang, p_es, p_en = detect_es_en(self._model, audio)
        segments, _info = self._model.transcribe(
            audio,
            language=lang,
            beam_size=1,
            vad_filter=False,
            condition_on_previous_text=False,
            temperature=0.0,
        )
        text = "".join(segment.text for segment in segments).strip()
        return SttResult(text=text, lang=lang, lang_prob=p_es + p_en, p_es=p_es, p_en=p_en)
