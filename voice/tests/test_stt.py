import logging
from types import SimpleNamespace

import numpy as np
import pytest

from audio_io import classify_mic_error
from config import Tunables
from protocol import ErrorCode
from stt import (
    SttError,
    WhisperTranscriber,
    add_nvidia_dll_dirs,
    classify_runtime_error,
    compute_type_order,
    detect_es_en,
    is_oom,
    load_model,
    single_pass_decode,
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


# ------------------------------------------------------------ single encoder pass


def seg(text, no_speech_prob=0.05):
    return SimpleNamespace(text=text, no_speech_prob=no_speech_prob)


def single_pass_fake(result):
    """A `single_pass` seam that records its calls and returns `result`."""
    calls = []

    def run(model, audio, forced, options):
        calls.append((forced, options))
        if isinstance(result, Exception):
            raise result
        return result

    run.calls = calls
    return run


def test_the_single_pass_result_is_transcribed_without_touching_the_public_two_pass_api():
    model = FakeWhisper({"es": 0.8, "en": 0.15})
    fast = single_pass_fake(("es", 0.8, 0.15, [seg("abrí "), seg("Spotify")]))
    result = WhisperTranscriber(model, "float16", "cuda", single_pass=fast).transcribe(clip(2))
    assert (result.text, result.lang) == ("abrí Spotify", "es")
    assert result.lang_prob == pytest.approx(0.95) and (result.p_es, result.p_en) == (0.8, 0.15)
    assert model.detections == 0 and model.transcribe_kwargs is None  # no second encoder pass, no public call
    forced, options = fast.calls[0]
    assert forced is None
    assert (options["beam_size"], options["vad_filter"], options["temperature"]) == (1, False, 0.0)
    assert options["condition_on_previous_text"] is False
    assert "without_timestamps" not in options  # command mode keeps timestamps, like the public call


def test_the_single_pass_keeps_the_hallucination_gate_and_the_low_probability_path():
    limit = Tunables().no_speech_prob_max
    fast = single_pass_fake(("es", 0.05, 0.02, [seg("abre la calculadora"), seg("Gracias.", limit + 0.2)]))
    result = WhisperTranscriber(FakeWhisper({}), "float16", "cuda", single_pass=fast).transcribe(clip(2))
    assert (result.text, result.dropped_segments, result.lang) == ("abre la calculadora", 1, "es")
    assert result.lang_prob == pytest.approx(0.07)  # still reported, still decoded in the higher language


def test_a_confirm_listen_hands_its_language_and_prompt_to_the_single_pass():
    fast = single_pass_fake(("es", 1.0, 0.0, [seg("Sí.")]))
    result = WhisperTranscriber(FakeWhisper({}), "float16", "cuda", single_pass=fast).transcribe(
        clip(1), mode="confirm", lang="es", context="¿Cierro la calculadora?"
    )
    forced, options = fast.calls[0]
    assert forced == "es" and options["initial_prompt"] == "¿Cierro la calculadora?"
    assert options["max_new_tokens"] == Tunables().confirm_max_new_tokens
    assert (result.text, result.lang, result.lang_prob) == ("Sí.", "es", 1.0)


def test_when_the_internals_fail_it_falls_back_to_the_public_path_and_logs_once(caplog):
    model = FakeWhisper({"es": 0.8, "en": 0.15})
    fast = single_pass_fake(AttributeError("no attribute 'generate_segments'"))
    stt = WhisperTranscriber(model, "float16", "cuda", single_pass=fast)
    with caplog.at_level(logging.WARNING, logger="niva.voice.stt"):
        first = stt.transcribe(clip(2))
        second = stt.transcribe(clip(2))
    assert first.text == second.text == "abrí Spotify" and first.lang == "es"
    assert model.detections == 2 and model.transcribe_kwargs["language"] == "es"
    assert len(fast.calls) == 1, "a broken fast path is not retried on every utterance"
    assert len([r for r in caplog.records if "single-pass" in r.getMessage()]) == 1


@pytest.mark.parametrize("mode", ["command", "confirm"])
@pytest.mark.parametrize("forced", [None, "es"])
def test_the_single_pass_gets_exactly_the_options_the_public_path_uses(mode, forced):
    kwargs = dict(mode=mode, lang=forced, context="¿Cierro la calculadora?")
    public_model = FakeWhisper({"es": 0.8, "en": 0.15})
    WhisperTranscriber(public_model, "float16", "cuda", tunables=Tunables(single_pass_stt=False)).transcribe(
        clip(2), **kwargs
    )
    fast = single_pass_fake(("es", 0.8, 0.15, [seg("x")]))
    WhisperTranscriber(FakeWhisper({}), "float16", "cuda", single_pass=fast).transcribe(clip(2), **kwargs)
    public_options = {k: v for k, v in public_model.transcribe_kwargs.items() if k != "language"}
    assert fast.calls[0][1] == public_options


@pytest.mark.parametrize(
    "error", [ImportError("x"), AttributeError("x"), KeyError("x"), TypeError("x"), NameError("x")]
)
def test_an_internals_mismatch_disables_the_single_pass_for_good(error, caplog):
    fast = single_pass_fake(error)
    stt = WhisperTranscriber(FakeWhisper({"es": 0.8, "en": 0.15}), "float16", "cuda", single_pass=fast)
    with caplog.at_level(logging.WARNING, logger="niva.voice.stt"):
        stt.transcribe(clip(1))
        stt.transcribe(clip(1))
    assert len(fast.calls) == 1 and stt._single_pass is None
    assert [r.getMessage() for r in caplog.records if "single-pass" in r.getMessage()][0].startswith(
        "single-pass Whisper unavailable"
    )


def test_any_other_single_pass_failure_costs_that_call_only(caplog):
    outcomes = [ValueError("audio too short to encode"), ("es", 0.8, 0.15, [seg("fast")])]
    calls = []

    def flaky(model, audio, forced, options):
        calls.append(forced)
        outcome = outcomes.pop(0)
        if isinstance(outcome, Exception):
            raise outcome
        return outcome

    model = FakeWhisper({"es": 0.8, "en": 0.15})
    stt = WhisperTranscriber(model, "float16", "cuda", single_pass=flaky)
    with caplog.at_level(logging.WARNING, logger="niva.voice.stt"):
        first = stt.transcribe(clip(1))
        second = stt.transcribe(clip(1))
    assert first.text == "abrí Spotify" and model.transcribe_kwargs is not None  # the public path answered once
    assert second.text == "fast" and len(calls) == 2 and stt._single_pass is flaky  # and the fast path came back
    messages = [r.getMessage() for r in caplog.records if "single-pass" in r.getMessage()]
    assert len(messages) == 1 and messages[0].startswith("single-pass Whisper failed once")


def test_a_gpu_out_of_memory_is_not_mistaken_for_missing_internals():
    fast = single_pass_fake(RuntimeError("CUDA failed with error out of memory"))
    stt = WhisperTranscriber(FakeWhisper({"es": 0.8, "en": 0.1}), "float16", "cuda", single_pass=fast)
    with pytest.raises(RuntimeError, match="out of memory"):
        stt.transcribe(clip(1))


def test_the_switch_off_uses_the_public_path_only():
    model = FakeWhisper({"es": 0.8, "en": 0.15})
    stt = WhisperTranscriber(model, "float16", "cuda", tunables=Tunables(single_pass_stt=False))
    assert stt._single_pass is None
    assert stt.transcribe(clip(1)).text == "abrí Spotify" and model.detections == 1


# The decoder itself, against the real faster-whisper classes but a fake model.


def faster_whisper_classes():
    add_nvidia_dll_dirs()
    fw = pytest.importorskip("faster_whisper")
    return fw.WhisperModel, pytest.importorskip("faster_whisper.transcribe").TranscriptionOptions


class FakeHfTokenizer:
    IDS = {"<|endoftext|>": 50257, "<|startoftranscript|>": 50258, "<|transcribe|>": 50359, "<|translate|>": 50358,
           "<|notimestamps|>": 50363, "<|nospeech|>": 50362, "<|startoflm|>": 50360, "<|startofprev|>": 50361,
           "<|es|>": 50262, "<|en|>": 50259}

    def token_to_id(self, token):
        return self.IDS.get(token, 50400)

    def encode(self, text, add_special_tokens=False):
        return SimpleNamespace(ids=[ord(c) for c in text])


def fake_faster_whisper(probs, multilingual=True):
    WhisperModel, _ = faster_whisper_classes()

    class Fake:
        transcribe = WhisperModel.transcribe  # only its signature (the defaults) is read

        def __init__(self):
            self.model = SimpleNamespace(is_multilingual=multilingual, detect_language=self._detect)
            self.hf_tokenizer = FakeHfTokenizer()
            self.encoded, self.detected, self.decoded = [], [], []
            self.encoder_output = object()

        def _detect(self, encoder_output):
            self.detected.append(encoder_output)
            return [[(f"<|{code}|>", p) for code, p in probs.items()]]

        def feature_extractor(self, audio):
            return np.zeros((80, len(audio) // 160 + 1), dtype=np.float32)

        def encode(self, features):
            self.encoded.append(features.shape)
            return self.encoder_output

        def generate_segments(self, features, tokenizer, options, log_progress, encoder_output=None):
            self.decoded.append((features.shape, tokenizer, options, encoder_output))
            return iter([seg("abrí "), seg("Spotify")])

    return Fake()


def test_one_encoder_pass_feeds_both_the_language_choice_and_the_decoder():
    model = fake_faster_whisper({"fr": 0.6, "es": 0.3, "en": 0.1})
    options = {"beam_size": 1, "temperature": 0.0, "max_new_tokens": 24}
    chosen, p_es, p_en, segments = single_pass_decode(model, clip(2), None, options)
    assert (chosen, p_es, p_en) == ("es", 0.3, 0.1)  # es vs en only: "fr" never wins
    assert [s.text for s in segments] == ["abrí ", "Spotify"]
    assert model.encoded == [(80, 3000)], "one encoder pass over a padded 30 s window"
    assert model.detected == [model.encoder_output], "the language comes from that same encoder output"
    features_shape, tokenizer, options, encoder_output = model.decoded[0]
    assert encoder_output is model.encoder_output, "the decoder reuses the encoder output"
    assert tokenizer.language_code == "es"
    assert (options.beam_size, options.temperatures, options.max_new_tokens) == (1, [0.0], 24)


def test_the_decoder_options_are_the_callers_plus_the_public_defaults():
    model = fake_faster_whisper({"es": 0.1, "en": 0.8})
    options = {
        "beam_size": 1,
        "condition_on_previous_text": False,
        "temperature": 0.0,
        "max_new_tokens": 12,
        "without_timestamps": True,
    }
    chosen, *_ = single_pass_decode(model, clip(1), None, options)
    options = model.decoded[0][2]
    assert chosen == "en" and model.decoded[0][1].language_code == "en"
    assert options.without_timestamps is True and options.condition_on_previous_text is False
    assert options.no_speech_threshold == 0.6 and options.log_prob_threshold == -1.0  # faster-whisper's own defaults
    assert options.suppress_tokens and -1 not in options.suppress_tokens  # expanded like transcribe() does


def test_a_forced_language_skips_the_detection_but_still_encodes_once():
    model = fake_faster_whisper({"es": 0.1, "en": 0.8})
    options = {"initial_prompt": "¿Cierro?", "without_timestamps": True}
    chosen, p_es, p_en, _ = single_pass_decode(model, clip(1), "es", options)
    assert (chosen, p_es, p_en) == ("es", 1.0, 0.0)
    assert model.detected == [] and len(model.encoded) == 1
    assert model.decoded[0][2].initial_prompt == "¿Cierro?"


def test_an_english_only_model_is_rejected_so_the_caller_falls_back():
    with pytest.raises(RuntimeError):
        single_pass_decode(fake_faster_whisper({"en": 1.0}, multilingual=False), clip(1), None, {})


def test_the_pinned_faster_whisper_still_has_the_internals_the_single_pass_uses():
    """A guard for upgrades: if this fails, the single-pass path is falling back (slower, not wrong)."""
    import dataclasses
    import inspect

    WhisperModel, TranscriptionOptions = faster_whisper_classes()
    assert "encoder_output" in inspect.signature(WhisperModel.generate_segments).parameters
    transcribe_params = set(inspect.signature(WhisperModel.transcribe).parameters)
    fields = {f.name for f in dataclasses.fields(TranscriptionOptions)}
    assert fields - {"temperatures"} <= transcribe_params
    assert hasattr(WhisperModel, "encode") and hasattr(WhisperModel, "generate_segments")
