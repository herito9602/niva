//! Who answers the reducer's request effects.
//!
//! The reducer emits `StartRequest`, `RunTool` and `CancelRequest`; this
//! module is the host side of those three. A [`Requests`] value also sees
//! every event before the reducer does (to record confirmation answers) and
//! knows the language replies are shown in.
//!
//! [`AssistantRequests`] runs a core [`Assistant`] on a bounded worker pool
//! and feeds its events back. Debug builds give it the dev adapters, release
//! builds the real Ollama and the Windows adapter of the safe actions
//! (`release.rs`); there is no "not available" stub any more.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

#[cfg(test)]
use niva_core::replies::DEFAULT_LANG;
use niva_core::replies::Lang;
use niva_core::replies::{self, Msg};
use niva_core::state_machine::{Event, RequestInput, State};

use crate::effect_runner::EventSink;
use crate::worker_pool::{Job, QUEUE, WORKERS, WorkerPool};

pub trait Requests: Send + Sync {
    /// Starts the LLM call of `req`. Never blocks: the result comes back as
    /// events through the dispatcher queue.
    fn start(&self, req: u64, input: RequestInput);
    /// Runs the validated tools of `req` (after a safe verdict, or after a
    /// confirmation was accepted).
    fn run_tools(&self, req: u64);
    /// The sleep countdown of `req` ended with nobody cancelling: run the held
    /// sleep. Asked for by the reducer's `FinishCountdown` effect only.
    fn finish_countdown(&self, req: u64);
    /// The reducer finished or dropped `req`.
    fn cancel(&self, req: u64);
    /// Sees each event with the state just before the reducer handles it.
    fn observe(&self, state: &State, event: &Event);
    /// Language of the current request, for the reducer's reply templates.
    fn lang(&self) -> Lang;
}

/// Does nothing (tests of the runner and the dispatcher that only look at
/// the reducer's own behavior).
#[cfg(test)]
pub struct Ignore;

#[cfg(test)]
impl Requests for Ignore {
    fn start(&self, _req: u64, _input: RequestInput) {}
    fn run_tools(&self, _req: u64) {}
    fn finish_countdown(&self, _req: u64) {}
    fn cancel(&self, _req: u64) {}
    fn observe(&self, _state: &State, _event: &Event) {}
    fn lang(&self) -> Lang {
        DEFAULT_LANG
    }
}

/// Hands a job to a worker. `false` means it was refused (the job is dropped).
pub type Spawn = Box<dyn Fn(Job) -> bool + Send + Sync>;

/// Drives a core [`Assistant`](niva_core::assistant::Assistant): the LLM call
/// and the tools block, so each runs on a worker of a bounded pool (a stuck
/// provider can hold at most the pool's threads; the rest is refused with a
/// "busy" error at once); the events they produce go back through `sink`.
pub struct AssistantRequests {
    assistant: std::sync::Arc<niva_core::assistant::Assistant>,
    sink: EventSink,
    spawn: Spawn,
    /// Ids the reducer gave up on (`cancel`), newest last, bounded. A job that
    /// was still queued when its request was abandoned is skipped at dequeue,
    /// so a stuck provider is not asked again for work nobody waits for. A
    /// follow-up that supersedes a Thinking request is NOT recorded here (the
    /// reducer does not `cancel` it): it is guarded by `Assistant::begin_user`,
    /// which refuses an id older than `latest_req`.
    abandoned: Arc<Mutex<VecDeque<u64>>>,
}

/// More than the pool can hold (workers + queue), so the id of a queued job
/// is never evicted before the job is dequeued.
const ABANDONED_KEPT: usize = 64;

const _: () = assert!(ABANDONED_KEPT > WORKERS + QUEUE);

impl AssistantRequests {
    /// The app's pool: `WORKERS` threads, `QUEUE` waiting jobs.
    pub fn new(assistant: niva_core::assistant::Assistant, sink: EventSink) -> Self {
        Self::with_pool(assistant, sink, WorkerPool::new(WORKERS, QUEUE))
    }

    pub fn with_pool(
        assistant: niva_core::assistant::Assistant,
        sink: EventSink,
        pool: WorkerPool,
    ) -> Self {
        Self::with_spawn(
            assistant,
            sink,
            Box::new(move |job| pool.submit(job).is_ok()),
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
            abandoned: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    /// A job for `req` that runs `work` unless the request was abandoned while
    /// the job waited in the queue.
    fn job(&self, req: u64, work: impl FnOnce() -> Vec<Event> + Send + 'static) -> Job {
        let sink = self.sink.clone();
        let abandoned = self.abandoned.clone();
        Box::new(move || {
            let skip = abandoned
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .contains(&req);
            if skip {
                return;
            }
            for event in work() {
                sink(event);
            }
        })
    }

    /// The pool refused the work of `req`: say so now, and forget its plan, so
    /// nothing half-started runs later.
    fn refuse(&self, req: u64) {
        self.assistant.cancel_request(req);
        (self.sink)(Event::FailureText {
            req,
            text: replies::render(&Msg::Busy, self.assistant.lang()),
        });
    }
}

impl Requests for AssistantRequests {
    fn start(&self, req: u64, input: RequestInput) {
        let assistant = self.assistant.clone();
        let accepted = (self.spawn)(self.job(req, move || assistant.start_request(req, &input)));
        if !accepted {
            self.refuse(req);
        }
    }

    fn run_tools(&self, req: u64) {
        let assistant = self.assistant.clone();
        let accepted = (self.spawn)(self.job(req, move || assistant.run_tools(req)));
        if !accepted {
            self.refuse(req);
        }
    }

    fn finish_countdown(&self, req: u64) {
        let assistant = self.assistant.clone();
        let accepted = (self.spawn)(self.job(req, move || assistant.finish_countdown(req)));
        if !accepted {
            // Refused: say so now, and drop the held sleep so it cannot run later.
            self.refuse(req);
        }
    }

    fn cancel(&self, req: u64) {
        // Mark it abandoned BEFORE clearing the plan: a worker that dequeues
        // between the two steps must already see the skip, or it could start
        // a job whose plan was just cleared.
        {
            let mut abandoned = self
                .abandoned
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            abandoned.push_back(req);
            if abandoned.len() > ABANDONED_KEPT {
                abandoned.pop_front();
            }
        }
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

    #[cfg(debug_assertions)]
    mod pool {
        use super::*;

        // --- the bounded worker pool (open point of Batches 18/19) ------------------
        // Debug builds only: the PC is the dev module's dry-run PC.

        use std::sync::mpsc::{channel, sync_channel};
        use std::time::Duration;

        use niva_core::assistant::Assistant;
        use niva_core::ports::{ChatReply, ChatRequest, LlmError, LlmProvider};
        use niva_core::tools::{ArgValue, ToolCall};

        use crate::worker_pool::WorkerPool;

        /// An LLM that answers `Hi!` once it is released (a slow provider).
        struct SlowLlm {
            started: std::sync::mpsc::Sender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
            calls: Vec<ToolCall>,
        }

        impl LlmProvider for SlowLlm {
            fn chat(&self, _request: &ChatRequest) -> Result<ChatReply, LlmError> {
                let _ = self.started.send(());
                let _ = self.release.lock().unwrap().recv();
                Ok(ChatReply {
                    text: "Hi!".into(),
                    calls: self.calls.clone(),
                })
            }
        }

        fn user(text: &str) -> RequestInput {
            RequestInput::User {
                text: text.into(),
                slot: None,
            }
        }

        fn slow_requests(
            workers: usize,
            queue: usize,
        ) -> (
            AssistantRequests,
            std::sync::mpsc::Receiver<Event>,
            std::sync::mpsc::Receiver<()>,
            std::sync::mpsc::SyncSender<()>,
        ) {
            slow_requests_with(Vec::new(), Arc::new(|_| {}), workers, queue)
        }

        /// Same rig; the model answers with `calls` and the dry-run PC reports
        /// each action it performs to `log`.
        fn slow_requests_with(
            calls: Vec<ToolCall>,
            log: crate::dev::LogSink,
            workers: usize,
            queue: usize,
        ) -> (
            AssistantRequests,
            std::sync::mpsc::Receiver<Event>,
            std::sync::mpsc::Receiver<()>,
            std::sync::mpsc::SyncSender<()>,
        ) {
            let (started_tx, started_rx) = channel();
            let (release_tx, release_rx) = sync_channel(8);
            let llm = SlowLlm {
                started: started_tx,
                release: Mutex::new(release_rx),
                calls,
            };
            let assistant = Assistant::new(Arc::new(llm), Arc::new(crate::dev::DryRunPc::new(log)));
            let (events_tx, events_rx) = channel();
            let sink: EventSink = Arc::new(move |event| {
                let _ = events_tx.send(event);
            });
            let requests =
                AssistantRequests::with_pool(assistant, sink, WorkerPool::new(workers, queue));
            (requests, events_rx, started_rx, release_tx)
        }

        // --- sleep countdown on the worker pool -----------------------------

        use std::sync::atomic::{AtomicBool, Ordering};

        fn sleep_call() -> ToolCall {
            ToolCall {
                name: "power_action".into(),
                args: [("action".to_string(), ArgValue::Str("sleep".into()))].into(),
            }
        }

        /// A held sleep, and the handles a test needs around it.
        struct SleepRig {
            requests: AssistantRequests,
            events: std::sync::mpsc::Receiver<Event>,
            refuse: Arc<AtomicBool>,
            lines: Arc<Mutex<Vec<String>>>,
        }

        /// Requests over the dry-run PC whose pool refuses work while `refuse`
        /// is set; the model asks for a sleep. The request is accepted and its
        /// tools ran, so the sleep is held. Returns what the PC did.
        fn held_sleep() -> SleepRig {
            use niva_core::assistant::ConfirmInput;

            let lines = Arc::new(Mutex::new(Vec::new()));
            let log: crate::dev::LogSink = {
                let lines = lines.clone();
                Arc::new(move |line| lines.lock().unwrap().push(line))
            };
            let llm = SlowLlm {
                started: channel().0,
                release: Mutex::new({
                    let (tx, rx) = sync_channel(8);
                    tx.send(()).unwrap();
                    rx
                }),
                calls: vec![sleep_call()],
            };
            let assistant = Assistant::new(Arc::new(llm), Arc::new(crate::dev::DryRunPc::new(log)));
            let (tx, events) = channel();
            let sink: EventSink = Arc::new(move |event| {
                let _ = tx.send(event);
            });
            let refuse = Arc::new(AtomicBool::new(false));
            let spawn_refuse = refuse.clone();
            let requests = AssistantRequests::with_spawn(
                assistant,
                sink,
                Box::new(move |job| {
                    if spawn_refuse.load(Ordering::SeqCst) {
                        return false;
                    }
                    job();
                    true
                }),
            );
            requests.start(1, user("put the pc to sleep"));
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(5)).unwrap(),
                Event::ToolCalls { req: 1, .. }
            ));
            requests
                .assistant
                .record_confirmation(ConfirmInput::Click(true));
            requests.run_tools(1);
            assert_eq!(
                events.recv_timeout(Duration::from_secs(5)).unwrap(),
                Event::ToolsDone {
                    req: 1,
                    outcome: niva_core::state_machine::ToolsOutcome::SleepCountdown {
                        cancel_text: None
                    }
                }
            );
            SleepRig {
                requests,
                events,
                refuse,
                lines,
            }
        }

        fn slept(lines: &Arc<Mutex<Vec<String>>>) -> usize {
            lines
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("would sleep the PC"))
                .count()
        }

        #[test]
        fn accepting_a_sleep_does_not_run_it_and_finishing_runs_it_once_on_the_pool() {
            let SleepRig {
                requests,
                events,
                lines,
                ..
            } = held_sleep();
            assert_eq!(slept(&lines), 0, "nothing slept before the deadline");
            requests.finish_countdown(1);
            assert_eq!(
                events.recv_timeout(Duration::from_secs(5)).unwrap(),
                Event::Reply {
                    req: 1,
                    text: "Putting the PC to sleep.".into()
                }
            );
            assert_eq!(slept(&lines), 1);
            requests.finish_countdown(1);
            assert!(events.recv_timeout(Duration::from_millis(200)).is_err());
            assert_eq!(slept(&lines), 1, "a second finish does nothing");
        }

        #[test]
        fn cancelling_the_request_drops_the_held_sleep() {
            let SleepRig {
                requests,
                events,
                lines,
                ..
            } = held_sleep();
            requests.cancel(1);
            requests.finish_countdown(1);
            assert!(events.recv_timeout(Duration::from_millis(200)).is_err());
            assert_eq!(slept(&lines), 0);
        }

        #[test]
        fn a_refused_finish_fails_the_request_and_drops_the_held_sleep() {
            let SleepRig {
                requests,
                events,
                refuse,
                lines,
            } = held_sleep();
            refuse.store(true, Ordering::SeqCst);
            requests.finish_countdown(1);
            match events.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::FailureText { req: 1, text } => assert!(text.contains("busy"), "{text}"),
                other => panic!("{other:?}"),
            }
            refuse.store(false, Ordering::SeqCst);
            requests.finish_countdown(1);
            assert!(events.recv_timeout(Duration::from_millis(200)).is_err());
            assert_eq!(slept(&lines), 0, "the approval was dropped, not queued");
        }

        #[test]
        fn requests_run_on_the_pool_and_their_events_reach_the_sink() {
            let (requests, events, started, release) = slow_requests(1, 2);
            requests.start(1, user("hello"));
            started.recv().unwrap();
            release.send(()).unwrap();
            assert_eq!(
                events.recv_timeout(Duration::from_secs(5)).unwrap(),
                Event::Reply {
                    req: 1,
                    text: "Hi!".into()
                }
            );
        }

        #[test]
        fn a_full_pool_answers_busy_at_once_instead_of_spawning_more_threads() {
            let (requests, events, started, release) = slow_requests(1, 1);
            requests.start(1, user("one")); // runs, blocked in the model
            started.recv().unwrap();
            requests.start(2, user("two")); // waits in the queue
            requests.start(3, user("three")); // refused
            assert_eq!(
                events.recv_timeout(Duration::from_secs(5)).unwrap(),
                Event::FailureText {
                    req: 3,
                    text: "I'm still busy with other requests. Try again in a moment.".into()
                }
            );
            // The first two still complete once the model answers.
            release.send(()).unwrap();
            release.send(()).unwrap();
            let mut replies = Vec::new();
            for _ in 0..2 {
                replies.push(events.recv_timeout(Duration::from_secs(5)).unwrap());
            }
            assert!(replies.iter().all(|e| matches!(e, Event::Reply { .. })));
        }

        #[test]
        fn refused_tool_runs_fail_the_request_instead_of_waiting_for_the_tool_timer() {
            let (requests, events, started, release) = slow_requests(1, 1);
            requests.start(1, user("one"));
            started.recv().unwrap();
            requests.start(2, user("two"));
            requests.run_tools(2);
            assert_eq!(
                events.recv_timeout(Duration::from_secs(5)).unwrap(),
                Event::FailureText {
                    req: 2,
                    text: "I'm still busy with other requests. Try again in a moment.".into()
                }
            );
            release.send(()).unwrap();
            release.send(()).unwrap();
        }

        #[test]
        fn a_queued_request_the_reducer_abandoned_is_never_sent_to_the_model() {
            let (requests, events, started, release) = slow_requests(1, 2);
            requests.start(1, user("one")); // runs, blocked in the model
            started.recv().unwrap();
            requests.start(2, user("two")); // waits in the queue
            requests.cancel(2); // the reducer gave up on it (its timer fired)
            release.send(()).unwrap();
            release.send(()).unwrap();
            assert!(matches!(
                events.recv_timeout(Duration::from_secs(5)).unwrap(),
                Event::Reply { req: 1, .. }
            ));
            assert!(
                started.recv_timeout(Duration::from_millis(500)).is_err(),
                "the abandoned request reached the model"
            );
            assert!(events.try_recv().is_err(), "the abandoned request answered");
        }

        #[test]
        fn a_queued_tool_run_for_an_abandoned_request_does_no_tool_work() {
            let (log_tx, log_rx) = channel();
            let log: crate::dev::LogSink = Arc::new(move |line| {
                let _ = log_tx.send(line);
            });
            let open = ToolCall {
                name: "open_app".into(),
                args: [("name".to_string(), ArgValue::Str("notepad".into()))].into(),
            };
            let (requests, events, started, release) = slow_requests_with(vec![open], log, 1, 3);
            requests.start(1, user("open notepad"));
            started.recv().unwrap();
            release.send(()).unwrap();
            // The model answered: the plan of request 1 is stored, nothing ran.
            events.recv_timeout(Duration::from_secs(5)).unwrap();
            while log_rx.try_recv().is_ok() {}

            // Hold the only worker, queue the tool run behind it.
            let (gate_tx, gate_rx) = channel::<()>();
            assert!((requests.spawn)(Box::new(move || {
                let _ = gate_rx.recv();
            })));
            requests.run_tools(1);
            // The race window of `cancel`: the id is already abandoned while
            // the plan is still stored.
            requests.abandoned.lock().unwrap().push_back(1);
            // FIFO and a single worker: when this marker runs, the tool job
            // has been dequeued and finished (or skipped).
            let (done_tx, done_rx) = channel();
            assert!((requests.spawn)(Box::new(move || {
                let _ = done_tx.send(());
            })));
            gate_tx.send(()).unwrap();
            done_rx.recv_timeout(Duration::from_secs(5)).unwrap();

            assert!(
                events.try_recv().is_err(),
                "the abandoned job produced events"
            );
            assert!(log_rx.try_recv().is_err(), "the abandoned job ran a tool");
        }

        #[test]
        fn cancelling_one_request_does_not_skip_a_queued_live_one() {
            let (requests, events, started, release) = slow_requests(1, 3);
            requests.start(1, user("one")); // runs, blocked in the model
            started.recv().unwrap();
            requests.start(2, user("two")); // queued, will be cancelled
            requests.start(3, user("three")); // queued, still live
            requests.cancel(2);
            release.send(()).unwrap();
            release.send(()).unwrap();
            for expected in [1, 3] {
                match events.recv_timeout(Duration::from_secs(5)).unwrap() {
                    Event::Reply { req, .. } => assert_eq!(req, expected),
                    other => panic!("{other:?}"),
                }
            }
            // Exactly one more model call (request 3); request 2 never started.
            started.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(started.try_recv().is_err(), "request 2 reached the model");
            assert!(events.try_recv().is_err());
        }

        #[test]
        fn the_abandoned_list_keeps_the_newest_64_ids_and_evicts_the_oldest() {
            let (requests, _events, _started, _release) = slow_requests(1, 1);
            for req in 1..=70 {
                requests.cancel(req);
            }
            let kept: Vec<u64> = requests.abandoned.lock().unwrap().iter().copied().collect();
            assert_eq!(kept.len(), 64);
            assert_eq!(kept.first(), Some(&7));
            assert_eq!(kept.last(), Some(&70));
            assert!(!kept.contains(&6));
        }

        #[test]
        fn the_busy_text_follows_the_language_of_the_last_request() {
            let (requests, events, started, release) = slow_requests(1, 1);
            requests.start(1, user("abre el bloc de notas por favor"));
            started.recv().unwrap();
            requests.start(2, user("two"));
            requests.start(3, user("three"));
            match events.recv_timeout(Duration::from_secs(5)).unwrap() {
                Event::FailureText { req: 3, text } => assert!(text.contains("ocupado"), "{text}"),
                other => panic!("{other:?}"),
            }
            release.send(()).unwrap();
            release.send(()).unwrap();
        }
    }
}
