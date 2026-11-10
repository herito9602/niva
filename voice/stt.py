"""Speech to text: faster-whisper on CUDA, language restricted to es/en.

Setup follows spike S1 (spikes/voice/S1_RESULTS.md): ``small``, float16, beam 1,
no VAD filter, detect Spanish vs English first, then transcribe in the winner.

Two things on top of S1 (live run and offline repro of 2026-11-07, voice/README.md):

* A confirmation answer is one word, which detection and Whisper both get wrong
  ("Si" detected as English and written "C"). The host says which language it
  just spoke and what it asked, so a confirm listen forces that language and gives
  the question as ``initial_prompt``. The prompt is the QUESTION and never a list
  of answers: priming with "Yes. No. Okay." turned silence into "Okay.".
* A segment whose ``no_speech_prob`` is above ``Tunables.no_speech_prob_max`` is a
  hallucination ("Thank you." over the tail of Niva's own voice) and is dropped;
  what is left may be nothing, which the host treats as an empty transcript.
Compute types are probed and fall back truthfully: a model that loaded in a
different type than float16 says so in the log, and a GPU that cannot be used
is an error, not a silent CPU run.
"""

from __future__ import annotations

import dataclasses
import inspect
import logging
import os
import sys
from collections.abc import Callable, Iterable
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import numpy as np

from config import SAMPLE_RATE, Tunables
from protocol import ErrorCode

log = logging.getLogger("niva.voice.stt")

# Preference order per device; only types the device supports are tried.
CUDA_COMPUTE_ORDER = ("float16", "int8_float16", "int8", "float32")
CPU_COMPUTE_ORDER = ("int8", "int8_float32", "float32")

_OOM_HINTS = ("out of memory", "alloc_failed", "memoryallocation", "cudaerrormemory")

# What a faster-whisper whose internals changed raises in `single_pass_decode`. Only these
# switch the single-pass path off for good; any other failure is treated as that call's problem.
_INTERNALS_MISMATCH = (ImportError, AttributeError, KeyError, TypeError, NameError)


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
    lang_prob: float  # combined es + en probability; 1.0 when the host named the language
    p_es: float = 0.0
    p_en: float = 0.0
    dropped_segments: int = 0  # segments the no-speech gate removed


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


# What a single-pass decoder returns: the language, its es/en probabilities, and the
# Whisper segments (each with `.text` and `.no_speech_prob`).
SinglePassResult = tuple[str, float, float, list[Any]]
SinglePass = Callable[[Any, np.ndarray, "str | None", "dict[str, Any]"], SinglePassResult]


def single_pass_decode(model: Any, audio: np.ndarray, forced: str | None, options: dict[str, Any]) -> SinglePassResult:
    """Encode the audio once; pick es/en from that encoder output; decode from it.

    This is the ONLY place that uses faster-whisper 1.2.1 internals (the version
    installed; a test guards them): ``feature_extractor``, ``encode``, ``model.detect_language``,
    ``hf_tokenizer``, ``TranscriptionOptions`` and ``generate_segments`` with its
    ``encoder_output`` argument. ``generate_segments`` is the same code
    ``transcribe()`` runs after its language detection, so segmenting, the
    no-speech skip and the temperature fallback are unchanged; only the second
    encoder pass is gone. Any mismatch raises, and the caller falls back.

    ``options`` are ``transcribe()`` keyword arguments (``language`` and
    ``vad_filter`` are handled here / not needed).
    """
    from faster_whisper.audio import pad_or_trim
    from faster_whisper.tokenizer import Tokenizer
    from faster_whisper.transcribe import TranscriptionOptions, get_suppressed_tokens

    if not model.model.is_multilingual:
        raise RuntimeError("single-pass decoding needs a multilingual model")
    features = model.feature_extractor(audio)
    content_frames = features.shape[-1] - 1  # as generate_segments counts them
    if content_frames <= 0:
        raise ValueError("audio too short to encode")
    encoder_output = model.encode(pad_or_trim(features[..., :content_frames]))

    if forced is not None:
        chosen, p_es, p_en = forced, float(forced == "es"), float(forced == "en")
    else:
        # Restricted to Spanish vs English, like `detect_es_en`: same comparison, no second encoder pass.
        probs = {token[2:-2]: float(prob) for token, prob in model.model.detect_language(encoder_output)[0]}
        p_es, p_en = probs.get("es", 0.0), probs.get("en", 0.0)
        chosen = "es" if p_es >= p_en else "en"

    tokenizer = Tokenizer(model.hf_tokenizer, True, task="transcribe", language=chosen)
    defaults = {name: param.default for name, param in inspect.signature(model.transcribe).parameters.items()}
    values: dict[str, Any] = {}
    for field in dataclasses.fields(TranscriptionOptions):
        if field.name == "temperatures":
            temperature = options.get("temperature", defaults["temperature"])
            values[field.name] = list(temperature) if isinstance(temperature, (list, tuple)) else [temperature]
        elif field.name in options:
            values[field.name] = options[field.name]
        else:
            values[field.name] = defaults[field.name]  # KeyError here = a different faster-whisper
    if values["suppress_tokens"]:
        values["suppress_tokens"] = get_suppressed_tokens(tokenizer, list(values["suppress_tokens"]))
    segments = list(model.generate_segments(features, tokenizer, TranscriptionOptions(**values), False, encoder_output))
    return chosen, p_es, p_en, segments


class WhisperTranscriber:
    def __init__(
        self,
        model: Any,
        compute_type: str,
        device: str,
        tunables: Tunables | None = None,
        single_pass: SinglePass | None = None,
    ):
        self._model = model
        self.compute_type = compute_type
        self.device = device
        self._t = tunables or Tunables()
        # `None` = the public two-pass path. The seam lets tests drive the fast path with fakes.
        self._single_pass: SinglePass | None = single_pass or (single_pass_decode if self._t.single_pass_stt else None)

    @classmethod
    def load(
        cls,
        name_or_path: str = "small",
        *,
        allow_cpu: bool = False,
        local_files_only: bool = True,
        tunables: Tunables | None = None,
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
        transcriber = cls(model, compute_type, device, tunables)
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

    def _max_new_tokens(self, mode: str, samples: int) -> int:
        t = self._t
        if mode == "confirm":
            return t.confirm_max_new_tokens
        seconds = samples / SAMPLE_RATE
        wanted = int(seconds * t.command_tokens_per_s) + 1
        return max(t.command_min_new_tokens, min(t.command_max_new_tokens, wanted))

    def transcribe(
        self,
        audio: np.ndarray,
        *,
        mode: str = "command",
        lang: str | None = None,
        context: str | None = None,
    ) -> SttResult:
        """``lang`` and ``context`` only count in confirm mode (see the module doc)."""
        confirm = mode == "confirm"
        forced = lang if confirm and lang in ("es", "en") else None
        # One set of options for both paths: the single-pass decode must behave like the public call.
        options: dict[str, Any] = {
            "beam_size": 1,
            "vad_filter": False,
            "condition_on_previous_text": False,
            "temperature": 0.0,
            "max_new_tokens": self._max_new_tokens(mode, len(audio)),
        }
        if confirm:
            options["without_timestamps"] = True
            if forced is not None and context:
                options["initial_prompt"] = context
        segments = None
        if self._single_pass is not None:
            try:
                chosen, p_es, p_en, segments = self._single_pass(self._model, audio, forced, dict(options))
            except Exception as exc:  # noqa: BLE001 - fall back to the public API for this call
                if is_oom(exc):
                    raise
                if isinstance(exc, _INTERNALS_MISMATCH):
                    log.warning(
                        "single-pass Whisper unavailable (%s: %s); using the two-pass public API from now on",
                        type(exc).__name__,
                        exc,
                    )
                    self._single_pass = None  # a different faster-whisper: log once, not on every utterance
                else:
                    log.warning(
                        "single-pass Whisper failed once (%s: %s); using the two-pass public API for this call",
                        type(exc).__name__,
                        exc,
                    )
        if segments is None:
            if forced is not None:
                chosen, p_es, p_en = forced, float(forced == "es"), float(forced == "en")
            else:
                chosen, p_es, p_en = detect_es_en(self._model, audio)
            segments, _info = self._model.transcribe(audio, language=chosen, **options)
        kept: list[str] = []
        dropped = 0
        for segment in segments:
            if getattr(segment, "no_speech_prob", 0.0) > self._t.no_speech_prob_max:
                dropped += 1
            else:
                kept.append(segment.text)
        text = "".join(kept).strip()
        return SttResult(
            text=text,
            lang=chosen,
            lang_prob=1.0 if forced is not None else p_es + p_en,
            p_es=p_es,
            p_en=p_en,
            dropped_segments=dropped,
        )
