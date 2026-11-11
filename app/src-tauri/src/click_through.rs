//! Click-through of the island window.
//!
//! The island window (290x76) is larger than the notch (210x36; room for the
//! spring overshoot), so most of it is empty and transparent. It must never
//! block clicks to the apps underneath.
//!
//! The whole window ignores the mouse, always: `set_ignore_cursor_events(true)`
//! is called from Rust on prepare and on every show, and nothing ever turns it
//! off. The notch has no buttons, so there is no hit test and no cursor poller;
//! the webview needs no permission for any of it.

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

    #[test]
    fn window_ignores_cursor_events_in_every_state() {
        let win = FakeWindow::default();
        prepare_island(&win);
        show_island_window(&win);
        show_island_window(&win);
        assert!(apply_click_through(&win));
        let flags: Vec<String> = calls(&win)
            .into_iter()
            .filter(|c| c.starts_with("ignore"))
            .collect();
        assert!(!flags.is_empty());
        assert!(flags.iter().all(|c| c == "ignore true"), "{flags:?}");
    }
}
