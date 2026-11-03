//! Reducer: `reduce(state, event) -> (state, effects)`.
//!
//! Pure and synchronous: no I/O, no clock, no async runtime. Time reaches the
//! reducer only as `Event::Watchdog` events, and everything it wants done in
//! the outside world leaves as `Effect` values for the host effect runner.
//!
//! The contract is the "Transition matrix (event x state)" in
//! `openspec/changes/niva-mvp/design.md`.

use std::time::Duration;

#[cfg(test)]
mod coverage;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_activation;
#[cfg(test)]
mod tests_confirm;
#[cfg(test)]
mod tests_input;
#[cfg(test)]
mod tests_races;
#[cfg(test)]
mod tests_scenarios;
#[cfg(test)]
mod tests_speech;
#[cfg(test)]
mod tests_tools;

/// The 8 assistant states. They map 1:1 to the 8 mascot poses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Listening,
    Thinking,
    Acting,
    ConfirmationNeeded,
    Success,
    Error,
    ReminderAlert,
}

impl Phase {
    pub const ALL: [Phase; 8] = [
        Phase::Idle,
        Phase::Listening,
        Phase::Thinking,
        Phase::Acting,
        Phase::ConfirmationNeeded,
        Phase::Success,
        Phase::Error,
        Phase::ReminderAlert,
    ];

    /// Column of this state in the transition matrix. The match is exhaustive
    /// on purpose: adding a state breaks the build until the matrix is
    /// extended.
    pub const fn index(self) -> usize {
        match self {
            Phase::Idle => 0,
            Phase::Listening => 1,
            Phase::Thinking => 2,
            Phase::Acting => 3,
            Phase::ConfirmationNeeded => 4,
            Phase::Success => 5,
            Phase::Error => 6,
            Phase::ReminderAlert => 7,
        }
    }
}

/// One-turn slot holding a partial request while the user answers a question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingSlot {
    /// A reminder that still lacks its time (or text).
    Reminder { text: Option<String> },
    /// A `close_app` request with several matching executables.
    Close { candidates: Vec<String> },
}

/// How far the current listen got, for the 6 s / 15 s / STT timers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Utterance {
    /// Listening, no `speech_start` yet (6 s no-speech timer).
    WaitingForSpeech,
    /// `speech_start` seen (15 s max-utterance cap).
    Speaking,
    /// End of speech was forced, waiting for the STT result.
    Ending,
}

/// Reducer state. All fields are plain data so the host can derive views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct State {
    pub phase: Phase,
    /// Set only by `Event::VoiceReadinessChanged`. Defaults to `false`.
    pub voice_available: bool,
    pub pending: Option<PendingSlot>,
    /// Id of the open sidecar listen (command or confirm).
    pub listen_id: Option<u64>,
    /// Id of the `Speak` currently in flight and its reply kind.
    pub speak_id: Option<u64>,
    pub speak_kind: Option<ReplyKind>,
    /// Id of the LLM/tool request in flight (late results are dropped by it).
    pub request_id: Option<u64>,
    pub utterance: Utterance,
    /// The in-flight request is the one-turn follow-up of a pending slot.
    pub followup_turn: bool,
    /// The in-flight LLM call is the no-tools `web_search` summary.
    pub summarizing: bool,
    /// The open confirmation has no voice channel (power action or voice down).
    pub confirm_click_only: bool,
    /// Reminders merged into the current alert.
    pub alert_count: u32,
    /// Id of the alert chime while it is still playing (alert only).
    pub chime_id: Option<u64>,
    /// Timers started by the reducer and not yet fired or cancelled.
    pub timers: Vec<Timer>,
    pub(crate) next_id: u64,
    /// Next timer token; every `StartTimer` gets a fresh one.
    pub(crate) next_token: u64,
}

/// A running reducer-owned timer: its kind and the token issued by its
/// `StartTimer`. The runner echoes the token in `Event::Watchdog`; a fire whose
/// token is not the running one belongs to an earlier instance and is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timer {
    pub kind: TimerKind,
    pub token: u64,
}

impl PartialEq<TimerKind> for Timer {
    fn eq(&self, kind: &TimerKind) -> bool {
        self.kind == *kind
    }
}

impl Default for State {
    fn default() -> Self {
        State {
            phase: Phase::Idle,
            voice_available: false,
            pending: None,
            listen_id: None,
            speak_id: None,
            speak_kind: None,
            request_id: None,
            utterance: Utterance::WaitingForSpeech,
            followup_turn: false,
            summarizing: false,
            confirm_click_only: false,
            alert_count: 0,
            chime_id: None,
            timers: Vec::new(),
            next_id: 1,
            next_token: 1,
        }
    }
}

impl State {
    /// A fresh idle state that keeps `previous`'s voice flag and its id and
    /// token allocators. Numbering never restarts, so an input still queued
    /// from before the reset can never match an id issued after it.
    pub fn reset_from(previous: &State) -> State {
        State {
            voice_available: previous.voice_available,
            next_id: previous.next_id,
            next_token: previous.next_token,
            ..State::default()
        }
    }
}

/// Decides the follow-up transition at `speak_done`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplyKind {
    /// Success/error/cancel template or LLM reply: start the 3 s settle timer.
    Final,
    /// Confirmation prompt: open `listen{confirm}` and the 10 s timer.
    ConfirmPrompt,
    /// Question of a pending slot: listen directly (voice) or wait for text.
    AskFollowUp,
    /// Reminder alert.
    Alert,
}

/// Localized template ids; the host renders them in the reply language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    VoiceNotReady,
    Error,
    Cancelled,
    ActionDone,
    SearchUnavailable,
    ReminderAlert,
}

/// What a `Speak` effect says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Say {
    Template(Template),
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListenMode {
    Command,
    Confirm,
}

/// Timers. Durations are the design defaults; the host may tune them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerKind {
    /// 6 s until `speech_start`.
    NoSpeech,
    /// 15 s after `speech_start`.
    MaxUtterance,
    /// Wait for the STT result after a forced end of speech.
    SttTimeout,
    /// 15 s LLM call.
    Llm,
    /// 15 s wait for text while a pending slot is open and voice is down.
    FollowUp,
    /// 5 s tool run.
    Tool,
    /// 10 s confirmation.
    Confirm,
    /// 3 s settle before hiding the island.
    Settle,
    /// 30 s cap of a reminder alert.
    AlertEnd,
    /// Per-`Speak` watchdog. Started by the runner (length-based) for every
    /// `Speak`, never by the reducer, so it has no fixed duration. Its token is
    /// the id of the `Speak` it guards.
    SpeakWatchdog,
}

impl TimerKind {
    pub const ALL: [TimerKind; 10] = [
        TimerKind::NoSpeech,
        TimerKind::MaxUtterance,
        TimerKind::SttTimeout,
        TimerKind::Llm,
        TimerKind::FollowUp,
        TimerKind::Tool,
        TimerKind::Confirm,
        TimerKind::Settle,
        TimerKind::AlertEnd,
        TimerKind::SpeakWatchdog,
    ];

    pub const fn duration(self) -> Option<Duration> {
        match self {
            TimerKind::NoSpeech => Some(Duration::from_secs(6)),
            TimerKind::MaxUtterance => Some(Duration::from_secs(15)),
            TimerKind::SttTimeout => Some(Duration::from_secs(10)),
            TimerKind::Llm => Some(Duration::from_secs(15)),
            TimerKind::FollowUp => Some(Duration::from_secs(15)),
            TimerKind::Tool => Some(Duration::from_secs(5)),
            TimerKind::Confirm => Some(Duration::from_secs(10)),
            TimerKind::Settle => Some(Duration::from_secs(3)),
            TimerKind::AlertEnd => Some(Duration::from_secs(30)),
            TimerKind::SpeakWatchdog => None,
        }
    }
}

/// Verdict of the service-side validation of the LLM tool calls (done in
/// `thinking`, before any tool runs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolVerdict {
    /// Safe calls: run them.
    RunSafe,
    /// Only risky calls: confirm the first one.
    Confirm { prompt: String, click_only: bool },
    /// Unknown tool, no match or invalid arguments: nothing executes.
    Invalid,
}

/// Outcome of the tool run reported by the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolsOutcome {
    Completed,
    /// A `web_search` result is ready for the no-tools summary call.
    SearchResult,
    /// Safe calls finished and a risky call of the same response remains.
    RiskyRemaining {
        prompt: String,
        click_only: bool,
    },
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Wake word from the sidecar.
    Wake,
    /// Global shortcut or tray toggle.
    Shortcut,
    SpeechStart {
        id: u64,
    },
    Transcript {
        id: u64,
        text: String,
    },
    NoSpeech {
        id: u64,
    },
    EmptyTranscript {
        id: u64,
    },
    /// Debug window / degraded text input.
    TextSubmitted {
        text: String,
    },
    /// A batch of due reminders (`count` of them) is ready to alert. `silent`
    /// (set by the service while a fullscreen app is active) means chime only.
    ReminderDue {
        count: u32,
        silent: bool,
    },
    ToolCalls {
        req: u64,
        verdict: ToolVerdict,
    },
    ToolsDone {
        req: u64,
        outcome: ToolsOutcome,
    },
    Reply {
        req: u64,
        text: String,
    },
    Failure {
        req: u64,
    },
    /// A tool needs one more piece of input (always raised from `thinking`).
    AskFollowUp {
        req: u64,
        question: String,
        slot: PendingSlot,
    },
    Accept,
    Reject,
    SpeakStarted {
        id: u64,
    },
    SpeakDone {
        id: u64,
    },
    TtsFailed {
        id: u64,
    },
    /// A timer fired (or the speak watchdog expired). `token` is the one from
    /// the `StartTimer` that armed it; for `SpeakWatchdog` it is the speak id.
    Watchdog {
        kind: TimerKind,
        token: u64,
    },
    /// The alert chime with this id finished (or was skipped).
    ChimeDone {
        id: u64,
    },
    /// Voice supervisor: sidecar ready / down.
    VoiceReadinessChanged {
        ready: bool,
    },
    /// Click on the reminder alert.
    Dismiss,
}

/// Input of a `StartRequest` effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestInput {
    /// A user utterance, with the pending slot as context for a follow-up.
    User {
        text: String,
        slot: Option<PendingSlot>,
    },
    /// The no-tools summary call after a `web_search`.
    SearchSummary,
}

/// Things the host must do. The reducer never performs them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Speak {
        id: u64,
        kind: ReplyKind,
        say: Say,
    },
    /// Also stops a playing chime.
    StopSpeaking,
    Listen {
        id: u64,
        mode: ListenMode,
    },
    CancelListen {
        id: u64,
    },
    /// Ask the sidecar to end the utterance now (max-utterance cap).
    ForceEndOfSpeech {
        id: u64,
    },
    /// Start an LLM call for `req`.
    StartRequest {
        req: u64,
        input: RequestInput,
    },
    RunTool {
        req: u64,
    },
    CancelRequest {
        req: u64,
    },
    /// Starting a kind that is already running restarts it with a new token.
    StartTimer {
        kind: TimerKind,
        token: u64,
    },
    CancelTimer {
        kind: TimerKind,
    },
    ShowIsland,
    HideIsland,
    /// Play the alert chime and report `ChimeDone{id}` when it ends.
    Chime {
        id: u64,
    },
    /// Keep a due reminder queued in the service until the state is idle.
    EnqueueReminder {
        count: u32,
    },
    /// Emit `ReminderDue` for the queued batch (sent on every entry to idle).
    DrainReminders,
}

// ---------------------------------------------------------------------------
// Transition helpers (state table: entry and exit of the 8 states)
// ---------------------------------------------------------------------------

impl State {
    pub(crate) fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Starts (or restarts) a reducer-owned timer.
    pub(crate) fn start_timer(&mut self, kind: TimerKind, fx: &mut Vec<Effect>) {
        let token = self.next_token;
        self.next_token += 1;
        match self.timers.iter_mut().find(|t| t.kind == kind) {
            Some(running) => running.token = token,
            None => self.timers.push(Timer { kind, token }),
        }
        fx.push(Effect::StartTimer { kind, token });
    }

    /// Cancels a timer if it is running; otherwise does nothing.
    pub(crate) fn cancel_timer(&mut self, kind: TimerKind, fx: &mut Vec<Effect>) {
        if let Some(pos) = self.timers.iter().position(|t| t.kind == kind) {
            self.timers.remove(pos);
            fx.push(Effect::CancelTimer { kind });
        }
    }

    /// Single door between states. Leaving a state cleans up everything it
    /// owns: its timers, its open listen, its speech (and the alert chime) and,
    /// unless the next state continues the request, the in-flight request.
    /// Callers that *completed* the request clear `request_id` first so no
    /// `CancelRequest` is emitted for it.
    pub(crate) fn transition(&mut self, to: Phase, fx: &mut Vec<Effect>) {
        let from = self.phase;
        for timer in self.timers.drain(..) {
            fx.push(Effect::CancelTimer { kind: timer.kind });
        }
        if let Some(id) = self.listen_id.take() {
            fx.push(Effect::CancelListen { id });
        }
        let was_speaking = self.speak_id.take().is_some();
        self.speak_kind = None;
        if was_speaking || from == Phase::ReminderAlert {
            fx.push(Effect::StopSpeaking);
        }
        let continues_request = matches!(
            to,
            Phase::Thinking | Phase::Acting | Phase::ConfirmationNeeded
        );
        if !continues_request && let Some(req) = self.request_id.take() {
            fx.push(Effect::CancelRequest { req });
        }
        // Only the follow-up listen carries the pending slot forward.
        if !(from == Phase::Thinking && to == Phase::Listening) {
            self.pending = None;
        }
        self.followup_turn = false;
        self.summarizing = false;
        self.confirm_click_only = false;
        self.alert_count = 0;
        self.chime_id = None;
        self.utterance = Utterance::WaitingForSpeech;
        self.phase = to;
        if from == Phase::Idle && to != Phase::Idle {
            fx.push(Effect::ShowIsland);
        }
        if to == Phase::Idle {
            fx.push(Effect::HideIsland);
            fx.push(Effect::DrainReminders);
        }
    }

    /// Emits a `Speak` and records it as the active speech.
    pub(crate) fn speak(&mut self, kind: ReplyKind, say: Say, fx: &mut Vec<Effect>) {
        let id = self.alloc_id();
        self.speak_id = Some(id);
        self.speak_kind = Some(kind);
        fx.push(Effect::Speak { id, kind, say });
    }

    /// Opens a sidecar listen and records its id.
    pub(crate) fn open_listen(&mut self, mode: ListenMode, fx: &mut Vec<Effect>) {
        let id = self.alloc_id();
        self.listen_id = Some(id);
        fx.push(Effect::Listen { id, mode });
    }

    pub(crate) fn enter_listening(&mut self, fx: &mut Vec<Effect>) {
        self.transition(Phase::Listening, fx);
        self.open_listen(ListenMode::Command, fx);
        self.start_timer(TimerKind::NoSpeech, fx);
    }

    pub(crate) fn enter_error(&mut self, template: Template, fx: &mut Vec<Effect>) {
        self.transition(Phase::Error, fx);
        self.speak(ReplyKind::Final, Say::Template(template), fx);
    }

    pub(crate) fn enter_success(&mut self, say: Say, fx: &mut Vec<Effect>) {
        self.transition(Phase::Success, fx);
        self.speak(ReplyKind::Final, say, fx);
    }
}

/// How a spoken confirmation answer reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    Unclear,
}

const YES_TOKENS: [&str; 9] = [
    "sí", "si", "claro", "dale", "ok", "yes", "yeah", "yep", "confirm",
];
const NO_TOKENS: [&str; 5] = ["no", "cancela", "cancelar", "cancel", "nope"];

/// Whole-utterance yes/no matcher: lowercase, trimmed, punctuation stripped, a
/// trailing "por favor" / "please" tolerated. A longer sentence that merely
/// contains a token is `Unclear` (ambient speech must not confirm anything).
pub fn classify_confirmation(text: &str) -> Answer {
    let lowered = text.to_lowercase();
    let cleaned: String = lowered
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    let mut words: Vec<&str> = cleaned.split_whitespace().collect();
    for polite in [["por", "favor"].as_slice(), ["please"].as_slice()] {
        if words.len() > polite.len() && words.ends_with(polite) {
            words.truncate(words.len() - polite.len());
            break;
        }
    }
    match words.as_slice() {
        [word] if YES_TOKENS.contains(word) => Answer::Yes,
        [word] if NO_TOKENS.contains(word) => Answer::No,
        _ => Answer::Unclear,
    }
}

// ---------------------------------------------------------------------------
// The reducer
// ---------------------------------------------------------------------------

/// `reduce(state, event) -> (state, effects)`. Pure; see the module docs.
pub fn reduce(mut state: State, event: Event) -> (State, Vec<Effect>) {
    let mut fx = Vec::new();
    if state.is_stale(&event) {
        return (state, fx);
    }
    match event {
        Event::Wake => state.on_wake(&mut fx),
        Event::Shortcut => state.on_shortcut(&mut fx),
        Event::SpeechStart { .. } => state.on_speech_start(&mut fx),
        Event::Transcript { text, .. } => state.on_transcript(text, &mut fx),
        Event::NoSpeech { .. } => state.on_no_speech(&mut fx),
        Event::EmptyTranscript { .. } => state.on_empty_transcript(&mut fx),
        Event::TextSubmitted { text } => state.on_text_submitted(text, &mut fx),
        Event::ToolCalls { req, verdict } => state.on_tool_calls(req, verdict, &mut fx),
        Event::ToolsDone { outcome, .. } => state.on_tools_done(outcome, &mut fx),
        Event::Reply { text, .. } => state.on_reply(text, &mut fx),
        Event::Failure { .. } => state.on_failure(&mut fx),
        Event::AskFollowUp { question, slot, .. } => {
            state.on_ask_follow_up(question, slot, &mut fx)
        }
        // Nothing to do: the reply's follow-up starts at speak_done.
        Event::SpeakStarted { .. } => {}
        // `TtsFailed` is treated as done: the text stays shown.
        Event::SpeakDone { .. } | Event::TtsFailed { .. } => state.on_speak_done(&mut fx),
        Event::Watchdog { kind, .. } => state.on_watchdog(kind, &mut fx),
        Event::VoiceReadinessChanged { ready } => state.on_voice_readiness(ready, &mut fx),
        Event::Accept => state.on_accept(&mut fx),
        Event::Reject => state.on_reject(&mut fx),
        Event::ReminderDue { count, silent } => state.on_reminder_due(count, silent, &mut fx),
        Event::Dismiss => state.on_dismiss(&mut fx),
        Event::ChimeDone { .. } => state.on_chime_done(&mut fx),
    }
    (state, fx)
}

impl State {
    /// Id-carrying events of a finished listen/speak/request are dropped in
    /// every state ("stale id" row of the matrix).
    fn is_stale(&self, event: &Event) -> bool {
        match event {
            Event::SpeechStart { id }
            | Event::Transcript { id, .. }
            | Event::NoSpeech { id }
            | Event::EmptyTranscript { id } => self.listen_id != Some(*id),
            Event::SpeakStarted { id } | Event::SpeakDone { id } | Event::TtsFailed { id } => {
                self.speak_id != Some(*id)
            }
            Event::ToolCalls { req, .. }
            | Event::ToolsDone { req, .. }
            | Event::Reply { req, .. }
            | Event::Failure { req }
            | Event::AskFollowUp { req, .. } => self.request_id != Some(*req),
            // The speak watchdog is bound to the speak it was armed for.
            Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token,
            } => self.speak_id != Some(*token),
            // Any other timer is bound to the instance that is running now.
            Event::Watchdog { kind, token } => !self
                .timers
                .iter()
                .any(|t| t.kind == *kind && t.token == *token),
            Event::ChimeDone { id } => self.chime_id != Some(*id),
            Event::Wake
            | Event::Shortcut
            | Event::TextSubmitted { .. }
            | Event::ReminderDue { .. }
            | Event::Accept
            | Event::Reject
            | Event::VoiceReadinessChanged { .. }
            | Event::Dismiss => false,
        }
    }

    /// Voice guard: listen only if the sidecar is usable, otherwise `error`
    /// with the "voice not ready" template (the text window is offered).
    fn activate(&mut self, fx: &mut Vec<Effect>) {
        if self.voice_available {
            self.enter_listening(fx);
        } else {
            self.enter_error(Template::VoiceNotReady, fx);
        }
    }

    fn on_wake(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Idle => self.activate(fx),
            // Self-trigger guard: ignored while Niva speaks. Voice down: a
            // wake cannot come from a dead sidecar, so there is nothing to do.
            Phase::Success | Phase::Error => {
                if self.speak_id.is_none() && self.voice_available {
                    self.enter_listening(fx);
                }
            }
            Phase::ReminderAlert => {
                if self.voice_available {
                    self.enter_listening(fx);
                }
            }
            Phase::Listening | Phase::Thinking | Phase::Acting | Phase::ConfirmationNeeded => {}
        }
    }

    fn on_shortcut(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Idle | Phase::Success | Phase::Error | Phase::ReminderAlert => self.activate(fx),
            Phase::Listening | Phase::Thinking | Phase::Acting => self.transition(Phase::Idle, fx),
            Phase::ConfirmationNeeded => self.reject(fx),
        }
    }

    fn on_speech_start(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Listening => {
                if self.utterance == Utterance::WaitingForSpeech {
                    self.cancel_timer(TimerKind::NoSpeech, fx);
                    self.start_timer(TimerKind::MaxUtterance, fx);
                    self.utterance = Utterance::Speaking;
                }
            }
            Phase::Idle
            | Phase::Thinking
            | Phase::Acting
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => {}
        }
    }

    /// Starts the LLM call for a user utterance. The pending slot, if any, is
    /// consumed: it travels with the request as context for exactly one turn.
    fn think_about(&mut self, text: String, fx: &mut Vec<Effect>) {
        let slot = self.pending.take();
        self.transition(Phase::Thinking, fx);
        self.followup_turn = slot.is_some();
        self.begin_request(RequestInput::User { text, slot }, fx);
    }

    fn begin_request(&mut self, input: RequestInput, fx: &mut Vec<Effect>) {
        let req = self.alloc_id();
        self.request_id = Some(req);
        fx.push(Effect::StartRequest { req, input });
        self.start_timer(TimerKind::Llm, fx);
    }

    fn on_transcript(&mut self, text: String, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Listening => {
                self.listen_id = None; // the sidecar listen is already over
                self.think_about(text, fx);
            }
            // A click-only confirmation has no voice channel: a spoken answer
            // cannot be a real one, whatever it says.
            Phase::ConfirmationNeeded if self.confirm_click_only => {}
            Phase::ConfirmationNeeded => match classify_confirmation(&text) {
                Answer::Yes => self.accept(fx),
                Answer::No => self.reject(fx),
                Answer::Unclear => self.reissue_confirm_listen(fx),
            },
            Phase::Idle
            | Phase::Thinking
            | Phase::Acting
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => {}
        }
    }

    fn on_no_speech(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Listening => {
                self.listen_id = None;
                self.transition(Phase::Idle, fx); // silent, no error UI
            }
            Phase::ConfirmationNeeded => self.reissue_confirm_listen(fx),
            Phase::Idle
            | Phase::Thinking
            | Phase::Acting
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => {}
        }
    }

    fn on_empty_transcript(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Listening => {
                self.listen_id = None;
                self.enter_error(Template::Error, fx);
            }
            Phase::ConfirmationNeeded => self.reissue_confirm_listen(fx),
            Phase::Idle
            | Phase::Thinking
            | Phase::Acting
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => {}
        }
    }

    /// Unclear answer, silence or empty STT during a voice confirmation: listen
    /// again with a new id while the 10 s timer keeps running (never restarted).
    fn reissue_confirm_listen(&mut self, fx: &mut Vec<Effect>) {
        if self.voice_available && !self.confirm_click_only {
            self.open_listen(ListenMode::Confirm, fx);
        }
    }

    fn accept(&mut self, fx: &mut Vec<Effect>) {
        debug_assert!(self.request_id.is_some(), "accept without a request");
        // `transition` stops the prompt if it is still playing and closes the
        // confirm listen, so the first valid answer wins over the other channel.
        self.transition(Phase::Acting, fx);
        self.start_timer(TimerKind::Tool, fx);
        if let Some(req) = self.request_id {
            fx.push(Effect::RunTool { req });
        }
    }

    fn on_accept(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::ConfirmationNeeded => self.accept(fx),
            Phase::Idle
            | Phase::Listening
            | Phase::Thinking
            | Phase::Acting
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => {}
        }
    }

    fn on_reject(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::ConfirmationNeeded => self.reject(fx),
            Phase::Idle
            | Phase::Listening
            | Phase::Thinking
            | Phase::Acting
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => {}
        }
    }

    fn on_reminder_due(&mut self, count: u32, silent: bool, fx: &mut Vec<Effect>) {
        if count == 0 {
            return;
        }
        match self.phase {
            Phase::Idle => {
                self.transition(Phase::ReminderAlert, fx);
                self.alert_count = count;
                let chime = self.alloc_id();
                self.chime_id = Some(chime);
                fx.push(Effect::Chime { id: chime });
                // Silent (fullscreen policy): the chime only, no speech.
                if !silent {
                    self.speak(ReplyKind::Alert, Say::Template(Template::ReminderAlert), fx);
                }
                self.start_timer(TimerKind::AlertEnd, fx);
            }
            // Merged into the alert on screen; the view reads the new count.
            Phase::ReminderAlert => self.alert_count = self.alert_count.saturating_add(count),
            // Never interrupts anything else (least of all a confirmation).
            Phase::Listening
            | Phase::Thinking
            | Phase::Acting
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error => fx.push(Effect::EnqueueReminder { count }),
        }
    }

    fn on_dismiss(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::ReminderAlert => self.transition(Phase::Idle, fx),
            Phase::Idle
            | Phase::Listening
            | Phase::Thinking
            | Phase::Acting
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error => {}
        }
    }

    fn on_chime_done(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::ReminderAlert => {
                self.chime_id = None;
                self.end_alert_if_finished(fx);
            }
            Phase::Idle
            | Phase::Listening
            | Phase::Thinking
            | Phase::Acting
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error => {}
        }
    }

    /// The alert ends at the later of "chime done" and "speech done" (the 30 s
    /// cap and a click end it earlier).
    fn end_alert_if_finished(&mut self, fx: &mut Vec<Effect>) {
        if self.chime_id.is_none() && self.speak_id.is_none() {
            self.transition(Phase::Idle, fx);
        }
    }

    fn on_text_submitted(&mut self, text: String, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Idle
            | Phase::Listening
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => self.think_about(text, fx),
            // Only while a pending slot waits for its answer.
            Phase::Thinking => {
                if self.pending.is_some() {
                    self.think_about(text, fx);
                }
            }
            Phase::Acting | Phase::ConfirmationNeeded => {}
        }
    }

    /// The request is over (answered or failed): late results of this id are
    /// dropped and no `CancelRequest` is needed.
    fn finish_request(&mut self) {
        self.request_id = None;
    }

    /// The template for a failure of the current request.
    fn failure_template(&self) -> Template {
        if self.summarizing {
            Template::SearchUnavailable
        } else {
            Template::Error
        }
    }

    /// Fails the in-flight request with the matching error template.
    fn fail_request(&mut self, fx: &mut Vec<Effect>) {
        let template = self.failure_template();
        self.finish_request();
        self.enter_error(template, fx);
    }

    fn ask_confirmation(&mut self, prompt: String, click_only: bool, fx: &mut Vec<Effect>) {
        self.transition(Phase::ConfirmationNeeded, fx);
        self.confirm_click_only = click_only;
        // The 10 s timer and `listen{confirm}` start at speak_done.
        self.speak(ReplyKind::ConfirmPrompt, Say::Text(prompt), fx);
    }

    fn on_tool_calls(&mut self, req: u64, verdict: ToolVerdict, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Thinking => {}
            Phase::Idle
            | Phase::Listening
            | Phase::Acting
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => return,
        }
        // The summary call is no-tools: any tool call in its answer is invalid.
        let verdict = if self.summarizing {
            ToolVerdict::Invalid
        } else {
            verdict
        };
        match verdict {
            ToolVerdict::RunSafe => {
                self.transition(Phase::Acting, fx);
                self.start_timer(TimerKind::Tool, fx);
                fx.push(Effect::RunTool { req });
            }
            ToolVerdict::Confirm { prompt, click_only } => {
                self.ask_confirmation(prompt, click_only, fx);
            }
            ToolVerdict::Invalid => self.fail_request(fx),
        }
    }

    fn on_tools_done(&mut self, outcome: ToolsOutcome, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Acting => {}
            Phase::Idle
            | Phase::Listening
            | Phase::Thinking
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => return,
        }
        match outcome {
            ToolsOutcome::Completed => {
                self.finish_request();
                self.enter_success(Say::Template(Template::ActionDone), fx);
            }
            ToolsOutcome::SearchResult => {
                self.finish_request();
                self.transition(Phase::Thinking, fx);
                self.summarizing = true;
                self.begin_request(RequestInput::SearchSummary, fx);
            }
            ToolsOutcome::RiskyRemaining { prompt, click_only } => {
                self.ask_confirmation(prompt, click_only, fx);
            }
            ToolsOutcome::Failed => {
                self.finish_request();
                self.enter_error(Template::Error, fx);
            }
        }
    }

    fn on_reply(&mut self, text: String, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Thinking | Phase::Acting => {
                self.finish_request();
                self.enter_success(Say::Text(text), fx);
            }
            Phase::Idle
            | Phase::Listening
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => {}
        }
    }

    fn on_failure(&mut self, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Thinking | Phase::Acting => self.fail_request(fx),
            Phase::Idle
            | Phase::Listening
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => {}
        }
    }

    fn on_ask_follow_up(&mut self, question: String, slot: PendingSlot, fx: &mut Vec<Effect>) {
        match self.phase {
            Phase::Thinking => {}
            Phase::Idle
            | Phase::Listening
            | Phase::Acting
            | Phase::ConfirmationNeeded
            | Phase::Success
            | Phase::Error
            | Phase::ReminderAlert => return,
        }
        if self.followup_turn {
            // One follow-up turn only: a second ambiguity ends the request.
            self.fail_request(fx);
            return;
        }
        self.cancel_timer(TimerKind::Llm, fx);
        self.finish_request();
        self.pending = Some(slot);
        // The state and the mascot stay `thinking` while the question plays.
        self.speak(ReplyKind::AskFollowUp, Say::Text(question), fx);
    }

    /// Speech finished (`SpeakDone`, `TtsFailed` or the synthesized one from
    /// the speak watchdog): start whatever the reply kind waits for.
    fn on_speak_done(&mut self, fx: &mut Vec<Effect>) {
        let kind = self.speak_kind.take();
        if self.speak_id.take().is_none() {
            return;
        }
        let Some(kind) = kind else { return };
        match kind {
            ReplyKind::Final => {
                if matches!(self.phase, Phase::Success | Phase::Error) {
                    self.start_timer(TimerKind::Settle, fx);
                }
            }
            ReplyKind::ConfirmPrompt => {
                if self.phase == Phase::ConfirmationNeeded {
                    self.start_timer(TimerKind::Confirm, fx);
                    // Click-only: no voice channel, the buttons are the only way.
                    if self.voice_available && !self.confirm_click_only {
                        self.open_listen(ListenMode::Confirm, fx);
                    }
                }
            }
            ReplyKind::AskFollowUp => {
                if self.phase == Phase::Thinking {
                    if self.voice_available {
                        self.enter_listening(fx);
                    } else {
                        // Slot stays open; `TextSubmitted` or this timer ends it.
                        self.start_timer(TimerKind::FollowUp, fx);
                    }
                }
            }
            ReplyKind::Alert => {
                if self.phase == Phase::ReminderAlert {
                    self.end_alert_if_finished(fx);
                }
            }
        }
    }

    /// A timer of the current instance fired (stale ones were dropped by
    /// `is_stale`). The timer is forgotten first; in a phase that does not own
    /// that kind nothing else happens (no `CancelTimer` is needed: it is gone).
    fn on_watchdog(&mut self, kind: TimerKind, fx: &mut Vec<Effect>) {
        if kind != TimerKind::SpeakWatchdog {
            self.timers.retain(|t| t.kind != kind);
        }
        match kind {
            // The runner synthesizes the `speak_done` the sidecar never sent.
            TimerKind::SpeakWatchdog => self.on_speak_done(fx),
            TimerKind::NoSpeech => match self.phase {
                Phase::Listening => self.transition(Phase::Idle, fx),
                Phase::Idle
                | Phase::Thinking
                | Phase::Acting
                | Phase::ConfirmationNeeded
                | Phase::Success
                | Phase::Error
                | Phase::ReminderAlert => {}
            },
            TimerKind::MaxUtterance => match self.phase {
                Phase::Listening => {
                    if let Some(id) = self.listen_id {
                        fx.push(Effect::ForceEndOfSpeech { id });
                    }
                    self.utterance = Utterance::Ending;
                    self.start_timer(TimerKind::SttTimeout, fx);
                }
                Phase::Idle
                | Phase::Thinking
                | Phase::Acting
                | Phase::ConfirmationNeeded
                | Phase::Success
                | Phase::Error
                | Phase::ReminderAlert => {}
            },
            TimerKind::SttTimeout => match self.phase {
                Phase::Listening => self.enter_error(Template::Error, fx),
                Phase::Idle
                | Phase::Thinking
                | Phase::Acting
                | Phase::ConfirmationNeeded
                | Phase::Success
                | Phase::Error
                | Phase::ReminderAlert => {}
            },
            // `transition` cancels the in-flight request.
            TimerKind::Llm => match self.phase {
                Phase::Thinking => {
                    let template = self.failure_template();
                    self.enter_error(template, fx);
                }
                Phase::Idle
                | Phase::Listening
                | Phase::Acting
                | Phase::ConfirmationNeeded
                | Phase::Success
                | Phase::Error
                | Phase::ReminderAlert => {}
            },
            TimerKind::Tool => match self.phase {
                Phase::Acting => {
                    let template = self.failure_template();
                    self.enter_error(template, fx);
                }
                Phase::Idle
                | Phase::Listening
                | Phase::Thinking
                | Phase::ConfirmationNeeded
                | Phase::Success
                | Phase::Error
                | Phase::ReminderAlert => {}
            },
            TimerKind::FollowUp => match self.phase {
                Phase::Thinking => self.transition(Phase::Idle, fx),
                Phase::Idle
                | Phase::Listening
                | Phase::Acting
                | Phase::ConfirmationNeeded
                | Phase::Success
                | Phase::Error
                | Phase::ReminderAlert => {}
            },
            TimerKind::Confirm => match self.phase {
                Phase::ConfirmationNeeded => self.reject(fx),
                Phase::Idle
                | Phase::Listening
                | Phase::Thinking
                | Phase::Acting
                | Phase::Success
                | Phase::Error
                | Phase::ReminderAlert => {}
            },
            TimerKind::Settle => match self.phase {
                Phase::Success | Phase::Error => self.transition(Phase::Idle, fx),
                Phase::Idle
                | Phase::Listening
                | Phase::Thinking
                | Phase::Acting
                | Phase::ConfirmationNeeded
                | Phase::ReminderAlert => {}
            },
            TimerKind::AlertEnd => match self.phase {
                Phase::ReminderAlert => self.transition(Phase::Idle, fx),
                Phase::Idle
                | Phase::Listening
                | Phase::Thinking
                | Phase::Acting
                | Phase::ConfirmationNeeded
                | Phase::Success
                | Phase::Error => {}
            },
        }
    }

    fn on_voice_readiness(&mut self, ready: bool, fx: &mut Vec<Effect>) {
        self.voice_available = ready;
        if ready {
            // Clears degraded. A confirmation already in progress stays
            // click-only; later prompts may use voice again.
            return;
        }
        match self.phase {
            Phase::Listening => {
                // CO4: the Speak is skipped by the runner while voice is down,
                // but its synthesized speak_done starts the 3 s settle so the
                // island does not stay stuck in `error`.
                self.enter_error(Template::Error, fx);
            }
            Phase::ConfirmationNeeded => {
                // The voice channel is gone (the sidecar is down: nothing to
                // cancel there); the click path and the 10 s timer remain.
                self.listen_id = None;
                self.confirm_click_only = true;
            }
            // Voice down in success/error: text only, no point in settling.
            Phase::Success | Phase::Error => self.transition(Phase::Idle, fx),
            // LLM and tools do not need the sidecar; the alert chime is
            // played from Rust; idle just becomes degraded.
            Phase::Idle | Phase::Thinking | Phase::Acting | Phase::ReminderAlert => {}
        }
    }

    /// Reject, Shortcut or timeout in `confirmation-needed`: the request ends
    /// with the "cancelled" template, not fed back to the LLM; the remaining
    /// risky calls are discarded by the service.
    fn reject(&mut self, fx: &mut Vec<Effect>) {
        self.request_id = None;
        self.enter_success(Say::Template(Template::Cancelled), fx);
    }
}
