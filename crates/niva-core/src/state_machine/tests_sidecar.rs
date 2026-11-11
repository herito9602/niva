//! Task 3.2 (CO3): the sidecar `error{code}` event. Voice goes down, the turn
//! in a listen ends with a template that says what happened, and everything
//! that does not need the microphone (a confirmation by click, the sleep
//! countdown, a running action, an alert) carries on untouched.

use super::tests::*;
use super::*;

const CODE: SidecarErrorCode = SidecarErrorCode::MicBusy;

fn err(code: SidecarErrorCode) -> Event {
    Event::SidecarError { code, id: None }
}

fn err_for(code: SidecarErrorCode, id: u64) -> Event {
    Event::SidecarError { code, id: Some(id) }
}

fn voice_changed(ready: bool) -> Event {
    Event::VoiceReadinessChanged { ready }
}

/// `state` after an error that only marks voice down: everything else equal.
fn only_voice_down(state: &State) -> State {
    let mut expected = state.clone();
    expected.voice_available = false;
    expected
}

fn run(mut state: State, events: Vec<Event>) -> (State, Vec<Effect>) {
    let mut all = Vec::new();
    for event in events {
        let context = format!("{event:?}");
        let (next, fx) = reduce(state, event);
        assert_invariants(&next, &context);
        state = next;
        all.extend(fx);
    }
    (state, all)
}

// --- The codes -------------------------------------------------------------------

#[test]
fn the_six_unusable_codes_use_the_protocol_spelling() {
    let wire: Vec<&str> = SidecarErrorCode::ALL.iter().map(|c| c.wire()).collect();
    assert_eq!(
        wire,
        [
            "MIC_MISSING",
            "MIC_BUSY",
            "CUDA_OOM",
            "MODEL_LOAD",
            "PROTOCOL",
            "INTERNAL"
        ]
    );
    for code in SidecarErrorCode::ALL {
        assert_eq!(SidecarErrorCode::from_wire(code.wire()), Some(code));
    }
}

#[test]
fn tts_failed_and_unknown_codes_are_not_sidecar_errors() {
    // TTS_FAILED is the per-speak `TtsFailed` event, which never takes voice down.
    assert_eq!(SidecarErrorCode::from_wire("TTS_FAILED"), None);
    assert_eq!(SidecarErrorCode::from_wire("SOMETHING_ELSE"), None);
    assert_eq!(SidecarErrorCode::from_wire("mic_busy"), None);
    assert_eq!(SidecarErrorCode::from_wire(""), None);
}

#[test]
fn every_code_marks_voice_down_in_every_phase() {
    for code in SidecarErrorCode::ALL {
        for phase in Phase::ALL {
            let (s, _) = step(&state_in(phase), err(code));
            assert!(!s.voice_available, "{code:?} in {phase:?}");
        }
    }
}

#[test]
fn tts_failed_never_takes_voice_down() {
    let cases = [
        (Phase::Success, ReplyKind::Final),
        (Phase::Error, ReplyKind::Final),
        (Phase::ConfirmationNeeded, ReplyKind::ConfirmPrompt),
        (Phase::Thinking, ReplyKind::AskFollowUp),
        (Phase::ReminderAlert, ReplyKind::Alert),
        (Phase::Countdown, ReplyKind::Countdown),
    ];
    for (phase, kind) in cases {
        let (s, _) = step(
            &speaking(state_in(phase), SPEAK_ID, kind),
            Event::TtsFailed { id: SPEAK_ID },
        );
        assert!(s.voice_available, "{phase:?}");
    }
}

// --- Listening ---------------------------------------------------------------------

#[test]
fn an_error_while_listening_ends_the_turn_with_the_code_template() {
    for code in SidecarErrorCode::ALL {
        let (s, fx) = step(&state_in(Phase::Listening), err(code));
        assert_eq!(s.phase, Phase::Error, "{code:?}");
        assert!(!s.voice_available);
        assert_eq!(s.listen_id, None);
        assert_eq!(
            fx,
            vec![
                Effect::CancelTimer {
                    kind: TimerKind::NoSpeech
                },
                Effect::CancelListen { id: LISTEN_ID },
                speak_final(NEXT_ID, tmpl(Template::Sidecar(code))),
            ],
            "{code:?}"
        );
    }
}

#[test]
fn the_template_follows_the_code_not_a_generic_error() {
    let (_, a) = step(
        &state_in(Phase::Listening),
        err(SidecarErrorCode::MicMissing),
    );
    let (_, b) = step(&state_in(Phase::Listening), err(SidecarErrorCode::CudaOom));
    assert_ne!(a, b);
    assert!(!a.contains(&speak_final(NEXT_ID, tmpl(Template::Error))));
}

#[test]
fn an_error_in_any_part_of_the_listen_ends_it() {
    for utterance in [
        Utterance::WaitingForSpeech,
        Utterance::Speaking,
        Utterance::Ending,
    ] {
        let mut listening = state_in(Phase::Listening);
        listening.utterance = utterance;
        listening.timers = running(&[TimerKind::MaxUtterance, TimerKind::SttTimeout]);
        let (s, fx) = step(&listening, err(CODE));
        assert_eq!(s.phase, Phase::Error, "{utterance:?}");
        assert!(s.timers.is_empty());
        assert!(fx.contains(&Effect::CancelListen { id: LISTEN_ID }));
        assert!(
            !fx.contains(&Effect::HideIsland),
            "the text must be readable"
        );
    }
}

#[test]
fn an_error_in_a_follow_up_listen_clears_the_pending_slot() {
    let mut listening = state_in(Phase::Listening);
    listening.pending = Some(slot());
    let (s, _) = step(&listening, err(CODE));
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.pending, None);
}

#[test]
fn the_error_text_settles_to_idle_like_any_final_reply() {
    let (s, _) = step(&state_in(Phase::Listening), err(CODE));
    let (s, fx) = step(&s, Event::SpeakDone { id: NEXT_ID });
    assert_eq!(fx, vec![start(TimerKind::Settle)]);
    let (s, _) = step(&s, fire(&s, TimerKind::Settle));
    assert_eq!(s.phase, Phase::Idle);
}

#[test]
fn with_voice_down_the_next_activation_is_voice_not_ready() {
    let (s, _) = run(
        state_in(Phase::Listening),
        vec![err(CODE), Event::SpeakDone { id: NEXT_ID }],
    );
    let (s, _) = step(&s, Event::Shortcut);
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.listen_id, None, "never a listen to a dead sidecar");
}

// --- Confirmation ------------------------------------------------------------------

#[test]
fn an_error_during_a_confirmation_leaves_it_pending_and_click_only() {
    for code in SidecarErrorCode::ALL {
        let mut start = state_in(Phase::ConfirmationNeeded);
        start.cancel_text = Some("Cancelled closing notepad.".into());
        let (s, fx) = step(&start, err(code));
        assert!(fx.is_empty(), "{code:?}: nothing to cancel, nothing to say");
        assert_eq!(s.phase, Phase::ConfirmationNeeded);
        assert!(s.confirm_click_only);
        assert_eq!(s.listen_id, None);
        assert!(!s.voice_available);
        assert_eq!(s.request_id, Some(REQ_ID), "not cancelled");
        assert_eq!(s.confirm_id, Some(CONFIRM_ID), "same prompt");
        assert_eq!(s.timers, start.timers, "the 10 s timer keeps running");
        assert_eq!(s.cancel_text, start.cancel_text);
    }
}

#[test]
fn an_error_never_answers_the_confirmation() {
    let (s, fx) = step(&state_in(Phase::ConfirmationNeeded), err(CODE));
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::RunTool { .. } | Effect::Speak { .. }))
    );
    assert_eq!(s.phase, Phase::ConfirmationNeeded);
}

#[test]
fn after_an_error_a_spoken_answer_is_ignored_for_good() {
    let (s, _) = step(&state_in(Phase::ConfirmationNeeded), err(CODE));
    for (id, text) in [
        (CONFIRM_LISTEN_ID, "yes"),
        (CONFIRM_LISTEN_ID, "no"),
        (NEXT_ID, "yes"),
    ] {
        assert_ignored(
            s.clone(),
            Event::Transcript {
                id,
                text: text.into(),
                lang: None,
            },
        );
    }
    assert_ignored(
        s.clone(),
        Event::EmptyTranscript {
            id: CONFIRM_LISTEN_ID,
        },
    );
    assert_ignored(
        s,
        Event::NoSpeech {
            id: CONFIRM_LISTEN_ID,
        },
    );
}

#[test]
fn after_an_error_the_click_still_accepts_or_rejects() {
    let (s, _) = step(&state_in(Phase::ConfirmationNeeded), err(CODE));
    let (accepted, fx) = step(&s, Event::Accept { id: CONFIRM_ID });
    assert_eq!(accepted.phase, Phase::Acting);
    assert!(fx.contains(&Effect::RunTool { req: REQ_ID }));

    let (rejected, _) = step(&s, Event::Reject { id: CONFIRM_ID });
    assert_eq!(rejected.phase, Phase::Success);
    assert_eq!(rejected.request_id, None);
}

#[test]
fn after_an_error_the_ten_second_timer_still_rejects_and_the_shortcut_cancels() {
    let (s, _) = step(&state_in(Phase::ConfirmationNeeded), err(CODE));
    let (timeout, _) = step(&s, fire(&s, TimerKind::Confirm));
    assert_eq!(timeout.phase, Phase::Success, "the cancelled reply");
    let (shortcut, _) = step(&s, Event::Shortcut);
    assert_eq!(shortcut.phase, Phase::Success);
}

#[test]
fn an_error_while_the_prompt_plays_means_no_confirm_listen_afterwards() {
    let (s, fx) = step(&prompt_playing(), err(CODE));
    assert!(fx.is_empty(), "the prompt keeps playing");
    assert!(s.confirm_click_only);
    let (after, fx) = step(&s, Event::SpeakDone { id: SPEAK_ID });
    assert_eq!(after.listen_id, None);
    assert_eq!(fx, vec![start(TimerKind::Confirm)], "only the 10 s timer");
}

#[test]
fn a_confirmation_stays_click_only_when_voice_returns() {
    let (s, _) = run(
        state_in(Phase::ConfirmationNeeded),
        vec![err(CODE), voice_changed(true)],
    );
    assert!(s.voice_available);
    assert!(s.confirm_click_only);
    assert_eq!(s.listen_id, None);
}

// --- Sleep countdown -----------------------------------------------------------------

#[test]
fn an_error_in_the_countdown_only_marks_voice_down() {
    for start in [state_in(Phase::Countdown), countdown_unarmed()] {
        let (s, fx) = step(&start, err(CODE));
        assert_eq!(s, only_voice_down(&start));
        assert!(fx.is_empty(), "{fx:?}");
        assert!(
            !fx.iter()
                .any(|e| matches!(e, Effect::FinishCountdown { .. })),
            "the sleep must not start early"
        );
    }
}

#[test]
fn after_an_error_the_countdown_is_still_cancellable_by_input_and_shortcut() {
    let (s, _) = step(&state_in(Phase::Countdown), err(CODE));
    for cancel in [Event::CountdownCancel { id: COUNTDOWN_ID }, Event::Shortcut] {
        let (after, fx) = step(&s, cancel);
        assert_eq!(after.phase, Phase::Success);
        assert!(fx.contains(&Effect::StopWatchingInput));
        assert!(fx.contains(&Effect::CancelRequest { req: REQ_ID }));
        assert!(
            !fx.iter()
                .any(|e| matches!(e, Effect::FinishCountdown { .. }))
        );
    }
}

#[test]
fn after_an_error_an_unwatched_countdown_still_fails_closed() {
    let (s, _) = step(&countdown_unarmed(), err(CODE));
    let (fault, fx) = step(&s, Event::CountdownFault { id: COUNTDOWN_ID });
    assert_eq!(fault.phase, Phase::Error);
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::FinishCountdown { .. }))
    );

    let (guard, fx) = step(&s, fire(&s, TimerKind::Tool));
    assert_eq!(guard.phase, Phase::Error, "the 5 s guard is still armed");
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::FinishCountdown { .. }))
    );
    assert_eq!(guard.request_id, None);
}

#[test]
fn after_an_error_an_uncancelled_countdown_runs_its_deadline_as_before() {
    let (s, _) = step(&state_in(Phase::Countdown), err(CODE));
    let (after, fx) = step(&s, fire(&s, TimerKind::Countdown));
    assert_eq!(after.phase, Phase::Acting);
    assert!(fx.contains(&Effect::FinishCountdown { req: REQ_ID }));
}

#[test]
fn an_error_never_arms_or_restarts_the_countdown() {
    let (s, _) = step(&countdown_unarmed(), err(CODE));
    assert!(s.timers.iter().all(|t| t.kind != TimerKind::Countdown));
    let (s, fx) = step(&s, Event::CountdownReady { id: COUNTDOWN_ID });
    assert!(s.timers.iter().any(|t| t.kind == TimerKind::Countdown));
    assert!(fx.iter().any(|e| matches!(e, Effect::Speak { .. })));
}

// --- Thinking, acting, alert and the rest -----------------------------------------------

#[test]
fn an_error_never_interrupts_thinking_acting_or_an_alert() {
    for phase in [Phase::Thinking, Phase::Acting, Phase::ReminderAlert] {
        let start = state_in(phase);
        let (s, fx) = step(&start, err(CODE));
        assert_eq!(s, only_voice_down(&start), "{phase:?}");
        assert!(fx.is_empty(), "{phase:?}: {fx:?}");
    }
}

#[test]
fn a_running_action_still_finishes_after_an_error() {
    let (s, _) = run(
        state_in(Phase::Acting),
        vec![
            err(CODE),
            Event::ToolsDone {
                req: REQ_ID,
                outcome: ToolsOutcome::Completed,
            },
        ],
    );
    assert_eq!(s.phase, Phase::Success);
}

#[test]
fn a_question_that_plays_during_an_error_waits_for_text_afterwards() {
    let (s, fx) = step(&question_playing(), err(CODE));
    assert!(fx.is_empty(), "the question keeps playing");
    let (after, fx) = step(&s, Event::SpeakDone { id: SPEAK_ID });
    assert_eq!(after.phase, Phase::Thinking);
    assert_eq!(after.pending, Some(slot()));
    assert_eq!(fx, vec![start(TimerKind::FollowUp)]);
}

#[test]
fn an_alert_goes_on_and_still_ends_on_its_own() {
    let (s, _) = step(&alert_playing(), err(CODE));
    assert_eq!(s.phase, Phase::ReminderAlert);
    let (after, _) = step(&s, Event::SpeakDone { id: SPEAK_ID });
    assert_eq!(after.phase, Phase::Success, "the alert is over and settles");
}

#[test]
fn an_error_in_idle_only_degrades() {
    let start = state_in(Phase::Idle);
    let (s, fx) = step(&start, err(CODE));
    assert_eq!(s, only_voice_down(&start));
    assert!(fx.is_empty());
}

#[test]
fn an_error_while_a_reply_shows_does_not_hide_it() {
    for phase in [Phase::Success, Phase::Error] {
        for start in [
            state_in(phase),
            speaking(state_in(phase), SPEAK_ID, ReplyKind::Final),
        ] {
            let (s, fx) = step(&start, err(CODE));
            assert_eq!(s, only_voice_down(&start), "{phase:?}");
            assert!(fx.is_empty(), "{phase:?}: {fx:?}");
        }
    }
}

// --- Stale, duplicate and sequences ---------------------------------------------------------

#[test]
fn an_error_for_another_listen_or_speak_is_dropped_in_every_phase() {
    for phase in Phase::ALL {
        assert_ignored(state_in(phase), err_for(CODE, 9999));
    }
    assert_ignored(prompt_playing(), err_for(CODE, 9999));
}

#[test]
fn an_error_for_the_current_listen_or_speak_is_taken() {
    let (s, _) = step(&state_in(Phase::Listening), err_for(CODE, LISTEN_ID));
    assert_eq!(s.phase, Phase::Error);

    let (s, _) = step(
        &state_in(Phase::ConfirmationNeeded),
        err_for(CODE, CONFIRM_LISTEN_ID),
    );
    assert!(s.confirm_click_only);

    let (s, _) = step(&prompt_playing(), err_for(CODE, SPEAK_ID));
    assert!(s.confirm_click_only);
}

#[test]
fn an_error_for_a_listen_that_already_ended_is_dropped() {
    // The utterance was transcribed: that listen is over.
    let (thinking, _) = step(
        &state_in(Phase::Listening),
        Event::Transcript {
            id: LISTEN_ID,
            text: "hi".into(),
            lang: None,
        },
    );
    assert_ignored(thinking, err_for(CODE, LISTEN_ID));
}

#[test]
fn a_duplicate_error_changes_nothing() {
    let (once, _) = step(&state_in(Phase::Listening), err_for(CODE, LISTEN_ID));
    assert_ignored(once.clone(), err_for(CODE, LISTEN_ID));
    assert_ignored(once.clone(), err(CODE));
    assert_ignored(once, err(SidecarErrorCode::Internal));

    let (once, _) = step(&state_in(Phase::ConfirmationNeeded), err(CODE));
    assert_ignored(once, err(CODE));
}

#[test]
fn the_supervisor_voice_down_after_an_error_does_not_hide_the_message() {
    // The sidecar reports `error{code}` and exits: the supervisor then says the
    // voice is down. The second signal must not cut the first one short.
    let (s, _) = run(state_in(Phase::Listening), vec![err(CODE)]);
    let (after, fx) = step(&s, voice_changed(false));
    assert_eq!(after, s);
    assert!(fx.is_empty());

    let (s, _) = run(state_in(Phase::Success), vec![err(CODE)]);
    assert_ignored(s, voice_changed(false));
}

#[test]
fn voice_comes_back_only_through_voice_ready() {
    let (s, _) = run(
        state_in(Phase::Idle),
        vec![err(CODE), Event::TextSubmitted { text: "hi".into() }],
    );
    assert!(!s.voice_available);
    let (s, _) = step(&s, voice_changed(true));
    assert!(s.voice_available);
}

#[test]
fn the_reducer_is_deterministic_for_errors() {
    for phase in Phase::ALL {
        for code in SidecarErrorCode::ALL {
            let a = reduce(state_in(phase), err(code));
            let b = reduce(state_in(phase), err(code));
            assert_eq!(a, b, "{phase:?} {code:?}");
        }
    }
}
