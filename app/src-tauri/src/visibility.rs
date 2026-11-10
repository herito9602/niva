//! Show/hide bookkeeping of the island window.
//!
//! Hiding is delayed so the frontend can play the slide-up animation. A
//! generation counter makes sure a delayed hide never hides a window that was
//! shown again in the meantime.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// How long the host waits before hiding the window after `niva://island
/// {visible:false}`: the exit animation (`NOTCH_OUT_MS` in
/// `src/notch/motion.ts`) plays first, plus a little slack. The contract tests
/// in `island_contract_tests.rs` pin `OUT <= HIDE <= OUT + 100`.
pub const HIDE_DELAY_MS: u64 = 240;

#[derive(Clone, Default)]
pub struct HideGuard {
    generation: Arc<AtomicU64>,
}

impl HideGuard {
    /// Call on every show: cancels any pending delayed hide.
    pub fn show(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Call on every hide: returns the ticket the delayed hide must present.
    pub fn hide(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Whether the delayed hide holding `ticket` may still hide the window.
    pub fn should_hide(&self, ticket: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == ticket
    }
}

/// What to do with the global shortcut registration result.
#[derive(Debug, PartialEq, Eq)]
pub enum ShortcutStatus {
    Active,
    /// Registration failed (for example the combination is taken by another
    /// app): keep running, the tray toggle still works.
    Unavailable(String),
}

pub fn shortcut_status<E: std::fmt::Display>(result: Result<(), E>) -> ShortcutStatus {
    match result {
        Ok(()) => ShortcutStatus::Active,
        Err(err) => ShortcutStatus::Unavailable(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hide_with_no_later_show_goes_through() {
        let g = HideGuard::default();
        g.show();
        let ticket = g.hide();
        assert!(g.should_hide(ticket));
    }

    #[test]
    fn a_show_during_the_delay_cancels_the_pending_hide() {
        let g = HideGuard::default();
        g.show();
        let ticket = g.hide();
        g.show();
        assert!(!g.should_hide(ticket));
    }

    #[test]
    fn only_the_latest_of_two_hides_goes_through() {
        let g = HideGuard::default();
        let first = g.hide();
        let second = g.hide();
        assert!(!g.should_hide(first));
        assert!(g.should_hide(second));
    }

    #[test]
    fn a_hide_after_a_cancelled_one_works_again() {
        let g = HideGuard::default();
        let cancelled = g.hide();
        g.show();
        let again = g.hide();
        assert!(!g.should_hide(cancelled));
        assert!(g.should_hide(again));
    }

    #[test]
    fn clones_share_the_same_counter() {
        let g = HideGuard::default();
        let ticket = g.hide();
        g.clone().show();
        assert!(!g.should_hide(ticket));
    }

    #[test]
    fn a_failed_shortcut_registration_degrades_instead_of_failing() {
        assert_eq!(
            shortcut_status(Ok::<(), String>(())),
            ShortcutStatus::Active
        );
        assert_eq!(
            shortcut_status(Err::<(), _>("already registered")),
            ShortcutStatus::Unavailable("already registered".into())
        );
    }
}
