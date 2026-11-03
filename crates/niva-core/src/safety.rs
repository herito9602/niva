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

use crate::ports::{CloseOutcome, PcActions, PcError, ProcessInfo};
use crate::tools::{ParsedCall, PowerKind, SafeAction, ToolCall, ToolError, parse_call};

mod matcher;
#[cfg(test)]
mod tests;

pub use matcher::{MIN_QUERY_CHARS, is_protected, resolve_close_target};

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

/// How the user answered a confirmation. Typed text never confirms anything:
/// the reducer ignores `TextSubmitted` while a confirmation is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    /// A click on the confirmation buttons. **Host requirement:** only the
    /// native click handler (the `answer_confirmation` path that comes from
    /// the island) may produce this; no code reachable from the webview's
    /// debug commands, the model or typed text may construct it. Restart and
    /// shutdown are confirmable through this channel alone.
    Click,
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
    Power(Approved<Power>),
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
                RiskyKind::Close(CloseApp { query, exe })
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
        if request.click_only() && answer.channel != Channel::Click {
            return Err(ApprovalError::ClickRequired);
        }
        let id = request.confirmation_id;
        Ok(match request.kind {
            RiskyKind::Close(close) => Approval::Close(Approved::new(close, id)),
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
        match resolve_close_target(&approved.action.query, &pc.running_processes()) {
            Ok(now) if now.to_lowercase() == confirmed => {
                pc.close_app(approved).map_err(ExecError::Pc)
            }
            Err(ToolError::NoMatch) => Err(ExecError::NotRunning),
            _ => Err(ExecError::TargetChanged),
        }
    }

    pub fn execute_power(
        &self,
        approved: Approved<Power>,
        pc: &dyn PcActions,
    ) -> Result<(), PcError> {
        pc.power(approved)
    }
}
