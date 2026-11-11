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
//! - The notch shows no text, so nothing is held for reading: the `Settle`
//!   (1.5 s, the reducer's) starts at the end of the speech and that is all.
//!
//! - `WatchInput{id}` reads a baseline of the user's input and polls it every
//!   100 ms until `StopWatchingInput` (or until it saw input or lost the
//!   source); an unreadable source is a `CountdownFault`, never silence. There
//!   is no task, and no read, when no countdown runs.
//! - `FinishCountdown{req}` goes to the [`Requests`] handler, nowhere else.
//!
//! Voice (task 3.4): `Listen`, `CancelListen` and `ForceEndOfSpeech` go to the
//! [`SpeechEngine`] port, `Speak` and `StopSpeaking` to the [`Speaker`] port
//! and `Chime` to the [`AudioCue`] port. While the speaker says it is not
//! available (the sidecar is down, or voice is marked down), a `Speak` is
//! skipped and reported done at once, and a `Chime` still plays: it does not
//! need the sidecar. Without those ports (tests, `niva-eval`) the effects
//! behave as the stubs did: `Speak` and `Chime` end at once, the listen
//! effects do nothing.
//!
//! Stubs until slice 4 (logged): `EnqueueReminder` and `DrainReminders` do
//! nothing. `StartRequest`, `RunTool` and `CancelRequest` go to the
//! [`Requests`] handler. No log line carries a reply or a transcript.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use niva_core::activity::{InputWatch, POLL_INTERVAL, Watch};
use niva_core::ports::{
    ActivityError, AudioCue, InputActivity, Speaker, SpeechEngine, SpokenPrompt,
};
use niva_core::state_machine::{Effect, Event, ListenMode, ReplyKind, Say, Template, TimerKind};
use tokio::task::JoinHandle;

use crate::fullscreen::{Fullscreen, FullscreenProbe};
use crate::requests::Requests;
use crate::ui_adapter::{say_text, template_caption};

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

/// The replies that tell the user the voice cannot be used.
fn is_voice_down_notice(say: &Say) -> bool {
    matches!(
        say,
        Say::Template(Template::VoiceNotReady | Template::Sidecar(_))
    )
}

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

/// Where the text of each spoken reply goes in debug builds (the dev log, the
/// debug window). Release builds never call it: the notch shows no text and
/// the logs carry none.
pub type SpeakLog = Arc<dyn Fn(String) + Send + Sync>;

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
    /// Where the user's input is read (sleep countdown).
    activity: Arc<dyn InputActivity>,
    /// The poller of the running countdown, if any.
    watcher: Option<JoinHandle<()>>,
    /// The voice sidecar's listening half, its speaking half and the host's own
    /// sounds. Each is optional: without one the effect is the old stub.
    speech: Option<Arc<dyn SpeechEngine>>,
    speaker: Option<Arc<dyn Speaker>>,
    cue: Option<Arc<dyn AudioCue>>,
    /// The confirmation question spoken last, for the confirm listen that
    /// follows it. Any other `Speak`, and a command `Listen`, make it stale.
    prompt: Option<SpokenPrompt>,
    /// Debug builds: one line with the reply text per `Speak`.
    speak_log: Option<SpeakLog>,
    /// What is in front of the desktop (exclusive fullscreen hides the notch).
    fullscreen: Option<Arc<dyn FullscreenProbe>>,
    /// The id of the listen waiting for the fullscreen cue to end, if any. The
    /// cue's callback opens it only while it is still this id; a new `Listen`
    /// or its own `CancelListen` clears it.
    pending_listen: Arc<Mutex<Option<u64>>>,
    /// How long the pending cue delays the microphone: added to the reducer's
    /// no-speech window, which starts right after the `Listen` effect.
    cue_delay: Duration,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
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
            speak_log: None,
            fullscreen: None,
            pending_listen: Arc::new(Mutex::new(None)),
            cue_delay: Duration::ZERO,
            activity: Arc::new(NoActivity),
            watcher: None,
            speech: None,
            speaker: None,
            cue: None,
            prompt: None,
        }
    }

    /// Wires the voice sidecar: what it hears and what it says.
    pub fn with_voice(mut self, speech: Arc<dyn SpeechEngine>, speaker: Arc<dyn Speaker>) -> Self {
        self.speech = Some(speech);
        self.speaker = Some(speaker);
        self
    }

    /// Wires the host's own sounds (the alert chime).
    pub fn with_cue(mut self, cue: Arc<dyn AudioCue>) -> Self {
        self.cue = Some(cue);
        self
    }

    /// Where the runner asks what is in front of the desktop. Without one the
    /// notch is assumed visible.
    pub fn with_fullscreen(mut self, probe: Arc<dyn FullscreenProbe>) -> Self {
        self.fullscreen = Some(probe);
        self
    }

    /// Debug builds: where the text of each `Speak` is logged. Ignored in release.
    pub fn with_speak_log(mut self, log: SpeakLog) -> Self {
        self.speak_log = Some(log);
        self
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
                if let Some(speaker) = &self.speaker {
                    speaker.stop();
                }
                if let Some(cue) = &self.cue {
                    cue.stop();
                }
            }
            Effect::Listen { id, mode } => self.listen(id, mode),
            Effect::CancelListen { id } => {
                // Only the listen that is waiting for its cue is dropped; the
                // cancel of any other listen must not touch it.
                let mut pending = lock(&self.pending_listen);
                if *pending == Some(id) {
                    *pending = None;
                    self.cue_delay = Duration::ZERO;
                }
                drop(pending);
                if let Some(speech) = &self.speech {
                    speech.cancel_listen(id);
                }
            }
            Effect::ForceEndOfSpeech { id } => {
                if let Some(speech) = &self.speech {
                    speech.force_end_of_speech(id);
                }
            }
            Effect::StartRequest { req, input } => self.requests.start(req, input),
            Effect::RunTool { req } => self.requests.run_tools(req),
            Effect::CancelRequest { req } => self.requests.cancel(req),
            Effect::WatchInput { id } => self.watch_input(id),
            Effect::StopWatchingInput => self.stop_watching(),
            Effect::FinishCountdown { req } => self.requests.finish_countdown(req),
            Effect::Chime { id } => self.chime(id),
            other => eprintln!("[niva] stub effect (later slice): {other:?}"),
        }
    }

    /// Opens a listen. Over an exclusive fullscreen app the notch cannot be
    /// drawn, so a command listen is announced by the host chime and opens when
    /// the chime is over: the microphone never hears the cue, and a listen
    /// cancelled in the meantime never opens.
    fn listen(&mut self, id: u64, mode: ListenMode) {
        // A new listen supersedes any listen still waiting for its cue.
        *lock(&self.pending_listen) = None;
        self.cue_delay = Duration::ZERO;
        let (MediaMode::Instant, Some(speech)) = (self.mode, self.speech.clone()) else {
            eprintln!("[niva] listen {id} ({mode:?}): no voice attached");
            return;
        };
        // The question only helps a confirm answer; a command listen also
        // forgets it, so a later confirm never reuses an old one.
        if mode == ListenMode::Command {
            self.prompt = None;
        }
        let hidden = self
            .fullscreen
            .as_ref()
            .is_some_and(|probe| probe.current() == Fullscreen::Exclusive);
        if mode == ListenMode::Command
            && hidden
            && let Some(cue) = self.cue.clone()
        {
            *lock(&self.pending_listen) = Some(id);
            // The reducer starts its no-speech window right after this effect;
            // it must count from when the microphone opens.
            self.cue_delay = cue.listening_cue_len();
            let pending = self.pending_listen.clone();
            cue.listening_cue(Box::new(move || {
                let mut pending = lock(&pending);
                if *pending == Some(id) {
                    *pending = None;
                    if !speech.listen(id, ListenMode::Command) {
                        eprintln!("[niva] listen {id}: the voice is not running");
                    }
                }
            }));
            return;
        }
        if !speech.listen_with_prompt(id, mode, self.prompt.as_ref()) {
            eprintln!("[niva] listen {id}: the voice is not running");
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
        let duration = if kind == TimerKind::NoSpeech {
            duration + std::mem::take(&mut self.cue_delay)
        } else {
            duration
        };
        let sink = self.sink.clone();
        let deadline = tokio::time::Instant::now() + duration;
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

    /// The alert chime, played by the host (it works with the sidecar down).
    /// `ChimeDone` follows when it ended or could not play.
    fn chime(&mut self, id: u64) {
        if self.mode != MediaMode::Instant {
            eprintln!("[niva] chime {id} held (no ChimeDone)");
            return;
        }
        match &self.cue {
            Some(cue) => {
                let sink = self.sink.clone();
                cue.chime(Box::new(move || sink(Event::ChimeDone { id })));
            }
            None => (self.sink)(Event::ChimeDone { id }),
        }
    }

    fn speak(&mut self, id: u64, kind: ReplyKind, say: &Say) {
        // Never the text itself: a reply can quote what the user asked.
        eprintln!("[niva] speak {id} ({kind:?}, {} chars)", char_count(say));
        self.arm_watchdog(id, speak_watchdog_duration(say));
        // Every speak replaces the remembered question, held or not, so a
        // later confirm listen never carries an older confirmation's question.
        let lang = self.requests.lang();
        // Debug builds only: the reply text goes to the dev log, nowhere else.
        if cfg!(debug_assertions)
            && let Some(log) = &self.speak_log
        {
            log(format!("niva says: {}", say_text(say, lang)));
        }
        self.prompt = (kind == ReplyKind::ConfirmPrompt).then(|| SpokenPrompt {
            lang,
            text: say_text(say, lang),
        });
        if self.mode != MediaMode::Instant {
            return; // the debug window holds the media: only the watchdog ends it
        }
        // With the sidecar down (or no speaker at all) the speech "ends" at
        // once; the watchdog above still guards a sidecar that never answers.
        let spoken = self
            .speaker
            .as_ref()
            .filter(|speaker| speaker.is_available())
            .is_some_and(|speaker| speaker.speak(id, &say_text(say, lang), lang));
        // The host chime stands in for a voice that cannot speak the notice
        // (design L15). When the sidecar is up it says the error itself, and a
        // chime on top would overlap the speech.
        if !spoken
            && is_voice_down_notice(say)
            && let Some(cue) = &self.cue
        {
            cue.chime(Box::new(|| {}));
        }
        if !spoken {
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
        lang: Mutex<niva_core::replies::Lang>,
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
            *self.lang.lock().unwrap()
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

    // --- settle: 1.5 s after the speech, with no reading hold (design L7) --

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
    async fn settle_timer_is_1500ms() {
        let mut r = rig();
        r.runner
            .run(vec![speak(1, Say::Text("Done".into())), settle(5)]);
        advance(ms(1499)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(5)));
        advance(ms(1)).await;
        assert_eq!(drain(&mut r.rx), vec![settle_fire(5)]);
    }

    #[tokio::test(start_paused = true)]
    async fn long_reply_settle_starts_at_speech_end_without_hold() {
        let mut r = rig();
        r.runner.set_media_mode(MediaMode::Held);
        // 400 characters, spoken for 8 s (held media): the settle is the plain
        // 1.5 s from the moment the speech ends, never a reading time.
        r.runner.run(vec![speak(1, text_of_len(400))]);
        advance(ms(8_000)).await;
        r.runner.run(vec![settle(6)]);
        advance(ms(1499)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(6)));
        advance(ms(1)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(6)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_long_reply_with_no_voice_settles_at_once_plus_1_5_s() {
        let mut r = rig();
        r.runner.run(vec![speak(1, text_of_len(400)), settle(5)]);
        advance(ms(1499)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(5)));
        advance(ms(1)).await;
        assert!(drain(&mut r.rx).contains(&settle_fire(5)));
    }

    #[tokio::test(start_paused = true)]
    async fn a_new_activation_cancels_the_pending_settle() {
        let mut r = rig();
        r.runner
            .run(vec![speak(1, Say::Text("Done".into())), settle(5)]);
        advance(ms(1000)).await;
        // The reducer cancels the settle when the next activation starts.
        r.runner.run(vec![Effect::CancelTimer {
            kind: TimerKind::Settle,
        }]);
        advance(ms(5000)).await;
        assert!(!drain(&mut r.rx).contains(&settle_fire(5)));
    }

    // --- dev log: reply text goes to the debug log only (design L5) -------

    type Lines = Arc<Mutex<Vec<String>>>;

    fn with_log(r: &mut Rig) -> Lines {
        let lines: Lines = Arc::default();
        let sink = lines.clone();
        let log: SpeakLog = Arc::new(move |line| sink.lock().unwrap().push(line));
        r.runner = EffectRunner::new(r.runner.sink.clone(), r.host.clone(), r.requests.clone())
            .with_speak_log(log);
        lines
    }

    #[tokio::test(start_paused = true)]
    #[cfg(debug_assertions)]
    async fn speak_logs_reply_text_to_dev_log_in_debug_only() {
        let mut r = rig();
        let lines = with_log(&mut r);
        r.runner.run(vec![
            speak(1, Say::Text("Opening Notepad".into())),
            speak(2, Say::Template(Template::ActionDone)),
        ]);
        let lines = lines.lock().unwrap().clone();
        assert_eq!(lines.len(), 2, "one line per Speak: {lines:?}");
        assert!(lines[0].contains("Opening Notepad"), "{lines:?}");
        assert!(lines[1].contains(template_caption(Template::ActionDone)));
    }

    #[tokio::test(start_paused = true)]
    #[cfg(not(debug_assertions))]
    async fn speak_logs_reply_text_to_dev_log_in_debug_only() {
        let mut r = rig();
        let lines = with_log(&mut r);
        r.runner
            .run(vec![speak(1, Say::Text("Opening Notepad".into()))]);
        assert!(
            lines.lock().unwrap().is_empty(),
            "release builds never log text"
        );
    }

    // --- timers (task 1.14: settle fires at 1.5 s) -------------------------

    #[tokio::test(start_paused = true)]
    async fn settle_timer_fires_at_exactly_1_5_s_with_its_token() {
        let mut r = rig();
        r.runner.run(vec![Effect::StartTimer {
            kind: TimerKind::Settle,
            token: 7,
        }]);
        advance(ms(1499)).await;
        assert!(drain(&mut r.rx).is_empty(), "fired before 1.5 s");
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

    // --- voice (task 3.4): the ports ---------------------------------------

    use niva_core::ports::{AudioCue, Speaker, SpeechEngine};
    use niva_core::replies::Lang;

    /// Records every call; `up` is what `is_available` answers.
    #[derive(Default)]
    struct FakeVoice {
        calls: Mutex<Vec<String>>,
        up: std::sync::atomic::AtomicBool,
    }

    impl FakeVoice {
        fn up() -> Arc<FakeVoice> {
            let voice = Arc::new(FakeVoice::default());
            voice.up.store(true, std::sync::atomic::Ordering::SeqCst);
            voice
        }

        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl SpeechEngine for FakeVoice {
        fn listen(&self, id: u64, mode: ListenMode) -> bool {
            self.listen_with_prompt(id, mode, None)
        }
        fn listen_with_prompt(
            &self,
            id: u64,
            mode: ListenMode,
            prompt: Option<&niva_core::ports::SpokenPrompt>,
        ) -> bool {
            let context = prompt.map_or(String::new(), |p| {
                format!(" [{}] {}", p.lang.code(), p.text)
            });
            self.calls
                .lock()
                .unwrap()
                .push(format!("listen {id} {mode:?}{context}"));
            true
        }
        fn cancel_listen(&self, id: u64) -> bool {
            self.calls.lock().unwrap().push(format!("cancel {id}"));
            true
        }
        fn force_end_of_speech(&self, id: u64) -> bool {
            self.calls.lock().unwrap().push(format!("force_end {id}"));
            true
        }
    }

    impl Speaker for FakeVoice {
        fn is_available(&self) -> bool {
            self.up.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn speak(&self, id: u64, text: &str, lang: Lang) -> bool {
            self.calls
                .lock()
                .unwrap()
                .push(format!("speak {id} {} {text}", lang.code()));
            true
        }
        fn stop(&self) -> bool {
            self.calls.lock().unwrap().push("stop".into());
            true
        }
    }

    #[derive(Default)]
    struct FakeCue {
        calls: Mutex<Vec<String>>,
        done: Mutex<Vec<Box<dyn FnOnce() + Send>>>,
        listening_ms: std::sync::atomic::AtomicU64,
    }

    impl AudioCue for FakeCue {
        fn chime(&self, done: Box<dyn FnOnce() + Send>) {
            self.calls.lock().unwrap().push("chime".into());
            self.done.lock().unwrap().push(done);
        }
        fn listening_cue(&self, done: Box<dyn FnOnce() + Send>) {
            self.calls.lock().unwrap().push("listening".into());
            self.done.lock().unwrap().push(done);
        }
        fn listening_cue_len(&self) -> Duration {
            ms(self.listening_ms.load(std::sync::atomic::Ordering::SeqCst))
        }
        fn stop(&self) {
            self.calls.lock().unwrap().push("cue stop".into());
        }
    }

    fn voice_rig(voice: &Arc<FakeVoice>) -> Rig {
        let mut r = rig();
        r.runner = EffectRunner::new(r.runner.sink.clone(), r.host.clone(), r.requests.clone())
            .with_voice(voice.clone(), voice.clone());
        r
    }

    // --- voice down: the host plays the chime itself (design L15) ----------

    fn cue_rig(voice: Option<&Arc<FakeVoice>>) -> (Rig, Arc<FakeCue>) {
        let cue = Arc::new(FakeCue::default());
        let mut r = rig();
        let mut runner =
            EffectRunner::new(r.runner.sink.clone(), r.host.clone(), r.requests.clone())
                .with_cue(cue.clone());
        if let Some(voice) = voice {
            runner = runner.with_voice(voice.clone(), voice.clone());
        }
        r.runner = runner;
        (r, cue)
    }

    fn say(template: Template) -> Effect {
        Effect::Speak {
            id: 1,
            kind: ReplyKind::Final,
            say: Say::Template(template),
        }
    }

    #[tokio::test(start_paused = true)]
    async fn sidecar_down_activation_plays_host_chime_and_error_view() {
        // Sidecar down: the speaker is not available, so nothing can be said.
        let voice = FakeVoice::default();
        let voice = Arc::new(voice);
        let (mut r, cue) = cue_rig(Some(&voice));
        r.runner.run(vec![say(Template::VoiceNotReady)]);
        assert_eq!(*cue.calls.lock().unwrap(), vec!["chime"]);
        // No sidecar involvement: the voice got nothing.
        assert!(voice.calls().is_empty(), "{:?}", voice.calls());
    }

    #[tokio::test(start_paused = true)]
    async fn mic_error_is_spoken_by_the_sidecar_with_no_host_chime_when_it_is_up() {
        use niva_core::state_machine::SidecarErrorCode::{MicBusy, MicMissing};
        for code in [MicBusy, MicMissing] {
            let voice = FakeVoice::up();
            let (mut r, cue) = cue_rig(Some(&voice));
            r.runner.run(vec![say(Template::Sidecar(code))]);
            assert!(cue.calls.lock().unwrap().is_empty(), "{code:?}");
            assert_eq!(voice.calls().len(), 1, "{:?}", voice.calls());
            assert!(voice.calls()[0].starts_with("speak 1 "), "{code:?}");
        }
    }

    #[tokio::test(start_paused = true)]
    async fn mic_error_with_the_voice_down_plays_the_chime_and_says_nothing() {
        use niva_core::state_machine::SidecarErrorCode::MicBusy;
        let voice = Arc::new(FakeVoice::default());
        let (mut r, cue) = cue_rig(Some(&voice));
        r.runner.run(vec![say(Template::Sidecar(MicBusy))]);
        assert_eq!(*cue.calls.lock().unwrap(), vec!["chime"]);
        assert!(voice.calls().is_empty(), "{:?}", voice.calls());
    }

    #[tokio::test(start_paused = true)]
    async fn a_failed_speak_of_a_voice_notice_falls_back_to_the_chime() {
        struct Refusing;
        impl Speaker for Refusing {
            fn is_available(&self) -> bool {
                true
            }
            fn speak(&self, _: u64, _: &str, _: Lang) -> bool {
                false
            }
            fn stop(&self) -> bool {
                true
            }
        }
        let voice = FakeVoice::up();
        let (mut r, cue) = cue_rig(None);
        r.runner = EffectRunner::new(r.runner.sink.clone(), r.host.clone(), r.requests.clone())
            .with_voice(voice.clone(), Arc::new(Refusing))
            .with_cue(cue.clone());
        r.runner.run(vec![say(Template::VoiceNotReady)]);
        assert_eq!(*cue.calls.lock().unwrap(), vec!["chime"]);
    }

    #[tokio::test(start_paused = true)]
    async fn ordinary_replies_play_no_chime() {
        let voice = FakeVoice::up();
        let (mut r, cue) = cue_rig(Some(&voice));
        r.runner.run(vec![
            say(Template::ActionDone),
            say(Template::Error),
            speak(2, Say::Text("Opened Notepad.".into())),
        ]);
        assert!(cue.calls.lock().unwrap().is_empty());
    }

    // --- exclusive fullscreen: a cue instead of the notch (design L16) ------

    struct FakeFullscreen(Fullscreen);

    impl FullscreenProbe for FakeFullscreen {
        fn current(&self) -> Fullscreen {
            self.0
        }
    }

    fn fullscreen_rig(state: Fullscreen) -> (Rig, Arc<FakeCue>, Arc<FakeVoice>) {
        let voice = FakeVoice::up();
        let (mut r, cue) = cue_rig(None);
        r.runner = EffectRunner::new(r.runner.sink.clone(), r.host.clone(), r.requests.clone())
            .with_voice(voice.clone(), voice.clone())
            .with_cue(cue.clone())
            .with_fullscreen(Arc::new(FakeFullscreen(state)));
        (r, cue, voice)
    }

    fn command_listen(id: u64) -> Effect {
        Effect::Listen {
            id,
            mode: ListenMode::Command,
        }
    }

    #[tokio::test(start_paused = true)]
    async fn exclusive_fullscreen_plays_cue_on_listen_start_instead_of_notch() {
        let (mut r, cue, voice) = fullscreen_rig(Fullscreen::Exclusive);
        r.runner.run(vec![command_listen(3)]);
        // The short listening cue, not the long alert chime.
        assert_eq!(*cue.calls.lock().unwrap(), vec!["listening"]);
        // The microphone opens only when the cue is over, so it never hears it.
        assert!(voice.calls().is_empty(), "{:?}", voice.calls());
        let done = cue.done.lock().unwrap().pop().expect("the cue's callback");
        done();
        assert_eq!(voice.calls(), vec!["listen 3 Command"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_listen_cancelled_during_the_cue_never_opens() {
        let (mut r, cue, voice) = fullscreen_rig(Fullscreen::Exclusive);
        r.runner
            .run(vec![command_listen(3), Effect::CancelListen { id: 3 }]);
        let done = cue.done.lock().unwrap().pop().expect("the cue's callback");
        done();
        assert_eq!(voice.calls(), vec!["cancel 3"]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_cancel_of_another_listen_keeps_the_pending_one() {
        let (mut r, cue, voice) = fullscreen_rig(Fullscreen::Exclusive);
        r.runner
            .run(vec![command_listen(3), Effect::CancelListen { id: 2 }]);
        let done = cue.done.lock().unwrap().pop().expect("the cue's callback");
        done();
        assert_eq!(voice.calls(), vec!["cancel 2", "listen 3 Command"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_confirm_listen_supersedes_a_pending_cue_listen() {
        let (mut r, cue, voice) = fullscreen_rig(Fullscreen::Exclusive);
        r.runner.run(vec![
            command_listen(3),
            Effect::Listen {
                id: 4,
                mode: ListenMode::Confirm,
            },
        ]);
        // The confirm listen opened the microphone at once.
        assert_eq!(voice.calls(), vec!["listen 4 Confirm"]);
        let done = cue.done.lock().unwrap().pop().expect("the cue's callback");
        done();
        // The stale cue opens nothing more.
        assert_eq!(voice.calls(), vec!["listen 4 Confirm"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_newer_cue_listen_supersedes_the_older_one() {
        let (mut r, cue, voice) = fullscreen_rig(Fullscreen::Exclusive);
        r.runner.run(vec![command_listen(3), command_listen(4)]);
        let mut dones: Vec<_> = cue.done.lock().unwrap().drain(..).collect();
        for done in dones.drain(..) {
            done();
        }
        assert_eq!(voice.calls(), vec!["listen 4 Command"]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_no_speech_window_counts_from_when_the_cue_is_over() {
        let (mut r, cue, _voice) = fullscreen_rig(Fullscreen::Exclusive);
        cue.listening_ms
            .store(150, std::sync::atomic::Ordering::SeqCst);
        r.runner.run(vec![
            command_listen(3),
            Effect::StartTimer {
                kind: TimerKind::NoSpeech,
                token: 3,
            },
        ]);
        advance(ms(6_149)).await;
        assert!(drain(&mut r.rx).is_empty());
        advance(ms(1)).await;
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::Watchdog {
                kind: TimerKind::NoSpeech,
                token: 3
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_no_speech_window_is_not_extended_without_a_cue() {
        let (mut r, _cue, _voice) = fullscreen_rig(Fullscreen::No);
        r.runner.run(vec![
            command_listen(3),
            Effect::StartTimer {
                kind: TimerKind::NoSpeech,
                token: 3,
            },
        ]);
        advance(ms(6_000)).await;
        assert_eq!(drain(&mut r.rx).len(), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn without_exclusive_fullscreen_the_listen_opens_at_once_with_no_cue() {
        for state in [Fullscreen::No, Fullscreen::Windowed] {
            let (mut r, cue, voice) = fullscreen_rig(state);
            r.runner.run(vec![
                command_listen(3),
                Effect::Listen {
                    id: 4,
                    mode: ListenMode::Confirm,
                },
            ]);
            assert!(cue.calls.lock().unwrap().is_empty(), "{state:?}");
            assert_eq!(voice.calls(), vec!["listen 3 Command", "listen 4 Confirm"]);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_confirm_listen_gets_no_cue_even_over_an_exclusive_game() {
        let (mut r, cue, voice) = fullscreen_rig(Fullscreen::Exclusive);
        r.runner.run(vec![Effect::Listen {
            id: 4,
            mode: ListenMode::Confirm,
        }]);
        assert!(cue.calls.lock().unwrap().is_empty());
        assert_eq!(voice.calls(), vec!["listen 4 Confirm"]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_listen_effects_go_to_the_speech_port() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        r.runner.run(vec![
            Effect::Listen {
                id: 3,
                mode: ListenMode::Command,
            },
            Effect::ForceEndOfSpeech { id: 3 },
            Effect::CancelListen { id: 3 },
            Effect::Listen {
                id: 4,
                mode: ListenMode::Confirm,
            },
        ]);
        assert_eq!(
            voice.calls(),
            vec![
                "listen 3 Command",
                "force_end 3",
                "cancel 3",
                "listen 4 Confirm"
            ]
        );
        assert!(drain(&mut r.rx).is_empty());
    }

    fn confirm_prompt(id: u64, text: &str) -> Effect {
        Effect::Speak {
            id,
            kind: ReplyKind::ConfirmPrompt,
            say: Say::Text(text.into()),
        }
    }

    fn listen(id: u64, mode: ListenMode) -> Effect {
        Effect::Listen { id, mode }
    }

    #[tokio::test(start_paused = true)]
    async fn a_confirm_listen_carries_the_prompt_just_spoken_and_a_command_listen_does_not() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        *r.requests.lang.lock().unwrap() = Lang::Es;
        r.runner.run(vec![
            confirm_prompt(1, "¿Cierro la calculadora?"),
            listen(2, ListenMode::Confirm),
            listen(3, ListenMode::Confirm), // reissued after an unclear answer
            listen(4, ListenMode::Command),
        ]);
        assert_eq!(
            voice.calls(),
            vec![
                "speak 1 es ¿Cierro la calculadora?",
                "listen 2 Confirm [es] ¿Cierro la calculadora?",
                "listen 3 Confirm [es] ¿Cierro la calculadora?",
                "listen 4 Command"
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_reply_spoken_in_held_mode_still_makes_the_question_stale() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        r.runner.run(vec![confirm_prompt(1, "Close it?")]);
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![speak(2, Say::Text("Done".into()))]);
        r.runner.set_media_mode(MediaMode::Instant);
        r.runner.run(vec![listen(3, ListenMode::Confirm)]);
        let listens: Vec<String> = voice
            .calls()
            .into_iter()
            .filter(|c| c.starts_with("listen"))
            .collect();
        assert_eq!(listens, vec!["listen 3 Confirm"]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_missing_or_stale_prompt_means_a_confirm_listen_without_one() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        // No prompt was spoken at all.
        r.runner.run(vec![listen(1, ListenMode::Confirm)]);
        // A prompt, then another reply, then a confirm listen: the prompt is stale.
        r.runner.run(vec![
            confirm_prompt(2, "Close it?"),
            speak(3, Say::Text("Done".into())),
            listen(4, ListenMode::Confirm),
        ]);
        // A prompt, a command turn, then a confirm listen: stale as well.
        r.runner.run(vec![
            confirm_prompt(5, "Close it?"),
            listen(6, ListenMode::Command),
            listen(7, ListenMode::Confirm),
        ]);
        let listens: Vec<String> = voice
            .calls()
            .into_iter()
            .filter(|c| c.starts_with("listen"))
            .collect();
        assert_eq!(
            listens,
            vec![
                "listen 1 Confirm",
                "listen 4 Confirm",
                "listen 6 Command",
                "listen 7 Confirm"
            ]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_speak_goes_to_the_sidecar_and_waits_for_its_answer() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        r.runner.run(vec![speak(5, Say::Text("Done".into()))]);
        assert_eq!(voice.calls(), vec!["speak 5 en Done"]);
        // Nothing is synthesized: the sidecar's `speak_done` ends it.
        assert!(drain(&mut r.rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_template_is_spoken_in_the_language_of_the_request() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        *r.requests.lang.lock().unwrap() = Lang::Es;
        r.runner
            .run(vec![speak(5, Say::Template(Template::ActionDone))]);
        let calls = voice.calls();
        assert!(calls[0].starts_with("speak 5 es "), "{calls:?}");
        assert!(calls[0].len() > "speak 5 es ".len());
    }

    #[tokio::test(start_paused = true)]
    async fn a_sidecar_that_never_answers_a_speak_is_ended_by_the_watchdog() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        r.runner.run(vec![speak(5, Say::Text("Done".into()))]);
        advance(ms(4_999)).await;
        assert!(drain(&mut r.rx).is_empty());
        advance(ms(1)).await;
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token: 5
            }]
        );
    }

    #[tokio::test(start_paused = true)]
    async fn the_sidecar_answering_disarms_the_watchdog() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        r.runner.run(vec![speak(5, Say::Text("Done".into()))]);
        r.runner.observe(&Event::SpeakDone { id: 5 });
        advance(ms(60_000)).await;
        assert!(drain(&mut r.rx).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_speak_while_voice_is_down_is_skipped_and_done_at_once() {
        let voice = Arc::new(FakeVoice::default());
        let mut r = voice_rig(&voice);
        r.runner.run(vec![speak(5, Say::Text("Done".into()))]);
        assert!(voice.calls().is_empty());
        assert_eq!(drain(&mut r.rx), vec![Event::SpeakDone { id: 5 }]);
        advance(ms(60_000)).await;
        assert!(drain(&mut r.rx).is_empty(), "the watchdog was left armed");
    }

    #[tokio::test(start_paused = true)]
    async fn held_media_sends_nothing_to_the_sidecar() {
        let voice = FakeVoice::up();
        let mut r = voice_rig(&voice);
        r.runner.set_media_mode(MediaMode::Held);
        r.runner.run(vec![
            speak(5, Say::Text("Done".into())),
            Effect::Listen {
                id: 6,
                mode: ListenMode::Command,
            },
        ]);
        assert!(voice.calls().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn stopping_speech_stops_the_sidecar_and_the_chime() {
        let voice = FakeVoice::up();
        let cue = Arc::new(FakeCue::default());
        let mut r = voice_rig(&voice);
        r.runner = EffectRunner::new(r.runner.sink.clone(), r.host.clone(), r.requests.clone())
            .with_voice(voice.clone(), voice.clone())
            .with_cue(cue.clone());
        r.runner.run(vec![Effect::StopSpeaking]);
        assert_eq!(voice.calls(), vec!["stop"]);
        assert_eq!(*cue.calls.lock().unwrap(), vec!["cue stop"]);
    }

    #[tokio::test(start_paused = true)]
    async fn the_chime_plays_even_with_voice_down_and_reports_done_when_it_ends() {
        let voice = Arc::new(FakeVoice::default());
        let cue = Arc::new(FakeCue::default());
        let mut r = voice_rig(&voice);
        r.runner = EffectRunner::new(r.runner.sink.clone(), r.host.clone(), r.requests.clone())
            .with_voice(voice.clone(), voice.clone())
            .with_cue(cue.clone());
        r.runner.run(vec![Effect::Chime { id: 9 }]);
        assert_eq!(*cue.calls.lock().unwrap(), vec!["chime"]);
        assert!(
            drain(&mut r.rx).is_empty(),
            "ChimeDone before the chime ended"
        );
        let done = cue.done.lock().unwrap().pop().unwrap();
        done();
        assert_eq!(drain(&mut r.rx), vec![Event::ChimeDone { id: 9 }]);
    }

    #[tokio::test(start_paused = true)]
    async fn a_runner_without_the_voice_ports_keeps_the_old_behavior() {
        let mut r = rig();
        r.runner.run(vec![
            Effect::Listen {
                id: 1,
                mode: ListenMode::Command,
            },
            Effect::Chime { id: 2 },
            speak(3, Say::Text("x".into())),
        ]);
        assert_eq!(
            drain(&mut r.rx),
            vec![Event::ChimeDone { id: 2 }, Event::SpeakDone { id: 3 }]
        );
    }
}
