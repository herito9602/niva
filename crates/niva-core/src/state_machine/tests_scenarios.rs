//! End-to-end event sequences from `State::default()`, with the structural
//! invariants checked after every step.

use super::tests::*;
use super::*;

/// Drives the reducer and checks the invariants after each event.
struct Run {
    state: State,
}

impl Run {
    fn new() -> Self {
        Run {
            state: State::default(),
        }
    }

    #[track_caller]
    fn send(&mut self, event: Event) -> Vec<Effect> {
        let context = format!("{event:?}");
        let (next, fx) = reduce(self.state.clone(), event);
        assert_invariants(&next, &context);
        self.state = next;
        fx
    }

    fn listen_id(&self) -> u64 {
        self.state.listen_id.expect("a listen is open")
    }

    fn request_id(&self) -> u64 {
        self.state.request_id.expect("a request is in flight")
    }

    fn speak_id(&self) -> u64 {
        self.state.speak_id.expect("something is being spoken")
    }

    fn phase(&self) -> Phase {
        self.state.phase
    }

    /// Voice ready, a wake word and an utterance: ends in `thinking`.
    #[track_caller]
    fn hear(&mut self, text: &str) {
        self.send(Event::VoiceReadinessChanged { ready: true });
        assert_eq!(self.phase(), Phase::Idle);
        self.send(Event::Wake);
        assert_eq!(self.phase(), Phase::Listening);
        let id = self.listen_id();
        self.send(Event::SpeechStart { id });
        self.send(Event::Transcript {
            id,
            text: text.into(),
        });
        assert_eq!(self.phase(), Phase::Thinking);
    }
}

#[test]
fn scenario_safe_tool_from_wake_word_to_idle() {
    let mut run = Run::new();
    run.hear("open notepad");
    let req = run.request_id();

    let fx = run.send(Event::ToolCalls {
        req,
        verdict: ToolVerdict::RunSafe,
    });
    assert_eq!(run.phase(), Phase::Acting);
    assert!(fx.contains(&Effect::RunTool { req }));

    let fx = run.send(Event::ToolsDone {
        req,
        outcome: ToolsOutcome::Completed,
    });
    assert_eq!(run.phase(), Phase::Success);
    let speak = run.speak_id();
    assert!(fx.contains(&speak_final(speak, tmpl(Template::ActionDone))));

    run.send(Event::SpeakStarted { id: speak });
    let fx = run.send(Event::SpeakDone { id: speak });
    assert!(matches!(
        fx.as_slice(),
        [Effect::StartTimer {
            kind: TimerKind::Settle,
            ..
        }]
    ));
    let settle = fire(&run.state, TimerKind::Settle);
    let fx = run.send(settle);
    assert_eq!(run.phase(), Phase::Idle);
    assert!(fx.contains(&Effect::HideIsland));
}

#[test]
fn scenario_risky_call_confirmed_by_voice() {
    let mut run = Run::new();
    run.hear("close spotify");
    let req = run.request_id();

    run.send(Event::ToolCalls {
        req,
        verdict: ToolVerdict::Confirm {
            prompt: "Close Spotify.exe?".into(),
            click_only: false,
        },
    });
    assert_eq!(run.phase(), Phase::ConfirmationNeeded);
    assert_eq!(
        run.state.listen_id, None,
        "no listen while the prompt plays"
    );

    let prompt = run.speak_id();
    let fx = run.send(Event::SpeakDone { id: prompt });
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Listen {
            mode: ListenMode::Confirm,
            ..
        }
    )));

    let listen = run.listen_id();
    let fx = run.send(Event::Transcript {
        id: listen,
        text: "yes".into(),
    });
    assert_eq!(run.phase(), Phase::Acting);
    assert!(fx.contains(&Effect::RunTool { req }));

    run.send(Event::ToolsDone {
        req,
        outcome: ToolsOutcome::Completed,
    });
    assert_eq!(run.phase(), Phase::Success);
    let speak = run.speak_id();
    run.send(Event::SpeakDone { id: speak });
    let settle = fire(&run.state, TimerKind::Settle);
    run.send(settle);
    assert_eq!(run.phase(), Phase::Idle);
}

#[test]
fn scenario_follow_up_question_survives_voice_loss_and_is_answered_by_text() {
    let mut run = Run::new();
    run.hear("remind me to call mom");
    let req = run.request_id();

    let fx = run.send(Event::AskFollowUp {
        req,
        question: "When?".into(),
        slot: slot(),
    });
    assert_eq!(run.phase(), Phase::Thinking, "the mascot stays thinking");
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Speak {
            kind: ReplyKind::AskFollowUp,
            ..
        }
    )));

    run.send(Event::VoiceReadinessChanged { ready: false });
    let question = run.speak_id();
    let fx = run.send(Event::SpeakDone { id: question });
    assert!(matches!(
        fx.as_slice(),
        [Effect::StartTimer {
            kind: TimerKind::FollowUp,
            ..
        }]
    ));
    assert_eq!(run.state.pending, Some(slot()));

    let fx = run.send(Event::TextSubmitted {
        text: "at 9".into(),
    });
    assert_eq!(run.phase(), Phase::Thinking);
    assert!(run.state.followup_turn);
    assert_eq!(run.state.pending, None, "the slot lives for one turn");
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::StartRequest {
            input: RequestInput::User { slot: Some(_), .. },
            ..
        }
    )));
}
