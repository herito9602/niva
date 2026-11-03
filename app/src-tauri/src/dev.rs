//! Debug-build stand-ins for the adapters that arrive in PRs 2c and 2d, so the
//! whole assistant loop can be tried by hand from the debug window.
//!
//! `lib.rs` declares this module under `cfg(debug_assertions)`; release
//! builds contain none of it (`npm run check:release` looks for
//! [`DEV_LLM_MARKER`] and [`DRY_RUN_MARKER`] in the exe).
//!
//! - [`DevLlm`] maps a few simple Spanish and English commands to tool calls
//!   with plain rules. It is not a model: it exists to exercise every path of
//!   the loop (and of the safety gate) deterministically.
//! - [`DryRunPc`], [`DryRunSearch`] and [`DryRunReminders`] never touch the
//!   system: they log what they would do and return realistic results.
//!   `DryRunPc` keeps a fake process list (with `explorer.exe`, which is
//!   protected) that follows the open and close calls.
//!
//! SECURITY (design "Safety Gate", Channel::Click): the debug window can click
//! Confirm, and that click is a `Channel::Click` answer. That is acceptable
//! ONLY because every `PcActions` a debug build can run is [`DryRunPc`]
//! ([`assistant`] is the only constructor). `lib.rs` pins its signature with a
//! const, which is only a tripwire; the test
//! `the_assistant_of_a_debug_build_only_ever_acts_on_the_dry_run_pc` below is
//! what enforces it by checking that a confirmed shutdown and close only
//! produce dry-run effects.
//! PR 2d must keep power actions and `close_app` dry-run in debug builds, or
//! remove the debug Accept path, before it wires real adapters.

use std::sync::{Arc, Mutex};

use niva_core::assistant::{Assistant, PENDING_REMINDER_ALL_MARKER, PENDING_REMINDER_MARKER};
use niva_core::ports::{
    ChatReply, ChatRequest, CloseOutcome, LlmError, LlmProvider, PcActions, PcError, ProcessInfo,
    ReminderError, ReminderStore, SearchError, SearchProvider, VolumeResult,
};
use niva_core::safety::{Approved, CloseApp, Power};
use niva_core::tools::{ArgValue, ReminderTime, ToolCall, VolumeChange};

/// Appears in every dev LLM log line (and so in the debug exe only).
pub const DEV_LLM_MARKER: &str = "niva-dev-llm";
/// Appears in every dry-run log line (and so in the debug exe only).
pub const DRY_RUN_MARKER: &str = "niva-dry-run";

/// Where the dev adapters report what they did (stderr and the debug window).
pub type LogSink = Arc<dyn Fn(String) + Send + Sync>;

/// The assistant wired to the dev adapters.
pub fn assistant(log: LogSink) -> Assistant {
    Assistant::new(
        Arc::new(DevLlm { log: log.clone() }),
        Arc::new(DryRunPc::new(log.clone())),
    )
    .with_search(Arc::new(DryRunSearch { log: log.clone() }))
    .with_reminders(Arc::new(DryRunReminders {
        log,
        items: Mutex::default(),
    }))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// ---------------------------------------------------------------------------
// Dev LLM
// ---------------------------------------------------------------------------

pub struct DevLlm {
    log: LogSink,
}

impl LlmProvider for DevLlm {
    fn chat(&self, request: &ChatRequest) -> Result<ChatReply, LlmError> {
        let spanish = request.system.contains("Spanish");
        let reply = if request.allow_tools {
            decide(&request.system, &request.user, spanish)
        } else {
            ChatReply {
                text: summary(&request.user, spanish),
                calls: Vec::new(),
            }
        };
        let what = if reply.calls.is_empty() {
            "plain text".to_string()
        } else {
            reply
                .calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>()
                .join(" + ")
        };
        (self.log)(format!("[{DEV_LLM_MARKER}] {:?} -> {what}", request.user));
        Ok(reply)
    }
}

fn summary(user: &str, spanish: bool) -> String {
    let question = user
        .lines()
        .find_map(|line| line.strip_prefix("Question: "))
        .unwrap_or("that");
    if spanish {
        format!("Resumen de prueba sobre \"{question}\", sacado de la búsqueda simulada.")
    } else {
        format!("Dev summary about \"{question}\", taken from the dry-run search.")
    }
}

fn arg(key: &str, value: ArgValue) -> (String, ArgValue) {
    (key.to_string(), value)
}

fn call(name: &str, args: Vec<(String, ArgValue)>) -> ToolCall {
    ToolCall {
        name: name.into(),
        args: args.into_iter().collect(),
    }
}

fn text(value: &str) -> ArgValue {
    ArgValue::Str(value.into())
}

/// What the dev LLM answers to `user`. `system` carries the pending context
/// of a follow-up turn (the reminder text asked about).
pub fn decide(system: &str, user: &str, spanish: bool) -> ChatReply {
    let utterance = user.trim();
    let calls = if let Some(pending) = pending_reminder_text(system) {
        vec![reminder_call(Some(pending), utterance)]
    } else if system.contains(PENDING_REMINDER_ALL_MARKER) {
        vec![reminder_call(None, utterance)]
    } else if asks_for_reminder_list(utterance) {
        vec![call("list_reminders", Vec::new())]
    } else if starts_with_reminder_verb(utterance) {
        vec![reminder_call(None, utterance)]
    } else {
        let clauses = split_clauses(utterance);
        let parsed: Vec<ToolCall> = clauses.iter().filter_map(|c| clause_call(c)).collect();
        if !clauses.is_empty() && parsed.len() == clauses.len() {
            parsed
        } else {
            clause_call(utterance).into_iter().collect()
        }
    };
    let text = if calls.is_empty() {
        chit_chat(spanish)
    } else {
        String::new()
    };
    ChatReply { text, calls }
}

fn chit_chat(spanish: bool) -> String {
    if spanish {
        "Hola, soy el asistente de prueba. Prueba: abre notepad, cierra spotify, volumen 30, \
         recuérdame llamar a mamá en 10 minutos, busca rust, bloquea el pc."
            .into()
    } else {
        "Hi, I'm the dev assistant. Try: open notepad, close spotify, volume 30, remind me to \
         call mom in 10 minutes, search rust, lock the pc."
            .into()
    }
}

/// The reminder text between the quotes after [`PENDING_REMINDER_MARKER`].
fn pending_reminder_text(system: &str) -> Option<String> {
    let rest = &system[system.find(PENDING_REMINDER_MARKER)? + PENDING_REMINDER_MARKER.len()..];
    let start = rest.find('"')? + 1;
    let end = start + rest[start..].find('"')?;
    Some(rest[start..end].to_string())
}

fn split_clauses(utterance: &str) -> Vec<String> {
    let mut clauses = vec![utterance.to_string()];
    for separator in [" and ", " y ", " then ", " luego ", " después "] {
        clauses = clauses
            .iter()
            .flat_map(|clause| {
                clause
                    .split(separator)
                    .map(str::trim)
                    .filter(|part| !part.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect();
    }
    clauses
}

fn words(clause: &str) -> Vec<&str> {
    clause.split_whitespace().collect()
}

fn starts_with_reminder_verb(utterance: &str) -> bool {
    let first = words(utterance)
        .first()
        .map(|w| w.to_lowercase())
        .unwrap_or_default();
    reminder_verb(&first)
}

fn reminder_verb(word: &str) -> bool {
    ["remind", "recuerd", "recuérd", "record", "avisa", "avísa"]
        .iter()
        .any(|verb| word.starts_with(verb))
}

/// "list my reminders", "lista mis recordatorios" (the plural noun, and no
/// "to ..." / "que ..." that would make it a new reminder).
fn asks_for_reminder_list(utterance: &str) -> bool {
    let lower = utterance.to_lowercase();
    (lower.contains("reminders") || lower.contains("recordatorios"))
        && !lower.contains(" to ")
        && !lower.contains(" que ")
}

/// A `create_reminder` call with whatever the sentence gives. `known_text` is
/// the text from the pending slot (a follow-up answer carries only the time).
fn reminder_call(known_text: Option<String>, utterance: &str) -> ToolCall {
    let tokens = words(utterance);
    let (time_start, time_len, time_arg) = find_time(&tokens);
    let mut args = Vec::new();
    if let Some(known) = known_text {
        args.push(arg("text", text(&known)));
    } else {
        // Drop the verb, then "me" / "to" / "que" / "de", and the time words.
        let mut rest: Vec<&str> = tokens
            .iter()
            .enumerate()
            .filter(|(i, _)| time_arg.is_none() || *i < time_start || *i >= time_start + time_len)
            .map(|(_, w)| *w)
            .collect();
        if rest
            .first()
            .is_some_and(|w| reminder_verb(&w.to_lowercase()))
        {
            rest.remove(0);
        }
        while rest
            .first()
            .is_some_and(|w| matches!(w.to_lowercase().as_str(), "me" | "to" | "que" | "de" | "te"))
        {
            rest.remove(0);
        }
        let body = rest.join(" ");
        if !body.is_empty() {
            args.push(arg("text", text(&body)));
        }
    }
    if let Some(time) = time_arg {
        args.push(time);
    }
    call("create_reminder", args)
}

/// Finds "in N minutes", "en N minutos", "at H[:MM]", "a las H[:MM]" or a
/// bare number (an answer to "at what time?"). Returns the token span and the
/// argument.
fn find_time(tokens: &[&str]) -> (usize, usize, Option<(String, ArgValue)>) {
    let lower: Vec<String> = tokens.iter().map(|t| t.to_lowercase()).collect();
    for (i, token) in lower.iter().enumerate() {
        let number = lower.get(i + 1).and_then(|n| n.parse::<i64>().ok());
        let unit = lower.get(i + 2).map(String::as_str).unwrap_or("");
        if matches!(token.as_str(), "in" | "en")
            && let Some(n) = number
        {
            let minutes = if unit.starts_with("hour") || unit.starts_with("hora") {
                n * 60
            } else {
                n
            };
            return (i, 3, Some(arg("delay_minutes", ArgValue::Int(minutes))));
        }
        if token == "at" && is_clock(lower.get(i + 1)) {
            return (i, 2, Some(arg("at_local_time", text(tokens[i + 1]))));
        }
        if token == "las" && i > 0 && lower[i - 1] == "a" && is_clock(lower.get(i + 1)) {
            return (i - 1, 3, Some(arg("at_local_time", text(tokens[i + 1]))));
        }
    }
    if let [only] = tokens
        && let Ok(n) = only.parse::<i64>()
    {
        return (0, 1, Some(arg("delay_minutes", ArgValue::Int(n))));
    }
    (0, 0, None)
}

fn is_clock(token: Option<&String>) -> bool {
    token.is_some_and(|t| {
        !t.is_empty()
            && t.chars().all(|c| c.is_ascii_digit() || c == ':')
            && t.chars().next().is_some_and(|c| c.is_ascii_digit())
    })
}

fn first_number(clause: &str) -> Option<i64> {
    clause
        .split(|c: char| !c.is_ascii_digit())
        .find_map(|part| part.parse().ok())
}

/// The words after the first one, minus articles and polite endings.
fn target_after_verb(clause: &str) -> String {
    let mut rest: Vec<&str> = words(clause).into_iter().skip(1).collect();
    while rest.first().is_some_and(|w| {
        matches!(
            w.to_lowercase().as_str(),
            "the" | "el" | "la" | "my" | "mi" | "app" | "aplicación" | "aplicacion"
        )
    }) {
        rest.remove(0);
    }
    let polite_one = ["please", "app"];
    while rest
        .last()
        .is_some_and(|w| polite_one.contains(&w.to_lowercase().as_str()))
    {
        rest.pop();
    }
    if rest.len() >= 2 && rest[rest.len() - 2..].join(" ").to_lowercase() == "por favor" {
        rest.truncate(rest.len() - 2);
    }
    rest.join(" ")
}

/// One command, or `None` if the clause is not one the dev LLM knows.
fn clause_call(clause: &str) -> Option<ToolCall> {
    let lower = clause.to_lowercase();
    let first = lower.split_whitespace().next().unwrap_or("");
    let has = |keys: &[&str]| keys.iter().any(|k| lower.contains(k));
    match first {
        "open" | "abre" | "abrí" | "abri" | "abrir" => {
            return Some(call(
                "open_app",
                vec![arg("name", text(&target_after_verb(clause)))],
            ));
        }
        "close" | "cierra" | "cerrá" | "cerra" | "cerrar" => {
            return Some(call(
                "close_app",
                vec![arg("name", text(&target_after_verb(clause)))],
            ));
        }
        "search" | "find" | "google" | "busca" | "buscá" | "buscar" => {
            return Some(call(
                "web_search",
                vec![arg("query", text(&target_after_verb(clause)))],
            ));
        }
        _ => {}
    }
    if has(&[
        "volume", "volumen", "mute", "silenci", "sonido", "louder", "quieter",
    ]) || matches!(first, "sube" | "baja" | "subí" | "bajá")
    {
        let change = if has(&["unmute", "desmut", "activa el sonido"]) {
            ("mute", ArgValue::Bool(false))
        } else if has(&["mute", "silenci"]) {
            ("mute", ArgValue::Bool(true))
        } else if let Some(level) = first_number(clause) {
            ("level", ArgValue::Int(level))
        } else if has(&["up", "louder", "raise", "sube", "subir", "subí", "aumenta"]) {
            ("delta", ArgValue::Int(10))
        } else if has(&[
            "down", "quieter", "lower", "baja", "bajar", "bajá", "reduce",
        ]) {
            ("delta", ArgValue::Int(-10))
        } else {
            return None;
        };
        return Some(call("set_volume", vec![arg(change.0, change.1)]));
    }
    let power = if has(&["lock", "bloque"]) {
        "lock"
    } else if has(&["sleep", "suspend", "hibern"]) {
        "sleep"
    } else if has(&["restart", "reboot", "reinicia"]) {
        "restart"
    } else if has(&["shut down", "shutdown", "apaga"]) {
        "shutdown"
    } else {
        return None;
    };
    Some(call("power_action", vec![arg("action", text(power))]))
}

// ---------------------------------------------------------------------------
// Dry-run adapters
// ---------------------------------------------------------------------------

/// Apps `open_app` knows: (search word, executable it starts).
const KNOWN_APPS: [(&str, &str); 5] = [
    ("notepad", "notepad.exe"),
    ("spotify", "Spotify.exe"),
    ("chrome", "chrome.exe"),
    ("calculator", "CalculatorApp.exe"),
    ("paint", "mspaint.exe"),
];

struct PcState {
    processes: Vec<ProcessInfo>,
    level: u8,
    muted: bool,
}

/// A PC that only exists in memory. Its realistic process list has two
/// Notepads (so "close notepad" is ambiguous), Spotify, two Chrome processes
/// and `explorer.exe` (protected).
pub struct DryRunPc {
    log: LogSink,
    state: Mutex<PcState>,
}

impl DryRunPc {
    pub fn new(log: LogSink) -> Self {
        let processes = [
            "explorer.exe",
            "notepad.exe",
            "notepad++.exe",
            "Spotify.exe",
            "chrome.exe",
            "chrome.exe",
        ]
        .iter()
        .map(|name| ProcessInfo {
            exe_name: (*name).into(),
            in_user_session: true,
        })
        .collect();
        DryRunPc {
            log,
            state: Mutex::new(PcState {
                processes,
                level: 50,
                muted: false,
            }),
        }
    }

    fn say(&self, line: String) {
        (self.log)(format!("[{DRY_RUN_MARKER}] {line}"));
    }
}

impl PcActions for DryRunPc {
    fn open_app(&self, name: &str) -> Result<(), PcError> {
        let wanted = name.to_lowercase();
        let Some((_, exe)) = KNOWN_APPS.iter().find(|(key, _)| wanted.contains(key)) else {
            self.say(format!("open_app {name:?}: no such app (NotFound)"));
            return Err(PcError::NotFound);
        };
        let mut state = lock(&self.state);
        if !state
            .processes
            .iter()
            .any(|p| p.exe_name.eq_ignore_ascii_case(exe))
        {
            state.processes.push(ProcessInfo {
                exe_name: (*exe).into(),
                in_user_session: true,
            });
        }
        self.say(format!("would start {exe}"));
        Ok(())
    }

    fn running_processes(&self) -> Vec<ProcessInfo> {
        lock(&self.state).processes.clone()
    }

    fn close_app(&self, approved: Approved<CloseApp>) -> Result<CloseOutcome, PcError> {
        let exe = approved.action().exe().to_string();
        // Notepad++ pretends to ask to save changes: it stays open.
        if exe.eq_ignore_ascii_case("notepad++.exe") {
            self.say(format!(
                "would post WM_CLOSE to {exe}: still running (save dialog)"
            ));
            return Ok(CloseOutcome::StillRunning);
        }
        lock(&self.state)
            .processes
            .retain(|p| !p.exe_name.eq_ignore_ascii_case(&exe));
        self.say(format!("would post WM_CLOSE to every {exe} window: closed"));
        Ok(CloseOutcome::Closed)
    }

    fn set_volume(&self, change: VolumeChange) -> Result<VolumeResult, PcError> {
        let mut state = lock(&self.state);
        let mut clamped = false;
        match change {
            VolumeChange::Set(level) => state.level = level,
            VolumeChange::Delta(delta) => {
                let target = i16::from(state.level) + i16::from(delta);
                clamped = !(0..=100).contains(&target);
                state.level = target.clamp(0, 100) as u8;
            }
            VolumeChange::Mute(muted) => state.muted = muted,
        }
        self.say(format!(
            "would set volume {change:?}: now {}%{}",
            state.level,
            if state.muted { ", muted" } else { "" }
        ));
        Ok(VolumeResult {
            level: state.level,
            muted: state.muted,
            clamped,
        })
    }

    fn power(&self, approved: Approved<Power>) -> Result<(), PcError> {
        self.say(format!(
            "would {} the PC (nothing happened)",
            approved.action().kind().as_str()
        ));
        Ok(())
    }
}

pub struct DryRunSearch {
    log: LogSink,
}

impl SearchProvider for DryRunSearch {
    fn search(&self, query: &str) -> Result<Vec<String>, SearchError> {
        (self.log)(format!(
            "[{DRY_RUN_MARKER}] would search the web for {query:?}"
        ));
        let lower = query.to_lowercase();
        if lower.contains("offline") {
            return Err(SearchError::Unavailable);
        }
        if lower.contains("nothing") || lower.contains("nada") {
            return Ok(Vec::new());
        }
        Ok(vec![
            format!("Canned result 1 for \"{query}\"."),
            format!("Canned result 2 for \"{query}\"."),
        ])
    }
}

pub struct DryRunReminders {
    log: LogSink,
    items: Mutex<Vec<(String, ReminderTime)>>,
}

impl ReminderStore for DryRunReminders {
    fn create(&self, text: &str, when: ReminderTime) -> Result<(), ReminderError> {
        (self.log)(format!(
            "[{DRY_RUN_MARKER}] would store reminder {text:?} ({when:?})"
        ));
        lock(&self.items).push((text.into(), when));
        Ok(())
    }

    fn list(&self) -> Result<Vec<String>, ReminderError> {
        Ok(lock(&self.items).iter().map(|(t, _)| t.clone()).collect())
    }
}

#[cfg(test)]
mod tests {
    use niva_core::safety::resolve_close_target;
    use niva_core::tools::ToolName;

    use super::*;

    fn names(reply: &ChatReply) -> Vec<&str> {
        reply.calls.iter().map(|c| c.name.as_str()).collect()
    }

    fn decide_en(user: &str) -> ChatReply {
        decide("", user, false)
    }

    fn arg_of<'a>(call: &'a ToolCall, key: &str) -> &'a ArgValue {
        call.args
            .get(key)
            .unwrap_or_else(|| panic!("no {key}: {call:?}"))
    }

    #[test]
    fn open_and_close_map_to_their_tools_with_the_app_name() {
        for (utterance, tool, app) in [
            ("open notepad", "open_app", "notepad"),
            ("abrí Spotify", "open_app", "Spotify"),
            ("open the calculator please", "open_app", "calculator"),
            ("close spotify", "close_app", "spotify"),
            ("cierra el notepad", "close_app", "notepad"),
            ("cerrá chrome por favor", "close_app", "chrome"),
        ] {
            let reply = decide_en(utterance);
            assert_eq!(names(&reply), vec![tool], "{utterance}");
            assert_eq!(arg_of(&reply.calls[0], "name"), &text(app), "{utterance}");
        }
    }

    #[test]
    fn volume_commands_cover_level_delta_and_mute() {
        let level = |u: &str| decide_en(u).calls[0].args.clone();
        assert_eq!(level("volume 30")["level"], ArgValue::Int(30));
        assert_eq!(level("volumen al 45")["level"], ArgValue::Int(45));
        assert_eq!(level("turn the volume up")["delta"], ArgValue::Int(10));
        assert_eq!(level("baja el volumen")["delta"], ArgValue::Int(-10));
        assert_eq!(level("mute")["mute"], ArgValue::Bool(true));
        assert_eq!(level("unmute")["mute"], ArgValue::Bool(false));
        assert_eq!(level("volume 150")["level"], ArgValue::Int(150));
    }

    #[test]
    fn reminders_with_and_without_a_time_and_the_list() {
        let full = decide_en("remind me to call mom in 10 minutes");
        assert_eq!(names(&full), vec!["create_reminder"]);
        assert_eq!(arg_of(&full.calls[0], "text"), &text("call mom"));
        assert_eq!(arg_of(&full.calls[0], "delay_minutes"), &ArgValue::Int(10));

        let at = decide_en("recuérdame llamar a mamá a las 18:30");
        assert_eq!(arg_of(&at.calls[0], "text"), &text("llamar a mamá"));
        assert_eq!(arg_of(&at.calls[0], "at_local_time"), &text("18:30"));

        let no_time = decide_en("remind me to call mom");
        assert_eq!(no_time.calls[0].args.len(), 1);
        let nothing = decide_en("remind me");
        assert!(nothing.calls[0].args.is_empty());
        let no_text = decide_en("remind me in 5 minutes");
        assert_eq!(no_text.calls[0].args.len(), 1);
        assert_eq!(
            arg_of(&no_text.calls[0], "delay_minutes"),
            &ArgValue::Int(5)
        );

        for utterance in ["list my reminders", "lista mis recordatorios"] {
            assert_eq!(
                names(&decide_en(utterance)),
                vec!["list_reminders"],
                "{utterance}"
            );
        }
    }

    #[test]
    fn a_follow_up_answer_completes_the_pending_reminder() {
        let system = format!(
            "You are Niva.\n{PENDING_REMINDER_MARKER} \"call mom\". The user is now giving the missing details."
        );
        for answer in ["in 10 minutes", "10", "en 10 minutos"] {
            let reply = decide(&system, answer, false);
            let call = &reply.calls[0];
            assert_eq!(arg_of(call, "text"), &text("call mom"), "{answer}");
            assert_eq!(
                arg_of(call, "delay_minutes"),
                &ArgValue::Int(10),
                "{answer}"
            );
        }
    }

    #[test]
    fn a_follow_up_for_a_reminder_with_nothing_known_fills_what_it_can() {
        let system = format!("You are Niva.\nThe user is {PENDING_REMINDER_ALL_MARKER}.");
        let reply = decide(&system, "call mom at 6", false);
        assert_eq!(names(&reply), vec!["create_reminder"]);
        assert_eq!(arg_of(&reply.calls[0], "text"), &text("call mom"));
        assert_eq!(arg_of(&reply.calls[0], "at_local_time"), &text("6"));
    }

    #[test]
    fn search_power_and_chit_chat() {
        let reply = decide_en("search rust lifetimes");
        assert_eq!(names(&reply), vec!["web_search"]);
        assert_eq!(arg_of(&reply.calls[0], "query"), &text("rust lifetimes"));
        for (utterance, action) in [
            ("lock the pc", "lock"),
            ("bloquea la compu", "lock"),
            ("put the pc to sleep", "sleep"),
            ("suspende el pc", "sleep"),
            ("restart the pc", "restart"),
            ("reinicia la compu", "restart"),
            ("shut down the pc", "shutdown"),
            ("apaga la compu", "shutdown"),
        ] {
            let reply = decide_en(utterance);
            assert_eq!(names(&reply), vec!["power_action"], "{utterance}");
            assert_eq!(
                arg_of(&reply.calls[0], "action"),
                &text(action),
                "{utterance}"
            );
        }
        let chat = decide_en("how are you?");
        assert!(chat.calls.is_empty() && !chat.text.is_empty());
        assert!(decide("", "hola", true).text.starts_with("Hola"));
    }

    #[test]
    fn several_commands_in_one_sentence_become_several_calls() {
        assert_eq!(
            names(&decide_en("search rust and close spotify")),
            vec!["web_search", "close_app"]
        );
        assert_eq!(
            names(&decide_en("volume 30 and close spotify")),
            vec!["set_volume", "close_app"]
        );
        assert_eq!(
            names(&decide_en("abre notepad y sube el volumen")),
            vec!["open_app", "set_volume"]
        );
        // A reminder keeps its own "and".
        let reminder = decide_en("remind me to call mom and dad in 10 minutes");
        assert_eq!(names(&reminder), vec!["create_reminder"]);
        assert_eq!(
            arg_of(&reminder.calls[0], "text"),
            &text("call mom and dad")
        );
    }

    #[test]
    fn every_dev_call_is_a_registered_tool() {
        for utterance in [
            "open notepad",
            "close spotify",
            "volume 30",
            "remind me to x in 5 minutes",
            "list my reminders",
            "search rust",
            "lock the pc",
        ] {
            for call in decide_en(utterance).calls {
                assert!(ToolName::from_name(&call.name).is_some(), "{call:?}");
            }
        }
    }

    fn quiet() -> LogSink {
        Arc::new(|_| {})
    }

    #[test]
    fn the_fake_process_list_is_realistic_and_explorer_is_protected() {
        let pc = DryRunPc::new(quiet());
        let list = pc.running_processes();
        for needed in [
            "notepad.exe",
            "notepad++.exe",
            "Spotify.exe",
            "explorer.exe",
        ] {
            assert!(list.iter().any(|p| p.exe_name == needed), "{needed}");
        }
        assert!(resolve_close_target("explorer", &list).is_err());
        assert!(resolve_close_target("notepad", &list).is_err(), "ambiguous");
        assert_eq!(
            resolve_close_target("spotify", &list),
            Ok("Spotify.exe".to_string())
        );
    }

    #[test]
    fn opening_unknown_apps_fails_and_known_ones_appear_in_the_list() {
        let pc = DryRunPc::new(quiet());
        assert_eq!(pc.open_app("foobar"), Err(PcError::NotFound));
        assert_eq!(pc.open_app("Calculator"), Ok(()));
        assert!(
            pc.running_processes()
                .iter()
                .any(|p| p.exe_name == "CalculatorApp.exe")
        );
    }

    #[test]
    fn volume_is_remembered_and_clamped() {
        let pc = DryRunPc::new(quiet());
        let up = pc.set_volume(VolumeChange::Delta(80)).unwrap();
        assert_eq!((up.level, up.clamped), (100, true));
        let muted = pc.set_volume(VolumeChange::Mute(true)).unwrap();
        assert!(muted.muted && muted.level == 100);
    }

    #[test]
    fn the_assistant_of_a_debug_build_only_ever_acts_on_the_dry_run_pc() {
        // The debug window's click-to-confirm is a Channel::Click answer, which
        // is safe only because of this (see the SECURITY note at the top).
        use niva_core::assistant::ConfirmInput;
        use niva_core::state_machine::RequestInput;

        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink: LogSink = {
            let lines = lines.clone();
            Arc::new(move |line| lock(&lines).push(line))
        };
        let assistant = assistant(sink);
        for (req, text) in [(1, "shut down the pc"), (2, "close spotify")] {
            let input = RequestInput::User {
                text: text.into(),
                slot: None,
            };
            assert!(!assistant.start_request(req, &input).is_empty());
            assistant.record_confirmation(ConfirmInput::Click(true));
            assert!(!assistant.run_tools(req).is_empty());
        }
        let lines = lock(&lines);
        let acted: Vec<&String> = lines
            .iter()
            .filter(|l| l.contains("would") || l.contains("nothing happened"))
            .collect();
        assert!(acted.len() >= 2, "{lines:?}");
        assert!(
            lines
                .iter()
                .filter(|l| !l.contains(DEV_LLM_MARKER))
                .all(|l| l.contains(DRY_RUN_MARKER)),
            "every non-LLM adapter line is a dry-run line: {lines:?}"
        );
    }

    #[test]
    fn the_dev_adapters_log_with_their_markers() {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let sink: LogSink = {
            let lines = lines.clone();
            Arc::new(move |line| lock(&lines).push(line))
        };
        let pc = DryRunPc::new(sink.clone());
        pc.open_app("notepad").unwrap();
        let llm = DevLlm { log: sink };
        llm.chat(&ChatRequest {
            system: String::new(),
            user: "open notepad".into(),
            allow_tools: true,
        })
        .unwrap();
        let lines = lock(&lines);
        assert!(lines.iter().any(|l| l.contains(DRY_RUN_MARKER)));
        assert!(lines.iter().any(|l| l.contains(DEV_LLM_MARKER)));
    }
}
