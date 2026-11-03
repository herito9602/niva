//! Task 1.8: Wake, Shortcut, SpeechStart in all 8 states, including the
//! voice-down branches. Spec: `voice-activation`.

use super::tests::*;
use super::*;

fn not_ready_from_idle() -> Vec<Effect> {
    vec![
        Effect::ShowIsland,
        speak_final(100, tmpl(Template::VoiceNotReady)),
    ]
}

#[test]
fn wake_in_idle_starts_listening_when_voice_is_ready() {
    let (s, fx) = step(&state_in(Phase::Idle), Event::Wake);
    assert_eq!(s.phase, Phase::Listening);
    assert_eq!(s.listen_id, Some(100));
    let mut want = vec![Effect::ShowIsland];
    want.extend(listen_cmd(100));
    assert_eq!(fx, want);
}

#[test]
fn wake_and_shortcut_in_idle_with_voice_down_go_to_error_with_voice_not_ready() {
    for event in [Event::Wake, Event::Shortcut] {
        let (s, fx) = step(&no_voice(state_in(Phase::Idle)), event);
        assert_eq!(s.phase, Phase::Error);
        assert_eq!(s.listen_id, None, "never a listen to a dead sidecar");
        assert_eq!(fx, not_ready_from_idle());
    }
}

#[test]
fn wake_is_ignored_while_listening_thinking_acting_and_confirming() {
    for phase in [
        Phase::Listening,
        Phase::Thinking,
        Phase::Acting,
        Phase::ConfirmationNeeded,
    ] {
        assert_ignored(state_in(phase), Event::Wake);
    }
}

#[test]
fn wake_after_speak_done_in_success_or_error_starts_listening() {
    for phase in [Phase::Success, Phase::Error] {
        let (s, fx) = step(&state_in(phase), Event::Wake);
        assert_eq!(s.phase, Phase::Listening, "{phase:?}");
        let mut want = vec![Effect::CancelTimer {
            kind: TimerKind::Settle,
        }];
        want.extend(listen_cmd(100));
        assert_eq!(fx, want, "{phase:?}");
    }
}

#[test]
fn wake_is_ignored_while_niva_speaks_or_when_voice_is_down() {
    for phase in [Phase::Success, Phase::Error] {
        let talking = speaking(state_in(phase), 7, ReplyKind::Final);
        assert_ignored(talking, Event::Wake);
        assert_ignored(no_voice(state_in(phase)), Event::Wake);
    }
    assert_ignored(no_voice(state_in(Phase::ReminderAlert)), Event::Wake);
}

#[test]
fn wake_in_an_alert_stops_chime_and_speech_and_listens() {
    let (s, fx) = step(&state_in(Phase::ReminderAlert), Event::Wake);
    assert_eq!(s.phase, Phase::Listening);
    assert_eq!(s.alert_count, 0);
    let mut want = vec![
        Effect::CancelTimer {
            kind: TimerKind::AlertEnd,
        },
        Effect::StopSpeaking,
    ];
    want.extend(listen_cmd(100));
    assert_eq!(fx, want);
}

#[test]
fn shortcut_in_idle_starts_listening_when_voice_is_ready() {
    let (s, fx) = step(&state_in(Phase::Idle), Event::Shortcut);
    assert_eq!(s.phase, Phase::Listening);
    let mut want = vec![Effect::ShowIsland];
    want.extend(listen_cmd(100));
    assert_eq!(fx, want);
}

#[test]
fn shortcut_while_listening_cancels_the_listen_and_goes_idle() {
    let mut start = state_in(Phase::Listening);
    start.pending = Some(slot());
    let (s, fx) = step(&start, Event::Shortcut);
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(s.pending, None);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::NoSpeech
            },
            Effect::CancelListen { id: 1 },
            Effect::HideIsland,
            Effect::DrainReminders,
        ]
    );
}

#[test]
fn shortcut_while_thinking_cancels_the_request() {
    let (s, fx) = step(&state_in(Phase::Thinking), Event::Shortcut);
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(s.request_id, None, "a late LLM result is dropped by id");
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
fn shortcut_while_a_follow_up_question_is_spoken_stops_it_and_clears_the_slot() {
    let mut start = speaking(state_in(Phase::Thinking), 7, ReplyKind::AskFollowUp);
    start.request_id = None;
    start.pending = Some(slot());
    let (s, fx) = step(&start, Event::Shortcut);
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(s.pending, None);
    assert_eq!(s.speak_id, None);
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
fn shortcut_while_acting_cancels_the_request() {
    let (s, fx) = step(&state_in(Phase::Acting), Event::Shortcut);
    assert_eq!(s.phase, Phase::Idle);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::Tool
            },
            Effect::CancelRequest { req: 2 },
            Effect::HideIsland,
            Effect::DrainReminders,
        ]
    );
}

#[test]
fn shortcut_in_confirmation_is_a_reject_with_the_cancelled_reply() {
    let (s, fx) = step(&state_in(Phase::ConfirmationNeeded), Event::Shortcut);
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
}

#[test]
fn shortcut_while_the_confirm_prompt_plays_also_stops_speaking() {
    let mut start = speaking(
        state_in(Phase::ConfirmationNeeded),
        7,
        ReplyKind::ConfirmPrompt,
    );
    start.listen_id = None;
    let (_, fx) = step(&start, Event::Shortcut);
    assert_eq!(fx.first(), Some(&Effect::StopSpeaking));
    assert_eq!(
        fx.last(),
        Some(&speak_final(100, tmpl(Template::Cancelled)))
    );
}

#[test]
fn shortcut_in_success_or_error_barges_in_and_listens() {
    for phase in [Phase::Success, Phase::Error] {
        let talking = speaking(state_in(phase), 7, ReplyKind::Final);
        let (s, fx) = step(&talking, Event::Shortcut);
        assert_eq!(s.phase, Phase::Listening, "{phase:?}");
        let mut want = vec![Effect::StopSpeaking];
        want.extend(listen_cmd(100));
        assert_eq!(fx, want, "{phase:?}");

        let (s, fx) = step(&state_in(phase), Event::Shortcut);
        assert_eq!(s.phase, Phase::Listening, "{phase:?} after speak_done");
        assert_eq!(
            fx[0],
            Effect::CancelTimer {
                kind: TimerKind::Settle
            }
        );
    }
}

#[test]
fn shortcut_in_success_error_or_alert_with_voice_down_goes_to_error() {
    for phase in [Phase::Success, Phase::Error, Phase::ReminderAlert] {
        let (s, fx) = step(&no_voice(state_in(phase)), Event::Shortcut);
        assert_eq!(s.phase, Phase::Error, "{phase:?}");
        assert_eq!(s.listen_id, None);
        assert_eq!(
            fx.last(),
            Some(&speak_final(100, tmpl(Template::VoiceNotReady))),
            "{phase:?}"
        );
    }
}

#[test]
fn shortcut_in_an_alert_stops_it_and_listens() {
    let (s, fx) = step(&state_in(Phase::ReminderAlert), Event::Shortcut);
    assert_eq!(s.phase, Phase::Listening);
    let mut want = vec![
        Effect::CancelTimer {
            kind: TimerKind::AlertEnd,
        },
        Effect::StopSpeaking,
    ];
    want.extend(listen_cmd(100));
    assert_eq!(fx, want);
}

#[test]
fn speech_start_switches_the_no_speech_timer_to_the_max_utterance_cap() {
    let (s, fx) = step(&state_in(Phase::Listening), Event::SpeechStart { id: 1 });
    assert_eq!(s.phase, Phase::Listening);
    assert_eq!(s.utterance, Utterance::Speaking);
    assert_eq!(s.timers, vec![TimerKind::MaxUtterance]);
    assert_eq!(
        fx,
        vec![
            Effect::CancelTimer {
                kind: TimerKind::NoSpeech
            },
            start(TimerKind::MaxUtterance),
        ]
    );
}

#[test]
fn a_second_speech_start_does_not_restart_the_cap() {
    let (s, _) = step(&state_in(Phase::Listening), Event::SpeechStart { id: 1 });
    assert_ignored(s, Event::SpeechStart { id: 1 });
}

#[test]
fn speech_start_is_ignored_outside_listening() {
    for phase in Phase::ALL {
        if phase == Phase::Listening {
            continue;
        }
        let mut s = state_in(phase);
        s.listen_id = Some(1); // even a matching id leaves the cell ignored
        assert_ignored(s, Event::SpeechStart { id: 1 });
    }
}
