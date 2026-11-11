//! Dispatcher: the single place where events meet the reducer.
//!
//! Every producer (shortcut, tray, timers, debug window) sends an [`Input`]
//! into one queue. [`Core::handle`] reduces it, publishes the UI events and
//! hands the effects to the [`EffectRunner`], so the reducer is only ever
//! called from one task, in arrival order.

use std::sync::Arc;

use niva_core::state_machine::{Effect, Event, State, reduce};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::effect_runner::EffectRunner;
#[cfg(any(test, debug_assertions))]
use crate::effect_runner::MediaMode;
use crate::latency::{FxObs, Latency};
use crate::ui_adapter::{UiEvent, UiPort, ViewModel};

/// Builds an event from the current state (used when an event carries an id
/// the caller cannot know, e.g. the debug window's `Transcript`). `None`
/// means "nothing to dispatch"; it never panics the dispatcher task.
#[cfg(any(test, debug_assertions))]
pub type Resolver = Box<dyn FnOnce(&State) -> Option<Event> + Send>;

/// Producer side of the dispatcher queue (managed Tauri state).
pub type InputTx = UnboundedSender<Input>;

pub enum Input {
    Event(Event),
    #[cfg(any(test, debug_assertions))]
    Resolve(Resolver),
    /// Debug window: back to a fresh idle state (voice flag kept).
    #[cfg(debug_assertions)]
    Reset,
    /// Debug window: switch the stubbed media between instant and held.
    #[cfg(debug_assertions)]
    SetMedia(MediaMode),
}

/// Events dispatched once at startup in the TESTS (task 1.15, CO1). The
/// reducer defaults to `voice_available = false`, and since task 3.4 the voice
/// supervisor is the only thing that turns it on, in debug and release alike
/// (so the shortcut shows the `error` pose until the sidecar says `ready`).
/// The app no longer calls this; the tests use it to start with a ready voice.
#[cfg(test)]
pub fn startup_events(debug_build: bool) -> Vec<Event> {
    if debug_build {
        vec![Event::VoiceReadinessChanged { ready: true }]
    } else {
        Vec::new()
    }
}

/// Synchronous heart of the dispatcher (testable without a Tauri app).
pub struct Core {
    state: State,
    runner: EffectRunner,
    view: ViewModel,
    ui: Arc<dyn UiPort<UiEvent>>,
    now_ms: Box<dyn Fn() -> u64 + Send>,
    /// The per-turn latency trace (task 3.5): a read-only observer of each
    /// dispatch. Absent in most tests.
    latency: Option<Latency>,
}

impl Core {
    pub fn new(
        runner: EffectRunner,
        ui: Arc<dyn UiPort<UiEvent>>,
        now_ms: Box<dyn Fn() -> u64 + Send>,
    ) -> Self {
        Core {
            state: State::default(),
            runner,
            view: ViewModel::new(),
            ui,
            now_ms,
            latency: None,
        }
    }

    /// Turns the latency trace on.
    pub fn set_latency(&mut self, latency: Latency) {
        self.latency = Some(latency);
    }

    #[cfg(test)]
    pub fn state(&self) -> &State {
        &self.state
    }

    #[cfg(any(test, debug_assertions))]
    pub fn set_media_mode(&mut self, mode: MediaMode) {
        self.runner.set_media_mode(mode);
    }

    pub fn handle(&mut self, input: Input) {
        match input {
            Input::Event(event) => self.dispatch(event),
            #[cfg(any(test, debug_assertions))]
            Input::Resolve(resolve) => {
                if let Some(event) = resolve(&self.state) {
                    self.dispatch(event);
                }
            }
            #[cfg(debug_assertions)]
            Input::Reset => self.reset(),
            #[cfg(debug_assertions)]
            Input::SetMedia(mode) => self.set_media_mode(mode),
        }
    }

    fn dispatch(&mut self, event: Event) {
        // The trace sees the event as ids and kinds only, and the time it
        // reached the reducer.
        let seen = self
            .latency
            .as_ref()
            .map(|latency| (latency.now(), latency.observe(&event)));
        self.runner.observe(&event);
        // Records a confirmation answer (click or spoken yes) for the tool loop
        // before the reducer consumes the event.
        self.runner.requests().observe(&self.state, &event);
        let (state, effects) = reduce(std::mem::take(&mut self.state), event);
        self.state = state;
        let traced = seen.map(|seen| (seen, FxObs::of(&effects)));
        self.apply(effects);
        if let (Some(latency), Some(((t, obs), fx))) = (self.latency.as_mut(), traced) {
            latency.step(t, obs, &fx, Some(self.runner.requests().lang()));
        }
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        let ui_events = self.view.update(&self.state, &effects, (self.now_ms)());
        // Window effects first, so the island is on screen before the webview
        // is told to animate it in.
        self.runner.run(effects);
        for ui_event in ui_events {
            self.ui.publish(ui_event);
        }
    }

    #[cfg(debug_assertions)]
    fn reset(&mut self) {
        // The request in flight ends with the reset, and with it a held sleep.
        if let Some(req) = self.state.request_id {
            self.runner.requests().cancel(req);
        }
        self.runner.cancel_all();
        self.state = State::reset_from(&self.state);
        self.apply(vec![Effect::HideIsland]);
    }
}

/// Drains the queue until every sender is gone.
pub async fn run(mut core: Core, mut rx: UnboundedReceiver<Input>) {
    while let Some(input) = rx.recv().await {
        core.handle(input);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use niva_core::state_machine::{Event, Phase, ReplyKind, ToolVerdict};
    use tokio::sync::mpsc;

    use super::*;
    use crate::effect_runner::{EffectRunner, EventSink, Host, MediaMode};
    use crate::requests::{Ignore, Requests};
    use crate::ui_adapter::{UiEvent, UiPort};

    #[derive(Default)]
    struct FakeHost {
        calls: Mutex<Vec<&'static str>>,
    }

    impl Host for FakeHost {
        fn show_island(&self) {
            self.calls.lock().unwrap().push("show");
        }
        fn hide_island(&self) {
            self.calls.lock().unwrap().push("hide");
        }
    }

    #[derive(Default)]
    struct FakeUi {
        events: Mutex<Vec<UiEvent>>,
    }

    impl UiPort<UiEvent> for FakeUi {
        fn publish(&self, event: UiEvent) {
            self.events.lock().unwrap().push(event);
        }
    }

    impl FakeUi {
        fn last_view(&self) -> crate::ui_adapter::ViewPayload {
            self.events
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find_map(|e| match e {
                    UiEvent::View(v) => Some(v.clone()),
                    _ => None,
                })
                .expect("a published view")
        }
    }

    struct Rig {
        core: Core,
        rx: mpsc::UnboundedReceiver<Input>,
        host: Arc<FakeHost>,
        ui: Arc<FakeUi>,
        /// What Niva said, as the dev log gets it (the view carries no text).
        spoken: Arc<Mutex<Vec<String>>>,
    }

    fn rig(debug_build: bool) -> Rig {
        rig_with(debug_build, |_| Arc::new(Ignore))
    }

    fn rig_with(debug_build: bool, requests: impl FnOnce(EventSink) -> Arc<dyn Requests>) -> Rig {
        rig_build(debug_build, requests, None)
    }

    /// Like `rig_with`, with an activity source for the sleep countdown (none:
    /// the runner's default, which fails closed).
    fn rig_build(
        debug_build: bool,
        requests: impl FnOnce(EventSink) -> Arc<dyn Requests>,
        activity: Option<Arc<dyn niva_core::ports::InputActivity>>,
    ) -> Rig {
        rig_parts(debug_build, requests, activity, None)
    }

    /// The host's own sounds, counted: every chime the runner asks for.
    #[derive(Default)]
    struct CountingCue {
        chimes: Mutex<u32>,
    }

    impl niva_core::ports::AudioCue for CountingCue {
        fn chime(&self, done: Box<dyn FnOnce() + Send>) {
            *self.chimes.lock().unwrap() += 1;
            done();
        }
        fn listening_cue(&self) {}
        fn stop(&self) {}
    }

    fn rig_parts(
        debug_build: bool,
        requests: impl FnOnce(EventSink) -> Arc<dyn Requests>,
        activity: Option<Arc<dyn niva_core::ports::InputActivity>>,
        cue: Option<Arc<dyn niva_core::ports::AudioCue>>,
    ) -> Rig {
        let (tx, rx) = mpsc::unbounded_channel();
        let host = Arc::new(FakeHost::default());
        let ui = Arc::new(FakeUi::default());
        let sink: EventSink = Arc::new(move |event| {
            let _ = tx.send(Input::Event(event));
        });
        let spoken: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = spoken.clone();
        let mut runner = EffectRunner::new(sink.clone(), host.clone(), requests(sink))
            .with_speak_log(Arc::new(move |line| {
                let text = line.strip_prefix("niva says: ").unwrap_or(&line);
                log.lock().unwrap().push(text.to_owned());
            }));
        if let Some(activity) = activity {
            runner = runner.with_activity(activity);
        }
        if let Some(cue) = cue {
            runner = runner.with_cue(cue);
        }
        let mut core = Core::new(runner, ui.clone(), Box::new(|| 1_000));
        for event in startup_events(debug_build) {
            core.handle(Input::Event(event));
        }
        Rig {
            core,
            rx,
            host,
            ui,
            spoken,
        }
    }

    impl Rig {
        /// The last thing Niva said (the text of the latest `Speak`).
        fn said(&self) -> Option<String> {
            self.spoken.lock().unwrap().last().cloned()
        }

        fn send(&mut self, event: Event) {
            self.core.handle(Input::Event(event));
            self.pump();
        }

        /// Handles every queued input (timer fires and synthesized replies).
        fn pump(&mut self) {
            while let Ok(input) = self.rx.try_recv() {
                self.core.handle(input);
            }
        }

        async fn advance(&mut self, d: Duration) {
            tokio::time::advance(d).await;
            for _ in 0..5 {
                tokio::task::yield_now().await;
            }
            self.pump();
        }

        fn phase(&self) -> Phase {
            self.core.state().phase
        }

        fn listen_id(&self) -> u64 {
            self.core.state().listen_id.expect("open listen")
        }

        fn req(&self) -> u64 {
            self.core.state().request_id.expect("a request")
        }

        fn reach_thinking(&mut self) {
            self.send(Event::Shortcut);
            let id = self.listen_id();
            self.send(Event::Transcript {
                id,
                text: "do it".into(),
                lang: None,
            });
            assert_eq!(self.phase(), Phase::Thinking);
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    // --- task 1.15: startup readiness ------------------------------------

    #[test]
    fn debug_builds_dispatch_voice_ready_at_startup_and_release_does_not() {
        assert_eq!(
            startup_events(true),
            vec![Event::VoiceReadinessChanged { ready: true }]
        );
        assert_eq!(startup_events(false), vec![]);
    }

    #[tokio::test(start_paused = true)]
    async fn release_path_shortcut_shows_the_error_pose_with_the_voice_not_ready_caption() {
        let mut r = rig(false);
        assert!(!r.core.state().voice_available);
        r.send(Event::Shortcut);
        assert_eq!(r.phase(), Phase::Error);
        let view = r.ui.last_view();
        assert_eq!(view.state, "error");
        assert_eq!(
            r.said().as_deref(),
            Some("Voice is not ready. Type a command instead.")
        );
        assert_eq!(*r.host.calls.lock().unwrap(), vec!["show"]);
    }

    // --- voice down: error look, host chime, hide after the settle (design L15) ---

    fn voice_down_rig() -> (Rig, Arc<CountingCue>) {
        let cue = Arc::new(CountingCue::default());
        let r = rig_parts(false, |_| Arc::new(Ignore), None, Some(cue.clone()));
        (r, cue)
    }

    #[tokio::test(start_paused = true)]
    async fn sidecar_down_activation_plays_host_chime_and_error_view() {
        let (mut r, cue) = voice_down_rig();
        assert!(!r.core.state().voice_available);
        r.send(Event::Shortcut);
        assert_eq!(r.ui.last_view().state, "error");
        assert_eq!(*cue.chimes.lock().unwrap(), 1, "the host plays the chime");
        assert!(!r.ui.last_view().speaking);
    }

    #[tokio::test(start_paused = true)]
    async fn mic_error_plays_chime_and_error_view() {
        let cue = Arc::new(CountingCue::default());
        let mut r = rig_parts(true, |_| Arc::new(Ignore), None, Some(cue.clone()));
        r.send(Event::Shortcut);
        assert_eq!(r.phase(), Phase::Listening);
        assert_eq!(
            *cue.chimes.lock().unwrap(),
            0,
            "listening plays nothing here"
        );
        r.send(Event::SidecarError {
            code: niva_core::state_machine::SidecarErrorCode::MicBusy,
            id: None,
        });
        assert_eq!(r.ui.last_view().state, "error");
        assert_eq!(*cue.chimes.lock().unwrap(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn voice_down_view_hides_after_settle() {
        let (mut r, _cue) = voice_down_rig();
        r.send(Event::Shortcut);
        assert_eq!(r.phase(), Phase::Error);
        r.advance(ms(1499)).await;
        assert_eq!(r.phase(), Phase::Error);
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Idle);
        assert!(
            r.ui.events
                .lock()
                .unwrap()
                .ends_with(&[UiEvent::Island { visible: false }])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn debug_path_shortcut_starts_listening() {
        let mut r = rig(true);
        assert!(r.core.state().voice_available);
        r.send(Event::Shortcut);
        assert_eq!(r.phase(), Phase::Listening);
        assert_eq!(r.ui.last_view().state, "listening");
        assert!(
            r.ui.events
                .lock()
                .unwrap()
                .contains(&UiEvent::Island { visible: true })
        );
    }

    // --- task 3.5: the latency trace ---------------------------------------

    #[tokio::test(start_paused = true)]
    async fn a_dispatched_voice_turn_writes_one_record_and_no_text() {
        use crate::latency::Stamp;

        let mut r = rig(true);
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let tick = Arc::new(Mutex::new(1_000u64));
        r.core.set_latency(Latency::new(
            Box::new(move || {
                let mut tick = tick.lock().unwrap();
                *tick += 10;
                *tick
            }),
            Box::new(|| 1_700_000_000_000),
            Box::new(|_| {
                Some(Stamp {
                    end_of_speech: Some(900),
                    transcript: 1_000,
                })
            }),
            Box::new(|| Some("small".into())),
            {
                let lines = lines.clone();
                Box::new(move |line| lines.lock().unwrap().push(line))
            },
        ));
        r.send(Event::Shortcut);
        let id = r.listen_id();
        r.send(Event::Transcript {
            id,
            text: "SECRET-WORDS open the thing".into(),
            lang: Some(niva_core::replies::Lang::En),
        });
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "SECRET-REPLY here you go".into(),
        });
        // The stub speaker reports the speech done at once: no sound.
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let line = &lines[0];
        assert!(!line.contains("SECRET"), "{line}");
        let record: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(record["channel"], "voice");
        assert_eq!(record["path"], "chat");
        assert_eq!(record["req"], req);
        assert_eq!(record["lang"], "en");
        assert_eq!(record["t_end_of_speech"], 900);
        assert_eq!(record["t_transcript"], 1_000);
        assert!(record.get("t_first_audio").is_none(), "{line}");
        assert!(record.get("total_ms").is_none(), "{line}");
    }

    #[tokio::test(start_paused = true)]
    async fn a_full_voice_action_turn_through_the_core_has_its_total_and_stages() {
        use crate::latency::Stamp;
        use niva_core::state_machine::ToolsOutcome;

        let mut r = rig(true);
        // Hold the media so the speech is not synthesised as done: the first
        // sound is reported by hand, like the sidecar's `speak_started`.
        r.core.set_media_mode(MediaMode::Held);
        let lines = Arc::new(Mutex::new(Vec::<String>::new()));
        let tick = Arc::new(Mutex::new(1_000u64));
        r.core.set_latency(Latency::new(
            Box::new(move || {
                let mut tick = tick.lock().unwrap();
                *tick += 10;
                *tick
            }),
            Box::new(|| 1_700_000_000_000),
            Box::new(|_| {
                Some(Stamp {
                    end_of_speech: Some(900),
                    transcript: 1_000,
                })
            }),
            Box::new(|| Some("small".into())),
            {
                let lines = lines.clone();
                Box::new(move |line| lines.lock().unwrap().push(line))
            },
        ));
        r.send(Event::Shortcut);
        let id = r.listen_id();
        r.send(Event::Transcript {
            id,
            text: "open the thing".into(),
            lang: Some(niva_core::replies::Lang::En),
        });
        let req = r.req();
        r.send(Event::ToolCalls {
            req,
            verdict: niva_core::state_machine::ToolVerdict::RunSafe,
        });
        r.send(Event::ToolsDone {
            req,
            outcome: ToolsOutcome::Completed,
        });
        assert!(
            lines.lock().unwrap().is_empty(),
            "open until the first sound"
        );
        let speak = r
            .core
            .state()
            .speak_id
            .expect("the template is being spoken");
        r.send(Event::SpeakStarted { id: speak });
        let lines = lines.lock().unwrap();
        assert_eq!(lines.len(), 1, "{lines:?}");
        let record: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(record["path"], "action");
        assert_eq!(record["channel"], "voice");
        assert_eq!(record["validation"], "run_safe");
        let first = record["t_first_audio"].as_u64().expect("a first sound");
        assert_eq!(record["total_ms"].as_u64(), Some(first - 900));
        for key in ["stt_ms", "llm_ms", "tool_ms", "tts_ms", "other_ms"] {
            assert!(record.get(key).is_some(), "{key} missing: {record}");
        }
        assert_eq!(record["stt_ms"], 100);
    }

    // --- task 3.5: the first real run (Batch 38) -----------------------------
    //
    // The shapes of `e2e-1.jsonl`, driven through the real assistant: a scripted
    // model, the dry-run PC, the real reducer, runner and dispatcher, and a
    // clock the test sets before each step (so the stage times are exact).

    mod first_run {
        use std::sync::atomic::{AtomicU64, Ordering};

        use niva_core::assistant::Assistant;
        use niva_core::ports::{ChatReply, ChatRequest, LlmError, LlmProvider};
        use niva_core::tools::{ArgValue, ToolCall};

        use super::*;
        use crate::latency::Stamp;
        use crate::requests::AssistantRequests;

        struct Script(Mutex<Vec<Result<ChatReply, LlmError>>>);

        impl LlmProvider for Script {
            fn chat(&self, _request: &ChatRequest) -> Result<ChatReply, LlmError> {
                self.0.lock().unwrap().remove(0)
            }
        }

        fn call(name: &str, args: &[(&str, ArgValue)]) -> Result<ChatReply, LlmError> {
            Ok(ChatReply {
                text: String::new(),
                calls: vec![ToolCall {
                    name: name.into(),
                    args: args
                        .iter()
                        .map(|(key, value)| ((*key).to_string(), value.clone()))
                        .collect(),
                }],
            })
        }

        struct Traced {
            r: Rig,
            clock: Arc<AtomicU64>,
            lines: Arc<Mutex<Vec<String>>>,
        }

        /// Held media (the first sound is reported by hand, like the sidecar's
        /// `speak_started`), the voice stamped as: end of speech at 1000,
        /// transcript at 1900.
        fn traced(script: Vec<Result<ChatReply, LlmError>>) -> Traced {
            let mut r = rig_with(true, move |sink| {
                let assistant = Assistant::new(
                    Arc::new(Script(Mutex::new(script))),
                    Arc::new(crate::dev::DryRunPc::new(Arc::new(|_| {}))),
                );
                Arc::new(AssistantRequests::with_spawn(
                    assistant,
                    sink,
                    Box::new(|job| {
                        job();
                        true
                    }),
                ))
            });
            r.core.set_media_mode(MediaMode::Held);
            let clock = Arc::new(AtomicU64::new(0));
            let lines = Arc::new(Mutex::new(Vec::<String>::new()));
            let for_clock = clock.clone();
            r.core.set_latency(Latency::new(
                Box::new(move || for_clock.load(Ordering::SeqCst)),
                Box::new(|| 1_700_000_000_000),
                Box::new(|_| {
                    Some(Stamp {
                        end_of_speech: Some(1_000),
                        transcript: 1_900,
                    })
                }),
                Box::new(|| Some("small".into())),
                {
                    let lines = lines.clone();
                    Box::new(move |line| lines.lock().unwrap().push(line))
                },
            ));
            Traced { r, clock, lines }
        }

        impl Traced {
            fn at(&self, t: u64) {
                self.clock.store(t, Ordering::SeqCst);
            }

            /// Handles one event at `t`, without draining what it queued.
            fn send_at(&mut self, t: u64, event: Event) {
                self.at(t);
                self.r.core.handle(Input::Event(event));
            }

            /// Handles the next queued input at `t` (the model's answer, a
            /// tool's result).
            fn next_at(&mut self, t: u64) {
                self.at(t);
                let input = self.r.rx.try_recv().expect("a queued input");
                self.r.core.handle(input);
            }

            /// Wake, then the words: the request starts at 1900.
            fn command(&mut self) -> u64 {
                self.send_at(500, Event::Shortcut);
                let id = self.r.listen_id();
                self.send_at(
                    1_900,
                    Event::Transcript {
                        id,
                        text: "something".into(),
                        lang: Some(niva_core::replies::Lang::Es),
                    },
                );
                self.r.req()
            }

            fn speak_id(&self) -> u64 {
                self.r.core.state().speak_id.expect("a speech")
            }

            fn records(&self) -> Vec<serde_json::Value> {
                self.lines
                    .lock()
                    .unwrap()
                    .iter()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect()
            }
        }

        fn ms_of(record: &serde_json::Value, key: &str) -> u64 {
            record[key]
                .as_u64()
                .unwrap_or_else(|| panic!("{key} missing: {record}"))
        }

        /// Record 3 of the evidence: a volume command that worked was labelled
        /// `cancelled`, its model time counted twice, and no tool time at all.
        #[tokio::test(start_paused = true)]
        async fn a_safe_action_is_an_action_with_stages_that_add_up_to_the_total() {
            let mut t = traced(vec![call("set_volume", &[("level", ArgValue::Int(30))])]);
            let req = t.command();
            // The model answers 2061 ms after the request, the tool takes 17 ms,
            // the first sound comes 1038 ms after the speech was sent.
            t.next_at(3_961); // ToolCalls (RunSafe)
            t.next_at(3_978); // Reply: the tools' answer
            let speak = t.speak_id();
            t.send_at(5_016, Event::SpeakStarted { id: speak });
            let records = t.records();
            assert_eq!(records.len(), 1, "{records:?}");
            let record = &records[0];
            assert_eq!(record["path"], "action", "{record}");
            assert_eq!(record["validation"], "run_safe");
            assert_eq!(record["req"], req);
            assert_eq!(ms_of(record, "stt_ms"), 900);
            assert_eq!(ms_of(record, "llm_ms"), 3_961 - 1_900, "{record}");
            assert_eq!(ms_of(record, "tool_ms"), 17, "{record}");
            assert_eq!(ms_of(record, "tts_ms"), 1_038);
            assert_eq!(ms_of(record, "total_ms"), 4_016);
            assert_eq!(ms_of(record, "t_llm_done"), 3_961);
            assert_eq!(ms_of(record, "t_tool_done"), 3_978);
            let named: u64 = ["stt_ms", "llm_ms", "tool_ms", "tts_ms", "other_ms"]
                .iter()
                .map(|key| ms_of(record, key))
                .sum();
            assert_eq!(named, ms_of(record, "total_ms"), "{record}");
        }

        /// Records 1 and 2: the model failed after about 2 s, and the 2 s were
        /// lost in `other_ms` (no `llm_ms`).
        #[tokio::test(start_paused = true)]
        async fn a_failed_model_call_has_its_own_time_and_leaves_no_other_time() {
            let mut t = traced(vec![Err(LlmError::Unavailable)]);
            t.command();
            t.next_at(3_935); // the failure, 2035 ms after the request
            let speak = t.speak_id();
            t.send_at(4_100, Event::SpeakStarted { id: speak });
            let records = t.records();
            assert_eq!(records.len(), 1, "{records:?}");
            let record = &records[0];
            assert_eq!(record["path"], "error");
            assert_eq!(record["validation"], "llm_failed");
            assert_eq!(ms_of(record, "llm_ms"), 2_035, "{record}");
            assert_eq!(ms_of(record, "other_ms"), 0, "{record}");
        }

        /// Records 7 to 9: a risky call asked for (a lock) is a confirmation,
        /// and answering it writes no second record; the next command is its
        /// own record.
        #[tokio::test(start_paused = true)]
        async fn a_confirmation_is_one_record_and_the_next_command_is_another() {
            let mut t = traced(vec![
                call("power_action", &[("action", ArgValue::Str("lock".into()))]),
                call("set_volume", &[("level", ArgValue::Int(30))]),
            ]);
            let first = t.command();
            t.next_at(3_900); // ToolCalls (Confirm): the question is spoken
            let prompt = t.speak_id();
            t.send_at(4_700, Event::SpeakStarted { id: prompt });
            // The owner clicks Cancel while the question is still playing.
            let id = t.r.ui.last_view().confirm.expect("the buttons").id;
            t.send_at(5_000, Event::Reject { id });
            let records = t.records();
            assert_eq!(records.len(), 1, "{records:?}");
            assert_eq!(records[0]["path"], "confirm");
            assert_eq!(records[0]["validation"], "confirm");
            let said = t.speak_id();
            t.send_at(9_500, Event::SpeakDone { id: said });
            let second = t.command();
            // Cancelled before the question ended, so no confirm listen was
            // opened: four ids per turn, like the evidence's 22 -> 26 -> 30 -> 34.
            assert_eq!(second, first + 4);
            t.next_at(12_000);
            t.next_at(12_010);
            let speak = t.speak_id();
            t.send_at(13_000, Event::SpeakStarted { id: speak });
            let records = t.records();
            assert_eq!(records.len(), 2, "{records:?}");
            assert_eq!(records[1]["path"], "action");
        }

        /// A second command that starts before the first one's speech made a
        /// sound closes the first (no sound, never invented), and is itself
        /// recorded.
        #[tokio::test(start_paused = true)]
        async fn a_command_before_the_first_sound_writes_the_first_turn_without_audio() {
            let mut t = traced(vec![
                call("set_volume", &[("level", ArgValue::Int(30))]),
                call("set_volume", &[("level", ArgValue::Int(40))]),
            ]);
            let first = t.command();
            t.next_at(3_961);
            t.next_at(3_978);
            // No SpeakStarted: the owner presses the shortcut again.
            t.send_at(4_500, Event::Shortcut);
            let records = t.records();
            assert_eq!(records.len(), 1, "{records:?}");
            assert_eq!(records[0]["req"], first);
            assert_eq!(records[0]["path"], "action", "{}", records[0]);
            assert!(records[0].get("t_first_audio").is_none());
            assert!(records[0].get("total_ms").is_none());
            let id = t.r.listen_id();
            t.send_at(
                5_000,
                Event::Transcript {
                    id,
                    text: "again".into(),
                    lang: None,
                },
            );
            t.next_at(6_000);
            t.next_at(6_010);
            let speak = t.speak_id();
            t.send_at(7_000, Event::SpeakStarted { id: speak });
            let records = t.records();
            assert_eq!(records.len(), 2, "{records:?}");
            assert_eq!(records[1]["path"], "action");
        }
    }

    // --- reducer + runner loop -------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn no_speech_timer_closes_the_island_after_6_s() {
        let mut r = rig(true);
        r.send(Event::Shortcut);
        r.advance(ms(5999)).await;
        assert_eq!(r.phase(), Phase::Listening);
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Idle);
        assert_eq!(*r.host.calls.lock().unwrap(), vec!["show", "hide"]);
        assert!(
            r.ui.events
                .lock()
                .unwrap()
                .ends_with(&[UiEvent::Island { visible: false }])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn success_hides_after_speech_plus_settle() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "Done".into(),
        });
        assert_eq!(r.phase(), Phase::Success, "settle is still pending");
        r.advance(ms(1499)).await;
        assert_eq!(r.phase(), Phase::Success);
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn a_long_reply_is_not_held_for_reading() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "word ".repeat(80),
        });
        assert_eq!(r.phase(), Phase::Success);
        // 400 characters: the same 1.5 s settle, no 12 s reading hold.
        r.advance(ms(1499)).await;
        assert_eq!(r.phase(), Phase::Success);
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn activation_during_settle_cancels_pending_hide() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "Done".into(),
        });
        r.advance(ms(1_000)).await;
        assert_eq!(r.phase(), Phase::Success);
        r.send(Event::Shortcut);
        assert_eq!(r.phase(), Phase::Listening);
        // The listen is open. Speech starts, so nothing else may end the
        // session: the old settle must not fire.
        let id = r.listen_id();
        r.send(Event::SpeechStart { id });
        r.advance(ms(12_000)).await;
        assert_eq!(r.phase(), Phase::Listening, "the stale settle fired");
    }

    #[tokio::test(start_paused = true)]
    async fn error_hides_after_settle() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Failure { req });
        assert_eq!(r.phase(), Phase::Error);
        r.advance(ms(1499)).await;
        assert_eq!(r.phase(), Phase::Error);
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Idle);
        assert!(
            r.ui.events
                .lock()
                .unwrap()
                .ends_with(&[UiEvent::Island { visible: false }])
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_confirmation_never_stays_pending_even_if_the_speech_never_ends() {
        let mut r = rig(true);
        r.core.set_media_mode(MediaMode::Held);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Close Notepad?".into(),
                click_only: false,
                cancel_text: None,
            },
        });
        assert_eq!(r.phase(), Phase::ConfirmationNeeded);
        assert_eq!(r.core.state().speak_kind, Some(ReplyKind::ConfirmPrompt));
        // Speak watchdog (5 s) -> synthesized SpeakDone -> 10 s confirm timer.
        r.advance(ms(5000)).await;
        assert_eq!(r.phase(), Phase::ConfirmationNeeded);
        assert_eq!(r.core.state().speak_id, None);
        r.advance(ms(9999)).await;
        assert_eq!(r.phase(), Phase::ConfirmationNeeded);
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Success);
        assert_eq!(r.said().as_deref(), Some("Cancelled."));
    }

    #[tokio::test(start_paused = true)]
    async fn the_instant_alert_settles_after_its_speech_and_a_silent_held_one_ends_at_the_30_s_cap()
    {
        let mut r = rig(true);
        r.send(Event::ReminderDue {
            count: 1,
            silent: false,
        });
        assert_eq!(
            r.phase(),
            Phase::Success,
            "the speech ended instantly: the alert is over and settles"
        );
        r.advance(ms(1_500)).await;
        assert_eq!(r.phase(), Phase::Idle);

        // A reminder that is never spoken (fullscreen) with the chime held:
        // only the 30 s cap ends it.
        let mut r = rig(true);
        r.core.set_media_mode(MediaMode::Held);
        r.send(Event::ReminderDue {
            count: 1,
            silent: true,
        });
        assert_eq!(r.phase(), Phase::ReminderAlert);
        r.advance(ms(29_999)).await;
        assert_eq!(r.phase(), Phase::ReminderAlert);
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn the_confirmation_deadline_uses_the_injected_clock() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Close Notepad?".into(),
                click_only: false,
                cancel_text: None,
            },
        });
        // Instant media: the prompt is done at once, so the timer has started.
        let confirm = r.ui.last_view().confirm.expect("confirm");
        assert_eq!(confirm.expires_at, Some(11_000));
    }

    #[tokio::test(start_paused = true)]
    async fn a_resolver_input_sees_the_current_state() {
        let mut r = rig(true);
        r.send(Event::Shortcut);
        r.core.handle(Input::Resolve(Box::new(|state| {
            Some(Event::Transcript {
                id: state.listen_id.unwrap(),
                text: "hello".into(),
                lang: None,
            })
        })));
        r.pump();
        assert_eq!(r.phase(), Phase::Thinking);
        assert_eq!(r.ui.last_view().state, "thinking");
    }

    #[tokio::test(start_paused = true)]
    async fn a_stale_resolver_event_is_dropped_by_the_reducer() {
        let mut r = rig(true);
        r.core.handle(Input::Resolve(Box::new(|state| {
            Some(Event::Transcript {
                id: state.listen_id.unwrap_or(0),
                text: "hello".into(),
                lang: None,
            })
        })));
        r.pump();
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn a_resolver_with_no_event_is_a_no_op_and_cannot_panic_the_dispatcher() {
        let mut r = rig(true);
        r.send(Event::Shortcut);
        let before = r.core.state().clone();
        r.core.handle(Input::Resolve(Box::new(|_| None)));
        r.pump();
        assert_eq!(*r.core.state(), before);
    }

    // --- release behavior: the release assistant over the real loop --------

    /// The release assembly (`release::assistant`) behind the real reducer and
    /// dispatcher, with a scripted model and fake Windows parts. Jobs run
    /// inline; their events still go through the queue.
    #[cfg(windows)]
    mod release_loop {
        use std::sync::{Arc, Mutex};

        use niva_core::ports::{ChatReply, ChatRequest, LlmError, LlmProvider};
        use niva_core::tools::{ArgValue, ToolCall};

        use super::*;
        use crate::adapters::system_clock::SystemClock;
        use crate::adapters::windows_pc::WindowsPc;
        use crate::adapters::windows_pc::fakes::{FakeApps, FakeOutput, FakePower};
        use crate::requests::AssistantRequests;

        /// Fails the test if it ever reaches the search network.
        struct NoSearchNetwork;

        impl crate::adapters::search::SearchTransport for NoSearchNetwork {
            fn post(
                &self,
                _key: &crate::adapters::search::ApiKey,
                _body: &str,
                _timeout: Duration,
            ) -> Result<crate::adapters::search::Reply, crate::adapters::search::NetError>
            {
                panic!("a dispatcher test made a search call");
            }
        }

        struct Script(Mutex<Vec<Result<ChatReply, LlmError>>>);

        impl LlmProvider for Script {
            fn chat(&self, _request: &ChatRequest) -> Result<ChatReply, LlmError> {
                self.0.lock().unwrap().remove(0)
            }
        }

        fn calls(calls: Vec<ToolCall>) -> Result<ChatReply, LlmError> {
            Ok(ChatReply {
                text: String::new(),
                calls,
            })
        }

        fn call(name: &str, key: &str, value: &str) -> ToolCall {
            ToolCall {
                name: name.into(),
                args: [(key.to_string(), ArgValue::Str(value.into()))]
                    .into_iter()
                    .collect(),
            }
        }

        fn release_rig(
            answer: Result<ChatReply, LlmError>,
        ) -> (Rig, Arc<FakeApps>, Arc<FakeOutput>) {
            let apps = Arc::new(FakeApps::default());
            let output = Arc::new(FakeOutput::default());
            let (a, o) = (apps.clone(), output.clone());
            let power = Arc::new(FakePower::default());
            let rig = rig_with(true, move |sink| {
                let assistant = crate::release::assistant(
                    Arc::new(Script(Mutex::new(vec![answer]))),
                    Arc::new(WindowsPc::new(a, o, power)),
                    Arc::new(SystemClock),
                    Arc::new(crate::adapters::search::TavilySearch::new(
                        Arc::new(NoSearchNetwork),
                        None,
                    )),
                );
                Arc::new(AssistantRequests::with_spawn(
                    assistant,
                    sink,
                    Box::new(|job| {
                        job();
                        true
                    }),
                ))
            });
            (rig, apps, output)
        }

        #[tokio::test(start_paused = true)]
        async fn a_typed_close_command_shows_the_island_buttons_and_closes_nothing_before_the_click()
         {
            let (mut r, apps, output) =
                release_rig(calls(vec![call("close_app", "name", "spotify")]));
            r.send(Event::TextSubmitted {
                text: "close spotify".into(),
            });
            assert_eq!(r.phase(), Phase::ConfirmationNeeded);
            let view = r.ui.last_view();
            let confirm = view.confirm.expect("the confirmation");
            assert_eq!(Some(confirm.id), r.core.state().confirm_id);
            assert_eq!(r.said().as_deref(), Some("Close Spotify.exe?"));
            assert!(apps.opened.lock().unwrap().is_empty());
            assert!(apps.closed.lock().unwrap().is_empty());
            assert!(output.changes.lock().unwrap().is_empty());
        }

        #[tokio::test(start_paused = true)]
        async fn a_click_on_confirm_closes_the_app_and_a_click_on_cancel_does_not() {
            for accept in [true, false] {
                let (mut r, apps, _) =
                    release_rig(calls(vec![call("close_app", "name", "spotify")]));
                r.send(Event::TextSubmitted {
                    text: "close spotify".into(),
                });
                let id = r.ui.last_view().confirm.expect("a confirmation").id;
                r.send(if accept {
                    Event::Accept { id }
                } else {
                    Event::Reject { id }
                });
                let closed = apps.closed.lock().unwrap().clone();
                if accept {
                    assert_eq!(closed, ["Spotify.exe"]);
                    assert_eq!(r.phase(), Phase::Success);
                    assert_eq!(r.said().as_deref(), Some("Closed Spotify.exe."));
                } else {
                    assert!(closed.is_empty(), "a cancel closes nothing");
                }
            }
        }

        #[tokio::test(start_paused = true)]
        async fn a_typed_open_command_runs_for_real_and_confirms() {
            let (mut r, apps, _) = release_rig(calls(vec![call("open_app", "name", "notepad")]));
            *apps.launched_as.lock().unwrap() = Some("Bloc de notas".into());
            r.send(Event::TextSubmitted {
                text: "open notepad".into(),
            });
            assert_eq!(r.phase(), Phase::Success);
            assert_eq!(*apps.opened.lock().unwrap(), ["notepad"]);
            // The reply names the entry that was launched, not the model's word.
            assert_eq!(r.said().as_deref(), Some("Opened Bloc de notas."));
        }

        #[tokio::test(start_paused = true)]
        async fn with_ollama_down_a_typed_command_gets_a_truthful_error() {
            let (mut r, apps, _) = release_rig(Err(LlmError::Unavailable));
            r.send(Event::TextSubmitted {
                text: "open notepad".into(),
            });
            assert_eq!(r.phase(), Phase::Error);
            assert_eq!(
                r.said().as_deref(),
                Some("I can't think right now. Is the language model running?")
            );
            assert!(apps.opened.lock().unwrap().is_empty());
        }
    }

    // --- the whole loop with the dev adapters (debug builds) ---------------

    #[cfg(debug_assertions)]
    mod assistant_loop {
        use std::sync::Arc;

        use niva_core::state_machine::Event;

        use super::*;
        use crate::requests::AssistantRequests;

        fn dev_rig() -> Rig {
            rig_with(true, |sink| {
                let assistant = crate::dev::assistant(Arc::new(|_| {}));
                // Inline: the job runs on the dispatcher thread; its events
                // still go through the queue.
                Arc::new(AssistantRequests::with_spawn(
                    assistant,
                    sink,
                    Box::new(|job| {
                        job();
                        true
                    }),
                ))
            })
        }

        impl Rig {
            fn type_text(&mut self, text: &str) {
                self.send(Event::TextSubmitted { text: text.into() });
            }

            /// A spoken yes on the open confirm listen.
            fn say_text_yes(&mut self) {
                let id = self.listen_id();
                self.send(Event::Transcript {
                    id,
                    text: "yes".into(),
                    lang: None,
                });
            }

            fn caption(&self) -> String {
                self.said().unwrap_or_default()
            }

            /// A click on the buttons the island shows now: it sends back the
            /// confirmation id the view carries.
            fn click(&mut self, accept: bool) {
                let id = self.ui.last_view().confirm.expect("a confirmation").id;
                self.click_for(id, accept);
            }

            fn click_for(&mut self, id: u64, accept: bool) {
                self.send(if accept {
                    Event::Accept { id }
                } else {
                    Event::Reject { id }
                });
            }
        }

        // --- sleep countdown through the whole loop (debug build, dry-run PC) ---

        use crate::test_support::FakeActivity;

        /// The dev assistant (dry-run PC) with a fake activity source. The log
        /// collects what the dry-run PC says it would do.
        fn sleep_rig() -> (Rig, Arc<FakeActivity>, Arc<Mutex<Vec<String>>>) {
            let lines = Arc::new(Mutex::new(Vec::new()));
            let log: crate::dev::LogSink = {
                let lines = lines.clone();
                Arc::new(move |line| lines.lock().unwrap().push(line))
            };
            let activity = Arc::new(FakeActivity::default());
            let rig = rig_build(
                true,
                |sink| {
                    let assistant = crate::dev::assistant(log);
                    Arc::new(AssistantRequests::with_spawn(
                        assistant,
                        sink,
                        Box::new(|job| {
                            job();
                            true
                        }),
                    ))
                },
                Some(activity.clone()),
            );
            (rig, activity, lines)
        }

        fn slept(lines: &Arc<Mutex<Vec<String>>>) -> usize {
            lines
                .lock()
                .unwrap()
                .iter()
                .filter(|l| l.contains("would sleep the PC"))
                .count()
        }

        #[tokio::test(start_paused = true)]
        async fn suspende_la_pc_asks_then_counts_down_in_spanish_and_sleeps_at_the_end() {
            let (mut r, _activity, lines) = sleep_rig();
            r.type_text("suspendé la PC");
            assert_eq!(r.caption(), "¿Suspender el PC?");
            r.click(true);
            assert_eq!(r.phase(), Phase::Countdown);
            assert_eq!(
                r.caption(),
                "Suspendiendo en 10 segundos. Presiona una tecla o mueve el mouse para cancelar."
            );
            // The countdown has its own state and no number in the view.
            assert_eq!(r.ui.last_view().state, "countdown");
            r.advance(ms(9_999)).await;
            assert_eq!(r.phase(), Phase::Countdown);
            assert_eq!(slept(&lines), 0, "nothing slept before the deadline");
            r.advance(ms(1)).await;
            assert_eq!(slept(&lines), 1);
            assert_eq!(r.ui.last_view().state, "success");
            assert_eq!(r.caption(), "Suspendiendo el PC.");
        }

        #[tokio::test(start_paused = true)]
        async fn input_after_the_grace_period_cancels_and_the_pc_never_sleeps() {
            let (mut r, activity, lines) = sleep_rig();
            r.type_text("put the pc to sleep");
            r.click(true);
            r.advance(ms(1_500)).await;
            assert_eq!(r.phase(), Phase::Countdown);
            activity.touch();
            r.advance(ms(100)).await;
            assert_eq!(r.phase(), Phase::Success);
            assert_eq!(r.caption(), "Suspend cancelled.");
            r.advance(ms(30_000)).await;
            assert_eq!(slept(&lines), 0);
            let reads = activity.reads();
            r.advance(ms(30_000)).await;
            assert_eq!(activity.reads(), reads, "no polling at rest");
        }

        #[tokio::test(start_paused = true)]
        async fn jitter_right_after_the_accept_click_does_not_cancel() {
            let (mut r, activity, lines) = sleep_rig();
            r.type_text("sleep");
            r.click(true);
            for _ in 0..9 {
                activity.touch();
                r.advance(ms(100)).await;
            }
            assert_eq!(r.phase(), Phase::Countdown, "the jitter was not input");
            r.advance(ms(9_100)).await;
            assert_eq!(slept(&lines), 1);
        }

        #[tokio::test(start_paused = true)]
        async fn the_shortcut_cancels_the_countdown() {
            let (mut r, _activity, lines) = sleep_rig();
            r.type_text("sleep");
            r.click(true);
            r.advance(ms(3_000)).await;
            r.send(Event::Shortcut);
            assert_eq!(r.caption(), "Suspend cancelled.");
            r.advance(ms(30_000)).await;
            assert_eq!(slept(&lines), 0);
        }

        #[tokio::test(start_paused = true)]
        async fn holding_the_audio_does_not_make_the_countdown_vanish_or_change_its_length() {
            let (mut r, _activity, lines) = sleep_rig();
            r.core.set_media_mode(MediaMode::Held);
            r.type_text("sleep");
            // The held prompt: its watchdog (5 s) ends it, then the click.
            r.advance(ms(5_000)).await;
            r.click(true);
            assert_eq!(r.phase(), Phase::Countdown);
            // The announcement is never "done" (held), its own watchdog fires at
            // 5 s: the countdown is still there, with the same deadline.
            r.advance(ms(5_000)).await;
            assert_eq!(r.phase(), Phase::Countdown);
            assert_eq!(r.ui.last_view().state, "countdown");
            assert_eq!(slept(&lines), 0);
            r.advance(ms(5_000)).await;
            assert_eq!(slept(&lines), 1, "10 s after it started, speech or not");
        }

        #[tokio::test(start_paused = true)]
        async fn an_unreadable_activity_source_never_suspends_and_says_so() {
            let (mut r, activity, lines) = sleep_rig();
            activity.set_unreadable(true);
            r.type_text("suspende la pc");
            r.click(true);
            assert_eq!(r.ui.last_view().state, "error");
            assert_eq!(
                r.caption(),
                "No pude comprobar tu actividad, así que no suspendí el PC."
            );
            r.advance(ms(30_000)).await;
            assert_eq!(slept(&lines), 0);
        }

        #[tokio::test(start_paused = true)]
        async fn a_runner_without_an_activity_source_fails_closed_in_the_real_loop() {
            let mut r = dev_rig();
            r.type_text("sleep");
            r.click(true);
            assert_eq!(r.ui.last_view().state, "error");
            assert!(r.caption().contains("did not suspend"), "{}", r.caption());
        }

        #[tokio::test(start_paused = true)]
        async fn a_reset_during_the_countdown_stops_everything() {
            let (mut r, activity, lines) = sleep_rig();
            r.type_text("sleep");
            r.click(true);
            r.advance(ms(2_000)).await;
            r.core.handle(Input::Reset);
            r.pump();
            assert_eq!(r.phase(), Phase::Idle);
            let reads = activity.reads();
            r.advance(ms(30_000)).await;
            assert_eq!(slept(&lines), 0);
            assert_eq!(activity.reads(), reads, "the poller stopped with the reset");
        }

        #[tokio::test(start_paused = true)]
        async fn lock_has_no_countdown_and_restart_stays_click_only() {
            let (mut r, activity, _lines) = sleep_rig();
            r.type_text("lock the pc");
            r.say_text_yes();
            assert_eq!(r.caption(), "Locking the PC.");
            assert_eq!(activity.reads(), 0, "nothing was watched");
            r.type_text("restart the pc");
            assert_eq!(r.core.state().listen_id, None);
            r.click(true);
            assert_eq!(r.caption(), "Restarting the PC.");
            assert_eq!(activity.reads(), 0);
        }

        #[tokio::test(start_paused = true)]
        async fn a_typed_command_runs_and_the_island_shows_the_template_reply() {
            let mut r = dev_rig();
            r.type_text("open notepad");
            assert_eq!(r.ui.last_view().state, "success");
            // The dry-run adapter reports a display name, like the real one.
            assert_eq!(r.caption(), "Opened Notepad.");
            r.type_text("abrí Spotify");
            assert_eq!(r.caption(), "Abrí Spotify.");
        }

        #[tokio::test(start_paused = true)]
        async fn the_reducers_own_templates_follow_the_request_language() {
            let mut r = dev_rig();
            r.type_text("cierra el spotify");
            assert_eq!(r.caption(), "¿Cerrar Spotify.exe?");
            r.click(false);
            assert_eq!(r.caption(), "Cancelado.");
        }

        #[tokio::test(start_paused = true)]
        async fn a_close_waits_for_a_click_and_the_view_carries_the_prompt() {
            let mut r = dev_rig();
            r.type_text("close spotify");
            assert_eq!(r.phase(), Phase::ConfirmationNeeded);
            let confirm = r.ui.last_view().confirm.expect("a confirmation");
            assert_eq!(r.caption(), "Close Spotify.exe?");
            assert_eq!(Some(confirm.id), r.core.state().confirm_id);
            r.click(true);
            assert_eq!(r.ui.last_view().state, "success");
            assert_eq!(r.caption(), "Closed Spotify.exe.");
        }

        #[tokio::test(start_paused = true)]
        async fn the_confirmation_deadline_appears_only_after_the_prompt_was_spoken() {
            // Task 2.7 through the real loop.
            let mut r = dev_rig();
            r.core.set_media_mode(MediaMode::Held);
            r.type_text("close spotify");
            let confirm = r.ui.last_view().confirm.expect("a confirmation");
            assert_eq!(confirm.expires_at, None, "the prompt is still being spoken");
            let speak = r.core.state().speak_id.expect("the prompt is playing");
            r.send(Event::SpeakDone { id: speak });
            let confirm = r.ui.last_view().confirm.expect("a confirmation");
            assert_eq!(confirm.expires_at, Some(11_000));
        }

        #[tokio::test(start_paused = true)]
        async fn restart_cannot_be_confirmed_by_voice_only_by_click() {
            let mut r = dev_rig();
            r.type_text("restart the pc");
            assert_eq!(
                r.caption(),
                "Restart the PC? Press Ctrl+Shift+Enter to confirm, or Esc to cancel."
            );
            // No confirm listen is open for a click-only prompt.
            assert_eq!(r.core.state().listen_id, None);
            r.send(Event::Transcript {
                id: 1,
                text: "yes".into(),
                lang: None,
            });
            assert_eq!(r.phase(), Phase::ConfirmationNeeded);
            r.click(true);
            assert_eq!(r.caption(), "Restarting the PC.");
        }

        #[tokio::test(start_paused = true)]
        async fn an_ambiguous_close_asks_and_the_answer_picks_by_exact_name() {
            let mut r = dev_rig();
            r.type_text("close notepad");
            assert_eq!(r.caption(), "Which one: notepad++.exe or notepad.exe?");
            r.type_text("notepad");
            assert_eq!(r.caption(), "Close notepad.exe?");
            r.click(true);
            assert_eq!(r.caption(), "Closed notepad.exe.");
        }

        #[tokio::test(start_paused = true)]
        async fn a_reminder_without_a_time_asks_once_then_completes() {
            let mut r = dev_rig();
            r.type_text("remind me to call mom");
            assert_eq!(r.caption(), "At what time?");
            r.type_text("in 10 minutes");
            assert_eq!(r.caption(), "Reminder set in 10 minutes: call mom.");
            r.type_text("list my reminders");
            assert_eq!(r.caption(), "You have 1 reminder: call mom.");
        }

        #[tokio::test(start_paused = true)]
        async fn web_search_goes_through_the_summary_call() {
            let mut r = dev_rig();
            r.type_text("search rust");
            assert_eq!(r.ui.last_view().state, "success");
            assert!(r.caption().starts_with("Dev summary"), "{}", r.caption());
        }

        #[tokio::test(start_paused = true)]
        async fn a_search_mixed_with_a_risky_call_is_rejected_and_nothing_runs() {
            let mut r = dev_rig();
            r.type_text("search rust and close spotify");
            assert_eq!(r.ui.last_view().state, "error");
            assert!(r.caption().starts_with("I can't search and do that"));
            // The next command still sees Spotify running.
            r.type_text("close spotify");
            assert_eq!(r.caption(), "Close Spotify.exe?");
        }

        // The manual try-it checklist (tasks.md, Batch 17), as tests.

        #[tokio::test(start_paused = true)]
        async fn a_second_ambiguity_in_the_follow_up_ends_in_an_error() {
            let mut r = dev_rig();
            r.type_text("close notepad");
            r.type_text("close note");
            assert_eq!(r.ui.last_view().state, "error");
            assert_eq!(r.caption(), "Something went wrong.");
        }

        #[tokio::test(start_paused = true)]
        async fn the_other_notepad_stays_open_with_a_save_dialog() {
            let mut r = dev_rig();
            r.type_text("close notepad");
            r.type_text("notepad");
            r.click(true);
            assert_eq!(r.caption(), "Closed notepad.exe.");
            r.type_text("close notepad");
            assert_eq!(r.caption(), "Close notepad++.exe?");
            r.click(true);
            assert_eq!(r.ui.last_view().state, "success");
            assert_eq!(
                r.caption(),
                "notepad++.exe is still running. It may be asking to save."
            );
        }

        #[tokio::test(start_paused = true)]
        async fn a_closed_app_is_gone_from_the_fake_process_list() {
            let mut r = dev_rig();
            r.type_text("close spotify");
            r.click(true);
            r.type_text("close spotify");
            assert_eq!(r.ui.last_view().state, "error");
            assert_eq!(r.caption(), "I couldn't find a running app with that name.");
        }

        #[tokio::test(start_paused = true)]
        async fn safe_calls_run_before_the_confirmation_and_share_the_reply() {
            let mut r = dev_rig();
            r.type_text("volume 30 and close spotify");
            assert_eq!(r.caption(), "Close Spotify.exe?");
            r.click(true);
            assert_eq!(r.caption(), "Volume at 30%. Closed Spotify.exe.");
        }

        #[tokio::test(start_paused = true)]
        async fn a_reminder_with_nothing_asks_what_and_when_and_in_spanish() {
            let mut r = dev_rig();
            r.type_text("remind me");
            assert_eq!(r.caption(), "What should I remind you about, and when?");
            r.type_text("call mom at 6");
            assert_eq!(r.caption(), "Reminder set at 06:00: call mom.");
            r.type_text("recuérdame llamar a mamá a las 18:30");
            assert_eq!(r.caption(), "Recordatorio a las 18:30: llamar a mamá.");
        }

        #[tokio::test(start_paused = true)]
        async fn search_with_no_results_or_offline_is_an_error() {
            let mut r = dev_rig();
            r.type_text("search nothing");
            assert_eq!(r.ui.last_view().state, "error");
            assert_eq!(r.caption(), "I found nothing for that search.");
            r.type_text("search offline");
            assert_eq!(r.caption(), "I can't search right now.");
        }

        #[tokio::test(start_paused = true)]
        async fn lock_is_confirmed_and_shutdown_can_be_cancelled() {
            let mut r = dev_rig();
            r.type_text("lock the pc");
            assert_eq!(r.caption(), "Lock the PC?");
            r.click(true);
            assert_eq!(r.caption(), "Locking the PC.");
            r.type_text("shut down the pc");
            assert_eq!(
                r.caption(),
                "Shut down the PC? Press Ctrl+Shift+Enter to confirm, or Esc to cancel."
            );
            r.click(false);
            assert_eq!(r.caption(), "Cancelled.");
        }

        #[tokio::test(start_paused = true)]
        async fn the_confirmation_times_out_after_10_s_and_cancels() {
            let mut r = dev_rig();
            r.type_text("close spotify");
            r.advance(ms(10_000)).await;
            assert_eq!(r.caption(), "Cancelled.");
        }

        // Batch 18 (review fixes), through the real loop.

        #[tokio::test(start_paused = true)]
        async fn a_double_click_on_prompt_one_does_not_confirm_prompt_two() {
            let mut r = dev_rig();
            r.type_text("close spotify and close chrome");
            assert_eq!(r.caption(), "Close Spotify.exe?");
            let first = r.ui.last_view().confirm.expect("first").id;
            r.click_for(first, true);
            r.click_for(first, true);
            assert_eq!(r.phase(), Phase::ConfirmationNeeded);
            assert_eq!(r.caption(), "Close chrome.exe?");
            let second = r.ui.last_view().confirm.expect("second").id;
            assert_ne!(first, second, "each prompt has its own id");
            r.click_for(second, true);
            assert_eq!(r.caption(), "Closed Spotify.exe. Closed chrome.exe.");
        }

        #[tokio::test(start_paused = true)]
        async fn cancelling_after_a_safe_call_reports_what_ran() {
            let mut r = dev_rig();
            r.type_text("volume 30 and close spotify");
            r.click(false);
            assert_eq!(r.caption(), "Volume at 30%. Cancelled closing Spotify.exe.");
        }

        #[tokio::test(start_paused = true)]
        async fn the_confirmation_timing_out_after_a_safe_call_reports_what_ran() {
            let mut r = dev_rig();
            r.type_text("volume 30 and close spotify");
            r.advance(ms(10_000)).await;
            assert_eq!(r.caption(), "Volume at 30%. Cancelled closing Spotify.exe.");
        }

        #[tokio::test(start_paused = true)]
        async fn the_protected_explorer_is_never_offered() {
            let mut r = dev_rig();
            r.type_text("close explorer");
            assert_eq!(r.ui.last_view().state, "error");
            assert_eq!(r.caption(), "I couldn't find a running app with that name.");
        }
    }

    /// Remembers which requests the reducer gave up on.
    #[derive(Default)]
    struct RecordsCancels {
        cancelled: Mutex<Vec<u64>>,
    }

    impl Requests for RecordsCancels {
        fn start(&self, _req: u64, _input: niva_core::state_machine::RequestInput) {}
        fn run_tools(&self, _req: u64) {}
        fn finish_countdown(&self, _req: u64) {}
        fn cancel(&self, req: u64) {
            self.cancelled.lock().unwrap().push(req);
        }
        fn observe(&self, _state: &State, _event: &Event) {}
        fn lang(&self) -> niva_core::replies::Lang {
            niva_core::replies::DEFAULT_LANG
        }
    }

    #[cfg(debug_assertions)]
    #[tokio::test(start_paused = true)]
    async fn a_reset_ends_the_request_in_flight_so_a_held_sleep_is_dropped() {
        let recorder = Arc::new(RecordsCancels::default());
        let mut r = rig_with(true, {
            let recorder = recorder.clone();
            move |_| recorder
        });
        r.reach_thinking();
        let req = r.req();
        r.core.handle(Input::Reset);
        r.pump();
        assert_eq!(*recorder.cancelled.lock().unwrap(), vec![req]);
    }

    #[cfg(debug_assertions)]
    #[tokio::test(start_paused = true)]
    async fn reset_never_restarts_the_id_allocators() {
        let mut r = rig(true);
        r.send(Event::Shortcut);
        let first_listen = r.listen_id();
        r.core.handle(Input::Reset);
        r.pump();
        r.send(Event::Shortcut);
        assert!(
            r.listen_id() > first_listen,
            "a queued event of the old numbering must stay stale"
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test(start_paused = true)]
    async fn reset_returns_to_idle_keeps_voice_and_cancels_pending_timers() {
        let mut r = rig(true);
        r.core.set_media_mode(MediaMode::Held);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "Done".into(),
        });
        r.core.handle(Input::Reset);
        r.pump();
        assert_eq!(r.phase(), Phase::Idle);
        assert!(r.core.state().voice_available);
        assert_eq!(r.ui.last_view().state, "idle");
        assert!(
            r.ui.events
                .lock()
                .unwrap()
                .ends_with(&[UiEvent::Island { visible: false }])
        );
        r.advance(ms(120_000)).await;
        assert_eq!(r.phase(), Phase::Idle, "no leftover timer fired");
        r.send(Event::Shortcut);
        assert_eq!(r.phase(), Phase::Listening);
    }
}
