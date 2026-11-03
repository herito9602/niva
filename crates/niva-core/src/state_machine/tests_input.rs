//! Task 1.9: Transcript, stale-id drop (all id-carrying events), NoSpeech,
//! EmptyTranscript and TextSubmitted rows. Spec: `speech-io`,
//! `language-handling`.

use super::tests::*;
use super::*;

fn user(text: &str, slot: Option<PendingSlot>) -> Effect {
    Effect::StartRequest {
        req: 100,
        input: RequestInput::User {
            text: text.into(),
            slot,
        },
    }
}

fn llm_timer() -> Effect {
    start(TimerKind::Llm)
}

fn transcript(id: u64) -> Event {
    Event::Transcript {
        id,
        text: "open notepad".into(),
    }
}

#[test]
fn transcript_in_listening_starts_an_llm_request() {
    let (s, fx) = step(&state_in(Phase::Listening), transcript(1));
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(s.request_id, Some(100));
    assert_eq!(
        s.listen_id, None,
        "the listen already ended: no cancel_listen"
    );
    assert_eq!(s.timers, vec![TimerKind::Llm]);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::NoSpeech
            },
            user("open notepad", None),
            llm_timer(),
        ]
    );
}

#[test]
fn a_follow_up_transcript_carries_the_pending_slot_for_one_turn() {
    let mut start = state_in(Phase::Listening);
    start.pending = Some(slot());
    let (s, fx) = step(&start, transcript(1));
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(s.pending, None, "the slot lives for one turn");
    assert!(s.followup_turn);
    assert!(fx.contains(&user("open notepad", Some(slot()))));
}

#[test]
fn a_transcript_after_the_utterance_cap_clears_the_max_utterance_timer() {
    let mut start = state_in(Phase::Listening);
    start.timers = running(&[TimerKind::MaxUtterance]);
    start.utterance = Utterance::Speaking;
    let (s, fx) = step(&start, transcript(1));
    assert_eq!(s.utterance, Utterance::WaitingForSpeech);
    assert!(fx.contains(&Effect::CancelTimer {
        kind: TimerKind::MaxUtterance
    }));
}

#[test]
fn transcript_is_ignored_outside_listening_even_with_a_matching_id() {
    for phase in [
        Phase::Idle,
        Phase::Thinking,
        Phase::Acting,
        Phase::Success,
        Phase::Error,
        Phase::ReminderAlert,
    ] {
        let mut s = state_in(phase);
        s.listen_id = Some(1);
        assert_ignored(s, transcript(1));
    }
}

#[test]
fn stale_ids_are_dropped_by_every_id_carrying_event_in_every_state() {
    let stale = 999;
    let events = [
        Event::SpeechStart { id: stale },
        Event::Transcript {
            id: stale,
            text: "yes".into(),
        },
        Event::NoSpeech { id: stale },
        Event::EmptyTranscript { id: stale },
        Event::SpeakStarted { id: stale },
        Event::SpeakDone { id: stale },
        Event::TtsFailed { id: stale },
        Event::ToolCalls {
            req: stale,
            verdict: ToolVerdict::RunSafe,
        },
        Event::ToolsDone {
            req: stale,
            outcome: ToolsOutcome::Completed,
        },
        Event::Reply {
            req: stale,
            text: "hi".into(),
        },
        Event::Failure { req: stale },
        Event::AskFollowUp {
            req: stale,
            question: "when?".into(),
            slot: slot(),
        },
    ];
    for phase in Phase::ALL {
        let mut with_speech = speaking(state_in(phase), 7, ReplyKind::Final);
        with_speech.listen_id = state_in(phase).listen_id;
        for base in [state_in(phase), with_speech] {
            for event in &events {
                assert_ignored(base.clone(), event.clone());
            }
        }
    }
}

#[test]
fn no_speech_in_listening_goes_idle_silently_and_clears_the_slot() {
    let mut start = state_in(Phase::Listening);
    start.pending = Some(slot());
    let (s, fx) = step(&start, Event::NoSpeech { id: 1 });
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(s.pending, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::NoSpeech
            },
            Effect::HideIsland,
            Effect::DrainReminders,
        ]
    );
}

#[test]
fn no_speech_is_ignored_in_the_states_that_never_listen() {
    for phase in [
        Phase::Idle,
        Phase::Thinking,
        Phase::Acting,
        Phase::Success,
        Phase::Error,
        Phase::ReminderAlert,
    ] {
        let mut s = state_in(phase);
        s.listen_id = Some(1);
        assert_ignored(s, Event::NoSpeech { id: 1 });
    }
}

#[test]
fn empty_transcript_in_listening_is_an_error_distinct_from_no_speech() {
    let mut start = state_in(Phase::Listening);
    start.pending = Some(slot());
    let (s, fx) = step(&start, Event::EmptyTranscript { id: 1 });
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.pending, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::NoSpeech
            },
            speak_final(100, tmpl(Template::Error)),
        ]
    );
}

#[test]
fn empty_transcript_is_ignored_in_the_states_that_never_listen() {
    for phase in [
        Phase::Idle,
        Phase::Thinking,
        Phase::Acting,
        Phase::Success,
        Phase::Error,
        Phase::ReminderAlert,
    ] {
        let mut s = state_in(phase);
        s.listen_id = Some(1);
        assert_ignored(s, Event::EmptyTranscript { id: 1 });
    }
}

#[test]
fn text_in_idle_starts_a_request_and_shows_the_island() {
    let (s, fx) = step(
        &state_in(Phase::Idle),
        Event::TextSubmitted {
            text: "hola".into(),
        },
    );
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(
        fx,
        vec![Effect::ShowIsland, user("hola", None), llm_timer()]
    );
}

#[test]
fn text_while_listening_cancels_the_listen_first() {
    let (s, fx) = step(
        &state_in(Phase::Listening),
        Event::TextSubmitted {
            text: "hola".into(),
        },
    );
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(s.listen_id, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::NoSpeech
            },
            Effect::CancelListen { id: 1 },
            user("hola", None),
            llm_timer(),
        ]
    );
}

#[test]
fn text_while_thinking_is_ignored_unless_a_pending_slot_is_open() {
    let event = Event::TextSubmitted {
        text: "at nine".into(),
    };
    assert_ignored(state_in(Phase::Thinking), event.clone());

    let (s, fx) = step(&waiting_for_text(), event);
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(s.pending, None, "text consumes the open slot");
    assert!(s.followup_turn);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::FollowUp
            },
            user("at nine", Some(slot())),
            llm_timer(),
        ]
    );
}

#[test]
fn text_while_the_follow_up_question_is_spoken_stops_it_first() {
    let mut start = speaking(waiting_for_text(), 7, ReplyKind::AskFollowUp);
    start.timers.clear();
    let (s, fx) = step(
        &start,
        Event::TextSubmitted {
            text: "at nine".into(),
        },
    );
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(s.speak_id, None);
    assert_eq!(fx.first(), Some(&Effect::StopSpeaking));
    assert!(fx.contains(&user("at nine", Some(slot()))));
}

#[test]
fn text_is_ignored_while_acting_and_while_confirming() {
    for phase in [Phase::Acting, Phase::ConfirmationNeeded] {
        assert_ignored(state_in(phase), Event::TextSubmitted { text: "x".into() });
    }
}

#[test]
fn text_after_a_reply_stops_speech_cancels_settle_and_starts_a_request() {
    for phase in [Phase::Success, Phase::Error] {
        let event = Event::TextSubmitted {
            text: "again".into(),
        };
        let talking = speaking(state_in(phase), 7, ReplyKind::Final);
        let (s, fx) = step(&talking, event.clone());
        assert_eq!(s.phase, Phase::Thinking, "{phase:?}");
        assert_eq!(
            fx,
            vec![Effect::StopSpeaking, user("again", None), llm_timer()]
        );

        let (_, fx) = step(&state_in(phase), event);
        assert_eq!(
            fx,
            vec![
                Effect::CancelTimer {
                    kind: TimerKind::Settle
                },
                user("again", None),
                llm_timer()
            ]
        );
    }
}

#[test]
fn text_in_an_alert_stops_chime_and_speech_then_thinks() {
    let (s, fx) = step(
        &state_in(Phase::ReminderAlert),
        Event::TextSubmitted { text: "ok".into() },
    );
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::AlertEnd
            },
            Effect::StopSpeaking,
            user("ok", None),
            llm_timer(),
        ]
    );
}
