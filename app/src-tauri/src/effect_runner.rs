//! Host effect runner: performs the reducer's `Effect`s.
//!
//! Slice 1 (PR 1c) runs the effects that need no other subsystem and stubs
//! the rest; PR 2b hands the request effects to a [`Requests`] handler. The
//! runner owns three guarantees from the design:
//!
//! - `StartTimer{kind, token}` becomes a tokio task that echoes the same pair
//!   as `Watchdog{kind, token}`; starting a kind again replaces the old task.
//! - Every `Speak` arms a speak watchdog (`token` = speak id, length-based
//!   duration), so a `Speak` always ends in `SpeakDone`, `TtsFailed` or a
//!   watchdog event and a confirmation can never stay pending (W2).
//! - Every `Chime{id}` reports `ChimeDone{id}`.
//! - A final reply stays up long enough to read: the `Settle` that follows its
//!   speech is stretched to `niva_core::reading` (see [`ReadingHold`]).
//!
//! - `WatchInput{id}` reads a baseline of the user's input and polls it every
//!   100 ms until `StopWatchingInput` (or until it saw input or lost the
//!   source); an unreadable source is a `CountdownFault`, never silence. There
//!   is no task, and no read, when no countdown runs.
//! - `FinishCountdown{req}` goes to the [`Requests`] handler, nowhere else.
//!
//! Stubs until later slices (all logged): `Speak` has no TTS (the speech
//! "ends" at once, see [`MediaMode`]), `Chime` plays no sound, `Listen`,
//! `CancelListen`, `ForceEndOfSpeech` (slice 3) and `EnqueueReminder`,
//! `DrainReminders` (slice 4) do nothing. `StartRequest`, `RunTool` and
//! `CancelRequest` go to the [`Requests`] handler.

use std::{sync::Arc, time::Duration};

use niva_core::activity::{InputWatch, POLL_INTERVAL, Watch};
use niva_core::ports::{ActivityError, InputActivity};
use niva_core::reading::{reading_time, settle_delay};
use niva_core::state_machine::{Effect, Event, ReplyKind, Say, TimerKind};
use tokio::task::JoinHandle;

use crate::requests::Requests;
use crate::ui_adapter::template_caption;

/// Where timer fires and synthesized replies go: the dispatcher's queue.
pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

/// Window operations the runner needs from the host.
pub trait Host: Send + Sync {
    fn show_island(&self);
    fn hide_island(&self);
}

/// How the stubbed audio behaves until the voice sidecar exists (slice 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MediaMode {
    /// No audio device: `Speak` ends with a synthesized `SpeakDone` and
    /// `Chime` with `ChimeDone`, both at once (the design's "sidecar down"
    /// behavior).
    #[default]
    Instant,
    /// A sidecar that never answers: nothing is synthesized, so only the
    /// speak watchdog, the alert cap or a manual event ends the media. Used by
    /// the debug window to look at a state for longer than a blink.
    #[cfg(any(test, debug_assertions))]
    Held,
}

/// Minimum speak watchdog and the per-character estimate (design: max(5 s,
/// 80 ms per character)).
const MIN_SPEAK_WATCHDOG: Duration = Duration::from_secs(5);
const SPEAK_WATCHDOG_PER_CHAR: Duration = Duration::from_millis(80);

fn char_count(say: &Say) -> usize {
    match say {
        Say::Text(text) => text.chars().count(),
        Say::Template(template) => template_caption(*template).chars().count(),
    }
}

/// Length of the speak watchdog for a reply.
pub fn speak_watchdog_duration(say: &Say) -> Duration {
    let chars = char_count(say);
    let estimate = SPEAK_WATCHDOG_PER_CHAR * u32::try_from(chars).unwrap_or(u32::MAX);
    estimate.max(MIN_SPEAK_WATCHDOG)
}

/// When the final reply on screen was shown and how long it needs to be read.
/// The next `Settle` start (at the end of its speech) uses it once.
struct ReadingHold {
    /// The final `Speak` this hold belongs to; a newer `Speak` replaces it and
    /// `StopSpeaking` clears it.
    speak_id: u64,
    shown_at: tokio::time::Instant,
    reading: Duration,
}

/// The activity source of a runner nobody gave one: it cannot be read, so a
/// countdown fails closed (no suspend) instead of running unwatched.
struct NoActivity;

impl InputActivity for NoActivity {
    fn input_stamp(&self) -> Result<u64, ActivityError> {
        Err(ActivityError)
    }
}

pub struct EffectRunner {
    sink: EventSink,
    host: Arc<dyn Host>,
    requests: Arc<dyn Requests>,
    mode: MediaMode,
    timers: Vec<(TimerKind, JoinHandle<()>)>,
    /// The speak watchdog of the speech in flight: (speak id, task).
    watchdog: Option<(u64, JoinHandle<()>)>,
    /// Set by a final `Speak`, consumed by the `Settle` that follows it.
    reading: Option<ReadingHold>,
    /// Where the user's input is read (sleep countdown).
    activity: Arc<dyn InputActivity>,
    /// The poller of the running countdown, if any.
    watcher: Option<JoinHandle<()>>,
}

impl EffectRunner {
    pub fn new(sink: EventSink, host: Arc<dyn Host>, requests: Arc<dyn Requests>) -> Self {
        EffectRunner {
            sink,
            host,
            requests,
            mode: MediaMode::default(),
            timers: Vec::new(),
            watchdog: None,
            reading: None,
            activity: Arc::new(NoActivity),
            watcher: None,
        }
    }

    /// Where the sleep countdown reads the user's input. Without one every
    /// countdown fails closed.
    pub fn with_activity(mut self, activity: Arc<dyn InputActivity>) -> Self {
        self.activity = activity;
        self
    }

    #[cfg(any(test, debug_assertions))]
    pub fn set_media_mode(&mut self, mode: MediaMode) {
        self.mode = mode;
    }

    pub fn requests(&self) -> &Arc<dyn Requests> {
        &self.requests
    }

    pub fn run(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            self.run_one(effect);
        }
    }

    /// Lets the runner see events the reducer is about to receive: a finished
    /// or failed speech disarms its watchdog.
    pub fn observe(&mut self, event: &Event) {
        if let Event::SpeakDone { id } | Event::TtsFailed { id } = event {
            self.disarm_watchdog(Some(*id));
        }
    }

    /// Aborts every timer and the speak watchdog (debug reset).
    pub fn cancel_all(&mut self) {
        for (_, task) in self.timers.drain(..) {
            task.abort();
        }
        self.disarm_watchdog(None);
        self.reading = None;
        self.stop_watching();
    }

    fn run_one(&mut self, effect: Effect) {
        match effect {
            Effect::StartTimer { kind, token } => self.start_timer(kind, token),
            Effect::CancelTimer { kind } => self.drop_timer(kind),
            Effect::ShowIsland => self.host.show_island(),
            Effect::HideIsland => self.host.hide_island(),
            Effect::Speak { id, kind, say } => self.speak(id, kind, &say),
            Effect::StopSpeaking => {
                self.disarm_watchdog(None);
                self.reading = None;
            }
            Effect::StartRequest { req, input } => self.requests.start(req, input),
            Effect::RunTool { req } => self.requests.run_tools(req),
            Effect::CancelRequest { req } => self.requests.cancel(req),
            Effect::WatchInput { id } => self.watch_input(id),
            Effect::StopWatchingInput => self.stop_watching(),
            Effect::FinishCountdown { req } => self.requests.finish_countdown(req),
            Effect::Chime { id } => {
                if self.mode == MediaMode::Instant {
                    (self.sink)(Event::ChimeDone { id });
                } else {
                    eprintln!("[niva] chime {id} held (no ChimeDone)");
                }
            }
            other => eprintln!("[niva] stub effect (later slice): {other:?}"),
        }
    }

    /// Takes the baseline of the user's input, tells the reducer it is watching
    /// (or that it cannot) and polls until stopped. The task ends by itself
    /// after the first input or the first failed read.
    fn watch_input(&mut self, id: u64) {
        self.stop_watching();
        let source = self.activity.clone();
        let mut watch = match InputWatch::start(&*source) {
            Ok(watch) => watch,
            Err(_) => {
                (self.sink)(Event::CountdownFault { id });
                return;
            }
        };
        (self.sink)(Event::CountdownReady { id });
        let sink = self.sink.clone();
        let started = tokio::time::Instant::now();
        self.watcher = Some(tokio::spawn(async move {
            // Fixed ticks from the baseline (no drift, and the first one does not
            // depend on when the task is first polled).
            let mut next = started;
            loop {
                next += POLL_INTERVAL;
                tokio::time::sleep_until(next).await;
                match watch.poll(started.elapsed(), &*source) {
                    Watch::Quiet => {}
                    Watch::Input => {
                        sink(Event::CountdownCancel { id });
                        break;
                    }
                    Watch::Unreadable => {
                        sink(Event::CountdownFault { id });
                        break;
                    }
                }
            }
        }));
    }

    fn stop_watching(&mut self) {
        if let Some(task) = self.watcher.take() {
            task.abort();
        }
    }

    fn start_timer(&mut self, kind: TimerKind, token: u64) {
        let Some(duration) = kind.duration() else {
            // The speak watchdog belongs to the runner, never to the reducer.
            eprintln!("[niva] ignoring reducer-started {kind:?}");
            return;
        };
        let sink = self.sink.clone();
        let now = tokio::time::Instant::now();
        // Only the Settle that follows the final speech uses (and consumes) the
        // hold; a timer of any other kind leaves it for that Settle.
        let hold = if kind == TimerKind::Settle {
            self.reading.take()
        } else {
            None
        };
        let duration = match hold {
            // The quiet time after the speech, or what is left of the reading time.
            Some(hold) => {
                eprintln!(
                    "[niva] settle after the reading time of speak {}",
                    hold.speak_id
                );
                settle_delay(
                    duration,
                    hold.reading,
                    now.saturating_duration_since(hold.shown_at),
                )
            }
            None => duration,
        };
        let deadline = now + duration;
        let task = tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            sink(Event::Watchdog { kind, token });
        });
        self.drop_timer(kind);
        self.timers.push((kind, task));
    }

    fn drop_timer(&mut self, kind: TimerKind) {
        self.timers.retain(|(running, task)| {
            let drop = *running == kind;
            if drop {
                task.abort();
            }
            !drop
        });
    }

    fn speak(&mut self, id: u64, kind: ReplyKind, say: &Say) {
        eprintln!("[niva] speak {id} (no TTS yet): {say:?}");
        // Only final replies are held for reading; prompts wait for an answer.
        self.reading = (kind == ReplyKind::Final).then(|| ReadingHold {
            speak_id: id,
            shown_at: tokio::time::Instant::now(),
            reading: reading_time(char_count(say)),
        });
        self.arm_watchdog(id, speak_watchdog_duration(say));
        if self.mode == MediaMode::Instant {
            (self.sink)(Event::SpeakDone { id });
            self.disarm_watchdog(Some(id));
        }
    }

    fn arm_watchdog(&mut self, id: u64, duration: Duration) {
        self.disarm_watchdog(None);
        let sink = self.sink.clone();
        let deadline = tokio::time::Instant::now() + duration;
        let task = tokio::spawn(async move {
            tokio::time::sleep_until(deadline).await;
            sink(Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token: id,
            });
        });
        self.watchdog = Some((id, task));
    }

    /// Drops the watchdog; with `Some(id)` only if it guards that speech.
    fn disarm_watchdog(&mut self, id: Option<u64>) {
        let matches = match (&self.watchdog, id) {
            (Some((armed, _)), Some(wanted)) => *armed == wanted,
            (Some(_), None) => true,
            (None, _) => false,
        };
        if matches && let Some((_, task)) = self.watchdog.take() {
            task.abort();
        }
    }
}

impl Drop for EffectRunner {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use niva_core::state_machine::{
        Effect, Event, ListenMode, ReplyKind, RequestInput, Say, Template, TimerKind,
    };
    use tokio::sync::mpsc;

    use super::*;
    use crate::test_support::FakeActivity;

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
    struct RecordingRequests {
        calls: Mutex<Vec<String>>,
    }

    impl Requests for RecordingRequests {
        fn start(&self, req: u64, input: RequestInput) {
            self.calls
                .lock()
                .unwrap()
                .push(format!("start {req} {input:?}"));
        }
        fn run_tools(&self, req: u64) {
            self.calls.lock().unwrap().push(format!("run {req}"));
        }
        fn cancel(&self, req: u64) {
            self.calls.lock().unwrap().push(format!("cancel {req}"));
        }
        fn finish_countdown(&self, req: u64) {
            self.calls.lock().unwrap().push(format!("finish {req}"));
        }
        fn observe(&self, _: &niva_core::state_machine::State, _: &Event) {}
        fn lang(&self) -> niva_core::replies::Lang {
            niva_core::replies::Lang::default()
        }
    }

    struct Rig {
        runner: EffectRunner,
        rx: mpsc::UnboundedReceiver<Event>,
        host: Arc<FakeHost>,
        requests: Arc<RecordingRequests>,
    }

    fn rig() -> Rig {
        let (tx, rx) = mpsc::unbounded_channel();
        let host = Arc::new(FakeHost::default());
        let sink: EventSink = Arc::new(move |event| {
            let _ = tx.send(event);
        });
        let requests = Arc::new(RecordingRequests::default());
        Rig {
            runner: EffectRunner::new(sink, host.clone(), requests.clone()),
            rx,
            host,
            requests,
        }
    }

    /// Advances the paused clock and lets the spawned timer tasks run.
    async fn advance(d: Duration) {
        tokio::time::advance(d).await;
        for _ in 0..5 {
            tokio::task::yield_now().await;
        }
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<Event>) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(event) = rx.try_recv() {
            out.push(event);
        }
        out
    }

    fn speak(id: u64, say: Say) -> Effect {
        Effect::Speak {
            id,
            kind: ReplyKind::Final,
            say,
        }
    }

    fn text_of_len(n: usize) -> Say {
        Say::Text("x".repeat(n))
    }

    // --- reading time: a long final reply keeps the island up ------------

    fn settle(token: u64) -> Effect {
        Effect::StartTimer {
            kind: TimerKind::Settle,
            token,
        }
    }

    fn settle_fire(token: u64) -> Event {
        Event::Watchdog {
            kind: TimerKind::Settle,
            token,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_short_final_reply_settles_after_3_s_as_before() {
        let mut r = rig();
        r.runner
            .run(vec![speak(1, Say::Text("Done".into())), settle(5)]);
        advance(ms(2999)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(5)));
        advance(ms(1)).await;
        assert_eq!(drain(&mut r.rx), vec![settle_fire(5)]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_long_final_reply_stays_for_its_reading_time() {
        let mut r = rig();
        // 400 characters: the 12 s cap, measured from the moment it was shown.
        r.runner.run(vec![speak(1, text_of_len(400)), settle(5)]);
        advance(ms(11_999)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(5)));
        advance(ms(1)).await;
        assert_eq!(drain(&mut r.rx), vec![settle_fire(5)]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_reading_time_counts_from_when_the_reply_was_shown_not_from_the_speech_end() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        // 100 chars: 7 s of reading; the (held) speech ends after 8 s.
        r.runner.run(vec![speak(1, text_of_len(100))]);
        advance(ms(8_000)).await;
        r.runner.run(vec![settle(6)]);
        advance(ms(2999)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(6)));
        advance(ms(1)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(6)));
    }

    #[tokio::test(start_paused = true)]
    async fn speech_that_ends_before_the_reading_time_waits_for_the_rest() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        // 150 chars: 9.75 s of reading, the speech ends after 4 s -> hide at 9.75 s.
        r.runner.run(vec![speak(1, text_of_len(150))]);
        advance(ms(4_000)).await;
        r.runner.run(vec![settle(6)]);
        advance(ms(5_749)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(6)));
        advance(ms(1)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(6)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_template_reply_keeps_the_plain_3_s_settle() {
        let mut r = rig();
        r.runner.run(vec![
            speak(1, Say::Template(Template::ActionDone)),
            settle(5),
        ]);
        advance(ms(3000)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(5)));
    }

    #[tokio::test(start_paused = true)]
    async fn the_reading_time_belongs_to_one_reply_only() {
        let mut r = rig();
        r.runner.run(vec![speak(1, text_of_len(400)), settle(5)]);
        advance(ms(12_000)).await;
        drain(&mut r.rx);
        // A later settle that no long reply precedes is the plain 3 s again.
        r.runner.run(vec![settle(6)]);
        advance(ms(3000)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(6)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_timer_of_another_kind_does_not_consume_the_reading_hold() {
        let mut r = rig();
        r.runner.run(vec![
            speak(1, text_of_len(400)),
            Effect::StartTimer {
                kind: TimerKind::Llm,
                token: 9,
            },
            settle(5),
        ]);
        advance(ms(11_999)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(5)));
        advance(ms(1)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(5)));
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_the_speech_clears_the_reading_hold() {
        let mut r = rig();
        r.runner
            .run(vec![speak(1, text_of_len(400)), Effect::StopSpeaking]);
        r.runner.run(vec![settle(5)]);
        advance(ms(3000)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(5)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_speak_replaces_the_reading_hold_of_the_older_one() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![speak(1, text_of_len(400))]);
        advance(ms(1000)).await;
        // A short final reply takes over: its Settle is the plain 3 s.
        r.runner
            .run(vec![speak(2, Say::Text("Done".into())), settle(5)]);
        advance(ms(3000)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(5)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_confirmation_prompt_never_gets_a_reading_hold() {
        let mut r = rig();
        r.runner.run(vec![
            Effect::Speak {
                id: 1,
                kind: ReplyKind::ConfirmPrompt,
                say: text_of_len(400),
            },
            settle(5),
        ]);
        advance(ms(3000)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(5)));
    }

    // --- timers (task 1.14: settle fires at 3 s) -------------------------

    #[tokio::test(start_paused = true)]
    async fn settle_timer_fires_at_exactly_3_s_with_its_token() {
        let mut r = rig();
        r.runner.run(vec![Effect::StartTimer {
            kind: TimerKind::Settle,
            token: 7,
        }]);
        advance(ms(2999)).await;
        assert!(drain(&mut r.rx).is_empty(), "fired before 3 s");
        advance(ms(1)).await;
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::Watchdog {
                kind: TimerKind::Settle,
                token: 7
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn every_reducer_timer_fires_at_its_duration_and_echoes_kind_and_token() {
        let mut checked = 0;
        for (i, kind) in TimerKind::ALL.into_iter().enumerate() {
            let Some(duration) = kind.duration() else {
                continue;
            };
            let token = 100 + i as u64;
            let mut r = rig();
            r.runner.run(vec![Effect::StartTimer { kind, token }]);
            advance(duration - ms(1)).await;
            assert!(drain(&mut r.rx).is_empty(), "{kind:?} fired early");
            advance(ms(1)).await;
            assert_eq!(
                drain(&mut r.rx),
                vec![Event::Watchdog { kind, token }],
                "{kind:?}"
            );
            checked += 1;
        }
        assert_eq!(checked, 10);
    }

    #[tokio::test(start_paused = true)]
    async fn restarting_a_kind_drops_the_earlier_instance() {
        let mut r = rig();
        r.runner.run(vec![Effect::StartTimer {
            kind: TimerKind::Llm,
            token: 1,
        }]);
        advance(ms(10_000)).await;
        r.runner.run(vec![Effect::StartTimer {
            kind: TimerKind::Llm,
            token: 2,
        }]);
        advance(ms(5_000)).await;
        assert!(
            drain(&mut r.rx).is_empty(),
            "the first instance would fire here"
        );
        advance(ms(10_000)).await;
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::Watchdog {
                kind: TimerKind::Llm,
                token: 2
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_timer_stops_only_that_kind() {
        let mut r = rig();
        r.runner.run(vec![
            Effect::StartTimer {
                kind: TimerKind::Settle,
                token: 1,
            },
            Effect::StartTimer {
                kind: TimerKind::Confirm,
                token: 2,
            },
            Effect::CancelTimer {
                kind: TimerKind::Settle,
            },
        ]);
        advance(ms(10_000)).await;
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::Watchdog {
                kind: TimerKind::Confirm,
                token: 2
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reducer_started_speak_watchdog_is_ignored() {
        let mut r = rig();
        r.runner.run(vec![Effect::StartTimer {
            kind: TimerKind::SpeakWatchdog,
            token: 1,
        }]);
        advance(ms(60_000)).await;
        assert!(drain(&mut r.rx).is_empty());
    }

    // --- Speak: always ends (W2 runner contract) -------------------------

    #[tokio::test(start_paused = true)]
    async fn speak_without_tts_is_done_at_once() {
        let mut r = rig();
        r.runner.run(vec![speak(4, Say::Template(Template::Error))]);
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        assert_eq!(drain(&mut r.rx), vec![Event::SpeakDone { id: 4 }]);
        advance(ms(60_000)).await;
        assert!(
            drain(&mut r.rx).is_empty(),
            "the watchdog must not fire after the synthesized SpeakDone"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn held_speak_arms_the_watchdog_with_the_speak_id_as_token() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![speak(9, text_of_len(10))]);
        advance(ms(4999)).await;
        assert!(drain(&mut r.rx).is_empty());
        advance(ms(1)).await;
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token: 9
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn held_watchdog_is_length_based_for_long_text() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![speak(3, text_of_len(100))]);
        advance(ms(7999)).await;
        assert!(drain(&mut r.rx).is_empty(), "100 chars x 80 ms = 8 s");
        advance(ms(1)).await;
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token: 3
            }]
        );
    }

    #[test]
    fn watchdog_duration_is_the_max_of_5_s_and_80_ms_per_char() {
        assert_eq!(speak_watchdog_duration(&text_of_len(0)), ms(5000));
        assert_eq!(speak_watchdog_duration(&text_of_len(62)), ms(5000));
        assert_eq!(speak_watchdog_duration(&text_of_len(63)), ms(5040));
        assert_eq!(speak_watchdog_duration(&text_of_len(100)), ms(8000));
        assert!(speak_watchdog_duration(&Say::Template(Template::Error)) >= ms(5000));
        // Multi-byte characters count once.
        assert_eq!(
            speak_watchdog_duration(&Say::Text("é".repeat(100))),
            ms(8000)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn spoken_done_or_failure_of_the_same_id_cancels_the_watchdog() {
        for done in [Event::SpeakDone { id: 5 }, Event::TtsFailed { id: 5 }] {
            let mut r = rig();
            r.runner.set_media_mode(MediaMode::Held);
            r.runner.run(vec![speak(5, text_of_len(10))]);
            r.runner.observe(&done);
            advance(ms(60_000)).await;
            assert!(drain(&mut r.rx).is_empty(), "{done:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn spoken_done_of_another_id_keeps_the_watchdog() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![speak(5, text_of_len(10))]);
        r.runner.observe(&Event::SpeakDone { id: 4 });
        advance(ms(5000)).await;
        assert_eq!(drain(&mut r.rx).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn stop_speaking_cancels_the_watchdog() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        r.runner
            .run(vec![speak(5, text_of_len(10)), Effect::StopSpeaking]);
        advance(ms(60_000)).await;
        assert!(drain(&mut r.rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_newer_speak_replaces_the_older_watchdog() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![speak(5, text_of_len(10))]);
        advance(ms(3000)).await;
        r.runner.run(vec![speak(6, text_of_len(10))]);
        advance(ms(2500)).await;
        assert!(drain(&mut r.rx).is_empty(), "the id 5 watchdog was dropped");
        advance(ms(2500)).await;
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token: 6
            }]
        );
    }

    // --- Chime -> ChimeDone ----------------------------------------------

    #[tokio::test(start_paused = true)]
    async fn chime_reports_chime_done_with_the_same_id() {
        let mut r = rig();
        r.runner.run(vec![Effect::Chime { id: 8 }]);
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        assert_eq!(drain(&mut r.rx), vec![Event::ChimeDone { id: 8 }]);
        r.runner.run(vec![Effect::Chime { id: 21 }]);
        for _ in 0..3 {
            tokio::task::yield_now().await;
        }
        assert_eq!(drain(&mut r.rx), vec![Event::ChimeDone { id: 21 }]);
    }

    #[tokio::test(start_paused = true)]
    async fn held_chime_never_completes_on_its_own() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![Effect::Chime { id: 8 }]);
        advance(ms(60_000)).await;
        assert!(drain(&mut r.rx).is_empty());
    }

    // --- window effects and logged stubs ---------------------------------

    #[tokio::test(start_paused = true)]
    async fn show_and_hide_island_reach_the_host_in_order() {
        let mut r = rig();
        r.runner.run(vec![
            Effect::ShowIsland,
            Effect::HideIsland,
            Effect::ShowIsland,
        ]);
        assert_eq!(*r.host.calls.lock().unwrap(), vec!["show", "hide", "show"]);
    }

    #[tokio::test(start_paused = true)]
    async fn request_effects_go_to_the_requests_handler_in_order() {
        let mut r = rig();
        r.runner.run(vec![
            Effect::StartRequest {
                req: 2,
                input: RequestInput::SearchSummary,
            },
            Effect::RunTool { req: 2 },
            Effect::CancelRequest { req: 2 },
        ]);
        assert_eq!(
            *r.requests.calls.lock().unwrap(),
            vec!["start 2 SearchSummary", "run 2", "cancel 2"]
        );
        advance(ms(60_000)).await;
        assert!(
            drain(&mut r.rx).is_empty(),
            "the handler answers on its own"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn effects_of_later_slices_are_logged_and_produce_no_event() {
        let mut r = rig();
        r.runner.run(vec![
            Effect::Listen {
                id: 1,
                mode: ListenMode::Command,
            },
            Effect::CancelListen { id: 1 },
            Effect::ForceEndOfSpeech { id: 1 },
            Effect::EnqueueReminder { count: 1 },
            Effect::DrainReminders,
        ]);
        advance(ms(60_000)).await;
        assert!(drain(&mut r.rx).is_empty());
        assert!(r.host.calls.lock().unwrap().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_all_drops_every_timer_and_the_watchdog() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![
            Effect::StartTimer {
                kind: TimerKind::Confirm,
                token: 1,
            },
            speak(2, text_of_len(10)),
        ]);
        r.runner.cancel_all();
        advance(ms(60_000)).await;
        assert!(drain(&mut r.rx).is_empty());
    }

    // --- sleep countdown: watching the user's input -----------------------

    fn watching_rig() -> (Rig, Arc<FakeActivity>) {
        let mut r = rig();
        let activity = Arc::new(FakeActivity::default());
        r.runner = EffectRunner::new(r.runner.sink.clone(), r.host.clone(), r.requests.clone())
            .with_activity(activity.clone());
        (r, activity)
    }

    #[tokio::test(start_paused = true)]
    async fn nothing_polls_the_input_before_a_watch_is_asked_for() {
        let (mut r, activity) = watching_rig();
        advance(ms(60_000)).await;
        assert_eq!(activity.reads(), 0);
        assert!(drain(&mut r.rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_watch_takes_the_baseline_answers_ready_and_polls_every_100_ms() {
        let (mut r, activity) = watching_rig();
        r.runner.run(vec![Effect::WatchInput { id: 3 }]);
        assert_eq!(drain(&mut r.rx), vec![Event::CountdownReady { id: 3 }]);
        assert_eq!(activity.reads(), 1, "the baseline");
        advance(ms(100)).await;
        assert_eq!(activity.reads(), 2);
        advance(ms(900)).await;
        assert_eq!(activity.reads(), 11);
        assert!(drain(&mut r.rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn mouse_jitter_right_after_the_accept_click_does_not_cancel() {
        let (mut r, activity) = watching_rig();
        r.runner.run(vec![Effect::WatchInput { id: 3 }]);
        drain(&mut r.rx);
        for _ in 0..9 {
            activity.touch();
            advance(ms(100)).await;
        }
        advance(ms(8_000)).await;
        assert!(drain(&mut r.rx).is_empty(), "the jitter was not input");
    }

    #[tokio::test(start_paused = true)]
    async fn input_after_the_grace_period_cancels_once_and_polling_stops() {
        let (mut r, activity) = watching_rig();
        r.runner.run(vec![Effect::WatchInput { id: 3 }]);
        drain(&mut r.rx);
        advance(ms(1_500)).await;
        activity.touch();
        advance(ms(100)).await;
        assert_eq!(drain(&mut r.rx), vec![Event::CountdownCancel { id: 3 }]);
        let reads = activity.reads();
        activity.touch();
        advance(ms(5_000)).await;
        assert_eq!(activity.reads(), reads, "no polling after the cancel");
        assert!(drain(&mut r.rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn an_unreadable_source_at_the_start_is_a_fault_and_never_ready() {
        let (mut r, activity) = watching_rig();
        activity.set_unreadable(true);
        r.runner.run(vec![Effect::WatchInput { id: 3 }]);
        assert_eq!(drain(&mut r.rx), vec![Event::CountdownFault { id: 3 }]);
        advance(ms(5_000)).await;
        assert_eq!(activity.reads(), 1, "no polling without a baseline");
        assert!(drain(&mut r.rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_source_that_fails_later_is_a_fault_and_polling_stops() {
        let (mut r, activity) = watching_rig();
        r.runner.run(vec![Effect::WatchInput { id: 3 }]);
        drain(&mut r.rx);
        advance(ms(300)).await;
        activity.set_unreadable(true);
        advance(ms(100)).await;
        assert_eq!(drain(&mut r.rx), vec![Event::CountdownFault { id: 3 }]);
        let reads = activity.reads();
        advance(ms(5_000)).await;
        assert_eq!(activity.reads(), reads);
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_the_watch_stops_the_polling() {
        let (mut r, activity) = watching_rig();
        r.runner.run(vec![Effect::WatchInput { id: 3 }]);
        drain(&mut r.rx);
        advance(ms(500)).await;
        r.runner.run(vec![Effect::StopWatchingInput]);
        let reads = activity.reads();
        activity.touch();
        advance(ms(60_000)).await;
        assert_eq!(activity.reads(), reads);
        assert!(drain(&mut r.rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_second_watch_replaces_the_first() {
        let (mut r, activity) = watching_rig();
        r.runner.run(vec![Effect::WatchInput { id: 3 }]);
        advance(ms(2_000)).await;
        r.runner.run(vec![Effect::WatchInput { id: 4 }]);
        drain(&mut r.rx);
        advance(ms(1_500)).await;
        activity.touch();
        advance(ms(100)).await;
        assert_eq!(drain(&mut r.rx), vec![Event::CountdownCancel { id: 4 }]);
    }

    #[tokio::test(start_paused = true)]
    async fn cancel_all_stops_the_watch() {
        let (mut r, activity) = watching_rig();
        r.runner.run(vec![Effect::WatchInput { id: 3 }]);
        drain(&mut r.rx);
        r.runner.cancel_all();
        let reads = activity.reads();
        advance(ms(60_000)).await;
        assert_eq!(activity.reads(), reads);
    }

    #[tokio::test(start_paused = true)]
    async fn a_runner_with_no_activity_source_fails_closed() {
        let mut r = rig();
        r.runner.run(vec![Effect::WatchInput { id: 7 }]);
        assert_eq!(drain(&mut r.rx), vec![Event::CountdownFault { id: 7 }]);
    }

    #[tokio::test(start_paused = true)]
    async fn finishing_the_countdown_goes_to_the_requests_handler_and_nowhere_else() {
        let mut r = rig();
        r.runner.run(vec![Effect::FinishCountdown { req: 9 }]);
        assert_eq!(*r.requests.calls.lock().unwrap(), vec!["finish 9"]);
        advance(ms(60_000)).await;
        assert!(drain(&mut r.rx).is_empty());
    }
}
