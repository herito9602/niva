//! UI adapter: turns reducer output into the `niva://view` and `niva://island`
//! webview events (design: "Tauri events").
//!
//! [`ViewModel`] is the pure part: it folds each reduction (state plus effects)
//! into the events to publish. [`TauriUiAdapter`] is the thin `UiPort`
//! implementation that emits them.

use niva_core::replies::{self, Lang};
use niva_core::state_machine::{Effect, Phase, ReplyKind, Say, State, Template, TimerKind};
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

fn say_text(say: &Say, lang: Lang) -> String {
    match say {
        Say::Text(text) => text.clone(),
        Say::Template(template) => replies::template(*template, lang).to_owned(),
    }
}

/// Name of a state in `niva://view` (the mascot spec's state names). The sleep
/// countdown is shown with the `acting` pose: the view carries a `countdown`
/// next to it, and the mascot needs no ninth look.
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
        Phase::Countdown => "acting",
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConfirmView {
    /// Identity of this confirmation (`State::confirm_id`): the buttons send it
    /// back with their answer, and a click for another id is dropped.
    pub id: u64,
    pub prompt: String,
    /// Texts of the island's two buttons, in the language of the request.
    #[serde(rename = "confirmLabel")]
    pub confirm_label: &'static str,
    #[serde(rename = "cancelLabel")]
    pub cancel_label: &'static str,
    /// Epoch milliseconds; absent until the confirmation timer starts
    /// (after `speak_done`).
    #[serde(rename = "expiresAt", skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

/// The sleep countdown the island shows. It counts down from `ends_at` in the
/// view; the reducer sends no event per second.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CountdownView {
    /// Epoch milliseconds when the countdown ends.
    #[serde(rename = "endsAt")]
    pub ends_at: u64,
    /// Length of the countdown in seconds (the widest number the text shows).
    pub seconds: u64,
    /// The island text, in the request's language; `{n}` stands for the whole
    /// seconds left.
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReminderView {
    pub id: u64,
    pub text: String,
}

/// Payload of `niva://view`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ViewPayload {
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<ConfirmView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder: Option<ReminderView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub countdown: Option<CountdownView>,
}

impl ViewPayload {
    pub fn idle() -> Self {
        ViewPayload {
            state: phase_name(Phase::Idle),
            caption: None,
            confirm: None,
            reminder: None,
            countdown: None,
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
    lang: Lang,
    phase: Phase,
    caption: Option<String>,
    confirm_prompt: Option<String>,
    expires_at: Option<u64>,
    /// Epoch ms when the sleep countdown ends; set when its timer starts.
    countdown_ends_at: Option<u64>,
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
            lang: Lang::default(),
            phase: Phase::Idle,
            caption: None,
            confirm_prompt: None,
            expires_at: None,
            countdown_ends_at: None,
            last: ViewPayload::idle(),
        }
    }

    /// Language the reducer's reply templates are shown in (the request's).
    pub fn set_lang(&mut self, lang: Lang) {
        self.lang = lang;
    }

    /// `now_ms` (epoch ms) timestamps the confirmation deadline.
    ///
    /// Order: `Island{visible:true}` first, then the view, then
    /// `Island{visible:false}`, so the webview never animates a hidden island
    /// into a stale view.
    pub fn update(&mut self, state: &State, effects: &[Effect], now_ms: u64) -> Vec<UiEvent> {
        if state.phase != self.phase {
            // The question of a follow-up turn stays on screen while Niva
            // listens for the answer.
            let follow_up_listen = self.phase == Phase::Thinking
                && state.phase == Phase::Listening
                && state.pending.is_some();
            self.phase = state.phase;
            if !follow_up_listen {
                self.caption = None;
            }
            self.confirm_prompt = None;
            self.expires_at = None;
            self.countdown_ends_at = None;
        }
        let mut shown = Vec::new();
        let mut hidden = Vec::new();
        for effect in effects {
            match effect {
                Effect::Speak { kind, say, .. } => {
                    let text = say_text(say, self.lang);
                    if *kind == ReplyKind::ConfirmPrompt {
                        self.confirm_prompt = Some(text.clone());
                        self.expires_at = None;
                    }
                    self.caption = Some(text);
                }
                Effect::StartTimer {
                    kind: TimerKind::Confirm,
                    ..
                } => {
                    let length = TimerKind::Confirm.duration().unwrap_or_default();
                    self.expires_at = Some(now_ms + length.as_millis() as u64);
                }
                Effect::StartTimer {
                    kind: TimerKind::Countdown,
                    ..
                } => {
                    let length = TimerKind::Countdown.duration().unwrap_or_default();
                    self.countdown_ends_at = Some(now_ms + length.as_millis() as u64);
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
        let confirm = match (state.phase, &self.confirm_prompt) {
            (Phase::ConfirmationNeeded, Some(prompt)) => {
                let (confirm_label, cancel_label) = replies::confirm_labels(self.lang);
                Some(ConfirmView {
                    id: state.confirm_id.unwrap_or(0),
                    prompt: prompt.clone(),
                    confirm_label,
                    cancel_label,
                    expires_at: self.expires_at,
                })
            }
            _ => None,
        };
        // Stub: reminder texts arrive with the reminder service (slice 4).
        let reminder = (state.phase == Phase::ReminderAlert).then(|| ReminderView {
            id: 0,
            text: match state.alert_count {
                1 => "1 reminder due".to_owned(),
                n => format!("{n} reminders due"),
            },
        });
        let countdown = match (state.phase, self.countdown_ends_at) {
            (Phase::Countdown, Some(ends_at)) => Some(CountdownView {
                ends_at,
                seconds: TimerKind::Countdown
                    .duration()
                    .unwrap_or_default()
                    .as_secs(),
                text: replies::countdown_display(self.lang).to_owned(),
            }),
            _ => None,
        };
        ViewPayload {
            state: phase_name(state.phase),
            caption: self.caption.clone(),
            confirm,
            reminder,
            countdown,
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
        assert_eq!(view.caption, None);
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
    fn a_text_reply_becomes_the_caption() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        let events = sim.send(Event::Reply {
            req,
            text: "Opening Notepad".into(),
        });
        let view = view_of(&events);
        assert_eq!(view.state, "success");
        assert_eq!(view.caption.as_deref(), Some("Opening Notepad"));
    }

    #[test]
    fn a_template_speak_becomes_its_english_caption() {
        let mut sim = Sim::new();
        let events = sim.send(Event::Shortcut);
        assert_eq!(
            view_of(&events).caption.as_deref(),
            Some("Voice is not ready. Type a command instead.")
        );
    }

    #[test]
    fn the_follow_up_question_stays_visible_while_listening_for_the_answer() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        sim.send(Event::AskFollowUp {
            req,
            question: "At what time?".into(),
            slot: PendingSlot::Reminder { text: None },
        });
        let speak = sim.state.speak_id.expect("the question is playing");
        let events = sim.send(Event::SpeakDone { id: speak });
        let view = view_of(&events);
        assert_eq!(view.state, "listening");
        assert_eq!(view.caption.as_deref(), Some("At what time?"));
    }

    #[test]
    fn templates_are_shown_in_the_language_the_view_model_is_given() {
        let mut sim = Sim::new();
        sim.view.set_lang(Lang::Es);
        let events = sim.send(Event::Shortcut);
        assert_eq!(
            view_of(&events).caption.as_deref(),
            Some("La voz no está lista. Escribe un comando.")
        );
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
    fn a_sidecar_error_while_listening_is_shown_in_the_request_language() {
        for (lang, text) in [
            (
                Lang::Es,
                "Otra app está usando el micrófono. Escribe un comando.",
            ),
            (
                Lang::En,
                "The microphone is in use by another app. Type a command instead.",
            ),
        ] {
            let mut sim = Sim::ready();
            sim.view.set_lang(lang);
            sim.send(Event::Shortcut);
            let events = sim.send(Event::SidecarError {
                code: niva_core::state_machine::SidecarErrorCode::MicBusy,
                id: None,
            });
            let view = view_of(&events);
            assert_eq!(view.state, "error");
            assert_eq!(view.caption.as_deref(), Some(text));
        }
    }

    #[test]
    fn the_caption_is_cleared_when_the_phase_changes() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        sim.send(Event::Reply {
            req,
            text: "Done".into(),
        });
        // Barge-in: success -> listening drops the old caption.
        let events = sim.send(Event::Shortcut);
        let view = view_of(&events);
        assert_eq!(view.state, "listening");
        assert_eq!(view.caption, None);
    }

    #[test]
    fn a_follow_up_question_is_the_caption_while_the_state_stays_thinking() {
        let mut sim = Sim::ready();
        sim.thinking();
        let req = sim.req();
        let events = sim.send(Event::AskFollowUp {
            req,
            question: "At what time?".into(),
            slot: PendingSlot::Reminder { text: None },
        });
        let view = view_of(&events);
        assert_eq!(view.state, "thinking");
        assert_eq!(view.caption.as_deref(), Some("At what time?"));
    }

    #[test]
    fn the_confirmation_carries_the_prompt_and_the_confirmation_id() {
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
        assert_eq!(confirm.prompt, "Close Notepad?");
        assert_eq!(confirm.expires_at, None, "no deadline before speak_done");
    }

    #[test]
    fn the_buttons_are_labelled_in_the_language_of_the_request() {
        for (lang, confirm, cancel) in [
            (Lang::En, "Confirm", "Cancel"),
            (Lang::Es, "Confirmar", "Cancelar"),
        ] {
            let mut sim = Sim::ready();
            sim.view.set_lang(lang);
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
            let shown = view_of(&events).confirm.clone().expect("confirm");
            assert_eq!((shown.confirm_label, shown.cancel_label), (confirm, cancel));
        }
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
        assert_eq!(confirm.prompt, "Close Notepad?");
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
    fn the_alert_carries_a_reminder_summary() {
        let mut sim = Sim::new();
        let events = sim.send(Event::ReminderDue {
            count: 1,
            silent: false,
        });
        assert_eq!(
            view_of(&events).reminder.clone().unwrap().text,
            "1 reminder due"
        );
        let events = sim.send(Event::ReminderDue {
            count: 2,
            silent: false,
        });
        assert_eq!(
            view_of(&events).reminder.clone().unwrap().text,
            "3 reminders due"
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
                "caption": "Close Notepad?",
                "confirm": {
                    "id": confirm_id,
                    "prompt": "Close Notepad?",
                    "confirmLabel": "Confirm",
                    "cancelLabel": "Cancel",
                    "expiresAt": 10_500,
                },
            })
        );
        // Optional fields are omitted, not null.
        assert_eq!(
            serde_json::to_value(view_of(&events)).unwrap(),
            json!({
                "state": "confirmation-needed",
                "caption": "Close Notepad?",
                "confirm": {
                    "id": confirm_id,
                    "prompt": "Close Notepad?",
                    "confirmLabel": "Confirm",
                    "cancelLabel": "Cancel",
                },
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
    fn the_countdown_phase_is_shown_with_the_acting_pose() {
        let mut sim = Sim::ready();
        accepted_sleep(&mut sim);
        assert_eq!(sim.state.phase, Phase::Countdown);
        assert_eq!(phase_name(Phase::Countdown), "acting");
    }

    #[test]
    fn the_view_carries_no_countdown_until_the_host_is_watching() {
        let mut sim = Sim::ready();
        accepted_sleep(&mut sim);
        let events = sim.at(1_000, Event::Wake);
        assert!(events.is_empty());
        assert_eq!(sim.view.last.countdown, None);
        assert_eq!(sim.view.last.state, "acting");
    }

    #[test]
    fn the_deadline_is_published_when_the_deadline_timer_starts() {
        let mut sim = Sim::ready();
        accepted_sleep(&mut sim);
        let id = countdown_id(&sim);
        let events = sim.at(5_000, Event::CountdownReady { id });
        let view = view_of(&events);
        assert_eq!(view.state, "acting");
        assert_eq!(
            view.countdown,
            Some(CountdownView {
                ends_at: 15_000,
                seconds: 10,
                text: "Suspending in {n} s. Press a key or move the mouse to cancel.".into(),
            })
        );
        assert_eq!(
            view.caption.as_deref(),
            Some("Suspending in 10 seconds. Press a key or move the mouse to cancel.")
        );
    }

    #[test]
    fn the_countdown_text_follows_the_request_language() {
        let mut sim = Sim::ready();
        sim.view.set_lang(Lang::Es);
        accepted_sleep(&mut sim);
        let id = countdown_id(&sim);
        let events = sim.at(0, Event::CountdownReady { id });
        let view = view_of(&events);
        assert_eq!(
            view.countdown.clone().unwrap().text,
            "Suspendiendo en {n} s. Presiona una tecla o mueve el mouse para cancelar."
        );
        assert_eq!(
            view.caption.as_deref(),
            Some("Suspendiendo en 10 segundos. Presiona una tecla o mueve el mouse para cancelar.")
        );
    }

    #[test]
    fn the_countdown_json_matches_the_documented_shape() {
        let mut sim = Sim::ready();
        accepted_sleep(&mut sim);
        let id = countdown_id(&sim);
        let events = sim.at(2_000, Event::CountdownReady { id });
        assert_eq!(
            serde_json::to_value(view_of(&events)).unwrap(),
            json!({
                "state": "acting",
                "caption": "Suspending in 10 seconds. Press a key or move the mouse to cancel.",
                "countdown": {
                    "endsAt": 12_000,
                    "seconds": 10,
                    "text": "Suspending in {n} s. Press a key or move the mouse to cancel.",
                },
            })
        );
    }

    #[test]
    fn the_countdown_leaves_the_view_when_it_is_cancelled_or_finished() {
        let mut sim = Sim::ready();
        accepted_sleep(&mut sim);
        let id = countdown_id(&sim);
        sim.send(Event::CountdownReady { id });
        let events = sim.send(Event::CountdownCancel { id });
        let view = view_of(&events);
        assert_eq!(view.state, "success");
        assert_eq!(view.countdown, None);
        assert_eq!(view.caption.as_deref(), Some("Suspend cancelled."));

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
        let view = view_of(&events);
        assert_eq!(view.state, "acting");
        assert_eq!(view.countdown, None);
        assert_eq!(view.caption, None);
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
            json!({"state": "idle"})
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
                ("island".into(), VIEW_EVENT.into(), json!({"state": "idle"})),
                ("debug".into(), VIEW_EVENT.into(), json!({"state": "idle"})),
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
