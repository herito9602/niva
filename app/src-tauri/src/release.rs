//! How a release build assembles its assistant (PR 2d-a, completed in 2d-b).
//!
//! The model is the real Ollama (kept warm by the keeper) and the PC is the
//! Windows adapter of every action. A risky plan (`close_app`, `power_action`)
//! asks for a confirmation like in debug, and the confirmation keys answer it
//! (`confirm_keys.rs`: Esc rejects, the Ctrl+Shift+Enter chord accepts a restart
//! or a shutdown, voice answers the rest); the safety gate (`Approved<T>`)
//! decides what may run. When Ollama is down every command gets the "can't think
//! right now" reply; there is no fallback model. Nothing here knows the dev LLM
//! or the dry-run PC.
//!
//! This is a function of its inputs on purpose: the tests below drive it with
//! fakes, so what a release build does with a risky plan is checked without
//! Windows and without a model.

use std::sync::Arc;

use niva_core::assistant::Assistant;
use niva_core::ports::{Clock, LlmProvider, PcActions, SearchProvider};

/// `search` is the real Tavily provider in a shipped build; without a key it
/// answers "not configured" and the reply says so.
pub fn assistant(
    llm: Arc<dyn LlmProvider>,
    pc: Arc<dyn PcActions>,
    clock: Arc<dyn Clock>,
    search: Arc<dyn SearchProvider>,
) -> Assistant {
    Assistant::new(llm, pc)
        .with_clock(clock)
        .with_search(search)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use niva_core::assistant::ConfirmInput;
    use niva_core::ports::{ChatReply, ChatRequest, CloseOutcome, LlmError, PcError};
    use niva_core::replies::{self, Lang, Msg};
    use niva_core::state_machine::{Event, RequestInput, ToolVerdict, ToolsOutcome};
    use niva_core::tools::{ArgValue, PowerKind, ToolCall, VolumeChange};

    use super::*;
    use crate::adapters::search::{ApiKey, NetError, Reply, SearchTransport, TavilySearch};
    use crate::adapters::system_clock::SystemClock;
    use crate::adapters::windows_pc::WindowsPc;
    use crate::adapters::windows_pc::fakes::{FakeApps, FakeOutput, FakePower};

    /// A transport that fails the test if a release test ever reaches the network.
    struct NoNetwork;

    impl SearchTransport for NoNetwork {
        fn post(
            &self,
            _key: &ApiKey,
            _body: &str,
            _timeout: std::time::Duration,
        ) -> Result<Reply, NetError> {
            panic!("a release test made a search call");
        }
    }

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

    fn power(action: &str) -> ToolCall {
        call("power_action", &[("action", ArgValue::Str(action.into()))])
    }

    struct Release {
        assistant: Assistant,
        apps: Arc<FakeApps>,
        output: Arc<FakeOutput>,
        power: Arc<FakePower>,
    }

    fn release(answer: Result<ChatReply, LlmError>) -> Release {
        let apps = Arc::new(FakeApps::default());
        let output = Arc::new(FakeOutput::default());
        let power = Arc::new(FakePower::default());
        let pc = Arc::new(WindowsPc::new(apps.clone(), output.clone(), power.clone()));
        let llm = Arc::new(ScriptedLlm(Mutex::new(vec![answer])));
        Release {
            assistant: assistant(
                llm,
                pc,
                Arc::new(SystemClock),
                Arc::new(TavilySearch::new(Arc::new(NoNetwork), None)),
            ),
            apps,
            output,
            power,
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
                lang: None,
            },
        )
    }

    fn failure_text(events: &[Event]) -> String {
        match events {
            [Event::FailureText { text, .. }] => text.clone(),
            other => panic!("expected one failure, got {other:?}"),
        }
    }

    fn reply_text(events: &[Event]) -> String {
        match events {
            [Event::Reply { text, .. }] => text.clone(),
            other => panic!("expected one reply, got {other:?}"),
        }
    }

    /// No adapter acted: nothing opened, closed, changed or powered. (The
    /// process list may have been read: that is how a close is resolved.)
    fn untouched(release: &Release) {
        assert!(release.apps.opened.lock().unwrap().is_empty());
        assert!(release.apps.closed.lock().unwrap().is_empty());
        assert!(release.power.runs.lock().unwrap().is_empty());
        assert!(release.output.changes.lock().unwrap().is_empty());
    }

    // --- risky plans: a confirmation, then the real action -------------------------

    #[test]
    fn a_risky_plan_asks_for_a_confirmation_and_touches_nothing_before_the_click() {
        for (risky, click_only) in [
            (call("close_app", &name_arg("spotify")), false),
            (power("lock"), false),
            (power("sleep"), false),
            (power("restart"), true),
            (power("shutdown"), true),
        ] {
            let release = release(calls(vec![risky.clone()]));
            let events = ask(&release, "do that please");
            match events.as_slice() {
                [
                    Event::ToolCalls {
                        verdict:
                            ToolVerdict::Confirm {
                                click_only: only, ..
                            },
                        ..
                    },
                ] => assert_eq!(*only, click_only, "{risky:?}"),
                other => panic!("{risky:?}: expected a confirmation, got {other:?}"),
            }
            untouched(&release);
        }
    }

    #[test]
    fn a_confirmed_close_closes_the_confirmed_exe_and_says_so() {
        let release = release(calls(vec![call("close_app", &name_arg("spotify"))]));
        ask(&release, "close spotify");
        release
            .assistant
            .record_confirmation(ConfirmInput::Click(true));
        let done = release.assistant.run_tools(1);
        assert_eq!(
            reply_text(&done),
            replies::render(
                &Msg::Closed {
                    exe: "Spotify.exe".into()
                },
                Lang::En
            )
        );
        assert_eq!(*release.apps.closed.lock().unwrap(), ["Spotify.exe"]);
    }

    #[test]
    fn a_close_that_leaves_the_window_open_says_the_app_did_not_close() {
        let release = release(calls(vec![call("close_app", &name_arg("spotify"))]));
        *release.apps.close_result.lock().unwrap() = Some(Ok(CloseOutcome::StillRunning));
        ask(&release, "close spotify");
        release
            .assistant
            .record_confirmation(ConfirmInput::Click(true));
        let done = release.assistant.run_tools(1);
        assert_eq!(
            reply_text(&done),
            replies::render(
                &Msg::StillRunning {
                    exe: "Spotify.exe".into()
                },
                Lang::En
            )
        );
    }

    #[test]
    fn an_unreadable_process_list_is_an_error_reply_and_nothing_is_asked() {
        let release = release(calls(vec![call("close_app", &name_arg("spotify"))]));
        *release.apps.process_error.lock().unwrap() = Some(PcError::Failed("no snapshot".into()));
        let events = ask(&release, "close spotify");
        assert_eq!(
            failure_text(&events),
            replies::render(&Msg::ToolFailed, Lang::En)
        );
        untouched(&release);
    }

    #[test]
    fn a_confirmed_lock_runs_through_the_power_control() {
        let release = release(calls(vec![power("lock")]));
        ask(&release, "lock the pc");
        release
            .assistant
            .record_confirmation(ConfirmInput::Click(true));
        let done = release.assistant.run_tools(1);
        assert_eq!(
            reply_text(&done),
            replies::render(
                &Msg::PowerDone {
                    action: PowerKind::Lock
                },
                Lang::En
            )
        );
        assert_eq!(*release.power.runs.lock().unwrap(), [PowerKind::Lock]);
    }

    #[test]
    fn restart_and_shutdown_cannot_be_confirmed_by_voice() {
        for action in ["restart", "shutdown"] {
            let release = release(calls(vec![power(action)]));
            ask(&release, "do it");
            release
                .assistant
                .record_confirmation(ConfirmInput::Voice("yes".into()));
            let done = release.assistant.run_tools(1);
            assert_eq!(
                failure_text(&done),
                replies::render(&Msg::ToolFailed, Lang::En),
                "{action}"
            );
            untouched(&release);
        }
    }

    #[test]
    fn a_confirmed_restart_and_shutdown_run_through_the_power_control() {
        for (action, kind) in [
            ("restart", PowerKind::Restart),
            ("shutdown", PowerKind::Shutdown),
        ] {
            let release = release(calls(vec![power(action)]));
            ask(&release, "do it");
            release
                .assistant
                .record_confirmation(ConfirmInput::Click(true));
            let done = release.assistant.run_tools(1);
            assert_eq!(
                reply_text(&done),
                replies::render(&Msg::PowerDone { action: kind }, Lang::En)
            );
            assert_eq!(*release.power.runs.lock().unwrap(), [kind]);
        }
    }

    #[test]
    fn a_failed_power_action_is_a_failure_reply() {
        let release = release(calls(vec![power("restart")]));
        *release.power.result.lock().unwrap() = Some(Err(PcError::Failed("denied".into())));
        ask(&release, "restart");
        release
            .assistant
            .record_confirmation(ConfirmInput::Click(true));
        let done = release.assistant.run_tools(1);
        assert_eq!(
            failure_text(&done),
            replies::render(&Msg::ToolFailed, Lang::En)
        );
    }

    #[test]
    fn a_confirmed_sleep_waits_for_its_countdown_and_only_then_reaches_the_pc() {
        let release = release(calls(vec![power("sleep")]));
        ask(&release, "sleep");
        release
            .assistant
            .record_confirmation(ConfirmInput::Click(true));
        let done = release.assistant.run_tools(1);
        assert!(
            matches!(
                done.as_slice(),
                [Event::ToolsDone {
                    outcome: ToolsOutcome::SleepCountdown { .. },
                    ..
                }]
            ),
            "{done:?}"
        );
        assert!(release.power.runs.lock().unwrap().is_empty());
        let finished = release.assistant.finish_countdown(1);
        assert_eq!(
            reply_text(&finished),
            replies::render(
                &Msg::PowerDone {
                    action: PowerKind::Sleep
                },
                Lang::En
            )
        );
        assert_eq!(*release.power.runs.lock().unwrap(), [PowerKind::Sleep]);
    }

    #[test]
    fn a_pc_without_modern_standby_says_sleep_is_not_available() {
        let release = release(calls(vec![power("sleep")]));
        *release.power.result.lock().unwrap() = Some(Err(PcError::NotSupported));
        ask(&release, "sleep");
        release
            .assistant
            .record_confirmation(ConfirmInput::Click(true));
        release.assistant.run_tools(1);
        let finished = release.assistant.finish_countdown(1);
        assert_eq!(
            failure_text(&finished),
            replies::render(&Msg::SleepNotAvailable, Lang::En)
        );
    }

    // --- the safe actions and the model ---------------------------------------------

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
        for forbidden in ["Assistant::new(", "dev::", "DryRunPc"] {
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
    fn the_core_has_no_opt_out_of_risky_actions_any_more() {
        // The opt-out of PR 2d-a is gone: a release build asks for confirmations
        // and the gate decides. Nothing may bring a switch back unnoticed.
        let source = lf(include_str!("release.rs"));
        let constructor = source
            .split("pub fn assistant(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("assistant() is defined");
        let needle = ["without", "_risky", "_actions"].concat();
        assert!(!constructor.contains(&needle));
        assert!(!lf(LIB_RS).contains(&needle));
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
    fn reminders_are_not_available_yet_and_say_so() {
        let release = release(calls(vec![call("list_reminders", &[])]));
        ask(&release, "what are my reminders");
        let done = release.assistant.run_tools(1);
        assert_eq!(
            failure_text(&done),
            replies::render(&Msg::RemindersUnavailable, Lang::En)
        );
    }

    #[test]
    fn a_search_without_a_key_says_it_needs_one_and_makes_no_call() {
        // `NoNetwork` panics if the provider tried to send anything.
        for (utterance, lang) in [
            ("search rust", Lang::En),
            ("busca rust por favor", Lang::Es),
        ] {
            let release = release(calls(vec![call(
                "web_search",
                &[("query", ArgValue::Str("rust".into()))],
            )]));
            ask(&release, utterance);
            let done = release.assistant.run_tools(1);
            assert_eq!(
                failure_text(&done),
                replies::render(&Msg::SearchNotConfigured, lang),
                "{utterance}"
            );
            untouched(&release);
        }
    }

    #[test]
    fn the_release_build_uses_the_real_search_provider() {
        let body = release_build_requests();
        assert!(
            body.contains("TavilySearch::from_environment()"),
            "the release build searches through Tavily with the user's key"
        );
    }
}
