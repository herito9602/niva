//! What the notch can do over a fullscreen app (design L16, spec
//! `fullscreen-policy`).
//!
//! The shell tells which kind of fullscreen app is in front
//! (`SHQueryUserNotificationState`, the same call apps use to decide whether to
//! pop notifications). Two things follow from it:
//!
//! - A reminder that falls due over any fullscreen app is `silent`: the chime
//!   and the coral pulse, no speech and no text ([`reminder_due`]).
//! - Over an exclusive Direct3D fullscreen app the notch cannot be drawn
//!   ([`Fullscreen::Exclusive`]). Listening is announced by the host cue on every
//!   activation (design L17), so it needs no probe of its own.
//!
//! Whether a particular game is truly exclusive or composited (fullscreen
//! optimizations) can only be told by trying it: that stays a hand test.

use niva_core::state_machine::Event;

/// What is in front of the desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fullscreen {
    /// Nothing special, or the state could not be read (fail towards showing).
    No,
    /// A presentation or a fullscreen app that composes normally (a video
    /// player, a browser in F11): the notch can still be drawn over it.
    Windowed,
    /// Exclusive Direct3D fullscreen: the app owns the swap chain and nothing
    /// else can be drawn over it.
    Exclusive,
}

impl Fullscreen {
    /// A reminder must not speak (nor show text) over any fullscreen app.
    #[cfg_attr(not(test), allow(dead_code))] // until the reminder service calls it
    pub fn is_fullscreen(self) -> bool {
        self != Fullscreen::No
    }
}

/// Values of `QUERY_USER_NOTIFICATION_STATE` (shellapi.h).
const QUNS_BUSY: i32 = 2;
const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;
const QUNS_PRESENTATION_MODE: i32 = 4;

/// Maps the shell's notification state to what the notch can do.
pub fn classify(state: i32) -> Fullscreen {
    match state {
        QUNS_RUNNING_D3D_FULL_SCREEN => Fullscreen::Exclusive,
        QUNS_BUSY | QUNS_PRESENTATION_MODE => Fullscreen::Windowed,
        _ => Fullscreen::No,
    }
}

/// Where the host asks what is in front.
pub trait FullscreenProbe: Send + Sync {
    fn current(&self) -> Fullscreen;
}

/// The shell's answer on Windows.
#[cfg(windows)]
pub struct WindowsFullscreen;

#[cfg(windows)]
impl FullscreenProbe for WindowsFullscreen {
    fn current(&self) -> Fullscreen {
        use windows_sys::Win32::UI::Shell::SHQueryUserNotificationState;
        let mut state = 0;
        // SAFETY: `state` is a valid out pointer for the call's duration.
        let hr = unsafe { SHQueryUserNotificationState(&mut state) };
        if hr < 0 {
            Fullscreen::No
        } else {
            classify(state)
        }
    }
}

/// The event for `count` reminders falling due now: silent over a fullscreen
/// app (chime and pulse only), spoken otherwise.
#[cfg_attr(not(test), allow(dead_code))] // until the reminder service calls it
pub fn reminder_due(count: u32, probe: &dyn FullscreenProbe) -> Event {
    Event::ReminderDue {
        count,
        silent: probe.current().is_fullscreen(),
    }
}

#[cfg(test)]
mod tests {
    use niva_core::state_machine::{Effect, Phase, State, reduce};
    use serde_json::Value;

    use super::*;
    use crate::ui_adapter::{UiEvent, ViewModel};

    struct Fixed(Fullscreen);

    impl FullscreenProbe for Fixed {
        fn current(&self) -> Fullscreen {
            self.0
        }
    }

    #[test]
    fn the_shell_states_map_to_what_the_notch_can_do() {
        // 1 not present, 2 busy, 3 D3D fullscreen, 4 presentation, 5 accepts
        // notifications, 6 quiet time, 7 app.
        let got: Vec<Fullscreen> = (1..=7).map(classify).collect();
        assert_eq!(
            got,
            vec![
                Fullscreen::No,
                Fullscreen::Windowed,
                Fullscreen::Exclusive,
                Fullscreen::Windowed,
                Fullscreen::No,
                Fullscreen::No,
                Fullscreen::No,
            ]
        );
        assert_eq!(classify(0), Fullscreen::No, "unknown values show the notch");
    }

    fn deliver(event: Event) -> (State, Vec<Effect>, Vec<UiEvent>) {
        let (state, fx) = reduce(State::default(), event);
        let ui = ViewModel::new().update(&state, &fx, 0);
        (state, fx, ui)
    }

    fn view_json(ui: &[UiEvent]) -> Value {
        ui.iter()
            .find_map(|e| match e {
                UiEvent::View(v) => Some(serde_json::to_value(v).unwrap()),
                _ => None,
            })
            .expect("a view")
    }

    #[test]
    fn reminder_fires_with_chime_and_speaks_without_text() {
        let event = reminder_due(1, &Fixed(Fullscreen::No));
        let (state, fx, ui) = deliver(event);
        assert_eq!(state.phase, Phase::ReminderAlert);
        assert!(fx.iter().any(|e| matches!(e, Effect::Chime { .. })));
        assert!(fx.iter().any(|e| matches!(e, Effect::Speak { .. })));
        let view = view_json(&ui);
        assert_eq!(view["state"], "reminder-alert");
        assert_eq!(view["reminder"], serde_json::json!({ "count": 1 }));
        assert!(
            !view.to_string().contains(' '),
            "no sentence in the view: {view}"
        );
    }

    #[test]
    fn fullscreen_reminder_chime_no_speech_no_text() {
        for state in [Fullscreen::Windowed, Fullscreen::Exclusive] {
            let event = reminder_due(2, &Fixed(state));
            assert_eq!(
                event,
                Event::ReminderDue {
                    count: 2,
                    silent: true
                }
            );
            let (reduced, fx, ui) = deliver(event);
            assert_eq!(reduced.phase, Phase::ReminderAlert);
            assert!(fx.iter().any(|e| matches!(e, Effect::Chime { .. })));
            assert!(
                !fx.iter().any(|e| matches!(e, Effect::Speak { .. })),
                "{state:?}: a fullscreen reminder is never spoken"
            );
            let view = view_json(&ui);
            assert_eq!(view["speaking"], false);
            assert_eq!(view["reminder"], serde_json::json!({ "count": 2 }));
        }
    }
}
