//! Localized replies (design D4: the reply source is single-valued).
//!
//! Templates write every reply except `web_search` summaries and chit-chat:
//! tool successes, all tool errors, "which one?" and follow-up questions,
//! `close_app` and power outcomes, confirmation prompts and cancels. The LLM
//! is never on this path. Spanish and English only (spec `language-handling`).
//!
//! - [`template`] renders the reducer's [`Template`] ids.
//! - [`Msg`] + [`render`] cover the replies that carry a value (an app name,
//!   a volume level, ...).
//! - [`detect_language`] is the text heuristic of design D1 (accented
//!   characters and an es/en word count); it says `None` on a tie so the
//!   caller applies a fixed default and never the previous request's language.

use crate::state_machine::Template;
use crate::tools::{CalendarDate, PowerKind, ReminderTime};

#[cfg(test)]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    Es,
    #[default]
    En,
}

impl Lang {
    /// ISO 639-1 code (what the TTS voice and the LLM prompt are given).
    pub const fn code(self) -> &'static str {
        match self {
            Lang::Es => "es",
            Lang::En => "en",
        }
    }
}

/// Longest reply the island shows or the voice speaks, in characters (spec
/// `assistant-core` "Replies are short"; the island reading time relies on
/// it). Templates are short by construction. Everything else goes through
/// [`bound_reply`]: free text from the model and search summaries. A summary is
/// appended after earlier tool lines, so long earlier lines can crowd it out
/// of the bound.
///
/// The prompt (`prompt::system_prompt`) asks the model for at most 300
/// characters, the wording E1 measured; this 400 is the product cap that
/// catches a model that ignores that ask. Change one, re-check the other.
pub const REPLY_MAX_CHARS: usize = 400;

/// A sentence cut shorter than this is not worth keeping: the reply would
/// shrink to a greeting. A word cut is used instead.
const MIN_SENTENCE_CHARS: usize = REPLY_MAX_CHARS / 4;

/// Cuts a model reply to [`REPLY_MAX_CHARS`] (spec `assistant-core`, "Replies
/// are short"). Text within the bound is only trimmed. A longer text is cut at
/// the last sentence end that keeps a useful amount of it, with no ellipsis;
/// failing that, at the last whole word, trailing commas dropped, with an
/// ellipsis that counts toward the bound. The island must never show a cut
/// inside a word. The one exception is a single token longer than the bound
/// (no space to cut at): it is cut hard and marked with the ellipsis.
pub fn bound_reply(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= REPLY_MAX_CHARS {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    match sentence_end(&chars) {
        Some(end) => chars[..end].iter().collect(),
        None => word_cut(&chars),
    }
}

/// The end (exclusive) of the last sentence that fits: a `.`, `!`, `?` or
/// `…`, optional closing quotes or brackets, then a space (so `3.5` and
/// `e.g.`-style runs are not ends).
fn sentence_end(chars: &[char]) -> Option<usize> {
    let mut best = None;
    for (at, c) in chars.iter().enumerate().take(REPLY_MAX_CHARS) {
        if !matches!(c, '.' | '!' | '?' | '…') {
            continue;
        }
        let mut end = at + 1;
        while chars
            .get(end)
            .is_some_and(|c| matches!(c, '"' | '\'' | ')' | ']' | '”' | '’'))
        {
            end += 1;
        }
        let at_boundary = chars.get(end).is_none_or(|c| c.is_whitespace());
        if end <= REPLY_MAX_CHARS && at_boundary && end >= MIN_SENTENCE_CHARS {
            best = Some(end);
        }
    }
    best
}

fn word_cut(chars: &[char]) -> String {
    // One char of room for the ellipsis.
    let room = REPLY_MAX_CHARS - 1;
    let head = &chars[..room];
    // The head's last word is whole only if a space follows it in the text.
    let whole = chars[room].is_whitespace();
    let kept: &[char] = if whole {
        head
    } else {
        match head.iter().rposition(|c| c.is_whitespace()) {
            Some(at) => &head[..at],
            None => head,
        }
    };
    let mut out: String = kept.iter().collect();
    let trimmed = out
        .trim_end_matches(|c: char| {
            c.is_whitespace() || matches!(c, ',' | ';' | ':' | '-' | '–' | '—')
        })
        .len();
    out.truncate(trimmed);
    if !out.ends_with(['.', '!', '?']) {
        out.push('…');
    }
    out
}

/// Used when the text gives no signal (a tie) and there is no pending turn.
pub const DEFAULT_LANG: Lang = Lang::En;

/// Every reply that carries a value. Adding a variant breaks the exhaustive
/// match in [`render`] (and the sample list in the tests) until it has both
/// languages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    // open_app
    Opened {
        app: String,
    },
    AppNotFound {
        app: String,
    },
    AppIndexLoading,
    OpenFailed {
        app: String,
    },
    // close_app
    ConfirmClose {
        exe: String,
    },
    Closed {
        exe: String,
    },
    StillRunning {
        exe: String,
    },
    /// No running app matches: not running, or a near-miss name (one wording).
    CloseNoMatch,
    CloseTooShort {
        min: usize,
    },
    CloseTargetChanged,
    CloseNotRunning,
    /// The window closed but the app keeps running in the background.
    ClosedKeepsRunning {
        exe: String,
    },
    /// The app is running but has no window to close (nothing was posted).
    CloseNoWindow {
        exe: String,
    },
    AskWhichApp {
        candidates: Vec<String>,
    },
    // power
    ConfirmPower {
        action: PowerKind,
    },
    PowerDone {
        action: PowerKind,
    },
    // volume
    VolumeSet {
        level: u8,
        clamped: bool,
    },
    Muted,
    Unmuted {
        level: u8,
    },
    // reminders
    ReminderCreated {
        text: String,
        when: ReminderTime,
    },
    AskReminderTime,
    AskReminderText,
    AskReminderBoth,
    ReminderTimeInvalid,
    Reminders {
        items: Vec<String>,
    },
    NoReminders,
    RemindersUnavailable,
    // search
    SearchNothing,
    /// The search provider has no API key.
    SearchNotConfigured,
    /// The source rejected the API key.
    SearchKeyRejected,
    /// The key's quota is used up.
    SearchQuotaUsed,
    // generic errors
    ToolFailed,
    InvalidRequest,
    MixedSearchRisky,
    /// The response had more tool calls than one request may carry.
    TooManyCalls {
        max: usize,
    },
    /// Two `web_search` calls in one response.
    DuplicateSearch,
    /// A risky `close_app` call that was not confirmed, so it did not run.
    CancelledClose {
        exe: String,
    },
    /// A risky power action that was not confirmed, so it did not run.
    CancelledPower {
        action: PowerKind,
    },
    LlmUnavailable,
    /// Release builds before the Ollama and Windows adapters exist.
    NotAvailableYet,
    /// Sleep on a PC where it has not been built for (no Modern Standby): the
    /// PC was not touched.
    SleepNotAvailable,
    /// Every worker is busy and the queue is full (a hung provider or tool).
    Busy,
}

/// The reducer's template ids, in the request's language. The English texts
/// are the ones the host showed before this module existed.
pub fn template(template: Template, lang: Lang) -> &'static str {
    match (template, lang) {
        (Template::VoiceNotReady, Lang::En) => "Voice is not ready. Type a command instead.",
        (Template::VoiceNotReady, Lang::Es) => "La voz no está lista. Escribe un comando.",
        (Template::Error, Lang::En) => "Something went wrong.",
        (Template::Error, Lang::Es) => "Algo salió mal.",
        (Template::Cancelled, Lang::En) => "Cancelled.",
        (Template::Cancelled, Lang::Es) => "Cancelado.",
        (Template::ActionDone, Lang::En) => "Done.",
        (Template::ActionDone, Lang::Es) => "Listo.",
        (Template::SearchUnavailable, Lang::En) => "I can't search right now.",
        (Template::SearchUnavailable, Lang::Es) => "No puedo buscar ahora mismo.",
        (Template::ReminderAlert, Lang::En) => "Reminder.",
        (Template::ReminderAlert, Lang::Es) => "Recordatorio.",
        (Template::SleepCountdown, Lang::En) => {
            "Suspending in 10 seconds. Press a key or move the mouse to cancel."
        }
        (Template::SleepCountdown, Lang::Es) => {
            "Suspendiendo en 10 segundos. Presiona una tecla o mueve el mouse para cancelar."
        }
        (Template::SleepCancelled, Lang::En) => "Suspend cancelled.",
        (Template::SleepCancelled, Lang::Es) => "Suspensión cancelada.",
        (Template::SleepNotWatched, Lang::En) => {
            "I couldn't check for your input, so I did not suspend the PC."
        }
        (Template::SleepNotWatched, Lang::Es) => {
            "No pude comprobar tu actividad, así que no suspendí el PC."
        }
    }
}

/// What the island shows while the sleep countdown runs: `{n}` is replaced by the
/// whole seconds left, counted down in the view from a deadline (not by one
/// event per second).
pub fn countdown_display(lang: Lang) -> &'static str {
    match lang {
        Lang::En => "Suspending in {n} s. Press a key or move the mouse to cancel.",
        Lang::Es => "Suspendiendo en {n} s. Presiona una tecla o mueve el mouse para cancelar.",
    }
}

/// The labels of the island's confirmation buttons, as `(confirm, cancel)`, in
/// the request's language. They come from here (not from the frontend) so the
/// buttons speak the language of the question they answer.
pub fn confirm_labels(lang: Lang) -> (&'static str, &'static str) {
    match lang {
        Lang::En => ("Confirm", "Cancel"),
        Lang::Es => ("Confirmar", "Cancelar"),
    }
}

/// "a", "a or b", "a, b or c".
fn join_or(items: &[String], or: &str) -> String {
    match items {
        [] => String::new(),
        [only] => only.clone(),
        [init @ .., last] => format!("{} {or} {last}", init.join(", ")),
    }
}

fn date_text(date: CalendarDate) -> String {
    format!("{:04}-{:02}-{:02}", date.year, date.month, date.day)
}

fn when_text(when: ReminderTime, lang: Lang) -> String {
    match (when, lang) {
        (ReminderTime::InMinutes(1), Lang::En) => "in 1 minute".into(),
        (ReminderTime::InMinutes(n), Lang::En) => format!("in {n} minutes"),
        (ReminderTime::InMinutes(1), Lang::Es) => "en 1 minuto".into(),
        (ReminderTime::InMinutes(n), Lang::Es) => format!("en {n} minutos"),
        (ReminderTime::AtLocal { hour, minute, date }, Lang::En) => match date {
            Some(date) => format!("{} at {hour:02}:{minute:02}", date_text(date)),
            None => format!("at {hour:02}:{minute:02}"),
        },
        (ReminderTime::AtLocal { hour, minute, date }, Lang::Es) => match date {
            Some(date) => format!("el {} a las {hour:02}:{minute:02}", date_text(date)),
            None => format!("a las {hour:02}:{minute:02}"),
        },
    }
}

pub fn render(msg: &Msg, lang: Lang) -> String {
    match lang {
        Lang::En => render_en(msg),
        Lang::Es => render_es(msg),
    }
}

fn render_en(msg: &Msg) -> String {
    match msg {
        Msg::Opened { app } => format!("Opened {app}."),
        Msg::AppNotFound { app } => format!("I couldn't find an app called {app}."),
        Msg::AppIndexLoading => "I'm still loading the app list. Try again in a moment.".into(),
        Msg::OpenFailed { app } => format!("I couldn't open {app}."),
        Msg::ConfirmClose { exe } => format!("Close {exe}?"),
        Msg::Closed { exe } => format!("Closed {exe}."),
        Msg::StillRunning { exe } => {
            format!("{exe} is still running. It may be asking to save.")
        }
        Msg::CloseNoMatch => "I couldn't find a running app with that name.".into(),
        Msg::CloseTooShort { min } => {
            format!("Give me at least {min} characters of the app name.")
        }
        Msg::CloseTargetChanged => {
            "The app changed after you confirmed, so I didn't close anything.".into()
        }
        Msg::CloseNotRunning => "That app is no longer running.".into(),
        Msg::ClosedKeepsRunning { exe } => {
            format!("I closed the window of {exe}, but it keeps running in the background.")
        }
        Msg::CloseNoWindow { exe } => {
            format!("I found no window of {exe} to close. Nothing was done.")
        }
        Msg::AskWhichApp { candidates } => format!("Which one: {}?", join_or(candidates, "or")),
        Msg::ConfirmPower { action } => match action {
            PowerKind::Lock => "Lock the PC?",
            PowerKind::Sleep => "Put the PC to sleep?",
            PowerKind::Restart => "Restart the PC?",
            PowerKind::Shutdown => "Shut down the PC?",
        }
        .into(),
        Msg::PowerDone { action } => match action {
            PowerKind::Lock => "Locking the PC.",
            PowerKind::Sleep => "Suspending the PC.",
            PowerKind::Restart => "Restarting the PC.",
            PowerKind::Shutdown => "Shutting down the PC.",
        }
        .into(),
        Msg::VolumeSet { level, clamped } => {
            if *clamped {
                format!("Volume at {level}%, the limit.")
            } else {
                format!("Volume at {level}%.")
            }
        }
        Msg::Muted => "Muted.".into(),
        Msg::Unmuted { level } => format!("Unmuted. Volume at {level}%."),
        Msg::ReminderCreated { text, when } => {
            format!("Reminder set {}: {text}.", when_text(*when, Lang::En))
        }
        Msg::AskReminderTime => "At what time?".into(),
        Msg::AskReminderText => "What should I remind you about?".into(),
        Msg::AskReminderBoth => "What should I remind you about, and when?".into(),
        Msg::ReminderTimeInvalid => "Give me a time in the future.".into(),
        Msg::Reminders { items } => match items.len() {
            1 => format!("You have 1 reminder: {}.", items.join("; ")),
            n => format!("You have {n} reminders: {}.", items.join("; ")),
        },
        Msg::NoReminders => "You have no reminders.".into(),
        Msg::RemindersUnavailable => "I can't manage reminders yet.".into(),
        Msg::SearchNothing => "I found nothing for that search.".into(),
        Msg::SearchNotConfigured => "Web search needs a key. See the setup note.".into(),
        Msg::SearchKeyRejected => "The search key was rejected. Check it in the setup note.".into(),
        Msg::SearchQuotaUsed => "The search quota is used up for now.".into(),
        Msg::ToolFailed => "That didn't work.".into(),
        Msg::InvalidRequest => "I didn't understand that request.".into(),
        Msg::MixedSearchRisky => {
            "I can't search and do that in the same request. Ask me one at a time.".into()
        }
        Msg::TooManyCalls { max } => {
            format!("That is too many actions at once. Ask for at most {max}.")
        }
        Msg::DuplicateSearch => "I can only run one search per request.".into(),
        Msg::CancelledClose { exe } => format!("Cancelled closing {exe}."),
        Msg::CancelledPower { action } => match action {
            PowerKind::Lock => "Cancelled locking the PC.",
            PowerKind::Sleep => "Cancelled putting the PC to sleep.",
            PowerKind::Restart => "Cancelled restarting the PC.",
            PowerKind::Shutdown => "Cancelled shutting down the PC.",
        }
        .into(),
        Msg::LlmUnavailable => "I can't think right now. Is the language model running?".into(),
        Msg::NotAvailableYet => "Assistant commands are not available yet.".into(),
        Msg::SleepNotAvailable => "Sleep isn't available on this PC yet. Nothing was done.".into(),
        Msg::Busy => "I'm still busy with other requests. Try again in a moment.".into(),
    }
}

fn render_es(msg: &Msg) -> String {
    match msg {
        Msg::Opened { app } => format!("Abrí {app}."),
        Msg::AppNotFound { app } => format!("No encontré ninguna app llamada {app}."),
        Msg::AppIndexLoading => {
            "Todavía estoy cargando la lista de apps. Inténtalo de nuevo en un momento.".into()
        }
        Msg::OpenFailed { app } => format!("No pude abrir {app}."),
        Msg::ConfirmClose { exe } => format!("¿Cerrar {exe}?"),
        Msg::Closed { exe } => format!("Cerré {exe}."),
        Msg::StillRunning { exe } => {
            format!("{exe} sigue abierto. Puede que esté pidiendo guardar.")
        }
        Msg::CloseNoMatch => "No encontré ninguna app abierta con ese nombre.".into(),
        Msg::CloseTooShort { min } => {
            format!("Dame al menos {min} caracteres del nombre de la app.")
        }
        Msg::CloseTargetChanged => {
            "La app cambió después de que confirmaras, así que no cerré nada.".into()
        }
        Msg::CloseNotRunning => "Esa app ya no está abierta.".into(),
        Msg::ClosedKeepsRunning { exe } => {
            format!("Cerré la ventana de {exe}, pero sigue ejecutándose en segundo plano.")
        }
        Msg::CloseNoWindow { exe } => {
            format!("No encontré ninguna ventana de {exe} para cerrar. No hice nada.")
        }
        Msg::AskWhichApp { candidates } => format!("¿Cuál: {}?", join_or(candidates, "o")),
        Msg::ConfirmPower { action } => match action {
            PowerKind::Lock => "¿Bloquear el PC?",
            PowerKind::Sleep => "¿Suspender el PC?",
            PowerKind::Restart => "¿Reiniciar el PC?",
            PowerKind::Shutdown => "¿Apagar el PC?",
        }
        .into(),
        Msg::PowerDone { action } => match action {
            PowerKind::Lock => "Bloqueando el PC.",
            PowerKind::Sleep => "Suspendiendo el PC.",
            PowerKind::Restart => "Reiniciando el PC.",
            PowerKind::Shutdown => "Apagando el PC.",
        }
        .into(),
        Msg::VolumeSet { level, clamped } => {
            if *clamped {
                format!("Volumen al {level}%, el límite.")
            } else {
                format!("Volumen al {level}%.")
            }
        }
        Msg::Muted => "Silenciado.".into(),
        Msg::Unmuted { level } => format!("Sonido activado. Volumen al {level}%."),
        Msg::ReminderCreated { text, when } => {
            format!("Recordatorio {}: {text}.", when_text(*when, Lang::Es))
        }
        Msg::AskReminderTime => "¿A qué hora?".into(),
        Msg::AskReminderText => "¿Qué quieres que te recuerde?".into(),
        Msg::AskReminderBoth => "¿Qué te recuerdo y cuándo?".into(),
        Msg::ReminderTimeInvalid => "Dame una hora en el futuro.".into(),
        Msg::Reminders { items } => match items.len() {
            1 => format!("Tienes 1 recordatorio: {}.", items.join("; ")),
            n => format!("Tienes {n} recordatorios: {}.", items.join("; ")),
        },
        Msg::NoReminders => "No tienes recordatorios.".into(),
        Msg::RemindersUnavailable => "Todavía no puedo gestionar recordatorios.".into(),
        Msg::SearchNothing => "No encontré nada para esa búsqueda.".into(),
        Msg::SearchNotConfigured => {
            "La búsqueda web necesita una clave. Consulta la nota de configuración.".into()
        }
        Msg::SearchKeyRejected => {
            "La clave de búsqueda fue rechazada. Revísala en la nota de configuración.".into()
        }
        Msg::SearchQuotaUsed => "La cuota de búsquedas se agotó por ahora.".into(),
        Msg::ToolFailed => "Eso no funcionó.".into(),
        Msg::InvalidRequest => "No entendí esa petición.".into(),
        Msg::MixedSearchRisky => {
            "No puedo buscar y hacer eso en la misma petición. Pídemelo por separado.".into()
        }
        Msg::TooManyCalls { max } => {
            format!("Son demasiadas acciones a la vez. Pídeme {max} como máximo.")
        }
        Msg::DuplicateSearch => "Solo puedo hacer una búsqueda por petición.".into(),
        Msg::CancelledClose { exe } => format!("Cancelé el cierre de {exe}."),
        Msg::CancelledPower { action } => match action {
            PowerKind::Lock => "Cancelé el bloqueo del PC.",
            PowerKind::Sleep => "Cancelé la suspensión del PC.",
            PowerKind::Restart => "Cancelé el reinicio del PC.",
            PowerKind::Shutdown => "Cancelé el apagado del PC.",
        }
        .into(),
        Msg::LlmUnavailable => {
            "No puedo pensar ahora mismo. ¿Está activo el modelo de lenguaje?".into()
        }
        Msg::NotAvailableYet => "Los comandos del asistente aún no están disponibles.".into(),
        Msg::SleepNotAvailable => {
            "La suspensión aún no está disponible en este PC. No hice nada.".into()
        }
        Msg::Busy => "Sigo ocupado con otras peticiones. Inténtalo de nuevo en un momento.".into(),
    }
}

// Command words and common function words. Deliberately small: the signal is
// the accented/ñ/¿¡ characters plus these. Words that are the same in both
// languages (a, no, ok, me) are in neither list.
const ES_WORDS: &[&str] = &[
    "el",
    "la",
    "los",
    "las",
    "de",
    "del",
    "que",
    "un",
    "una",
    "por",
    "para",
    "con",
    "mi",
    "mis",
    "al",
    "abre",
    "abrir",
    "cierra",
    "cerrar",
    "sube",
    "baja",
    "subir",
    "bajar",
    "volumen",
    "apaga",
    "apagar",
    "reinicia",
    "reiniciar",
    "bloquea",
    "bloquear",
    "suspende",
    "suspender",
    "silencia",
    "recuerdame",
    "recordame",
    "recordar",
    "busca",
    "buscar",
    "hola",
    "gracias",
    "favor",
    "quiero",
    "puedes",
    "pon",
    "poner",
    "hora",
    "cuando",
    "como",
    "donde",
    "computadora",
    "ordenador",
    "cuanto",
];
const EN_WORDS: &[&str] = &[
    "the", "an", "to", "of", "and", "is", "are", "you", "your", "my", "open", "close", "turn",
    "up", "down", "lock", "sleep", "restart", "shut", "shutdown", "mute", "unmute", "volume",
    "remind", "reminder", "search", "find", "what", "when", "where", "how", "who", "hello", "hi",
    "please", "thanks", "set", "list", "show", "pc", "computer", "time", "tell", "for", "with",
    "in", "at", "it", "this", "that", "can",
];

fn is_spanish_char(c: char) -> bool {
    matches!(c, 'á' | 'é' | 'í' | 'ó' | 'ú' | 'ü' | 'ñ' | '¿' | '¡')
}

/// `Some` when the text clearly reads as Spanish or English, `None` on a tie
/// or no signal (a lone app name, "ok").
pub fn detect_language(text: &str) -> Option<Lang> {
    let lowered = text.to_lowercase();
    let mut es = 2 * lowered.chars().filter(|c| is_spanish_char(*c)).count();
    let mut en = 0;
    for word in lowered.split(|c: char| !c.is_alphabetic()) {
        if ES_WORDS.contains(&word) {
            es += 1;
        }
        if EN_WORDS.contains(&word) {
            en += 1;
        }
    }
    match es.cmp(&en) {
        std::cmp::Ordering::Greater => Some(Lang::Es),
        std::cmp::Ordering::Less => Some(Lang::En),
        std::cmp::Ordering::Equal => None,
    }
}
