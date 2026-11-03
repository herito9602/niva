//! Ports (traits) implemented by host adapters. Filled in slices 2-5.
//!
//! Slice 2 (PR 2a) adds `LlmProvider`, `PcActions`, `Clock` and `UiPort`.
//! The ports are synchronous on purpose: `niva-core` has no async runtime,
//! and a late result is dropped by request id (see the reducer's stale-id
//! guard), so cancelling a call in flight is not needed at this layer. The
//! host runs blocking adapters off its event loop.

use std::time::SystemTime;

use crate::safety::{Approved, CloseApp, Power};
use crate::tools::{ToolCall, VolumeChange};

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
    /// Every running process. Resolution and protection happen in the core.
    fn running_processes(&self) -> Vec<ProcessInfo>;
    /// Risky: closes the approved executable (no force kill).
    fn close_app(&self, approved: Approved<CloseApp>) -> Result<CloseOutcome, PcError>;
    fn set_volume(&self, change: VolumeChange) -> Result<VolumeResult, PcError>;
    /// Risky: lock, sleep, restart or shutdown.
    fn power(&self, approved: Approved<Power>) -> Result<(), PcError>;
}

/// Wall clock. Time zone and local-time resolution arrive with reminders
/// (slice 4); the core only needs an instant for now.
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// Where UI events go. Generic over the event type so the core does not
/// depend on the host's view model (the host implements `UiPort<UiEvent>`).
pub trait UiPort<E>: Send + Sync {
    fn publish(&self, event: E);
}
