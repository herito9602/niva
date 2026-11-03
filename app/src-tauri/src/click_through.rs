//! Click-through of the island window.
//!
//! The island window is larger than any island (room for the shadow and the
//! spring overshoot), so most of it is empty and transparent. It must never
//! block clicks to the apps underneath.
//!
//! Today the island has no clickable control: the whole window ignores the
//! mouse (`set_ignore_cursor_events(true)`, called from Rust; the webview needs
//! no permission for it). PR 2d adds confirm buttons to the island; then the
//! window has to be hit-tested: list the buttons' rectangles in
//! [`ISLAND_INTERACTIVE`] and let the cursor poller (which already knows the
//! cursor in window-relative logical pixels) call [`ignore_cursor_events`] on
//! every sample, toggling the flag only when the answer changes.

use crate::cursor::CursorPayload;

/// A rectangle in logical pixels, relative to the window's top-left corner.
#[derive(Debug, Clone, Copy, PartialEq)]
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
}

/// Clickable areas of the island. Empty: the island has no controls yet.
pub const ISLAND_INTERACTIVE: &[Rect] = &[];

/// Whether the window should let the mouse through at `cursor`. It captures
/// the mouse only over an interactive rectangle; with no known cursor position
/// it lets everything through (never steal a click by mistake).
pub fn ignore_cursor_events(interactive: &[Rect], cursor: Option<CursorPayload>) -> bool {
    match cursor {
        Some(p) => !interactive.iter().any(|r| r.contains(p)),
        None => true,
    }
}

/// The window operations the island needs, so the wiring below can be tested
/// without a real window (the same idea as `Host` in `effect_runner.rs`).
pub trait IslandWindow {
    fn position_top_center(&self);
    fn set_ignore_cursor_events(&self, ignore: bool) -> Result<(), String>;
    fn show(&self);
}

/// Lets the mouse through the window wherever there is nothing to click
/// (everywhere, for now). A refusal is logged, never silent: a window that
/// keeps capturing the mouse blocks the apps underneath. Returns whether it
/// was applied.
pub fn apply_click_through(win: &impl IslandWindow) -> bool {
    match win.set_ignore_cursor_events(ignore_cursor_events(ISLAND_INTERACTIVE, None)) {
        Ok(()) => true,
        Err(err) => {
            eprintln!("[niva] could not make the island window click-through: {err}");
            false
        }
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
    use std::sync::Mutex;

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

    #[test]
    fn with_no_interactive_area_everything_goes_through() {
        for cursor in [at(0.0, 0.0), at(320.0, 70.0), at(-5.0, 900.0), None] {
            assert!(ignore_cursor_events(&[], cursor));
        }
    }

    #[test]
    fn the_island_has_no_clickable_area_yet() {
        assert!(ISLAND_INTERACTIVE.is_empty());
        assert!(ignore_cursor_events(ISLAND_INTERACTIVE, at(320.0, 70.0)));
    }

    #[test]
    fn over_a_button_the_window_captures_the_mouse() {
        assert!(!ignore_cursor_events(&[BUTTON], at(140.0, 65.0)));
    }

    #[test]
    fn outside_the_buttons_the_mouse_goes_through() {
        assert!(ignore_cursor_events(&[BUTTON], at(99.0, 65.0)));
        assert!(ignore_cursor_events(&[BUTTON], at(140.0, 10.0)));
        assert!(ignore_cursor_events(&[BUTTON], at(500.0, 500.0)));
    }

    #[test]
    fn the_top_left_edge_is_inside_and_the_bottom_right_is_outside() {
        assert!(!ignore_cursor_events(&[BUTTON], at(100.0, 50.0)));
        assert!(ignore_cursor_events(&[BUTTON], at(180.0, 65.0)));
        assert!(ignore_cursor_events(&[BUTTON], at(140.0, 80.0)));
    }

    #[test]
    fn any_of_several_buttons_captures_the_mouse() {
        let cancel = Rect {
            x: 200.0,
            y: 50.0,
            width: 80.0,
            height: 30.0,
        };
        assert!(!ignore_cursor_events(&[BUTTON, cancel], at(240.0, 60.0)));
        assert!(ignore_cursor_events(&[BUTTON, cancel], at(190.0, 60.0)));
    }

    #[test]
    fn an_unknown_cursor_never_captures_the_mouse() {
        assert!(ignore_cursor_events(&[BUTTON], None));
    }
}
