"""The capture loop, frame by frame, with fakes."""

import logging

import pytest
from conftest import SILENCE, SOFT, SPEECH, WAKE_END, WAKE_WORD, Harness, frame, frames, levels

from capture import SidecarFatal
from config import FRAME_MS, Tunables
from protocol import CancelListen, ErrorCode, ForceEnd, Listen, Speak, StopSpeaking
from stt import SttError

# ------------------------------------------------------------------ one turn


def test_listen_after_silence_transcribes_and_echoes_the_id(h):
    h.feed(frames(SILENCE, 20))
    h.engine.on_command(Listen(41, "command"))
    h.feed(frames(SPEECH, 15) + frames(SILENCE, 25))
    assert h.types() == ["speech_start", "transcript"]
    assert h.events[0]["id"] == 41
    t = h.events[1]
    assert (t["id"], t["text"], t["lang"], t["lang_prob"]) == (41, "open spotify", "en", 0.99)
    assert not h.engine.listening


def test_a_confirm_listen_hands_its_language_and_question_to_the_transcriber(h):
    h.engine.on_command(Listen(5, "confirm", "es", "¿Cierro la calculadora?"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert h.stt.calls == [{"mode": "confirm", "lang": "es", "context": "¿Cierro la calculadora?"}]


def test_a_command_listen_hands_no_language_and_no_question(h):
    h.engine.on_command(Listen(5, "command", "es", "¿Cierro la calculadora?"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert h.stt.calls == [{"mode": "command", "lang": None, "context": None}]


def test_an_old_style_confirm_listen_still_works(h):
    h.engine.on_command(Listen(5, "confirm"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert h.stt.calls == [{"mode": "confirm", "lang": None, "context": None}]
    assert h.types() == ["speech_start", "transcript"]


def test_a_hallucination_the_gate_emptied_is_an_empty_transcript_with_its_own_reason(h):
    h.stt.text, h.stt.dropped = "", 1
    h.engine.on_command(Listen(5, "confirm", "en", "Should I close the calculator?"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert h.events[-1]["type"] == "transcript" and h.events[-1]["text"] == ""
    assert h.discards.counts["hallucination_gate"] == 1
    assert h.discards.counts["empty_transcript"] == 1


def test_end_ms_is_the_end_of_speech_not_the_decision_time(h):
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SPEECH, 10))
    speech_end = h.now
    while h.types()[-1:] != ["transcript"]:
        h.feed(frames(SILENCE, 1))
    end_ms = h.events[-1]["end_ms"]
    assert end_ms == speech_end
    assert h.now - end_ms == pytest.approx(500, abs=64)  # the decision comes about 500 ms later


def test_trailing_silence_sent_to_stt_is_trimmed_to_the_pad(h):
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 30))
    audio_levels = levels(h.stt.heard[0])
    trailing = len(audio_levels) - 1 - max(i for i, v in enumerate(audio_levels) if v == SPEECH)
    assert trailing == h.tunables.trailing_pad_frames


# -------------------------------------------------- audio before speech onset


def test_first_soft_syllable_is_kept(h):
    """The VAD confirms speech late; the audio before it still reaches STT."""
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SILENCE, 5) + frames(SOFT, 3) + frames(SPEECH, 12) + frames(SILENCE, 25))
    heard = levels(h.stt.heard[0])
    first_speech = heard.index(SPEECH)
    assert heard[first_speech - 3 : first_speech] == [SOFT, SOFT, SOFT]
    assert first_speech > 3  # and the audio starts before the soft syllable, not at it


def test_audio_before_onset_is_capped_by_the_named_tunable(h):
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SILENCE, 100) + frames(SPEECH, 10) + frames(SILENCE, 25))
    heard = levels(h.stt.heard[0])
    lead = heard.index(SPEECH)
    assert lead == h.tunables.pre_speech_frames  # 300 ms, not three seconds of waiting
    assert lead * FRAME_MS >= 300


def test_pre_speech_buffer_is_overwritten_as_it_rolls(h):
    """Only the most recent frames are kept: old audio cannot leak into a later turn."""
    h.feed([frame(1 + i) for i in range(150)])  # distinct levels, all below the soft level
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    heard = levels(h.stt.heard[0])
    before = [v for v in heard[: heard.index(SPEECH)]]
    assert min(before) >= 150 - h.tunables.pre_speech_frames


def test_a_turn_does_not_reuse_audio_from_the_previous_turn(h):
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    h.events.clear()
    h.engine.on_command(Listen(2, "confirm"))
    h.feed(frames(SILENCE, 10))
    assert h.events == []  # nothing replayed as speech


# ------------------------------------------------------ one breath after wake


def test_one_breath_hey_niva_open_spotify_keeps_the_whole_command(h):
    h.feed(frames(SILENCE, 30) + frames(WAKE_WORD, 14) + [frame(WAKE_END)])
    assert h.types() == ["wake"]
    command = [frame(SPEECH + i) for i in range(30)]  # distinct levels: count them
    h.feed(command)  # the user keeps talking before the host answers
    h.engine.on_command(Listen(5, "command"))
    h.feed(frames(SILENCE, 25))
    assert h.types() == ["wake", "speech_start", "transcript"]
    heard = levels(h.stt.heard[0])
    assert all(SPEECH + i in heard for i in range(30)), "words were lost after the wake word"
    assert h.events[-1]["id"] == 5


def test_listen_arriving_right_at_the_wake_detection(h):
    h.feed(frames(SILENCE, 30) + frames(WAKE_WORD, 14) + [frame(WAKE_END)])
    h.engine.on_command(Listen(6, "command"))
    h.feed([frame(SPEECH + i) for i in range(30)] + frames(SILENCE, 25))
    heard = levels(h.stt.heard[0])
    assert all(SPEECH + i in heard for i in range(30))


def test_command_that_starts_before_the_wake_detection_is_not_cut(h):
    """The detector fires a little after the phrase ends: the first syllable of a
    command said in one breath already went by. The wake pre-roll keeps it."""
    h.feed(frames(SILENCE, 30) + frames(WAKE_WORD, 10))
    early = [frame(SPEECH + i) for i in range(3)]  # command starts before the detection
    h.feed(early + [frame(WAKE_END)])
    h.feed([frame(SPEECH + 3 + i) for i in range(20)])
    h.engine.on_command(Listen(6, "command"))
    h.feed(frames(SILENCE, 25))
    heard = levels(h.stt.heard[0])
    assert all(SPEECH + i in heard for i in range(23))


def test_wake_preroll_is_bounded_so_little_of_the_wake_word_leaks_in(h):
    h.feed(frames(SILENCE, 30) + frames(WAKE_WORD, 14) + [frame(WAKE_END)])
    h.engine.on_command(Listen(6, "command"))
    # A real command lasts longer than the wake tail window (wake_tail_ms).
    h.feed(frames(SPEECH, 20) + frames(SILENCE, 25))
    heard = levels(h.stt.heard[0])
    wake_frames = sum(1 for v in heard if v in (WAKE_WORD, WAKE_END))
    assert wake_frames <= h.tunables.wake_preroll_frames


def test_one_wake_event_per_phrase_and_the_detector_is_reset(h):
    h.feed(frames(WAKE_WORD, 14) + [frame(WAKE_END)])
    h.feed(frames(SILENCE, 5))
    assert h.types() == ["wake"]
    assert h.wake.resets >= 2  # on mic ready and after the detection


def test_unclaimed_wake_is_dropped_and_logged(h):
    h.feed([frame(WAKE_END)])
    h.feed(frames(SILENCE, 3000 // FRAME_MS + 5))
    assert h.discards.counts["wake_unclaimed"] == 1
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SILENCE, 5))
    assert h.types() == ["wake"]


# ----------------------------------------------------------- wake ignored


def test_wake_is_ignored_while_speaking_and_the_reason_is_logged(h, caplog):
    h.engine.on_command(Speak(9, "Listo.", "es"))
    assert h.speaker.spoken == [(9, "Listo.", "es")]
    with caplog.at_level(logging.INFO, logger="niva.voice.discard"):
        h.feed([frame(WAKE_END)])
    assert h.events == []
    assert h.discards.counts["wake_while_speaking"] == 1
    assert [r.getMessage() for r in caplog.records] == ["discard reason=wake_while_speaking score=0.90"]


def test_wake_is_ignored_while_a_listen_is_open(h):
    h.engine.on_command(Listen(1, "command"))
    h.feed([frame(WAKE_END)])
    assert h.events == []
    assert h.discards.counts["wake_while_listening"] == 1


def test_speech_end_forgets_what_the_speaker_played(h):
    h.engine.on_command(Speak(9, "Listo.", "es"))
    h.feed(frames(SPEECH, 12))  # the reply, picked up by the microphone
    h.engine.on_speak_ended(9)
    assert not h.engine.speaking
    h.engine.on_command(Listen(10, "command"))
    h.feed(frames(SILENCE, 10))
    assert h.events == []  # her own voice did not become an utterance
    assert h.wake.resets >= 2


def test_a_late_speak_end_of_an_old_utterance_changes_nothing(h):
    h.engine.on_command(Speak(1, "a", "en"))
    h.engine.on_command(Speak(2, "b", "en"))
    assert h.speaker.stops == 1
    h.engine.on_speak_ended(1)
    assert h.engine.speaking  # still speaking id 2


def test_stop_speaking_stops_the_speaker_and_wake_works_again(h):
    h.engine.on_command(Speak(1, "a", "en"))
    h.engine.on_command(StopSpeaking())
    assert h.speaker.stops == 1
    h.feed([frame(WAKE_END)])
    assert h.types() == ["wake"]


def test_near_miss_scores_are_logged_with_the_peak_only(h, caplog):
    with caplog.at_level(logging.INFO, logger="niva.voice.discard"):
        for score in (0.32, 0.44, 0.38, 0.1):
            h.wake.forced = score
            h.feed([frame(SILENCE)])
    assert h.events == []
    assert h.discards.counts["wake_near_miss"] == 1
    assert [r.getMessage() for r in caplog.records] == ["discard reason=wake_near_miss score=0.44"]


# -------------------------------------------------------------- no speech


def test_no_speech_after_the_listen_window(h):
    h.engine.on_command(Listen(3, "confirm"))
    h.feed(frames(SILENCE, int(6000 / FRAME_MS) + 2))
    assert h.events == [{"type": "no_speech", "id": 3}]
    assert h.discards.counts["no_speech"] == 1
    assert not h.engine.listening


def test_empty_transcript_is_sent_with_its_id_and_logged_without_text(h, caplog):
    h.stt.text = "   "
    h.engine.on_command(Listen(8, "command"))
    with caplog.at_level(logging.INFO, logger="niva.voice.discard"):
        h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    transcript = h.events[-1]
    assert (transcript["type"], transcript["id"], transcript["text"]) == ("transcript", 8, "")
    assert h.discards.counts["empty_transcript"] == 1
    assert [r.getMessage() for r in caplog.records] == ["discard reason=empty_transcript"]


# ------------------------------------------------------- cancel and force


def test_cancel_listen_drops_the_utterance_silently(h):
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SPEECH, 5))
    h.events.clear()
    h.engine.on_command(CancelListen(1))
    h.feed(frames(SPEECH, 5) + frames(SILENCE, 25))
    assert h.events == []
    assert h.discards.counts["stale_listen"] == 1


def test_cancel_for_another_id_is_ignored(h):
    h.engine.on_command(Listen(2, "command"))
    h.engine.on_command(CancelListen(1))
    assert h.engine.listening
    assert h.discards.counts["stale_cancel"] == 1


def test_a_new_listen_replaces_the_old_one(h):
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SPEECH, 5))
    h.engine.on_command(Listen(2, "command"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert h.types() == ["speech_start", "speech_start", "transcript"]
    assert [e["id"] for e in h.events] == [1, 2, 2]  # the first listen is over, the second owns the rest
    assert h.discards.counts["stale_listen"] == 1


def test_force_end_during_speech_sends_what_was_said(h):
    h.engine.on_command(Listen(4, "command"))
    h.feed(frames(SPEECH, 20))
    h.engine.on_command(ForceEnd(4))
    assert h.types() == ["speech_start", "transcript"]
    assert h.events[-1]["id"] == 4
    assert h.events[-1]["end_ms"] == h.now


def test_force_end_before_any_speech_is_no_speech(h):
    h.engine.on_command(Listen(4, "command"))
    h.feed(frames(SILENCE, 5))
    h.engine.on_command(ForceEnd(4))
    assert h.events == [{"type": "no_speech", "id": 4}]


def test_force_end_for_a_stale_id_is_ignored(h):
    h.engine.on_command(Listen(4, "command"))
    h.feed(frames(SPEECH, 5))
    h.engine.on_command(ForceEnd(3))
    assert h.engine.listening


def test_utterance_backstop_ends_a_runaway_capture():
    h = Harness(Tunables(max_utterance_s=1.0)).start()
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SPEECH, 40))
    assert h.types() == ["speech_start", "transcript"]


# ----------------------------------------------------------- failures


def test_stt_out_of_memory_is_fatal_and_carries_the_listen_id(h):
    h.stt.fail = SttError(ErrorCode.CUDA_OOM, "out of memory")
    h.engine.on_command(Listen(12, "command"))
    with pytest.raises(SidecarFatal) as info:
        h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert (info.value.code, info.value.message_id) == (ErrorCode.CUDA_OOM, 12)


def test_runtime_oom_is_recognised_and_unknown_failures_are_internal(h):
    h.stt.fail = RuntimeError("CUDA failed with error out of memory")
    h.engine.on_command(Listen(1, "command"))
    with pytest.raises(SidecarFatal) as info:
        h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert info.value.code == ErrorCode.CUDA_OOM
    h2 = Harness().start()
    h2.stt.fail = ValueError("boom")
    h2.engine.on_command(Listen(1, "command"))
    with pytest.raises(SidecarFatal) as info2:
        h2.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert info2.value.code == ErrorCode.INTERNAL


def test_mic_failure_reports_a_mic_error_with_the_listen_id_and_ends_the_listen(h):
    h.engine.on_command(Listen(21, "command"))
    h.feed(frames(SPEECH, 4))
    h.events.clear()
    h.engine.on_mic_error(ErrorCode.MIC_MISSING, "unplugged")
    assert h.events == [{"type": "error", "code": "MIC_MISSING", "msg": "unplugged", "id": 21}]
    assert not h.engine.listening


def test_listen_while_the_mic_is_down_answers_with_the_error_instead_of_hanging(h):
    h.engine.on_mic_error(ErrorCode.MIC_BUSY, "in use")
    h.events.clear()
    h.engine.on_command(Listen(22, "command"))
    assert h.events == [{"type": "error", "code": "MIC_BUSY", "msg": "in use", "id": 22}]
    assert h.discards.counts["listen_mic_down"] == 1


def test_mic_recovery_sends_ready_again_and_listening_works(h):
    h.engine.on_mic_error(ErrorCode.MIC_BUSY, "in use")
    h.events.clear()
    h.engine.on_mic_ok()
    h.feed(frames(SILENCE, 1))
    assert h.types() == ["ready"]
    assert h.events[0]["now_ms"] == h.now
    h.engine.on_command(Listen(23, "command"))
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert h.types() == ["ready", "speech_start", "transcript"]


# ------------------------------------------------- review regressions (C1, C2, S5)


def test_listen_arriving_before_speak_ended_does_not_hear_the_tail_of_the_reply(h):
    """C1: the reducer listens right at `speak_done`; the reply's own tail is still in the buffer."""
    h.engine.on_command(Speak(9, "Quieres cerrarlo?", "es"))
    h.feed(frames(SPEECH, 12))  # her own voice, picked up by the microphone
    h.engine.on_command(Listen(10, "confirm"))  # before `speak_ended` reached the loop
    h.feed(frames(SILENCE, 10))
    assert h.events == []  # she did not become the answer
    assert not h.engine.speaking
    h.engine.on_speak_ended(9)  # the late notice changes nothing
    h.feed(frames(SPEECH, 10) + frames(SILENCE, 25))
    assert h.types() == ["speech_start", "transcript"]
    assert levels(h.stt.heard[0]).count(SPEECH) == 10  # only what the user said


def test_wake_pause_command_is_not_ended_by_the_wake_word_tail(h):
    """C2: speech found only in the replayed pre-roll is provisional."""
    h.feed(frames(SILENCE, 30) + frames(WAKE_WORD, 14) + [frame(WAKE_END)])
    h.engine.on_command(Listen(5, "command"))
    assert h.types() == ["wake"]  # no speech_start from the replay
    h.feed(frames(SILENCE, 25))  # a pause longer than the 500 ms wait
    assert h.types() == ["wake"]  # no transcript, no no_speech: the turn is still open
    assert h.engine.listening
    command = [frame(SPEECH + i) for i in range(20)]
    h.feed(command + frames(SILENCE, 25))
    assert h.types() == ["wake", "speech_start", "transcript"]
    heard = levels(h.stt.heard[0])
    assert all(SPEECH + i in heard for i in range(20))
    assert WAKE_WORD not in heard and WAKE_END not in heard  # the wake word did not reach STT


def test_wake_followed_only_by_silence_still_ends_with_no_speech_at_the_normal_time(h):
    h.feed(frames(WAKE_WORD, 14) + [frame(WAKE_END)])
    h.engine.on_command(Listen(5, "command"))
    h.feed(frames(SILENCE, int(6000 / FRAME_MS) - 2))
    assert h.types() == ["wake"]
    h.feed(frames(SILENCE, 4))
    assert h.types() == ["wake", "no_speech"]


def test_wake_word_tail_heard_after_the_detection_does_not_start_the_turn(h):
    """Live bug: the detector fires before the word ends, so "...vis" is heard
    after the detection. A pause then must not end the turn on that tail."""
    h.feed(frames(SILENCE, 30) + frames(WAKE_WORD, 8) + [frame(WAKE_END)])
    h.feed(frames(WAKE_WORD, 3))  # the rest of the word, before the host's listen
    h.engine.on_command(Listen(7, "command"))
    h.feed(frames(WAKE_WORD, 5))  # ...and a bit more after it (~256 ms of tail in all)
    h.feed(frames(SILENCE, 28))  # the user waits ~900 ms for the island
    assert h.types() == ["wake"]  # the pause did not end a turn made of the tail
    assert h.engine.listening
    command = [frame(SPEECH + i) for i in range(20)]
    h.feed(command + frames(SILENCE, 25))
    assert h.types() == ["wake", "speech_start", "transcript"]
    assert len(h.stt.heard) == 1
    heard = levels(h.stt.heard[0])
    assert all(SPEECH + i in heard for i in range(20))
    assert WAKE_WORD not in heard and WAKE_END not in heard


def test_wake_tail_then_only_silence_still_ends_with_no_speech_in_bounded_time(h):
    h.feed(frames(WAKE_WORD, 8) + [frame(WAKE_END)] + frames(WAKE_WORD, 6))
    h.engine.on_command(Listen(8, "command"))
    h.feed(frames(WAKE_WORD, 4))
    h.feed(frames(SILENCE, int(6000 / FRAME_MS)))
    assert h.types() == ["wake", "no_speech"]
    assert h.stt.heard == []
    assert not h.engine.listening


def test_replacing_a_listen_does_not_replay_the_old_turns_speech(h):
    """S5: the first turn's speech sits in the buffer; the new listen must not take it."""
    h.engine.on_command(Listen(1, "command"))
    h.feed(frames(SPEECH, 5))
    h.engine.on_command(Listen(2, "command"))
    h.feed(frames(SILENCE, 25))
    assert h.types() == ["speech_start"]  # only listen 1's own onset
    assert h.events[0]["id"] == 1
    assert h.engine.listening


# ------------------------------------------------- review regressions (W3, W4)


def test_ready_waits_for_the_first_real_frame(h):
    h.engine.on_mic_ok()
    assert h.events == []
    h.feed(frames(SILENCE, 1))
    assert h.types() == ["ready"]
    h.feed(frames(SILENCE, 3))
    assert h.types() == ["ready"]


def test_an_identical_mic_error_is_not_repeated(h):
    h.engine.on_mic_error(ErrorCode.MIC_MISSING, "stalled")
    h.engine.on_mic_ok()  # reopened, but no audio yet
    h.engine.on_mic_error(ErrorCode.MIC_MISSING, "stalled")
    assert h.types() == ["error"]
    h.engine.on_mic_error(ErrorCode.MIC_BUSY, "in use")  # a different failure is news
    assert h.types() == ["error", "error"]


def test_end_ms_comes_from_the_frame_stamps_not_from_when_the_loop_gets_to_them(h):
    """W4: a backlog after an STT call must not move the end of speech."""
    h.engine.on_command(Listen(1, "command"))
    stamp = 5_000
    for item in frames(SPEECH, 10):
        stamp += 32
        h.engine.on_frame(item, stamp)
    last_speech = stamp
    h.now = 9_000_000  # the loop is late: the processing clock is far ahead
    while h.types()[-1:] != ["transcript"]:
        stamp += 32
        h.engine.on_frame(frame(SILENCE), stamp)
    assert h.events[-1]["end_ms"] == last_speech


# ------------------------------------------------- second review (W1, S3)


def test_end_ms_of_a_late_listen_comes_from_the_replayed_frames_own_stamp(h):
    """The whole command sits in the recording when `listen` finally arrives."""
    h.feed(frames(SILENCE, 5) + frames(WAKE_WORD, 6) + [frame(WAKE_END)])
    h.feed(frames(SPEECH, 20))  # longer than the wake tail window (wake_tail_ms)
    last_speech = h.now
    h.feed(frames(SILENCE, 30))
    h.now += 2000  # the host answers about two seconds late
    h.engine.on_command(Listen(5, "command"))
    assert h.types() == ["wake", "speech_start", "transcript"]
    assert h.events[-1]["end_ms"] == last_speech


def test_one_frame_over_the_threshold_does_not_stop_the_no_speech_clock(h):
    h.engine.on_command(Listen(3, "command"))
    h.feed(frames(SPEECH, 1))  # a click, not speech: the onset needs two frames
    h.feed(frames(SILENCE, int(6000 / FRAME_MS) + 2))
    assert h.events == [{"type": "no_speech", "id": 3}]
