//! Sleep countdown (owner decision 2026-11-04): the `Countdown` phase.
//!
//! The safety properties of the reducer side: the OS call is impossible before
//! the deadline, a cancel and the deadline never both win, stale timers do
//! nothing, and watching stops whenever the countdown ends.

use super::tests::*;
use super::*;

const SLEEP_CANCELLED: Say = Say::Template(Template::SleepCancelled);

fn finishes(fx: &[Effect]) -> usize {
    fx.iter()
        .filter(|e| matches!(e, Effect::FinishCountdown { .. }))
        .count()
}

fn stops_watching(fx: &[Effect]) -> bool {
    fx.contains(&Effect::StopWatchingInput)
}

/// Accepted sleep: `Acting` with the tool run in flight.
fn accepted() -> State {
    state_in(Phase::Acting)
}

fn outcome(cancel_text: Option<&str>) -> Event {
    Event::ToolsDone {
        req: REQ_ID,
        outcome: ToolsOutcome::SleepCountdown {
            cancel_text: cancel_text.map(str::to_string),
        },
    }
}

/// Runs a full path from `state` to a running (armed) countdown, with real ids.
fn start_countdown(mut state: State) -> (State, u64, u64) {
    fn apply(state: &mut State, event: Event) {
        let (next, _) = reduce(state.clone(), event);
        *state = next;
    }
    apply(
        &mut state,
        Event::TextSubmitted {
            text: "suspend the pc".into(),
        },
    );
    let req = state.request_id.expect("a request");
    apply(
        &mut state,
        Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Put the PC to sleep?".into(),
                click_only: false,
                cancel_text: None,
            },
        },
    );
    let speak = state.speak_id.expect("the prompt is playing");
    apply(&mut state, Event::SpeakDone { id: speak });
    let confirm = state.confirm_id.expect("a confirmation");
    apply(&mut state, Event::Accept { id: confirm });
    apply(
        &mut state,
        Event::ToolsDone {
            req,
            outcome: ToolsOutcome::SleepCountdown { cancel_text: None },
        },
    );
    let id = state.countdown_id.expect("a countdown");
    apply(&mut state, Event::CountdownReady { id });
    let token = state
        .timers
        .iter()
        .find(|t| t.kind == TimerKind::Countdown)
        .expect("the deadline")
        .token;
    (state, id, token)
}

// --- entering the countdown ---------------------------------------------

#[test]
fn the_sleep_outcome_enters_the_countdown_and_asks_the_host_to_watch() {
    let (s, fx) = step(&accepted(), outcome(None));
    assert_eq!(s.phase, Phase::Countdown);
    assert_eq!(s.countdown_id, Some(NEXT_ID));
    assert_eq!(s.request_id, Some(REQ_ID), "the request stays open");
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::Tool
            },
            start(TimerKind::Tool),
            Effect::WatchInput { id: NEXT_ID },
        ]
    );
    assert_invariants(&s, "entering");
}

#[test]
fn there_is_no_deadline_before_the_host_says_it_is_watching() {
    let (s, fx) = step(&accepted(), outcome(None));
    assert!(s.timers.iter().all(|t| t.kind != TimerKind::Countdown));
    assert_eq!(finishes(&fx), 0);
    assert!(!fx.iter().any(|e| matches!(e, Effect::Speak { .. })));
}

#[test]
fn the_host_ready_starts_the_ten_second_deadline_and_announces() {
    let (s, fx) = step(
        &countdown_unarmed(),
        Event::CountdownReady { id: COUNTDOWN_ID },
    );
    assert_eq!(s.phase, Phase::Countdown);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::Tool
            },
            start(TimerKind::Countdown),
            Effect::Speak {
                id: NEXT_ID,
                kind: ReplyKind::Countdown,
                say: Say::Template(Template::SleepCountdown),
            },
        ]
    );
    assert_eq!(s.speak_kind, Some(ReplyKind::Countdown));
    assert_eq!(
        TimerKind::Countdown.duration(),
        Some(Duration::from_secs(10))
    );
    assert_invariants(&s, "armed");
}

#[test]
fn a_second_ready_for_the_same_countdown_changes_nothing() {
    let armed = state_in(Phase::Countdown);
    assert_ignored(armed, Event::CountdownReady { id: COUNTDOWN_ID });
}

#[test]
fn a_host_that_never_answers_ends_in_an_error_with_no_finish() {
    let (s, fx) = step(
        &countdown_unarmed(),
        fire(&countdown_unarmed(), TimerKind::Tool),
    );
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(finishes(&fx), 0);
    assert!(stops_watching(&fx));
    assert!(fx.contains(&Effect::CancelRequest { req: REQ_ID }));
    assert!(fx.contains(&speak_final(NEXT_ID, tmpl(Template::SleepNotWatched))));
}

#[test]
fn the_deadline_does_not_depend_on_speech_finishing() {
    let (mut s, _) = step(
        &countdown_unarmed(),
        Event::CountdownReady { id: COUNTDOWN_ID },
    );
    let deadline = s.timers.clone();
    // The announcement may take any time, end, or fail: the deadline is the same.
    let speak = s.speak_id.expect("announcing");
    for event in [
        Event::SpeakStarted { id: speak },
        Event::SpeakDone { id: speak },
    ] {
        (s, _) = step(&s, event);
        assert_eq!(s.phase, Phase::Countdown);
        assert_eq!(s.timers, deadline);
    }
    let (after, fx) = step(&s, fire(&s, TimerKind::Countdown));
    assert_eq!(after.phase, Phase::Acting);
    assert_eq!(finishes(&fx), 1);
}

#[test]
fn a_speech_watchdog_or_tts_failure_during_the_countdown_leaves_it_running() {
    let (s, _) = step(
        &countdown_unarmed(),
        Event::CountdownReady { id: COUNTDOWN_ID },
    );
    let speak = s.speak_id.unwrap();
    for event in [
        Event::TtsFailed { id: speak },
        Event::Watchdog {
            kind: TimerKind::SpeakWatchdog,
            token: speak,
        },
    ] {
        let (after, fx) = step(&s, event);
        assert_eq!(after.phase, Phase::Countdown);
        assert!(after.timers.iter().any(|t| t.kind == TimerKind::Countdown));
        assert_eq!(finishes(&fx), 0);
        assert!(fx.is_empty());
    }
}

// --- the deadline ---------------------------------------------------------

#[test]
fn the_deadline_finishes_the_countdown_exactly_once() {
    let s = state_in(Phase::Countdown);
    let event = fire(&s, TimerKind::Countdown);
    let (after, fx) = step(&s, event.clone());
    assert_eq!(after.phase, Phase::Acting);
    assert_eq!(
        fx,
        vec![
            Effect::StopWatchingInput,
            start(TimerKind::Tool),
            Effect::FinishCountdown { req: REQ_ID },
        ]
    );
    assert_eq!(after.countdown_id, None);
    // The same fire again (a duplicate) finds no countdown timer: dropped.
    assert_ignored(after, event);
}

#[test]
fn nothing_but_the_deadline_can_emit_finish_countdown() {
    // Every event, in every phase, with the countdown unarmed too.
    let phases = Phase::ALL
        .map(state_in)
        .into_iter()
        .chain([countdown_unarmed()]);
    for start_state in phases {
        let events = [
            Event::Wake,
            Event::Shortcut,
            Event::Dismiss,
            Event::TextSubmitted { text: "hi".into() },
            Event::Accept { id: CONFIRM_ID },
            Event::Reject { id: CONFIRM_ID },
            Event::Reply {
                req: REQ_ID,
                text: "x".into(),
            },
            Event::Failure { req: REQ_ID },
            Event::ToolsDone {
                req: REQ_ID,
                outcome: ToolsOutcome::Completed,
            },
            outcome(None),
            Event::CountdownReady { id: COUNTDOWN_ID },
            Event::CountdownCancel { id: COUNTDOWN_ID },
            Event::CountdownFault { id: COUNTDOWN_ID },
            Event::VoiceReadinessChanged { ready: false },
            fire(&start_state, TimerKind::Tool),
            fire(&start_state, TimerKind::Confirm),
            fire(&start_state, TimerKind::Llm),
            fire(&start_state, TimerKind::Settle),
        ];
        for event in events {
            let (_, fx) = step(&start_state, event.clone());
            assert_eq!(
                finishes(&fx),
                0,
                "{event:?} in {:?} must not finish a countdown",
                start_state.phase
            );
        }
    }
}

// --- cancelling -------------------------------------------------------------

#[test]
fn a_cancel_leaves_the_countdown_says_so_and_drops_the_request() {
    let s = state_in(Phase::Countdown);
    let (after, fx) = step(&s, Event::CountdownCancel { id: COUNTDOWN_ID });
    assert_eq!(after.phase, Phase::Success);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::Countdown
            },
            Effect::StopWatchingInput,
            Effect::CancelRequest { req: REQ_ID },
            speak_final(NEXT_ID, SLEEP_CANCELLED),
        ]
    );
    assert_eq!(finishes(&fx), 0);
    assert_eq!(after.request_id, None);
    assert_eq!(after.countdown_id, None);
}

#[test]
fn cancelling_before_the_countdown_is_armed_also_works() {
    let s = countdown_unarmed();
    let (after, fx) = step(&s, Event::CountdownCancel { id: COUNTDOWN_ID });
    assert_eq!(after.phase, Phase::Success);
    assert!(stops_watching(&fx));
    assert!(fx.contains(&Effect::CancelRequest { req: REQ_ID }));
    assert_eq!(finishes(&fx), 0);
}

#[test]
fn the_shortcut_cancels_the_countdown_with_the_same_words() {
    let s = state_in(Phase::Countdown);
    let (after, fx) = step(&s, Event::Shortcut);
    assert_eq!(after.phase, Phase::Success);
    assert!(stops_watching(&fx));
    assert!(fx.contains(&speak_final(NEXT_ID, SLEEP_CANCELLED)));
    assert!(fx.contains(&Effect::CancelRequest { req: REQ_ID }));
}

#[test]
fn a_cancel_text_from_the_assistant_replaces_the_template() {
    let (s, _) = step(
        &accepted(),
        outcome(Some("Volume at 30%. Suspend cancelled.")),
    );
    assert_eq!(
        s.cancel_text.as_deref(),
        Some("Volume at 30%. Suspend cancelled.")
    );
    assert_invariants(&s, "with a cancel text");
    let (_, fx) = step(&s, Event::Shortcut);
    assert!(fx.contains(&speak_final(
        NEXT_ID + 1,
        Say::Text("Volume at 30%. Suspend cancelled.".into())
    )));
}

#[test]
fn a_cancel_is_a_cancel_not_a_done() {
    let s = state_in(Phase::Countdown);
    let (_, fx) = step(&s, Event::CountdownCancel { id: COUNTDOWN_ID });
    assert!(!fx.iter().any(|e| matches!(
        e,
        Effect::Speak {
            say: Say::Template(Template::ActionDone),
            ..
        }
    )));
}

#[test]
fn an_old_deadline_timer_does_nothing_after_the_cancel() {
    let s = state_in(Phase::Countdown);
    let stale = fire(&s, TimerKind::Countdown);
    let (cancelled, _) = step(&s, Event::CountdownCancel { id: COUNTDOWN_ID });
    let (after, fx) = step(&cancelled, stale);
    assert_eq!(after, cancelled);
    assert!(fx.is_empty());
}

// --- the race ---------------------------------------------------------------

#[test]
fn cancel_then_deadline_never_finishes() {
    let s = state_in(Phase::Countdown);
    let deadline = fire(&s, TimerKind::Countdown);
    let (s, fx1) = step(&s, Event::CountdownCancel { id: COUNTDOWN_ID });
    let (_, fx2) = step(&s, deadline);
    assert_eq!(finishes(&fx1) + finishes(&fx2), 0);
}

#[test]
fn deadline_then_cancel_finishes_once_and_the_late_cancel_is_ignored() {
    let s = state_in(Phase::Countdown);
    let (s, fx1) = step(&s, fire(&s, TimerKind::Countdown));
    let (after, fx2) = step(&s, Event::CountdownCancel { id: COUNTDOWN_ID });
    assert_eq!(finishes(&fx1), 1);
    assert_eq!(finishes(&fx2), 0);
    assert_eq!(after, s, "too late: the late cancel changes nothing");
    assert!(fx2.is_empty());
}

#[test]
fn the_late_shortcut_after_the_deadline_still_ends_the_request_so_the_held_approval_goes() {
    // The reducer already moved on (`Acting`): the shortcut ends the request,
    // `CancelRequest` reaches the assistant, which drops what it still holds
    // (if the worker has not taken it yet, nothing is suspended).
    let s = state_in(Phase::Countdown);
    let (s, _) = step(&s, fire(&s, TimerKind::Countdown));
    let (after, fx) = step(&s, Event::Shortcut);
    assert_eq!(after.phase, Phase::Idle);
    assert!(fx.contains(&Effect::CancelRequest { req: REQ_ID }));
    assert_eq!(finishes(&fx), 0);
}

// --- stale countdowns -----------------------------------------------------------

#[test]
fn a_timer_of_an_earlier_countdown_never_fires_a_later_one() {
    let (first, id1, token1) = start_countdown(State::default());
    let (cancelled, _) = step(&first, Event::CountdownCancel { id: id1 });
    // A second sleep right after: a new request, a new id, a new token.
    let (second, id2, token2) = start_countdown(cancelled);
    assert_ne!(id1, id2);
    assert_ne!(token1, token2);
    let stale = Event::Watchdog {
        kind: TimerKind::Countdown,
        token: token1,
    };
    assert_ignored(second.clone(), stale);
    // ...and a cancel meant for the first countdown does not touch the second.
    assert_ignored(second.clone(), Event::CountdownCancel { id: id1 });
    assert_ignored(second.clone(), Event::CountdownFault { id: id1 });
    let (after, fx) = step(
        &second,
        Event::Watchdog {
            kind: TimerKind::Countdown,
            token: token2,
        },
    );
    assert_eq!(after.phase, Phase::Acting);
    assert_eq!(finishes(&fx), 1);
}

#[test]
fn a_reset_forgets_the_countdown_and_its_old_timer_fires_into_nothing() {
    let (running_state, _, token) = start_countdown(State::default());
    let fresh = State::reset_from(&running_state);
    assert_eq!(fresh.phase, Phase::Idle);
    assert_ignored(
        fresh,
        Event::Watchdog {
            kind: TimerKind::Countdown,
            token,
        },
    );
}

#[test]
fn a_new_request_cannot_start_while_a_countdown_runs() {
    assert_ignored(
        state_in(Phase::Countdown),
        Event::TextSubmitted { text: "hi".into() },
    );
    assert_ignored(
        countdown_unarmed(),
        Event::TextSubmitted { text: "hi".into() },
    );
}

#[test]
fn ids_of_another_countdown_are_dropped_in_every_phase() {
    for phase in Phase::ALL {
        for event in [
            Event::CountdownReady { id: 77 },
            Event::CountdownCancel { id: 77 },
            Event::CountdownFault { id: 77 },
        ] {
            assert_ignored(state_in(phase), event);
        }
    }
    assert_ignored(countdown_unarmed(), Event::CountdownCancel { id: 77 });
}

// --- failing closed ---------------------------------------------------------

#[test]
fn a_fault_ends_in_an_honest_error_and_never_finishes() {
    for start_state in [state_in(Phase::Countdown), countdown_unarmed()] {
        let (after, fx) = step(&start_state, Event::CountdownFault { id: COUNTDOWN_ID });
        assert_eq!(after.phase, Phase::Error);
        assert_eq!(finishes(&fx), 0);
        assert!(stops_watching(&fx));
        assert!(fx.contains(&Effect::CancelRequest { req: REQ_ID }));
        assert!(fx.contains(&speak_final(NEXT_ID, tmpl(Template::SleepNotWatched))));
        // The deadline that was running is cancelled: it can never fire.
        let timer = start_state.timers[0].kind;
        assert!(fx.contains(&Effect::CancelTimer { kind: timer }));
    }
}

// --- watching stops ---------------------------------------------------------

#[test]
fn watching_stops_on_every_way_out_of_the_countdown() {
    let armed = state_in(Phase::Countdown);
    let exits = [
        Event::CountdownCancel { id: COUNTDOWN_ID },
        Event::CountdownFault { id: COUNTDOWN_ID },
        Event::Shortcut,
        fire(&armed, TimerKind::Countdown),
    ];
    for event in exits {
        let (after, fx) = step(&armed, event.clone());
        assert_ne!(after.phase, Phase::Countdown, "{event:?}");
        assert!(stops_watching(&fx), "{event:?}: {fx:?}");
    }
    let (after, fx) = step(
        &countdown_unarmed(),
        fire(&countdown_unarmed(), TimerKind::Tool),
    );
    assert_ne!(after.phase, Phase::Countdown);
    assert!(stops_watching(&fx));
}

#[test]
fn staying_in_the_countdown_never_stops_watching() {
    let armed = state_in(Phase::Countdown);
    for event in [
        Event::ReminderDue {
            count: 1,
            silent: false,
        },
        Event::VoiceReadinessChanged { ready: false },
        Event::Wake,
        Event::Dismiss,
    ] {
        let (after, fx) = step(&armed, event.clone());
        assert_eq!(after.phase, Phase::Countdown, "{event:?}");
        assert!(!stops_watching(&fx), "{event:?}");
    }
}

// --- the rest of the world during a countdown ----------------------------------

#[test]
fn a_reminder_due_during_the_countdown_is_queued_not_shown_over_it() {
    let (after, fx) = step(
        &state_in(Phase::Countdown),
        Event::ReminderDue {
            count: 2,
            silent: false,
        },
    );
    assert_eq!(after.phase, Phase::Countdown);
    assert_eq!(fx, vec![Effect::EnqueueReminder { count: 2 }]);
}

#[test]
fn clicks_voice_and_late_results_do_not_touch_a_countdown() {
    let s = state_in(Phase::Countdown);
    for event in [
        Event::Accept { id: 1 },
        Event::Reject { id: 1 },
        Event::Reply {
            req: REQ_ID,
            text: "x".into(),
        },
        Event::Failure { req: REQ_ID },
        Event::FailureText {
            req: REQ_ID,
            text: "x".into(),
        },
        Event::ToolsDone {
            req: REQ_ID,
            outcome: ToolsOutcome::Completed,
        },
        outcome(None),
        Event::Wake,
        Event::Dismiss,
    ] {
        assert_ignored(s.clone(), event);
    }
}

#[test]
fn a_spoken_cancel_is_not_heard_yet_and_the_event_slice_3_will_send_cancels() {
    // Seam for slice 3 (voice): a spoken "cancel" will arrive as
    // `CountdownCancel{id}` from the voice layer. No listen is open in the
    // countdown today, so a stray transcript changes nothing.
    let s = state_in(Phase::Countdown);
    assert_eq!(s.listen_id, None);
    assert_ignored(
        s.clone(),
        Event::Transcript {
            id: 1,
            text: "cancel".into(),
        },
    );
    let (after, _) = step(&s, Event::CountdownCancel { id: COUNTDOWN_ID });
    assert_eq!(after.phase, Phase::Success);
}

#[test]
fn the_sleep_outcome_only_means_something_while_acting() {
    for phase in Phase::ALL {
        if phase == Phase::Acting {
            continue;
        }
        assert_ignored(state_in(phase), outcome(None));
    }
}

#[test]
fn the_countdown_keeps_a_hold_of_the_request_until_it_ends() {
    // `transition` into the countdown must not cancel the request it continues.
    let (s, fx) = step(&accepted(), outcome(None));
    assert!(!fx.iter().any(|e| matches!(e, Effect::CancelRequest { .. })));
    assert_eq!(s.request_id, Some(REQ_ID));
}

#[test]
fn the_announcement_says_ten_seconds_and_matches_the_timer() {
    use crate::replies::{Lang, template};
    let seconds = TimerKind::Countdown.duration().unwrap().as_secs();
    assert_eq!(seconds, 10);
    for lang in [Lang::En, Lang::Es] {
        let text = template(Template::SleepCountdown, lang);
        assert!(text.contains(&seconds.to_string()), "{lang:?}: {text}");
    }
}
