//! Safety gate (design D10, "Safety Gate"; spec `action-safety`).
//!
//! Risky actions are enforced in the type system, not in the prompt:
//!
//! - `PcActions::close_app` and `PcActions::power` accept only
//!   [`Approved<CloseApp>`] / [`Approved<Power>`].
//! - `Approved` has private fields and a `pub(crate)` constructor, and
//!   [`RiskyAction`] is sealed, so code outside this crate cannot build one,
//!   not even by copying the struct literal. The only producer is
//!   [`SafetyGate::approve`].
//! - A sleep never becomes an `Approved<Power>`: `approve` returns it as
//!   [`Approval::Sleep`] holding a [`HeldPower`], which nothing can turn back
//!   into an `Approved`. The only route from a held sleep to the PC is
//!   [`SafetyGate::release_held`] (the countdown's end); `execute_power`
//!   also refuses a sleep as a second barrier.
//! - [`SafetyGate::approve`] consumes a [`RiskyRequest`] (which only
//!   [`SafetyGate::validate`] makes, once per risky call, with its own
//!   [`ConfirmationId`]), checks the confirmation id and the channel, and
//!   requires an explicit accepted answer. A request is not `Clone`, so one
//!   confirmation yields at most one `Approved`.
//!
//! The compile-fail doc tests below pin the guarantee:
//!
//! ```compile_fail,E0451
//! // Struct literal: the fields are private.
//! use niva_core::safety::{Approved, CloseApp};
//! fn forge(action: CloseApp) -> Approved<CloseApp> {
//!     Approved { action, confirmation_id: 1 }
//! }
//! ```
//!
//! ```compile_fail,E0624
//! // The constructor is crate-private.
//! use niva_core::safety::{Approved, Power};
//! fn forge(action: Power) -> Approved<Power> {
//!     Approved::new(action, 1)
//! }
//! ```
//!
//! ```compile_fail,E0277
//! // The risky-action trait is sealed: a foreign type cannot implement it,
//! // so no foreign `Approved<T>` can exist.
//! struct F;
//! impl niva_core::safety::RiskyAction for F {}
//! ```
//!
//! ```compile_fail,E0382
//! // One confirmation, one approval: `approve` consumes the request.
//! use niva_core::safety::{ConfirmationAnswer, RiskyRequest, SafetyGate};
//! fn twice(gate: &SafetyGate, request: RiskyRequest, answer: ConfirmationAnswer) {
//!     let _ = gate.approve(request, answer);
//!     let _ = gate.approve(request, answer);
//! }
//! ```
//!
//! ```compile_fail,E0599
//! // ...and a request cannot be cloned to dodge that.
//! use niva_core::safety::RiskyRequest;
//! fn dup(request: RiskyRequest) -> (RiskyRequest, RiskyRequest) {
//!     (request.clone(), request)
//! }
//! ```
//!
//! ```compile_fail,E0308
//! // A risky port method cannot be called with a raw, unapproved action.
//! use niva_core::ports::PcActions;
//! use niva_core::safety::Power;
//! fn run(pc: &dyn PcActions, power: Power) {
//!     let _ = pc.power(power);
//! }
//! ```
//!
//! A held sleep (the approval waiting for its countdown) is one more way to
//! hold an approval, so it is pinned the same way:
//!
//! ```compile_fail,E0308
//! // An approved sleep is a `HeldPower`, not an `Approved<Power>`: it cannot
//! // be handed to the port, so it cannot reach the PC without the countdown.
//! use niva_core::ports::PcActions;
//! use niva_core::safety::{Approval, ConfirmationAnswer, RiskyRequest, SafetyGate};
//! fn bypass(
//!     gate: &SafetyGate,
//!     pc: &dyn PcActions,
//!     request: RiskyRequest,
//!     answer: ConfirmationAnswer,
//! ) {
//!     if let Ok(Approval::Sleep(held)) = gate.approve(request, answer) {
//!         let _ = pc.power(held);
//!     }
//! }
//! ```
//!
//! ```compile_fail,E0451
//! // The held approval cannot be built by hand: its field is private.
//! use niva_core::safety::{Approved, HeldPower, Power};
//! fn forge(approved: Approved<Power>) -> HeldPower {
//!     HeldPower { approved }
//! }
//! ```
//!
//! ```compile_fail,E0616
//! // ...and the approval cannot be taken back out of it.
//! use niva_core::safety::{Approved, HeldPower, Power};
//! fn steal(held: HeldPower) -> Approved<Power> {
//!     held.approved
//! }
//! ```
//!
//! ```compile_fail,E0382
//! // One countdown, one release: `release_held` consumes the held approval.
//! use niva_core::ports::PcActions;
//! use niva_core::safety::{HeldPower, SafetyGate};
//! fn twice(gate: &SafetyGate, pc: &dyn PcActions, held: HeldPower) {
//!     let _ = gate.release_held(held, pc);
//!     let _ = gate.release_held(held, pc);
//! }
//! ```
//!
//! ```compile_fail,E0599
//! // ...and a held approval cannot be cloned to dodge that.
//! use niva_core::safety::HeldPower;
//! fn dup(held: HeldPower) -> (HeldPower, HeldPower) {
//!     (held.clone(), held)
//! }
//! ```
//!
//! ```compile_fail,E0599
//! // Only `SafetyGate::approve` makes one: there is no public constructor.
//! use niva_core::safety::{Approved, HeldPower, Power};
//! fn forge(approved: Approved<Power>) -> HeldPower {
//!     HeldPower::new(approved)
//! }
//! ```

use crate::ports::{CloseOutcome, PcActions, PcError, ProcessInfo};
use crate::tools::{ParsedCall, PowerKind, SafeAction, ToolCall, ToolError, parse_call};

mod matcher;
#[cfg(test)]
mod tests;

pub use matcher::{MIN_QUERY_CHARS, is_protected, resolve_close_target, resolve_pinned};

mod sealed {
    pub trait Sealed {}
}

/// An action that needs an [`Approved`] value to run. Sealed.
pub trait RiskyAction: sealed::Sealed {}

/// A resolved `close_app` target: the executable the prompt named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseApp {
    query: String,
    exe: String,
    /// The user picked `exe` by its whole file name (the answer to "which
    /// one?"): execution re-resolves it the same way.
    pinned: bool,
}

impl CloseApp {
    /// The (trimmed) text the model gave, used to re-resolve at execution.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// The executable name shown in the confirmation prompt, e.g. `Spotify.exe`.
    pub fn exe(&self) -> &str {
        &self.exe
    }
}

/// A `power_action`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Power {
    kind: PowerKind,
}

impl Power {
    pub fn kind(&self) -> PowerKind {
        self.kind
    }
}

impl sealed::Sealed for CloseApp {}
impl sealed::Sealed for Power {}
impl RiskyAction for CloseApp {}
impl RiskyAction for Power {}

/// Identity of one confirmation: the request it belongs to plus the index of
/// the risky call inside that request's LLM response. The gate mints it, so
/// two risky calls of one response never share an id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationId {
    pub request: u64,
    pub call: u32,
}

/// Proof that the user confirmed `action` (confirmation `confirmation_id`).
/// Not `Clone`: one confirmation authorizes one execution.
#[derive(Debug, PartialEq, Eq)]
pub struct Approved<T: RiskyAction> {
    action: T,
    confirmation_id: ConfirmationId,
}

impl<T: RiskyAction> Approved<T> {
    pub(crate) fn new(action: T, confirmation_id: ConfirmationId) -> Self {
        Approved {
            action,
            confirmation_id,
        }
    }

    pub fn action(&self) -> &T {
        &self.action
    }

    pub fn confirmation_id(&self) -> ConfirmationId {
        self.confirmation_id
    }
}

/// An approved sleep waiting for its countdown. `SafetyGate::approve` returns
/// it instead of an `Approved<Power>`, so a sleep approval can never be handed
/// to the PC directly. It owns the only copy of the approval: not `Clone`, no accessor hands the approval out, and
/// [`SafetyGate::release_held`] consumes it, so one confirmation can still
/// execute at most once. Cancelling a countdown is dropping this value.
#[derive(Debug, PartialEq, Eq)]
pub struct HeldPower {
    approved: Approved<Power>,
}

impl HeldPower {
    pub fn confirmation_id(&self) -> ConfirmationId {
        self.approved.confirmation_id
    }
}

/// How the user answered a confirmation. Typed text never confirms anything:
/// the reducer ignores `TextSubmitted` while a confirmation is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// A physical input of the user: the Ctrl+Shift+Enter chord (host
    /// `ConfirmKeys`, which accepts nothing but the id of the confirmation on
    /// screen, one answer per confirmation, and nothing in the first half
    /// second after it was armed) or a click on the confirmation buttons
    /// (host `ConfirmGate`, until they are removed). No code reachable from
    /// the model, typed text or speech may construct it, and no webview
    /// command other than the island's `answer_confirmation` may.
    /// Debug builds add one exception: the debug window's Confirm button also
    /// becomes a physical answer, which is safe only because every
    /// `PcActions` a debug build runs is dry-run (design "Safety Gate").
    /// Restart and shutdown are confirmable through this channel alone.
    Physical,
    Voice,
}

/// The user's answer to one confirmation, as the host reports it. (Not the
/// reducer's `state_machine::Answer`, which classifies a spoken utterance;
/// the host maps that verdict to `accepted`.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConfirmationAnswer {
    pub confirmation_id: ConfirmationId,
    pub channel: Channel,
    pub accepted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalError {
    /// The answer is for another confirmation.
    WrongConfirmation,
    /// Rejected (the caller maps unclear voice answers to "not accepted").
    NotConfirmed,
    /// Restart and shutdown cannot be confirmed by voice.
    ClickRequired,
}

#[derive(Debug, PartialEq, Eq)]
enum RiskyKind {
    Close(CloseApp),
    Power(Power),
}

/// A validated risky call waiting for the user's answer. It can only be
/// turned into an [`Approval`] by [`SafetyGate::approve`], which consumes it.
/// Not `Clone`: one confirmation can mint at most one `Approved`.
#[derive(Debug, PartialEq, Eq)]
pub struct RiskyRequest {
    kind: RiskyKind,
    confirmation_id: ConfirmationId,
}

impl RiskyRequest {
    pub fn confirmation_id(&self) -> ConfirmationId {
        self.confirmation_id
    }

    /// Voice answers are not accepted (power restart and shutdown).
    pub fn click_only(&self) -> bool {
        match &self.kind {
            RiskyKind::Close(_) => false,
            RiskyKind::Power(power) => power.kind.click_only(),
        }
    }

    /// What the prompt names: the exact executable, or the power action.
    pub fn target(&self) -> &str {
        match &self.kind {
            RiskyKind::Close(close) => &close.exe,
            RiskyKind::Power(power) => power.kind.as_str(),
        }
    }
}

/// The result of a successful approval.
#[derive(Debug, PartialEq, Eq)]
pub enum Approval {
    Close(Approved<CloseApp>),
    /// Lock, restart or shutdown. Never a sleep.
    Power(Approved<Power>),
    /// A sleep: held for its countdown, released only by
    /// [`SafetyGate::release_held`].
    Sleep(HeldPower),
}

/// A call after validation and target resolution.
#[derive(Debug, PartialEq, Eq)]
pub enum Validated {
    Safe(SafeAction),
    Risky(RiskyRequest),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecError {
    /// The target resolves to a different executable set than the one the
    /// user confirmed. Nothing was closed.
    TargetChanged,
    /// The confirmed app is no longer running.
    NotRunning,
    Pc(PcError),
}

/// Validates calls and issues [`Approved`] values.
#[derive(Debug, Clone, Copy, Default)]
pub struct SafetyGate;

impl SafetyGate {
    pub fn new() -> Self {
        SafetyGate
    }

    /// Schema validation plus target resolution. Never executes anything.
    /// A risky request gets its own [`ConfirmationId`] from `request` (the
    /// reducer's request id) and `call_index` (the call's position in the
    /// LLM response), so no two risky calls share one. The gate is stateless:
    /// the caller must use a fresh `request` per LLM response (retries
    /// included) and each call's own index, or an old answer could match.
    pub fn validate(
        &self,
        call: &ToolCall,
        processes: &[ProcessInfo],
        request: u64,
        call_index: u32,
    ) -> Result<Validated, ToolError> {
        let kind = match parse_call(call)? {
            ParsedCall::Safe(action) => return Ok(Validated::Safe(action)),
            ParsedCall::Close { query } => {
                let exe = resolve_close_target(&query, processes)?;
                RiskyKind::Close(CloseApp {
                    query,
                    exe,
                    pinned: false,
                })
            }
            ParsedCall::Power(kind) => RiskyKind::Power(Power { kind }),
        };
        Ok(Validated::Risky(RiskyRequest {
            kind,
            confirmation_id: ConfirmationId {
                request,
                call: call_index,
            },
        }))
    }

    /// The user's follow-up answer to "which one?": `exe` is the whole file
    /// name of one of the candidates. It must still be a running, unprotected
    /// process of the user's session, and the confirmation binds to exactly
    /// that executable. The model's own `close_app` call must not come here:
    /// `validate` keeps an exact name ambiguous.
    pub fn validate_pinned(
        &self,
        exe: &str,
        processes: &[ProcessInfo],
        request: u64,
        call_index: u32,
    ) -> Result<RiskyRequest, ToolError> {
        let exe = resolve_pinned(exe, processes)?;
        Ok(RiskyRequest {
            kind: RiskyKind::Close(CloseApp {
                query: exe.clone(),
                exe,
                pinned: true,
            }),
            confirmation_id: ConfirmationId {
                request,
                call: call_index,
            },
        })
    }

    /// Turns a risky request plus an explicit answer into an [`Approval`].
    pub fn approve(
        &self,
        request: RiskyRequest,
        answer: ConfirmationAnswer,
    ) -> Result<Approval, ApprovalError> {
        if answer.confirmation_id != request.confirmation_id {
            return Err(ApprovalError::WrongConfirmation);
        }
        if !answer.accepted {
            return Err(ApprovalError::NotConfirmed);
        }
        if request.click_only() && answer.channel != Channel::Physical {
            return Err(ApprovalError::ClickRequired);
        }
        let id = request.confirmation_id;
        Ok(match request.kind {
            RiskyKind::Close(close) => Approval::Close(Approved::new(close, id)),
            RiskyKind::Power(power) if power.kind.needs_countdown() => Approval::Sleep(HeldPower {
                approved: Approved::new(power, id),
            }),
            RiskyKind::Power(power) => Approval::Power(Approved::new(power, id)),
        })
    }

    /// Closes the approved app. Re-resolves the target first and aborts if
    /// the matched executable changed (design: "Binding").
    pub fn execute_close(
        &self,
        approved: Approved<CloseApp>,
        pc: &dyn PcActions,
    ) -> Result<CloseOutcome, ExecError> {
        let confirmed = approved.action.exe.to_lowercase();
        // Cannot happen (`validate` never resolves a protected process);
        // asserted so a future bug shows up in tests.
        debug_assert!(!is_protected(&confirmed));
        // An unreadable snapshot is an error, never "not running".
        let processes = pc.running_processes().map_err(ExecError::Pc)?;
        let now = if approved.action.pinned {
            resolve_pinned(&approved.action.query, &processes)
        } else {
            resolve_close_target(&approved.action.query, &processes)
        };
        match now {
            Ok(now) if now.to_lowercase() == confirmed => {
                pc.close_app(approved).map_err(ExecError::Pc)
            }
            Err(ToolError::NoMatch) => Err(ExecError::NotRunning),
            _ => Err(ExecError::TargetChanged),
        }
    }

    /// Runs an approved power action at once. `approve` never gives a sleep
    /// here (it gives [`Approval::Sleep`]); if one got in anyway it is refused
    /// and the PC is not touched.
    pub fn execute_power(
        &self,
        approved: Approved<Power>,
        pc: &dyn PcActions,
    ) -> Result<(), PcError> {
        if approved.action.kind.needs_countdown() {
            return Err(PcError::Failed(
                "sleep must wait for its countdown".to_string(),
            ));
        }
        pc.power(approved)
    }

    /// The only way an approved sleep reaches the PC. Call it when the
    /// countdown ended with nobody cancelling; it consumes the held approval.
    pub fn release_held(&self, held: HeldPower, pc: &dyn PcActions) -> Result<(), PcError> {
        pc.power(held.approved)
    }
}
