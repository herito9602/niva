//! UI adapter: turns reducer output into the `niva://view` and `niva://island`
//! webview events (design: "Tauri events").
//!
//! [`ViewModel`] is the pure part: it folds each reduction (state plus effects)
//! into the events to publish. [`TauriUiAdapter`] is the thin `UiPort`
//! implementation that emits them.

use niva_core::replies::{self, Lang};
use niva_core::state_machine::{Effect, Phase, Say, State, Template, TimerKind};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter};

pub const VIEW_EVENT: &str = "niva://view";
pub const ISLAND_EVENT: &str = "niva://island";
pub const CURSOR_EVENT: &str = "niva://cursor";

pub const ISLAND_LABEL: &str = "island";
pub const DEBUG_LABEL: &str = "debug";

/// Windows that receive an event. Events are targeted (never app-wide): the
/// island gets everything; the debug window (debug builds only) mirrors the
/// view only.
pub fn event_targets(event_name: &str, debug_build: bool) -> Vec<&'static str> {
    let mut targets = vec![ISLAND_LABEL];
    if debug_build && event_name == VIEW_EVENT {
        targets.push(DEBUG_LABEL);
    }
    targets
}

/// English text of a reply template (used where the language does not matter,
/// e.g. to size the speak watchdog).
pub fn template_caption(template: Template) -> &'static str {
    replies::template(template, Lang::En)
}

pub(crate) fn say_text(say: &Say, lang: Lang) -> String {
    match say {
        Say::Text(text) => text.clone(),
        Say::Template(template) => replies::template(*template, lang).to_owned(),
    }
}

/// Name of a state in `niva://view`. The sleep countdown has a name of its own:
/// the notch shows it with the confirmation look and no number.
fn phase_name(phase: Phase) -> &'static str {
    match phase {
        Phase::Idle => "idle",
        Phase::Listening => "listening",
        Phase::Thinking => "thinking",
        Phase::Acting => "acting",
        Phase::ConfirmationNeeded => "confirmation-needed",
        Phase::Success => "success",
        Phase::Error => "error",
        Phase::ReminderAlert => "reminder-alert",
        Phase::Countdown => "countdown",
    }
}

/// The open confirmation. It carries no prompt and no labels: the notch draws
/// no text, and the question is spoken.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfirmView {
    /// Identity of this confirmation (`State::confirm_id`): the confirm keys
    /// answer with it, and an answer for another id is dropped.
    pub id: u64,
    /// Epoch milliseconds; absent until the confirmation timer starts
    /// (after `speak_done`).
    #[serde(rename = "expiresAt", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// The confirmation is answered by the Ctrl+Shift+Enter chord (restart and
    /// shutdown; `State::confirm_click_only`). Speech cannot confirm it.
    pub chord: bool,
}

/// The reminder alert: how many reminders merged into it, never their text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReminderView {
    pub count: u32,
}

/// Payload of `niva://view` (design L5): the state, whether Niva's voice is
/// audible, and the confirmation or reminder details. No reply text, no
/// labels, no countdown numbers.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewPayload {
    pub state: &'static str,
    /// Niva's voice is audible now (`State::audible_speak`). The notch shows the
    /// speaking look while this is true, over the phase look.
    pub speaking: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<ConfirmView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<ReminderView>,
}

impl ViewPayload {
    pub fn idle() -> Self {
        ViewPayload {
            state: phase_name(Phase::Idle),
            speaking: false,
            confirm: None,
            reminder: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    View(ViewPayload),
    Island { visible: bool },
}

impl UiEvent {
    pub fn name(&self) -> &'static str {
        match self {
            UiEvent::View(_) => VIEW_EVENT,
            UiEvent::Island { .. } => ISLAND_EVENT,
        }
    }

    pub fn payload(&self) -> Value {
        match self {
            UiEvent::View(view) => serde_json::to_value(view).unwrap_or(Value::Null),
            UiEvent::Island { visible } => json!({ "visible": visible }),
        }
    }
}

/// Where UI events go (design: `UiPort`). The trait lives in `niva-core`; the
/// host implements it for its own [`UiEvent`].
pub use niva_core::ports::UiPort;

/// Folds reducer output into UI events, publishing a view only when it changed.
pub struct ViewModel {
    phase: Phase,
    expires_at: Option<u64>,
    last: ViewPayload,
}

impl Default for ViewModel {
    fn default() -> Self {
        Self::new()
    }
}

impl ViewModel {
    pub fn new() -> Self {
        ViewModel {
            phase: Phase::Idle,
            expires_at: None,
            last: ViewPayload::idle(),
        }
    }

    /// `now_ms` (epoch ms) timestamps the confirmation deadline.
    ///
    /// Order: `Island{visible:true}` first, then the view, then
    /// `Island{visible:false}`, so the webview never animates a hidden island
    /// into a stale view.
    pub fn update(&mut self, state: &State, effects: &[Effect], now_ms: u64) -> Vec<UiEvent> {
        if state.phase != self.phase {
            self.phase = state.phase;
            self.expires_at = None;
        }
        let mut shown = Vec::new();
        let mut hidden = Vec::new();
        for effect in effects {
            match effect {
                Effect::StartTimer {
                    kind: TimerKind::Confirm,
                    ..
                } => {
                    let length = TimerKind::Confirm.duration().unwrap_or_default();
                    self.expires_at = Some(now_ms + length.as_millis() as u64);
                }
                Effect::ShowIsland => shown.push(UiEvent::Island { visible: true }),
                Effect::HideIsland => hidden.push(UiEvent::Island { visible: false }),
                _ => {}
            }
        }
        let view = self.payload(state);
        let mut out = shown;
        if view != self.last {
            self.last = view.clone();
            out.push(UiEvent::View(view));
        }
        out.extend(hidden);
        out
    }

    fn payload(&self, state: &State) -> ViewPayload {
        let confirm = (state.phase == Phase::ConfirmationNeeded).then(|| ConfirmView {
            id: state.confirm_id.unwrap_or(0),
            expires_at: self.expires_at,
            chord: state.confirm_click_only,
        });
        let reminder = (state.phase == Phase::ReminderAlert).then_some(ReminderView {
            count: state.alert_count,
        });
        ViewPayload {
            state: phase_name(state.phase),
            speaking: state.audible_speak.is_some(),
            confirm,
            reminder,
        }
    }
}

/// Emits one named event to one window (the Tauri side of [`TauriUiAdapter`]).
pub trait EmitTo: Send + Sync {
    fn emit_to(&self, label: &str, name: &str, payload: Value) -> Result<(), String>;
}

impl EmitTo for AppHandle {
    fn emit_to(&self, label: &str, name: &str, payload: Value) -> Result<(), String> {
        Emitter::emit_to(self, label, name, payload).map_err(|err| err.to_string())
    }
}

/// `UiPort` that emits each event to its target windows only.
pub struct TauriUiAdapter {
    emitter: Arc<dyn EmitTo>,
    debug_build: bool,
}

impl TauriUiAdapter {
    pub fn new(emitter: Arc<dyn EmitTo>, debug_build: bool) -> Self {
        TauriUiAdapter {
            emitter,
            debug_build,
        }
    }
}

impl UiPort<UiEvent> for TauriUiAdapter {
    fn publish(&self, event: UiEvent) {
        let payload = event.payload();
        // One failing window never blocks the others.
        for label in event_targets(event.name(), self.debug_build) {
            if let Err(err) = self.emitter.emit_to(label, event.name(), payload.clone()) {
                eprintln!("[niva] emit {} to {label} failed: {err}", event.name());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use niva_core::state_machine::{
        Event, PendingSlot, State, Template, ToolVerdict, ToolsOutcome, reduce,
    };
    use serde_json::json;

    use super::*;

    struct Sim {
        state: State,
        view: ViewModel,
    }

    impl Sim {
        fn new() -> Self {
            Sim {
                state: State::default(),
                view: ViewModel::new(),
            }
        }

        fn ready() -> Self {
            let mut sim = Sim::new();
            sim.send(Event::VoiceReadinessChanged { ready: true });
            sim
        }

        fn at(&mut self, now_ms: u64, event: Event) -> Vec<UiEvent> {
            let (state, fx) = reduce(self.state.clone(), event);
            self.state = state;
            self.view.update(&self.state, &fx, now_ms)
        }

        fn send(&mut self, event: Event) -> Vec<UiEvent> {
            self.at(0, event)
        }

        fn transcript(&mut self, text: &str) -> Vec<UiEvent> {
            let id = self.state.listen_id.expect("an open listen");
            self.send(Event::Transcript {
                id,
                text: text.into(),
                lang: None,
            })
        }

        /// Wake path: shortcut, transcript, then the given verdict.
        fn thinking(&mut self) {
            self.send(Event::Shortcut);
            self.transcript("do something");
        }

        fn req(&self) -> u64 {
            self.state.request_id.expect("a request")
        }
    }

    fn view_of(events: &[UiEvent]) -> &ViewPayload {
        events
            .iter()
            .find_map(|e| match e {
                UiEvent::View(v) => Some(v),
                _ => None,
            })
            .expect("a view event")
    }

    #[test]
    fn showing_the_island_publishes_island_first_then_the_view() {
        let mut sim = Sim::ready();
        let events = sim.send(Event::Shortcut);
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], UiEvent::Island { visible: true });
        let view = view_of(&events);
        assert_eq!(view.state, "listening");
        assert!(!view.speaking);
    }

    #[test]
    fn hiding_the_island_publishes_the_view_then_island_hidden() {
        let mut sim = Sim::ready();
        sim.send(Event::Shortcut);
        let events = sim.send(Event::Shortcut);
        assert_eq!(events.len(), 2);
        assert_eq!(view_of(&events).state, "idle");
        assert_eq!(events[1], UiEvent::Island { visible: false });
    }

    #[test]
    fn an_ignored_event_publishes_nothing() {
        let mut sim = Sim::new();
        assert_eq!(sim.send(Event::Accept { id: 1 }), vec![]);
        sim.send(Event::VoiceReadinessChanged { ready: true });
        sim.send(Event::Shortcut);
        // SpeechStart for the open listen changes no visible state.
        let id = sim.state.listen_id.unwrap();
        assert_eq!(sim.send(Event::SpeechStart { id }), vec![]);
    }

    #[test]
    fn every_phase_has_its_view_name() {
        let mut sim = Sim::ready();
        assert_eq!(view_of(&sim.send(Event::Shortcut)).state, "listening");
        assert_eq!(view_of(&sim.transcript("hi")).state, "thinking");
        let req = sim.req();
        let events = sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::RunSafe,
        });
        assert_eq!(view_of(&events).state, "acting");
        let events = sim.send(Event::ToolsDone {
            req,
            outcome: ToolsOutcome::Completed,
        });
        assert_eq!(view_of(&events).state, "success");

        let mut sim = Sim::new();
        assert_eq!(view_of(&sim.send(Event::Shortcut)).state, "error");
        let events = sim.send(Event::ReminderDue {
            count: 1,
            silent: false,
        });
        assert!(events.is_empty(), "queued while in error");
        let mut sim = Sim::new();
        let events = sim.send(Event::ReminderDue {
            count: 1,
            silent: false,
        });
        assert_eq!(view_of(&events).state, "reminder-alert");
    }

    #[test]
    fn confirmation_view_name_is_kebab_case() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        let events = sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Close Notepad?".into(),
                click_only: false,
                cancel_text: None,
            },
        });
        assert_eq!(view_of(&events).state, "confirmation-needed");
    }

    #[test]
    fn every_template_has_a_distinct_non_empty_caption() {
        let mut all = vec![
            Template::VoiceNotReady,
            Template::Error,
            Template::Cancelled,
            Template::ActionDone,
            Template::SearchUnavailable,
            Template::ReminderAlert,
            Template::SleepCountdown,
            Template::SleepCancelled,
            Template::SleepNotWatched,
        ];
        all.extend(niva_core::state_machine::SidecarErrorCode::ALL.map(Template::Sidecar));
        let captions: Vec<&str> = all.iter().map(|&t| template_caption(t)).collect();
        for (i, c) in captions.iter().enumerate() {
            assert!(!c.is_empty());
            assert!(!captions[..i].contains(c), "duplicate caption {c}");
        }
        assert_eq!(captions.len(), 15);
    }

    #[test]
    fn a_sidecar_error_while_listening_shows_the_error_view() {
        let mut sim = Sim::ready();
        sim.send(Event::Shortcut);
        let events = sim.send(Event::SidecarError {
            code: niva_core::state_machine::SidecarErrorCode::MicBusy,
            id: None,
        });
        assert_eq!(view_of(&events).state, "error");
    }

    #[test]
    fn a_follow_up_question_keeps_the_thinking_state() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        sim.send(Event::AskFollowUp {
            req,
            question: "At what time?".into(),
            slot: PendingSlot::Reminder { text: None },
        });
        assert_eq!(sim.view.last.state, "thinking");
    }

    #[test]
    fn the_confirmation_carries_its_id_and_no_prompt() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        let events = sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Close Notepad?".into(),
                click_only: false,
                cancel_text: None,
            },
        });
        let confirm = view_of(&events).confirm.clone().expect("confirm");
        assert_eq!(Some(confirm.id), sim.state.confirm_id);
        assert_ne!(confirm.id, req, "the confirmation has its own id");
        assert_eq!(confirm.expires_at, None, "no deadline before speak_done");
    }

    fn confirm_view_for(prompt: &str, click_only: bool) -> ConfirmView {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        let events = sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: prompt.into(),
                click_only,
                cancel_text: None,
            },
        });
        view_of(&events).confirm.clone().expect("confirm")
    }

    #[test]
    fn view_confirm_chord_true_for_power_confirmation() {
        let confirm = confirm_view_for("Restart the PC?", true);
        assert!(confirm.chord);
        assert_eq!(serde_json::to_value(&confirm).unwrap()["chord"], true);
    }

    #[test]
    fn view_confirm_chord_false_for_close() {
        let confirm = confirm_view_for("Close Notepad?", false);
        assert!(!confirm.chord);
        assert_eq!(serde_json::to_value(&confirm).unwrap()["chord"], false);
    }

    #[test]
    fn the_confirmation_deadline_appears_when_the_confirm_timer_starts() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Close Notepad?".into(),
                click_only: false,
                cancel_text: None,
            },
        });
        let speak_id = sim.state.speak_id.unwrap();
        let events = sim.at(1_000, Event::SpeakDone { id: speak_id });
        let confirm = view_of(&events).confirm.clone().expect("confirm");
        assert_eq!(confirm.expires_at, Some(11_000));
    }

    #[test]
    fn the_confirmation_is_cleared_when_the_state_leaves() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Close Notepad?".into(),
                click_only: true,
                cancel_text: None,
            },
        });
        let id = sim.state.confirm_id.expect("a confirmation");
        let events = sim.send(Event::Accept { id });
        let view = view_of(&events);
        assert_eq!(view.state, "acting");
        assert_eq!(view.confirm, None);
    }

    #[test]
    fn reminder_view_carries_count_only() {
        let mut sim = Sim::new();
        let events = sim.send(Event::ReminderDue {
            count: 1,
            silent: false,
        });
        let reminder = view_of(&events).reminder.clone().unwrap();
        assert_eq!(reminder, ReminderView { count: 1 });
        let events = sim.send(Event::ReminderDue {
            count: 2,
            silent: false,
        });
        let reminder = view_of(&events).reminder.clone().unwrap();
        assert_eq!(reminder.count, 3, "merged into the alert on screen");
        assert_eq!(
            serde_json::to_value(&reminder).unwrap(),
            json!({ "count": 3 })
        );
    }

    #[test]
    fn view_json_matches_the_documented_shape() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        let events = sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Close Notepad?".into(),
                click_only: false,
                cancel_text: None,
            },
        });
        let confirm_id = sim.state.confirm_id.unwrap();
        let speak_id = sim.state.speak_id.unwrap();
        let events2 = sim.at(500, Event::SpeakDone { id: speak_id });
        assert_eq!(
            serde_json::to_value(view_of(&events2)).unwrap(),
            json!({
                "state": "confirmation-needed",
                "speaking": false,
                "confirm": { "id": confirm_id, "expiresAt": 10_500, "chord": false },
            })
        );
        // Optional fields are omitted, not null.
        assert_eq!(
            serde_json::to_value(view_of(&events)).unwrap(),
            json!({
                "state": "confirmation-needed",
                "speaking": false,
                "confirm": { "id": confirm_id, "chord": false },
            })
        );
    }

    // --- sleep countdown (owner decision 2026-11-04) ---------------------------

    /// Drives the reducer to an accepted sleep (`Countdown`, host not ready yet).
    fn accepted_sleep(sim: &mut Sim) {
        sim.thinking();
        let req = sim.req();
        sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Put the PC to sleep?".into(),
                click_only: false,
                cancel_text: None,
            },
        });
        let speak = sim.state.speak_id.expect("the prompt is playing");
        sim.send(Event::SpeakDone { id: speak });
        let confirm = sim.state.confirm_id.expect("a confirmation");
        sim.send(Event::Accept { id: confirm });
        sim.send(Event::ToolsDone {
            req,
            outcome: ToolsOutcome::SleepCountdown { cancel_text: None },
        });
    }

    fn countdown_id(sim: &Sim) -> u64 {
        sim.state.countdown_id.expect("a countdown")
    }

    #[test]
    fn countdown_phase_serializes_as_countdown() {
        let mut sim = Sim::ready();
        accepted_sleep(&mut sim);
        assert_eq!(sim.state.phase, Phase::Countdown);
        assert_eq!(phase_name(Phase::Countdown), "countdown");
        assert_eq!(sim.view.last.state, "countdown");
        // No number, no deadline, no text: the notch pulses amber and Niva speaks.
        assert_eq!(
            serde_json::to_value(&sim.view.last).unwrap(),
            json!({ "state": "countdown", "speaking": false })
        );
        let id = countdown_id(&sim);
        let events = sim.at(5_000, Event::CountdownReady { id });
        assert!(events.is_empty(), "arming the deadline changes no view");
    }

    #[test]
    fn the_countdown_leaves_the_view_when_it_is_cancelled_or_finished() {
        let mut sim = Sim::ready();
        accepted_sleep(&mut sim);
        let id = countdown_id(&sim);
        sim.send(Event::CountdownReady { id });
        let events = sim.send(Event::CountdownCancel { id });
        assert_eq!(view_of(&events).state, "success");

        let mut sim = Sim::ready();
        accepted_sleep(&mut sim);
        let id = countdown_id(&sim);
        sim.send(Event::CountdownReady { id });
        let token = sim
            .state
            .timers
            .iter()
            .find(|t| t.kind == TimerKind::Countdown)
            .expect("the deadline")
            .token;
        let events = sim.send(Event::Watchdog {
            kind: TimerKind::Countdown,
            token,
        });
        assert_eq!(view_of(&events).state, "acting");
    }

    // --- speaking flag and the trimmed payload (design L5, L6) -----------------

    /// A reply being spoken: returns the speak id.
    fn replying(sim: &mut Sim) -> u64 {
        sim.thinking();
        let req = sim.req();
        sim.send(Event::Reply {
            req,
            text: "Opening Notepad".into(),
        });
        sim.state.speak_id.expect("the reply is being synthesized")
    }

    #[test]
    fn view_not_speaking_during_synthesis() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        let events = sim.send(Event::Reply {
            req,
            text: "Opening Notepad".into(),
        });
        let view = view_of(&events);
        assert_eq!(view.state, "success");
        assert!(!view.speaking, "the voice is not audible yet");
        assert_eq!(sim.state.audible_speak, None);
    }

    #[test]
    fn view_speaking_true_only_while_audible_speak_set() {
        let mut sim = Sim::ready();
        let speak = replying(&mut sim);
        // First sound: the flag goes up (a view of its own).
        let events = sim.send(Event::SpeakStarted { id: speak });
        let view = view_of(&events);
        assert!(view.speaking);
        assert_eq!(view.state, "success", "the phase is unchanged");
        // A start for another speech changes nothing.
        assert_eq!(sim.send(Event::SpeakStarted { id: speak + 1 }), vec![]);
        // Speech ends: the flag drops and the phase look returns.
        let events = sim.send(Event::SpeakDone { id: speak });
        let view = view_of(&events);
        assert!(!view.speaking);
        assert_eq!(view.state, "success");
        assert_eq!(
            serde_json::to_value(view).unwrap()["speaking"],
            json!(false)
        );
    }

    #[test]
    fn speaking_drops_when_the_speech_is_cut_off() {
        let mut sim = Sim::ready();
        let speak = replying(&mut sim);
        sim.send(Event::SpeakStarted { id: speak });
        assert!(sim.view.last.speaking);
        // Barge-in with the shortcut: the speech is stopped and listening starts.
        let events = sim.send(Event::Shortcut);
        let view = view_of(&events);
        assert_eq!(view.state, "listening");
        assert!(!view.speaking);
    }

    #[test]
    fn speaking_overrides_the_reminder_pulse_in_the_view() {
        let mut sim = Sim::new();
        let events = sim.send(Event::ReminderDue {
            count: 1,
            silent: false,
        });
        assert!(!view_of(&events).speaking);
        let speak = sim.state.speak_id.expect("the reminder is spoken");
        let events = sim.send(Event::SpeakStarted { id: speak });
        let view = view_of(&events);
        assert_eq!(view.state, "reminder-alert");
        assert!(view.speaking, "the speaking look overrides the pulse");
        assert_eq!(view.reminder, Some(ReminderView { count: 1 }));
    }

    /// Keys that would carry text or labels to the island.
    const TEXT_KEYS: [&str; 6] = [
        "caption",
        "prompt",
        "text",
        "confirmLabel",
        "cancelLabel",
        "countdown",
    ];

    fn assert_no_text(value: &Value) {
        match value {
            Value::Object(map) => {
                for (key, inner) in map {
                    assert!(!TEXT_KEYS.contains(&key.as_str()), "text key {key}");
                    assert_no_text(inner);
                }
            }
            Value::Array(items) => items.iter().for_each(assert_no_text),
            Value::String(text) => {
                assert!(
                    text.len() < 24 && !text.contains(' '),
                    "a sentence in the payload: {text}"
                );
            }
            _ => {}
        }
    }

    #[test]
    fn view_payload_has_no_reply_text_keys() {
        let mut sim = Sim::ready();
        let mut seen: Vec<ViewPayload> = Vec::new();
        let mut keep = |events: Vec<UiEvent>| {
            for e in events {
                if let UiEvent::View(v) = e {
                    seen.push(v);
                }
            }
        };
        keep(sim.send(Event::Shortcut));
        let id = sim.state.listen_id.unwrap();
        keep(sim.send(Event::Transcript {
            id,
            text: "close notepad".into(),
            lang: None,
        }));
        let req = sim.req();
        keep(sim.send(Event::ToolCalls {
            req,
            verdict: ToolVerdict::Confirm {
                prompt: "Close Notepad now?".into(),
                click_only: true,
                cancel_text: None,
            },
        }));
        let speak = sim.state.speak_id.unwrap();
        keep(sim.send(Event::SpeakStarted { id: speak }));
        keep(sim.at(500, Event::SpeakDone { id: speak }));
        let confirm = sim.state.confirm_id.unwrap();
        keep(sim.send(Event::Accept { id: confirm }));
        keep(sim.send(Event::ToolsDone {
            req,
            outcome: ToolsOutcome::Completed,
        }));
        let mut alert = Sim::new();
        keep(alert.send(Event::ReminderDue {
            count: 2,
            silent: false,
        }));
        let mut countdown = Sim::ready();
        accepted_sleep(&mut countdown);
        let cid = countdown_id(&countdown);
        keep(countdown.send(Event::CountdownReady { id: cid }));
        seen.push(countdown.view.last.clone());
        assert!(seen.len() >= 6, "{seen:?}");
        for view in seen {
            assert_no_text(&serde_json::to_value(&view).unwrap());
        }
    }

    /// The view has no caption any more, so the tests do not pin one.
    #[test]
    fn ui_adapter_tests_no_longer_pin_captions() {
        let source = include_str!("ui_adapter.rs");
        let tests = &source[source.find("mod tests {").unwrap()..];
        // The needle below is the only mention: any other would pin a caption.
        assert_eq!(tests.matches(".caption").count(), 1);
    }

    #[test]
    fn event_names_and_island_payload() {
        assert_eq!(UiEvent::Island { visible: true }.name(), "niva://island");
        assert_eq!(UiEvent::View(ViewPayload::idle()).name(), "niva://view");
        assert_eq!(
            UiEvent::Island { visible: false }.payload(),
            json!({"visible": false})
        );
        assert_eq!(
            UiEvent::View(ViewPayload::idle()).payload(),
            json!({"state": "idle", "speaking": false})
        );
    }

    #[derive(Default)]
    struct FakeEmitter {
        sent: std::sync::Mutex<Vec<(String, String, Value)>>,
        fail_for: Option<&'static str>,
    }

    impl EmitTo for FakeEmitter {
        fn emit_to(&self, label: &str, name: &str, payload: Value) -> Result<(), String> {
            if self.fail_for == Some(label) {
                return Err("gone".into());
            }
            self.sent
                .lock()
                .unwrap()
                .push((label.into(), name.into(), payload));
            Ok(())
        }
    }

    fn adapter(emitter: &Arc<FakeEmitter>, debug_build: bool) -> TauriUiAdapter {
        TauriUiAdapter::new(emitter.clone(), debug_build)
    }

    #[test]
    fn targets_are_the_island_plus_the_debug_window_for_views_in_debug_builds_only() {
        assert_eq!(event_targets(VIEW_EVENT, false), vec!["island"]);
        assert_eq!(event_targets(VIEW_EVENT, true), vec!["island", "debug"]);
        assert_eq!(event_targets(ISLAND_EVENT, true), vec!["island"]);
        assert_eq!(event_targets(CURSOR_EVENT, true), vec!["island"]);
    }

    #[test]
    fn the_adapter_emits_each_event_to_its_targets_with_name_and_payload() {
        let emitter = Arc::new(FakeEmitter::default());
        let ui = adapter(&emitter, true);
        ui.publish(UiEvent::View(ViewPayload::idle()));
        ui.publish(UiEvent::Island { visible: true });
        let sent = emitter.sent.lock().unwrap().clone();
        assert_eq!(
            sent,
            vec![
                (
                    "island".into(),
                    VIEW_EVENT.into(),
                    json!({"state": "idle", "speaking": false})
                ),
                (
                    "debug".into(),
                    VIEW_EVENT.into(),
                    json!({"state": "idle", "speaking": false})
                ),
                (
                    "island".into(),
                    ISLAND_EVENT.into(),
                    json!({"visible": true})
                ),
            ]
        );
    }

    #[test]
    fn in_release_builds_the_debug_window_never_receives_anything() {
        let emitter = Arc::new(FakeEmitter::default());
        let ui = adapter(&emitter, false);
        ui.publish(UiEvent::View(ViewPayload::idle()));
        let sent = emitter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "island");
    }

    #[test]
    fn a_failing_window_does_not_block_the_others_or_panic() {
        let emitter = Arc::new(FakeEmitter {
            fail_for: Some("island"),
            ..FakeEmitter::default()
        });
        adapter(&emitter, true).publish(UiEvent::View(ViewPayload::idle()));
        let sent = emitter.sent.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].0, "debug");
    }
}
