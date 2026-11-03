//! Debug window support (debug builds only: `lib.rs` declares this module
//! under `cfg(debug_assertions)`, so release builds contain neither the window
//! nor the `debug_dispatch` command).
//!
//! The window can send any reducer event. Events that carry an id the window
//! cannot know (listen, speak, request and chime ids, timer tokens) are
//! resolved against the live state when the dispatcher handles them, exactly
//! as a real sidecar or service would echo them.

use niva_core::state_machine::{Event, PendingSlot, State, TimerKind, ToolVerdict, ToolsOutcome};
use serde::Deserialize;
use tauri::{AppHandle, State as TauriState, WebviewUrl, WebviewWindowBuilder};

use crate::dispatcher::{Input, InputTx};
use crate::effect_runner::MediaMode;

pub const DEBUG_WINDOW_LABEL: &str = crate::ui_adapter::DEBUG_LABEL;
/// Longest free-form text (characters) the debug window may inject.
pub const MAX_TEXT_CHARS: usize = 500;
/// Event the dry-run adapters use to tell the debug window what they did.
pub const DEV_LOG_EVENT: &str = "niva://dev-log";
/// Prompt used by the debug window's confirmation buttons.
pub const CONFIRM_PROMPT: &str = "Close Notepad?";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictName {
    RunSafe,
    Confirm,
    ConfirmClickOnly,
    Invalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeName {
    Completed,
    SearchResult,
    RiskyRemaining,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimerName {
    NoSpeech,
    MaxUtterance,
    SttTimeout,
    Llm,
    FollowUp,
    Tool,
    Confirm,
    Settle,
    AlertEnd,
    SpeakWatchdog,
}

impl TimerName {
    fn kind(self) -> TimerKind {
        match self {
            TimerName::NoSpeech => TimerKind::NoSpeech,
            TimerName::MaxUtterance => TimerKind::MaxUtterance,
            TimerName::SttTimeout => TimerKind::SttTimeout,
            TimerName::Llm => TimerKind::Llm,
            TimerName::FollowUp => TimerKind::FollowUp,
            TimerName::Tool => TimerKind::Tool,
            TimerName::Confirm => TimerKind::Confirm,
            TimerName::Settle => TimerKind::Settle,
            TimerName::AlertEnd => TimerKind::AlertEnd,
            TimerName::SpeakWatchdog => TimerKind::SpeakWatchdog,
        }
    }
}

/// What the debug window sends: one variant per reducer event, plus two
/// debug-only controls (`Reset`, `SetMedia`).
///
/// SECURITY: never add tool-execution or confirmation variants that a release
/// webview can reach. `Accept`/`Reject` below exist only because this whole
/// module is compiled out of release builds; in a release build they must come
/// from native paths only (a user click handled by the host, or the voice
/// confirm listen), never from a command the webview can invoke.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DebugEvent {
    Wake,
    Shortcut,
    SpeechStart,
    Transcript {
        text: String,
    },
    NoSpeech,
    EmptyTranscript,
    TextSubmitted {
        text: String,
    },
    ReminderDue {
        count: u32,
        silent: bool,
    },
    ToolCalls {
        verdict: VerdictName,
    },
    ToolsDone {
        outcome: OutcomeName,
    },
    Reply {
        text: String,
    },
    Failure,
    AskFollowUp {
        question: String,
    },
    /// Click on Confirm. Both kinds of button send the id of the confirmation
    /// shown when they were clicked (`view.confirm.id`). A message with no id
    /// never matches any confirmation (it resolves to id 0).
    Accept {
        #[serde(default)]
        id: Option<u64>,
    },
    Reject {
        #[serde(default)]
        id: Option<u64>,
    },
    SpeakStarted,
    SpeakDone,
    TtsFailed,
    Watchdog {
        kind: TimerName,
    },
    ChimeDone,
    VoiceReady {
        ready: bool,
    },
    Dismiss,
    /// Back to a fresh idle state (voice flag kept).
    Reset,
    /// Hold the stubbed audio (`true`) or let it end at once (`false`).
    SetMedia {
        held: bool,
    },
}

impl DebugEvent {
    /// The dispatcher input for this message.
    pub fn into_input(self) -> Input {
        match self {
            DebugEvent::Reset => Input::Reset,
            DebugEvent::SetMedia { held: true } => Input::SetMedia(MediaMode::Held),
            DebugEvent::SetMedia { held: false } => Input::SetMedia(MediaMode::Instant),
            event => Input::Resolve(Box::new(move |state| event.resolve(state))),
        }
    }

    /// The reducer event for this message against the current state; `None`
    /// for the two controls. An id with no live target resolves to 0, which no
    /// allocator hands out, so the reducer drops it as stale.
    pub fn resolve(&self, state: &State) -> Option<Event> {
        let listen = state.listen_id.unwrap_or(0);
        let speak = state.speak_id.unwrap_or(0);
        let req = state.request_id.unwrap_or(0);
        Some(match self {
            DebugEvent::Wake => Event::Wake,
            DebugEvent::Shortcut => Event::Shortcut,
            DebugEvent::SpeechStart => Event::SpeechStart { id: listen },
            DebugEvent::Transcript { text } => Event::Transcript {
                id: listen,
                text: cap(text),
            },
            DebugEvent::NoSpeech => Event::NoSpeech { id: listen },
            DebugEvent::EmptyTranscript => Event::EmptyTranscript { id: listen },
            DebugEvent::TextSubmitted { text } => Event::TextSubmitted { text: cap(text) },
            DebugEvent::ReminderDue { count, silent } => Event::ReminderDue {
                count: *count,
                silent: *silent,
            },
            DebugEvent::ToolCalls { verdict } => Event::ToolCalls {
                req,
                verdict: match verdict {
                    VerdictName::RunSafe => ToolVerdict::RunSafe,
                    VerdictName::Invalid => ToolVerdict::Invalid,
                    VerdictName::Confirm => confirm(false),
                    VerdictName::ConfirmClickOnly => confirm(true),
                },
            },
            DebugEvent::ToolsDone { outcome } => Event::ToolsDone {
                req,
                outcome: match outcome {
                    OutcomeName::Completed => ToolsOutcome::Completed,
                    OutcomeName::SearchResult => ToolsOutcome::SearchResult,
                    OutcomeName::Failed => ToolsOutcome::Failed,
                    OutcomeName::RiskyRemaining => ToolsOutcome::RiskyRemaining {
                        prompt: CONFIRM_PROMPT.into(),
                        click_only: false,
                        cancel_text: None,
                    },
                },
            },
            DebugEvent::Reply { text } => Event::Reply {
                req,
                text: cap(text),
            },
            DebugEvent::Failure => Event::Failure { req },
            DebugEvent::AskFollowUp { question } => Event::AskFollowUp {
                req,
                question: cap(question),
                slot: PendingSlot::Reminder { text: None },
            },
            // An id-less click never matches: 0 is never allocated. The id
            // must be fixed when the button is clicked, not here.
            DebugEvent::Accept { id } => Event::Accept {
                id: id.unwrap_or(0),
            },
            DebugEvent::Reject { id } => Event::Reject {
                id: id.unwrap_or(0),
            },
            DebugEvent::SpeakStarted => Event::SpeakStarted { id: speak },
            DebugEvent::SpeakDone => Event::SpeakDone { id: speak },
            DebugEvent::TtsFailed => Event::TtsFailed { id: speak },
            DebugEvent::Watchdog { kind } => {
                let kind = kind.kind();
                let token = if kind == TimerKind::SpeakWatchdog {
                    speak
                } else {
                    state
                        .timers
                        .iter()
                        .find(|t| t.kind == kind)
                        .map_or(0, |t| t.token)
                };
                Event::Watchdog { kind, token }
            }
            DebugEvent::ChimeDone => Event::ChimeDone {
                id: state.chime_id.unwrap_or(0),
            },
            DebugEvent::VoiceReady { ready } => Event::VoiceReadinessChanged { ready: *ready },
            DebugEvent::Dismiss => Event::Dismiss,
            DebugEvent::Reset | DebugEvent::SetMedia { .. } => return None,
        })
    }
}

/// Free-form debug text, limited to [`MAX_TEXT_CHARS`] characters.
fn cap(text: &str) -> String {
    text.chars().take(MAX_TEXT_CHARS).collect()
}

fn confirm(click_only: bool) -> ToolVerdict {
    ToolVerdict::Confirm {
        prompt: CONFIRM_PROMPT.into(),
        click_only,
        cancel_text: None,
    }
}

/// Command of the debug window: feeds one event into the dispatcher.
#[tauri::command]
pub fn debug_dispatch(event: DebugEvent, tx: TauriState<'_, InputTx>) -> Result<(), String> {
    tx.send(event.into_input()).map_err(|err| err.to_string())
}

/// Creates the debug window (a normal, focusable window, unlike the island).
pub fn open_window(app: &AppHandle) -> tauri::Result<()> {
    WebviewWindowBuilder::new(
        app,
        DEBUG_WINDOW_LABEL,
        WebviewUrl::App("index.html?window=debug".into()),
    )
    .title("Niva debug")
    .inner_size(620.0, 760.0)
    .build()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use niva_core::state_machine::{
        Event, PendingSlot, State, TimerKind, ToolVerdict, ToolsOutcome, reduce,
    };

    use super::*;
    use crate::dispatcher::Input;

    fn parse(json: &str) -> DebugEvent {
        serde_json::from_str(json).expect("valid debug event")
    }

    fn after(events: &[Event]) -> State {
        let mut state = State::default();
        for event in events {
            state = reduce(state, event.clone()).0;
        }
        state
    }

    fn ready_listening() -> State {
        after(&[
            Event::VoiceReadinessChanged { ready: true },
            Event::Shortcut,
        ])
    }

    #[test]
    fn events_deserialize_from_tagged_json() {
        assert_eq!(
            parse(r#"{"type":"shortcut"}"#)
                .resolve(&State::default())
                .unwrap(),
            Event::Shortcut
        );
        assert_eq!(
            parse(r#"{"type":"voice_ready","ready":false}"#)
                .resolve(&State::default())
                .unwrap(),
            Event::VoiceReadinessChanged { ready: false }
        );
        assert_eq!(
            parse(r#"{"type":"text_submitted","text":"open notepad"}"#)
                .resolve(&State::default())
                .unwrap(),
            Event::TextSubmitted {
                text: "open notepad".into()
            }
        );
        assert_eq!(
            parse(r#"{"type":"reminder_due","count":2,"silent":true}"#)
                .resolve(&State::default())
                .unwrap(),
            Event::ReminderDue {
                count: 2,
                silent: true
            }
        );
        assert!(serde_json::from_str::<DebugEvent>(r#"{"type":"nope"}"#).is_err());
    }

    #[test]
    fn listen_events_carry_the_open_listen_id() {
        let state = ready_listening();
        let id = state.listen_id.unwrap();
        assert_eq!(
            parse(r#"{"type":"transcript","text":"hi"}"#)
                .resolve(&state)
                .unwrap(),
            Event::Transcript {
                id,
                text: "hi".into()
            }
        );
        assert_eq!(
            parse(r#"{"type":"speech_start"}"#).resolve(&state).unwrap(),
            Event::SpeechStart { id }
        );
        assert_eq!(
            parse(r#"{"type":"no_speech"}"#).resolve(&state).unwrap(),
            Event::NoSpeech { id }
        );
        assert_eq!(
            parse(r#"{"type":"empty_transcript"}"#)
                .resolve(&state)
                .unwrap(),
            Event::EmptyTranscript { id }
        );
    }

    #[test]
    fn id_events_without_an_open_target_use_id_zero_which_the_reducer_drops() {
        let idle = State::default();
        let event = parse(r#"{"type":"transcript","text":"hi"}"#)
            .resolve(&idle)
            .unwrap();
        assert_eq!(
            event,
            Event::Transcript {
                id: 0,
                text: "hi".into()
            }
        );
        let (state, fx) = reduce(idle.clone(), event);
        assert_eq!(state, idle);
        assert!(fx.is_empty());
        assert_eq!(
            parse(r#"{"type":"speak_done"}"#).resolve(&idle).unwrap(),
            Event::SpeakDone { id: 0 }
        );
        assert_eq!(
            parse(r#"{"type":"chime_done"}"#).resolve(&idle).unwrap(),
            Event::ChimeDone { id: 0 }
        );
        assert_eq!(
            parse(r#"{"type":"failure"}"#).resolve(&idle).unwrap(),
            Event::Failure { req: 0 }
        );
    }

    #[test]
    fn request_events_carry_the_request_id() {
        let mut state = ready_listening();
        let id = state.listen_id.unwrap();
        state = reduce(
            state,
            Event::Transcript {
                id,
                text: "x".into(),
            },
        )
        .0;
        let req = state.request_id.unwrap();
        assert_eq!(
            parse(r#"{"type":"reply","text":"Done"}"#)
                .resolve(&state)
                .unwrap(),
            Event::Reply {
                req,
                text: "Done".into()
            }
        );
        assert_eq!(
            parse(r#"{"type":"failure"}"#).resolve(&state).unwrap(),
            Event::Failure { req }
        );
        assert_eq!(
            parse(r#"{"type":"ask_follow_up","question":"When?"}"#)
                .resolve(&state)
                .unwrap(),
            Event::AskFollowUp {
                req,
                question: "When?".into(),
                slot: PendingSlot::Reminder { text: None }
            }
        );
    }

    #[test]
    fn tool_events_map_their_variants() {
        let state = after(&[]);
        let tool = |v: &str| {
            parse(&format!(r#"{{"type":"tool_calls","verdict":"{v}"}}"#))
                .resolve(&state)
                .unwrap()
        };
        assert_eq!(
            tool("run_safe"),
            Event::ToolCalls {
                req: 0,
                verdict: ToolVerdict::RunSafe
            }
        );
        assert_eq!(
            tool("invalid"),
            Event::ToolCalls {
                req: 0,
                verdict: ToolVerdict::Invalid
            }
        );
        assert_eq!(
            tool("confirm"),
            Event::ToolCalls {
                req: 0,
                verdict: ToolVerdict::Confirm {
                    prompt: CONFIRM_PROMPT.into(),
                    click_only: false,
                    cancel_text: None,
                }
            }
        );
        assert_eq!(
            tool("confirm_click_only"),
            Event::ToolCalls {
                req: 0,
                verdict: ToolVerdict::Confirm {
                    prompt: CONFIRM_PROMPT.into(),
                    click_only: true,
                    cancel_text: None,
                }
            }
        );
        let done = |o: &str| {
            parse(&format!(r#"{{"type":"tools_done","outcome":"{o}"}}"#))
                .resolve(&state)
                .unwrap()
        };
        assert_eq!(
            done("completed"),
            Event::ToolsDone {
                req: 0,
                outcome: ToolsOutcome::Completed
            }
        );
        assert_eq!(
            done("search_result"),
            Event::ToolsDone {
                req: 0,
                outcome: ToolsOutcome::SearchResult
            }
        );
        assert_eq!(
            done("failed"),
            Event::ToolsDone {
                req: 0,
                outcome: ToolsOutcome::Failed
            }
        );
        assert_eq!(
            done("risky_remaining"),
            Event::ToolsDone {
                req: 0,
                outcome: ToolsOutcome::RiskyRemaining {
                    prompt: CONFIRM_PROMPT.into(),
                    click_only: false,
                    cancel_text: None,
                }
            }
        );
    }

    #[test]
    fn a_click_carries_the_confirmation_id_it_was_rendered_for() {
        let state = after(&[
            Event::VoiceReadinessChanged { ready: true },
            Event::Shortcut,
        ]);
        // With an id: sent as is, even if it is not the open one (the reducer
        // drops it as stale).
        assert_eq!(
            parse(r#"{"type":"accept","id":41}"#)
                .resolve(&state)
                .unwrap(),
            Event::Accept { id: 41 }
        );
        assert_eq!(
            parse(r#"{"type":"reject","id":41}"#)
                .resolve(&state)
                .unwrap(),
            Event::Reject { id: 41 }
        );
        // An id-less click never matches (0 is never allocated), open
        // confirmation or not: the id is fixed when the button is clicked (the
        // frontend sends the rendered one), never when the event is processed.
        assert_eq!(
            parse(r#"{"type":"accept"}"#).resolve(&state).unwrap(),
            Event::Accept { id: 0 }
        );
        let mut confirming = state;
        confirming.confirm_id = Some(9);
        assert_eq!(
            parse(r#"{"type":"reject"}"#).resolve(&confirming).unwrap(),
            Event::Reject { id: 0 }
        );
        assert_eq!(
            parse(r#"{"type":"accept"}"#).resolve(&confirming).unwrap(),
            Event::Accept { id: 0 }
        );
    }

    #[test]
    fn watchdog_carries_the_token_of_the_running_timer() {
        let state = ready_listening();
        let token = state
            .timers
            .iter()
            .find(|t| t.kind == TimerKind::NoSpeech)
            .expect("no-speech timer")
            .token;
        assert_eq!(
            parse(r#"{"type":"watchdog","kind":"no_speech"}"#)
                .resolve(&state)
                .unwrap(),
            Event::Watchdog {
                kind: TimerKind::NoSpeech,
                token
            }
        );
        // Not running: token 0, which the reducer treats as stale.
        assert_eq!(
            parse(r#"{"type":"watchdog","kind":"settle"}"#)
                .resolve(&state)
                .unwrap(),
            Event::Watchdog {
                kind: TimerKind::Settle,
                token: 0
            }
        );
    }

    #[test]
    fn speak_watchdog_uses_the_speak_id_as_token() {
        let mut state = after(&[
            Event::VoiceReadinessChanged { ready: true },
            Event::Shortcut,
        ]);
        let id = state.listen_id.unwrap();
        state = reduce(
            state,
            Event::Transcript {
                id,
                text: "x".into(),
            },
        )
        .0;
        let req = state.request_id.unwrap();
        state = reduce(
            state,
            Event::Reply {
                req,
                text: "ok".into(),
            },
        )
        .0;
        let speak = state.speak_id.expect("speaking");
        assert_eq!(
            parse(r#"{"type":"watchdog","kind":"speak_watchdog"}"#)
                .resolve(&state)
                .unwrap(),
            Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token: speak
            }
        );
    }

    #[test]
    fn every_timer_kind_name_parses() {
        for (name, kind) in [
            ("no_speech", TimerKind::NoSpeech),
            ("max_utterance", TimerKind::MaxUtterance),
            ("stt_timeout", TimerKind::SttTimeout),
            ("llm", TimerKind::Llm),
            ("follow_up", TimerKind::FollowUp),
            ("tool", TimerKind::Tool),
            ("confirm", TimerKind::Confirm),
            ("settle", TimerKind::Settle),
            ("alert_end", TimerKind::AlertEnd),
            ("speak_watchdog", TimerKind::SpeakWatchdog),
        ] {
            let event = parse(&format!(r#"{{"type":"watchdog","kind":"{name}"}}"#))
                .resolve(&State::default())
                .unwrap();
            assert!(
                matches!(event, Event::Watchdog { kind: k, .. } if k == kind),
                "{name}"
            );
        }
        assert_eq!(TimerKind::ALL.len(), 10);
    }

    #[test]
    fn reset_and_media_map_to_their_inputs_and_events_to_resolvers() {
        assert!(matches!(
            parse(r#"{"type":"reset"}"#).into_input(),
            Input::Reset
        ));
        assert!(matches!(
            parse(r#"{"type":"set_media","held":true}"#).into_input(),
            Input::SetMedia(crate::effect_runner::MediaMode::Held)
        ));
        assert!(matches!(
            parse(r#"{"type":"set_media","held":false}"#).into_input(),
            Input::SetMedia(crate::effect_runner::MediaMode::Instant)
        ));
        let Input::Resolve(resolve) = parse(r#"{"type":"dismiss"}"#).into_input() else {
            panic!("expected a resolver");
        };
        assert_eq!(resolve(&State::default()), Some(Event::Dismiss));
    }

    #[test]
    fn free_form_text_is_capped_in_characters() {
        let long = "ñ".repeat(MAX_TEXT_CHARS + 50);
        for json in [
            serde_json::json!({"type": "transcript", "text": long}),
            serde_json::json!({"type": "text_submitted", "text": long}),
            serde_json::json!({"type": "reply", "text": long}),
            serde_json::json!({"type": "ask_follow_up", "question": long}),
        ] {
            let event: DebugEvent = serde_json::from_value(json).unwrap();
            let text = match event.resolve(&ready_listening()).unwrap() {
                Event::Transcript { text, .. }
                | Event::TextSubmitted { text }
                | Event::Reply { text, .. } => text,
                Event::AskFollowUp { question, .. } => question,
                other => panic!("unexpected {other:?}"),
            };
            assert_eq!(text.chars().count(), MAX_TEXT_CHARS);
        }
        let short = parse(r#"{"type":"text_submitted","text":"abc"}"#)
            .resolve(&State::default())
            .unwrap();
        assert_eq!(short, Event::TextSubmitted { text: "abc".into() });
    }

    #[test]
    fn a_control_variant_in_a_resolver_yields_no_event_instead_of_panicking() {
        let Input::Resolve(resolve) = (DebugEvent::Dismiss).into_input() else {
            panic!("expected a resolver");
        };
        assert_eq!(resolve(&State::default()), Some(Event::Dismiss));
        assert_eq!(DebugEvent::Reset.resolve(&State::default()), None);
        assert_eq!(
            DebugEvent::SetMedia { held: true }.resolve(&State::default()),
            None
        );
    }

    /// `events.json` drives both the vitest checks and the window; every entry
    /// must be a message the command accepts.
    #[test]
    fn every_button_and_script_event_of_the_window_parses() {
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../src/debug/events.json")).unwrap();
        let buttons = data["buttons"].as_array().unwrap();
        assert!(buttons.len() >= 22);
        for button in buttons {
            serde_json::from_value::<DebugEvent>(button["event"].clone())
                .unwrap_or_else(|e| panic!("button {}: {e}", button["label"]));
        }
        let scripts = data["scripts"].as_object().unwrap();
        assert_eq!(scripts.len(), 8);
        for (state, script) in scripts {
            let events = script.as_array().unwrap();
            assert!(!events.is_empty(), "{state}");
            for event in events {
                serde_json::from_value::<DebugEvent>(event.clone())
                    .unwrap_or_else(|e| panic!("script {state}: {e}"));
            }
        }
    }

    /// Replays each script through the real reducer and checks it ends in the
    /// state it is named after (the buttons are only useful if they do).
    #[test]
    fn each_state_script_ends_in_its_state() {
        use niva_core::state_machine::Phase;
        let data: serde_json::Value =
            serde_json::from_str(include_str!("../../src/debug/events.json")).unwrap();
        let expect = [
            ("idle", Phase::Idle),
            ("listening", Phase::Listening),
            ("thinking", Phase::Thinking),
            ("acting", Phase::Acting),
            ("confirmation-needed", Phase::ConfirmationNeeded),
            ("success", Phase::Success),
            ("error", Phase::Error),
            ("reminder-alert", Phase::ReminderAlert),
        ];
        for (name, phase) in expect {
            let mut state = State::default();
            for event in data["scripts"][name].as_array().unwrap() {
                let event: DebugEvent = serde_json::from_value(event.clone()).unwrap();
                match event.resolve(&state) {
                    Some(e) => state = reduce(state, e).0,
                    None => state = State::default(), // reset
                }
            }
            assert_eq!(state.phase, phase, "{name}");
        }
    }
}
