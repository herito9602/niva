//! The voice sidecar adapter (task 3.4): the Python speech sidecar of `voice/`
//! behind the core's speech and speaker ports.
//!
//! - [`wire`]: the JSON-lines protocol, both directions (pure).
//! - [`translate`]: sidecar message -> reducer event, the clock mapping of
//!   `end_ms`, the language gate, the per-request timestamps (pure).
//! - [`policy`]: restart counter, out-of-memory downgrade, degraded text mode,
//!   the `ready` timeout, the slow retry (pure).
//! - [`config`]: where Python, `voice/` and the models are found.
//! - [`process`]: spawn, pipes and kill; a job object keeps the sidecar from
//!   outliving the app.
//! - [`link`]: the `SpeechEngine` and `Speaker` ports.
//! - [`supervisor`]: the thread that ties them together.
//! - [`LevelSink`]: the audio level of Niva's voice, straight to the notch.
//!
//! The reducer never sees the process. It sees `VoiceReadinessChanged{ready}`
//! (the supervisor), `SidecarError{code, id}` and the speech events (the
//! translator), and sends `Listen`, `Speak` and friends to the ports.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::ui_adapter::{EmitTo, ISLAND_LABEL};

pub mod config;
pub mod link;
pub mod policy;
pub mod process;
pub mod supervisor;
pub mod translate;
pub mod wire;

pub use link::Link;
pub use policy::Status;
pub use supervisor::{EventOut, Gate, LevelOut, StatusOut, Supervisor};

use config::VoiceConfig;
use process::CommandLauncher;
use supervisor::{Deps, Settings};
use translate::Log;
pub use translate::{HostClock, ListenModes};
use wire::Level;

/// How long the first start waits for the LLM to be warm (design "Model-load
/// order": "a bounded wait (e.g. 30 s)").
pub const LLM_WARM_UP_WAIT: Duration = Duration::from_secs(30);

/// The event the notch listens to for the audio level of Niva's voice.
pub const LEVEL_EVENT: &str = "niva://level";

/// Sends each level to the island window and nowhere else. A level is not a
/// reducer event, never goes through the dispatcher, and is never logged (not
/// even a failure to deliver it): it is decoration at 30 lines a second.
pub struct LevelSink {
    emitter: Arc<dyn EmitTo>,
}

impl LevelSink {
    pub fn new(emitter: Arc<dyn EmitTo>) -> Self {
        LevelSink { emitter }
    }

    pub fn send(&self, level: Level) {
        let payload = json!({ "src": "tts", "bands": level.bands });
        // A window that is not there (closing, not built yet) just misses a frame.
        let _ = self.emitter.emit_to(ISLAND_LABEL, LEVEL_EVENT, payload);
    }

    pub fn into_out(self) -> LevelOut {
        Arc::new(move |level| self.send(level))
    }
}

/// The running voice: the handle for "Restart voice" and shutdown, and the
/// ports for the effect runner.
pub struct Voice {
    supervisor: Supervisor,
    link: Arc<Link>,
    modes: Arc<ListenModes>,
}

impl Voice {
    /// Starts the supervisor. Nothing here fails: a missing sidecar is a
    /// status ([`Status::Degraded`]) and text mode, never an error of the app.
    ///
    /// `llm_warm` says whether the LLM is warm; the first start waits for it
    /// (at most [`LLM_WARM_UP_WAIT`]) so the larger GPU allocation lands first.
    ///
    /// `levels` takes the audio level of Niva's voice for the notch.
    ///
    /// `clock` is the host's monotonic clock (milliseconds); the latency trace
    /// reads the same one, which is what puts every stage on one axis.
    pub fn start(
        events: EventOut,
        status: StatusOut,
        clock: HostClock,
        log: Log,
        levels: LevelOut,
        llm_warm: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    ) -> Voice {
        let config = VoiceConfig::detect();
        let mut settings = Settings::new(config.ladder());
        settings.gate = llm_warm.map(|open| Gate {
            open,
            max_wait: LLM_WARM_UP_WAIT,
        });
        let modes = Arc::new(ListenModes::default());
        let link = Link::new(modes.clone());
        let supervisor = Supervisor::start(Deps {
            launcher: Arc::new(CommandLauncher::new(|model| {
                // Read again at every start, so "Restart voice" sees a
                // folder or a variable that was fixed meanwhile.
                VoiceConfig::detect().launch_spec(model)
            })),
            link: link.clone(),
            modes: modes.clone(),
            events,
            clock,
            log,
            status,
            levels,
            settings,
        });
        Voice {
            supervisor,
            link,
            modes,
        }
    }

    pub fn link(&self) -> Arc<Link> {
        self.link.clone()
    }

    /// What the voice side shares with the latency trace (task 3.5).
    pub fn modes(&self) -> Arc<ListenModes> {
        self.modes.clone()
    }

    /// The tray's "Restart voice".
    pub fn restart(&self) {
        self.supervisor.restart();
    }

    /// Stops the sidecar before the app exits.
    pub fn shutdown(&self) {
        self.supervisor.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::{Value, json};

    use super::wire::Level;
    use super::{LEVEL_EVENT, LevelSink};
    use crate::ui_adapter::EmitTo;

    #[derive(Default)]
    struct Recorder {
        sent: Mutex<Vec<(String, String, Value)>>,
        fail: bool,
    }

    impl EmitTo for Recorder {
        fn emit_to(&self, label: &str, name: &str, payload: Value) -> Result<(), String> {
            self.sent
                .lock()
                .unwrap()
                .push((label.into(), name.into(), payload));
            if self.fail {
                Err("window gone".into())
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn level_emitted_to_island_window_only() {
        let recorder = Arc::new(Recorder::default());
        let sink = LevelSink::new(recorder.clone());
        sink.send(Level {
            bands: [0, 12, 40, 55, 30, 8, 100],
        });
        let sent = recorder.sent.lock().unwrap();
        assert_eq!(
            *sent,
            vec![(
                "island".to_owned(),
                "niva://level".to_owned(),
                json!({"src": "tts", "bands": [0, 12, 40, 55, 30, 8, 100]})
            )]
        );
        assert_eq!(LEVEL_EVENT, "niva://level");
    }

    #[test]
    fn a_missing_window_is_not_an_error_and_is_not_reported() {
        let recorder = Arc::new(Recorder {
            fail: true,
            ..Recorder::default()
        });
        // Nothing to assert on but "does not panic": the sink has no log to write to.
        LevelSink::new(recorder.clone()).send(Level { bands: [1; 7] });
        assert_eq!(recorder.sent.lock().unwrap().len(), 1);
    }

    #[test]
    fn the_out_closure_feeds_the_sink() {
        let recorder = Arc::new(Recorder::default());
        let out = LevelSink::new(recorder.clone()).into_out();
        out(Level { bands: [5; 7] });
        out(Level { bands: [6; 7] });
        assert_eq!(recorder.sent.lock().unwrap().len(), 2);
    }
}
