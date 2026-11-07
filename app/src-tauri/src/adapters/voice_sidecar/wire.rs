//! The JSON-lines protocol, host side (the sidecar's side is `voice/protocol.py`,
//! which is the contract; a test below reads it and checks the names).
//!
//! Pure: parsing and encoding only. A line that cannot be read becomes a
//! [`ParseError`] that names the problem and carries none of the line, because
//! a transcript line holds what the user said and must never reach a log.

use std::fmt;

use niva_core::replies::Lang;
use niva_core::state_machine::ListenMode;
use serde_json::{Value, json};

/// Longest line the host accepts from the sidecar. A transcript is a few
/// hundred bytes; anything near this is a runaway process, not a message.
pub const MAX_LINE_BYTES: usize = 256 * 1024;

/// Text that must never be printed: `Debug` shows only its size.
#[derive(Clone, PartialEq, Eq)]
pub struct Private(String);

impl Private {
    pub fn new(text: impl Into<String>) -> Self {
        Private(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Private {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{} chars>", self.0.chars().count())
    }
}

/// What the sidecar says.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// The microphone delivered its first frame. `now_ms` is the sidecar's
    /// monotonic clock at that moment (the clock `end_ms` is on).
    Ready {
        now_ms: u64,
    },
    Wake {
        score: f64,
    },
    SpeechStart {
        id: u64,
    },
    Transcript {
        id: u64,
        text: Private,
        lang: Option<Lang>,
        /// Combined es + en probability.
        lang_prob: f64,
        end_ms: u64,
    },
    NoSpeech {
        id: u64,
    },
    SpeakStarted {
        id: u64,
    },
    SpeakDone {
        id: u64,
    },
    Error {
        code: String,
        msg: String,
        id: Option<u64>,
    },
}

/// Why a line was not a message. Never carries the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    TooLong,
    NotJson,
    NotAnObject,
    /// No `type`, or one this host does not know.
    UnknownType,
    /// A field is missing or has the wrong type or range.
    BadField(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::TooLong => write!(f, "line too long"),
            ParseError::NotJson => write!(f, "not JSON"),
            ParseError::NotAnObject => write!(f, "not a JSON object"),
            ParseError::UnknownType => write!(f, "unknown message type"),
            ParseError::BadField(name) => write!(f, "bad field '{name}'"),
        }
    }
}

fn uint(obj: &Value, key: &'static str) -> Result<u64, ParseError> {
    obj.get(key)
        .and_then(Value::as_u64)
        .ok_or(ParseError::BadField(key))
}

fn string(obj: &Value, key: &'static str) -> Result<String, ParseError> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or(ParseError::BadField(key))
}

fn number(obj: &Value, key: &'static str) -> Result<f64, ParseError> {
    obj.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .ok_or(ParseError::BadField(key))
}

/// Reads one line (without its newline) from the sidecar.
pub fn parse_line(line: &str) -> Result<Incoming, ParseError> {
    if line.len() > MAX_LINE_BYTES {
        return Err(ParseError::TooLong);
    }
    let value: Value = serde_json::from_str(line).map_err(|_| ParseError::NotJson)?;
    if !value.is_object() {
        return Err(ParseError::NotAnObject);
    }
    let kind = value
        .get("type")
        .and_then(Value::as_str)
        .ok_or(ParseError::UnknownType)?;
    Ok(match kind {
        "ready" => Incoming::Ready {
            now_ms: uint(&value, "now_ms")?,
        },
        "wake" => Incoming::Wake {
            score: number(&value, "score")?,
        },
        "speech_start" => Incoming::SpeechStart {
            id: uint(&value, "id")?,
        },
        "transcript" => Incoming::Transcript {
            id: uint(&value, "id")?,
            text: Private::new(string(&value, "text")?),
            lang: match value.get("lang").and_then(Value::as_str) {
                Some("es") => Some(Lang::Es),
                Some("en") => Some(Lang::En),
                _ => None,
            },
            lang_prob: number(&value, "lang_prob")?,
            end_ms: uint(&value, "end_ms")?,
        },
        "no_speech" => Incoming::NoSpeech {
            id: uint(&value, "id")?,
        },
        "speak_started" => Incoming::SpeakStarted {
            id: uint(&value, "id")?,
        },
        "speak_done" => Incoming::SpeakDone {
            id: uint(&value, "id")?,
        },
        "error" => Incoming::Error {
            code: string(&value, "code")?,
            msg: value
                .get("msg")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            id: match value.get("id") {
                None | Some(Value::Null) => None,
                Some(_) => Some(uint(&value, "id")?),
            },
        },
        _ => return Err(ParseError::UnknownType),
    })
}

/// What the host says.
#[derive(Clone, PartialEq, Eq)]
pub enum Outgoing {
    /// `context` is only for a confirm listen: the question just spoken.
    Listen {
        id: u64,
        mode: ListenMode,
        context: Option<ListenContext>,
    },
    CancelListen {
        id: u64,
    },
    ForceEnd {
        id: u64,
    },
    Speak {
        id: u64,
        text: Private,
        lang: Lang,
    },
    StopSpeaking,
    Shutdown,
}

/// The language and text of the confirmation question, so the sidecar can
/// force the language and prompt Whisper with it. The text is private.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListenContext {
    pub lang: Lang,
    pub text: Private,
}

impl fmt::Debug for Outgoing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Outgoing::Listen { id, mode, context } => {
                write!(
                    f,
                    "Listen {{ id: {id}, mode: {mode:?}, context: {context:?} }}"
                )
            }
            Outgoing::CancelListen { id } => write!(f, "CancelListen {{ id: {id} }}"),
            Outgoing::ForceEnd { id } => write!(f, "ForceEnd {{ id: {id} }}"),
            Outgoing::Speak { id, text, lang } => {
                write!(f, "Speak {{ id: {id}, text: {text:?}, lang: {lang:?} }}")
            }
            Outgoing::StopSpeaking => write!(f, "StopSpeaking"),
            Outgoing::Shutdown => write!(f, "Shutdown"),
        }
    }
}

impl Outgoing {
    /// One line of JSON with its newline.
    pub fn to_line(&self) -> String {
        let value = match self {
            Outgoing::Listen { id, mode, context } => {
                let mut listen = json!({
                    "type": "listen",
                    "id": id,
                    "mode": match mode {
                        ListenMode::Command => "command",
                        ListenMode::Confirm => "confirm",
                    },
                });
                if let Some(context) = context {
                    listen["lang"] = json!(context.lang.code());
                    listen["context"] = json!(context.text.as_str());
                }
                listen
            }
            Outgoing::CancelListen { id } => json!({"type": "cancel_listen", "id": id}),
            Outgoing::ForceEnd { id } => json!({"type": "force_end", "id": id}),
            Outgoing::Speak { id, text, lang } => json!({
                "type": "speak",
                "id": id,
                "text": text.as_str(),
                "lang": lang.code(),
            }),
            Outgoing::StopSpeaking => json!({"type": "stop_speaking"}),
            Outgoing::Shutdown => json!({"type": "shutdown"}),
        };
        format!("{value}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<Incoming, ParseError> {
        parse_line(line)
    }

    #[test]
    fn every_sidecar_message_of_the_protocol_parses() {
        assert_eq!(
            parse(r#"{"type":"ready","now_ms":123456}"#),
            Ok(Incoming::Ready { now_ms: 123456 })
        );
        assert_eq!(
            parse(r#"{"type":"wake","score":0.71}"#),
            Ok(Incoming::Wake { score: 0.71 })
        );
        assert_eq!(
            parse(r#"{"type":"speech_start","id":7}"#),
            Ok(Incoming::SpeechStart { id: 7 })
        );
        assert_eq!(
            parse(
                r#"{"type":"transcript","id":7,"text":"abre Spotify","lang":"es","lang_prob":0.98,"end_ms":123999}"#
            ),
            Ok(Incoming::Transcript {
                id: 7,
                text: Private::new("abre Spotify"),
                lang: Some(Lang::Es),
                lang_prob: 0.98,
                end_ms: 123999,
            })
        );
        assert_eq!(
            parse(r#"{"type":"no_speech","id":7}"#),
            Ok(Incoming::NoSpeech { id: 7 })
        );
        assert_eq!(
            parse(r#"{"type":"speak_started","id":8}"#),
            Ok(Incoming::SpeakStarted { id: 8 })
        );
        assert_eq!(
            parse(r#"{"type":"speak_done","id":8}"#),
            Ok(Incoming::SpeakDone { id: 8 })
        );
        assert_eq!(
            parse(r#"{"type":"error","code":"MIC_BUSY","msg":"in use","id":7}"#),
            Ok(Incoming::Error {
                code: "MIC_BUSY".into(),
                msg: "in use".into(),
                id: Some(7),
            })
        );
    }

    #[test]
    fn an_error_without_an_id_is_process_level() {
        let got = parse(r#"{"type":"error","code":"CUDA_OOM","msg":"out of memory"}"#);
        assert_eq!(
            got,
            Ok(Incoming::Error {
                code: "CUDA_OOM".into(),
                msg: "out of memory".into(),
                id: None,
            })
        );
        let null = parse(r#"{"type":"error","code":"INTERNAL","msg":"x","id":null}"#);
        assert!(matches!(null, Ok(Incoming::Error { id: None, .. })));
    }

    #[test]
    fn an_empty_transcript_is_a_transcript_with_empty_text() {
        let got = parse(
            r#"{"type":"transcript","id":3,"text":"","lang":"en","lang_prob":0.4,"end_ms":10}"#,
        );
        assert!(
            matches!(got, Ok(Incoming::Transcript { ref text, .. }) if text.as_str().is_empty())
        );
    }

    #[test]
    fn an_unknown_language_is_none_not_an_error() {
        let got = parse(
            r#"{"type":"transcript","id":3,"text":"x","lang":"fr","lang_prob":0.9,"end_ms":10}"#,
        );
        assert!(matches!(got, Ok(Incoming::Transcript { lang: None, .. })));
    }

    #[test]
    fn unreadable_lines_are_errors_and_never_panics() {
        assert_eq!(parse("not json"), Err(ParseError::NotJson));
        assert_eq!(parse(""), Err(ParseError::NotJson));
        assert_eq!(parse("[1,2]"), Err(ParseError::NotAnObject));
        assert_eq!(parse("42"), Err(ParseError::NotAnObject));
        assert_eq!(parse("{}"), Err(ParseError::UnknownType));
        assert_eq!(parse(r#"{"type":5}"#), Err(ParseError::UnknownType));
        assert_eq!(
            parse(r#"{"type":"teleport","id":1}"#),
            Err(ParseError::UnknownType)
        );
    }

    #[test]
    fn ids_must_be_unsigned_integers() {
        for bad in [
            r#"{"type":"speech_start","id":-1}"#,
            r#"{"type":"speech_start","id":1.5}"#,
            r#"{"type":"speech_start","id":"7"}"#,
            r#"{"type":"speech_start","id":true}"#,
            r#"{"type":"speech_start"}"#,
        ] {
            assert_eq!(parse(bad), Err(ParseError::BadField("id")), "{bad}");
        }
        assert_eq!(
            parse(r#"{"type":"error","code":"X","msg":"","id":"7"}"#),
            Err(ParseError::BadField("id"))
        );
    }

    #[test]
    fn a_ready_without_its_clock_is_rejected() {
        assert_eq!(
            parse(r#"{"type":"ready"}"#),
            Err(ParseError::BadField("now_ms"))
        );
    }

    #[test]
    fn a_parse_error_never_carries_the_line() {
        // A transcript of the wrong shape: the words must not come back in the error.
        let line =
            r#"{"type":"transcript","id":"open the secret bank file","text":1,"lang_prob":1}"#;
        let err = parse(line).unwrap_err();
        assert!(!format!("{err} {err:?}").contains("secret"));
    }

    #[test]
    fn a_line_over_the_limit_is_refused() {
        let long = format!(
            r#"{{"type":"error","code":"X","msg":"{}"}}"#,
            "a".repeat(MAX_LINE_BYTES)
        );
        assert_eq!(parse(&long), Err(ParseError::TooLong));
    }

    #[test]
    fn debug_hides_transcript_and_reply_text() {
        let transcript = Incoming::Transcript {
            id: 1,
            text: Private::new("my password is hunter2"),
            lang: None,
            lang_prob: 1.0,
            end_ms: 0,
        };
        let speak = Outgoing::Speak {
            id: 2,
            text: Private::new("your balance is 4000"),
            lang: Lang::En,
        };
        let shown = format!("{transcript:?} {speak:?}");
        assert!(!shown.contains("hunter2") && !shown.contains("4000"));
        assert!(shown.contains("chars"));
    }

    #[test]
    fn the_debug_of_a_listen_never_shows_the_question() {
        let listen = Outgoing::Listen {
            id: 9,
            mode: ListenMode::Confirm,
            context: Some(ListenContext {
                lang: Lang::En,
                text: Private::new("Should I close the secret spreadsheet?"),
            }),
        };
        let shown = format!("{listen:?}");
        assert!(
            !shown.contains("secret") && shown.contains("chars"),
            "{shown}"
        );
    }

    fn json_of(out: &Outgoing) -> Value {
        let line = out.to_line();
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        serde_json::from_str(line.trim_end()).unwrap()
    }

    #[test]
    fn every_host_command_encodes_as_the_protocol_says() {
        assert_eq!(
            json_of(&Outgoing::Listen {
                id: 7,
                mode: ListenMode::Command,
                context: None
            }),
            json!({"type":"listen","id":7,"mode":"command"})
        );
        assert_eq!(
            json_of(&Outgoing::Listen {
                id: 8,
                mode: ListenMode::Confirm,
                context: None
            }),
            json!({"type":"listen","id":8,"mode":"confirm"})
        );
        assert_eq!(
            json_of(&Outgoing::Listen {
                id: 9,
                mode: ListenMode::Confirm,
                context: Some(ListenContext {
                    lang: Lang::Es,
                    text: Private::new("¿Cierro la calculadora?")
                })
            }),
            json!({"type":"listen","id":9,"mode":"confirm","lang":"es","context":"¿Cierro la calculadora?"})
        );
        assert_eq!(
            json_of(&Outgoing::CancelListen { id: 7 }),
            json!({"type":"cancel_listen","id":7})
        );
        assert_eq!(
            json_of(&Outgoing::ForceEnd { id: 7 }),
            json!({"type":"force_end","id":7})
        );
        assert_eq!(
            json_of(&Outgoing::Speak {
                id: 9,
                text: Private::new("Hola, \"Niva\"\nlinea dos"),
                lang: Lang::Es
            }),
            json!({"type":"speak","id":9,"text":"Hola, \"Niva\"\nlinea dos","lang":"es"})
        );
        assert_eq!(
            json_of(&Outgoing::StopSpeaking),
            json!({"type":"stop_speaking"})
        );
        assert_eq!(json_of(&Outgoing::Shutdown), json!({"type":"shutdown"}));
    }

    #[test]
    fn a_reply_with_newlines_stays_one_line() {
        let line = Outgoing::Speak {
            id: 1,
            text: Private::new("a\nb\r\nc"),
            lang: Lang::En,
        }
        .to_line();
        assert_eq!(line.matches('\n').count(), 1);
    }

    /// `voice/protocol.py` is the contract: its docstring lists every message.
    /// This keeps the host's names from drifting away from it.
    #[test]
    fn the_names_match_the_sidecar_docstring() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../voice/protocol.py");
        let Ok(source) = std::fs::read_to_string(path) else {
            return; // a source tree without voice/ has nothing to compare with
        };
        for name in [
            "listen",
            "cancel_listen",
            "force_end",
            "speak",
            "stop_speaking",
            "shutdown",
            "ready",
            "wake",
            "speech_start",
            "transcript",
            "no_speech",
            "speak_started",
            "speak_done",
            "error",
        ] {
            assert!(
                source.contains(&format!("\"type\": \"{name}\"")),
                "{name} is not in voice/protocol.py"
            );
        }
    }
}
