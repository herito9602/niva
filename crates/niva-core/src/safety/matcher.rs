//! The `close_app` target matcher and the protected process list.
//!
//! Matching is a case-insensitive, whitespace-stripped substring test on the
//! process stem (file name without `.exe`). There is no fuzzy step anywhere:
//! the spike once mapped "Spotify" to `FortiTray.exe` with a fuzzy matcher.
//! When several executables match, the query is ambiguous, whatever it says
//! (even an exact file name): only [`resolve_pinned`], used for the user's
//! follow-up answer to "which one?", picks one by its whole file name.
//!
//! The process name comes from an adapter, so it is normalised before any
//! check (last path component, trimmed, trailing dots dropped, Unicode case
//! folded) and the protected list fails closed: a name that is still odd
//! afterwards (empty, control characters, characters Windows forbids in a
//! file name) counts as protected.

use crate::ports::ProcessInfo;
use crate::tools::ToolError;

/// Shortest accepted `close_app` query, in characters, after whitespace is
/// stripped (design "Safety Gate").
pub const MIN_QUERY_CHARS: usize = 3;

/// System processes (lower case stems) that are never matched and never
/// closed. `applicationframehost` hosts every UWP window and
/// `msedgewebview2` hosts Niva's own UI; both are additions to the design's
/// original list.
const PROTECTED_STEMS: [&str; 16] = [
    "explorer",
    "dwm",
    "csrss",
    "winlogon",
    "wininit",
    "lsass",
    "services",
    "smss",
    "svchost",
    "sihost",
    "ctfmon",
    "searchhost",
    "startmenuexperiencehost",
    "shellexperiencehost",
    "applicationframehost",
    "msedgewebview2",
];

/// Any stem starting with one of these is protected, with or without a
/// separator: Niva and its sidecar (`niva`, `NivaVoice`, `niva-eval`) and
/// Ollama with its helpers (`ollama app`, `ollama_llama_server`).
const PROTECTED_PREFIXES: [&str; 2] = ["niva", "ollama"];

/// Characters Windows does not allow in a file name (control characters are
/// checked separately). A path separator cannot survive the split.
const FORBIDDEN: [char; 7] = [':', '*', '?', '"', '<', '>', '|'];

/// `name` without a trailing `.exe`, any case.
fn strip_exe(name: &str) -> &str {
    let split = name.len().saturating_sub(4);
    if name.is_char_boundary(split) && name[split..].eq_ignore_ascii_case(".exe") {
        &name[..split]
    } else {
        name
    }
}

/// The file name of an adapter-supplied process name and its lower-case
/// stem, or `None` when the name is odd.
fn file_name_and_stem(raw: &str) -> Option<(&str, String)> {
    let last = raw.rsplit(['\\', '/']).next().unwrap_or("");
    let base = last
        .trim()
        .trim_end_matches(|c: char| c.is_whitespace() || c == '.');
    let stem = strip_exe(base).to_lowercase();
    let odd = stem.is_empty()
        || base
            .chars()
            .any(|c| c.is_control() || FORBIDDEN.contains(&c));
    (!odd).then_some((base, stem))
}

/// Lower case with all whitespace removed.
fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// True when the process is never matched and never closed. A name that
/// cannot be normalised is protected (fail closed).
pub fn is_protected(exe_name: &str) -> bool {
    let Some((_, stem)) = file_name_and_stem(exe_name) else {
        return true;
    };
    PROTECTED_STEMS.contains(&stem.as_str())
        || PROTECTED_PREFIXES
            .iter()
            .any(|prefix| stem.starts_with(prefix))
}

/// Resolves `query` to exactly one running, unprotected executable of the
/// user's session and returns its file name (no directory). Several distinct
/// executables are returned as [`ToolError::Ambiguous`] (never closed all).
pub fn resolve_close_target(query: &str, processes: &[ProcessInfo]) -> Result<String, ToolError> {
    let needle = normalize(strip_exe(query.trim()));
    if needle.chars().count() < MIN_QUERY_CHARS {
        return Err(ToolError::QueryTooShort {
            min: MIN_QUERY_CHARS,
        });
    }

    // (case-folded file name, file name as the adapter spelled it)
    let mut candidates: Vec<(String, &str)> = Vec::new();
    for process in processes {
        if !process.in_user_session || is_protected(&process.exe_name) {
            continue;
        }
        let Some((base, stem)) = file_name_and_stem(&process.exe_name) else {
            continue;
        };
        let key = base.to_lowercase();
        if normalize(&stem).contains(&needle) && !candidates.iter().any(|(known, _)| *known == key)
        {
            candidates.push((key, base));
        }
    }
    candidates.sort();

    match candidates.as_slice() {
        [] => Err(ToolError::NoMatch),
        [(_, only)] => Ok((*only).to_string()),
        several => Err(ToolError::Ambiguous(
            several
                .iter()
                .map(|(_, name)| (*name).to_string())
                .collect(),
        )),
    }
}

/// Resolves the user's follow-up pick: `exe` must be the whole file name
/// (case and surrounding spaces aside) of a running, unprotected executable of
/// the user's session. No substring, no prefix. This is the only exact-name
/// shortcut; the model's own `close_app` call never reaches it.
pub fn resolve_pinned(exe: &str, processes: &[ProcessInfo]) -> Result<String, ToolError> {
    let wanted = exe.trim().to_lowercase();
    if wanted.is_empty() {
        return Err(ToolError::NoMatch);
    }
    processes
        .iter()
        .filter(|process| process.in_user_session && !is_protected(&process.exe_name))
        .filter_map(|process| file_name_and_stem(&process.exe_name))
        .find(|(base, _)| base.to_lowercase() == wanted)
        .map(|(base, _)| base.to_string())
        .ok_or(ToolError::NoMatch)
}
