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
        // Records a confirmation answer (click or spoken yes) for the tool loop
        // before the reducer consumes the event.
        self.runner.requests().observe(&self.state, &event);
        let (state, effects) = reduce(std::mem::take(&mut self.state), event);
        self.state = state;
        self.apply(effects);
    }

    fn apply(&mut self, effects: Vec<Effect>) {
        self.view.set_lang(self.runner.requests().lang());
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
    use crate::requests::{Ignore, Requests, Unavailable};
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
        let (tx, rx) = mpsc::unbounded_channel();
        let host = Arc::new(FakeHost::default());
        let ui = Arc::new(FakeUi::default());
        let sink: EventSink = Arc::new(move |event| {
            let _ = tx.send(Input::Event(event));
        });
        let mut runner = EffectRunner::new(sink.clone(), host.clone(), requests(sink));
        if let Some(activity) = activity {
            runner = runner.with_activity(activity);
        }
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
    async fn a_long_reply_stays_up_long_enough_to_read_it() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "word ".repeat(80),
        });
        assert_eq!(r.phase(), Phase::Success);
        r.advance(ms(11_999)).await;
        assert_eq!(r.phase(), Phase::Success, "12 s cap not reached yet");
        r.advance(ms(1)).await;
        assert_eq!(r.phase(), Phase::Idle);
    }

    #[tokio::test(start_paused = true)]
    async fn a_shortcut_during_the_reading_hold_barges_in_and_the_old_hold_is_gone() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "word ".repeat(80),
        });
        r.advance(ms(2_000)).await;
        assert_eq!(r.phase(), Phase::Success);
        r.send(Event::Shortcut);
        assert_eq!(r.phase(), Phase::Listening);
        // The listen is open (6 s no-speech timer). Speech starts, so nothing
        // else may end the session: the old 12 s hold must not fire.
        let id = r.listen_id();
        r.send(Event::SpeechStart { id });
        r.advance(ms(12_000)).await;
        assert_eq!(r.phase(), Phase::Listening, "the stale hold timer fired");
    }

    #[tokio::test(start_paused = true)]
    async fn a_short_reply_still_settles_after_3_s_with_the_reading_rule() {
        let mut r = rig(true);
        r.reach_thinking();
        let req = r.req();
        r.send(Event::Reply {
            req,
            text: "Opening Notepad".into(),
        });
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

    // --- release behavior: requests fail safely ----------------------------

    #[tokio::test(start_paused = true)]
    async fn without_adapters_a_typed_command_ends_in_a_not_available_error() {
        let mut r = rig_with(true, |sink| Arc::new(Unavailable::new(sink)));
        r.send(Event::TextSubmitted {
            text: "close notepad".into(),
        });
        assert_eq!(r.phase(), Phase::Error);
        let view = r.ui.last_view();
        assert_eq!(view.state, "error");
        assert_eq!(
            view.caption.as_deref(),
            Some("Assistant commands are not available yet.")
        );
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
                });
            }

            fn caption(&self) -> String {
                self.ui.last_view().caption.unwrap_or_default()
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

        fn countdown_shown(r: &Rig) -> crate::ui_adapter::CountdownView {
            r.ui.last_view().countdown.expect("a countdown in the view")
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
            // The deadline is in the view (the clock reads 1 000 ms).
            assert_eq!(countdown_shown(&r).ends_at, 11_000);
            assert_eq!(r.ui.last_view().state, "acting");
            r.advance(ms(9_999)).await;
            assert_eq!(r.phase(), Phase::Countdown);
            assert_eq!(slept(&lines), 0, "nothing slept before the deadline");
            r.advance(ms(1)).await;
            assert_eq!(slept(&lines), 1);
            assert_eq!(r.ui.last_view().state, "success");
            assert_eq!(r.caption(), "Suspendiendo el PC.");
            assert_eq!(r.ui.last_view().countdown, None);
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
            let ends_at = countdown_shown(&r).ends_at;
            // The announcement is never "done" (held), its own watchdog fires at
            // 5 s: the countdown is still there, with the same deadline.
            r.advance(ms(5_000)).await;
            assert_eq!(r.phase(), Phase::Countdown);
            assert_eq!(countdown_shown(&r).ends_at, ends_at);
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
            assert_eq!(r.caption(), "Opened notepad.");
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
            assert_eq!(confirm.prompt, "Close Spotify.exe?");
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
            assert_eq!(r.caption(), "Restart the PC?");
            // No confirm listen is open for a click-only prompt.
            assert_eq!(r.core.state().listen_id, None);
            r.send(Event::Transcript {
                id: 1,
                text: "yes".into(),
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
            assert_eq!(r.caption(), "Shut down the PC?");
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
