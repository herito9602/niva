//! Input watch for the sleep countdown (owner decision 2026-11-04).
//!
//! The host polls the time of the user's last keyboard or mouse input (the
//! [`InputActivity`] port; on Windows `GetLastInputInfo`, never a hook and never
//! which key). This module is the pure decision on top of that reading:
//!
//! - a **baseline** is taken when the countdown starts, so the input that
//!   accepted the confirmation (a click, a key) cannot cancel it;
//! - during the **grace period** the baseline follows the stamp, so the hand
//!   still moving after an accept click (jitter) cannot cancel it either;
//! - afterwards **any change** of the stamp is input;
//! - a source that cannot be read is [`Watch::Unreadable`], never "quiet": the
//!   caller must then fail closed (no suspend).
//!
//! The watch has no clock: the caller says how long it has been running.

use std::time::Duration;

use crate::ports::{ActivityError, InputActivity};

/// Input is ignored for this long after the countdown starts. 1 s covers the
/// hand lifting off after an accept click (well under 0.5 s), the default
/// double-click interval (0.5 s) and the default keyboard repeat delay (0.5 s,
/// 1 s at most), and costs 10% of the 10 s the user has to react.
pub const GRACE: Duration = Duration::from_secs(1);

/// How often the host reads the source while a countdown runs: fast enough to
/// feel immediate, about 100 reads per countdown, none at rest.
pub const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// What one reading says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Watch {
    /// No input since the baseline (or still inside the grace period).
    Quiet,
    /// The user gave input: cancel the countdown.
    Input,
    /// The source could not be read: fail closed.
    Unreadable,
}

/// The baseline of one countdown.
#[derive(Debug)]
pub struct InputWatch {
    baseline: u64,
}

impl InputWatch {
    /// Takes the baseline. Fails if the source cannot be read: no countdown
    /// without a working watch.
    pub fn start(source: &dyn InputActivity) -> Result<InputWatch, ActivityError> {
        Ok(InputWatch {
            baseline: source.input_stamp()?,
        })
    }

    /// One reading, `elapsed` after `start`.
    pub fn poll(&mut self, elapsed: Duration, source: &dyn InputActivity) -> Watch {
        let Ok(stamp) = source.input_stamp() else {
            return Watch::Unreadable;
        };
        if elapsed < GRACE {
            self.baseline = stamp;
            return Watch::Quiet;
        }
        if stamp == self.baseline {
            Watch::Quiet
        } else {
            Watch::Input
        }
    }
}

#[cfg(test)]
mod tests;
