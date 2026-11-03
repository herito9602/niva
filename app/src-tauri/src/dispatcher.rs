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

/// Events dispatched once at startup (task 1.15, CO1). The reducer defaults to
/// `voice_available = false`. Debug builds announce the voice as ready so the
/// shortcut can be exercised before the real supervisor exists (slice 3);
/// release builds keep it false, so the shortcut shows the `error` pose.
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
        }
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
        self.runner.observe(&event);
        let (state, effects) = reduce(std::mem::take(&mut self.state), event);
        self.state = state;
        self.apply(effects);
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
    }

    fn rig(debug_build: bool) -> Rig {
        let (tx, rx) = mpsc::unbounded_channel();
        let host = Arc::new(FakeHost::default());
        let ui = Arc::new(FakeUi::default());
        let sink: EventSink = Arc::new(move |event| {
            let _ = tx.send(Input::Event(event));
        });
        let runner = EffectRunner::new(sink, host.clone());
        let mut core = Core::new(runner, ui.clone(), Box::new(|| 1_000));
        for event in startup_events(debug_build) {
            core.handle(Input::Event(event));
        }
        Rig { core, rx, host, ui }
    }

    impl Rig {
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
            view.caption.as_deref(),
            Some("Voice is not ready. Type a command instead.")
        );
        assert_eq!(*r.host.calls.lock().unwrap(), vec!["show"]);
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
    async fn success_reply_settles_to_idle_3_s_after_the_synthesized_speak_done() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "Done".into(),
        });
        assert_eq!(r.phase(), Phase::Success, "settle is still pending");
        r.advance(ms(2999)).await;
        assert_eq!(r.phase(), Phase::Success);
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Idle);
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
        assert_eq!(r.ui.last_view().caption.as_deref(), Some("Cancelled."));
    }

    #[tokio::test(start_paused = true)]
    async fn the_instant_alert_ends_at_once_and_the_held_alert_at_the_30_s_cap() {
        let mut r = rig(true);
        r.send(Event::ReminderDue {
            count: 1,
            silent: false,
        });
        assert_eq!(r.phase(), Phase::Idle, "chime and speech end instantly");

        let mut r = rig(true);
        r.core.set_media_mode(MediaMode::Held);
        r.send(Event::ReminderDue {
            count: 1,
            silent: false,
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
