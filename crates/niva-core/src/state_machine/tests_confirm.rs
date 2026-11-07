//! Task 1.12: Accept/Reject, confirm "unclear" reissue, ReminderDue (queue and
//! merge), Dismiss and the `VoiceReadinessChanged{ready}` row in all 8 states.
//! Spec: `action-safety`, `reminders`, `island-shell`.

use super::tests::*;
use super::*;

fn voice_changed(ready: bool) -> Event {
    Event::VoiceReadinessChanged { ready }
}

fn confirm_listen(id: u64) -> Effect {
    Effect::Listen {
        id,
        mode: ListenMode::Confirm,
    }
}

// --- Accept / Reject -------------------------------------------------------

#[test]
fn accept_runs_the_risky_tool_and_closes_the_confirm_listen() {
    let (s, fx) = step(
        &state_in(Phase::ConfirmationNeeded),
        Event::Accept { id: CONFIRM_ID },
    );
    assert_eq!(s.phase, Phase::Acting);
    assert_eq!(s.request_id, Some(2));
    assert_eq!(s.listen_id, None);
    assert_eq!(s.timers, vec![TimerKind::Tool]);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::Confirm
            },
            Effect::CancelListen { id: 3 },
            start(TimerKind::Tool),
            Effect::RunTool { req: 2 },
        ]
    );
}

#[test]
fn a_click_while_the_prompt_is_spoken_stops_speaking_first() {
    let (_, fx) = step(&prompt_playing(), Event::Accept { id: CONFIRM_ID });
    assert_eq!(fx.first(), Some(&Effect::StopSpeaking));
    assert!(fx.contains(&Effect::RunTool { req: 2 }));

    let (s, fx) = step(&prompt_playing(), Event::Reject { id: CONFIRM_ID });
    assert_eq!(s.phase, Phase::Success);
    assert_eq!(fx.first(), Some(&Effect::StopSpeaking));
}

#[test]
fn reject_ends_with_the_cancelled_reply_and_does_not_feed_the_llm() {
    let (s, fx) = step(
        &state_in(Phase::ConfirmationNeeded),
        Event::Reject { id: CONFIRM_ID },
    );
    assert_eq!(s.phase, Phase::Success);
    assert_eq!(s.request_id, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::Confirm
            },
            Effect::CancelListen { id: 3 },
            speak_final(100, tmpl(Template::Cancelled)),
        ]
    );
    assert!(!fx.iter().any(|e| matches!(e, Effect::StartRequest { .. })));
}

#[test]
fn a_click_only_confirmation_accepts_and_rejects_without_a_listen_to_close() {
    let mut start = state_in(Phase::ConfirmationNeeded);
    start.listen_id = None;
    start.confirm_click_only = true;
    let (s, fx) = step(&start, Event::Accept { id: CONFIRM_ID });
    assert_eq!(s.phase, Phase::Acting);
    assert!(!s.confirm_click_only, "flag is per confirmation");
    assert!(!fx.iter().any(|e| matches!(e, Effect::CancelListen { .. })));
}

#[test]
fn accept_and_reject_are_ignored_outside_confirmation() {
    for phase in Phase::ALL {
        if phase == Phase::ConfirmationNeeded {
            continue;
        }
        assert_ignored(state_in(phase), Event::Accept { id: CONFIRM_ID });
        assert_ignored(state_in(phase), Event::Reject { id: CONFIRM_ID });
    }
}

// --- A click answers one confirmation only ----------------------------------

#[test]
fn a_click_for_another_confirmation_is_dropped() {
    let confirming = state_in(Phase::ConfirmationNeeded);
    assert_ignored(confirming.clone(), Event::Accept { id: CONFIRM_ID + 1 });
    assert_ignored(confirming.clone(), Event::Reject { id: CONFIRM_ID + 1 });
    assert_ignored(confirming.clone(), Event::Accept { id: 0 });
    // The matching id still works.
    let (s, _) = step(&confirming, Event::Accept { id: CONFIRM_ID });
    assert_eq!(s.phase, Phase::Acting);
}

#[test]
fn every_prompt_gets_a_new_confirmation_id() {
    let (acting, _) = step(
        &state_in(Phase::ConfirmationNeeded),
        Event::Accept { id: CONFIRM_ID },
    );
    assert_eq!(acting.confirm_id, None, "no confirmation while acting");
    let (second, _) = step(
        &acting,
        Event::ToolsDone {
            req: REQ_ID,
            outcome: ToolsOutcome::RiskyRemaining {
                prompt: "Close Spotify.exe?".into(),
                click_only: false,
                cancel_text: None,
            },
        },
    );
    assert_eq!(second.phase, Phase::ConfirmationNeeded);
    assert_eq!(second.confirm_id, Some(NEXT_CONFIRM));
    assert_eq!(second.next_confirm, NEXT_CONFIRM + 1);
}

#[test]
fn a_double_accept_leaves_the_second_prompt_waiting() {
    // Two risky calls: the user clicks Accept twice on the first prompt, and the
    // second click reaches the reducer once prompt 2 is open.
    let (acting, _) = step(
        &state_in(Phase::ConfirmationNeeded),
        Event::Accept { id: CONFIRM_ID },
    );
    let (second, _) = step(
        &acting,
        Event::ToolsDone {
            req: REQ_ID,
            outcome: ToolsOutcome::RiskyRemaining {
                prompt: "Close Spotify.exe?".into(),
                click_only: false,
                cancel_text: None,
            },
        },
    );
    assert_ignored(second.clone(), Event::Accept { id: CONFIRM_ID });
    assert_ignored(second.clone(), Event::Reject { id: CONFIRM_ID });
    assert_eq!(second.phase, Phase::ConfirmationNeeded);
    // The answer for prompt 2 itself is accepted.
    let id = second.confirm_id.expect("open confirmation");
    let (s, _) = step(&second, Event::Accept { id });
    assert_eq!(s.phase, Phase::Acting);
}

#[test]
fn reset_keeps_confirmation_ids_increasing() {
    let confirming = state_in(Phase::ConfirmationNeeded);
    let fresh = State::reset_from(&confirming);
    assert_eq!(fresh.confirm_id, None);
    assert_eq!(fresh.next_confirm, confirming.next_confirm);
}

// --- What a cancelled confirmation says -----------------------------------------

/// The state after the service asked a second prompt that carries `cancel_text`.
fn second_prompt_with(cancel_text: Option<&str>) -> State {
    let (acting, _) = step(
        &state_in(Phase::ConfirmationNeeded),
        Event::Accept { id: CONFIRM_ID },
    );
    let (second, _) = step(
        &acting,
        Event::ToolsDone {
            req: REQ_ID,
            outcome: ToolsOutcome::RiskyRemaining {
                prompt: "Close Spotify.exe?".into(),
                click_only: false,
                cancel_text: cancel_text.map(str::to_string),
            },
        },
    );
    second
}

#[test]
fn a_cancelled_confirmation_says_what_ran_when_the_service_gave_the_text() {
    let second = second_prompt_with(Some("Volume at 30%. Cancelled closing Spotify.exe."));
    let id = second.confirm_id.expect("open confirmation");
    for event in [
        Event::Reject { id },
        Event::Shortcut,
        fire(&speaking_done(second.clone()), TimerKind::Confirm),
    ] {
        let start = if matches!(event, Event::Watchdog { .. }) {
            speaking_done(second.clone())
        } else {
            second.clone()
        };
        let (s, fx) = step(&start, event.clone());
        assert_eq!(s.phase, Phase::Success, "{event:?}");
        assert!(
            fx.iter().any(|e| matches!(
                e,
                Effect::Speak { say: Say::Text(text), .. }
                    if text == "Volume at 30%. Cancelled closing Spotify.exe."
            )),
            "{event:?}: {fx:?}"
        );
        assert_eq!(s.cancel_text, None);
    }
}

#[test]
fn without_a_text_a_cancel_is_the_plain_template() {
    let second = second_prompt_with(None);
    let id = second.confirm_id.expect("open confirmation");
    let (_, fx) = step(&second, Event::Reject { id });
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Speak {
            say: Say::Template(Template::Cancelled),
            ..
        }
    )));
}

#[test]
fn an_accept_forgets_the_cancel_text() {
    let second = second_prompt_with(Some("Volume at 30%. Cancelled closing Spotify.exe."));
    let id = second.confirm_id.expect("open confirmation");
    let (s, _) = step(&second, Event::Accept { id });
    assert_eq!(s.phase, Phase::Acting);
    assert_eq!(s.cancel_text, None);
}

/// The prompt of `state` has been spoken, so the confirm timer runs.
fn speaking_done(state: State) -> State {
    let speak = state.speak_id.expect("the prompt is being spoken");
    step(&state, Event::SpeakDone { id: speak }).0
}

// --- Voice answers and the "unclear" reissue --------------------------------

#[test]
fn the_matcher_accepts_only_a_whole_normalized_utterance() {
    for yes in [
        "sí",
        "Si",
        "claro",
        "dale",
        "ok",
        "OK.",
        "yes",
        "Yeah!",
        "yep",
        "confirm",
        "  sí  ",
        "yes, please",
        "sí por favor",
        "Dale, por favor.",
    ] {
        assert_eq!(classify_confirmation(yes), Answer::Yes, "{yes:?}");
    }
    for no in [
        "no",
        "No.",
        "cancela",
        "cancelar",
        "cancel",
        "nope",
        "no, please",
    ] {
        assert_eq!(classify_confirmation(no), Answer::No, "{no:?}");
    }
    for unclear in [
        "",
        "   ",
        "maybe",
        "no sé",
        "yes close it and also open chrome",
        "sí pero no",
        "please",
        "dale dale",
        "not really",
    ] {
        assert_eq!(
            classify_confirmation(unclear),
            Answer::Unclear,
            "{unclear:?}"
        );
    }
}

#[test]
fn a_spoken_yes_accepts_and_a_spoken_no_rejects() {
    let yes = Event::Transcript {
        id: 3,
        text: "Sí".into(),
        lang: None,
    };
    let (s, fx) = step(&state_in(Phase::ConfirmationNeeded), yes);
    assert_eq!(s.phase, Phase::Acting);
    assert!(fx.contains(&Effect::RunTool { req: 2 }));

    let no = Event::Transcript {
        id: 3,
        text: "cancela".into(),
        lang: None,
    };
    let (s, fx) = step(&state_in(Phase::ConfirmationNeeded), no);
    assert_eq!(s.phase, Phase::Success);
    assert!(fx.contains(&speak_final(100, tmpl(Template::Cancelled))));
}

#[test]
fn an_unclear_answer_reissues_the_listen_with_a_new_id_and_keeps_the_timer() {
    let unclear = [
        Event::Transcript {
            id: 3,
            text: "yes close it and also open chrome".into(),
            lang: None,
        },
        Event::NoSpeech { id: 3 },
        Event::EmptyTranscript { id: 3 },
    ];
    for event in unclear {
        let (s, fx) = step(&state_in(Phase::ConfirmationNeeded), event.clone());
        assert_eq!(s.phase, Phase::ConfirmationNeeded, "{event:?}");
        assert_eq!(s.listen_id, Some(100), "new id");
        assert_eq!(
            s.timers,
            vec![TimerKind::Confirm],
            "10 s timer is not restarted"
        );
        assert_eq!(fx, vec![confirm_listen(100)], "{event:?}");
    }
}

#[test]
fn the_old_listen_id_is_stale_after_a_reissue() {
    let (s, _) = step(
        &state_in(Phase::ConfirmationNeeded),
        Event::NoSpeech { id: 3 },
    );
    assert_ignored(
        s,
        Event::Transcript {
            id: 3,
            text: "yes".into(),
            lang: None,
        },
    );
}

#[test]
fn a_click_only_confirmation_has_no_listen_to_answer() {
    let mut start = state_in(Phase::ConfirmationNeeded);
    start.listen_id = None;
    start.confirm_click_only = true;
    assert_ignored(
        start,
        Event::Transcript {
            id: 3,
            text: "yes".into(),
            lang: None,
        },
    );
}

// --- ReminderDue / Dismiss --------------------------------------------------

#[test]
fn a_reminder_in_idle_opens_the_alert_with_chime_speech_and_a_30s_cap() {
    let (s, fx) = step(
        &state_in(Phase::Idle),
        Event::ReminderDue {
            count: 2,
            silent: false,
        },
    );
    assert_eq!(s.phase, Phase::ReminderAlert);
    assert_eq!(s.alert_count, 2);
    assert_eq!(s.speak_kind, Some(ReplyKind::Alert));
    assert_eq!(s.timers, vec![TimerKind::AlertEnd]);
    assert_eq!(
        fx,
        vec![
            Effect::ShowIsland,
            Effect::Chime { id: 100 },
            Effect::Speak {
                id: 101,
                kind: ReplyKind::Alert,
                say: tmpl(Template::ReminderAlert)
            },
            start(TimerKind::AlertEnd),
        ]
    );
}

#[test]
fn a_reminder_never_interrupts_any_other_state_it_is_queued() {
    for phase in [
        Phase::Listening,
        Phase::Thinking,
        Phase::Acting,
        Phase::ConfirmationNeeded,
        Phase::Success,
        Phase::Error,
    ] {
        let start = state_in(phase);
        let (s, fx) = step(
            &start,
            Event::ReminderDue {
                count: 1,
                silent: false,
            },
        );
        assert_eq!(s, start, "{phase:?} is untouched");
        assert_eq!(fx, vec![Effect::EnqueueReminder { count: 1 }], "{phase:?}");
    }
}

#[test]
fn reminders_during_an_alert_merge_into_it() {
    let (s, fx) = step(
        &state_in(Phase::ReminderAlert),
        Event::ReminderDue {
            count: 2,
            silent: false,
        },
    );
    assert_eq!(s.phase, Phase::ReminderAlert);
    assert_eq!(s.alert_count, 3);
    assert!(fx.is_empty(), "the view updates its count from the state");
}

#[test]
fn queued_reminders_are_drained_whenever_the_state_returns_to_idle() {
    let (s, _) = step(
        &state_in(Phase::Success),
        Event::ReminderDue {
            count: 1,
            silent: false,
        },
    );
    let (s, fx) = step(&s, fire(&s, TimerKind::Settle));
    assert_eq!(s.phase, Phase::Idle);
    assert!(fx.contains(&Effect::DrainReminders));
}

#[test]
fn dismiss_acknowledges_the_alert_and_returns_to_idle() {
    let (s, fx) = step(&alert_with_count(2), Event::Dismiss);
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(s.alert_count, 0);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::AlertEnd
            },
            Effect::StopSpeaking,
            Effect::HideIsland,
            Effect::DrainReminders,
        ]
    );
}

fn alert_with_count(n: u32) -> State {
    let mut s = state_in(Phase::ReminderAlert);
    s.alert_count = n;
    s
}

#[test]
fn dismiss_is_ignored_outside_the_alert() {
    for phase in Phase::ALL {
        if phase == Phase::ReminderAlert {
            continue;
        }
        assert_ignored(state_in(phase), Event::Dismiss);
    }
}

// --- VoiceReadinessChanged ---------------------------------------------------

#[test]
fn voice_readiness_changed_sets_the_flag_in_every_state() {
    for phase in Phase::ALL {
        let (s, _) = step(&state_in(phase), voice_changed(false));
        assert!(!s.voice_available, "{phase:?}");
        let (s, _) = step(&no_voice(state_in(phase)), voice_changed(true));
        assert!(s.voice_available, "{phase:?}");
    }
}

#[test]
fn voice_down_in_idle_only_degrades() {
    let (s, fx) = step(&state_in(Phase::Idle), voice_changed(false));
    assert_eq!(s.phase, Phase::Idle);
    assert!(fx.is_empty());
}

#[test]
fn voice_down_does_not_interrupt_thinking_or_acting() {
    for phase in [Phase::Thinking, Phase::Acting] {
        let start = state_in(phase);
        let (s, fx) = step(&start, voice_changed(false));
        assert_eq!(s.phase, phase);
        assert_eq!(s.request_id, start.request_id);
        assert_eq!(s.timers, start.timers);
        assert!(fx.is_empty(), "{phase:?}: the reply is shown as text");
    }
}

#[test]
fn voice_down_in_confirmation_keeps_the_click_path_and_the_timer() {
    let (s, fx) = step(&state_in(Phase::ConfirmationNeeded), voice_changed(false));
    assert_eq!(s.phase, Phase::ConfirmationNeeded);
    assert_eq!(s.listen_id, None, "the voice channel is gone");
    assert!(s.confirm_click_only);
    assert_eq!(s.timers, vec![TimerKind::Confirm]);
    assert!(
        fx.is_empty(),
        "the sidecar is down: nothing to cancel there"
    );
    assert_ignored(
        s.clone(),
        Event::Transcript {
            id: 3,
            text: "yes".into(),
            lang: None,
        },
    );
    let (s, _) = step(&s, Event::Accept { id: CONFIRM_ID });
    assert_eq!(s.phase, Phase::Acting, "click still works");
}

#[test]
fn voice_down_in_success_or_error_drops_to_idle_text_only() {
    for phase in [Phase::Success, Phase::Error] {
        let (s, fx) = step(&state_in(phase), voice_changed(false));
        assert_eq!(s.phase, Phase::Idle, "{phase:?}");
        assert_eq!(
            fx,
            vec![
                Effect::CancelTimer {
                    kind: TimerKind::Settle
                },
                Effect::HideIsland,
                Effect::DrainReminders,
            ]
        );
        let talking = speaking(state_in(phase), 7, ReplyKind::Final);
        let (_, fx) = step(&talking, voice_changed(false));
        assert_eq!(fx.first(), Some(&Effect::StopSpeaking));
    }
}

#[test]
fn voice_down_during_an_alert_leaves_it_up() {
    let start = alert_with_count(2);
    let (s, fx) = step(&start, voice_changed(false));
    assert_eq!(s.phase, Phase::ReminderAlert);
    assert_eq!(s.alert_count, 2);
    assert!(fx.is_empty(), "the chime comes from Rust and still works");
}

#[test]
fn voice_up_clears_degraded_without_touching_the_current_state() {
    for phase in Phase::ALL {
        let start = no_voice(state_in(phase));
        let (s, fx) = step(&start, voice_changed(true));
        let mut expected = start.clone();
        expected.voice_available = true;
        assert_eq!(s, expected, "{phase:?}");
        assert!(fx.is_empty(), "{phase:?}");
    }
}

#[test]
fn a_confirmation_in_progress_stays_click_only_when_voice_returns() {
    let (down, _) = step(&state_in(Phase::ConfirmationNeeded), voice_changed(false));
    let (up, fx) = step(&down, voice_changed(true));
    assert!(up.confirm_click_only);
    assert_eq!(up.listen_id, None);
    assert!(fx.is_empty());
}

#[test]
fn after_voice_returns_new_prompts_may_use_voice_again() {
    let (up, _) = step(&no_voice(state_in(Phase::Thinking)), voice_changed(true));
    let verdict = ToolVerdict::Confirm {
        prompt: "Close it?".into(),
        click_only: false,
        cancel_text: None,
    };
    let (confirm, _) = step(&up, Event::ToolCalls { req: 2, verdict });
    let (s, fx) = step(&confirm, Event::SpeakDone { id: 100 });
    assert!(fx.contains(&confirm_listen(101)));
    assert_eq!(s.listen_id, Some(101));
}
