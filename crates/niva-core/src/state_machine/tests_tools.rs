//! Task 1.10: ToolCalls, ToolsDone (incl. the `web_search` re-entry to
//! thinking), Reply/Failure and AskFollowUp rows. Spec: `assistant-core`,
//! `web-search`.

use super::tests::*;
use super::*;

fn cancel(kind: TimerKind) -> Effect {
    Effect::CancelTimer { kind }
}

fn confirm_prompt(id: u64, prompt: &str) -> Effect {
    Effect::Speak {
        id,
        kind: ReplyKind::ConfirmPrompt,
        say: Say::Text(prompt.into()),
    }
}

fn calls(verdict: ToolVerdict) -> Event {
    Event::ToolCalls { req: 2, verdict }
}

fn done(outcome: ToolsOutcome) -> Event {
    Event::ToolsDone { req: 2, outcome }
}

#[test]
fn safe_tool_calls_move_thinking_to_acting_and_run_the_tools() {
    let (s, fx) = step(&state_in(Phase::Thinking), calls(ToolVerdict::RunSafe));
    assert_eq!(s.phase, Phase::Acting);
    assert_eq!(s.request_id, Some(2));
    assert_eq!(s.timers, vec![TimerKind::Tool]);
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Llm),
            start(TimerKind::Tool),
            Effect::RunTool { req: 2 },
        ]
    );
}

#[test]
fn a_risky_call_asks_for_confirmation_by_speaking_the_prompt() {
    let verdict = ToolVerdict::Confirm {
        prompt: "Close Spotify.exe?".into(),
        click_only: false,
        cancel_text: None,
    };
    let (s, fx) = step(&state_in(Phase::Thinking), calls(verdict));
    assert_eq!(s.phase, Phase::ConfirmationNeeded);
    assert_eq!(s.request_id, Some(2));
    assert_eq!(s.speak_kind, Some(ReplyKind::ConfirmPrompt));
    assert!(!s.confirm_click_only);
    assert!(s.timers.is_empty(), "the 10 s timer starts at speak_done");
    assert_eq!(s.listen_id, None, "listen{{confirm}} opens at speak_done");
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Llm),
            confirm_prompt(100, "Close Spotify.exe?")
        ]
    );
}

#[test]
fn a_power_confirmation_is_click_only() {
    let verdict = ToolVerdict::Confirm {
        prompt: "Shut down?".into(),
        click_only: true,
        cancel_text: None,
    };
    let (s, _) = step(&state_in(Phase::Thinking), calls(verdict));
    assert!(s.confirm_click_only);
}

#[test]
fn invalid_tool_calls_end_in_error_and_nothing_executes() {
    let (s, fx) = step(&state_in(Phase::Thinking), calls(ToolVerdict::Invalid));
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.request_id, None);
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Llm),
            speak_final(100, tmpl(Template::Error))
        ]
    );
}

#[test]
fn tool_calls_are_ignored_outside_thinking() {
    for phase in Phase::ALL {
        if phase == Phase::Thinking {
            continue;
        }
        for verdict in [ToolVerdict::RunSafe, ToolVerdict::Invalid] {
            let mut s = state_in(phase);
            s.request_id = Some(2); // a matching id still leaves the cell ignored
            assert_ignored(s, calls(verdict));
        }
    }
}

#[test]
fn finished_safe_tools_end_in_success_with_the_template_reply() {
    let (s, fx) = step(&state_in(Phase::Acting), done(ToolsOutcome::Completed));
    assert_eq!(s.phase, Phase::Success);
    assert_eq!(s.request_id, None);
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Tool),
            speak_final(100, tmpl(Template::ActionDone))
        ]
    );
}

#[test]
fn a_web_search_result_re_enters_thinking_for_the_summary_call() {
    let (s, fx) = step(&state_in(Phase::Acting), done(ToolsOutcome::SearchResult));
    assert_eq!(s.phase, Phase::Thinking);
    assert_eq!(s.request_id, Some(100), "the summary is a new request");
    assert!(s.summarizing);
    assert_eq!(s.timers, vec![TimerKind::Llm]);
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Tool),
            Effect::StartRequest {
                req: 100,
                input: RequestInput::SearchSummary
            },
            start(TimerKind::Llm),
        ]
    );
}

#[test]
fn a_remaining_risky_call_moves_acting_to_confirmation() {
    let outcome = ToolsOutcome::RiskyRemaining {
        prompt: "Close Spotify.exe?".into(),
        click_only: false,
        cancel_text: None,
    };
    let (s, fx) = step(&state_in(Phase::Acting), done(outcome));
    assert_eq!(s.phase, Phase::ConfirmationNeeded);
    assert_eq!(s.request_id, Some(2));
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Tool),
            confirm_prompt(100, "Close Spotify.exe?")
        ]
    );
}

#[test]
fn a_tool_error_ends_in_error() {
    let (s, fx) = step(&state_in(Phase::Acting), done(ToolsOutcome::Failed));
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.request_id, None);
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Tool),
            speak_final(100, tmpl(Template::Error))
        ]
    );
}

#[test]
fn tools_done_is_ignored_outside_acting() {
    for phase in Phase::ALL {
        if phase == Phase::Acting {
            continue;
        }
        let mut s = state_in(phase);
        s.request_id = Some(2);
        assert_ignored(s, done(ToolsOutcome::Completed));
    }
}

#[test]
fn a_reply_in_thinking_or_acting_ends_in_success_and_cancels_its_timer() {
    let event = Event::Reply {
        req: 2,
        text: "It is 5 pm".into(),
    };
    for (phase, timer) in [
        (Phase::Thinking, TimerKind::Llm),
        (Phase::Acting, TimerKind::Tool),
    ] {
        let (s, fx) = step(&state_in(phase), event.clone());
        assert_eq!(s.phase, Phase::Success, "{phase:?}");
        assert_eq!(s.request_id, None);
        assert_eq!(
            fx,
            vec![
                cancel(timer),
                speak_final(100, Say::Text("It is 5 pm".into()))
            ],
            "{phase:?}"
        );
    }
}

#[test]
fn a_failure_in_thinking_or_acting_ends_in_error() {
    for (phase, timer) in [
        (Phase::Thinking, TimerKind::Llm),
        (Phase::Acting, TimerKind::Tool),
    ] {
        let (s, fx) = step(&state_in(phase), Event::Failure { req: 2 });
        assert_eq!(s.phase, Phase::Error, "{phase:?}");
        assert_eq!(s.request_id, None);
        assert_eq!(
            fx,
            vec![cancel(timer), speak_final(100, tmpl(Template::Error))],
            "{phase:?}"
        );
    }
}

#[test]
fn a_failed_search_summary_says_it_cannot_search_now() {
    let mut start = state_in(Phase::Thinking);
    start.summarizing = true;
    let (s, fx) = step(&start, Event::Failure { req: 2 });
    assert_eq!(s.phase, Phase::Error);
    assert!(!s.summarizing);
    assert_eq!(
        fx.last(),
        Some(&speak_final(100, tmpl(Template::SearchUnavailable)))
    );
}

#[test]
fn failure_text_ends_the_request_in_error_with_the_services_own_text() {
    for phase in [Phase::Thinking, Phase::Acting] {
        let mut start = state_in(phase);
        start.timers = running(&[TimerKind::Llm]);
        let (s, fx) = step(
            &start,
            Event::FailureText {
                req: 2,
                text: "I could not find that app.".into(),
            },
        );
        assert_eq!(s.phase, Phase::Error, "{phase:?}");
        assert_eq!(s.request_id, None, "{phase:?}: the request is over");
        assert_eq!(
            fx.last(),
            Some(&speak_final(
                100,
                Say::Text("I could not find that app.".into())
            )),
            "{phase:?}"
        );
        assert!(
            !fx.iter().any(|e| matches!(e, Effect::CancelRequest { .. })),
            "{phase:?}: nothing to cancel, the request finished"
        );
    }
}

#[test]
fn failure_text_for_an_old_request_is_dropped_and_ignored_elsewhere() {
    let mut s = state_in(Phase::Thinking);
    s.request_id = Some(2);
    assert_ignored(
        s,
        Event::FailureText {
            req: 1,
            text: "late".into(),
        },
    );
    for phase in Phase::ALL {
        if matches!(phase, Phase::Thinking | Phase::Acting) {
            continue;
        }
        let mut s = state_in(phase);
        s.request_id = Some(2);
        assert_ignored(
            s,
            Event::FailureText {
                req: 2,
                text: "x".into(),
            },
        );
    }
}

#[test]
fn reply_and_failure_are_ignored_outside_thinking_and_acting() {
    for phase in Phase::ALL {
        if matches!(phase, Phase::Thinking | Phase::Acting) {
            continue;
        }
        let mut s = state_in(phase);
        s.request_id = Some(2);
        assert_ignored(
            s.clone(),
            Event::Reply {
                req: 2,
                text: "x".into(),
            },
        );
        assert_ignored(s, Event::Failure { req: 2 });
    }
}

fn ask(question: &str) -> Event {
    Event::AskFollowUp {
        req: 2,
        question: question.into(),
        slot: slot(),
    }
}

#[test]
fn ask_follow_up_speaks_the_question_while_staying_in_thinking() {
    let (s, fx) = step(&state_in(Phase::Thinking), ask("At what time?"));
    assert_eq!(
        s.phase,
        Phase::Thinking,
        "the mascot keeps the thinking pose"
    );
    assert_eq!(s.pending, Some(slot()));
    assert_eq!(s.request_id, None);
    assert_eq!(s.speak_kind, Some(ReplyKind::AskFollowUp));
    assert!(s.timers.is_empty(), "no LLM timer and no settle timer");
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Llm),
            Effect::Speak {
                id: 100,
                kind: ReplyKind::AskFollowUp,
                say: Say::Text("At what time?".into())
            },
        ]
    );
}

#[test]
fn a_late_result_of_the_answered_request_is_dropped() {
    let (s, _) = step(&state_in(Phase::Thinking), ask("At what time?"));
    assert_ignored(
        s,
        Event::Reply {
            req: 2,
            text: "late".into(),
        },
    );
}

#[test]
fn a_second_ambiguity_in_the_follow_up_turn_fails_the_request() {
    let mut start = state_in(Phase::Thinking);
    start.followup_turn = true;
    let (s, fx) = step(&start, ask("Which one?"));
    assert_eq!(s.phase, Phase::Error);
    assert_eq!(s.pending, None);
    assert_eq!(
        fx,
        vec![
            cancel(TimerKind::Llm),
            speak_final(100, tmpl(Template::Error))
        ]
    );
}

#[test]
fn ask_follow_up_is_never_raised_from_acting_or_elsewhere() {
    for phase in Phase::ALL {
        if phase == Phase::Thinking {
            continue;
        }
        let mut s = state_in(phase);
        s.request_id = Some(2);
        assert_ignored(s, ask("x"));
    }
}
