//! Unit tests for the reducer. Shared fixtures live here; the exhaustive
//! (event x state) coverage table lives in `coverage.rs`.

use super::*;

/// Ids of the canonical fixtures (`state_in`), so tests do not repeat magic
/// numbers. `NEXT_ID` is what the next allocation returns.
pub(super) const LISTEN_ID: u64 = 1;
pub(super) const REQ_ID: u64 = 2;
pub(super) const CONFIRM_LISTEN_ID: u64 = 3;
pub(super) const SPEAK_ID: u64 = 7;
pub(super) const CHIME_ID: u64 = 8;
pub(super) const NEXT_ID: u64 = 100;
/// Token of the timers already running in a fixture, and of the first timer a
/// reduction starts from it.
pub(super) const FIXTURE_TOKEN: u64 = 1;
pub(super) const NEXT_TOKEN: u64 = 50;

/// Timers as a fixture has them running, all with `FIXTURE_TOKEN`.
pub(super) fn running(kinds: &[TimerKind]) -> Vec<Timer> {
    kinds
        .iter()
        .map(|&kind| Timer {
            kind,
            token: FIXTURE_TOKEN,
        })
        .collect()
}

/// `StartTimer` for the first timer a reduction starts from a fixture.
pub(super) fn start(kind: TimerKind) -> Effect {
    start_nth(kind, 0)
}

/// `StartTimer` for the `n`-th (0-based) timer started from a fixture.
pub(super) fn start_nth(kind: TimerKind, n: u64) -> Effect {
    Effect::StartTimer {
        kind,
        token: NEXT_TOKEN + n,
    }
}

/// The watchdog event the runner would deliver for `kind` in `state`: the token
/// of the running timer (the speak id for the speak watchdog). A timer that is
/// not running gets the never-issued token 0, i.e. a stale fire.
pub(super) fn fire(state: &State, kind: TimerKind) -> Event {
    let token = match kind {
        TimerKind::SpeakWatchdog => state.speak_id.unwrap_or(0),
        _ => state
            .timers
            .iter()
            .find(|t| t.kind == kind)
            .map_or(0, |t| t.token),
    };
    Event::Watchdog { kind, token }
}

/// Canonical state for each phase, voice ready, as it looks once the phase has
/// fully settled (prompt spoken, listen open, timers running).
pub(super) fn state_in(phase: Phase) -> State {
    let mut s = State {
        phase,
        voice_available: true,
        next_id: NEXT_ID,
        next_token: NEXT_TOKEN,
        ..State::default()
    };
    match phase {
        Phase::Idle => {}
        Phase::Listening => {
            s.listen_id = Some(LISTEN_ID);
            s.timers = running(&[TimerKind::NoSpeech]);
        }
        Phase::Thinking => {
            s.request_id = Some(REQ_ID);
            s.timers = running(&[TimerKind::Llm]);
        }
        Phase::Acting => {
            s.request_id = Some(REQ_ID);
            s.timers = running(&[TimerKind::Tool]);
        }
        Phase::ConfirmationNeeded => {
            s.request_id = Some(REQ_ID);
            s.listen_id = Some(CONFIRM_LISTEN_ID);
            s.timers = running(&[TimerKind::Confirm]);
        }
        Phase::Success | Phase::Error => {
            s.timers = running(&[TimerKind::Settle]);
        }
        Phase::ReminderAlert => {
            s.alert_count = 1;
            s.chime_id = Some(CHIME_ID);
            s.timers = running(&[TimerKind::AlertEnd]);
        }
    }
    s
}

/// `state_in` while its own speech is still playing.
pub(super) fn speaking(mut s: State, id: u64, kind: ReplyKind) -> State {
    s.speak_id = Some(id);
    s.speak_kind = Some(kind);
    s.timers.clear();
    s
}

pub(super) fn tmpl(t: Template) -> Say {
    Say::Template(t)
}

pub(super) fn no_voice(mut s: State) -> State {
    s.voice_available = false;
    s
}

pub(super) fn step(state: &State, event: Event) -> (State, Vec<Effect>) {
    reduce(state.clone(), event)
}

/// The event is dropped: same state, no effects.
#[track_caller]
pub(super) fn assert_ignored(state: State, event: Event) {
    let (after, fx) = reduce(state.clone(), event.clone());
    assert_eq!(after, state, "{event:?} must not change {:?}", state.phase);
    assert!(
        fx.is_empty(),
        "{event:?} must emit nothing in {:?}: {fx:?}",
        state.phase
    );
}

pub(super) fn listen_cmd(id: u64) -> [Effect; 2] {
    [
        Effect::Listen {
            id,
            mode: ListenMode::Command,
        },
        start(TimerKind::NoSpeech),
    ]
}

pub(super) fn speak_final(id: u64, say: Say) -> Effect {
    Effect::Speak {
        id,
        kind: ReplyKind::Final,
        say,
    }
}

pub(super) fn slot() -> PendingSlot {
    PendingSlot::Reminder {
        text: Some("call mom".into()),
    }
}

/// Prompt still being spoken in `confirmation-needed` (no listen yet).
pub(super) fn prompt_playing() -> State {
    let mut s = speaking(
        state_in(Phase::ConfirmationNeeded),
        SPEAK_ID,
        ReplyKind::ConfirmPrompt,
    );
    s.listen_id = None;
    s
}

/// Follow-up question being spoken: still `thinking`, slot open, no request.
pub(super) fn question_playing() -> State {
    let mut s = speaking(state_in(Phase::Thinking), SPEAK_ID, ReplyKind::AskFollowUp);
    s.request_id = None;
    s.pending = Some(slot());
    s
}

/// `thinking` after an `AskFollowUp` whose question already finished with
/// voice down: slot open, no request, follow-up timer running.
pub(super) fn waiting_for_text() -> State {
    let mut s = state_in(Phase::Thinking);
    s.request_id = None;
    s.pending = Some(slot());
    s.timers = running(&[TimerKind::FollowUp]);
    s
}

/// Alert whose chime is done and whose speech is still playing.
pub(super) fn alert_playing() -> State {
    let mut s = speaking(state_in(Phase::ReminderAlert), SPEAK_ID, ReplyKind::Alert);
    s.chime_id = None;
    s.timers = running(&[TimerKind::AlertEnd]);
    s
}

/// Structural invariants every reachable state must satisfy.
#[track_caller]
pub(super) fn assert_invariants(s: &State, context: &str) {
    match s.phase {
        Phase::Idle => {
            assert!(
                s.timers.is_empty()
                    && s.listen_id.is_none()
                    && s.speak_id.is_none()
                    && s.request_id.is_none()
                    && s.pending.is_none()
                    && s.chime_id.is_none()
                    && s.alert_count == 0,
                "{context}: idle must be clean: {s:?}"
            );
        }
        Phase::Listening => assert!(s.listen_id.is_some(), "{context}: {s:?}"),
        Phase::Acting => assert!(s.request_id.is_some(), "{context}: {s:?}"),
        Phase::ConfirmationNeeded => assert!(s.request_id.is_some(), "{context}: {s:?}"),
        Phase::Thinking => assert!(
            s.request_id.is_some() || s.pending.is_some(),
            "{context}: thinking needs a request or an open slot: {s:?}"
        ),
        Phase::Success | Phase::Error | Phase::ReminderAlert => {}
    }
    assert!(
        s.speak_id.is_some() == s.speak_kind.is_some(),
        "{context}: speak id and kind move together: {s:?}"
    );
    assert!(
        s.chime_id.is_none() || s.phase == Phase::ReminderAlert,
        "{context}: only an alert owns a chime: {s:?}"
    );
    let mut seen = Vec::new();
    for timer in &s.timers {
        assert!(
            !seen.contains(&timer.kind),
            "{context}: duplicate timer {:?}",
            timer.kind
        );
        seen.push(timer.kind);
        assert!(
            timer.token < s.next_token,
            "{context}: token {} was never issued: {s:?}",
            timer.token
        );
    }
}

// ---------------------------------------------------------------------------
// 1.6 skeleton
// ---------------------------------------------------------------------------

#[test]
fn default_state_is_idle_with_voice_down() {
    let state = State::default();
    assert_eq!(state.phase, Phase::Idle);
    assert!(!state.voice_available);
    assert_eq!(state.pending, None);
    assert_eq!(state.listen_id, None);
    assert_eq!(state.speak_id, None);
    assert_eq!(state.request_id, None);
}

#[test]
fn there_are_exactly_eight_states() {
    assert_eq!(Phase::ALL.len(), 8);
    for (i, phase) in Phase::ALL.iter().enumerate() {
        assert_eq!(phase.index(), i);
    }
}

// ---------------------------------------------------------------------------
// 1.7 state table: entry and exit of each of the 8 states
// ---------------------------------------------------------------------------

#[test]
fn entering_idle_hides_the_island_and_drains_queued_reminders() {
    let mut s = state_in(Phase::Success);
    let mut fx = Vec::new();
    s.transition(Phase::Idle, &mut fx);
    assert_eq!(s.phase, Phase::Idle);
    assert!(s.timers.is_empty());
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
}

#[test]
fn leaving_idle_shows_the_island() {
    let mut s = state_in(Phase::Idle);
    let mut fx = Vec::new();
    s.enter_listening(&mut fx);
    assert_eq!(s.phase, Phase::Listening);
    assert_eq!(s.listen_id, Some(100));
    assert_eq!(s.utterance, Utterance::WaitingForSpeech);
    assert_eq!(s.timers, vec![TimerKind::NoSpeech]);
    assert_eq!(
        fx,
        vec![
            Effect::ShowIsland,
            Effect::Listen {
                id: 100,
                mode: ListenMode::Command
            },
            start(TimerKind::NoSpeech),
        ]
    );
}

#[test]
fn moving_between_visible_states_does_not_show_the_island_again() {
    let mut s = state_in(Phase::Success);
    let mut fx = Vec::new();
    s.enter_listening(&mut fx);
    assert!(!fx.contains(&Effect::ShowIsland));
}

#[test]
fn leaving_listening_cancels_the_listen_and_its_timers() {
    let mut s = state_in(Phase::Listening);
    s.timers = running(&[TimerKind::NoSpeech, TimerKind::MaxUtterance]);
    let mut fx = Vec::new();
    s.transition(Phase::Idle, &mut fx);
    assert_eq!(s.listen_id, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::NoSpeech
            },
            Effect::CancelTimer {
                kind: TimerKind::MaxUtterance
            },
            Effect::CancelListen { id: 1 },
            Effect::HideIsland,
            Effect::DrainReminders,
        ]
    );
}

#[test]
fn leaving_thinking_for_a_terminal_state_cancels_the_request() {
    let mut s = state_in(Phase::Thinking);
    let mut fx = Vec::new();
    s.transition(Phase::Idle, &mut fx);
    assert_eq!(s.request_id, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::Llm
            },
            Effect::CancelRequest { req: 2 },
            Effect::HideIsland,
            Effect::DrainReminders,
        ]
    );
}

#[test]
fn the_request_survives_thinking_acting_and_confirmation() {
    for (from, to) in [
        (Phase::Thinking, Phase::Acting),
        (Phase::Thinking, Phase::ConfirmationNeeded),
        (Phase::Acting, Phase::ConfirmationNeeded),
        (Phase::Acting, Phase::Thinking),
        (Phase::ConfirmationNeeded, Phase::Acting),
    ] {
        let mut s = state_in(from);
        let mut fx = Vec::new();
        s.transition(to, &mut fx);
        assert_eq!(s.request_id, Some(2), "{from:?} -> {to:?}");
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::CancelRequest { .. })),
            "{from:?} -> {to:?}"
        );
    }
}

#[test]
fn leaving_a_state_stops_its_speech() {
    let mut s = speaking(state_in(Phase::Success), 7, ReplyKind::Final);
    let mut fx = Vec::new();
    s.transition(Phase::Idle, &mut fx);
    assert_eq!(s.speak_id, None);
    assert_eq!(s.speak_kind, None);
    assert!(fx.contains(&Effect::StopSpeaking));
}

#[test]
fn leaving_confirmation_closes_the_confirm_listen() {
    let mut s = state_in(Phase::ConfirmationNeeded);
    s.request_id = None;
    let mut fx = Vec::new();
    s.transition(Phase::Success, &mut fx);
    assert!(fx.contains(&Effect::CancelListen { id: 3 }));
    assert!(fx.contains(&Effect::CancelTimer {
        kind: TimerKind::Confirm
    }));
}

#[test]
fn leaving_the_alert_always_stops_chime_and_speech_and_resets_the_count() {
    let mut s = state_in(Phase::ReminderAlert);
    s.alert_count = 3;
    let mut fx = Vec::new();
    s.transition(Phase::Idle, &mut fx);
    assert_eq!(s.alert_count, 0);
    assert!(fx.contains(&Effect::StopSpeaking));
    assert!(fx.contains(&Effect::CancelTimer {
        kind: TimerKind::AlertEnd
    }));
}

#[test]
fn error_entry_speaks_a_final_template_and_success_entry_speaks_the_reply() {
    let mut s = state_in(Phase::Thinking);
    s.request_id = None;
    let mut fx = Vec::new();
    s.enter_error(Template::Error, &mut fx);
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.speak_id, Some(100));
    assert_eq!(s.speak_kind, Some(ReplyKind::Final));
    assert_eq!(
        fx.last(),
        Some(&Effect::Speak {
            id: 100,
            kind: ReplyKind::Final,
            say: tmpl(Template::Error)
        })
    );

    let mut s = state_in(Phase::Thinking);
    s.request_id = None;
    let mut fx = Vec::new();
    s.enter_success(Say::Text("done".into()), &mut fx);
    assert_eq!(s.phase, Phase::Success);
    assert_eq!(
        fx.last(),
        Some(&Effect::Speak {
            id: 100,
            kind: ReplyKind::Final,
            say: Say::Text("done".into())
        })
    );
}

#[test]
fn the_pending_slot_survives_only_the_follow_up_listen() {
    let mut s = state_in(Phase::Thinking);
    s.request_id = None;
    s.pending = Some(slot());
    let mut fx = Vec::new();
    s.enter_listening(&mut fx);
    assert_eq!(s.pending, Some(slot()));

    let mut s = state_in(Phase::Listening);
    s.pending = Some(slot());
    let mut fx = Vec::new();
    s.transition(Phase::Idle, &mut fx);
    assert_eq!(s.pending, None);
}

#[test]
fn ids_are_allocated_monotonically() {
    let mut s = state_in(Phase::Idle);
    assert_eq!(s.alloc_id(), 100);
    assert_eq!(s.alloc_id(), 101);
}

#[test]
fn timers_are_tracked_and_cancelled_individually() {
    let mut s = state_in(Phase::Idle);
    let mut fx = Vec::new();
    s.start_timer(TimerKind::Settle, &mut fx);
    s.start_timer(TimerKind::Settle, &mut fx);
    assert_eq!(
        s.timers,
        vec![TimerKind::Settle],
        "restart does not duplicate"
    );
    s.cancel_timer(TimerKind::Settle, &mut fx);
    s.cancel_timer(TimerKind::Settle, &mut fx);
    assert!(s.timers.is_empty());
    assert_eq!(
        fx,
        vec![
            start(TimerKind::Settle),
            start_nth(TimerKind::Settle, 1),
            Effect::CancelTimer {
                kind: TimerKind::Settle
            },
        ],
        "cancelling a timer that is not running is a no-op"
    );
}

#[test]
fn timer_durations_follow_the_design() {
    use std::time::Duration;
    let secs = |k: TimerKind| k.duration().map(|d| d.as_secs());
    assert_eq!(secs(TimerKind::NoSpeech), Some(6));
    assert_eq!(secs(TimerKind::MaxUtterance), Some(15));
    assert_eq!(secs(TimerKind::Llm), Some(15));
    assert_eq!(secs(TimerKind::FollowUp), Some(15));
    assert_eq!(secs(TimerKind::Tool), Some(5));
    assert_eq!(secs(TimerKind::Confirm), Some(10));
    assert_eq!(secs(TimerKind::Settle), Some(3));
    assert_eq!(secs(TimerKind::AlertEnd), Some(30));
    assert_eq!(TimerKind::SpeakWatchdog.duration(), None);
    assert!(TimerKind::SttTimeout.duration() > Some(Duration::ZERO));
}
