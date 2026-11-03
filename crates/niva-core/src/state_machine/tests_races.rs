//! Review-fix batch: id/generation guards, double events and races, the
//! watchdog sub-table (every timer kind x 8 states), voice-down sequences,
//! silent reminders and the alert-end rule, and defence-in-depth checks.

use super::tests::*;
use super::*;

fn voice_changed(ready: bool) -> Event {
    Event::VoiceReadinessChanged { ready }
}

fn yes(id: u64) -> Event {
    Event::Transcript {
        id,
        text: "yes".into(),
    }
}

fn reminder(count: u32, silent: bool) -> Event {
    Event::ReminderDue { count, silent }
}

/// Feeds `events` in order, checking the invariants after each one.
#[track_caller]
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

// --- Speak watchdog is bound to its speak id --------------------------------

#[test]
fn a_late_watchdog_of_an_old_speak_does_not_end_the_new_speak() {
    let old = Event::Watchdog {
        kind: TimerKind::SpeakWatchdog,
        token: SPEAK_ID,
    };
    let playing_new = speaking(state_in(Phase::Success), 9, ReplyKind::Final);
    assert_ignored(playing_new.clone(), old.clone());

    // The same guard on the confirmation prompt: no early confirm listen.
    let mut prompt_new = prompt_playing();
    prompt_new.speak_id = Some(9);
    assert_ignored(prompt_new, old);
}

#[test]
fn the_speak_watchdog_of_the_current_speak_still_ends_it() {
    let s = speaking(state_in(Phase::Success), 9, ReplyKind::Final);
    let (after, fx) = step(
        &s,
        Event::Watchdog {
            kind: TimerKind::SpeakWatchdog,
            token: 9,
        },
    );
    assert_eq!(after.speak_id, None);
    assert_eq!(fx, vec![start(TimerKind::Settle)]);
}

// --- Timer generations -------------------------------------------------------

#[test]
fn starting_a_running_timer_again_issues_a_new_token() {
    let mut s = state_in(Phase::Idle);
    let mut fx = Vec::new();
    s.start_timer(TimerKind::Settle, &mut fx);
    s.start_timer(TimerKind::Settle, &mut fx);
    assert_eq!(
        fx,
        vec![start(TimerKind::Settle), start_nth(TimerKind::Settle, 1)]
    );
    assert_eq!(s.timers.len(), 1);
    assert_eq!(s.timers[0].token, NEXT_TOKEN + 1);
}

#[test]
fn a_fire_of_the_first_llm_instance_is_ignored_after_the_search_summary_restarts_it() {
    let thinking = state_in(Phase::Thinking);
    let first_llm = fire(&thinking, TimerKind::Llm);
    let (s, _) = run(
        thinking,
        vec![
            Event::ToolCalls {
                req: REQ_ID,
                verdict: ToolVerdict::RunSafe,
            },
            Event::ToolsDone {
                req: REQ_ID,
                outcome: ToolsOutcome::SearchResult,
            },
        ],
    );
    assert!(s.summarizing);
    assert_eq!(s.phase, Phase::Thinking);
    assert_ignored(s.clone(), first_llm);

    let (after, _) = step(&s, fire(&s, TimerKind::Llm));
    assert_eq!(after.phase, Phase::Error, "the live instance still works");
}

#[test]
fn a_fire_of_a_previous_settle_instance_does_not_end_the_next_one() {
    let first = speaking(state_in(Phase::Success), SPEAK_ID, ReplyKind::Final);
    let (settling, _) = step(&first, Event::SpeakDone { id: SPEAK_ID });
    let old_settle = fire(&settling, TimerKind::Settle);

    // Barge in, ask again, answer again: a second Success with its own settle.
    let (s, _) = run(
        settling,
        vec![
            Event::Shortcut,
            Event::TextSubmitted {
                text: "again".into(),
            },
        ],
    );
    let req = s.request_id.expect("a request is in flight");
    let (s, _) = run(
        s,
        vec![Event::Reply {
            req,
            text: "ok".into(),
        }],
    );
    let speak = s.speak_id.expect("the reply is spoken");
    let (s, _) = run(s, vec![Event::SpeakDone { id: speak }]);
    assert!(s.timers.iter().any(|t| t.kind == TimerKind::Settle));

    assert_ignored(s.clone(), old_settle);
    let (after, _) = step(&s, fire(&s, TimerKind::Settle));
    assert_eq!(after.phase, Phase::Idle);
}

#[test]
fn a_fire_of_the_first_confirm_timer_does_not_reject_the_second_confirmation() {
    let first = prompt_playing();
    let (waiting, _) = step(&first, Event::SpeakDone { id: SPEAK_ID });
    let old_confirm = fire(&waiting, TimerKind::Confirm);

    let (s, _) = run(
        waiting,
        vec![
            Event::Accept { id: CONFIRM_ID },
            Event::ToolsDone {
                req: REQ_ID,
                outcome: ToolsOutcome::RiskyRemaining {
                    prompt: "Close Spotify.exe?".into(),
                    click_only: false,
                    cancel_text: None,
                },
            },
        ],
    );
    let speak = s.speak_id.expect("the second prompt is spoken");
    let (s, _) = run(s, vec![Event::SpeakDone { id: speak }]);
    assert_eq!(s.phase, Phase::ConfirmationNeeded);

    assert_ignored(s.clone(), old_confirm);
}

// --- Double events and races --------------------------------------------------

#[test]
fn accept_click_then_a_late_voice_yes_is_dropped() {
    let (acting, _) = step(
        &state_in(Phase::ConfirmationNeeded),
        Event::Accept { id: CONFIRM_ID },
    );
    assert_ignored(acting, yes(CONFIRM_LISTEN_ID));
}

#[test]
fn reject_then_the_confirm_watchdog_is_dropped() {
    let confirming = state_in(Phase::ConfirmationNeeded);
    let late = fire(&confirming, TimerKind::Confirm);
    let (rejected, _) = step(&confirming, Event::Reject { id: CONFIRM_ID });
    assert_ignored(rejected, late);
}

#[test]
fn accept_then_a_stale_confirm_watchdog_does_not_reject_or_rerun() {
    let confirming = state_in(Phase::ConfirmationNeeded);
    let late = fire(&confirming, TimerKind::Confirm);
    let (acting, _) = step(&confirming, Event::Accept { id: CONFIRM_ID });
    assert_ignored(acting, late);
}

#[test]
fn two_speak_done_for_the_same_id_start_the_timer_once() {
    let talking = speaking(state_in(Phase::Success), SPEAK_ID, ReplyKind::Final);
    let (once, fx) = step(&talking, Event::SpeakDone { id: SPEAK_ID });
    assert_eq!(fx, vec![start(TimerKind::Settle)]);
    assert_ignored(once.clone(), Event::SpeakDone { id: SPEAK_ID });
    assert_ignored(once, Event::TtsFailed { id: SPEAK_ID });
}

#[test]
fn reply_then_failure_for_the_same_request_ends_in_success() {
    let (replied, _) = step(
        &state_in(Phase::Thinking),
        Event::Reply {
            req: REQ_ID,
            text: "hi".into(),
        },
    );
    assert_eq!(replied.phase, Phase::Success);
    assert_ignored(replied, Event::Failure { req: REQ_ID });
}

#[test]
fn failure_then_reply_for_the_same_request_ends_in_error() {
    let (failed, _) = step(&state_in(Phase::Thinking), Event::Failure { req: REQ_ID });
    assert_eq!(failed.phase, Phase::Error);
    assert_ignored(
        failed,
        Event::Reply {
            req: REQ_ID,
            text: "late".into(),
        },
    );
}

#[test]
fn a_stale_listen_id_cannot_end_a_listen_with_no_speech_or_empty_transcript() {
    let listening = state_in(Phase::Listening);
    assert_ignored(listening.clone(), Event::NoSpeech { id: 99 });
    assert_ignored(listening, Event::EmptyTranscript { id: 99 });

    // The listen that already produced a transcript is stale too.
    let (thinking, _) = step(&state_in(Phase::Listening), yes(LISTEN_ID));
    assert_ignored(thinking.clone(), Event::NoSpeech { id: LISTEN_ID });
    assert_ignored(thinking, Event::EmptyTranscript { id: LISTEN_ID });
}

// --- Watchdog sub-table: every timer kind x 8 states ------------------------------

/// What a fire of a running timer does in a state.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Want {
    /// Leaves for this state.
    Goes(Phase),
    /// Same state (bookkeeping/effects allowed).
    Stays,
    /// Only the fired timer is forgotten: no effects, nothing else changes.
    Nothing,
}

/// Exhaustive on `TimerKind` and on `Phase`: a new variant of either breaks
/// the build here.
fn expected(kind: TimerKind, phase: Phase) -> Want {
    use Phase::*;
    let only = |home: &[Phase], then: Want| {
        if home.contains(&phase) {
            then
        } else {
            Want::Nothing
        }
    };
    match kind {
        TimerKind::NoSpeech => only(&[Listening], Want::Goes(Idle)),
        TimerKind::MaxUtterance => only(&[Listening], Want::Stays),
        TimerKind::SttTimeout => only(&[Listening], Want::Goes(Error)),
        TimerKind::Llm => only(&[Thinking], Want::Goes(Error)),
        TimerKind::FollowUp => only(&[Thinking], Want::Goes(Idle)),
        TimerKind::Tool => only(&[Acting], Want::Goes(Error)),
        TimerKind::Confirm => only(&[ConfirmationNeeded], Want::Goes(Success)),
        TimerKind::Settle => only(&[Success, Error], Want::Goes(Idle)),
        TimerKind::AlertEnd => only(&[ReminderAlert], Want::Goes(Idle)),
        // Covered by `the_speak_watchdog_cell_matches_speak_done`.
        TimerKind::SpeakWatchdog => Want::Nothing,
    }
}

#[test]
fn a_running_timer_that_fires_in_the_wrong_state_is_only_forgotten() {
    let mut cells = 0;
    for kind in TimerKind::ALL {
        if kind == TimerKind::SpeakWatchdog {
            continue;
        }
        for phase in Phase::ALL {
            let mut start_state = state_in(phase);
            start_state.timers.retain(|t| t.kind != kind);
            start_state.timers.push(Timer { kind, token: 9 });
            if kind == TimerKind::MaxUtterance {
                start_state.utterance = Utterance::Speaking;
            }
            let context = format!("{kind:?} in {phase:?}");
            let (after, fx) = reduce(start_state.clone(), Event::Watchdog { kind, token: 9 });
            assert_invariants(&after, &context);
            match expected(kind, phase) {
                Want::Goes(to) => assert_eq!(after.phase, to, "{context}: {fx:?}"),
                Want::Stays => {
                    assert_eq!(after.phase, phase, "{context}");
                    assert!(!fx.is_empty(), "{context}: must act");
                }
                Want::Nothing => {
                    let mut want = start_state.clone();
                    want.timers.retain(|t| t.kind != kind);
                    assert_eq!(after, want, "{context}");
                    assert!(fx.is_empty(), "{context}: {fx:?}");
                }
            }
            cells += 1;
        }
    }
    assert_eq!(cells, 9 * 8);
}

#[test]
fn a_timer_fire_with_a_wrong_token_is_ignored_in_every_state() {
    for kind in TimerKind::ALL {
        for phase in Phase::ALL {
            let mut s = state_in(phase);
            if kind != TimerKind::SpeakWatchdog {
                s.timers.retain(|t| t.kind != kind);
                s.timers.push(Timer { kind, token: 9 });
            }
            assert_ignored(s, Event::Watchdog { kind, token: 8 });
        }
    }
}

#[test]
fn the_speak_watchdog_cell_matches_speak_done_in_every_speaking_state() {
    for phase in Phase::ALL {
        let kind = match phase {
            Phase::Success | Phase::Error | Phase::Idle | Phase::Listening | Phase::Acting => {
                ReplyKind::Final
            }
            Phase::ConfirmationNeeded => ReplyKind::ConfirmPrompt,
            Phase::Thinking => ReplyKind::AskFollowUp,
            Phase::ReminderAlert => ReplyKind::Alert,
        };
        let talking = speaking(state_in(phase), 9, kind);
        let (via_done, fx_done) = step(&talking, Event::SpeakDone { id: 9 });
        let (via_dog, fx_dog) = step(
            &talking,
            Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token: 9,
            },
        );
        assert_eq!(via_dog, via_done, "{phase:?}");
        assert_eq!(fx_dog, fx_done, "{phase:?}");
    }
}

// --- Voice-down sequences ------------------------------------------------------------

#[test]
fn voice_down_while_a_question_plays_then_speak_done_waits_for_text() {
    let (down, fx) = step(&question_playing(), voice_changed(false));
    assert!(fx.is_empty(), "the question keeps playing");
    assert_eq!(down.speak_id, Some(SPEAK_ID));

    let (after, fx) = step(&down, Event::SpeakDone { id: SPEAK_ID });
    assert_eq!(after.phase, Phase::Thinking);
    assert_eq!(after.pending, Some(slot()));
    assert_eq!(fx, vec![start(TimerKind::FollowUp)]);
}

#[test]
fn voice_down_while_the_confirm_prompt_plays_then_speak_done_is_click_only() {
    let (down, fx) = step(&prompt_playing(), voice_changed(false));
    assert!(fx.is_empty());
    assert!(down.confirm_click_only);

    let (after, fx) = step(&down, Event::SpeakDone { id: SPEAK_ID });
    assert_eq!(after.listen_id, None);
    assert_eq!(fx, vec![start(TimerKind::Confirm)], "only the 10 s timer");
}

#[test]
fn voice_flapping_during_a_pending_slot_still_listens_when_the_question_ends() {
    let (s, _) = run(
        question_playing(),
        vec![voice_changed(false), voice_changed(true)],
    );
    assert_eq!(s.pending, Some(slot()));
    let (after, fx) = step(&s, Event::SpeakDone { id: SPEAK_ID });
    assert_eq!(after.phase, Phase::Listening);
    assert_eq!(after.pending, Some(slot()));
    assert_eq!(fx, listen_cmd(NEXT_ID).to_vec());
}

#[test]
fn voice_flapping_while_waiting_for_text_keeps_the_slot_for_the_text() {
    let (s, _) = run(
        waiting_for_text(),
        vec![voice_changed(false), voice_changed(true)],
    );
    assert_eq!(s.pending, Some(slot()));
    let (after, fx) = step(
        &s,
        Event::TextSubmitted {
            text: "at 9".into(),
        },
    );
    assert_eq!(after.phase, Phase::Thinking);
    assert!(after.followup_turn);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::StartRequest {
            input: RequestInput::User { slot: Some(_), .. },
            ..
        }
    )));
}

#[test]
fn voice_down_in_listening_with_a_pending_slot_clears_it() {
    let mut listening = state_in(Phase::Listening);
    listening.pending = Some(slot());
    let (s, _) = step(&listening, voice_changed(false));
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.pending, None);
}

// --- Defence in depth on the confirmation ----------------------------------------------

fn click_only_with_a_live_listen() -> State {
    let mut s = state_in(Phase::ConfirmationNeeded);
    s.confirm_click_only = true;
    s
}

#[test]
fn a_spoken_yes_is_ignored_when_the_confirmation_is_click_only() {
    assert_ignored(click_only_with_a_live_listen(), yes(CONFIRM_LISTEN_ID));
    assert_ignored(
        click_only_with_a_live_listen(),
        Event::Transcript {
            id: CONFIRM_LISTEN_ID,
            text: "no".into(),
        },
    );
}

#[test]
fn a_click_only_confirmation_never_reopens_a_voice_listen() {
    for event in [
        Event::NoSpeech {
            id: CONFIRM_LISTEN_ID,
        },
        Event::EmptyTranscript {
            id: CONFIRM_LISTEN_ID,
        },
    ] {
        assert_ignored(click_only_with_a_live_listen(), event);
    }
}

#[test]
fn the_reissue_needs_a_ready_voice_channel() {
    let down = no_voice(state_in(Phase::ConfirmationNeeded));
    assert_ignored(
        down,
        Event::NoSpeech {
            id: CONFIRM_LISTEN_ID,
        },
    );
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "accept without a request")]
fn accepting_without_a_request_is_a_logic_error() {
    let mut s = state_in(Phase::ConfirmationNeeded);
    s.request_id = None;
    s.listen_id = None;
    let _ = step(&s, Event::Accept { id: CONFIRM_ID });
}

// --- Tool calls while summarizing ----------------------------------------------------------

#[test]
fn tool_calls_during_the_search_summary_are_invalid() {
    for verdict in [
        ToolVerdict::RunSafe,
        ToolVerdict::Confirm {
            prompt: "Close it?".into(),
            click_only: false,
            cancel_text: None,
        },
    ] {
        let mut s = state_in(Phase::Thinking);
        s.summarizing = true;
        let (after, fx) = step(
            &s,
            Event::ToolCalls {
                req: REQ_ID,
                verdict: verdict.clone(),
            },
        );
        assert_eq!(after.phase, Phase::Error, "{verdict:?}");
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::RunTool { .. })),
            "{verdict:?}: nothing may run"
        );
        assert_eq!(
            fx.last(),
            Some(&speak_final(NEXT_ID, tmpl(Template::SearchUnavailable))),
            "{verdict:?}"
        );
    }
}

// --- Reminder alert: silent flag, alert end, counts ----------------------------------------

#[test]
fn a_silent_reminder_chimes_without_speaking() {
    let (s, fx) = step(&state_in(Phase::Idle), reminder(1, true));
    assert_eq!(s.phase, Phase::ReminderAlert);
    assert_eq!(s.speak_id, None);
    assert_eq!(s.chime_id, Some(NEXT_ID));
    assert_eq!(
        fx,
        vec![
            Effect::ShowIsland,
            Effect::Chime { id: NEXT_ID },
            start(TimerKind::AlertEnd),
        ]
    );
}

#[test]
fn a_silent_alert_ends_when_its_chime_is_done() {
    let (alert, _) = step(&state_in(Phase::Idle), reminder(1, true));
    let (s, fx) = step(&alert, Event::ChimeDone { id: NEXT_ID });
    assert_eq!(s.phase, Phase::Idle);
    assert!(fx.contains(&Effect::HideIsland));
}

#[test]
fn a_silent_alert_also_ends_by_click_or_the_30s_cap() {
    let (alert, _) = step(&state_in(Phase::Idle), reminder(1, true));
    let (s, _) = step(&alert, Event::Dismiss);
    assert_eq!(s.phase, Phase::Idle);
    let (s, _) = step(&alert, fire(&alert, TimerKind::AlertEnd));
    assert_eq!(s.phase, Phase::Idle);
}

#[test]
fn a_spoken_alert_ends_at_the_later_of_chime_and_speech() {
    let (alert, _) = step(&state_in(Phase::Idle), reminder(1, false));
    let chime = NEXT_ID;
    let speech = NEXT_ID + 1;

    // Chime first: the speech keeps the alert up.
    let (s, fx) = step(&alert, Event::ChimeDone { id: chime });
    assert_eq!(s.phase, Phase::ReminderAlert);
    assert!(fx.is_empty());
    let (s, _) = step(&s, Event::SpeakDone { id: speech });
    assert_eq!(s.phase, Phase::Idle);

    // Speech first: the chime keeps it up.
    let (s, fx) = step(&alert, Event::SpeakDone { id: speech });
    assert_eq!(s.phase, Phase::ReminderAlert);
    assert!(fx.is_empty());
    let (s, _) = step(&s, Event::ChimeDone { id: chime });
    assert_eq!(s.phase, Phase::Idle);
}

#[test]
fn the_30s_cap_ends_an_alert_whose_speech_never_finishes() {
    let (alert, _) = step(&state_in(Phase::Idle), reminder(1, false));
    let (s, fx) = step(&alert, fire(&alert, TimerKind::AlertEnd));
    assert_eq!(s.phase, Phase::Idle);
    assert!(fx.contains(&Effect::StopSpeaking));
}

#[test]
fn a_stale_chime_done_is_ignored() {
    assert_ignored(
        state_in(Phase::ReminderAlert),
        Event::ChimeDone { id: CHIME_ID + 1 },
    );
    assert_ignored(state_in(Phase::Idle), Event::ChimeDone { id: CHIME_ID });
}

#[test]
fn a_reminder_batch_of_zero_is_ignored_in_every_state() {
    for phase in Phase::ALL {
        assert_ignored(state_in(phase), reminder(0, false));
    }
}

#[test]
fn merged_alert_counts_saturate() {
    let mut alert = state_in(Phase::ReminderAlert);
    alert.alert_count = u32::MAX - 1;
    let (s, _) = step(&alert, reminder(5, false));
    assert_eq!(s.alert_count, u32::MAX);
}

#[test]
fn leaving_an_alert_forgets_its_chime() {
    let (s, _) = step(&state_in(Phase::ReminderAlert), Event::Dismiss);
    assert_eq!(s.chime_id, None);
}

// --- Determinism ------------------------------------------------------------------------------

#[test]
fn the_same_state_and_event_always_give_the_same_result() {
    let events = [
        Event::Wake,
        Event::Shortcut,
        Event::Accept { id: CONFIRM_ID },
        Event::Reject { id: CONFIRM_ID },
        Event::Dismiss,
        Event::TextSubmitted { text: "hi".into() },
        reminder(2, false),
        voice_changed(false),
        Event::SpeakDone { id: SPEAK_ID },
    ];
    for phase in Phase::ALL {
        for event in &events {
            let s = state_in(phase);
            assert_eq!(
                reduce(s.clone(), event.clone()),
                reduce(s, event.clone()),
                "{event:?} in {phase:?}"
            );
        }
    }
}
