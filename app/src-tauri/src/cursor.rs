//! Cursor poller: the island is `focusable:false`, so the webview never sees
//! mouse moves outside it. While the island is visible the host polls the
//! global cursor position (`GetCursorPos`, through Tauri's `cursor_position`)
//! at ~30 Hz and emits `niva://cursor {x, y}` relative to the window.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::ui_adapter::CURSOR_EVENT;

/// ~30 Hz.
const POLL_INTERVAL: Duration = Duration::from_millis(33);

/// Payload of `niva://cursor`: logical pixels relative to the window's
/// top-left corner (negative or beyond the size when the cursor is outside).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct CursorPayload {
    pub x: f64,
    pub y: f64,
}

/// Converts a global physical cursor position into window-relative logical
/// pixels.
pub fn window_relative(
    cursor: (f64, f64),
    window_origin: (i32, i32),
    scale_factor: f64,
) -> CursorPayload {
    CursorPayload {
        x: (cursor.0 - f64::from(window_origin.0)) / scale_factor,
        y: (cursor.1 - f64::from(window_origin.1)) / scale_factor,
    }
}

/// Suppresses repeats so a still cursor produces no events.
#[derive(Debug, Default)]
pub struct CursorTracker {
    last: Option<CursorPayload>,
}

impl CursorTracker {
    pub fn next(&mut self, sample: Option<CursorPayload>) -> Option<CursorPayload> {
        let sample = sample?;
        if self.last == Some(sample) {
            return None;
        }
        self.last = Some(sample);
        Some(sample)
    }
}

/// Where samples come from and go to (the Tauri side of the poller).
pub trait CursorSource: Send + 'static {
    /// The cursor in window-relative logical pixels; `None` when the window or
    /// the cursor position is unavailable.
    fn sample(&mut self) -> Option<CursorPayload>;
    fn emit(&mut self, payload: CursorPayload);
}

/// Start/stop bookkeeping. Each `try_start` bumps a generation so a stale
/// thread exits even if a new one started before it woke up.
#[derive(Clone, Default)]
pub struct PollControl {
    generation: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
}

impl PollControl {
    /// The generation of the thread to spawn, or `None` if already running.
    pub fn try_start(&self) -> Option<u64> {
        if self.running.swap(true, Ordering::SeqCst) {
            return None;
        }
        Some(self.generation.fetch_add(1, Ordering::SeqCst) + 1)
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    pub fn is_current(&self, generation: u64) -> bool {
        self.generation.load(Ordering::SeqCst) == generation
    }
}

/// One poll: sample, drop repeats, emit changes. Returns whether it emitted.
pub fn poll_once(tracker: &mut CursorTracker, source: &mut impl CursorSource) -> bool {
    match tracker.next(source.sample()) {
        Some(payload) => {
            source.emit(payload);
            true
        }
        None => false,
    }
}

/// Polls until this generation is superseded or stopped.
pub fn poll_loop(
    control: &PollControl,
    generation: u64,
    mut source: impl CursorSource,
    interval: Duration,
) {
    let mut tracker = CursorTracker::default();
    while control.is_current(generation) {
        poll_once(&mut tracker, &mut source);
        thread::sleep(interval);
    }
}

/// [`CursorSource`] backed by a Tauri window: samples the global cursor and
/// emits `niva://cursor` to that window only.
struct WindowSource {
    app: AppHandle,
    label: &'static str,
}

impl CursorSource for WindowSource {
    fn sample(&mut self) -> Option<CursorPayload> {
        let win = self.app.get_webview_window(self.label)?;
        match (
            self.app.cursor_position(),
            win.outer_position(),
            win.scale_factor(),
        ) {
            (Ok(cursor), Ok(origin), Ok(scale)) => Some(window_relative(
                (cursor.x, cursor.y),
                (origin.x, origin.y),
                scale,
            )),
            _ => None,
        }
    }

    fn emit(&mut self, payload: CursorPayload) {
        let _ = self.app.emit_to(self.label, CURSOR_EVENT, payload);
    }
}

/// Starts and stops the polling thread.
#[derive(Clone, Default)]
pub struct CursorPoller {
    control: PollControl,
}

impl CursorPoller {
    pub fn start(&self, app: &AppHandle, window_label: &'static str) {
        let Some(generation) = self.control.try_start() else {
            return;
        };
        let control = self.control.clone();
        let source = WindowSource {
            app: app.clone(),
            label: window_label,
        };
        thread::spawn(move || poll_loop(&control, generation, source, POLL_INTERVAL));
    }

    pub fn stop(&self) {
        self.control.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f64, y: f64) -> CursorPayload {
        CursorPayload { x, y }
    }

    #[test]
    fn position_is_relative_to_the_window_origin() {
        assert_eq!(
            window_relative((1000.0, 50.0), (700, 0), 1.0),
            p(300.0, 50.0)
        );
    }

    #[test]
    fn position_is_in_logical_pixels_on_a_scaled_display() {
        assert_eq!(
            window_relative((1050.0, 75.0), (900, 0), 1.5),
            p(100.0, 50.0)
        );
    }

    #[test]
    fn a_cursor_left_of_or_above_the_window_is_negative() {
        assert_eq!(
            window_relative((100.0, 10.0), (700, 20), 2.0),
            p(-300.0, -5.0)
        );
    }

    #[test]
    fn a_monitor_left_of_the_primary_has_negative_origins() {
        assert_eq!(
            window_relative((-1000.0, 5.0), (-1500, 0), 1.0),
            p(500.0, 5.0)
        );
    }

    #[test]
    fn tracker_emits_first_sample_then_only_changes() {
        let mut t = CursorTracker::default();
        assert_eq!(t.next(Some(p(1.0, 2.0))), Some(p(1.0, 2.0)));
        assert_eq!(t.next(Some(p(1.0, 2.0))), None);
        assert_eq!(t.next(Some(p(1.0, 3.0))), Some(p(1.0, 3.0)));
        assert_eq!(t.next(Some(p(1.0, 2.0))), Some(p(1.0, 2.0)));
    }

    #[test]
    fn tracker_ignores_failed_samples_and_remembers_the_last_position() {
        let mut t = CursorTracker::default();
        assert_eq!(t.next(None), None);
        t.next(Some(p(5.0, 5.0)));
        assert_eq!(t.next(None), None);
        assert_eq!(t.next(Some(p(5.0, 5.0))), None);
    }

    #[test]
    fn poll_rate_is_about_30_hz() {
        let hz = 1000 / POLL_INTERVAL.as_millis();
        assert!((29..=31).contains(&hz), "{hz} Hz");
    }

    #[test]
    fn payload_json_is_x_and_y() {
        assert_eq!(
            serde_json::to_value(p(1.5, -2.0)).unwrap(),
            serde_json::json!({"x": 1.5, "y": -2.0})
        );
    }

    #[derive(Default)]
    struct FakeSource {
        samples: std::collections::VecDeque<Option<CursorPayload>>,
        emitted: Arc<std::sync::Mutex<Vec<CursorPayload>>>,
        /// Stops the control once the queue is drained.
        stop_when_empty: Option<PollControl>,
    }

    impl CursorSource for FakeSource {
        fn sample(&mut self) -> Option<CursorPayload> {
            let next = self.samples.pop_front().flatten();
            if self.samples.is_empty()
                && let Some(control) = &self.stop_when_empty
            {
                control.stop();
            }
            next
        }
        fn emit(&mut self, payload: CursorPayload) {
            self.emitted.lock().unwrap().push(payload);
        }
    }

    #[test]
    fn a_still_cursor_emits_nothing_and_a_failed_sample_emits_nothing() {
        let mut tracker = CursorTracker::default();
        let mut src = FakeSource {
            samples: [
                Some(p(1.0, 1.0)),
                Some(p(1.0, 1.0)),
                None,
                Some(p(2.0, 1.0)),
            ]
            .into(),
            ..FakeSource::default()
        };
        let results: Vec<bool> = (0..4).map(|_| poll_once(&mut tracker, &mut src)).collect();
        assert_eq!(results, vec![true, false, false, true]);
        assert_eq!(*src.emitted.lock().unwrap(), vec![p(1.0, 1.0), p(2.0, 1.0)]);
    }

    #[test]
    fn a_second_start_while_running_spawns_nothing() {
        let c = PollControl::default();
        assert_eq!(c.try_start(), Some(1));
        assert_eq!(c.try_start(), None);
    }

    #[test]
    fn stop_invalidates_the_generation_and_a_restart_gets_a_new_one() {
        let c = PollControl::default();
        let first = c.try_start().unwrap();
        assert!(c.is_current(first));
        c.stop();
        assert!(!c.is_current(first));
        let second = c.try_start().expect("restartable after stop");
        assert!(second > first);
        assert!(c.is_current(second));
        assert!(!c.is_current(first), "the stale thread still exits");
    }

    #[test]
    fn the_loop_polls_until_stopped_and_never_for_a_stale_generation() {
        let control = PollControl::default();
        let generation = control.try_start().unwrap();
        let emitted = Arc::new(std::sync::Mutex::new(Vec::new()));
        let src = FakeSource {
            samples: [Some(p(1.0, 1.0)), Some(p(2.0, 2.0)), Some(p(2.0, 2.0))].into(),
            emitted: emitted.clone(),
            stop_when_empty: Some(control.clone()),
        };
        poll_loop(&control, generation, src, Duration::ZERO);
        assert_eq!(*emitted.lock().unwrap(), vec![p(1.0, 1.0), p(2.0, 2.0)]);

        // A superseded generation exits without sampling at all.
        let src = FakeSource {
            samples: [Some(p(9.0, 9.0))].into(),
            emitted: emitted.clone(),
            stop_when_empty: None,
        };
        poll_loop(&control, generation, src, Duration::ZERO);
        assert_eq!(emitted.lock().unwrap().len(), 2);
    }
}
