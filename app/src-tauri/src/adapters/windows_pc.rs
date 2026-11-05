//! The release `PcActions` (PR 2d-a, completed in 2d-b): every action for real.
//!
//! - `open_app` and `running_processes` come from [`WinApps`](super::win_apps).
//! - `set_volume` comes from [`win_volume`](super::win_volume) (Core Audio).
//! - `close_app` asks the windows of the approved exe to close
//!   ([`win_close`](super::win_close)); `power` is [`win_power`](super::win_power).
//!   Both take an `Approved<..>` the safety gate hands out only after a
//!   confirmation, which the island's buttons collect (PR 2d-b part 1). The gate
//!   re-resolves the target before `close_app` is called, so this adapter acts
//!   on the exact exe the user confirmed and nothing else.
//!
//! Every method runs on a pool worker, never on the event loop.

use std::sync::Arc;

use niva_core::ports::{CloseOutcome, PcActions, PcError, ProcessInfo, VolumeResult};
use niva_core::safety::{Approved, CloseApp, Power};
use niva_core::tools::{PowerKind, VolumeChange};

use super::win_apps::WinApps;
use super::win_power::{self, RealPower};
use super::win_volume::{self, CoreAudio};

/// What the PC needs from the app launcher, the process list and the windows.
pub trait AppControl: Send + Sync {
    /// Starts the app and returns the display name of what was launched.
    fn open(&self, name: &str) -> Result<String, PcError>;
    /// The process list. An unreadable snapshot is an error.
    fn processes(&self) -> Result<Vec<ProcessInfo>, PcError>;
    /// Asks the windows of `exe` to close and waits for them (never forces).
    fn close(&self, exe: &str) -> Result<CloseOutcome, PcError>;
}

impl AppControl for WinApps {
    fn open(&self, name: &str) -> Result<String, PcError> {
        WinApps::open(self, name)
    }

    fn processes(&self) -> Result<Vec<ProcessInfo>, PcError> {
        self.running_processes()
    }

    fn close(&self, exe: &str) -> Result<CloseOutcome, PcError> {
        WinApps::close(self, exe)
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

/// What the PC needs to lock, sleep, restart or shut down.
pub trait PowerControl: Send + Sync {
    fn run(&self, kind: PowerKind) -> Result<(), PcError>;
}

/// The real OS ([`win_power::apply`] over [`RealPower`]).
pub struct OsPower;

impl PowerControl for OsPower {
    fn run(&self, kind: PowerKind) -> Result<(), PcError> {
        win_power::apply(&RealPower, kind)
    }
}

pub struct WindowsPc {
    apps: Arc<dyn AppControl>,
    volume: Arc<dyn VolumeControl>,
    power: Arc<dyn PowerControl>,
}

impl WindowsPc {
    pub fn new(
        apps: Arc<dyn AppControl>,
        volume: Arc<dyn VolumeControl>,
        power: Arc<dyn PowerControl>,
    ) -> Self {
        WindowsPc {
            apps,
            volume,
            power,
        }
    }

    /// The real system. Starts the app indexer in the background.
    pub fn real() -> Self {
        Self::new(
            Arc::new(WinApps::start()),
            Arc::new(DefaultOutput),
            Arc::new(OsPower),
        )
    }
}

impl PcActions for WindowsPc {
    fn open_app(&self, name: &str) -> Result<(), PcError> {
        self.launch_app(name).map(drop)
    }

    fn launch_app(&self, name: &str) -> Result<String, PcError> {
        self.apps.open(name)
    }

    fn running_processes(&self) -> Result<Vec<ProcessInfo>, PcError> {
        self.apps.processes()
    }

    fn close_app(&self, approved: Approved<CloseApp>) -> Result<CloseOutcome, PcError> {
        self.apps.close(approved.action().exe())
    }

    fn set_volume(&self, change: VolumeChange) -> Result<VolumeResult, PcError> {
        self.volume.change(change)
    }

    fn power(&self, approved: Approved<Power>) -> Result<(), PcError> {
        self.power.run(approved.action().kind())
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
        /// What `close` is asked for, in order.
        pub closed: Mutex<Vec<String>>,
        /// What `close` answers; `Closed` if unset.
        pub close_result: Mutex<Option<Result<CloseOutcome, PcError>>>,
        /// When set, the process list fails with it.
        pub process_error: Mutex<Option<PcError>>,
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

        fn processes(&self) -> Result<Vec<ProcessInfo>, PcError> {
            *self.listed.lock().unwrap() += 1;
            if let Some(error) = self.process_error.lock().unwrap().clone() {
                return Err(error);
            }
            Ok(vec![ProcessInfo {
                exe_name: "Spotify.exe".into(),
                in_user_session: true,
            }])
        }

        fn close(&self, exe: &str) -> Result<CloseOutcome, PcError> {
            self.closed.lock().unwrap().push(exe.into());
            self.close_result
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(Ok(CloseOutcome::Closed))
        }
    }

    #[derive(Default)]
    pub struct FakePower {
        pub runs: Mutex<Vec<PowerKind>>,
        pub result: Mutex<Option<Result<(), PcError>>>,
    }

    impl PowerControl for FakePower {
        fn run(&self, kind: PowerKind) -> Result<(), PcError> {
            self.runs.lock().unwrap().push(kind);
            self.result.lock().unwrap().clone().unwrap_or(Ok(()))
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

    struct Rig {
        pc: WindowsPc,
        apps: Arc<FakeApps>,
        output: Arc<FakeOutput>,
        power: Arc<FakePower>,
    }

    fn rig() -> Rig {
        let apps = Arc::new(FakeApps::default());
        let output = Arc::new(FakeOutput::default());
        let power = Arc::new(FakePower::default());
        Rig {
            pc: WindowsPc::new(apps.clone(), output.clone(), power.clone()),
            apps,
            output,
            power,
        }
    }

    #[test]
    fn open_app_goes_to_the_app_launcher_and_keeps_its_errors() {
        let Rig { pc, apps, .. } = rig();
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
        let Rig { pc, apps, .. } = rig();
        *apps.launched_as.lock().unwrap() = Some("Bloc de notas".into());
        assert_eq!(pc.launch_app("notepad"), Ok("Bloc de notas".to_string()));
    }

    // Approved values cannot be forged, so they come from the real gate with a
    // real confirmation.
    fn approved_close(exe: &str) -> Approved<CloseApp> {
        let gate = SafetyGate::new();
        let processes = [ProcessInfo {
            exe_name: exe.into(),
            in_user_session: true,
        }];
        let request = gate
            .validate_pinned(exe, &processes, 1, 0)
            .expect("a running app validates");
        let answer = click_answer(request.confirmation_id(), true);
        match gate.approve(request, answer) {
            Ok(Approval::Close(approved)) => approved,
            other => panic!("expected an approved close: {other:?}"),
        }
    }

    fn power_request(action: &str) -> niva_core::safety::RiskyRequest {
        let call = ToolCall {
            name: "power_action".into(),
            args: [("action".to_string(), ArgValue::Str(action.into()))]
                .into_iter()
                .collect(),
        };
        match SafetyGate::new().validate(&call, &[], 1, 0) {
            Ok(Validated::Risky(request)) => request,
            other => panic!("{action} is a risky request: {other:?}"),
        }
    }

    fn approved_power(action: &str) -> Approved<Power> {
        let request = power_request(action);
        let answer = click_answer(request.confirmation_id(), true);
        match SafetyGate::new().approve(request, answer) {
            Ok(Approval::Power(approved)) => approved,
            other => panic!("expected an approved power action: {other:?}"),
        }
    }

    #[test]
    fn close_app_hands_the_confirmed_exe_to_the_window_closer_and_reports_its_outcome() {
        let Rig { pc, apps, .. } = rig();
        assert_eq!(
            pc.close_app(approved_close("Spotify.exe")),
            Ok(CloseOutcome::Closed)
        );
        assert_eq!(*apps.closed.lock().unwrap(), ["Spotify.exe"]);
        for outcome in [CloseOutcome::StillRunning, CloseOutcome::NoWindow] {
            *apps.close_result.lock().unwrap() = Some(Ok(outcome));
            assert_eq!(pc.close_app(approved_close("Spotify.exe")), Ok(outcome));
        }
    }

    #[test]
    fn close_app_keeps_the_errors_of_the_window_closer() {
        let Rig { pc, apps, .. } = rig();
        *apps.close_result.lock().unwrap() = Some(Err(PcError::Failed("nope".into())));
        assert_eq!(
            pc.close_app(approved_close("Spotify.exe")),
            Err(PcError::Failed("nope".into()))
        );
    }

    #[test]
    fn close_app_does_nothing_but_close() {
        let Rig {
            pc,
            apps,
            output,
            power,
        } = rig();
        pc.close_app(approved_close("Spotify.exe")).unwrap();
        assert!(apps.opened.lock().unwrap().is_empty());
        assert!(output.changes.lock().unwrap().is_empty());
        assert!(power.runs.lock().unwrap().is_empty());
    }

    #[test]
    fn power_hands_the_approved_action_to_the_power_control() {
        let Rig { pc, power, .. } = rig();
        for (action, kind) in [
            ("lock", PowerKind::Lock),
            ("restart", PowerKind::Restart),
            ("shutdown", PowerKind::Shutdown),
        ] {
            assert_eq!(pc.power(approved_power(action)), Ok(()), "{action}");
            assert_eq!(power.runs.lock().unwrap().last(), Some(&kind), "{action}");
        }
    }

    #[test]
    fn a_held_sleep_reaches_the_power_control_only_through_release_held() {
        let Rig { pc, power, .. } = rig();
        let gate = SafetyGate::new();
        let request = power_request("sleep");
        let answer = click_answer(request.confirmation_id(), true);
        let Ok(Approval::Sleep(held)) = gate.approve(request, answer) else {
            panic!("a sleep is held");
        };
        assert!(
            power.runs.lock().unwrap().is_empty(),
            "nothing before the countdown"
        );
        assert_eq!(gate.release_held(held, &pc), Ok(()));
        assert_eq!(*power.runs.lock().unwrap(), [PowerKind::Sleep]);
    }

    #[test]
    fn power_keeps_the_errors_of_the_power_control_including_not_supported() {
        let Rig { pc, power, .. } = rig();
        for error in [PcError::NotSupported, PcError::Failed("denied".into())] {
            *power.result.lock().unwrap() = Some(Err(error.clone()));
            assert_eq!(pc.power(approved_power("lock")), Err(error));
        }
    }

    #[test]
    fn power_does_nothing_but_power() {
        let Rig {
            pc, apps, output, ..
        } = rig();
        pc.power(approved_power("lock")).unwrap();
        assert!(apps.opened.lock().unwrap().is_empty());
        assert!(apps.closed.lock().unwrap().is_empty());
        assert_eq!(*apps.listed.lock().unwrap(), 0);
        assert!(output.changes.lock().unwrap().is_empty());
    }

    #[test]
    fn the_process_list_comes_from_the_launcher_side_and_a_failure_is_an_error() {
        let Rig { pc, apps, .. } = rig();
        assert_eq!(pc.running_processes().unwrap().len(), 1);
        assert_eq!(*apps.listed.lock().unwrap(), 1);
        *apps.process_error.lock().unwrap() = Some(PcError::Failed("no snapshot".into()));
        assert_eq!(
            pc.running_processes(),
            Err(PcError::Failed("no snapshot".into()))
        );
    }

    #[test]
    fn set_volume_goes_to_the_output_device_and_reports_its_result() {
        let Rig { pc, output, .. } = rig();
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

    // --- what the Windows adapters may never call ---------------------------------

    /// The code lines (comments dropped) of the adapters that act on windows,
    /// processes and power.
    fn adapter_code() -> String {
        [
            include_str!("win_close.rs"),
            include_str!("win_power.rs"),
            include_str!("win_apps.rs"),
            include_str!("windows_pc.rs"),
        ]
        .iter()
        .flat_map(|source| source.lines())
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
    }

    #[test]
    fn no_adapter_can_kill_a_process_or_suspend_with_the_blocking_call() {
        let code = adapter_code();
        // The needles are assembled so this test does not match itself.
        for forbidden in [
            ["Terminate", "Process"].concat(),
            ["Terminate", "Job", "Object"].concat(),
            ["Set", "Suspend", "State"].concat(),
            ["EWX", "_FORCE"].concat(),
        ] {
            assert!(
                !code.contains(&forbidden),
                "{forbidden} found in the adapters"
            );
        }
    }
}
