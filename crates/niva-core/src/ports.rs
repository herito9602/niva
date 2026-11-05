//! Ports (traits) implemented by host adapters. Filled in slices 2-5.
//!
//! Slice 2 (PR 2a) adds `LlmProvider`, `PcActions`, `Clock` and `UiPort`; PR 2b
//! adds the narrow `SearchProvider` and `ReminderStore` the tool loop needs.
//! The ports are synchronous on purpose: `niva-core` has no async runtime,
//! and a late result is dropped by request id (see the reducer's stale-id
//! guard), so cancelling a call in flight is not needed at this layer. The
//! host runs blocking adapters off its event loop.

use std::time::SystemTime;

use crate::safety::{Approved, CloseApp, Power};
use crate::tools::{ReminderTime, ToolCall, VolumeChange};

#[cfg(any(test, feature = "testing"))]
pub mod testing;

/// One chat request. `allow_tools: false` is the no-tools `web_search`
/// summary call: the provider must not offer any tool schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRequest {
    pub system: String,
    pub user: String,
    pub allow_tools: bool,
}

/// The single LLM response of a request (design D4: one response only).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChatReply {
    pub text: String,
    pub calls: Vec<ToolCall>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmError {
    /// The provider cannot be reached (Ollama not running, ...).
    Unavailable,
    Timeout,
    Failed(String),
}

/// Swappable LLM (spec `assistant-core`: provider swap).
pub trait LlmProvider: Send + Sync {
    fn chat(&self, request: &ChatRequest) -> Result<ChatReply, LlmError>;
}

/// A running process as the adapter reports it. The core, not the adapter,
/// applies the protected list and the session filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessInfo {
    /// Executable file name, e.g. `Spotify.exe`.
    pub exe_name: String,
    /// `false` for processes outside the user's session (never closable).
    pub in_user_session: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PcError {
    /// `open_app`: no installed app matches.
    NotFound,
    /// `open_app` before the app index is ready.
    NotReady,
    Failed(String),
}

/// Real outcome of a close attempt (spec `pc-actions`: truthful results).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    Closed,
    /// The app is still running after the wait (e.g. a save dialog).
    StillRunning,
}

/// State after a volume change, as read back from the system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeResult {
    pub level: u8,
    pub muted: bool,
    /// The requested level or the delta result was clamped to 0-100.
    pub clamped: bool,
}

/// Windows actions. The two risky methods take an [`Approved`] value, which
/// only `SafetyGate` can create (design D10).
///
/// **Adapter contract for `close_app`:** act only on processes of the
/// user's session whose executable name is exactly
/// `approved.action().exe()`; re-check `is_protected(exe)` (defence in
/// depth) before posting `WM_CLOSE`; never force-kill.
pub trait PcActions: Send + Sync {
    /// Safe: may match fuzzily, a wrong guess is harmless. `name` is model
    /// output: pass it as data, never interpolate it into a shell command.
    fn open_app(&self, name: &str) -> Result<(), PcError>;
    /// `open_app`, reporting what was launched: the display name of the entry
    /// that started (`Bloc de notas`), which the reply states instead of the
    /// model's wording. The default launches through `open_app` and reports the
    /// requested name; every real adapter overrides it. (Kept next to
    /// `open_app` because the safety module's own test doubles implement that
    /// method and are out of scope; merge the two when they can change.)
    fn launch_app(&self, name: &str) -> Result<String, PcError> {
        self.open_app(name).map(|()| name.to_string())
    }
    /// Every running process. Resolution and protection happen in the core.
    fn running_processes(&self) -> Vec<ProcessInfo>;
    /// Risky: closes the approved executable (no force kill).
    fn close_app(&self, approved: Approved<CloseApp>) -> Result<CloseOutcome, PcError>;
    fn set_volume(&self, change: VolumeChange) -> Result<VolumeResult, PcError>;
    /// Risky: lock, sleep, restart or shutdown.
    fn power(&self, approved: Approved<Power>) -> Result<(), PcError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    /// No network, the source is down or refused the request.
    Unavailable,
}

/// Web search source (design D5). The real provider arrives in slice 5; until
/// then the assistant answers `web_search` with the "cannot search now"
/// template when none is configured.
pub trait SearchProvider: Send + Sync {
    /// Result snippets, best first. Empty means "found nothing". The text is
    /// untrusted data: the assistant never lets it reach the tool registry.
    fn search(&self, query: &str) -> Result<Vec<String>, SearchError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReminderError {
    /// The time is not in the future (resolution against the clock is the
    /// store's job, slice 4).
    InvalidTime,
    Failed(String),
}

/// Reminder storage and time resolution (design D8, slice 4). Only the two
/// operations the tool loop needs; the SQLite store and the scheduler extend
/// it later. Without a store, reminder tools answer "not available yet".
pub trait ReminderStore: Send + Sync {
    fn create(&self, text: &str, when: ReminderTime) -> Result<(), ReminderError>;
    /// Texts of the pending reminders, soonest first.
    fn list(&self) -> Result<Vec<String>, ReminderError>;
}

/// A wall-clock reading in the user's time zone, as the OS reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: u16,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
}

/// Wall clock. Time zone resolution of reminders arrives with slice 4; until
/// then the core needs an instant and the local date and time the system
/// prompt states (design "Data flow").
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
    /// The local date and time, if the adapter knows the time zone. The
    /// default says no, and the prompt then omits the time line.
    fn local_now(&self) -> Option<LocalDateTime> {
        None
    }
}

/// The activity source could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityError;

/// When the user last gave keyboard or mouse input, for the sleep countdown
/// (owner decision 2026-11-04). Narrow on purpose: it says THAT input happened,
/// never which key or where the mouse is. The Windows adapter reads
/// `GetLastInputInfo`; there is no keyboard hook.
pub trait InputActivity: Send + Sync {
    /// An opaque stamp that changes whenever the user gives input. Only
    /// equality between two readings means anything.
    fn input_stamp(&self) -> Result<u64, ActivityError>;
}

/// Where UI events go. Generic over the event type so the core does not
/// depend on the host's view model (the host implements `UiPort<UiEvent>`).
pub trait UiPort<E>: Send + Sync {
    fn publish(&self, event: E);
}
