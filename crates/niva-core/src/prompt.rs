//! The system prompt and the tool schemas the model sees (design D3/D4).
//!
//! Both come from the E1 spike (`spikes/eval/run_e1.py`), whose numbers chose
//! the product model, so the wording and the order of the tools are kept as
//! measured. The schemas are plain data here (the core has no JSON
//! dependency); each LLM adapter turns them into its wire format. A test ties
//! them to the validator in `tools.rs`: the schema offers exactly the
//! arguments the validator accepts.
//!
//! Verify 2a S6: `set_volume` takes exactly one of `level`, `delta`, `mute`,
//! and none of them is required. The spike's `pc-control` schema required
//! both `level` and `mute`, which led a model to send both and hit
//! `Conflicting`.

use crate::ports::LocalDateTime;
use crate::replies::Lang;

#[cfg(test)]
mod tests;

/// The type of one tool argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    Integer,
    Boolean,
    Text,
    /// A string that must be one of these values.
    OneOf(&'static [&'static str]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    pub name: &'static str,
    pub kind: ParamKind,
    /// Empty when the name says it all (the power `action` enum).
    pub description: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub params: &'static [ParamSpec],
    pub required: &'static [&'static str],
}

const fn text(name: &'static str, description: &'static str) -> ParamSpec {
    ParamSpec {
        name,
        kind: ParamKind::Text,
        description,
    }
}

const fn integer(name: &'static str, description: &'static str) -> ParamSpec {
    ParamSpec {
        name,
        kind: ParamKind::Integer,
        description,
    }
}

/// The v1 tool set, in the order E1 sent it (the order is part of the prompt).
pub const TOOL_SPECS: [ToolSpec; 7] = [
    ToolSpec {
        name: "open_app",
        description: "Open an installed application by its name, e.g. 'Spotify', 'Calculator', 'Google Chrome'.",
        params: &[text("name", "Application name as the user said it.")],
        required: &["name"],
    },
    ToolSpec {
        name: "close_app",
        description: "Close a running application by its name. Asks the user for confirmation. Use the short name the user said (e.g. 'chrome', 'spotify'), without extra words such as Google or Microsoft.",
        params: &[text("name", "Application name as the user said it.")],
        required: &["name"],
    },
    ToolSpec {
        name: "set_volume",
        description: "Change the master volume. Pass exactly ONE of: level (set to a percentage), delta (raise or lower by a number of points) or mute (mute or unmute).",
        params: &[
            integer("level", "Absolute volume 0-100."),
            integer(
                "delta",
                "Relative change -100..100 (positive raises, negative lowers).",
            ),
            ParamSpec {
                name: "mute",
                kind: ParamKind::Boolean,
                description: "true to mute, false to unmute.",
            },
        ],
        required: &[],
    },
    ToolSpec {
        name: "web_search",
        description: "Search the web for information and get a short summary of the results.",
        params: &[text("query", "Search query.")],
        required: &["query"],
    },
    ToolSpec {
        name: "power_action",
        description: "Lock, sleep, restart or shut down the PC. Asks the user for confirmation. Only for an explicit command about the PC (lock it, put it to sleep, restart it, shut it down); never because the user says they are tired, done for the day, leaving or going to bed.",
        params: &[ParamSpec {
            name: "action",
            kind: ParamKind::OneOf(&["lock", "sleep", "restart", "shutdown"]),
            description: "",
        }],
        required: &["action"],
    },
    ToolSpec {
        name: "create_reminder",
        description: "Create a reminder. Give a delay_minutes (relative) OR an at_local_time (absolute). Do not compute dates yourself.",
        params: &[
            text("text", "What to remind the user of."),
            integer(
                "delay_minutes",
                "Minutes from now, for relative requests ('in 20 minutes').",
            ),
            text(
                "at_local_time",
                "Local time of day, 24-hour HH:MM, for absolute requests ('at 9 pm' -> '21:00').",
            ),
            text(
                "date",
                "YYYY-MM-DD, only if the user said an explicit calendar date.",
            ),
        ],
        required: &["text"],
    },
    ToolSpec {
        name: "list_reminders",
        description: "List the user's pending reminders.",
        params: &[],
        required: &[],
    },
];

/// Day of the week of a Gregorian date (Sakamoto's method), English name.
pub fn weekday_name(year: u16, month: u8, day: u8) -> &'static str {
    const OFFSETS: [i32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    const NAMES: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    let month = usize::from(month.clamp(1, 12));
    let year = i32::from(year) - i32::from(month < 3);
    let index = (year + year / 4 - year / 100 + year / 400 + OFFSETS[month - 1] + i32::from(day))
        .rem_euclid(7);
    NAMES[index as usize]
}

/// The prompt of a tool-calling request, as E1 measured it plus the reply
/// bound (task 2.4a). `now` is the user's local time; without it (an adapter
/// that does not know the time zone) the line is left out.
pub fn system_prompt(lang: Lang, now: Option<&LocalDateTime>) -> String {
    let language = match lang {
        Lang::Es => "Spanish",
        Lang::En => "English",
    };
    let time = now
        .map(|t| {
            format!(
                "Current local time: {} {:04}-{:02}-{:02} {:02}:{:02}.\n",
                weekday_name(t.year, t.month, t.day),
                t.year,
                t.month,
                t.day,
                t.hour,
                t.minute
            )
        })
        .unwrap_or_default();
    format!(
        "You are Niva, a voice assistant that controls the user's Windows PC.\n\
         {time}\
         Use the tools to carry out what the user asks. Your reply is spoken aloud: \
         reply in {language}, in one short sentence, without markdown, never over 300 characters.\n\
         Only do what the user asked. Do the requested actions in one go, then stop.\n\
         Never lock, sleep, restart or shut down the PC unless the user explicitly asked for it. \
         Never close an app unless the user explicitly asked for it. If the user only talks about \
         these things, negates them, or asks how to do them, do not call a tool. \
         Saying goodnight, being tired or going to bed is only conversation, not a request to sleep or lock the PC.\n\
         If a reminder has no time, do not create it: ask when instead.\n\
         Closing apps and power actions ask the user for confirmation inside the app; \
         you do not need to ask first.\n\
         For chit-chat, answer briefly without tools."
    )
}
