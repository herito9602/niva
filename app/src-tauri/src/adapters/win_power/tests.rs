//! `apply` over a fake power system: which OS calls each action makes, in which
//! order, and what it reports when one fails or is not available.

use std::sync::Mutex;

use niva_core::ports::PcError;
use niva_core::tools::PowerKind;

use super::*;

/// What a call of the fake does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Call {
    Lock,
    DetectStandby,
    Standby,
    Privilege,
    Unprivilege,
    Exit { restart: bool },
}

struct FakePower {
    calls: Mutex<Vec<Call>>,
    standby: Option<bool>,
    fail: Option<Call>,
}

impl FakePower {
    fn new() -> FakePower {
        FakePower {
            calls: Mutex::default(),
            standby: Some(true),
            fail: None,
        }
    }

    fn standby(mut self, standby: Option<bool>) -> FakePower {
        self.standby = standby;
        self
    }

    fn failing(mut self, call: Call) -> FakePower {
        self.fail = Some(call);
        self
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn record(&self, call: Call) -> Result<(), String> {
        self.calls.lock().unwrap().push(call.clone());
        if self.fail.as_ref() == Some(&call) {
            Err(format!("{call:?} failed"))
        } else {
            Ok(())
        }
    }
}

impl PowerSystem for FakePower {
    fn lock_workstation(&self) -> Result<(), String> {
        self.record(Call::Lock)
    }

    fn modern_standby(&self) -> Option<bool> {
        self.calls.lock().unwrap().push(Call::DetectStandby);
        self.standby
    }

    fn start_modern_standby(&self) -> Result<(), String> {
        self.record(Call::Standby)
    }

    fn enable_shutdown_privilege(&self) -> Result<(), String> {
        self.record(Call::Privilege)
    }

    fn disable_shutdown_privilege(&self) -> Result<(), String> {
        self.record(Call::Unprivilege)
    }

    fn exit_windows(&self, restart: bool) -> Result<(), String> {
        self.record(Call::Exit { restart })
    }
}

// --- lock ---------------------------------------------------------------------

#[test]
fn lock_locks_the_workstation_and_nothing_else() {
    let system = FakePower::new();
    assert_eq!(apply(&system, PowerKind::Lock), Ok(()));
    assert_eq!(system.calls(), [Call::Lock]);
}

#[test]
fn a_lock_that_fails_is_reported() {
    let system = FakePower::new().failing(Call::Lock);
    assert!(matches!(
        apply(&system, PowerKind::Lock),
        Err(PcError::Failed(_))
    ));
}

// --- sleep --------------------------------------------------------------------

#[test]
fn sleep_on_a_modern_standby_pc_starts_standby_without_blocking() {
    let system = FakePower::new().standby(Some(true));
    assert_eq!(apply(&system, PowerKind::Sleep), Ok(()));
    assert_eq!(system.calls(), [Call::DetectStandby, Call::Standby]);
}

#[test]
fn sleep_where_modern_standby_is_absent_is_not_available_and_makes_no_call() {
    let system = FakePower::new().standby(Some(false));
    assert_eq!(apply(&system, PowerKind::Sleep), Err(PcError::NotSupported));
    assert_eq!(system.calls(), [Call::DetectStandby], "detection only");
}

#[test]
fn sleep_where_modern_standby_is_unknown_is_not_available_and_makes_no_call() {
    let system = FakePower::new().standby(None);
    assert_eq!(apply(&system, PowerKind::Sleep), Err(PcError::NotSupported));
    assert_eq!(system.calls(), [Call::DetectStandby]);
}

#[test]
fn a_standby_request_that_fails_is_reported() {
    let system = FakePower::new().failing(Call::Standby);
    assert!(matches!(
        apply(&system, PowerKind::Sleep),
        Err(PcError::Failed(_))
    ));
}

// --- restart and shutdown ------------------------------------------------------

#[test]
fn restart_and_shutdown_enable_the_privilege_first_and_then_exit_windows() {
    for (kind, restart) in [(PowerKind::Restart, true), (PowerKind::Shutdown, false)] {
        let system = FakePower::new();
        assert_eq!(apply(&system, kind), Ok(()), "{kind:?}");
        assert_eq!(
            system.calls(),
            [Call::Privilege, Call::Exit { restart }, Call::Unprivilege],
            "{kind:?}"
        );
    }
}

#[test]
fn without_the_privilege_nothing_is_attempted_and_the_failure_is_reported() {
    for kind in [PowerKind::Restart, PowerKind::Shutdown] {
        let system = FakePower::new().failing(Call::Privilege);
        let result = apply(&system, kind);
        assert!(
            matches!(result, Err(PcError::Failed(_))),
            "{kind:?}: {result:?}"
        );
        assert_eq!(system.calls(), [Call::Privilege], "{kind:?}: no exit call");
    }
}

#[test]
fn the_privilege_is_dropped_again_even_when_the_exit_call_fails() {
    for (kind, restart) in [(PowerKind::Restart, true), (PowerKind::Shutdown, false)] {
        let system = FakePower::new().failing(Call::Exit { restart });
        assert!(apply(&system, kind).is_err());
        assert_eq!(
            system.calls().last(),
            Some(&Call::Unprivilege),
            "{kind:?}: {:?}",
            system.calls()
        );
    }
}

#[test]
fn a_privilege_that_cannot_be_dropped_does_not_change_the_answer() {
    let system = FakePower::new().failing(Call::Unprivilege);
    assert_eq!(apply(&system, PowerKind::Restart), Ok(()));
}

#[test]
fn an_exit_call_that_fails_is_reported_not_swallowed() {
    for (kind, restart) in [(PowerKind::Restart, true), (PowerKind::Shutdown, false)] {
        let system = FakePower::new().failing(Call::Exit { restart });
        let result = apply(&system, kind);
        assert!(
            matches!(result, Err(PcError::Failed(_))),
            "{kind:?}: {result:?}"
        );
    }
}

#[test]
fn an_action_makes_only_its_own_calls() {
    let system = FakePower::new();
    apply(&system, PowerKind::Restart).unwrap();
    assert!(!system.calls().contains(&Call::Lock));
    assert!(!system.calls().contains(&Call::Standby));
    let system = FakePower::new();
    apply(&system, PowerKind::Sleep).unwrap();
    assert!(
        !system
            .calls()
            .iter()
            .any(|c| matches!(c, Call::Exit { .. } | Call::Privilege | Call::Lock))
    );
}

// --- what the Windows part may call ---------------------------------------------

/// The code lines of this module's source (comments dropped): the Windows part
/// is checked as text, because its calls cannot be run in a test.
fn code() -> String {
    include_str!("../win_power.rs")
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_windows_part_never_forces_an_exit() {
    let code = code();
    for forbidden in ["EWX_FORCE", "EWX_FORCEIFHUNG", "SHUTDOWN_FORCE"] {
        assert!(
            !code.contains(forbidden),
            "{forbidden} would skip the apps' objection"
        );
    }
}

#[test]
fn the_windows_part_never_calls_the_blocking_suspend() {
    // It hung a real PC (design "Sleep"): modern standby goes through the
    // monitor-power broadcast, which returns at once.
    let code = code();
    assert!(!code.contains("SetSuspendState"));
    assert!(!code.contains("SetSystemPowerState"));
    assert!(code.contains("SC_MONITORPOWER"));
}

#[test]
fn the_windows_part_reads_the_always_on_always_connected_flag() {
    let code = code();
    assert!(code.contains("SystemPowerCapabilities"));
    assert!(code.contains("AoAc"));
}
