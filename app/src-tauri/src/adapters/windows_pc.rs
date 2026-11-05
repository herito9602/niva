//! The release `PcActions` (PR 2d-a): the SAFE Windows actions for real, and
//! nothing else.
//!
//! - `open_app` and `running_processes` come from [`WinApps`](super::win_apps).
//! - `set_volume` comes from [`win_volume`](super::win_volume) (Core Audio).
//! - `close_app` and `power` do NOT act. They take an `Approved<..>` the
//!   safety gate hands out only after a confirmation, and a release build of
//!   this PR cannot collect one (no voice, no click path on the island), so the
//!   assistant is built `without_risky_actions` and a risky plan is refused
//!   before any prompt (`release.rs`). These two methods are the second
//!   barrier: even if an `Approved` value ever reached them, they would answer
//!   with a failure and touch nothing. The real versions arrive in PR 2d-b.
//!
//! Every method runs on a pool worker, never on the event loop.

use std::sync::Arc;

use niva_core::ports::{CloseOutcome, PcActions, PcError, ProcessInfo, VolumeResult};
use niva_core::safety::{Approved, CloseApp, Power};
use niva_core::tools::VolumeChange;

use super::win_apps::WinApps;
use super::win_volume::{self, CoreAudio};

/// What the PC needs from the app launcher and the process list.
pub trait AppControl: Send + Sync {
    /// Starts the app and returns the display name of what was launched.
    fn open(&self, name: &str) -> Result<String, PcError>;
    fn processes(&self) -> Vec<ProcessInfo>;
}

impl AppControl for WinApps {
    fn open(&self, name: &str) -> Result<String, PcError> {
        WinApps::open(self, name)
    }

    fn processes(&self) -> Vec<ProcessInfo> {
        self.running_processes()
    }
}

/// What the PC needs from the sound device.
pub trait VolumeControl: Send + Sync {
    fn change(&self, change: VolumeChange) -> Result<VolumeResult, PcError>;
}

/// The default output device, one COM bracket per call.
pub struct DefaultOutput;

impl VolumeControl for DefaultOutput {
    fn change(&self, change: VolumeChange) -> Result<VolumeResult, PcError> {
        win_volume::apply(&CoreAudio, change)
    }
}

pub struct WindowsPc {
    apps: Arc<dyn AppControl>,
    volume: Arc<dyn VolumeControl>,
}

impl WindowsPc {
    pub fn new(apps: Arc<dyn AppControl>, volume: Arc<dyn VolumeControl>) -> Self {
        WindowsPc { apps, volume }
    }

    /// The real system. Starts the app indexer in the background.
    pub fn real() -> Self {
        Self::new(Arc::new(WinApps::start()), Arc::new(DefaultOutput))
    }
}

const NOT_AVAILABLE: &str = "not available in this build";

impl PcActions for WindowsPc {
    fn open_app(&self, name: &str) -> Result<(), PcError> {
        self.launch_app(name).map(drop)
    }

    fn launch_app(&self, name: &str) -> Result<String, PcError> {
        self.apps.open(name)
    }

    fn running_processes(&self) -> Vec<ProcessInfo> {
        self.apps.processes()
    }

    fn close_app(&self, _approved: Approved<CloseApp>) -> Result<CloseOutcome, PcError> {
        Err(PcError::Failed(format!("closing apps is {NOT_AVAILABLE}")))
    }

    fn set_volume(&self, change: VolumeChange) -> Result<VolumeResult, PcError> {
        self.volume.change(change)
    }

    fn power(&self, _approved: Approved<Power>) -> Result<(), PcError> {
        Err(PcError::Failed(format!(
            "power actions are {NOT_AVAILABLE}"
        )))
    }
}

#[cfg(test)]
pub(crate) mod fakes {
    //! Fake parts for the tests of this module and of `release.rs`.

    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct FakeApps {
        pub opened: Mutex<Vec<String>>,
        pub listed: Mutex<usize>,
        pub open_result: Mutex<Option<PcError>>,
        /// The display name the launcher reports; the requested name if unset.
        pub launched_as: Mutex<Option<String>>,
    }

    impl AppControl for FakeApps {
        fn open(&self, name: &str) -> Result<String, PcError> {
            self.opened.lock().unwrap().push(name.into());
            match self.open_result.lock().unwrap().clone() {
                Some(error) => Err(error),
                None => Ok(self
                    .launched_as
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| name.to_string())),
            }
        }

        fn processes(&self) -> Vec<ProcessInfo> {
            *self.listed.lock().unwrap() += 1;
            vec![ProcessInfo {
                exe_name: "Spotify.exe".into(),
                in_user_session: true,
            }]
        }
    }

    #[derive(Default)]
    pub struct FakeOutput {
        pub changes: Mutex<Vec<VolumeChange>>,
        pub result: Mutex<Option<Result<VolumeResult, PcError>>>,
    }

    impl VolumeControl for FakeOutput {
        fn change(&self, change: VolumeChange) -> Result<VolumeResult, PcError> {
            self.changes.lock().unwrap().push(change);
            self.result
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(Ok(VolumeResult {
                    level: 30,
                    muted: false,
                    clamped: false,
                }))
        }
    }
}

#[cfg(test)]
mod tests {
    use niva_core::assistant::click_answer;
    use niva_core::safety::{Approval, SafetyGate, Validated};
    use niva_core::tools::{ArgValue, ToolCall};

    use super::fakes::*;
    use super::*;

    fn pc() -> (WindowsPc, Arc<FakeApps>, Arc<FakeOutput>) {
        let apps = Arc::new(FakeApps::default());
        let output = Arc::new(FakeOutput::default());
        (WindowsPc::new(apps.clone(), output.clone()), apps, output)
    }

    #[test]
    fn open_app_goes_to_the_app_launcher_and_keeps_its_errors() {
        let (pc, apps, _) = pc();
        assert_eq!(pc.open_app("notepad"), Ok(()));
        assert_eq!(*apps.opened.lock().unwrap(), ["notepad"]);
        assert_eq!(pc.launch_app("notepad"), Ok("notepad".to_string()));
        for error in [
            PcError::NotFound,
            PcError::NotReady,
            PcError::Failed("x".into()),
        ] {
            *apps.open_result.lock().unwrap() = Some(error.clone());
            assert_eq!(pc.open_app("notepad"), Err(error));
        }
    }

    #[test]
    fn launch_app_reports_the_name_of_the_entry_the_launcher_started() {
        let (pc, apps, _) = pc();
        *apps.launched_as.lock().unwrap() = Some("Bloc de notas".into());
        assert_eq!(pc.launch_app("notepad"), Ok("Bloc de notas".to_string()));
    }

    // The second barrier (spec pc-actions: close_app and power MUST NOT act even
    // if called). The first one is the assistant refusing the plan; this one is
    // the adapter itself. Approved values cannot be forged, so they come from
    // the real gate with a real confirmation.
    fn approved_close() -> Approved<CloseApp> {
        let gate = SafetyGate::new();
        let processes = [ProcessInfo {
            exe_name: "Spotify.exe".into(),
            in_user_session: true,
        }];
        let request = gate
            .validate_pinned("Spotify.exe", &processes, 1, 0)
            .expect("a running app validates");
        let answer = click_answer(request.confirmation_id(), true);
        match gate.approve(request, answer) {
            Ok(Approval::Close(approved)) => approved,
            other => panic!("expected an approved close: {other:?}"),
        }
    }

    fn approved_lock() -> Approved<Power> {
        let gate = SafetyGate::new();
        let call = ToolCall {
            name: "power_action".into(),
            args: [("action".to_string(), ArgValue::Str("lock".into()))]
                .into_iter()
                .collect(),
        };
        let Ok(Validated::Risky(request)) = gate.validate(&call, &[], 1, 0) else {
            panic!("lock is a risky request");
        };
        let answer = click_answer(request.confirmation_id(), true);
        match gate.approve(request, answer) {
            Ok(Approval::Power(approved)) => approved,
            other => panic!("expected an approved power action: {other:?}"),
        }
    }

    #[test]
    fn close_app_fails_and_touches_nothing_even_with_a_real_approval() {
        let (pc, apps, output) = pc();
        let result = pc.close_app(approved_close());
        assert!(
            matches!(&result, Err(PcError::Failed(why)) if why.contains("not available")),
            "{result:?}"
        );
        assert!(apps.opened.lock().unwrap().is_empty());
        assert_eq!(
            *apps.listed.lock().unwrap(),
            0,
            "no process was even listed"
        );
        assert!(output.changes.lock().unwrap().is_empty());
    }

    #[test]
    fn power_fails_and_touches_nothing_even_with_a_real_approval() {
        let (pc, apps, output) = pc();
        let result = pc.power(approved_lock());
        assert!(
            matches!(&result, Err(PcError::Failed(why)) if why.contains("not available")),
            "{result:?}"
        );
        assert!(apps.opened.lock().unwrap().is_empty());
        assert_eq!(*apps.listed.lock().unwrap(), 0);
        assert!(output.changes.lock().unwrap().is_empty());
    }

    #[test]
    fn the_process_list_comes_from_the_launcher_side() {
        let (pc, apps, _) = pc();
        assert_eq!(pc.running_processes().len(), 1);
        assert_eq!(*apps.listed.lock().unwrap(), 1);
    }

    #[test]
    fn set_volume_goes_to_the_output_device_and_reports_its_result() {
        let (pc, _, output) = pc();
        let result = pc.set_volume(VolumeChange::Set(30)).unwrap();
        assert_eq!(result.level, 30);
        assert_eq!(*output.changes.lock().unwrap(), [VolumeChange::Set(30)]);
        *output.result.lock().unwrap() =
            Some(Err(PcError::Failed("no audio output device".into())));
        assert_eq!(
            pc.set_volume(VolumeChange::Mute(true)),
            Err(PcError::Failed("no audio output device".into()))
        );
    }
}
