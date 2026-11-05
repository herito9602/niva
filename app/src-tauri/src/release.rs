//! How a release build assembles its assistant (PR 2d-a).
//!
//! The model is the real Ollama (kept warm by the keeper), the PC is the
//! Windows adapter of the safe actions, and the assistant is built without
//! risky actions: a plan with a `close_app` or `power_action` call is refused
//! with a truthful "not available yet" before any confirmation prompt. When
//! Ollama is down every command gets the "can't think right now" reply; there
//! is no fallback model. Nothing here knows the dev LLM or the dry-run PC.
//!
//! This is a function of its inputs on purpose: the tests below drive it with
//! fakes, so what a release build does with a risky plan is checked without
//! Windows and without a model.

use std::sync::Arc;

use niva_core::assistant::Assistant;
use niva_core::ports::{Clock, LlmProvider, PcActions};

pub fn assistant(
    llm: Arc<dyn LlmProvider>,
    pc: Arc<dyn PcActions>,
    clock: Arc<dyn Clock>,
) -> Assistant {
    Assistant::new(llm, pc)
        .with_clock(clock)
        .without_risky_actions()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use niva_core::ports::{ChatReply, ChatRequest, LlmError, PcError, VolumeResult};
    use niva_core::replies::{self, Lang, Msg};
    use niva_core::state_machine::{Event, RequestInput, ToolVerdict, ToolsOutcome};
    use niva_core::tools::{ArgValue, ToolCall, VolumeChange};

    use super::*;
    use crate::adapters::system_clock::SystemClock;
    use crate::adapters::windows_pc::WindowsPc;
    use crate::adapters::windows_pc::fakes::{FakeApps, FakeOutput};

    struct ScriptedLlm(Mutex<Vec<Result<ChatReply, LlmError>>>);

    impl LlmProvider for ScriptedLlm {
        fn chat(&self, _request: &ChatRequest) -> Result<ChatReply, LlmError> {
            self.0.lock().unwrap().remove(0)
        }
    }

    fn call(name: &str, args: &[(&str, ArgValue)]) -> ToolCall {
        ToolCall {
            name: name.into(),
            args: args
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.clone()))
                .collect(),
        }
    }

    fn name_arg(value: &str) -> [(&str, ArgValue); 1] {
        [("name", ArgValue::Str(value.into()))]
    }

    struct Release {
        assistant: Assistant,
        apps: Arc<FakeApps>,
        output: Arc<FakeOutput>,
    }

    fn release(answer: Result<ChatReply, LlmError>) -> Release {
        let apps = Arc::new(FakeApps::default());
        let output = Arc::new(FakeOutput::default());
        let pc = Arc::new(WindowsPc::new(apps.clone(), output.clone()));
        let llm = Arc::new(ScriptedLlm(Mutex::new(vec![answer])));
        Release {
            assistant: assistant(llm, pc, Arc::new(SystemClock)),
            apps,
            output,
        }
    }

    fn calls(calls: Vec<ToolCall>) -> Result<ChatReply, LlmError> {
        Ok(ChatReply {
            text: String::new(),
            calls,
        })
    }

    fn ask(release: &Release, text: &str) -> Vec<Event> {
        release.assistant.start_request(
            1,
            &RequestInput::User {
                text: text.into(),
                slot: None,
            },
        )
    }

    fn failure_text(events: &[Event]) -> String {
        match events {
            [Event::FailureText { text, .. }] => text.clone(),
            other => panic!("expected one failure, got {other:?}"),
        }
    }

    fn untouched(release: &Release) {
        assert!(release.apps.opened.lock().unwrap().is_empty());
        assert_eq!(
            *release.apps.listed.lock().unwrap(),
            0,
            "no process listing"
        );
        assert!(release.output.changes.lock().unwrap().is_empty());
    }

    #[test]
    fn a_risky_plan_ends_in_a_truthful_not_available_reply_and_never_reaches_windows() {
        for risky in [
            call("close_app", &name_arg("spotify")),
            call("power_action", &[("action", ArgValue::Str("lock".into()))]),
            call("power_action", &[("action", ArgValue::Str("sleep".into()))]),
            call(
                "power_action",
                &[("action", ArgValue::Str("restart".into()))],
            ),
            call(
                "power_action",
                &[("action", ArgValue::Str("shutdown".into()))],
            ),
        ] {
            let release = release(calls(vec![risky.clone()]));
            let events = ask(&release, "do that please");
            assert_eq!(
                failure_text(&events),
                replies::render(&Msg::RiskyNotAvailableYet, Lang::En),
                "{risky:?}"
            );
            untouched(&release);
        }
    }

    #[test]
    fn a_risky_call_hidden_among_safe_ones_runs_nothing_at_all() {
        let release = release(calls(vec![
            call("open_app", &name_arg("notepad")),
            call("set_volume", &[("level", ArgValue::Int(10))]),
            call("close_app", &name_arg("spotify")),
        ]));
        let events = ask(&release, "do three things");
        assert_eq!(
            failure_text(&events),
            replies::render(&Msg::RiskyNotAvailableYet, Lang::En)
        );
        untouched(&release);
    }

    #[test]
    fn no_risky_plan_ever_asks_for_a_confirmation() {
        let release = release(calls(vec![call("close_app", &name_arg("spotify"))]));
        let events = ask(&release, "close spotify");
        assert!(
            !events.iter().any(|e| matches!(
                e,
                Event::ToolCalls {
                    verdict: ToolVerdict::Confirm { .. },
                    ..
                }
            )),
            "{events:?}"
        );
    }

    #[test]
    fn safe_actions_run_for_real_through_the_windows_pc() {
        let release = release(calls(vec![
            call("open_app", &name_arg("notepad")),
            call("set_volume", &[("level", ArgValue::Int(30))]),
        ]));
        let events = ask(&release, "open notepad and volume 30");
        assert!(matches!(
            events.as_slice(),
            [Event::ToolCalls {
                verdict: ToolVerdict::RunSafe,
                ..
            }]
        ));
        let done = release.assistant.run_tools(1);
        assert!(
            matches!(
                done.as_slice(),
                [Event::Reply { .. }]
                    | [Event::ToolsDone {
                        outcome: ToolsOutcome::Completed,
                        ..
                    }]
            ),
            "{done:?}"
        );
        assert_eq!(*release.apps.opened.lock().unwrap(), ["notepad"]);
        assert_eq!(
            *release.output.changes.lock().unwrap(),
            [VolumeChange::Set(30)]
        );
    }

    #[test]
    fn an_app_that_is_not_found_or_not_ready_is_said_truthfully() {
        for (error, msg) in [
            (
                PcError::NotFound,
                Msg::AppNotFound {
                    app: "photoshop".into(),
                },
            ),
            (PcError::NotReady, Msg::AppIndexLoading),
        ] {
            let release = release(calls(vec![call("open_app", &name_arg("photoshop"))]));
            *release.apps.open_result.lock().unwrap() = Some(error);
            ask(&release, "open photoshop");
            let done = release.assistant.run_tools(1);
            assert_eq!(failure_text(&done), replies::render(&msg, Lang::En));
        }
    }

    #[test]
    fn a_volume_failure_from_the_device_is_a_failure_reply() {
        let release = release(calls(vec![call(
            "set_volume",
            &[("level", ArgValue::Int(30))],
        )]));
        *release.output.result.lock().unwrap() =
            Some(Err(PcError::Failed("no audio output device".into())));
        ask(&release, "volume 30");
        let done = release.assistant.run_tools(1);
        assert_eq!(
            failure_text(&done),
            replies::render(&Msg::ToolFailed, Lang::En)
        );
        let _ = VolumeResult {
            level: 0,
            muted: false,
            clamped: false,
        };
    }

    #[test]
    fn with_ollama_down_every_command_gets_a_truthful_reply_and_no_other_model_answers() {
        for error in [LlmError::Unavailable, LlmError::Timeout] {
            let release = release(Err(error));
            let events = ask(&release, "open notepad");
            assert_eq!(
                failure_text(&events),
                replies::render(&Msg::LlmUnavailable, Lang::En)
            );
            untouched(&release);
        }
    }

    // --- the wiring itself ------------------------------------------------------
    // The release branch of `build_requests` is `cfg(not(debug_assertions))`,
    // so no test build compiles it. These are source-level assertions, in the
    // style of `capabilities_tests.rs`: the cheapest thing that fails if the
    // release path stops going through `assistant` above.

    const LIB_RS: &str = include_str!("lib.rs");
    const WORKSPACE_TOML: &str = include_str!("../../../Cargo.toml");

    fn lf(text: &str) -> String {
        text.replace("\r\n", "\n")
    }

    /// The body of the release `build_requests`.
    fn release_build_requests() -> String {
        let text = lf(LIB_RS);
        let start = text
            .find("#[cfg(not(debug_assertions))]\nfn build_requests")
            .expect("lib.rs has a release build_requests");
        let rest = &text[start..];
        let end = rest.find("\n}\n").expect("the function ends");
        rest[..end].to_string()
    }

    #[test]
    fn the_release_build_assembles_its_assistant_only_through_release_assistant() {
        let body = release_build_requests();
        assert!(
            body.contains("release::assistant("),
            "the release build must call release::assistant"
        );
        for forbidden in [
            "Assistant::new(",
            "dev::",
            "without_risky_actions",
            "DryRunPc",
        ] {
            assert!(
                !body.contains(forbidden),
                "the release build must not use {forbidden}"
            );
        }
        assert!(
            body.contains("WindowsPc::real()"),
            "the release PC is the Windows adapter"
        );
        // And no other place in lib.rs builds an assistant itself.
        assert_eq!(
            LIB_RS.matches("Assistant::new(").count(),
            0,
            "lib.rs builds assistants only through release:: and dev::"
        );
    }

    #[test]
    fn the_release_assistant_function_is_built_without_risky_actions() {
        // The behaviour is tested above with the real function; this pins that
        // the constructor line itself keeps the opt-out.
        let source = lf(include_str!("release.rs"));
        let constructor = source
            .split("pub fn assistant(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("assistant() is defined");
        assert!(constructor.contains(".without_risky_actions()"));
    }

    #[test]
    fn a_worker_panic_cannot_kill_the_release_app() {
        // `panic = "abort"` in the release profile turns the worker pool's
        // catch_unwind into a no-op: one panicking job would take the whole app
        // down. The release profile must unwind.
        let text = lf(WORKSPACE_TOML);
        let profile = text
            .split("[profile.release]")
            .nth(1)
            .expect("the workspace has a release profile")
            .split("\n[")
            .next()
            .unwrap();
        let aborts = profile.lines().any(|line| {
            let line = line.split('#').next().unwrap_or("").replace(' ', "");
            line == "panic=\"abort\""
        });
        assert!(!aborts, "[profile.release] must not set panic = \"abort\"");
    }

    #[test]
    fn reminders_and_search_are_not_available_yet_and_say_so() {
        let release = release(calls(vec![call("list_reminders", &[])]));
        ask(&release, "what are my reminders");
        let done = release.assistant.run_tools(1);
        assert_eq!(
            failure_text(&done),
            replies::render(&Msg::RemindersUnavailable, Lang::En)
        );
    }
}
