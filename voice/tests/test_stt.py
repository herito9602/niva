import numpy as np
import pytest

from audio_io import classify_mic_error
from config import Tunables
from protocol import ErrorCode
from stt import (
    SttError,
    WhisperTranscriber,
    classify_runtime_error,
    compute_type_order,
    detect_es_en,
    is_oom,
    load_model,
)


def test_compute_type_order_prefers_float16_and_skips_unsupported_types():
    assert compute_type_order({"float32", "int8", "float16", "int8_float16"}, "cuda") == [
        "float16",
        "int8_float16",
        "int8",
        "float32",
    ]
    assert compute_type_order({"int8", "float32"}, "cuda") == ["int8", "float32"]
    assert compute_type_order({"float16"}, "cpu") == []
    assert compute_type_order({"int8", "float32"}, "cpu") == ["int8", "float32"]


def test_load_model_uses_the_first_type_that_loads():
    tried = []

    def make(compute_type):
        tried.append(compute_type)
        if compute_type == "float16":
            raise RuntimeError("float16 not supported on this GPU")
        return f"model-{compute_type}"

    model, compute_type = load_model(make, {"float16", "int8_float16", "int8"}, "cuda")
    assert (model, compute_type) == ("model-int8_float16", "int8_float16")
    assert tried == ["float16", "int8_float16"]  # the fallback is real, and the caller can see it


def test_load_model_out_of_memory_stops_and_reports_cuda_oom():
    tried = []

    def make(compute_type):
        tried.append(compute_type)
        raise RuntimeError("CUDA failed with error out of memory")

    with pytest.raises(SttError) as info:
        load_model(make, {"float16", "int8_float16"}, "cuda")
    assert info.value.code == ErrorCode.CUDA_OOM
    assert tried == ["float16"]  # a smaller type would hide that the host should pick a smaller model


def test_load_model_failing_everywhere_is_model_load_and_names_each_attempt():
    def make(compute_type):
        raise OSError(f"cannot open model for {compute_type}")

    with pytest.raises(SttError) as info:
        load_model(make, {"float16", "int8"}, "cuda")
    assert info.value.code == ErrorCode.MODEL_LOAD
    assert "float16" in info.value.msg and "int8" in info.value.msg


def test_load_model_without_any_supported_type_is_model_load():
    with pytest.raises(SttError) as info:
        load_model(lambda ct: object(), set(), "cuda")
    assert info.value.code == ErrorCode.MODEL_LOAD


@pytest.mark.parametrize(
    "text",
    ["CUDA failed with error out of memory", "CUBLAS_STATUS_ALLOC_FAILED", "cudaErrorMemoryAllocation"],
)
def test_out_of_memory_wordings_are_recognised(text):
    assert is_oom(RuntimeError(text))
    assert classify_runtime_error(RuntimeError(text)) == ErrorCode.CUDA_OOM


def test_other_runtime_errors_are_internal():
    assert classify_runtime_error(ValueError("bad shape")) == ErrorCode.INTERNAL


class FakeWhisper:
    """Segments are texts, or (text, no_speech_prob) pairs."""

    def __init__(self, probs, segments=("abrí ", "Spotify")):
        self._probs, self._segments = probs, segments
        self.transcribe_kwargs = None
        self.detections = 0

    def detect_language(self, audio):
        self.detections += 1
        best = max(self._probs.items(), key=lambda kv: kv[1])
        return best[0], best[1], list(self._probs.items())

    def transcribe(self, audio, **kwargs):
        self.transcribe_kwargs = kwargs
        pairs = [(s, 0.05) if isinstance(s, str) else s for s in self._segments]
        return iter(type("Seg", (), {"text": t, "no_speech_prob": p}) for t, p in pairs), None


def clip(seconds):
    return np.zeros(int(16000 * seconds), dtype=np.float32)


def test_detect_picks_the_higher_of_spanish_and_english_never_another_language():
    lang, p_es, p_en = detect_es_en(FakeWhisper({"fr": 0.6, "es": 0.3, "en": 0.1}), np.zeros(10))
    assert (lang, p_es, p_en) == ("es", 0.3, 0.1)
    lang, _, _ = detect_es_en(FakeWhisper({"de": 0.7, "es": 0.1, "en": 0.2}), np.zeros(10))
    assert lang == "en"


def test_transcribe_uses_the_s1_settings_and_reports_the_combined_probability():
    model = FakeWhisper({"es": 0.8, "en": 0.15, "fr": 0.05})
    result = WhisperTranscriber(model, "float16", "cuda").transcribe(np.zeros(16000, dtype=np.float32))
    assert result.text == "abrí Spotify"
    assert result.lang == "es"
    assert result.lang_prob == pytest.approx(0.95)  # es + en, what the D1 minimum gate uses
    kwargs = model.transcribe_kwargs
    assert kwargs["language"] == "es"
    assert (kwargs["beam_size"], kwargs["vad_filter"], kwargs["temperature"]) == (1, False, 0.0)
    assert kwargs["condition_on_previous_text"] is False
    assert not kwargs.get("initial_prompt") and not kwargs.get("without_timestamps")  # command mode: no prompt


def test_a_confirm_listen_forces_its_language_and_passes_the_question_as_the_prompt():
    model = FakeWhisper({"es": 0.3, "en": 0.6}, segments=("Sí.",))  # detection would say English
    result = WhisperTranscriber(model, "float16", "cuda").transcribe(
        clip(1), mode="confirm", lang="es", context="¿Cierro la calculadora?"
    )
    assert model.detections == 0, "the language is known: nothing to detect"
    kwargs = model.transcribe_kwargs
    assert kwargs["language"] == "es"
    assert kwargs["initial_prompt"] == "¿Cierro la calculadora?"
    assert kwargs["max_new_tokens"] == Tunables().confirm_max_new_tokens
    assert kwargs["without_timestamps"] is True
    assert (result.text, result.lang) == ("Sí.", "es")


def test_a_confirm_listen_without_a_question_falls_back_to_detection_and_sends_no_prompt():
    model = FakeWhisper({"es": 0.1, "en": 0.8}, segments=("Yes.",))
    result = WhisperTranscriber(model, "float16", "cuda").transcribe(clip(1), mode="confirm")
    assert model.detections == 1
    assert model.transcribe_kwargs["language"] == "en" and result.lang == "en"
    assert not model.transcribe_kwargs.get("initial_prompt")
    assert model.transcribe_kwargs["max_new_tokens"] == Tunables().confirm_max_new_tokens  # still short


def test_a_command_ignores_a_language_and_a_context_it_was_not_meant_to_get():
    model = FakeWhisper({"es": 0.1, "en": 0.8})
    WhisperTranscriber(model, "float16", "cuda").transcribe(clip(1), mode="command", lang="es", context="Q?")
    assert model.detections == 1 and model.transcribe_kwargs["language"] == "en"
    assert not model.transcribe_kwargs.get("initial_prompt")


def test_a_segment_the_model_thinks_is_not_speech_is_dropped():
    t = Tunables()
    model = FakeWhisper({"es": 0.9, "en": 0.05}, segments=[("Thank you.", t.no_speech_prob_max + 0.01)])
    result = WhisperTranscriber(model, "float16", "cuda").transcribe(clip(0.32), mode="confirm", lang="en")
    assert result.text == "" and result.dropped_segments == 1


def test_real_words_next_to_a_hallucinated_segment_survive():
    model = FakeWhisper({"es": 0.9, "en": 0.05}, segments=[("abre Spotify", 0.1), ("Gracias.", 0.8)])
    result = WhisperTranscriber(model, "float16", "cuda").transcribe(clip(2))
    assert (result.text, result.dropped_segments) == ("abre Spotify", 1)


def test_a_segment_exactly_at_the_limit_is_kept():
    limit = Tunables().no_speech_prob_max
    model = FakeWhisper({"es": 0.9, "en": 0.05}, segments=[("hola", limit)])
    assert WhisperTranscriber(model, "float16", "cuda").transcribe(clip(1)).text == "hola"


def test_the_output_is_bounded_per_mode_and_by_the_length_of_the_clip():
    t = Tunables()
    model = FakeWhisper({"es": 0.9, "en": 0.05})
    stt = WhisperTranscriber(model, "float16", "cuda")
    stt.transcribe(clip(1.088), mode="confirm", lang="es", context="q")
    assert model.transcribe_kwargs["max_new_tokens"] == t.confirm_max_new_tokens == 12
    stt.transcribe(clip(1.088))  # a 1 s command cannot return hundreds of characters
    short = model.transcribe_kwargs["max_new_tokens"]
    assert short == t.command_min_new_tokens
    stt.transcribe(clip(8))
    assert short < model.transcribe_kwargs["max_new_tokens"] <= t.command_max_new_tokens
    stt.transcribe(clip(30))
    assert model.transcribe_kwargs["max_new_tokens"] == t.command_max_new_tokens


def test_a_command_with_a_low_combined_probability_is_still_transcribed_in_the_higher_language():
    # The live run: a real Spanish command measured 0.07 for es + en together.
    model = FakeWhisper({"fr": 0.8, "es": 0.05, "en": 0.02}, segments=["abre la calculadora"])
    result = WhisperTranscriber(model, "float16", "cuda").transcribe(clip(2))
    assert (result.text, result.lang) == ("abre la calculadora", "es")
    assert result.lang_prob == pytest.approx(0.07)  # still reported
    assert model.transcribe_kwargs["language"] == "es"


def test_warm_up_out_of_memory_is_reported_as_cuda_oom():
    class Oom(FakeWhisper):
        def detect_language(self, audio):
            raise RuntimeError("out of memory")

    with pytest.raises(SttError) as info:
        WhisperTranscriber(Oom({}), "float16", "cuda").warm_up()
    assert info.value.code == ErrorCode.CUDA_OOM


@pytest.mark.parametrize(
    ("text", "code"),
    [
        ("Error opening InputStream: Device unavailable [PaErrorCode -9985]", ErrorCode.MIC_BUSY),
        ("Error querying device -1", ErrorCode.MIC_MISSING),
        ("No Default Input Device Available", ErrorCode.MIC_MISSING),
        ("something nobody planned for", ErrorCode.MIC_MISSING),
    ],
)
def test_portaudio_failures_map_to_mic_errors(text, code):
    assert classify_mic_error(RuntimeError(text)).code == code
