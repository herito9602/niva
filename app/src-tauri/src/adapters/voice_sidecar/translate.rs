//! Sidecar message -> reducer events, pure but for the clock it is given.
//!
//! This is where every message of the protocol gets its meaning for the host:
//!
//! | sidecar | host |
//! |---|---|
//! | `ready{now_ms}` | the clock mapping, and [`Signal::Ready`] for the policy (which turns voice on) |
//! | `wake` | `Event::Wake` |
//! | `speech_start{id}` | `Event::SpeechStart{id}` |
//! | `transcript{id,..}` | `Event::Transcript`, or `Event::EmptyTranscript` for no words |
//! | `no_speech{id}` | `Event::NoSpeech{id}` |
//! | `speak_started{id}`, `speak_done{id}` | `Event::SpeakStarted`, `Event::SpeakDone` |
//! | `error{TTS_FAILED, id}` | `Event::TtsFailed{id}` |
//! | `error{code, id?}` of the six codes | `Event::SidecarError{code, id}` and [`Signal::Error`] |
//! | `error{unknown code}` | treated as `INTERNAL` |
//!
//! Nothing here logs what the user said or what Niva answers: only counts,
//! ids, the language and timestamps.

use std::sync::{Arc, Mutex, PoisonError};

use niva_core::state_machine::{Event, ListenMode, SidecarErrorCode};

use super::policy::Signal;
use super::wire::Incoming;

/// Below this combined es + en probability the log says so. Nothing is dropped
/// for it: the number is only evidence for tuning.
const LOW_LANG_PROB: f64 = 0.5;

/// Milliseconds on the host's monotonic clock.
pub type HostClock = Arc<dyn Fn() -> u64 + Send + Sync>;
pub type Log = Arc<dyn Fn(String) + Send + Sync>;

/// The mode of the listen in flight, written when the host sends `listen` and
/// read when its transcript comes back. Only the latest listen matters: an
/// answer to an older one is dropped by the reducer's stale-id guard anyway.
#[derive(Debug, Default)]
pub struct ListenModes(Mutex<Option<(u64, ListenMode)>>);

impl ListenModes {
    pub fn set(&self, id: u64, mode: ListenMode) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some((id, mode));
    }

    pub fn get(&self, id: u64) -> Option<ListenMode> {
        match *self.0.lock().unwrap_or_else(PoisonError::into_inner) {
            Some((latest, mode)) if latest == id => Some(mode),
            _ => None,
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Translated {
    pub events: Vec<Event>,
    /// What the restart policy must hear about.
    pub signal: Option<Signal>,
}

impl Translated {
    fn event(event: Event) -> Self {
        Translated {
            events: vec![event],
            signal: None,
        }
    }
}

pub struct Translator {
    clock: HostClock,
    log: Log,
    modes: Arc<ListenModes>,
    /// `host_ms = sidecar_ms + offset`, set by each `ready`.
    offset_ms: Option<i64>,
}

impl Translator {
    pub fn new(clock: HostClock, log: Log, modes: Arc<ListenModes>) -> Self {
        Translator {
            clock,
            log,
            modes,
            offset_ms: None,
        }
    }

    /// A sidecar clock reading on the host's clock. `None` before the first
    /// `ready`. The mapping is exact up to the pipe latency of that `ready`.
    pub fn to_host_ms(&self, sidecar_ms: u64) -> Option<i64> {
        self.offset_ms.map(|offset| {
            i64::try_from(sidecar_ms)
                .unwrap_or(i64::MAX)
                .saturating_add(offset)
        })
    }

    pub fn translate(&mut self, incoming: Incoming) -> Translated {
        match incoming {
            Incoming::Ready { now_ms } => {
                let host = i64::try_from((self.clock)()).unwrap_or(i64::MAX);
                self.offset_ms = Some(host - i64::try_from(now_ms).unwrap_or(i64::MAX));
                Translated {
                    events: Vec::new(),
                    signal: Some(Signal::Ready),
                }
            }
            Incoming::Wake { score } => {
                (self.log)(format!("[niva-voice] wake score={score:.2}"));
                Translated::event(Event::Wake)
            }
            Incoming::SpeechStart { id } => Translated::event(Event::SpeechStart { id }),
            Incoming::Transcript {
                id,
                text,
                lang,
                lang_prob,
                end_ms,
            } => {
                let received = (self.clock)();
                let mode = self.modes.get(id);
                let words = text.as_str().trim();
                // No gate on `lang_prob` (design D1, 2026-11-07): a real Spanish
                // command measured 0.07 and was thrown away. The sidecar already
                // picked the higher of es and en, and noise is its no-speech
                // gate's job, which ends in an empty transcript.
                let event = if words.is_empty() {
                    Event::EmptyTranscript { id }
                } else {
                    Event::Transcript {
                        id,
                        text: words.to_owned(),
                        lang,
                    }
                };
                // The per-request timestamps (design "Metric logging"): the end
                // of speech on the host's clock, and when the transcript arrived.
                let end = self
                    .to_host_ms(end_ms)
                    .map_or_else(|| "?".to_owned(), |t| t.to_string());
                (self.log)(format!(
                    "[niva-voice] transcript id={id} mode={} chars={} lang={} lang_prob={lang_prob:.2} \
                     t_end_of_speech={end} t_transcript={received}{}",
                    mode_name(mode),
                    words.chars().count(),
                    lang.map_or("?", |l| l.code()),
                    if lang_prob < LOW_LANG_PROB {
                        " (low language probability: processed anyway)"
                    } else {
                        ""
                    },
                ));
                Translated::event(event)
            }
            Incoming::NoSpeech { id } => Translated::event(Event::NoSpeech { id }),
            Incoming::SpeakStarted { id } => {
                (self.log)(format!(
                    "[niva-voice] speak_started id={id} t_first_audio={}",
                    (self.clock)()
                ));
                Translated::event(Event::SpeakStarted { id })
            }
            Incoming::SpeakDone { id } => Translated::event(Event::SpeakDone { id }),
            Incoming::Error { code, msg, id } => self.error(&code, &msg, id),
        }
    }

    fn error(&self, code: &str, msg: &str, id: Option<u64>) -> Translated {
        if code == "TTS_FAILED" {
            // The message of a speech failure can quote what was being said.
            return match id {
                Some(id) => {
                    (self.log)(format!("[niva-voice] TTS_FAILED id={id}"));
                    Translated::event(Event::TtsFailed { id })
                }
                None => {
                    (self.log)("[niva-voice] TTS_FAILED without an id: ignored".into());
                    Translated::default()
                }
            };
        }
        let known = SidecarErrorCode::from_wire(code);
        let code_name: String = code.chars().take(40).collect();
        // Only the code is logged: the sidecar's message is free text.
        (self.log)(match known {
            Some(_) => format!("[niva-voice] error {code_name} id={id:?}"),
            None => format!("[niva-voice] unknown error code {code_name:?}, treated as INTERNAL"),
        });
        let code = known.unwrap_or(SidecarErrorCode::Internal);
        let memory = msg.to_lowercase().contains("memory");
        Translated {
            events: vec![Event::SidecarError { code, id }],
            signal: Some(Signal::Error { code, memory }),
        }
    }
}

fn mode_name(mode: Option<ListenMode>) -> &'static str {
    match mode {
        Some(ListenMode::Command) => "command",
        Some(ListenMode::Confirm) => "confirm",
        None => "?",
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use niva_core::replies::Lang;

    use super::*;
    use crate::adapters::voice_sidecar::wire::Private;

    struct Rig {
        t: Translator,
        now: Arc<AtomicU64>,
        modes: Arc<ListenModes>,
        logs: Arc<Mutex<Vec<String>>>,
    }

    fn rig() -> Rig {
        let now = Arc::new(AtomicU64::new(1_000));
        let logs = Arc::new(Mutex::new(Vec::new()));
        let modes = Arc::new(ListenModes::default());
        let clock: HostClock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let log: Log = {
            let logs = logs.clone();
            Arc::new(move |line| logs.lock().unwrap().push(line))
        };
        Rig {
            t: Translator::new(clock, log, modes.clone()),
            now,
            modes,
            logs,
        }
    }

    fn transcript(id: u64, text: &str, prob: f64) -> Incoming {
        Incoming::Transcript {
            id,
            text: Private::new(text),
            lang: Some(Lang::En),
            lang_prob: prob,
            end_ms: 500,
        }
    }

    fn events(r: &mut Rig, incoming: Incoming) -> Vec<Event> {
        r.t.translate(incoming).events
    }

    #[test]
    fn simple_messages_map_to_their_events() {
        let mut r = rig();
        assert_eq!(
            events(&mut r, Incoming::Wake { score: 0.7 }),
            vec![Event::Wake]
        );
        assert_eq!(
            events(&mut r, Incoming::SpeechStart { id: 4 }),
            vec![Event::SpeechStart { id: 4 }]
        );
        assert_eq!(
            events(&mut r, Incoming::NoSpeech { id: 4 }),
            vec![Event::NoSpeech { id: 4 }]
        );
        assert_eq!(
            events(&mut r, Incoming::SpeakStarted { id: 5 }),
            vec![Event::SpeakStarted { id: 5 }]
        );
        assert_eq!(
            events(&mut r, Incoming::SpeakDone { id: 5 }),
            vec![Event::SpeakDone { id: 5 }]
        );
    }

    #[test]
    fn ready_sends_no_event_but_tells_the_policy() {
        let mut r = rig();
        let got = r.t.translate(Incoming::Ready { now_ms: 10 });
        assert!(got.events.is_empty());
        assert_eq!(got.signal, Some(Signal::Ready));
    }

    #[test]
    fn end_of_speech_is_mapped_to_the_host_clock_with_ready() {
        let mut r = rig();
        assert_eq!(r.t.to_host_ms(500), None);
        // At `ready` the host clock reads 1000 and the sidecar's reads 400.
        r.t.translate(Incoming::Ready { now_ms: 400 });
        assert_eq!(r.t.to_host_ms(400), Some(1_000));
        assert_eq!(r.t.to_host_ms(900), Some(1_500));
        // A sidecar that started earlier than the host's zero maps below zero.
        r.t.translate(Incoming::Ready { now_ms: 5_000 });
        assert_eq!(r.t.to_host_ms(4_000), Some(0));
        assert_eq!(r.t.to_host_ms(100), Some(-3_900));
    }

    #[test]
    fn a_later_ready_replaces_the_mapping() {
        let mut r = rig();
        r.t.translate(Incoming::Ready { now_ms: 400 });
        r.now.store(70_000, Ordering::SeqCst);
        r.t.translate(Incoming::Ready { now_ms: 60_100 });
        assert_eq!(r.t.to_host_ms(60_100), Some(70_000));
    }

    #[test]
    fn the_transcript_log_has_the_timestamps_and_no_words() {
        let mut r = rig();
        r.t.translate(Incoming::Ready { now_ms: 400 });
        r.modes.set(7, ListenMode::Command);
        r.now.store(2_000, Ordering::SeqCst);
        r.t.translate(Incoming::Transcript {
            id: 7,
            text: Private::new("open the secret spreadsheet"),
            lang: Some(Lang::En),
            lang_prob: 0.97,
            end_ms: 1_000,
        });
        let logs = r.logs.lock().unwrap().join("\n");
        assert!(logs.contains("t_end_of_speech=1600"), "{logs}");
        assert!(logs.contains("t_transcript=2000"), "{logs}");
        assert!(logs.contains("chars=27"), "{logs}");
        assert!(
            !logs.contains("secret") && !logs.contains("spreadsheet"),
            "{logs}"
        );
    }

    #[test]
    fn a_command_transcript_goes_to_the_reducer_trimmed() {
        let mut r = rig();
        r.modes.set(7, ListenMode::Command);
        assert_eq!(
            events(&mut r, transcript(7, "  abre Spotify \n", 0.95)),
            vec![Event::Transcript {
                id: 7,
                text: "abre Spotify".into(),
                lang: Some(Lang::En)
            }]
        );
    }

    #[test]
    fn no_words_is_an_empty_transcript_in_both_modes() {
        for mode in [ListenMode::Command, ListenMode::Confirm] {
            let mut r = rig();
            r.modes.set(7, mode);
            assert_eq!(
                events(&mut r, transcript(7, "  \n", 0.95)),
                vec![Event::EmptyTranscript { id: 7 }]
            );
        }
    }

    #[test]
    fn a_command_with_a_low_language_probability_is_still_processed() {
        // The live run: a real Spanish command had es + en = 0.07 and was dropped.
        let mut r = rig();
        r.modes.set(7, ListenMode::Command);
        assert_eq!(
            events(&mut r, transcript(7, "abre la calculadora", 0.07)),
            vec![Event::Transcript {
                id: 7,
                text: "abre la calculadora".into(),
                lang: Some(Lang::En)
            }]
        );
        let logs = r.logs.lock().unwrap().join("\n");
        assert!(logs.contains("lang_prob=0.07"), "{logs}");
        assert!(!logs.contains("calculadora"), "{logs}");
    }

    #[test]
    fn a_command_with_no_words_is_empty_whatever_its_language_probability() {
        let mut r = rig();
        r.modes.set(7, ListenMode::Command);
        assert_eq!(
            events(&mut r, transcript(7, "", 0.99)),
            vec![Event::EmptyTranscript { id: 7 }]
        );
    }

    #[test]
    fn a_confirmation_answer_does_not_depend_on_the_language_probability() {
        let mut r = rig();
        r.modes.set(8, ListenMode::Confirm);
        assert_eq!(
            events(&mut r, transcript(8, "yes", 0.1)),
            vec![Event::Transcript {
                id: 8,
                text: "yes".into(),
                lang: Some(Lang::En)
            }]
        );
    }

    #[test]
    fn a_transcript_of_an_older_listen_goes_through_for_the_reducer_to_drop() {
        let mut r = rig();
        r.modes.set(9, ListenMode::Command);
        // Mode unknown for id 3: not gated, not invented; the stale-id guard decides.
        assert_eq!(
            events(&mut r, transcript(3, "hola", 0.1)),
            vec![Event::Transcript {
                id: 3,
                text: "hola".into(),
                lang: Some(Lang::En)
            }]
        );
    }

    fn error(code: &str, msg: &str, id: Option<u64>) -> Incoming {
        Incoming::Error {
            code: code.into(),
            msg: msg.into(),
            id,
        }
    }

    #[test]
    fn the_six_codes_become_sidecar_errors_with_their_id() {
        for code in SidecarErrorCode::ALL {
            let mut r = rig();
            let got = r.t.translate(error(code.wire(), "boom", Some(3)));
            assert_eq!(got.events, vec![Event::SidecarError { code, id: Some(3) }]);
            assert_eq!(
                got.signal,
                Some(Signal::Error {
                    code,
                    memory: false
                })
            );
        }
    }

    #[test]
    fn a_process_level_error_has_no_id() {
        let mut r = rig();
        let got = r.t.translate(error("MIC_MISSING", "no microphone", None));
        assert_eq!(
            got.events,
            vec![Event::SidecarError {
                code: SidecarErrorCode::MicMissing,
                id: None
            }]
        );
    }

    #[test]
    fn an_unknown_code_is_internal() {
        let mut r = rig();
        let got = r.t.translate(error("DISK_ON_FIRE", "x", Some(1)));
        assert_eq!(
            got.events,
            vec![Event::SidecarError {
                code: SidecarErrorCode::Internal,
                id: Some(1)
            }]
        );
        assert!(r.logs.lock().unwrap().join("\n").contains("DISK_ON_FIRE"));
    }

    #[test]
    fn tts_failed_is_not_a_voice_down_error() {
        let mut r = rig();
        let got =
            r.t.translate(error("TTS_FAILED", "could not say: my secret", Some(5)));
        assert_eq!(got.events, vec![Event::TtsFailed { id: 5 }]);
        assert_eq!(got.signal, None);
        // The message can quote the reply: it is not logged.
        assert!(!r.logs.lock().unwrap().join("\n").contains("secret"));
    }

    #[test]
    fn tts_failed_without_an_id_is_dropped() {
        let mut r = rig();
        assert_eq!(
            r.t.translate(error("TTS_FAILED", "", None)),
            Translated::default()
        );
    }

    #[test]
    fn a_model_load_message_about_memory_is_flagged_for_the_policy() {
        let mut r = rig();
        let got =
            r.t.translate(error("MODEL_LOAD", "CUDA failed: Out Of Memory", None));
        assert_eq!(
            got.signal,
            Some(Signal::Error {
                code: SidecarErrorCode::ModelLoad,
                memory: true
            })
        );
    }

    #[test]
    fn the_error_log_has_the_code_but_not_the_message() {
        let mut r = rig();
        r.t.translate(error("MIC_BUSY", "free text with words", Some(3)));
        let logs = r.logs.lock().unwrap().join("\n");
        assert!(
            logs.contains("MIC_BUSY") && !logs.contains("free text"),
            "{logs}"
        );
    }

    #[test]
    fn the_detected_language_travels_with_the_command() {
        let mut r = rig();
        r.modes.set(7, ListenMode::Command);
        let got = events(
            &mut r,
            Incoming::Transcript {
                id: 7,
                text: Private::new("siguiente canción"),
                lang: Some(Lang::Es),
                lang_prob: 0.99,
                end_ms: 10,
            },
        );
        assert_eq!(
            got,
            vec![Event::Transcript {
                id: 7,
                text: "siguiente canción".into(),
                lang: Some(Lang::Es)
            }]
        );
    }

    #[test]
    fn the_modes_remember_only_the_latest_listen() {
        let modes = ListenModes::default();
        assert_eq!(modes.get(1), None);
        modes.set(1, ListenMode::Command);
        modes.set(2, ListenMode::Confirm);
        assert_eq!(modes.get(1), None);
        assert_eq!(modes.get(2), Some(ListenMode::Confirm));
    }
}
