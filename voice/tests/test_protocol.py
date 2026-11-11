import json

import pytest

import protocol
from protocol import (
    CancelListen,
    ErrorCode,
    ForceEnd,
    Listen,
    ProtocolError,
    Shutdown,
    Speak,
    StopSpeaking,
    parse_command,
)


def test_parses_every_host_command():
    assert parse_command('{"type":"listen","id":7,"mode":"command"}') == Listen(7, "command")
    assert parse_command('{"type":"listen","id":8,"mode":"confirm"}') == Listen(8, "confirm")
    assert parse_command('{"type":"cancel_listen","id":7}') == CancelListen(7)
    assert parse_command('{"type":"force_end","id":7}') == ForceEnd(7)
    assert parse_command('{"type":"speak","id":9,"text":"Hola, ¿qué tal?","lang":"es"}') == Speak(
        9, "Hola, ¿qué tal?", "es"
    )
    assert parse_command('{"type":"stop_speaking"}') == StopSpeaking()
    assert parse_command('{"type":"shutdown"}') == Shutdown()


def test_a_confirm_listen_can_carry_the_language_and_the_question_it_answers():
    line = json.dumps(
        {"type": "listen", "id": 8, "mode": "confirm", "lang": "es", "context": "¿Cierro la calculadora?"},
        ensure_ascii=False,
    )
    assert parse_command(line) == Listen(8, "confirm", "es", "¿Cierro la calculadora?")


def test_an_old_style_listen_has_no_language_and_no_context():
    command = parse_command('{"type":"listen","id":8,"mode":"confirm"}')
    assert (command.lang, command.context) == (None, None)
    command = parse_command('{"type":"listen","id":8,"mode":"confirm","lang":null,"context":null}')
    assert (command.lang, command.context) == (None, None)


@pytest.mark.parametrize(
    "line",
    [
        '{"type":"listen","id":8,"mode":"confirm","lang":"fr"}',
        '{"type":"listen","id":8,"mode":"confirm","lang":7}',
        '{"type":"listen","id":8,"mode":"confirm","context":7}',
    ],
)
def test_a_listen_with_a_bad_language_or_context_is_a_protocol_error(line):
    with pytest.raises(ProtocolError):
        parse_command(line)


def test_a_very_long_context_is_cut_to_its_last_characters():
    long_text = "x" * 500 + "¿Cierro la calculadora?"
    command = parse_command(json.dumps({"type": "listen", "id": 1, "mode": "confirm", "context": long_text}))
    assert len(command.context) == protocol.MAX_CONTEXT_CHARS
    assert command.context.endswith("¿Cierro la calculadora?")


def test_ids_are_echoed_as_large_unsigned_integers():
    big = 2**63 + 5
    assert parse_command(json.dumps({"type": "listen", "id": big, "mode": "command"})).id == big
    assert protocol.no_speech(big)["id"] == big


@pytest.mark.parametrize(
    "line",
    [
        "not json",
        "[]",
        '{"id":1}',
        '{"type":"dance"}',
        '{"type":"listen","id":-1,"mode":"command"}',
        '{"type":"listen","id":true,"mode":"command"}',
        '{"type":"listen","id":"7","mode":"command"}',
        '{"type":"listen","id":7,"mode":"whisper"}',
        '{"type":"speak","id":1,"text":3,"lang":"es"}',
        '{"type":"speak","id":1,"text":"hi","lang":"fr"}',
        '{"type":"cancel_listen"}',
    ],
)
def test_rejects_unreadable_lines(line):
    with pytest.raises(ProtocolError):
        parse_command(line)


def test_event_shapes_match_the_design():
    assert protocol.wake(0.71234) == {"type": "wake", "score": 0.712}
    assert protocol.speech_start(3) == {"type": "speech_start", "id": 3}
    assert protocol.transcript(3, "abrí Spotify", "es", 0.98765, 1234) == {
        "type": "transcript",
        "id": 3,
        "text": "abrí Spotify",
        "lang": "es",
        "lang_prob": 0.988,
        "end_ms": 1234,
    }
    assert protocol.speak_started(4) == {"type": "speak_started", "id": 4}
    assert protocol.speak_done(4) == {"type": "speak_done", "id": 4}
    assert protocol.ready(5) == {"type": "ready", "now_ms": 5}


def test_error_carries_an_id_only_when_given():
    assert "id" not in protocol.error(ErrorCode.MIC_BUSY, "x")
    assert protocol.error(ErrorCode.TTS_FAILED, "x", 9)["id"] == 9


def test_encode_is_one_utf8_line_without_newline():
    line = protocol.encode(protocol.transcript(1, "ñandú", "es", 1.0, 0))
    assert "\n" not in line
    assert "ñandú" in line


def test_error_codes_match_the_taxonomy():
    assert {c.value for c in ErrorCode} == {
        "MIC_MISSING",
        "MIC_BUSY",
        "CUDA_OOM",
        "MODEL_LOAD",
        "TTS_FAILED",
        "PROTOCOL",
        "INTERNAL",
    }


def test_error_codes_match_the_reducer_wire_names():
    """The host maps these strings to `SidecarErrorCode`; they must not drift."""
    import re
    from pathlib import Path

    source = Path(__file__).resolve().parents[2] / "crates" / "niva-core" / "src" / "state_machine.rs"
    if not source.exists():
        pytest.skip("Rust sources not present")
    wire = set(re.findall(r'=> "([A-Z_]+)",', source.read_text(encoding="utf-8")))
    reducer_codes = {"MIC_MISSING", "MIC_BUSY", "CUDA_OOM", "MODEL_LOAD", "PROTOCOL", "INTERNAL"}
    assert reducer_codes <= wire
    assert reducer_codes <= {c.value for c in ErrorCode}


# ------------------------------------------------------------------- level


def test_level_message_shape():
    msg = protocol.level([0, 10, 20, 30, 40, 50, 100])
    assert msg == {"type": "level", "src": "tts", "bands": [0, 10, 20, 30, 40, 50, 100]}
    assert json.loads(protocol.encode(msg)) == msg


def test_level_message_has_no_samples_or_text():
    msg = protocol.level([1] * 7)
    assert set(msg) == {"type", "src", "bands"}
    assert all(type(v) is int for v in msg["bands"])
    assert len(protocol.encode(msg)) < 80  # seven small integers, never audio


@pytest.mark.parametrize("src", ["mic", "tts2", ""])
def test_level_message_rejects_mic_src(src):
    with pytest.raises(ValueError):
        protocol.level([0] * 7, src=src)


@pytest.mark.parametrize("bad", [[0] * 6, [0] * 8, [0] * 6 + [101], [0] * 6 + [-1], [0] * 6 + [1.5]])
def test_level_message_rejects_a_malformed_band_list(bad):
    with pytest.raises(ValueError):
        protocol.level(bad)
