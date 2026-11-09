use std::sync::{Arc, Mutex};

use niva_core::state_machine::{PendingSlot, Say, Template};
use serde_json::Value;

use super::sink::FileSink;
use super::*;

const SENTINEL: &str = "ZQXJ-SENTINEL-7731";

fn stamp(end: Option<u64>, transcript: u64) -> Option<Stamp> {
    Some(Stamp {
        end_of_speech: end,
        transcript,
    })
}

fn env() -> Env {
    Env {
        at: 1_700_000_000_000,
        debug: false,
        lang: Some(Lang::Es),
        whisper: Some("small".into()),
    }
}

/// Drives the tracer the way the dispatcher does and collects the records.
struct Rig {
    tracer: Tracer,
    records: Vec<Record>,
}

impl Rig {
    fn new() -> Rig {
        Rig {
            tracer: Tracer::new(),
            records: Vec::new(),
        }
    }

    /// One dispatch: the event at `t`, the effects run by `t_after`.
    fn dispatch(&mut self, t: u64, obs: Obs, t_after: u64, effects: &[FxObs]) {
        let mut done = self.tracer.on_event(t, obs);
        done.extend(self.tracer.on_effects(t_after, effects));
        self.records
            .extend(done.into_iter().map(|turn| turn.into_record(&env())));
    }

    fn event(&mut self, t: u64, obs: Obs) {
        self.dispatch(t, obs, t, &[]);
    }

    fn effects(&mut self, t: u64, effects: &[FxObs]) {
        self.dispatch(t, Obs::Other, t, effects);
    }

    /// A voice command heard: speech ended at 1000, the transcript came at
    /// 1250, the request started at 1260.
    fn voice_command(&mut self, req: u64) {
        self.dispatch(
            1_260,
            Obs::Transcript {
                stamp: stamp(Some(1_000), 1_250),
            },
            1_261,
            &[FxObs::StartRequest {
                req,
                summary: false,
            }],
        );
    }

    fn last(&self) -> &Record {
        self.records.last().expect("a record")
    }
}

fn speak(id: u64, kind: ReplyKind) -> FxObs {
    FxObs::Speak { id, kind }
}

fn json(record: &Record) -> Value {
    serde_json::to_value(record).unwrap()
}

// --- the action path ---------------------------------------------------

#[test]
fn an_action_turn_has_every_stage_and_the_derived_durations() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_300,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    r.effects(2_301, &[FxObs::RunTool { req: 5 }]);
    r.event(
        2_400,
        Obs::ToolsDone {
            req: 5,
            kind: ToolsKind::Completed,
        },
    );
    r.effects(2_402, &[speak(9, ReplyKind::Final)]);
    assert!(
        r.records.is_empty(),
        "the turn is open until the first sound"
    );
    r.event(3_100, Obs::SpeakStarted { id: 9 });
    let rec = r.last();
    assert_eq!(rec.path, "action");
    assert_eq!(rec.channel, "voice");
    assert_eq!(rec.req, 5);
    assert_eq!(rec.validation, Some("run_safe"));
    assert_eq!(rec.lang, Some("es"));
    assert_eq!(rec.whisper.as_deref(), Some("small"));
    assert_eq!(rec.t_end_of_speech, Some(1_000));
    assert_eq!(rec.t_transcript, Some(1_250));
    assert_eq!(rec.t_request, 1_261);
    assert_eq!(rec.t_llm_done, Some(2_300));
    assert_eq!(rec.t_tool_done, Some(2_400));
    assert_eq!(rec.t_speak_sent, Some(2_402));
    assert_eq!(rec.t_first_audio, Some(3_100));
    assert_eq!(rec.stt_ms, Some(250));
    assert_eq!(rec.llm_ms, Some(2_300 - 1_261));
    assert_eq!(rec.tool_ms, Some(2_400 - 2_301));
    assert_eq!(rec.tts_ms, Some(3_100 - 2_402));
    assert_eq!(rec.total_ms, Some(2_100));
    // What the named stages leave out: 2100 - (250 + 1039 + 99 + 698).
    assert_eq!(rec.other_ms, Some(14));
}

#[test]
fn the_assistants_reply_after_the_tools_ends_the_action_and_is_not_a_second_model_answer() {
    // What the assistant really sends for a safe action: ToolCalls, then (after
    // RunTool) a Reply, never ToolsDone { Completed }. The first run labelled
    // this `cancelled` and counted the model time twice.
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_300,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    r.effects(2_301, &[FxObs::RunTool { req: 5 }]);
    r.event(2_400, Obs::Reply { req: 5 });
    r.effects(2_402, &[speak(9, ReplyKind::Final)]);
    r.event(3_100, Obs::SpeakStarted { id: 9 });
    let rec = r.last();
    assert_eq!(rec.path, "action");
    assert_eq!(rec.validation, Some("run_safe"));
    assert_eq!(rec.t_llm_done, Some(2_300));
    assert_eq!(rec.t_tool_done, Some(2_400));
    assert_eq!(rec.llm_ms, Some(2_300 - 1_261));
    assert_eq!(rec.tool_ms, Some(2_400 - 2_301));
    assert_eq!(rec.other_ms, Some(14));
}

#[test]
fn a_tool_failure_answer_keeps_the_tool_time_and_the_run_safe_validation() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    r.effects(2_001, &[FxObs::RunTool { req: 5 }]);
    r.event(2_200, Obs::Failure { req: 5 });
    r.effects(2_201, &[speak(3, ReplyKind::Final)]);
    r.event(2_800, Obs::SpeakStarted { id: 3 });
    let rec = r.last();
    assert_eq!(rec.path, "error");
    assert_eq!(rec.validation, Some("run_safe"));
    assert_eq!(rec.tool_ms, Some(199));
    assert_eq!(rec.llm_ms, Some(2_000 - 1_261));
}

#[test]
fn a_failed_model_call_counts_its_time_as_model_time() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(3_300, Obs::Failure { req: 5 });
    r.effects(3_301, &[speak(3, ReplyKind::Final)]);
    r.event(3_900, Obs::SpeakStarted { id: 3 });
    let rec = r.last();
    assert_eq!(rec.validation, Some("llm_failed"));
    assert_eq!(rec.llm_ms, Some(3_300 - 1_261));
    assert_eq!(rec.t_llm_done, Some(3_300));
}

#[test]
fn a_reply_with_no_model_call_and_no_tools_open_changes_nothing() {
    // The words that follow a released sleep countdown: the model call and the
    // tools are long over, so it is neither model time nor tool time.
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    r.effects(2_001, &[FxObs::RunTool { req: 5 }]);
    r.event(
        2_100,
        Obs::ToolsDone {
            req: 5,
            kind: ToolsKind::Sleep,
        },
    );
    r.event(9_000, Obs::Reply { req: 5 });
    let turn = r.tracer.turn.clone().expect("still open");
    let rec = turn.into_record(&env());
    assert_eq!(rec.t_llm_done, Some(2_000));
    assert_eq!(rec.llm_ms, Some(2_000 - 1_261));
    assert_eq!(rec.t_tool_done, Some(2_100));
    assert_eq!(rec.tool_ms, Some(99));
}

#[test]
fn there_is_never_a_first_token_field() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    let record = r.tracer.turn.clone().unwrap().into_record(&env());
    let text = serde_json::to_string(&record).unwrap();
    assert!(!text.contains("first_token"), "{text}");
}

#[test]
fn the_keys_the_summary_tool_reads_are_the_keys_the_record_writes() {
    // tools/latency_summary.py reads these; renaming one here breaks the tool.
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    r.effects(2_001, &[FxObs::RunTool { req: 5 }]);
    r.event(
        2_100,
        Obs::ToolsDone {
            req: 5,
            kind: ToolsKind::Completed,
        },
    );
    r.effects(2_101, &[speak(3, ReplyKind::Final)]);
    r.event(2_800, Obs::SpeakStarted { id: 3 });
    let value = json(r.last());
    for key in [
        "at", "path", "channel", "total_ms", "stt_ms", "llm_ms", "tool_ms", "tts_ms", "other_ms",
    ] {
        assert!(value.get(key).is_some(), "{key} missing: {value}");
    }
}

#[test]
fn the_record_is_one_line_of_json_with_the_fixed_keys() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.effects(1_300, &[speak(2, ReplyKind::Final)]);
    r.event(1_900, Obs::SpeakStarted { id: 2 });
    let line = serde_json::to_string(r.last()).unwrap();
    assert!(!line.contains('\n'));
    let value: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(value["v"], 1);
    assert_eq!(value["build"], "release");
    assert_eq!(value["at"], 1_700_000_000_000u64);
}

// --- the other paths ---------------------------------------------------

#[test]
fn a_reply_in_words_is_the_chat_path() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(2_800, Obs::Reply { req: 5 });
    r.effects(2_801, &[speak(3, ReplyKind::Final)]);
    r.event(3_500, Obs::SpeakStarted { id: 3 });
    let rec = r.last();
    assert_eq!(rec.path, "chat");
    assert_eq!(rec.validation, Some("reply"));
    assert_eq!(rec.t_tool_done, None);
    assert_eq!(rec.tool_ms, None);
    assert_eq!(rec.total_ms, Some(2_500));
}

#[test]
fn a_search_turn_counts_both_model_calls_and_the_tool() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    r.effects(2_001, &[FxObs::RunTool { req: 5 }]);
    r.dispatch(
        2_900,
        Obs::ToolsDone {
            req: 5,
            kind: ToolsKind::Search,
        },
        2_901,
        &[FxObs::StartRequest {
            req: 8,
            summary: true,
        }],
    );
    r.event(4_000, Obs::Reply { req: 8 });
    r.effects(4_001, &[speak(4, ReplyKind::Final)]);
    r.event(4_600, Obs::SpeakStarted { id: 4 });
    let rec = r.last();
    assert_eq!(rec.path, "search");
    assert_eq!(rec.llm_ms, Some((2_000 - 1_261) + (4_000 - 2_901)));
    assert_eq!(rec.tool_ms, Some(2_900 - 2_001));
    assert_eq!(rec.t_llm_done, Some(4_000));
    assert_eq!(rec.t_tool_done, Some(2_900));
    // The first answer's validation is the one kept.
    assert_eq!(rec.validation, Some("run_safe"));
}

#[test]
fn a_confirmation_prompt_is_labelled_confirm_and_not_an_action() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.dispatch(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::Confirm,
        },
        2_001,
        &[speak(6, ReplyKind::ConfirmPrompt)],
    );
    r.event(2_700, Obs::SpeakStarted { id: 6 });
    let rec = r.last();
    assert_eq!(rec.path, "confirm");
    assert_eq!(rec.validation, Some("confirm"));
    assert_eq!(rec.total_ms, Some(1_700));
    assert_eq!(rec.t_tool_done, None);
}

#[test]
fn a_confirmed_action_after_the_prompt_does_not_write_a_second_record() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.dispatch(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::Confirm,
        },
        2_001,
        &[speak(6, ReplyKind::ConfirmPrompt)],
    );
    r.event(2_700, Obs::SpeakStarted { id: 6 });
    r.event(2_900, Obs::SpeakEnded { id: 6 });
    r.effects(5_000, &[FxObs::RunTool { req: 5 }]);
    r.event(
        5_100,
        Obs::ToolsDone {
            req: 5,
            kind: ToolsKind::Completed,
        },
    );
    r.effects(5_101, &[speak(7, ReplyKind::Final)]);
    r.event(5_600, Obs::SpeakStarted { id: 7 });
    r.effects(9_000, &[FxObs::HideIsland]);
    assert_eq!(r.records.len(), 1);
}

#[test]
fn a_follow_up_question_is_its_own_label() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(2_000, Obs::AskFollowUp { req: 5 });
    r.effects(2_001, &[speak(6, ReplyKind::AskFollowUp)]);
    r.event(2_600, Obs::SpeakStarted { id: 6 });
    let rec = r.last();
    assert_eq!(rec.path, "followup");
    assert_eq!(rec.validation, Some("follow_up"));
}

#[test]
fn failures_are_the_error_path() {
    // The model failed.
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(2_000, Obs::Failure { req: 5 });
    r.effects(2_001, &[speak(3, ReplyKind::Final)]);
    r.event(2_500, Obs::SpeakStarted { id: 3 });
    assert_eq!(r.last().path, "error");
    assert_eq!(r.last().validation, Some("llm_failed"));
    // The plan was refused.
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::Invalid,
        },
    );
    r.effects(2_001, &[speak(3, ReplyKind::Final)]);
    r.event(2_500, Obs::SpeakStarted { id: 3 });
    assert_eq!(r.last().path, "error");
    assert_eq!(r.last().validation, Some("invalid"));
    // A tool failed.
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    r.event(
        2_200,
        Obs::ToolsDone {
            req: 5,
            kind: ToolsKind::Failed,
        },
    );
    r.effects(2_201, &[speak(3, ReplyKind::Final)]);
    r.event(2_800, Obs::SpeakStarted { id: 3 });
    assert_eq!(r.last().path, "error");
}

// --- turns that end without a sound -------------------------------------

#[test]
fn a_turn_with_no_sound_has_no_audio_fields_and_nothing_invented() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolCalls {
            req: 5,
            validation: Validation::RunSafe,
        },
    );
    r.effects(2_001, &[FxObs::RunTool { req: 5 }]);
    r.event(
        2_100,
        Obs::ToolsDone {
            req: 5,
            kind: ToolsKind::Completed,
        },
    );
    // Voice is down: the speech is reported done at once.
    r.effects(2_101, &[speak(3, ReplyKind::Final)]);
    r.event(2_102, Obs::SpeakEnded { id: 3 });
    assert_eq!(r.records.len(), 1);
    let rec = r.last();
    assert_eq!(rec.path, "action");
    assert_eq!(rec.t_first_audio, None);
    assert_eq!(rec.tts_ms, None);
    assert_eq!(rec.total_ms, None);
    assert_eq!(rec.other_ms, None);
    let value = json(rec);
    for key in ["t_first_audio", "tts_ms", "total_ms", "other_ms"] {
        assert!(value.get(key).is_none(), "{key} must be absent: {value}");
    }
    // The stages that did happen are there.
    assert_eq!(rec.t_speak_sent, Some(2_101));
    assert_eq!(rec.stt_ms, Some(250));
}

#[test]
fn a_typed_command_is_the_text_channel_with_no_speech_fields() {
    let mut r = Rig::new();
    r.dispatch(
        1_000,
        Obs::TextSubmitted,
        1_001,
        &[FxObs::StartRequest {
            req: 2,
            summary: false,
        }],
    );
    r.event(1_800, Obs::Reply { req: 2 });
    r.effects(1_801, &[speak(3, ReplyKind::Final)]);
    r.event(1_802, Obs::SpeakEnded { id: 3 });
    let rec = r.last();
    assert_eq!(rec.channel, "text");
    assert_eq!(rec.path, "chat");
    assert_eq!(rec.t_end_of_speech, None);
    assert_eq!(rec.t_transcript, None);
    assert_eq!(rec.stt_ms, None);
    assert_eq!(rec.total_ms, None);
    // No speech model is in use for a typed command.
    assert_eq!(rec.whisper, None);
    assert_eq!(rec.llm_ms, Some(799));
}

#[test]
fn a_transcript_the_sidecar_never_stamped_counts_as_text() {
    let mut r = Rig::new();
    r.dispatch(
        1_000,
        Obs::Transcript { stamp: None },
        1_001,
        &[FxObs::StartRequest {
            req: 2,
            summary: false,
        }],
    );
    r.effects(1_002, &[FxObs::HideIsland]);
    assert_eq!(r.last().channel, "text");
}

#[test]
fn a_speech_end_before_the_clock_mapping_leaves_the_speech_fields_out() {
    let mut r = Rig::new();
    r.dispatch(
        1_260,
        Obs::Transcript {
            stamp: stamp(None, 1_250),
        },
        1_261,
        &[FxObs::StartRequest {
            req: 5,
            summary: false,
        }],
    );
    r.effects(1_300, &[speak(2, ReplyKind::Final)]);
    r.event(1_900, Obs::SpeakStarted { id: 2 });
    let rec = r.last();
    assert_eq!(rec.channel, "voice");
    assert_eq!(rec.t_end_of_speech, None);
    assert_eq!(rec.stt_ms, None);
    assert_eq!(rec.total_ms, None);
    assert_eq!(rec.tts_ms, Some(600));
}

#[test]
fn a_cancelled_request_is_written_as_cancelled() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.effects(1_500, &[FxObs::CancelRequest { req: 5 }]);
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.last().path, "cancelled");
    assert_eq!(r.last().t_first_audio, None);
}

#[test]
fn a_new_command_closes_the_one_still_open() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.voice_command(9);
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.records[0].req, 5);
    assert_eq!(r.records[0].path, "cancelled");
    r.effects(2_000, &[FxObs::HideIsland]);
    assert_eq!(r.records.len(), 2);
    assert_eq!(r.records[1].req, 9);
}

#[test]
fn stopping_the_first_speech_before_a_sound_ends_the_turn() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(1_800, Obs::Reply { req: 5 });
    r.effects(1_801, &[speak(3, ReplyKind::Final)]);
    r.effects(1_900, &[FxObs::StopSpeaking]);
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.last().path, "chat");
    assert_eq!(r.last().t_first_audio, None);
}

#[test]
fn late_and_foreign_events_change_nothing() {
    let mut r = Rig::new();
    r.voice_command(5);
    // A result for another request.
    r.event(
        1_500,
        Obs::ToolCalls {
            req: 99,
            validation: Validation::Invalid,
        },
    );
    r.event(1_501, Obs::Reply { req: 99 });
    r.event(1_502, Obs::SpeakStarted { id: 77 });
    r.event(1_503, Obs::SpeakEnded { id: 77 });
    r.event(1_504, Obs::Failure { req: 99 });
    assert!(r.records.is_empty());
    r.effects(1_600, &[speak(2, ReplyKind::Final)]);
    r.event(1_700, Obs::SpeakStarted { id: 2 });
    // The same sound reported again, and a speech done afterwards.
    r.event(1_701, Obs::SpeakStarted { id: 2 });
    r.event(1_800, Obs::SpeakEnded { id: 2 });
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.last().t_first_audio, Some(1_700));
}

#[test]
fn a_reminder_alert_cannot_hijack_an_open_turn() {
    let mut r = Rig::new();
    r.voice_command(5);
    // An alert is spoken while the turn waits for the model.
    r.effects(1_400, &[speak(20, ReplyKind::Alert)]);
    r.event(1_500, Obs::SpeakStarted { id: 20 });
    r.event(1_600, Obs::SpeakEnded { id: 20 });
    assert!(r.records.is_empty(), "the alert must not close the turn");
    r.event(2_000, Obs::Reply { req: 5 });
    r.effects(2_001, &[speak(21, ReplyKind::Final)]);
    r.event(2_700, Obs::SpeakStarted { id: 21 });
    assert_eq!(r.records.len(), 1);
    assert_eq!(r.last().path, "chat");
    assert_eq!(r.last().t_first_audio, Some(2_700));
    assert_eq!(r.last().t_speak_sent, Some(2_001));
}

#[test]
fn a_sleep_countdown_announcement_is_the_sleep_path_not_cancelled() {
    let mut r = Rig::new();
    r.voice_command(5);
    r.event(
        2_000,
        Obs::ToolsDone {
            req: 5,
            kind: ToolsKind::Sleep,
        },
    );
    r.effects(2_001, &[speak(6, ReplyKind::Countdown)]);
    r.event(2_600, Obs::SpeakStarted { id: 6 });
    assert_eq!(r.last().path, "sleep");
    assert_eq!(r.last().t_first_audio, Some(2_600));
}

#[test]
fn a_reminder_alert_with_no_turn_open_writes_nothing() {
    let mut r = Rig::new();
    r.effects(100, &[speak(1, ReplyKind::Alert)]);
    r.event(200, Obs::SpeakStarted { id: 1 });
    r.event(300, Obs::SpeakEnded { id: 1 });
    assert!(r.records.is_empty());
}

#[test]
fn clocks_that_disagree_give_no_duration_instead_of_a_wrapped_one() {
    let mut r = Rig::new();
    // The end of speech maps AFTER the transcript (a bad clock mapping).
    r.dispatch(
        1_260,
        Obs::Transcript {
            stamp: stamp(Some(5_000), 1_250),
        },
        1_261,
        &[FxObs::StartRequest {
            req: 5,
            summary: false,
        }],
    );
    r.effects(1_300, &[speak(2, ReplyKind::Final)]);
    r.event(1_900, Obs::SpeakStarted { id: 2 });
    let rec = r.last();
    assert_eq!(rec.stt_ms, None);
    assert_eq!(rec.total_ms, None);
    assert_eq!(rec.other_ms, None);
    assert_eq!(rec.tts_ms, Some(600));
}

// --- how events become observations ------------------------------------

fn no_stamps() -> impl Fn(u64) -> Option<Stamp> {
    |_| None
}

#[test]
fn the_watchdog_of_a_speech_ends_it_but_other_timers_do_not() {
    let wd = Event::Watchdog {
        kind: TimerKind::SpeakWatchdog,
        token: 4,
    };
    assert_eq!(Obs::of(&wd, &no_stamps()), Obs::SpeakEnded { id: 4 });
    let other = Event::Watchdog {
        kind: TimerKind::Settle,
        token: 4,
    };
    assert_eq!(Obs::of(&other, &no_stamps()), Obs::Other);
}

#[test]
fn a_transcript_takes_its_stamp_by_listen_id() {
    let event = Event::Transcript {
        id: 3,
        text: "x".into(),
        lang: None,
    };
    let found = Obs::of(&event, &|id| {
        (id == 3).then_some(Stamp {
            end_of_speech: Some(1),
            transcript: 2,
        })
    });
    assert_eq!(
        found,
        Obs::Transcript {
            stamp: stamp(Some(1), 2)
        }
    );
    assert_eq!(Obs::of(&event, &|_| None), Obs::Transcript { stamp: None });
}

/// Events and effects that carry the sentinel in every text they can hold.
fn hostile_events() -> Vec<Event> {
    let text = format!("open {SENTINEL} and {SENTINEL}");
    vec![
        Event::Transcript {
            id: 1,
            text: text.clone(),
            lang: Some(Lang::En),
        },
        Event::TextSubmitted { text: text.clone() },
        Event::Reply {
            req: 1,
            text: text.clone(),
        },
        Event::FailureText {
            req: 1,
            text: text.clone(),
        },
        Event::AskFollowUp {
            req: 1,
            question: text.clone(),
            slot: PendingSlot::Reminder { text: None },
        },
        Event::ToolCalls {
            req: 1,
            verdict: ToolVerdict::Confirm {
                prompt: text.clone(),
                click_only: false,
                cancel_text: Some(text.clone()),
            },
        },
        Event::ToolsDone {
            req: 1,
            outcome: ToolsOutcome::RiskyRemaining {
                prompt: text.clone(),
                click_only: false,
                cancel_text: Some(text),
            },
        },
    ]
}

fn hostile_effects() -> Vec<Effect> {
    let text = format!("close {SENTINEL}");
    vec![
        Effect::StartRequest {
            req: 1,
            input: RequestInput::User {
                text: text.clone(),
                slot: None,
                lang: Some(Lang::Es),
            },
        },
        Effect::Speak {
            id: 1,
            kind: ReplyKind::Final,
            say: Say::Text(text),
        },
        Effect::Speak {
            id: 2,
            kind: ReplyKind::Final,
            say: Say::Template(Template::ActionDone),
        },
    ]
}

#[test]
fn an_observation_cannot_hold_the_text_of_an_event_or_an_effect() {
    for event in hostile_events() {
        let obs = Obs::of(&event, &no_stamps());
        assert!(!format!("{obs:?}").contains(SENTINEL), "{obs:?}");
    }
    let fx = FxObs::of(&hostile_effects());
    assert!(!format!("{fx:?}").contains(SENTINEL), "{fx:?}");
}

#[test]
fn hostile_events_driven_through_latency_step_never_reach_the_file() {
    let root = std::env::temp_dir().join(format!("niva-latency-sentinel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let path = root.join("latency.jsonl");
    let sink = Arc::new(Mutex::new(FileSink::new(path.clone(), 1 << 20)));
    let now = Arc::new(Mutex::new(0u64));
    let clock = {
        let now = now.clone();
        Box::new(move || {
            let mut now = now.lock().unwrap();
            *now += 10;
            *now
        })
    };
    let mut latency = Latency::new(
        clock,
        Box::new(|| 1_700_000_000_000),
        Box::new(|_| stamp(Some(5), 8)),
        Box::new(|| Some("small".into())),
        {
            let sink = sink.clone();
            Box::new(move |line| {
                sink.lock().unwrap().append(&line).unwrap();
            })
        },
    );
    // Several turns with the sentinel in every text the reducer can carry.
    for round in 0..3u64 {
        for event in hostile_events() {
            let t = latency.now();
            let obs = latency.observe(&event);
            let fx = FxObs::of(&hostile_effects());
            latency.step(t, obs, &fx, Some(Lang::En));
        }
        let t = latency.now();
        latency.step(
            t,
            Obs::SpeakStarted { id: 1 + round },
            &[FxObs::HideIsland],
            None,
        );
    }
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.lines().count() >= 1, "the pipeline wrote nothing");
    assert!(!written.contains(SENTINEL), "{written}");
    assert!(
        !written.contains("open ") && !written.contains("close "),
        "{written}"
    );
    for line in written.lines() {
        let value: Value = serde_json::from_str(line).unwrap();
        // Every value is a number or a word from a short fixed list.
        for (key, field) in value.as_object().unwrap() {
            assert!(
                field.is_number()
                    || matches!(
                        key.as_str(),
                        "build" | "path" | "channel" | "lang" | "validation" | "whisper"
                    ),
                "unexpected text field {key}: {field}"
            );
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_panicking_sink_never_breaks_the_turn() {
    let mut latency = Latency::new(
        Box::new(|| 1),
        Box::new(|| 2),
        Box::new(|_| None),
        Box::new(|| None),
        Box::new(|_| panic!("the disk is on fire")),
    );
    let fx = [
        FxObs::StartRequest {
            req: 1,
            summary: false,
        },
        FxObs::HideIsland,
    ];
    // The panic stays inside `step`, which comes back as if nothing happened.
    latency.step(1, Obs::TextSubmitted, &fx, None);
    latency.step(2, Obs::Other, &[], None);
}
