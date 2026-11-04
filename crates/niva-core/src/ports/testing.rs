//! In-memory fakes of the ports, for tests of the core and of the host.
//!
//! Compiled only for niva-core's own tests or with the `testing` cargo
//! feature (enabled by `niva-eval`), so they are not
//! part of the production build. They hold no I/O and add no dependency.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use super::{
    ActivityError, ChatReply, ChatRequest, Clock, CloseOutcome, InputActivity, LlmError,
    LlmProvider, LocalDateTime, PcActions, PcError, ProcessInfo, ReminderError, ReminderStore,
    SearchError, SearchProvider, UiPort, VolumeResult,
};
use crate::safety::{Approved, CloseApp, Power};
use crate::tools::{PowerKind, ReminderTime, VolumeChange};

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

/// Search source with a fixed answer; records the queries.
pub struct FakeSearch {
    answer: Mutex<Result<Vec<String>, SearchError>>,
    queries: Mutex<Vec<String>>,
}

impl FakeSearch {
    pub fn new(answer: Result<Vec<String>, SearchError>) -> Self {
        FakeSearch {
            answer: Mutex::new(answer),
            queries: Mutex::default(),
        }
    }

    pub fn queries(&self) -> Vec<String> {
        lock(&self.queries).clone()
    }
}

impl SearchProvider for FakeSearch {
    fn search(&self, query: &str) -> Result<Vec<String>, SearchError> {
        lock(&self.queries).push(query.into());
        lock(&self.answer).clone()
    }
}

/// Reminder store that keeps what it was given in memory.
#[derive(Default)]
pub struct FakeReminders {
    created: Mutex<Vec<(String, ReminderTime)>>,
    reject_time: Mutex<bool>,
}

impl FakeReminders {
    /// Makes `create` fail with `InvalidTime` (a time in the past).
    pub fn reject_time(&self, reject: bool) {
        *lock(&self.reject_time) = reject;
    }

    pub fn created(&self) -> Vec<(String, ReminderTime)> {
        lock(&self.created).clone()
    }
}

impl ReminderStore for FakeReminders {
    fn create(&self, text: &str, when: ReminderTime) -> Result<(), ReminderError> {
        if *lock(&self.reject_time) {
            return Err(ReminderError::InvalidTime);
        }
        lock(&self.created).push((text.into(), when));
        Ok(())
    }

    fn list(&self) -> Result<Vec<String>, ReminderError> {
        Ok(lock(&self.created).iter().map(|(t, _)| t.clone()).collect())
    }
}

/// Clock that only moves when told to.
pub struct FakeClock {
    now: Mutex<SystemTime>,
    local: Mutex<Option<LocalDateTime>>,
}

impl FakeClock {
    pub fn at(now: SystemTime) -> Self {
        FakeClock {
            now: Mutex::new(now),
            local: Mutex::new(None),
        }
    }

    /// Makes `local_now` answer (the default clock does not know the time zone).
    pub fn with_local(self, local: LocalDateTime) -> Self {
        *lock(&self.local) = Some(local);
        self
    }

    pub fn advance(&self, by: Duration) {
        *lock(&self.now) += by;
    }
}

impl Clock for FakeClock {
    fn now(&self) -> SystemTime {
        *lock(&self.now)
    }

    fn local_now(&self) -> Option<LocalDateTime> {
        *lock(&self.local)
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

/// Activity source whose stamp the test moves by hand.
pub struct FakeActivity {
    stamp: Mutex<u64>,
    unreadable: Mutex<bool>,
    reads: Mutex<usize>,
}

impl FakeActivity {
    pub fn new(stamp: u64) -> Self {
        FakeActivity {
            stamp: Mutex::new(stamp),
            unreadable: Mutex::new(false),
            reads: Mutex::new(0),
        }
    }

    /// The user gave input.
    pub fn touch(&self) {
        *lock(&self.stamp) += 1;
    }

    pub fn set_unreadable(&self, unreadable: bool) {
        *lock(&self.unreadable) = unreadable;
    }

    /// How many times the source was read.
    pub fn reads(&self) -> usize {
        *lock(&self.reads)
    }
}

impl InputActivity for FakeActivity {
    fn input_stamp(&self) -> Result<u64, ActivityError> {
        *lock(&self.reads) += 1;
        if *lock(&self.unreadable) {
            return Err(ActivityError);
        }
        Ok(*lock(&self.stamp))
    }
}
