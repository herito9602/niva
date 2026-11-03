//! In-memory fakes of the ports, for tests of the core and of the host.
//!
//! Compiled only for niva-core's own tests or with the `testing` cargo
//! feature (enabled by `niva-eval`), so they are not
//! part of the production build. They hold no I/O and add no dependency.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use super::{
    ChatReply, ChatRequest, Clock, CloseOutcome, LlmError, LlmProvider, PcActions, PcError,
    ProcessInfo, UiPort, VolumeResult,
};
use crate::safety::{Approved, CloseApp, Power};
use crate::tools::{PowerKind, VolumeChange};

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// LLM that replays a script, one entry per call, and records the requests.
#[derive(Default)]
pub struct FakeLlm {
    script: Mutex<VecDeque<Result<ChatReply, LlmError>>>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl FakeLlm {
    pub fn new(script: Vec<Result<ChatReply, LlmError>>) -> Self {
        FakeLlm {
            script: Mutex::new(script.into()),
            requests: Mutex::default(),
        }
    }

    pub fn requests(&self) -> Vec<ChatRequest> {
        lock(&self.requests).clone()
    }
}

impl LlmProvider for FakeLlm {
    fn chat(&self, request: &ChatRequest) -> Result<ChatReply, LlmError> {
        lock(&self.requests).push(request.clone());
        lock(&self.script)
            .pop_front()
            .unwrap_or_else(|| Err(LlmError::Failed("fake script exhausted".into())))
    }
}

/// What the fake PC was asked to do, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PcCall {
    OpenApp(String),
    CloseApp { exe: String },
    SetVolume(VolumeChange),
    Power(PowerKind),
}

/// PC with a fixed process list. Actions succeed unless configured to fail
/// and return realistic results (no dry-run placeholders).
pub struct FakePc {
    processes: Mutex<Vec<ProcessInfo>>,
    calls: Mutex<Vec<PcCall>>,
    open_result: Mutex<Result<(), PcError>>,
    close_outcome: Mutex<CloseOutcome>,
    volume: Mutex<VolumeResult>,
}

impl FakePc {
    pub fn with_processes(exe_names: &[&str]) -> Self {
        let processes = exe_names
            .iter()
            .map(|name| ProcessInfo {
                exe_name: (*name).into(),
                in_user_session: true,
            })
            .collect();
        FakePc {
            processes: Mutex::new(processes),
            calls: Mutex::default(),
            open_result: Mutex::new(Ok(())),
            close_outcome: Mutex::new(CloseOutcome::Closed),
            volume: Mutex::new(VolumeResult {
                level: 60,
                muted: false,
                clamped: false,
            }),
        }
    }

    pub fn set_processes(&self, processes: Vec<ProcessInfo>) {
        *lock(&self.processes) = processes;
    }

    pub fn set_open_result(&self, result: Result<(), PcError>) {
        *lock(&self.open_result) = result;
    }

    pub fn set_close_outcome(&self, outcome: CloseOutcome) {
        *lock(&self.close_outcome) = outcome;
    }

    pub fn calls(&self) -> Vec<PcCall> {
        lock(&self.calls).clone()
    }
}

impl PcActions for FakePc {
    fn open_app(&self, name: &str) -> Result<(), PcError> {
        lock(&self.calls).push(PcCall::OpenApp(name.into()));
        lock(&self.open_result).clone()
    }

    fn running_processes(&self) -> Vec<ProcessInfo> {
        lock(&self.processes).clone()
    }

    fn close_app(&self, approved: Approved<CloseApp>) -> Result<CloseOutcome, PcError> {
        lock(&self.calls).push(PcCall::CloseApp {
            exe: approved.action().exe().into(),
        });
        Ok(*lock(&self.close_outcome))
    }

    fn set_volume(&self, change: VolumeChange) -> Result<VolumeResult, PcError> {
        lock(&self.calls).push(PcCall::SetVolume(change));
        let mut volume = lock(&self.volume);
        match change {
            VolumeChange::Set(level) => volume.level = level,
            VolumeChange::Delta(delta) => {
                let target = i16::from(volume.level) + i16::from(delta);
                volume.clamped = !(0..=100).contains(&target);
                volume.level = target.clamp(0, 100) as u8;
            }
            VolumeChange::Mute(muted) => volume.muted = muted,
        }
        Ok(*volume)
    }

    fn power(&self, approved: Approved<Power>) -> Result<(), PcError> {
        lock(&self.calls).push(PcCall::Power(approved.action().kind()));
        Ok(())
    }
}

/// Clock that only moves when told to.
pub struct FakeClock {
    now: Mutex<SystemTime>,
}

impl FakeClock {
    pub fn at(now: SystemTime) -> Self {
        FakeClock {
            now: Mutex::new(now),
        }
    }

    pub fn advance(&self, by: Duration) {
        *lock(&self.now) += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        *lock(&self.now)
    }
}

/// UI that records what was published.
pub struct FakeUi<E> {
    events: Mutex<Vec<E>>,
}

impl<E> Default for FakeUi<E> {
    fn default() -> Self {
        FakeUi {
            events: Mutex::new(Vec::new()),
        }
    }
}

impl<E: Clone> FakeUi<E> {
    pub fn events(&self) -> Vec<E> {
        lock(&self.events).clone()
    }
}

impl<E: Send> UiPort<E> for FakeUi<E> {
    fn publish(&self, event: E) {
        lock(&self.events).push(event);
    }
}
