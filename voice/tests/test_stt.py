import numpy as np
import pytest

from audio_io import classify_mic_error
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
    def __init__(self, probs, segments=("abrí ", "Spotify")):
        self._probs, self._segments = probs, segments
        self.transcribe_kwargs = None

    def detect_language(self, audio):
        best = max(self._probs.items(), key=lambda kv: kv[1])
        return best[0], best[1], list(self._probs.items())

    def transcribe(self, audio, **kwargs):
        self.transcribe_kwargs = kwargs
        return iter(type("Seg", (), {"text": s}) for s in self._segments), None


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
    assert model.transcribe_kwargs == {
        "language": "es",
        "beam_size": 1,
        "vad_filter": False,
        "condition_on_previous_text": False,
        "temperature": 0.0,
    }


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
