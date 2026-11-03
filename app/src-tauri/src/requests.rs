//! Who answers the reducer's request effects.
//!
//! The reducer emits `StartRequest`, `RunTool` and `CancelRequest`; this
//! module is the host side of those three. A [`Requests`] value also sees
//! every event before the reducer does (to record confirmation answers) and
//! knows the language replies are shown in.
//!
//! - [`Unavailable`] is what release builds use until the real Ollama and
//!   Windows adapters exist (PRs 2c and 2d): every request ends in a "not
//!   available yet" error and nothing runs. It contains no LLM, no PC code.
//! - [`AssistantRequests`] (debug builds and tests) runs a core
//!   [`Assistant`] on a worker thread and feeds its events back.

use niva_core::replies::Lang;
#[cfg(any(test, not(debug_assertions)))]
use niva_core::replies::{self, DEFAULT_LANG, Msg};
use niva_core::state_machine::{Event, RequestInput, State};

use crate::effect_runner::EventSink;

pub trait Requests: Send + Sync {
    /// Starts the LLM call of `req`. Never blocks: the result comes back as
    /// events through the dispatcher queue.
    fn start(&self, req: u64, input: RequestInput);
    /// Runs the validated tools of `req` (after a safe verdict, or after a
    /// confirmation was accepted).
    fn run_tools(&self, req: u64);
    /// The reducer finished or dropped `req`.
    fn cancel(&self, req: u64);
    /// Sees each event with the state just before the reducer handles it.
    fn observe(&self, state: &State, event: &Event);
    /// Language of the current request, for the reducer's reply templates.
    fn lang(&self) -> Lang;
}

/// Release builds before slice 2c/2d: every request fails safely.
#[cfg(any(test, not(debug_assertions)))]
pub struct Unavailable {
    sink: EventSink,
}

#[cfg(any(test, not(debug_assertions)))]
impl Unavailable {
    pub fn new(sink: EventSink) -> Self {
        Unavailable { sink }
    }
}

#[cfg(any(test, not(debug_assertions)))]
impl Requests for Unavailable {
    fn start(&self, req: u64, _input: RequestInput) {
        (self.sink)(Event::FailureText {
            req,
            text: replies::render(&Msg::NotAvailableYet, DEFAULT_LANG),
        });
    }

    fn run_tools(&self, _req: u64) {}

    fn cancel(&self, _req: u64) {}

    fn observe(&self, _state: &State, _event: &Event) {}

    fn lang(&self) -> Lang {
        DEFAULT_LANG
    }
}

/// Does nothing (tests of the runner and the dispatcher that only look at
/// the reducer's own behavior).
#[cfg(test)]
pub struct Ignore;

#[cfg(test)]
impl Requests for Ignore {
    fn start(&self, _req: u64, _input: RequestInput) {}
    fn run_tools(&self, _req: u64) {}
    fn cancel(&self, _req: u64) {}
    fn observe(&self, _state: &State, _event: &Event) {}
    fn lang(&self) -> Lang {
        DEFAULT_LANG
    }
}

/// Runs a job off the dispatcher task.
#[cfg(any(test, debug_assertions))]
pub type Spawn = Box<dyn Fn(Box<dyn FnOnce() + Send>) + Send + Sync>;

/// Drives a core [`Assistant`](niva_core::assistant::Assistant): the LLM call
/// and the tools block, so each runs on its own thread; the events they
/// produce go back through `sink`.
#[cfg(any(test, debug_assertions))]
pub struct AssistantRequests {
    assistant: std::sync::Arc<niva_core::assistant::Assistant>,
    sink: EventSink,
    spawn: Spawn,
}

#[cfg(any(test, debug_assertions))]
impl AssistantRequests {
    pub fn new(assistant: niva_core::assistant::Assistant, sink: EventSink) -> Self {
        Self::with_spawn(
            assistant,
            sink,
            Box::new(|job| {
                std::thread::spawn(job);
            }),
        )
    }

    /// For tests: `spawn` may run the job inline.
    pub fn with_spawn(
        assistant: niva_core::assistant::Assistant,
        sink: EventSink,
        spawn: Spawn,
    ) -> Self {
        AssistantRequests {
            assistant: std::sync::Arc::new(assistant),
            sink,
            spawn,
        }
    }
}

#[cfg(any(test, debug_assertions))]
impl Requests for AssistantRequests {
    fn start(&self, req: u64, input: RequestInput) {
        let assistant = self.assistant.clone();
        let sink = self.sink.clone();
        (self.spawn)(Box::new(move || {
            for event in assistant.start_request(req, &input) {
                sink(event);
            }
        }));
    }

    fn run_tools(&self, req: u64) {
        let assistant = self.assistant.clone();
        let sink = self.sink.clone();
        (self.spawn)(Box::new(move || {
            for event in assistant.run_tools(req) {
                sink(event);
            }
        }));
    }

    fn cancel(&self, req: u64) {
        self.assistant.cancel_request(req);
    }

    fn observe(&self, state: &State, event: &Event) {
        self.assistant.observe(state, event);
    }

    fn lang(&self) -> Lang {
        self.assistant.lang()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    fn sink_into(events: &Arc<Mutex<Vec<Event>>>) -> EventSink {
        let events = events.clone();
        Arc::new(move |event| events.lock().unwrap().push(event))
    }

    #[test]
    fn a_release_request_fails_safely_with_a_not_available_error() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let requests = Unavailable::new(sink_into(&events));
        requests.start(
            7,
            RequestInput::User {
                text: "close notepad".into(),
                slot: None,
            },
        );
        requests.run_tools(7);
        requests.cancel(7);
        assert_eq!(
            *events.lock().unwrap(),
            vec![Event::FailureText {
                req: 7,
                text: "Assistant commands are not available yet.".into()
            }]
        );
        assert_eq!(requests.lang(), DEFAULT_LANG);
    }
}
