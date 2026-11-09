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
//!
//! The reducer never sees the process. It sees `VoiceReadinessChanged{ready}`
//! (the supervisor), `SidecarError{code, id}` and the speech events (the
//! translator), and sends `Listen`, `Speak` and friends to the ports.

use std::sync::Arc;
use std::time::Duration;

pub mod config;
pub mod link;
pub mod policy;
pub mod process;
pub mod supervisor;
pub mod translate;
pub mod wire;

pub use link::Link;
pub use policy::Status;
pub use supervisor::{EventOut, Gate, StatusOut, Supervisor};

use config::VoiceConfig;
use process::CommandLauncher;
use supervisor::{Deps, Settings};
use translate::Log;
pub use translate::{HostClock, ListenModes};

/// How long the first start waits for the LLM to be warm (design "Model-load
/// order": "a bounded wait (e.g. 30 s)").
pub const LLM_WARM_UP_WAIT: Duration = Duration::from_secs(30);

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
    /// `clock` is the host's monotonic clock (milliseconds); the latency trace
    /// reads the same one, which is what puts every stage on one axis.
    pub fn start(
        events: EventOut,
        status: StatusOut,
        clock: HostClock,
        log: Log,
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
