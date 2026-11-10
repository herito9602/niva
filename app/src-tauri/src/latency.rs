//! The per-turn latency trace (task 3.5, design "Metric logging").
//!
//! One record per request, with the timestamps of its stages on ONE monotonic
//! clock of the host (milliseconds since the app started; the voice sidecar's
//! `end_ms` is mapped onto it by `adapters::voice_sidecar::translate`), the
//! stage durations derived from them, the path the request took, the channel
//! and the validation outcome. The point is to measure the goals of the
//! `speech-io` spec with the owner's real microphone: mean < 2.6 s and p95
//! <= 3.0 s from the end of speech to the first audio on the action path.
//!
//! # What is, and is not, in a record
//!
//! Numbers and fixed words only. The tracer never sees the text of a command,
//! of a reply or of a prompt: [`Obs`] and [`FxObs`] are built from the reducer's
//! events and effects and carry ids and kinds, so no user text can reach a
//! record by construction (and a test with a sentinel pins it end to end).
//!
//! # Where it listens
//!
//! The dispatcher already sees every event and every effect, in order, so the
//! trace is a read-only observer there; the reducer, the runner and the
//! assistant are untouched. A "turn" starts at the `StartRequest` the user's
//! words caused and ends at its first sound (`SpeakStarted`): that is the
//! metric. A turn that never makes a sound (voice down, text mode, a failed
//! speech, a cancel) ends at the speech's end, or at the next turn, and is
//! written with the missing fields absent, never invented.
//!
//! # What the assistant really emits
//!
//! A safe action does NOT end with `ToolsDone { Completed }`: the assistant
//! answers the tools with `Reply` (or `FailureText`) after the `ToolCalls` that
//! started them. So a `Reply` is the model's words only while a model call is
//! open; after a `RunTool` it is the end of the tools. Counting every `Reply` as
//! a model answer labelled the first real action turns `cancelled` and counted
//! their model time twice (the first run, Batch 38).
//!
//! Not covered, on purpose: `t_llm_first_token` (the Ollama adapter does not
//! stream, so there is none; the field is left out), and whatever happens
//! after the first sound (a confirmed action's tools run after the turn was
//! written; the confirmation answer's latency is not part of the goal).

use std::panic::{AssertUnwindSafe, catch_unwind};

use niva_core::replies::Lang;
use niva_core::state_machine::{
    Effect, Event, ReplyKind, RequestInput, TimerKind, ToolVerdict, ToolsOutcome,
};
use serde::Serialize;

pub mod sink;

/// Version of the record shape, bumped if a field changes meaning.
pub const RECORD_VERSION: u32 = 1;

/// What the voice sidecar side knows about one transcript, on the host clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    /// The end of the user's speech, mapped from the sidecar's clock. `None`
    /// before the first `ready` gave the mapping, or if it maps below zero.
    pub end_of_speech: Option<u64>,
    /// When the transcript reached the host.
    pub transcript: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Voice,
    Text,
}

impl Channel {
    pub const fn as_str(self) -> &'static str {
        match self {
            Channel::Voice => "voice",
            Channel::Text => "text",
        }
    }
}

/// What the turn actually did (decided from what happened, not from a guess
/// about the words).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// Safe tools ran and a template answer followed.
    Action,
    /// A `web_search` ran and the model summarised it.
    Search,
    /// The model answered in words, no tools.
    Chat,
    /// The first sound is a confirmation question (kept apart: it is not an
    /// action-path measurement).
    Confirm,
    /// The first sound is a follow-up question about a missing argument.
    FollowUp,
    /// The first sound is the sleep countdown's announcement (kept apart: it
    /// is not an action-path measurement either).
    Sleep,
    /// The model or a tool failed, or the plan was refused.
    Error,
    /// The turn ended before it produced anything (a cancel, a new command).
    Cancelled,
}

impl Path {
    pub const fn as_str(self) -> &'static str {
        match self {
            Path::Action => "action",
            Path::Search => "search",
            Path::Chat => "chat",
            Path::Confirm => "confirm",
            Path::FollowUp => "followup",
            Path::Sleep => "sleep",
            Path::Error => "error",
            Path::Cancelled => "cancelled",
        }
    }
}

/// What the first answer of the model turned into (design "Validation before
/// execution").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Validation {
    RunSafe,
    Confirm,
    Invalid,
    /// An answer in words with no tool call.
    Reply,
    FollowUp,
    LlmFailed,
}

impl Validation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Validation::RunSafe => "run_safe",
            Validation::Confirm => "confirm",
            Validation::Invalid => "invalid",
            Validation::Reply => "reply",
            Validation::FollowUp => "follow_up",
            Validation::LlmFailed => "llm_failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolsKind {
    Completed,
    /// A `web_search` result is ready for the summary call.
    Search,
    /// A risky call remains: a confirmation follows.
    Risky,
    Failed,
    Sleep,
}

/// A reducer event reduced to what the trace needs. No text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Obs {
    /// The words of a voice command; `stamp` is what the sidecar side knew.
    Transcript {
        stamp: Option<Stamp>,
    },
    TextSubmitted,
    ToolCalls {
        req: u64,
        validation: Validation,
    },
    ToolsDone {
        req: u64,
        kind: ToolsKind,
    },
    Reply {
        req: u64,
    },
    Failure {
        req: u64,
    },
    AskFollowUp {
        req: u64,
    },
    SpeakStarted {
        id: u64,
    },
    /// `SpeakDone`, `TtsFailed` or the speak watchdog of speech `id`.
    SpeakEnded {
        id: u64,
    },
    Other,
}

impl Obs {
    /// `stamp_of(listen id)` is the sidecar side's record of that transcript.
    pub fn of(event: &Event, stamp_of: &dyn Fn(u64) -> Option<Stamp>) -> Obs {
        match event {
            Event::Transcript { id, .. } => Obs::Transcript {
                stamp: stamp_of(*id),
            },
            Event::TextSubmitted { .. } => Obs::TextSubmitted,
            Event::ToolCalls { req, verdict } => Obs::ToolCalls {
                req: *req,
                validation: match verdict {
                    ToolVerdict::RunSafe => Validation::RunSafe,
                    ToolVerdict::Confirm { .. } => Validation::Confirm,
                    ToolVerdict::Invalid => Validation::Invalid,
                },
            },
            Event::ToolsDone { req, outcome } => Obs::ToolsDone {
                req: *req,
                kind: match outcome {
                    ToolsOutcome::Completed => ToolsKind::Completed,
                    ToolsOutcome::SearchResult => ToolsKind::Search,
                    ToolsOutcome::RiskyRemaining { .. } => ToolsKind::Risky,
                    ToolsOutcome::Failed => ToolsKind::Failed,
                    ToolsOutcome::SleepCountdown { .. } => ToolsKind::Sleep,
                },
            },
            Event::Reply { req, .. } => Obs::Reply { req: *req },
            Event::Failure { req } | Event::FailureText { req, .. } => Obs::Failure { req: *req },
            Event::AskFollowUp { req, .. } => Obs::AskFollowUp { req: *req },
            Event::SpeakStarted { id } => Obs::SpeakStarted { id: *id },
            Event::SpeakDone { id } | Event::TtsFailed { id } => Obs::SpeakEnded { id: *id },
            Event::Watchdog {
                kind: TimerKind::SpeakWatchdog,
                token,
            } => Obs::SpeakEnded { id: *token },
            _ => Obs::Other,
        }
    }
}

/// A reducer effect reduced to what the trace needs. No text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FxObs {
    /// `summary`: the no-tools call after a `web_search` (same turn).
    StartRequest {
        req: u64,
        summary: bool,
    },
    RunTool {
        req: u64,
    },
    CancelRequest {
        req: u64,
    },
    Speak {
        id: u64,
        kind: ReplyKind,
    },
    StopSpeaking,
    HideIsland,
}

impl FxObs {
    pub fn of(effects: &[Effect]) -> Vec<FxObs> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::StartRequest { req, input } => Some(FxObs::StartRequest {
                    req: *req,
                    summary: matches!(input, RequestInput::SearchSummary),
                }),
                Effect::RunTool { req } => Some(FxObs::RunTool { req: *req }),
                Effect::CancelRequest { req } => Some(FxObs::CancelRequest { req: *req }),
                Effect::Speak { id, kind, .. } => Some(FxObs::Speak {
                    id: *id,
                    kind: *kind,
                }),
                Effect::StopSpeaking => Some(FxObs::StopSpeaking),
                Effect::HideIsland => Some(FxObs::HideIsland),
                _ => None,
            })
            .collect()
    }
}

/// One turn in flight. Timestamps are host milliseconds.
#[derive(Debug, Clone)]
pub struct Turn {
    req: u64,
    current_req: u64,
    channel: Channel,
    t_end_of_speech: Option<u64>,
    t_transcript: Option<u64>,
    t_request: u64,
    llm_started: u64,
    /// A model call is in flight (the first one, or a search's summary): its
    /// answer, a `Reply` or a failure, is the model's.
    llm_open: bool,
    tool_started: Option<u64>,
    t_llm_done: Option<u64>,
    t_tool_done: Option<u64>,
    llm_ms: Option<u64>,
    tool_ms: Option<u64>,
    validation: Option<Validation>,
    search: bool,
    sleep: bool,
    failed: bool,
    completed: bool,
    cancelled: bool,
    first_speak: Option<(u64, ReplyKind)>,
    t_speak_sent: Option<u64>,
    t_first_audio: Option<u64>,
}

impl Turn {
    fn begin(req: u64, t: u64, stamp: Option<Stamp>) -> Turn {
        // A voice turn is one the sidecar side stamped; anything else (typed
        // text, the debug window's injected transcript) is text.
        Turn {
            req,
            current_req: req,
            channel: if stamp.is_some() {
                Channel::Voice
            } else {
                Channel::Text
            },
            t_end_of_speech: stamp.and_then(|s| s.end_of_speech),
            t_transcript: stamp.map(|s| s.transcript),
            t_request: t,
            llm_started: t,
            llm_open: true,
            tool_started: None,
            t_llm_done: None,
            t_tool_done: None,
            llm_ms: None,
            tool_ms: None,
            validation: None,
            search: false,
            sleep: false,
            failed: false,
            completed: false,
            cancelled: false,
            first_speak: None,
            t_speak_sent: None,
            t_first_audio: None,
        }
    }

    /// The model call in flight answered (or failed) at `t`. Only the first
    /// answer of a call counts: nothing is added when none is open.
    fn llm_done(&mut self, t: u64) {
        if !self.llm_open {
            return;
        }
        self.llm_open = false;
        self.t_llm_done = Some(t);
        let took = t.saturating_sub(self.llm_started);
        self.llm_ms = Some(self.llm_ms.unwrap_or(0).saturating_add(took));
    }

    /// The tools that `RunTool` started ended at `t` (by `ToolsDone`, or by the
    /// `Reply`/`FailureText` that answers them). Without a start there is
    /// nothing to close.
    fn tool_done(&mut self, t: u64) -> bool {
        let Some(started) = self.tool_started.take() else {
            return false;
        };
        self.t_tool_done = Some(t);
        let took = t.saturating_sub(started);
        self.tool_ms = Some(self.tool_ms.unwrap_or(0).saturating_add(took));
        true
    }

    fn first_validation(&mut self, validation: Validation) {
        self.validation.get_or_insert(validation);
    }

    /// The label, from what the turn did.
    pub fn path(&self) -> Path {
        match self.first_speak.map(|(_, kind)| kind) {
            Some(ReplyKind::ConfirmPrompt) => return Path::Confirm,
            Some(ReplyKind::AskFollowUp) => return Path::FollowUp,
            Some(ReplyKind::Countdown) => return Path::Sleep,
            _ => {}
        }
        if self.sleep {
            return Path::Sleep;
        }
        if self.failed {
            Path::Error
        } else if self.search {
            Path::Search
        } else if self.validation == Some(Validation::Reply) {
            Path::Chat
        } else if self.completed {
            Path::Action
        } else {
            Path::Cancelled
        }
    }

    /// The record of this turn, with the host's facts around it.
    pub fn into_record(self, env: &Env) -> Record {
        let path = self.path();
        let voice = self.channel == Channel::Voice;
        let stt_ms = diff(self.t_transcript, self.t_end_of_speech);
        let tts_ms = diff(self.t_first_audio, self.t_speak_sent);
        let total_ms = diff(self.t_first_audio, self.t_end_of_speech);
        // What the named stages do not explain (the hand-offs between them).
        let other_ms = total_ms.map(|total| {
            let named = [stt_ms, self.llm_ms, self.tool_ms, tts_ms]
                .into_iter()
                .flatten()
                .fold(0u64, u64::saturating_add);
            total.saturating_sub(named)
        });
        Record {
            v: RECORD_VERSION,
            at: env.at,
            build: if env.debug { "debug" } else { "release" },
            req: self.req,
            path: path.as_str(),
            channel: self.channel.as_str(),
            lang: env.lang.map(Lang::code),
            validation: self.validation.map(Validation::as_str),
            whisper: env.whisper.clone().filter(|_| voice),
            t_end_of_speech: self.t_end_of_speech,
            t_transcript: self.t_transcript,
            t_request: self.t_request,
            t_llm_done: self.t_llm_done,
            t_tool_done: self.t_tool_done,
            t_speak_sent: self.t_speak_sent,
            t_first_audio: self.t_first_audio,
            stt_ms,
            llm_ms: self.llm_ms,
            tool_ms: self.tool_ms,
            tts_ms,
            other_ms,
            total_ms,
        }
    }
}

/// `later - earlier`, or nothing if either is missing or the clocks disagree.
fn diff(later: Option<u64>, earlier: Option<u64>) -> Option<u64> {
    later?.checked_sub(earlier?)
}

/// The host's facts around a finished turn.
#[derive(Debug, Clone)]
pub struct Env {
    /// Wall clock (epoch milliseconds) when the record is written: the one
    /// field the summary tool can filter a time range by.
    pub at: u64,
    pub debug: bool,
    pub lang: Option<Lang>,
    /// The Whisper model of the running sidecar.
    pub whisper: Option<String>,
}

/// One line of the log. Absent stages are absent keys (never zero, never
/// null), and no field can hold what the user said: every one is a number or
/// a word from a fixed list, bar the Whisper model name.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Record {
    pub v: u32,
    /// Epoch milliseconds when the turn was written (for time ranges).
    pub at: u64,
    pub build: &'static str,
    /// Id of the first request of the turn (the reducer's id; resets per run).
    pub req: u64,
    pub path: &'static str,
    pub channel: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lang: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub whisper: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_end_of_speech: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_transcript: Option<u64>,
    pub t_request: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_llm_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_tool_done: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_speak_sent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_first_audio: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stt_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub other_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_ms: Option<u64>,
}

/// The pure heart: events and effects in, finished turns out.
#[derive(Debug, Default)]
pub struct Tracer {
    turn: Option<Turn>,
    /// What the words of this dispatch were stamped with (`None`: typed).
    pending: Option<Stamp>,
}

impl Tracer {
    pub fn new() -> Self {
        Self::default()
    }

    /// The event the reducer just handled, at host time `t`. Call it before
    /// [`Tracer::on_effects`] for the same dispatch.
    pub fn on_event(&mut self, t: u64, obs: Obs) -> Vec<Turn> {
        // Words only matter for the `StartRequest` of this very dispatch.
        self.pending = None;
        let mut done = Vec::new();
        match obs {
            // The transcript time is the sidecar side's receipt (more exact
            // than the dispatcher's turn at the queue).
            Obs::Transcript { stamp } => self.pending = stamp,
            Obs::TextSubmitted => {}
            Obs::ToolCalls { req, validation } => {
                if let Some(turn) = self.live(req) {
                    turn.llm_done(t);
                    turn.first_validation(validation);
                    if validation == Validation::Invalid {
                        turn.failed = true;
                    }
                }
            }
            Obs::ToolsDone { req, kind } => {
                if let Some(turn) = self.live(req) {
                    turn.t_tool_done = Some(t);
                    turn.tool_done(t);
                    match kind {
                        ToolsKind::Completed => turn.completed = true,
                        ToolsKind::Search => turn.search = true,
                        ToolsKind::Failed => turn.failed = true,
                        ToolsKind::Sleep => turn.sleep = true,
                        ToolsKind::Risky => {}
                    }
                }
            }
            Obs::Reply { req } => {
                if let Some(turn) = self.live(req) {
                    if turn.llm_open {
                        // The model answered in words.
                        turn.llm_done(t);
                        turn.first_validation(Validation::Reply);
                    } else if turn.tool_done(t) {
                        // The assistant's answer to the tools it ran.
                        turn.completed = true;
                    }
                }
            }
            Obs::AskFollowUp { req } => {
                if let Some(turn) = self.live(req) {
                    turn.llm_done(t);
                    turn.first_validation(Validation::FollowUp);
                }
            }
            Obs::Failure { req } => {
                if let Some(turn) = self.live(req) {
                    turn.failed = true;
                    if turn.llm_open {
                        turn.llm_done(t);
                        turn.first_validation(Validation::LlmFailed);
                    } else {
                        turn.tool_done(t);
                    }
                }
            }
            Obs::SpeakStarted { id } => {
                if let Some(turn) = self.turn.as_mut()
                    && turn.first_speak.is_some_and(|(first, _)| first == id)
                    && turn.t_first_audio.is_none()
                {
                    turn.t_first_audio = Some(t);
                    done.extend(self.turn.take());
                }
            }
            Obs::SpeakEnded { id } => {
                if self
                    .turn
                    .as_ref()
                    .and_then(|turn| turn.first_speak)
                    .is_some_and(|(first, _)| first == id)
                {
                    done.extend(self.turn.take());
                }
            }
            Obs::Other => {}
        }
        done
    }

    /// The effects the reducer asked for in the same dispatch, run by `t`.
    pub fn on_effects(&mut self, t: u64, effects: &[FxObs]) -> Vec<Turn> {
        let mut done = Vec::new();
        for effect in effects {
            match *effect {
                FxObs::StartRequest {
                    req,
                    summary: false,
                } => {
                    // A new command ends whatever was still open.
                    done.extend(self.turn.take());
                    self.turn = Some(Turn::begin(req, t, self.pending.take()));
                }
                FxObs::StartRequest { req, summary: true } => {
                    if let Some(turn) = self.turn.as_mut() {
                        turn.current_req = req;
                        turn.llm_started = t;
                        turn.llm_open = true;
                        turn.search = true;
                    }
                }
                FxObs::RunTool { req } => {
                    if let Some(turn) = self.live(req) {
                        turn.tool_started = Some(t);
                    }
                }
                FxObs::CancelRequest { req } => {
                    if let Some(turn) = self.turn.as_mut()
                        && turn.current_req == req
                        && turn.first_speak.is_none()
                    {
                        turn.cancelled = true;
                        done.extend(self.turn.take());
                    }
                }
                FxObs::Speak { id, kind } => {
                    // A reminder alert is not the answer to this turn.
                    if let Some(turn) = self.turn.as_mut()
                        && kind != ReplyKind::Alert
                        && turn.first_speak.is_none()
                    {
                        turn.first_speak = Some((id, kind));
                        turn.t_speak_sent = Some(t);
                    }
                }
                FxObs::StopSpeaking => {
                    // The first speech was stopped before it made a sound.
                    if self
                        .turn
                        .as_ref()
                        .is_some_and(|turn| turn.first_speak.is_some())
                    {
                        done.extend(self.turn.take());
                    }
                }
                FxObs::HideIsland => done.extend(self.turn.take()),
            }
        }
        done
    }

    /// The turn, if `req` is its current request (late results of an older
    /// request are not its business).
    fn live(&mut self, req: u64) -> Option<&mut Turn> {
        self.turn.as_mut().filter(|turn| turn.current_req == req)
    }
}

/// The dispatcher's handle: clocks, lookups, the tracer and where the lines go.
pub struct Latency {
    tracer: Tracer,
    clock: Box<dyn Fn() -> u64 + Send>,
    wall: Box<dyn Fn() -> u64 + Send>,
    stamps: Box<dyn Fn(u64) -> Option<Stamp> + Send>,
    whisper: Box<dyn Fn() -> Option<String> + Send>,
    emit: Box<dyn Fn(String) + Send>,
}

impl Latency {
    /// `clock` MUST be the clock the voice translator was given (that is what
    /// puts the end of speech and everything else on one axis).
    pub fn new(
        clock: Box<dyn Fn() -> u64 + Send>,
        wall: Box<dyn Fn() -> u64 + Send>,
        stamps: Box<dyn Fn(u64) -> Option<Stamp> + Send>,
        whisper: Box<dyn Fn() -> Option<String> + Send>,
        emit: Box<dyn Fn(String) + Send>,
    ) -> Latency {
        Latency {
            tracer: Tracer::new(),
            clock,
            wall,
            stamps,
            whisper,
            emit,
        }
    }

    /// Host time now.
    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    /// An event as the trace sees it. Call it before the reducer takes the
    /// event.
    pub fn observe(&self, event: &Event) -> Obs {
        Obs::of(event, &*self.stamps)
    }

    /// Records one dispatch: `t_event` is when it began, and the effects were
    /// run by now. Never fails and never panics out: a broken trace must not
    /// break a turn.
    pub fn step(&mut self, t_event: u64, obs: Obs, effects: &[FxObs], lang: Option<Lang>) {
        let t_after = (self.clock)();
        let _ = catch_unwind(AssertUnwindSafe(|| {
            let mut done = self.tracer.on_event(t_event, obs);
            done.extend(self.tracer.on_effects(t_after, effects));
            for turn in done {
                let env = Env {
                    at: (self.wall)(),
                    debug: cfg!(debug_assertions),
                    lang,
                    whisper: (self.whisper)(),
                };
                if let Ok(line) = serde_json::to_string(&turn.into_record(&env)) {
                    (self.emit)(line);
                }
            }
        }));
    }
}

#[cfg(test)]
mod tests;
