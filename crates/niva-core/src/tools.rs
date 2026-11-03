//! Tool registry and argument validation.
//!
//! Every tool the model may call is a [`ToolName`] with a fixed [`Risk`].
//! [`parse_call`] checks one call against its schema before anything runs
//! ("validation before execution"): an unknown tool or bad arguments yield a
//! typed [`ToolError`] and nothing executes. Target resolution for the risky
//! `close_app` is the safety gate's job (`safety.rs`).

use std::collections::BTreeMap;

#[cfg(test)]
mod tests;

/// A tool argument as the host decoded it from the model's JSON. There is no
/// array or object variant: no tool schema uses one, so those are rejected
/// by the decoder.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

pub type Args = BTreeMap<String, ArgValue>;

/// One tool call from the model.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub name: String,
    pub args: Args,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Risk {
    Safe,
    Risky,
}

/// The static tool table (design "Safety Gate"). Adding a tool breaks the
/// exhaustive matches below until it is classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolName {
    OpenApp,
    CloseApp,
    SetVolume,
    CreateReminder,
    ListReminders,
    WebSearch,
    PowerAction,
}

impl ToolName {
    pub const ALL: [ToolName; 7] = [
        ToolName::OpenApp,
        ToolName::CloseApp,
        ToolName::SetVolume,
        ToolName::CreateReminder,
        ToolName::ListReminders,
        ToolName::WebSearch,
        ToolName::PowerAction,
    ];

    /// Name on the wire (the schema name the model sees).
    pub const fn as_str(self) -> &'static str {
        match self {
            ToolName::OpenApp => "open_app",
            ToolName::CloseApp => "close_app",
            ToolName::SetVolume => "set_volume",
            ToolName::CreateReminder => "create_reminder",
            ToolName::ListReminders => "list_reminders",
            ToolName::WebSearch => "web_search",
            ToolName::PowerAction => "power_action",
        }
    }

    /// Exact, case-sensitive lookup: anything else is an unknown tool.
    pub fn from_name(name: &str) -> Option<ToolName> {
        ToolName::ALL.into_iter().find(|tool| tool.as_str() == name)
    }

    pub const fn risk(self) -> Risk {
        match self {
            ToolName::CloseApp | ToolName::PowerAction => Risk::Risky,
            ToolName::OpenApp
            | ToolName::SetVolume
            | ToolName::CreateReminder
            | ToolName::ListReminders
            | ToolName::WebSearch => Risk::Safe,
        }
    }

    /// Argument names the schema allows (`additionalProperties: false`).
    const fn allowed_args(self) -> &'static [&'static str] {
        match self {
            ToolName::OpenApp | ToolName::CloseApp => &["name"],
            ToolName::SetVolume => &["level", "delta", "mute"],
            ToolName::CreateReminder => &["text", "delay_minutes", "at_local_time", "date"],
            ToolName::ListReminders => &[],
            ToolName::WebSearch => &["query"],
            ToolName::PowerAction => &["action"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerKind {
    Lock,
    Sleep,
    Restart,
    Shutdown,
}

impl PowerKind {
    pub const ALL: [PowerKind; 4] = [
        PowerKind::Lock,
        PowerKind::Sleep,
        PowerKind::Restart,
        PowerKind::Shutdown,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            PowerKind::Lock => "lock",
            PowerKind::Sleep => "sleep",
            PowerKind::Restart => "restart",
            PowerKind::Shutdown => "shutdown",
        }
    }

    /// Restart and shutdown are confirmable by click only (design D4/D10:
    /// ambient speech must not be able to confirm them).
    pub const fn click_only(self) -> bool {
        match self {
            PowerKind::Restart | PowerKind::Shutdown => true,
            PowerKind::Lock | PowerKind::Sleep => false,
        }
    }
}

/// A volume change. `Set` carries an already clamped level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeChange {
    Set(u8),
    Delta(i8),
    Mute(bool),
}

/// When a reminder fires, as the model stated it. Resolution against the
/// clock (past time, time zones) belongs to `reminders.rs` (slice 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReminderTime {
    InMinutes(u32),
    AtLocal {
        hour: u8,
        minute: u8,
        date: Option<CalendarDate>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CalendarDate {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

/// A validated Safe call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SafeAction {
    OpenApp {
        name: String,
    },
    SetVolume {
        change: VolumeChange,
        /// The requested level was out of range and was clamped.
        clamped: bool,
    },
    CreateReminder {
        text: String,
        when: ReminderTime,
    },
    ListReminders,
    WebSearch {
        query: String,
    },
}

/// A call that passed schema validation. A risky call is not resolved yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedCall {
    Safe(SafeAction),
    Close { query: String },
    Power(PowerKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgProblem {
    WrongType,
    Empty,
    NotPositive,
    OutOfRange,
    BadFormat,
    NotOneOf,
    TooLong,
}

/// Why a call cannot run. Nothing executes on any of these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolError {
    UnknownTool(String),
    UnknownArgument(String),
    /// Required argument(s) absent. For `set_volume` the list is the
    /// alternatives (exactly one is needed); for `create_reminder` the names
    /// are `text` and/or `time`.
    Missing(Vec<&'static str>),
    InvalidArgument {
        name: &'static str,
        problem: ArgProblem,
    },
    /// Mutually exclusive arguments given together.
    Conflicting(Vec<&'static str>),
    /// `close_app` query shorter than the minimum.
    QueryTooShort {
        min: usize,
    },
    /// `close_app`: no unprotected running executable matches.
    NoMatch,
    /// `close_app`: several distinct executables match; ask which one.
    Ambiguous(Vec<String>),
}

/// Longest free-text argument (app name, search query, reminder text), in
/// characters. A longer one is rejected, not truncated.
pub const MAX_TEXT_CHARS: usize = 200;

/// Longest reminder delay: 7 days.
pub const MAX_DELAY_MINUTES: i64 = 7 * 24 * 60;

/// Validates one call against its schema. [`ToolName::risk`] decides the
/// class: a Safe tool yields a ready [`SafeAction`], a Risky one a pending
/// call that still needs the gate. A tool the class does not cover fails
/// closed with `UnknownTool` instead of running.
pub fn parse_call(call: &ToolCall) -> Result<ParsedCall, ToolError> {
    let tool =
        ToolName::from_name(&call.name).ok_or_else(|| ToolError::UnknownTool(call.name.clone()))?;
    let args = &call.args;
    if let Some(extra) = args
        .keys()
        .find(|key| !tool.allowed_args().contains(&key.as_str()))
    {
        return Err(ToolError::UnknownArgument(extra.clone()));
    }
    match tool.risk() {
        Risk::Safe => parse_safe(tool, args).map(ParsedCall::Safe),
        Risk::Risky => parse_risky(tool, args),
    }
}

fn parse_safe(tool: ToolName, args: &Args) -> Result<SafeAction, ToolError> {
    match tool {
        ToolName::OpenApp => Ok(SafeAction::OpenApp {
            name: required_str(args, "name")?,
        }),
        ToolName::WebSearch => Ok(SafeAction::WebSearch {
            query: required_str(args, "query")?,
        }),
        ToolName::ListReminders => Ok(SafeAction::ListReminders),
        ToolName::SetVolume => parse_volume(args),
        ToolName::CreateReminder => parse_reminder(args),
        ToolName::CloseApp | ToolName::PowerAction => {
            Err(ToolError::UnknownTool(tool.as_str().into()))
        }
    }
}

fn parse_risky(tool: ToolName, args: &Args) -> Result<ParsedCall, ToolError> {
    match tool {
        ToolName::CloseApp => Ok(ParsedCall::Close {
            query: required_str(args, "name")?,
        }),
        ToolName::PowerAction => parse_power(args),
        ToolName::OpenApp
        | ToolName::SetVolume
        | ToolName::CreateReminder
        | ToolName::ListReminders
        | ToolName::WebSearch => Err(ToolError::UnknownTool(tool.as_str().into())),
    }
}

/// `Null` counts as absent: the spike's schema had nullable required fields,
/// so models send `null` for the arguments they do not use.
fn present<'a>(args: &'a Args, key: &str) -> Option<&'a ArgValue> {
    args.get(key).filter(|value| **value != ArgValue::Null)
}

fn wrong_type(name: &'static str) -> ToolError {
    ToolError::InvalidArgument {
        name,
        problem: ArgProblem::WrongType,
    }
}

fn invalid(name: &'static str, problem: ArgProblem) -> ToolError {
    ToolError::InvalidArgument { name, problem }
}

fn optional_str(args: &Args, name: &'static str) -> Result<Option<String>, ToolError> {
    match present(args, name) {
        None => Ok(None),
        Some(ArgValue::Str(text)) => {
            let text = text.trim();
            if text.is_empty() {
                Err(invalid(name, ArgProblem::Empty))
            } else if text.chars().count() > MAX_TEXT_CHARS {
                Err(invalid(name, ArgProblem::TooLong))
            } else {
                Ok(Some(text.to_string()))
            }
        }
        Some(_) => Err(wrong_type(name)),
    }
}

fn required_str(args: &Args, name: &'static str) -> Result<String, ToolError> {
    optional_str(args, name)?.ok_or(ToolError::Missing(vec![name]))
}

fn optional_int(args: &Args, name: &'static str) -> Result<Option<i64>, ToolError> {
    match present(args, name) {
        None => Ok(None),
        Some(ArgValue::Int(value)) => Ok(Some(*value)),
        Some(_) => Err(wrong_type(name)),
    }
}

fn optional_bool(args: &Args, name: &'static str) -> Result<Option<bool>, ToolError> {
    match present(args, name) {
        None => Ok(None),
        Some(ArgValue::Bool(value)) => Ok(Some(*value)),
        Some(_) => Err(wrong_type(name)),
    }
}

fn parse_power(args: &Args) -> Result<ParsedCall, ToolError> {
    let action = required_str(args, "action")?;
    PowerKind::ALL
        .into_iter()
        .find(|kind| kind.as_str() == action)
        .map(ParsedCall::Power)
        .ok_or(invalid("action", ArgProblem::NotOneOf))
}

fn parse_volume(args: &Args) -> Result<SafeAction, ToolError> {
    let level = optional_int(args, "level")?;
    let delta = optional_int(args, "delta")?;
    let mute = optional_bool(args, "mute")?;
    let given: Vec<&'static str> = [
        ("level", level.is_some()),
        ("delta", delta.is_some()),
        ("mute", mute.is_some()),
    ]
    .into_iter()
    .filter_map(|(name, is_given)| is_given.then_some(name))
    .collect();
    if given.len() > 1 {
        return Err(ToolError::Conflicting(given));
    }
    let (change, clamped) = match (level, delta, mute) {
        // Spec `pc-actions`: an out-of-range level is clamped and reported.
        (Some(level), _, _) => (
            VolumeChange::Set(level.clamp(0, 100) as u8),
            !(0..=100).contains(&level),
        ),
        (_, Some(delta), _) => (
            VolumeChange::Delta(delta.clamp(-100, 100) as i8),
            !(-100..=100).contains(&delta),
        ),
        (_, _, Some(mute)) => (VolumeChange::Mute(mute), false),
        _ => return Err(ToolError::Missing(vec!["level", "delta", "mute"])),
    };
    Ok(SafeAction::SetVolume { change, clamped })
}

fn parse_reminder(args: &Args) -> Result<SafeAction, ToolError> {
    let text = optional_str(args, "text")?;
    let delay = optional_int(args, "delay_minutes")?;
    let at = optional_str(args, "at_local_time")?;
    let date = optional_str(args, "date")?;

    if delay.is_some() && at.is_some() {
        return Err(ToolError::Conflicting(vec![
            "delay_minutes",
            "at_local_time",
        ]));
    }
    if delay.is_some() && date.is_some() {
        return Err(ToolError::Conflicting(vec!["delay_minutes", "date"]));
    }

    let when = match (delay, &at) {
        (Some(minutes), _) => {
            if minutes <= 0 {
                return Err(invalid("delay_minutes", ArgProblem::NotPositive));
            }
            if minutes > MAX_DELAY_MINUTES {
                return Err(invalid("delay_minutes", ArgProblem::OutOfRange));
            }
            Some(ReminderTime::InMinutes(minutes as u32))
        }
        (None, Some(at)) => {
            let (hour, minute) =
                parse_clock_time(at).ok_or(invalid("at_local_time", ArgProblem::BadFormat))?;
            let date = date
                .as_deref()
                .map(|date| parse_date(date).ok_or(invalid("date", ArgProblem::BadFormat)))
                .transpose()?;
            Some(ReminderTime::AtLocal { hour, minute, date })
        }
        (None, None) => None,
    };

    match (text, when) {
        (Some(text), Some(when)) => Ok(SafeAction::CreateReminder { text, when }),
        (Some(_), None) => Err(ToolError::Missing(vec!["time"])),
        (None, Some(_)) => Err(ToolError::Missing(vec!["text"])),
        (None, None) => Err(ToolError::Missing(vec!["text", "time"])),
    }
}

/// 24-hour clock time as `H`, `HH`, `H:MM`, `HH:MM` or `HH:MM:SS` (the forms
/// the eval accepts). Minutes and seconds take exactly two digits. Seconds
/// are range-checked and then dropped: reminders have minute resolution.
fn parse_clock_time(text: &str) -> Option<(u8, u8)> {
    let mut parts = text.split(':');
    let hour = digits(parts.next()?, 1, 2)?;
    let minute = match parts.next() {
        Some(minute) => digits(minute, 2, 2)?,
        None => 0,
    };
    if let Some(seconds) = parts.next()
        && digits(seconds, 2, 2)? > 59
    {
        return None;
    }
    if parts.next().is_some() {
        return None;
    }
    (hour < 24 && minute < 60).then_some((hour, minute))
}

fn two_digits(text: &str) -> Option<u8> {
    digits(text, 2, 2)
}

/// An all-ASCII-digit field of `min..=max` characters.
fn digits(text: &str, min: usize, max: usize) -> Option<u8> {
    if (min..=max).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
}

/// `YYYY-MM-DD` with a real calendar day.
fn parse_date(text: &str) -> Option<CalendarDate> {
    let mut parts = text.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() || year.len() != 4 || !year.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let year: u16 = year.parse().ok()?;
    let month = two_digits(month)?;
    let day = two_digits(day)?;
    let leap = year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    (1..=days_in_month)
        .contains(&day)
        .then_some(CalendarDate { year, month, day })
}
