//! Click-through of the island window.
//!
//! The island window (200x70) is larger than the notch (118x30; room for the
//! spring overshoot), so most of it is empty and transparent. It must never
//! block clicks to the apps underneath.
//!
//! The whole window ignores the mouse (`set_ignore_cursor_events(true)`, called
//! from Rust on prepare and on every show; the webview needs no permission for
//! it). The notch has no buttons and its UI reports no rectangles, so nothing
//! captures the mouse. The hit tester below is dead weight until the host
//! cleanup slice removes it.
//!
//! (Historical, from when the island had buttons: while it showed a confirmation
//! only the two buttons captured the mouse.
//! The cursor poller (which already knows the cursor in window-relative logical
//! pixels) feeds every sample, with the buttons' rectangles, to a [`HitTester`],
//! which toggles the flag only when the answer changes. Where the rectangles
//! come from is `confirm_buttons.rs` (the island reports them).)

use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::cursor::CursorPayload;

/// A rectangle in logical pixels, relative to the window's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// Left and top edges are inside, right and bottom edges are outside.
    fn contains(&self, p: CursorPayload) -> bool {
        p.x >= self.x && p.x < self.x + self.width && p.y >= self.y && p.y < self.y + self.height
    }

    fn grown(&self, margin: f64) -> Rect {
        Rect {
            x: self.x - margin,
            y: self.y - margin,
            width: self.width + 2.0 * margin,
            height: self.height + 2.0 * margin,
        }
    }
}

/// Capture starts when the cursor is this close to a button (logical px), at
/// once, from a single sample: a fast click must not pass through.
pub const ENTER_MARGIN: f64 = 6.0;
/// Capture holds until the cursor is this far from every button, so a cursor
/// resting on an edge does not flicker the flag.
pub const EXIT_MARGIN: f64 = 14.0;

/// Whether the window should let the mouse through at `cursor`, given the
/// interactive rectangles and whether it is capturing now (`capturing`: the
/// answer to the previous sample). It starts capturing within [`ENTER_MARGIN`]
/// of a rectangle and stops beyond [`EXIT_MARGIN`]. With no rectangles, or no
/// known cursor position, everything goes through (never steal a click by
/// mistake). Pure: the caller keeps the state.
pub fn ignore_cursor_events(
    interactive: &[Rect],
    cursor: Option<CursorPayload>,
    capturing: bool,
) -> bool {
    let Some(p) = cursor else { return true };
    let margin = if capturing { EXIT_MARGIN } else { ENTER_MARGIN };
    !interactive.iter().any(|r| r.grown(margin).contains(p))
}

/// The window operations the island needs, so the wiring below can be tested
/// without a real window (the same idea as `Host` in `effect_runner.rs`).
pub trait IslandWindow {
    fn position_top_center(&self);
    fn set_ignore_cursor_events(&self, ignore: bool) -> Result<(), String>;
    fn show(&self);
}

/// Lets the mouse through the whole window. A refusal is logged, never silent:
/// a window that keeps capturing the mouse blocks the apps underneath. Returns
/// whether it was applied.
pub fn apply_click_through(win: &impl IslandWindow) -> bool {
    match win.set_ignore_cursor_events(true) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("[niva] could not make the island window click-through: {err}");
            false
        }
    }
}

/// Keeps the window's click-through flag in step with the cursor and the
/// buttons: one `update` per cursor sample.
#[derive(Debug, Default)]
pub struct HitTester {
    /// What the window was last told; `None` until the first sample, so the
    /// first sample always sets it.
    applied: Option<bool>,
}

impl HitTester {
    /// The window was reset behind the tester's back (shown again, hidden): its
    /// flag is no longer what was last applied, so the next sample applies the
    /// answer again whatever it is.
    pub fn forget(&mut self) {
        self.applied = None;
    }

    /// Applies the answer for this sample, calling the window only when it
    /// differs from the last one that took effect. A refusal is logged and
    /// retried on the next sample (a stuck "capture" would block the apps under
    /// the island).
    pub fn update(
        &mut self,
        win: &impl IslandWindow,
        areas: &[Rect],
        cursor: Option<CursorPayload>,
    ) {
        let capturing = self.applied == Some(false);
        let ignore = ignore_cursor_events(areas, cursor, capturing);
        if self.applied == Some(ignore) {
            return;
        }
        match win.set_ignore_cursor_events(ignore) {
            Ok(()) => self.applied = Some(ignore),
            Err(err) => eprintln!("[niva] could not toggle the island's click-through: {err}"),
        }
    }
}

/// What the poller needs to decide whether the window captures the mouse: the
/// rectangles that may (empty unless buttons are on screen) and the tester that
/// remembers what the window was last told.
///
/// One lock orders everything: a poll reads the areas AND applies its answer
/// while holding the tester's lock, and the hide clears the areas AND lets the
/// mouse through under the same lock. A poll that read old areas can therefore
/// never set "capture" after the hide released it.
#[derive(Clone)]
pub struct HitTesting {
    areas: Arc<dyn Fn() -> Vec<Rect> + Send + Sync>,
    tester: Arc<Mutex<HitTester>>,
}

impl HitTesting {
    pub fn new(areas: Arc<dyn Fn() -> Vec<Rect> + Send + Sync>) -> Self {
        HitTesting {
            areas,
            tester: Arc::new(Mutex::new(HitTester::default())),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HitTester> {
        self.tester.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// One cursor sample.
    pub fn tick(&self, win: &impl IslandWindow, cursor: Option<CursorPayload>) {
        let mut tester = self.lock();
        let areas = (self.areas)();
        tester.update(win, &areas, cursor);
    }

    /// The window was reset behind the tester's back (every show).
    pub fn forget(&self) {
        self.lock().forget();
    }

    /// The island is leaving: `clear` ends the buttons (the gate), then the
    /// mouse goes through at once, all under the tester's lock.
    pub fn release(&self, win: &impl IslandWindow, clear: impl FnOnce()) {
        let mut tester = self.lock();
        clear();
        apply_click_through(win);
        tester.forget();
    }
}

/// Startup: place the window and make it click-through while it is hidden.
pub fn prepare_island(win: &impl IslandWindow) {
    win.position_top_center();
    apply_click_through(win);
}

/// Every show: place it, make it click-through, then show it, so it never
/// appears for a frame capturing the mouse.
pub fn show_island_window(win: &impl IslandWindow) {
    prepare_island(win);
    win.show();
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;

    #[derive(Default)]
    struct FakeWindow {
        calls: Mutex<Vec<String>>,
        refuse: bool,
    }

    impl IslandWindow for FakeWindow {
        fn position_top_center(&self) {
            self.calls.lock().unwrap().push("position".into());
        }
        fn set_ignore_cursor_events(&self, ignore: bool) -> Result<(), String> {
            self.calls.lock().unwrap().push(format!("ignore {ignore}"));
            if self.refuse {
                Err("window not ready".into())
            } else {
                Ok(())
            }
        }
        fn show(&self) {
            self.calls.lock().unwrap().push("show".into());
        }
    }

    fn calls(win: &FakeWindow) -> Vec<String> {
        win.calls.lock().unwrap().clone()
    }

    #[test]
    fn startup_places_the_window_and_makes_it_click_through() {
        let win = FakeWindow::default();
        prepare_island(&win);
        assert_eq!(calls(&win), vec!["position", "ignore true"]);
    }

    #[test]
    fn every_show_is_click_through_before_the_window_appears() {
        let win = FakeWindow::default();
        show_island_window(&win);
        show_island_window(&win);
        assert_eq!(
            calls(&win),
            vec![
                "position",
                "ignore true",
                "show",
                "position",
                "ignore true",
                "show"
            ]
        );
    }

    #[test]
    fn a_refused_click_through_is_reported_and_the_island_still_shows() {
        let win = FakeWindow {
            refuse: true,
            ..Default::default()
        };
        assert!(!apply_click_through(&win));
        show_island_window(&win);
        assert_eq!(calls(&win).last().map(String::as_str), Some("show"));
        assert!(apply_click_through(&FakeWindow::default()));
    }

    fn at(x: f64, y: f64) -> Option<CursorPayload> {
        Some(CursorPayload { x, y })
    }

    const BUTTON: Rect = Rect {
        x: 100.0,
        y: 50.0,
        width: 80.0,
        height: 30.0,
    };

    /// Runs a cursor trajectory through the pure function, feeding each answer
    /// back as the `capturing` state of the next sample, like the poller does.
    /// Returns, per sample, whether the window captures the mouse.
    fn trajectory(areas: &[Rect], points: &[(f64, f64)]) -> Vec<bool> {
        let mut capturing = false;
        points
            .iter()
            .map(|&(x, y)| {
                let ignore = ignore_cursor_events(areas, at(x, y), capturing);
                capturing = !ignore;
                capturing
            })
            .collect()
    }

    fn line(xs: &[f64]) -> Vec<(f64, f64)> {
        xs.iter().map(|&x| (x, 65.0)).collect()
    }

    #[test]
    fn with_no_interactive_area_everything_goes_through() {
        for cursor in [at(0.0, 0.0), at(320.0, 70.0), at(-5.0, 900.0), None] {
            for capturing in [false, true] {
                assert!(ignore_cursor_events(&[], cursor, capturing));
            }
        }
    }

    #[test]
    fn over_a_button_the_window_captures_the_mouse() {
        assert!(!ignore_cursor_events(&[BUTTON], at(140.0, 65.0), false));
    }

    #[test]
    fn far_from_the_buttons_the_mouse_goes_through() {
        assert!(ignore_cursor_events(&[BUTTON], at(10.0, 65.0), false));
        assert!(ignore_cursor_events(&[BUTTON], at(140.0, 5.0), false));
        assert!(ignore_cursor_events(&[BUTTON], at(500.0, 500.0), false));
    }

    #[test]
    fn capture_starts_at_the_enter_margin_and_not_before() {
        // Left and top edges: grown by 6, inclusive.
        assert!(!ignore_cursor_events(&[BUTTON], at(94.0, 65.0), false));
        assert!(ignore_cursor_events(&[BUTTON], at(93.9, 65.0), false));
        assert!(!ignore_cursor_events(&[BUTTON], at(140.0, 44.0), false));
        assert!(ignore_cursor_events(&[BUTTON], at(140.0, 43.9), false));
        // Right and bottom edges: grown by 6, exclusive.
        assert!(!ignore_cursor_events(&[BUTTON], at(185.9, 65.0), false));
        assert!(ignore_cursor_events(&[BUTTON], at(186.0, 65.0), false));
        assert!(!ignore_cursor_events(&[BUTTON], at(140.0, 85.9), false));
        assert!(ignore_cursor_events(&[BUTTON], at(140.0, 86.0), false));
    }

    #[test]
    fn once_captured_it_holds_until_the_release_margin() {
        // 10 px away: past the enter margin, inside the release margin.
        assert!(ignore_cursor_events(&[BUTTON], at(90.0, 65.0), false));
        assert!(!ignore_cursor_events(&[BUTTON], at(90.0, 65.0), true));
        // Grown by 14, same edge rules.
        assert!(!ignore_cursor_events(&[BUTTON], at(86.0, 65.0), true));
        assert!(ignore_cursor_events(&[BUTTON], at(85.9, 65.0), true));
        assert!(!ignore_cursor_events(&[BUTTON], at(193.9, 65.0), true));
        assert!(ignore_cursor_events(&[BUTTON], at(194.0, 65.0), true));
    }

    #[test]
    fn any_of_several_buttons_captures_the_mouse() {
        let cancel = Rect {
            x: 200.0,
            y: 50.0,
            width: 80.0,
            height: 30.0,
        };
        let both = [BUTTON, cancel];
        assert!(!ignore_cursor_events(&both, at(240.0, 60.0), false));
        // In the 20 px gap between them: each side's enter margin reaches 6 px
        // into it, and a cursor already captured crosses it without letting go.
        assert!(!ignore_cursor_events(&both, at(185.0, 60.0), false));
        assert!(ignore_cursor_events(&both, at(190.0, 60.0), false));
        assert!(!ignore_cursor_events(&both, at(190.0, 60.0), true));
        assert!(ignore_cursor_events(&both, at(190.0, 200.0), true));
    }

    #[test]
    fn an_unknown_cursor_never_captures_the_mouse() {
        assert!(ignore_cursor_events(&[BUTTON], None, false));
        assert!(ignore_cursor_events(&[BUTTON], None, true));
    }

    // --- trajectories over time ---------------------------------------------

    #[test]
    fn a_fast_entry_captures_on_the_very_first_sample_inside_the_margin() {
        // The cursor jumps from far away to 4 px outside the button in one
        // sample: capture is already on for that sample (no second sample).
        assert_eq!(
            trajectory(&[BUTTON], &[(600.0, 400.0), (96.0, 65.0)]),
            vec![false, true]
        );
        // And straight onto the button.
        assert_eq!(
            trajectory(&[BUTTON], &[(600.0, 400.0), (140.0, 65.0)]),
            vec![false, true]
        );
    }

    #[test]
    fn a_jump_over_the_whole_button_is_never_captured() {
        assert_eq!(
            trajectory(&[BUTTON], &[(0.0, 65.0), (300.0, 65.0)]),
            vec![false, false]
        );
    }

    #[test]
    fn jitter_on_the_edge_does_not_flicker_the_capture() {
        // Enters 5 px out, then wobbles between 5 and 12 px out: it stays on.
        let out = trajectory(
            &[BUTTON],
            &line(&[200.0, 95.0, 88.0, 95.0, 90.0, 94.0, 92.0, 89.0, 95.0]),
        );
        assert!(!out[0]);
        assert!(out[1..].iter().all(|&c| c), "{out:?}");
        // Wobbling outside the enter margin, never entering, never captures.
        let out = trajectory(&[BUTTON], &line(&[90.0, 92.0, 89.0, 91.0]));
        assert!(out.iter().all(|&c| !c), "{out:?}");
    }

    #[test]
    fn leaving_releases_past_the_release_margin_and_reentering_needs_the_enter_margin() {
        // 140 inside; 90 and 87 held; 84 is 16 px out: released; 88 and 92 are
        // outside the enter margin; 96 re-enters.
        assert_eq!(
            trajectory(
                &[BUTTON],
                &line(&[140.0, 90.0, 87.0, 84.0, 88.0, 92.0, 96.0])
            ),
            vec![true, true, true, false, false, false, true]
        );
    }

    #[test]
    fn the_buttons_vanishing_under_a_still_cursor_release_at_once() {
        let mut capturing = false;
        let mut seen = Vec::new();
        // The cursor never moves; the areas change underneath it.
        for areas in [&[BUTTON][..], &[BUTTON][..], &[][..], &[][..]] {
            capturing = !ignore_cursor_events(areas, at(140.0, 65.0), capturing);
            seen.push(capturing);
        }
        assert_eq!(seen, vec![true, true, false, false]);
    }

    #[test]
    fn a_scaled_display_hits_the_same_button_as_an_unscaled_one() {
        use crate::cursor::window_relative;
        // The window sits at x = 600 physical on a 1.5 display; the button is
        // at x = 100 logical, which is 600 + 150 = 750 physical.
        let physical = |x: f64, y: f64| Some(window_relative((x, y), (600, 0), 1.5));
        assert!(!ignore_cursor_events(
            &[BUTTON],
            physical(780.0, 90.0),
            false
        ));
        // 9 physical px = 6 logical px left of the button: on the enter margin.
        assert!(!ignore_cursor_events(
            &[BUTTON],
            physical(741.0, 90.0),
            false
        ));
        // 10 physical px = 6.67 logical: outside. Read as logical px it would hit.
        assert!(ignore_cursor_events(
            &[BUTTON],
            physical(740.0, 90.0),
            false
        ));
    }

    // --- the tester that talks to the window -----------------------------------

    fn applied(win: &FakeWindow) -> Vec<String> {
        calls(win)
            .into_iter()
            .filter(|c| c.starts_with("ignore"))
            .collect()
    }

    #[test]
    fn the_tester_only_calls_the_window_when_the_answer_changes() {
        let win = FakeWindow::default();
        let mut tester = HitTester::default();
        for cursor in [
            at(600.0, 400.0),
            at(601.0, 400.0),
            at(140.0, 65.0),
            at(141.0, 65.0),
            at(600.0, 400.0),
        ] {
            tester.update(&win, &[BUTTON], cursor);
        }
        assert_eq!(
            applied(&win),
            vec!["ignore true", "ignore false", "ignore true"]
        );
    }

    #[test]
    fn the_tester_releases_when_the_buttons_go_away_under_a_still_cursor() {
        let win = FakeWindow::default();
        let mut tester = HitTester::default();
        tester.update(&win, &[BUTTON], at(140.0, 65.0));
        tester.update(&win, &[BUTTON], at(140.0, 65.0));
        tester.update(&win, &[], at(140.0, 65.0));
        assert_eq!(applied(&win), vec!["ignore false", "ignore true"]);
    }

    #[test]
    fn after_the_window_was_reset_the_next_sample_applies_the_answer_again() {
        let win = FakeWindow::default();
        let mut tester = HitTester::default();
        tester.update(&win, &[BUTTON], at(140.0, 65.0));
        // A show resets the window to click-through; the cursor has not moved.
        tester.forget();
        tester.update(&win, &[BUTTON], at(140.0, 65.0));
        assert_eq!(applied(&win), vec!["ignore false", "ignore false"]);
    }

    #[test]
    fn a_refused_toggle_is_retried_on_the_next_sample() {
        let win = FakeWindow {
            refuse: true,
            ..Default::default()
        };
        let mut tester = HitTester::default();
        tester.update(&win, &[BUTTON], at(140.0, 65.0));
        tester.update(&win, &[BUTTON], at(140.0, 65.0));
        assert_eq!(applied(&win), vec!["ignore false", "ignore false"]);
    }

    // --- the poll and the hide share one lock ---------------------------------

    fn shared_areas(initial: Vec<Rect>) -> (Arc<Mutex<Vec<Rect>>>, HitTesting) {
        let store = Arc::new(Mutex::new(initial));
        let reader = store.clone();
        let hit = HitTesting::new(Arc::new(move || reader.lock().unwrap().clone()));
        (store, hit)
    }

    #[test]
    fn a_poll_reads_the_areas_while_holding_the_testers_lock() {
        let seen_locked = Arc::new(Mutex::new(None));
        let slot = seen_locked.clone();
        let probe: Arc<Mutex<Option<HitTesting>>> = Arc::new(Mutex::new(None));
        let probe_in = probe.clone();
        let hit = HitTesting::new(Arc::new(move || {
            let hit = probe_in.lock().unwrap().clone().unwrap();
            *slot.lock().unwrap() = Some(hit.tester.try_lock().is_err());
            vec![BUTTON]
        }));
        *probe.lock().unwrap() = Some(hit.clone());
        hit.tick(&FakeWindow::default(), at(140.0, 65.0));
        assert_eq!(*seen_locked.lock().unwrap(), Some(true));
    }

    #[test]
    fn the_hide_clears_and_releases_under_the_same_lock() {
        let (store, hit) = shared_areas(vec![BUTTON]);
        let win = FakeWindow::default();
        hit.tick(&win, at(140.0, 65.0));
        let locked_during_clear = Mutex::new(None);
        hit.release(&win, || {
            *locked_during_clear.lock().unwrap() = Some(hit.tester.try_lock().is_err());
            store.lock().unwrap().clear();
        });
        assert_eq!(*locked_during_clear.lock().unwrap(), Some(true));
        assert_eq!(applied(&win), vec!["ignore false", "ignore true"]);
    }

    #[test]
    fn a_poll_after_the_hide_cannot_recapture_the_mouse() {
        let (store, hit) = shared_areas(vec![BUTTON]);
        let win = FakeWindow::default();
        hit.tick(&win, at(140.0, 65.0));
        hit.release(&win, || store.lock().unwrap().clear());
        // The poller's next samples, cursor still where the button was.
        hit.tick(&win, at(140.0, 65.0));
        hit.tick(&win, at(141.0, 65.0));
        assert_eq!(
            applied(&win).last().map(String::as_str),
            Some("ignore true")
        );
    }
}
