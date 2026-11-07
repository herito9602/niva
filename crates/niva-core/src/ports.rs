//! Ports (traits) implemented by host adapters. Filled in slices 2-5.
//!
//! Slice 2 (PR 2a) adds `LlmProvider`, `PcActions`, `Clock` and `UiPort`; PR 2b
//! adds the narrow `SearchProvider` and `ReminderStore` the tool loop needs.
//! The ports are synchronous on purpose: `niva-core` has no async runtime,
//! and a late result is dropped by request id (see the reducer's stale-id
//! guard), so cancelling a call in flight is not needed at this layer. The
//! host runs blocking adapters off its event loop.

use std::time::SystemTime;

use crate::replies::Lang;
use crate::safety::{Approved, CloseApp, Power};
use crate::state_machine::ListenMode;
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
    /// `power`: this PC cannot do it yet (sleep without Modern Standby). The
    /// adapter made no OS call, and the reply says so instead of "that didn't
    /// work".
    NotSupported,
}

/// Real outcome of a close attempt (spec `pc-actions`: truthful results).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseOutcome {
    Closed,
    /// The app is still running after the wait (e.g. a save dialog).
    StillRunning,
    /// The windows are gone but the process keeps running (an app that stays in
    /// the background or in the tray).
    KeepsRunning,
    /// The app is running but shows no window that could be asked to close.
    /// Nothing was posted.
    NoWindow,
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
    /// Every running process. Resolution and protection happen in the core. A
    /// snapshot that cannot be read is an error: it must never look like
    /// "nothing is running".
    fn running_processes(&self) -> Result<Vec<ProcessInfo>, PcError>;
    /// Risky: closes the approved executable (no force kill).
    fn close_app(&self, approved: Approved<CloseApp>) -> Result<CloseOutcome, PcError>;
    fn set_volume(&self, change: VolumeChange) -> Result<VolumeResult, PcError>;
    /// Risky: lock, sleep, restart or shutdown.
    fn power(&self, approved: Approved<Power>) -> Result<(), PcError>;
}

/// A confirmation question as it was spoken: its language and its words. The
/// words can quote what the user asked, so `Debug` shows only their size.
#[derive(Clone, PartialEq, Eq)]
pub struct SpokenPrompt {
    pub lang: Lang,
    pub text: String,
}

impl std::fmt::Debug for SpokenPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "SpokenPrompt {{ lang: {:?}, text: <{} chars> }}",
            self.lang,
            self.text.chars().count()
        )
    }
}

/// The listening half of the voice sidecar (wake word, VAD and STT sit behind
/// it because they share the microphone stream). What the sidecar hears comes
/// back as events through the host's queue; these calls only send commands, so
/// they never block on audio. `false` means the command could not be sent
/// (no sidecar is running): the reducer's own timers then end the turn.
pub trait SpeechEngine: Send + Sync {
    fn listen(&self, id: u64, mode: ListenMode) -> bool;
    /// Like [`listen`](Self::listen), plus the confirmation question the host
    /// just spoke. A confirm answer is one short word that recognizers get
    /// wrong without context, so an engine that can use the question's
    /// language and text does (`None`: it detects the language). The default
    /// ignores the question.
    fn listen_with_prompt(&self, id: u64, mode: ListenMode, prompt: Option<&SpokenPrompt>) -> bool {
        let _ = prompt;
        self.listen(id, mode)
    }
    fn cancel_listen(&self, id: u64) -> bool;
    /// End the utterance now (the max-utterance cap).
    fn force_end_of_speech(&self, id: u64) -> bool;
}

/// The speaking half of the voice sidecar (TTS). `SpeakStarted`, `SpeakDone`
/// and `TtsFailed` come back as events.
pub trait Speaker: Send + Sync {
    /// Whether speech can be produced right now. While it cannot, the runner
    /// skips every `Speak` and reports it done at once.
    fn is_available(&self) -> bool;
    fn speak(&self, id: u64, text: &str, lang: Lang) -> bool;
    fn stop(&self) -> bool;
}

/// Short sounds played by the host itself, so they work when the sidecar is
/// down (design "Fullscreen policy": the chime is never the webview's or the
/// sidecar's).
pub trait AudioCue: Send + Sync {
    /// Plays the alert chime and calls `done` once, when it ended or could not
    /// play (no output device). Never blocks.
    fn chime(&self, done: Box<dyn FnOnce() + Send>);
    /// A soft cue for "listening started".
    fn listening_cue(&self);
    /// Cuts a playing cue short (its `done` still runs).
    fn stop(&self);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    /// No network, the source is down or refused the request.
    Unavailable,
    /// The provider has no API key: no request was made.
    NotConfigured,
    /// The source rejected the key (HTTP 401 or 403).
    KeyRejected,
    /// The key's quota or rate limit is used up.
    QuotaExceeded,
}

/// Web search source (design D5, Tavily). Without any provider the assistant
/// answers `web_search` with the "cannot search now" template; a provider
/// without a key answers [`SearchError::NotConfigured`], which has its own
/// truthful reply.
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
