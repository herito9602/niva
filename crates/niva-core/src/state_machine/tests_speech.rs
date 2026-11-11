//! Task 1.11: SpeakStarted/SpeakDone/TtsFailed per `ReplyKind`, the watchdog
//! timeout row for every state, and CO4 (`VoiceReadinessChanged{ready:false}`
//! while listening). Spec: `speech-io`.

use super::tests::*;
use super::*;

fn done_events(id: u64) -> [Event; 2] {
    [Event::SpeakDone { id }, Event::TtsFailed { id }]
}

#[test]
fn speak_started_matching_id_sets_audible_speak() {
    for phase in Phase::ALL {
        let talking = speaking(state_in(phase), 7, ReplyKind::Final);
        let (s, fx) = step(&talking, Event::SpeakStarted { id: 7 });
        assert_eq!(s.audible_speak, Some(7), "{phase:?}");
        assert_eq!(s.phase, phase, "state-only update");
        assert!(fx.is_empty(), "{phase:?}: {fx:?}");
        assert_eq!(s.speak_id, Some(7));
    }
}

#[test]
fn speak_started_mismatched_id_does_not_set_it() {
    for phase in Phase::ALL {
        let talking = speaking(state_in(phase), 7, ReplyKind::Final);
        assert_ignored(talking, Event::SpeakStarted { id: 8 });
    }
}

#[test]
fn speech_is_not_audible_until_speak_started() {
    let talking = speaking(state_in(Phase::Success), 7, ReplyKind::Final);
    assert_eq!(talking.audible_speak, None);
}

/// Every way `speak_id` clears also clears `audible_speak`.
#[test]
fn audible_speak_clears_wherever_speak_id_clears() {
    let audible = |mut s: State| {
        s.audible_speak = s.speak_id;
        s
    };
    let after_done = |event: Event| {
        let s = audible(speaking(state_in(Phase::Success), 7, ReplyKind::Final));
        step(&s, event).0
    };
    for s in [
        after_done(Event::SpeakDone { id: 7 }),
        after_done(Event::TtsFailed { id: 7 }),
        after_done(Event::Watchdog {
            kind: TimerKind::SpeakWatchdog,
            token: 7,
        }),
    ] {
        assert_eq!(s.speak_id, None);
        assert_eq!(s.audible_speak, None);
    }
    // Leaving the phase (shortcut barge-in, text submitted) clears it too.
    for event in [Event::Shortcut, Event::TextSubmitted { text: "hi".into() }] {
        let s = audible(speaking(state_in(Phase::Success), 7, ReplyKind::Final));
        let (after, _) = step(&s, event.clone());
        assert_eq!(after.speak_id, None, "{event:?}");
        assert_eq!(after.audible_speak, None, "{event:?}");
    }
    // A new speech starts silent even when the previous one was audible.
    let s = audible(speaking(
        state_in(Phase::Thinking),
        7,
        ReplyKind::AskFollowUp,
    ));
    let (after, _) = step(&s, Event::Shortcut);
    assert_eq!(after.audible_speak, None);
}

#[test]
fn a_final_reply_that_finished_starts_the_settle_timer() {
    for phase in [Phase::Success, Phase::Error] {
        for event in done_events(7) {
            let talking = speaking(state_in(phase), 7, ReplyKind::Final);
            let (s, fx) = step(&talking, event.clone());
            assert_eq!(s.phase, phase, "{event:?}");
            assert_eq!(s.speak_id, None);
            assert_eq!(s.timers, vec![TimerKind::Settle]);
            assert_eq!(fx, vec![start(TimerKind::Settle)]);
        }
    }
}

#[test]
fn a_finished_confirm_prompt_opens_the_confirm_listen_and_the_10s_timer() {
    for event in done_events(7) {
        let (s, fx) = step(&prompt_playing(), event.clone());
        assert_eq!(s.phase, Phase::ConfirmationNeeded, "{event:?}");
        assert_eq!(s.listen_id, Some(100));
        assert_eq!(s.timers, vec![TimerKind::Confirm]);
        assert_eq!(
            fx,
            vec![
                start(TimerKind::Confirm),
                Effect::Listen {
                    id: 100,
                    mode: ListenMode::Confirm
                },
            ]
        );
    }
}

#[test]
fn a_confirm_prompt_is_click_only_when_voice_is_down_or_the_action_is_power() {
    let mut power = prompt_playing();
    power.confirm_click_only = true;
    for from in [no_voice(prompt_playing()), power] {
        let (s, fx) = step(&from, Event::SpeakDone { id: 7 });
        assert_eq!(s.listen_id, None);
        assert_eq!(
            fx,
            vec![start(TimerKind::Confirm)],
            "the 10 s timer still runs"
        );
    }
}

#[test]
fn a_finished_follow_up_question_listens_directly_when_voice_is_ready() {
    for event in done_events(7) {
        let (s, fx) = step(&question_playing(), event.clone());
        assert_eq!(s.phase, Phase::Listening, "{event:?}");
        assert_eq!(
            s.pending,
            Some(slot()),
            "the slot rides the follow-up listen"
        );
        assert_eq!(s.timers, vec![TimerKind::NoSpeech]);
        assert_eq!(
            fx,
            listen_cmd(100).to_vec(),
            "no settle timer, no island change"
        );
    }
}

#[test]
fn a_finished_follow_up_question_waits_for_text_when_voice_is_down() {
    let (s, fx) = step(&no_voice(question_playing()), Event::SpeakDone { id: 7 });
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(s.pending, Some(slot()));
    assert_eq!(s.listen_id, None);
    assert_eq!(s.timers, vec![TimerKind::FollowUp]);
    assert_eq!(fx, vec![start(TimerKind::FollowUp)]);
}

/// Reminders, "The alert ends with the speech": the pulse never resumes once
/// the reminder was heard, even if the chime is still playing.
#[test]
fn reminder_alert_pulse_does_not_resume_after_speech() {
    let mut start = alert_playing();
    start.chime_id = Some(CHIME_ID);
    for event in done_events(SPEAK_ID) {
        let (s, _) = step(&start, event.clone());
        assert_ne!(s.phase, Phase::ReminderAlert, "{event:?}");
        assert_eq!(s.chime_id, None, "{event:?}");
        assert_eq!(s.alert_count, 0, "{event:?}");
    }
}

#[test]
fn reminder_alert_ends_with_speech_then_settles() {
    for event in done_events(SPEAK_ID) {
        let (s, fx) = step(&alert_playing(), event.clone());
        assert_eq!(s.phase, Phase::Success, "{event:?}");
        assert_eq!(s.speak_id, None);
        assert_eq!(
            fx,
            vec![
                Effect::CancelTimer {
                    kind: TimerKind::AlertEnd
                },
                Effect::StopSpeaking,
                start(TimerKind::Settle),
            ]
        );
        // The settle hides the notch like any other result.
        let (s, fx) = step(&s, fire(&s, TimerKind::Settle));
        assert_eq!(s.phase, Phase::Idle);
        assert_eq!(fx, vec![Effect::HideIsland, Effect::DrainReminders]);
    }
}

#[test]
fn watchdog_is_ignored_in_idle() {
    for kind in [TimerKind::Settle, TimerKind::Llm, TimerKind::SpeakWatchdog] {
        assert_ignored(state_in(Phase::Idle), fire(&state_in(Phase::Idle), kind));
    }
}

#[test]
fn no_speech_timeout_cancels_the_listen_and_goes_idle() {
    let mut start = state_in(Phase::Listening);
    start.pending = Some(slot());
    let (s, fx) = step(&start, fire(&start, TimerKind::NoSpeech));
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(s.pending, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelListen { id: 1 },
            Effect::HideIsland,
            Effect::DrainReminders
        ]
    );
}

#[test]
fn max_utterance_cap_forces_the_end_of_speech_and_arms_the_stt_timeout() {
    let (speech, _) = step(&state_in(Phase::Listening), Event::SpeechStart { id: 1 });
    let (s, fx) = step(&speech, fire(&speech, TimerKind::MaxUtterance));
    assert_eq!(s.phase, Phase::Listening);
    assert_eq!(s.utterance, Utterance::Ending);
    assert_eq!(s.timers, vec![TimerKind::SttTimeout]);
    assert_eq!(
        fx,
        vec![
            Effect::ForceEndOfSpeech { id: 1 },
            start_nth(TimerKind::SttTimeout, 1),
        ]
    );
}

#[test]
fn stt_timeout_after_the_forced_end_of_speech_is_an_error() {
    let (speech, _) = step(&state_in(Phase::Listening), Event::SpeechStart { id: 1 });
    let (ending, _) = step(&speech, fire(&speech, TimerKind::MaxUtterance));
    let (s, fx) = step(&ending, fire(&ending, TimerKind::SttTimeout));
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(
        fx,
        vec![
            Effect::CancelListen { id: 1 },
            speak_final(100, tmpl(Template::Error)),
        ]
    );
}

#[test]
fn a_timer_that_is_no_longer_running_is_ignored() {
    // The no-speech timer was replaced by the max-utterance cap.
    let (speech, _) = step(&state_in(Phase::Listening), Event::SpeechStart { id: 1 });
    assert_ignored(speech.clone(), fire(&speech, TimerKind::NoSpeech));
    let late = fire(&speech, TimerKind::SttTimeout);
    assert_ignored(speech, late);
    assert_ignored(
        state_in(Phase::Thinking),
        fire(&state_in(Phase::Thinking), TimerKind::Settle),
    );
}

#[test]
fn llm_timeout_cancels_the_request_and_errors() {
    let (s, fx) = step(
        &state_in(Phase::Thinking),
        fire(&state_in(Phase::Thinking), TimerKind::Llm),
    );
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.request_id, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelRequest { req: 2 },
            speak_final(100, tmpl(Template::Error)),
        ]
    );
}

#[test]
fn a_search_summary_timeout_says_it_cannot_search_now() {
    let mut start = state_in(Phase::Thinking);
    start.summarizing = true;
    let (s, fx) = step(&start, fire(&start, TimerKind::Llm));
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(
        fx.last(),
        Some(&speak_final(100, tmpl(Template::SearchUnavailable)))
    );
}

#[test]
fn follow_up_wait_timeout_clears_the_slot_and_goes_idle() {
    let mut start = state_in(Phase::Thinking);
    start.request_id = None;
    start.pending = Some(slot());
    start.timers = running(&[TimerKind::FollowUp]);
    let (s, fx) = step(&start, fire(&start, TimerKind::FollowUp));
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(s.pending, None);
    assert_eq!(fx, vec![Effect::HideIsland, Effect::DrainReminders]);
}

#[test]
fn tool_timeout_cancels_the_request_and_errors() {
    let (s, fx) = step(
        &state_in(Phase::Acting),
        fire(&state_in(Phase::Acting), TimerKind::Tool),
    );
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(
        fx,
        vec![
            Effect::CancelRequest { req: 2 },
            speak_final(100, tmpl(Template::Error)),
        ]
    );
}

#[test]
fn confirm_timeout_is_a_reject() {
    let start = state_in(Phase::ConfirmationNeeded);
    let (s, fx) = step(&start, fire(&start, TimerKind::Confirm));
    assert_eq!(s.phase, Phase::Success);
    assert_eq!(s.request_id, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelListen { id: 3 },
            speak_final(100, tmpl(Template::Cancelled)),
        ]
    );
}

#[test]
fn settle_timeout_goes_idle_and_hides_the_island() {
    for phase in [Phase::Success, Phase::Error] {
        let (s, fx) = step(&state_in(phase), fire(&state_in(phase), TimerKind::Settle));
        assert_eq!(s.phase, Phase::Idle, "{phase:?}");
        assert_eq!(fx, vec![Effect::HideIsland, Effect::DrainReminders]);
    }
}

#[test]
fn reminder_30s_alert_timer_bounds_unspoken_reminder() {
    // A reminder that is never spoken (fullscreen, sidecar silent) is bounded
    // by the 30 s timer.
    assert_eq!(
        TimerKind::AlertEnd.duration(),
        Some(std::time::Duration::from_secs(30))
    );
    let (alert, _) = step(
        &state_in(Phase::Idle),
        Event::ReminderDue {
            count: 1,
            silent: true,
        },
    );
    assert_eq!(alert.speak_id, None);
    let (s, fx) = step(&alert, fire(&alert, TimerKind::AlertEnd));
    assert_eq!(s.phase, Phase::Idle);
    assert!(fx.contains(&Effect::HideIsland));
}

#[test]
fn alert_end_goes_idle() {
    let start = state_in(Phase::ReminderAlert);
    let (s, fx) = step(&start, fire(&start, TimerKind::AlertEnd));
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(
        fx,
        vec![
            Effect::StopSpeaking,
            Effect::HideIsland,
            Effect::DrainReminders
        ]
    );
}

#[test]
fn the_speak_watchdog_synthesizes_speak_done_in_every_state_that_speaks() {
    let cases = [
        (
            speaking(state_in(Phase::Success), 7, ReplyKind::Final),
            Phase::Success,
        ),
        (
            speaking(state_in(Phase::Error), 7, ReplyKind::Final),
            Phase::Error,
        ),
        (prompt_playing(), Phase::ConfirmationNeeded),
        (question_playing(), Phase::Listening),
    ];
    for (start, expected_phase) in cases {
        let (via_done, fx_done) = step(&start, Event::SpeakDone { id: 7 });
        let (via_dog, fx_dog) = step(&start, fire(&start, TimerKind::SpeakWatchdog));
        assert_eq!(via_dog.phase, expected_phase);
        assert_eq!(via_dog, via_done, "{:?}", start.phase);
        assert_eq!(fx_dog, fx_done);
    }
    let (s, _) = step(
        &alert_playing(),
        fire(&alert_playing(), TimerKind::SpeakWatchdog),
    );
    assert_eq!(s.phase, Phase::Success, "the alert ends and settles");
}

#[test]
fn the_speak_watchdog_is_ignored_when_nothing_is_being_spoken() {
    for phase in Phase::ALL {
        assert_ignored(
            state_in(phase),
            fire(&state_in(phase), TimerKind::SpeakWatchdog),
        );
    }
}

#[test]
fn losing_voice_while_listening_errors_and_speaks_so_the_settle_timer_starts() {
    let (s, fx) = step(
        &state_in(Phase::Listening),
        Event::VoiceReadinessChanged { ready: false },
    );
    assert!(!s.voice_available);
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.listen_id, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::NoSpeech
            },
            Effect::CancelListen { id: 1 },
            speak_final(100, tmpl(Template::Error)),
        ]
    );
}

#[test]
fn the_error_template_after_losing_voice_still_arms_the_settle_timer() {
    let (s, fx) = step(
        &state_in(Phase::Listening),
        Event::VoiceReadinessChanged { ready: false },
    );
    // The runner skips the Speak while voice is down and synthesizes SpeakDone.
    let speak_id = s.speak_id.expect("error template is speaking");
    let (s, fx2) = step(&s, Event::SpeakDone { id: speak_id });
    assert!(fx.iter().any(|e| matches!(e, Effect::Speak { .. })));
    assert_eq!(fx2, vec![start(TimerKind::Settle)]);
    let (s, _) = step(&s, fire(&s, TimerKind::Settle));
    assert_eq!(s.phase, Phase::Idle);
}
