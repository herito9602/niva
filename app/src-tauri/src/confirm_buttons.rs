//! The island's Confirm and Cancel buttons, host side (PR 2d-b, part 1).
//!
//! The island webview has two narrow commands and nothing else:
//!
//! * [`answer_confirmation`] carries only `(confirmation id, accept or reject)`
//!   and becomes `Event::Accept { id }` or `Event::Reject { id }`, the same
//!   events the debug window's buttons produce (a click, as opposed to a spoken
//!   answer, which arrives as a transcript). No text, no action name.
//! * [`report_confirm_buttons`] carries the buttons' rectangles (numbers) so the
//!   cursor poller can make exactly those parts of the click-through window
//!   capture the mouse (`click_through.rs`).
//!
//! The host does not trust the webview's timing. [`ConfirmGate`] knows, from
//! the views the host itself published, which confirmation is on screen and
//! since when, and it enforces here, not in the page:
//!
//! * an answer for any other id is dropped (the reducer checks the id too);
//! * one answer per confirmation (a double click answers once);
//! * a **Confirm** that arrives less than [`ARM_MS`] after the island reported
//!   its buttons (the first non-empty report for that id, sent once they are
//!   mounted) is ignored, and so is every Confirm before that report: with no
//!   report Confirm can never pass. A click meant for another app therefore
//!   cannot land on Confirm as the island drops in, however slow the show is.
//!   **Cancel** is always honoured: cancelling is the safe direction.
//!
//! The clock is injected, so the guard is tested without waiting.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use niva_core::state_machine::Event;
use tauri::{State as TauriState, WebviewWindow};

use crate::click_through::Rect;
use crate::dispatcher::{Input, InputTx};
use crate::ui_adapter::{UiEvent, UiPort};

/// A Confirm arriving sooner than this after the buttons were reported is ignored.
/// The island dims the buttons for the same time (`CONFIRM_ARM_MS` in
/// `confirmCommands.ts`; a contract test keeps them equal).
pub const ARM_MS: u64 = 500;

/// Most rectangles the island may report: it has two buttons.
pub const MAX_AREAS: usize = 2;
/// Largest rectangle (logical px) a report may hold: a button, with room. A
/// contract test derives these from the button size in `layout.ts`.
pub const MAX_AREA_WIDTH: f64 = 120.0;
pub const MAX_AREA_HEIGHT: f64 = 48.0;

/// Milliseconds on a monotonic clock.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// Why an answer was dropped. Nothing is reported back to the page: a dropped
/// answer is not an error, it simply does not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ignored {
    /// No confirmation is on screen, or it is another one (a stale click).
    NoConfirmation,
    /// A Confirm before the island reported its buttons.
    NotShown,
    /// A Confirm inside the first [`ARM_MS`] after the report.
    TooSoon,
    /// This confirmation was already answered.
    AlreadyAnswered,
}

struct Shown {
    id: u64,
    /// When the island first reported its buttons (host clock, ms); `None`
    /// until then, and a Confirm cannot pass while it is.
    armed_at: Option<u64>,
    answered: bool,
    /// Where the buttons are, as the island reported them for this id.
    areas: Vec<Rect>,
}

/// What the host knows about the buttons on screen.
pub struct ConfirmGate {
    clock: Clock,
    shown: Mutex<Option<Shown>>,
}

impl ConfirmGate {
    pub fn new(clock: Clock) -> Self {
        ConfirmGate {
            clock,
            shown: Mutex::new(None),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Shown>> {
        self.shown.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A view was published: `confirm_id` is the confirmation it shows, if any.
    /// A new id starts the guard and forgets the old rectangles; the same id
    /// again changes nothing; no confirmation closes the gate.
    pub fn on_view(&self, confirm_id: Option<u64>) {
        let mut shown = self.lock();
        match confirm_id {
            None => *shown = None,
            Some(id) if shown.as_ref().is_some_and(|s| s.id == id) => {}
            Some(id) => {
                *shown = Some(Shown {
                    id,
                    armed_at: None,
                    answered: false,
                    areas: Vec::new(),
                });
            }
        }
    }

    /// The island is hiding (or anything else ended the buttons).
    pub fn clear(&self) {
        *self.lock() = None;
    }

    /// The island says where the buttons of confirmation `id` are. Dropped
    /// unless that confirmation is the one on screen. `bounds` is the window
    /// size in logical pixels. Returns whether it was taken.
    pub fn report(&self, id: u64, bounds: (f64, f64), areas: &[Rect]) -> bool {
        let mut shown = self.lock();
        match shown.as_mut() {
            Some(s) if s.id == id && !s.answered => {
                s.areas = sanitize(bounds, areas);
                if !s.areas.is_empty() && s.armed_at.is_none() {
                    s.armed_at = Some((self.clock)());
                }
                true
            }
            _ => false,
        }
    }

    /// Rectangles that may capture the mouse right now: none unless buttons
    /// are on screen and still waiting for an answer.
    pub fn areas(&self) -> Vec<Rect> {
        match self.lock().as_ref() {
            Some(s) if !s.answered => s.areas.clone(),
            _ => Vec::new(),
        }
    }

    /// The reducer event for a click on the buttons of confirmation `id`, or
    /// why the click is dropped.
    pub fn answer(&self, id: u64, accept: bool) -> Result<Event, Ignored> {
        let mut shown = self.lock();
        let s = match shown.as_mut() {
            Some(s) if s.id == id => s,
            _ => return Err(Ignored::NoConfirmation),
        };
        if s.answered {
            return Err(Ignored::AlreadyAnswered);
        }
        // Only Confirm waits. Without a report it never passes; if the clock
        // ever stepped back, the subtraction saturates at 0 and Confirm stays
        // ignored: the safe side.
        if accept {
            let Some(armed_at) = s.armed_at else {
                return Err(Ignored::NotShown);
            };
            if (self.clock)().saturating_sub(armed_at) < ARM_MS {
                return Err(Ignored::TooSoon);
            }
        }
        s.answered = true;
        Ok(if accept {
            Event::Accept { id }
        } else {
            Event::Reject { id }
        })
    }
}

/// What the page reported, reduced to usable rectangles. A report is rejected as
/// a whole (nothing is kept) if it has more than [`MAX_AREAS`] rectangles, a
/// non-finite number, or a rectangle bigger than a button
/// ([`MAX_AREA_WIDTH`] by [`MAX_AREA_HEIGHT`]): a page cannot cover the window.
/// What passes is clipped to the window and empty rectangles are dropped.
pub fn sanitize(bounds: (f64, f64), areas: &[Rect]) -> Vec<Rect> {
    let (width, height) = bounds;
    let acceptable = areas.len() <= MAX_AREAS
        && areas.iter().all(|r| {
            [r.x, r.y, r.width, r.height].iter().all(|n| n.is_finite())
                && r.width <= MAX_AREA_WIDTH
                && r.height <= MAX_AREA_HEIGHT
        });
    if !acceptable {
        return Vec::new();
    }
    areas
        .iter()
        .filter_map(|r| {
            let (x0, y0) = (r.x.max(0.0), r.y.max(0.0));
            let (x1, y1) = ((r.x + r.width).min(width), (r.y + r.height).min(height));
            (x1 > x0 && y1 > y0).then_some(Rect {
                x: x0,
                y: y0,
                width: x1 - x0,
                height: y1 - y0,
            })
        })
        .collect()
}

/// Sends the answer into the dispatcher queue if the gate lets it through.
/// A dropped answer is `Ok`: the page has nothing to do about it.
pub fn answer(gate: &ConfirmGate, tx: &InputTx, id: u64, accept: bool) -> Result<(), String> {
    match gate.answer(id, accept) {
        Ok(event) => tx.send(Input::Event(event)).map_err(|err| err.to_string()),
        Err(_) => Ok(()),
    }
}

/// Command of the island: answers the confirmation `id`. It can build no other
/// event than `Accept { id }` or `Reject { id }`.
#[tauri::command]
pub fn answer_confirmation(
    id: u64,
    accept: bool,
    gate: TauriState<'_, Arc<ConfirmGate>>,
    tx: TauriState<'_, InputTx>,
) -> Result<(), String> {
    answer(&gate, &tx, id, accept)
}

/// Command of the island: where the buttons of confirmation `id` are, in
/// logical pixels relative to the window.
#[tauri::command]
pub fn report_confirm_buttons(
    id: u64,
    areas: Vec<Rect>,
    window: WebviewWindow,
    gate: TauriState<'_, Arc<ConfirmGate>>,
) -> Result<(), String> {
    let scale = window.scale_factor().map_err(|err| err.to_string())?;
    let size = window.inner_size().map_err(|err| err.to_string())?;
    let bounds = (
        f64::from(size.width) / scale,
        f64::from(size.height) / scale,
    );
    gate.report(id, bounds, &areas);
    Ok(())
}

/// A [`UiPort`] that keeps the gate in step with what the island shows, then
/// forwards the event untouched. It sits in front of the Tauri adapter.
pub struct GateUi {
    inner: Arc<dyn UiPort<UiEvent>>,
    gate: Arc<ConfirmGate>,
}

impl GateUi {
    pub fn new(inner: Arc<dyn UiPort<UiEvent>>, gate: Arc<ConfirmGate>) -> Self {
        GateUi { inner, gate }
    }
}

impl UiPort<UiEvent> for GateUi {
    fn publish(&self, event: UiEvent) {
        match &event {
            UiEvent::View(view) => self.gate.on_view(view.confirm.as_ref().map(|c| c.id)),
            UiEvent::Island { visible: false } => self.gate.clear(),
            UiEvent::Island { visible: true } => {}
        }
        self.inner.publish(event);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};

    use niva_core::state_machine::{Event, Phase, State, ToolVerdict, reduce};
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;
    use crate::ui_adapter::{ConfirmView, UiEvent, UiPort, ViewPayload};

    struct Rig {
        now: Arc<AtomicU64>,
        gate: Arc<ConfirmGate>,
    }

    fn rig() -> Rig {
        let now = Arc::new(AtomicU64::new(10_000));
        let clock = now.clone();
        let gate = Arc::new(ConfirmGate::new(Arc::new(move || {
            clock.load(Ordering::SeqCst)
        })));
        Rig { now, gate }
    }

    impl Rig {
        fn advance(&self, ms: u64) {
            self.now.fetch_add(ms, Ordering::SeqCst);
        }

        /// The confirmation appears and the island reports its buttons, as it
        /// does once they are mounted: the guard starts now.
        fn show(&self, id: u64) {
            self.gate.on_view(Some(id));
            assert!(self.gate.report(id, WINDOW, &BUTTONS));
        }
    }

    const BUTTONS: [Rect; 2] = [
        Rect {
            x: 300.0,
            y: 90.0,
            width: 84.0,
            height: 32.0,
        },
        Rect {
            x: 392.0,
            y: 90.0,
            width: 84.0,
            height: 32.0,
        },
    ];
    const WINDOW: (f64, f64) = (640.0, 232.0);

    #[test]
    fn a_confirm_can_never_pass_before_the_buttons_are_reported() {
        // Fail closed: a slow show, a page that never reports, a lost report.
        let r = rig();
        r.gate.on_view(Some(7));
        assert_eq!(r.gate.answer(7, true), Err(Ignored::NotShown));
        r.advance(60_000);
        assert_eq!(r.gate.answer(7, true), Err(Ignored::NotShown));
    }

    #[test]
    fn cancel_passes_even_before_the_buttons_are_reported() {
        let r = rig();
        r.gate.on_view(Some(7));
        assert_eq!(r.gate.answer(7, false), Ok(Event::Reject { id: 7 }));
    }

    #[test]
    fn the_guard_counts_from_the_first_report_not_from_the_view() {
        let r = rig();
        r.gate.on_view(Some(7));
        // A slow show: the island takes 800 ms to mount the buttons.
        r.advance(800);
        r.gate.report(7, WINDOW, &BUTTONS);
        r.advance(ARM_MS - 1);
        assert_eq!(r.gate.answer(7, true), Err(Ignored::TooSoon));
        r.advance(1);
        assert_eq!(r.gate.answer(7, true), Ok(Event::Accept { id: 7 }));
    }

    #[test]
    fn later_reports_do_not_restart_the_guard() {
        let r = rig();
        r.show(7);
        r.advance(400);
        // The message wrapped and the buttons moved.
        r.gate.report(7, WINDOW, &[rect(300.0, 120.0, 84.0, 32.0)]);
        r.advance(100);
        assert_eq!(r.gate.answer(7, true), Ok(Event::Accept { id: 7 }));
    }

    #[test]
    fn an_empty_or_rejected_report_does_not_arm_the_guard() {
        let r = rig();
        r.gate.on_view(Some(7));
        r.gate.report(7, WINDOW, &[]);
        r.gate.report(7, WINDOW, &[rect(0.0, 0.0, 640.0, 232.0)]);
        r.advance(10_000);
        assert_eq!(r.gate.answer(7, true), Err(Ignored::NotShown));
    }

    #[test]
    fn a_new_confirmation_needs_its_own_report_to_arm() {
        let r = rig();
        r.show(1);
        r.advance(1_000);
        r.gate.answer(1, true).unwrap();
        r.gate.on_view(Some(2));
        r.advance(1_000);
        assert_eq!(r.gate.answer(2, true), Err(Ignored::NotShown));
    }

    // --- the accidental click guard ------------------------------------------

    #[test]
    fn an_answer_with_no_confirmation_on_screen_is_ignored() {
        let r = rig();
        assert_eq!(r.gate.answer(1, true), Err(Ignored::NoConfirmation));
        assert_eq!(r.gate.answer(1, false), Err(Ignored::NoConfirmation));
    }

    #[test]
    fn confirm_is_ignored_for_the_first_half_second_and_honoured_after() {
        let r = rig();
        r.show(7);
        assert_eq!(r.gate.answer(7, true), Err(Ignored::TooSoon));
        r.advance(ARM_MS - 1);
        assert_eq!(r.gate.answer(7, true), Err(Ignored::TooSoon));
        r.advance(1);
        assert_eq!(r.gate.answer(7, true), Ok(Event::Accept { id: 7 }));
    }

    #[test]
    fn cancel_is_always_honoured_even_at_once() {
        let r = rig();
        r.show(7);
        assert_eq!(r.gate.answer(7, false), Ok(Event::Reject { id: 7 }));
    }

    #[test]
    fn a_confirm_that_was_too_soon_does_not_spend_the_answer() {
        let r = rig();
        r.show(7);
        r.advance(100);
        assert_eq!(r.gate.answer(7, true), Err(Ignored::TooSoon));
        r.advance(500);
        assert_eq!(r.gate.answer(7, true), Ok(Event::Accept { id: 7 }));
    }

    #[test]
    fn each_new_confirmation_restarts_the_guard() {
        let r = rig();
        r.show(1);
        r.advance(5_000);
        assert_eq!(r.gate.answer(1, true), Ok(Event::Accept { id: 1 }));
        // The next prompt appears; a second click aimed at the first one must
        // not land on the second prompt's Confirm.
        r.show(2);
        r.advance(100);
        assert_eq!(r.gate.answer(2, true), Err(Ignored::TooSoon));
    }

    #[test]
    fn republishing_the_same_confirmation_does_not_restart_the_guard() {
        let r = rig();
        r.show(7);
        r.advance(600);
        // A new view for the same confirmation (the deadline appeared, say).
        r.show(7);
        assert_eq!(r.gate.answer(7, true), Ok(Event::Accept { id: 7 }));
    }

    // --- stale and repeated answers --------------------------------------------

    #[test]
    fn an_answer_for_another_id_is_ignored() {
        let r = rig();
        r.show(7);
        r.advance(1_000);
        assert_eq!(r.gate.answer(6, true), Err(Ignored::NoConfirmation));
        assert_eq!(r.gate.answer(8, false), Err(Ignored::NoConfirmation));
        assert_eq!(r.gate.answer(0, true), Err(Ignored::NoConfirmation));
        // The right one still works afterwards.
        assert_eq!(r.gate.answer(7, true), Ok(Event::Accept { id: 7 }));
    }

    #[test]
    fn a_late_click_for_the_previous_prompt_does_nothing() {
        let r = rig();
        r.show(1);
        r.advance(1_000);
        r.show(2);
        r.advance(1_000);
        assert_eq!(r.gate.answer(1, true), Err(Ignored::NoConfirmation));
    }

    #[test]
    fn a_double_click_answers_once() {
        let r = rig();
        r.show(7);
        r.advance(1_000);
        assert_eq!(r.gate.answer(7, true), Ok(Event::Accept { id: 7 }));
        assert_eq!(r.gate.answer(7, true), Err(Ignored::AlreadyAnswered));
        assert_eq!(r.gate.answer(7, false), Err(Ignored::AlreadyAnswered));
    }

    #[test]
    fn the_first_answer_wins_between_cancel_and_confirm() {
        let r = rig();
        r.show(7);
        r.advance(1_000);
        assert_eq!(r.gate.answer(7, false), Ok(Event::Reject { id: 7 }));
        assert_eq!(r.gate.answer(7, true), Err(Ignored::AlreadyAnswered));
    }

    #[test]
    fn a_view_without_a_confirmation_closes_the_gate() {
        let r = rig();
        r.show(7);
        r.advance(1_000);
        r.gate.on_view(None);
        assert_eq!(r.gate.answer(7, true), Err(Ignored::NoConfirmation));
        assert_eq!(r.gate.answer(7, false), Err(Ignored::NoConfirmation));
    }

    // --- the answer through the real reducer ------------------------------------

    fn confirming() -> State {
        let mut state = State::default();
        for event in [
            Event::VoiceReadinessChanged { ready: true },
            Event::Shortcut,
        ] {
            state = reduce(state, event).0;
        }
        let id = state.listen_id.expect("a listen");
        state = reduce(
            state,
            Event::Transcript {
                id,
                text: "close spotify".into(),
                lang: None,
            },
        )
        .0;
        let req = state.request_id.expect("a request");
        reduce(
            state,
            Event::ToolCalls {
                req,
                verdict: ToolVerdict::Confirm {
                    prompt: "Close Spotify.exe?".into(),
                    click_only: false,
                    cancel_text: None,
                },
            },
        )
        .0
    }

    #[test]
    fn a_stale_or_wrong_id_changes_nothing_in_the_reducer() {
        let state = confirming();
        assert_eq!(state.phase, Phase::ConfirmationNeeded);
        let id = state.confirm_id.expect("a confirmation");
        for wrong in [0, id + 1, id.wrapping_sub(1)] {
            let (after, effects) = reduce(state.clone(), Event::Accept { id: wrong });
            assert_eq!(after, state, "Accept {wrong}");
            assert!(effects.is_empty());
            let (after, effects) = reduce(state.clone(), Event::Reject { id: wrong });
            assert_eq!(after, state, "Reject {wrong}");
            assert!(effects.is_empty());
        }
    }

    #[test]
    fn a_double_click_reaches_the_reducer_once_and_the_straggler_is_stale() {
        let r = rig();
        let mut state = confirming();
        let id = state.confirm_id.expect("a confirmation");
        r.show(id);
        r.advance(1_000);

        let first = r.gate.answer(id, true).expect("the first click is sent");
        assert_eq!(r.gate.answer(id, true), Err(Ignored::AlreadyAnswered));
        state = reduce(state, first).0;
        assert_ne!(state.phase, Phase::ConfirmationNeeded);

        // Even if a second Accept had been queued before the gate answered, the
        // reducer drops it: the confirmation it names is over.
        let (after, effects) = reduce(state.clone(), Event::Accept { id });
        assert_eq!(after, state);
        assert!(effects.is_empty());
    }

    // --- the buttons' rectangles -------------------------------------------------

    #[test]
    fn the_buttons_capture_nothing_until_they_are_reported() {
        let r = rig();
        r.gate.on_view(Some(7));
        assert!(r.gate.areas().is_empty());
        assert!(r.gate.report(7, WINDOW, &BUTTONS));
        assert_eq!(r.gate.areas(), BUTTONS.to_vec());
    }

    #[test]
    fn a_report_for_another_confirmation_is_dropped() {
        let r = rig();
        assert!(!r.gate.report(7, WINDOW, &BUTTONS), "nothing on screen");
        r.gate.on_view(Some(7));
        assert!(!r.gate.report(6, WINDOW, &BUTTONS));
        assert!(r.gate.areas().is_empty());
    }

    #[test]
    fn a_new_confirmation_forgets_the_old_rectangles() {
        let r = rig();
        r.show(1);
        r.gate.on_view(Some(2));
        assert!(r.gate.areas().is_empty(), "no capture over the old place");
    }

    #[test]
    fn the_rectangles_go_when_the_buttons_do() {
        let r = rig();
        r.show(1);
        r.gate.report(1, WINDOW, &BUTTONS);
        r.gate.on_view(None);
        assert!(r.gate.areas().is_empty());

        r.show(2);
        r.gate.clear();
        assert!(r.gate.areas().is_empty());
        assert_eq!(r.gate.answer(2, false), Err(Ignored::NoConfirmation));
    }

    #[test]
    fn the_rectangles_go_once_the_question_is_answered() {
        let r = rig();
        r.show(1);
        r.gate.report(1, WINDOW, &BUTTONS);
        r.advance(1_000);
        r.gate.answer(1, true).unwrap();
        assert!(r.gate.areas().is_empty());
    }

    #[test]
    fn an_empty_report_releases_the_capture() {
        let r = rig();
        r.show(1);
        r.gate.report(1, WINDOW, &BUTTONS);
        assert!(r.gate.report(1, WINDOW, &[]));
        assert!(r.gate.areas().is_empty());
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn reported_rectangles_are_clipped_to_the_window_and_empty_ones_dropped() {
        let ok = rect(10.0, 10.0, 50.0, 20.0);
        let clean = sanitize(WINDOW, &[ok, rect(700.0, 10.0, 20.0, 20.0)]);
        assert_eq!(clean, vec![ok]);
        assert_eq!(
            sanitize(
                WINDOW,
                &[rect(-5.0, 10.0, 20.0, 20.0), rect(0.0, 0.0, 0.0, 10.0)]
            ),
            vec![rect(0.0, 10.0, 15.0, 20.0)]
        );
        assert_eq!(
            sanitize(WINDOW, &[rect(620.0, 220.0, 100.0, 40.0)]),
            vec![rect(620.0, 220.0, 20.0, 12.0)]
        );
    }

    #[test]
    fn a_report_with_bad_numbers_is_rejected_as_a_whole() {
        let ok = rect(10.0, 10.0, 50.0, 20.0);
        for bad in [
            rect(f64::NAN, 0.0, 10.0, 10.0),
            rect(0.0, f64::INFINITY, 10.0, 10.0),
            rect(0.0, 0.0, f64::INFINITY, 10.0),
        ] {
            assert_eq!(sanitize(WINDOW, &[ok, bad]), vec![]);
        }
    }

    #[test]
    fn a_report_of_more_than_two_rectangles_is_rejected_as_a_whole() {
        let many: Vec<Rect> = (0..3)
            .map(|i| rect(f64::from(i) * 100.0, 0.0, 80.0, 32.0))
            .collect();
        assert_eq!(MAX_AREAS, 2);
        assert_eq!(sanitize(WINDOW, &many[..2]).len(), 2);
        assert_eq!(sanitize(WINDOW, &many), vec![]);
    }

    #[test]
    fn a_rectangle_bigger_than_a_button_rejects_the_whole_report() {
        let ok = rect(10.0, 10.0, MAX_AREA_WIDTH, MAX_AREA_HEIGHT);
        assert_eq!(sanitize(WINDOW, &[ok]), vec![ok]);
        for big in [
            rect(0.0, 0.0, MAX_AREA_WIDTH + 1.0, 10.0),
            rect(0.0, 0.0, 10.0, MAX_AREA_HEIGHT + 1.0),
            rect(0.0, 0.0, 640.0, 232.0),
        ] {
            assert_eq!(sanitize(WINDOW, &[ok, big]), vec![]);
        }
    }

    // --- the command -------------------------------------------------------------

    #[test]
    fn an_accepted_answer_sends_exactly_one_event_input() {
        let r = rig();
        let (tx, mut rx) = unbounded_channel::<crate::dispatcher::Input>();
        r.show(7);
        r.advance(1_000);
        answer(&r.gate, &tx, 7, true).unwrap();
        answer(&r.gate, &tx, 7, true).unwrap();
        match rx.try_recv() {
            Ok(crate::dispatcher::Input::Event(Event::Accept { id: 7 })) => {}
            Ok(_) => panic!("the island sent something other than Accept"),
            Err(error) => panic!("nothing was sent: {error:?}"),
        }
        assert!(rx.try_recv().is_err(), "the double click sent one input");
    }

    #[test]
    fn an_ignored_answer_sends_nothing_and_is_not_an_error() {
        let r = rig();
        let (tx, mut rx) = unbounded_channel::<crate::dispatcher::Input>();
        r.show(7);
        // Too soon, wrong id, nothing on screen: all silently dropped.
        answer(&r.gate, &tx, 7, true).unwrap();
        answer(&r.gate, &tx, 99, false).unwrap();
        r.gate.on_view(None);
        answer(&r.gate, &tx, 7, false).unwrap();
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_dispatcher_that_is_gone_is_an_error_not_a_panic() {
        let r = rig();
        let (tx, rx) = unbounded_channel::<crate::dispatcher::Input>();
        drop(rx);
        r.show(7);
        assert!(answer(&r.gate, &tx, 7, false).is_err());
    }

    // --- following the views ----------------------------------------------------

    #[derive(Default)]
    struct Recorder(Mutex<Vec<UiEvent>>);

    impl UiPort<UiEvent> for Recorder {
        fn publish(&self, event: UiEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn view(confirm: Option<u64>) -> UiEvent {
        UiEvent::View(ViewPayload {
            state: "confirmation-needed",
            caption: None,
            confirm: confirm.map(|id| ConfirmView {
                id,
                prompt: "Close Spotify.exe?".into(),
                confirm_label: "Confirm",
                cancel_label: "Cancel",
                expires_at: None,
                chord: false,
            }),
            reminder: None,
            countdown: None,
        })
    }

    #[test]
    fn the_gate_follows_the_views_and_forwards_every_event() {
        let r = rig();
        let inner = Arc::new(Recorder::default());
        let ui = GateUi::new(inner.clone(), r.gate.clone());

        ui.publish(UiEvent::Island { visible: true });
        ui.publish(view(Some(7)));
        r.advance(1_000);
        assert_eq!(r.gate.answer(7, false), Ok(Event::Reject { id: 7 }));

        ui.publish(view(Some(8)));
        r.advance(1_000);
        ui.publish(UiEvent::View(ViewPayload::idle()));
        assert_eq!(r.gate.answer(8, true), Err(Ignored::NoConfirmation));

        assert_eq!(inner.0.lock().unwrap().len(), 4, "nothing was swallowed");
    }

    #[test]
    fn hiding_the_island_closes_the_gate() {
        let r = rig();
        let ui = GateUi::new(Arc::new(Recorder::default()), r.gate.clone());
        ui.publish(view(Some(7)));
        r.gate.report(7, WINDOW, &BUTTONS);
        ui.publish(UiEvent::Island { visible: false });
        assert!(r.gate.areas().is_empty());
        r.advance(1_000);
        assert_eq!(r.gate.answer(7, true), Err(Ignored::NoConfirmation));
    }
}
