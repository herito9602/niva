//! The reply-language heuristic of E1 (`spikes/eval/run_e1.py`, `detect_lang`),
//! kept as it was so the "reply language >= 95%" gate compares with the spike.
//! It judges the model's free text (chit-chat and any text that comes with a
//! tool call), not the templates, which are per-language by construction.
//!
//! `score_es` = 3 x (accented letters, `ñ`, `¿`, `¡`) + Spanish stopwords;
//! `score_en` = English stopwords. A tie, including 0 to 0, is undecidable.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detected {
    Es,
    En,
    Undecidable,
}

impl Detected {
    pub fn code(self) -> &'static str {
        match self {
            Detected::Es => "es",
            Detected::En => "en",
            Detected::Undecidable => "undecidable",
        }
    }
}

const ES_STOP: &[&str] = &[
    "el",
    "la",
    "los",
    "las",
    "un",
    "una",
    "de",
    "del",
    "que",
    "qué",
    "y",
    "en",
    "es",
    "por",
    "para",
    "con",
    "no",
    "sí",
    "te",
    "me",
    "se",
    "lo",
    "al",
    "su",
    "mi",
    "hola",
    "listo",
    "gracias",
    "claro",
    "estoy",
    "está",
    "tengo",
    "puedo",
    "abrí",
    "abriendo",
    "cerrando",
    "volumen",
    "recordatorio",
    "recordatorios",
    "minutos",
    "ahora",
    "bien",
    "muy",
    "más",
    "pero",
    "como",
    "cómo",
    "vos",
    "tu",
    "tus",
    "unos",
    "unas",
    "hay",
    "dale",
    "ya",
];

const EN_STOP: &[&str] = &[
    "the",
    "a",
    "an",
    "of",
    "to",
    "and",
    "in",
    "is",
    "it",
    "you",
    "your",
    "i",
    "me",
    "my",
    "for",
    "with",
    "that",
    "this",
    "are",
    "was",
    "have",
    "has",
    "can",
    "will",
    "sure",
    "hello",
    "hi",
    "thanks",
    "thank",
    "done",
    "okay",
    "ok",
    "opening",
    "closing",
    "volume",
    "reminder",
    "reminders",
    "minutes",
    "now",
    "what",
    "how",
    "do",
    "does",
    "not",
    "but",
    "on",
    "at",
    "be",
    "there",
    "here",
    "just",
];

const ES_CHARS: &str = "áéíóúñ¿¡ü";

/// The words of the spike's regex `[a-záéíóúñü]+` over the lowercased text.
fn words(lower: &str) -> Vec<&str> {
    lower
        .split(|c: char| !(c.is_ascii_lowercase() || "áéíóúñü".contains(c)))
        .filter(|w| !w.is_empty())
        .collect()
}

pub fn detect_lang(text: &str) -> Detected {
    let lower = text.to_lowercase();
    let accents = lower.chars().filter(|c| ES_CHARS.contains(*c)).count();
    let words = words(&lower);
    let score_es = 3 * accents + words.iter().filter(|w| ES_STOP.contains(w)).count();
    let score_en = words.iter().filter(|w| EN_STOP.contains(w)).count();
    match score_es.cmp(&score_en) {
        std::cmp::Ordering::Equal => Detected::Undecidable,
        std::cmp::Ordering::Greater => Detected::Es,
        std::cmp::Ordering::Less => Detected::En,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_english_and_spanish_replies() {
        assert_eq!(detect_lang("I'm fine, thanks. How are you?"), Detected::En);
        assert_eq!(detect_lang("Hola, estoy muy bien, gracias."), Detected::Es);
    }

    #[test]
    fn accents_weigh_three_stopwords() {
        // One accented letter (3) beats two English stopwords (2).
        assert_eq!(detect_lang("the cat é"), Detected::Es);
        assert_eq!(detect_lang("¿ok"), Detected::Es);
    }

    #[test]
    fn a_tie_or_no_signal_is_undecidable() {
        for text in ["", "   ", "Spotify", "12:30", "ok la", "xyz"] {
            // "ok la" is 1 English stopword against 1 Spanish one: a tie.
            assert_eq!(detect_lang(text), Detected::Undecidable, "{text:?}");
        }
    }

    #[test]
    fn words_are_runs_of_lowercase_letters_and_accented_vowels() {
        assert_eq!(words("hola, ¿cómo estás?"), ["hola", "cómo", "estás"]);
        assert_eq!(words("it's 3pm"), ["it", "s", "pm"]);
    }

    #[test]
    fn the_codes_are_the_spikes_strings() {
        assert_eq!(Detected::Es.code(), "es");
        assert_eq!(Detected::En.code(), "en");
        assert_eq!(Detected::Undecidable.code(), "undecidable");
    }
}
