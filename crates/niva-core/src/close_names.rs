//! Spanish and English names of built-in Windows apps, mapped to the stem of
//! their process (PR 2d-b).
//!
//! `close_app` matches the model's text against PROCESS names, and those differ
//! from what the user says: "calculadora" is `CalculatorApp.exe`, "bloc de
//! notas" is `notepad.exe`, "paint" is `mspaint.exe`. This table turns such a
//! name into the process stem BEFORE the safety gate sees it. Nothing here
//! decides what may be closed: the mapped stem still goes through the gate's
//! substring matcher, the protected list and the session filter, and a stem
//! that matches several executables is still ambiguous.
//!
//! The lookup is EXACT: the whole query, after one leading article and one
//! trailing `app` / `aplicacion` are dropped, must equal a term. Never a
//! substring, never fuzzy (design "Safety Gate"). There is deliberately no
//! group for the shell, the browser or a terminal.

/// One built-in app: the names people use for it and the stem of its process.
struct Entry {
    terms: &'static [&'static str],
    stem: &'static str,
}

/// Every term is exactly what [`normalize`] produces; a test enforces it.
const TABLE: &[Entry] = &[
    Entry {
        terms: &["calculator", "calculadora"],
        stem: "calculator",
    },
    Entry {
        terms: &["notepad", "bloc de notas"],
        stem: "notepad",
    },
    Entry {
        terms: &["paint", "pintura"],
        stem: "mspaint",
    },
    Entry {
        terms: &["photos", "fotos"],
        stem: "photos",
    },
    Entry {
        terms: &["camera", "camara"],
        stem: "windowscamera",
    },
    Entry {
        terms: &["snipping tool", "herramienta de recortes", "recortes"],
        stem: "snippingtool",
    },
    Entry {
        terms: &["sticky notes", "notas adhesivas", "notas rapidas"],
        stem: "stickynotes",
    },
    Entry {
        terms: &["magnifier", "lupa"],
        stem: "magnify",
    },
    Entry {
        terms: &["narrator", "narrador"],
        stem: "narrator",
    },
    Entry {
        terms: &["task manager", "administrador de tareas"],
        stem: "taskmgr",
    },
    Entry {
        terms: &["settings", "configuracion", "ajustes"],
        stem: "systemsettings",
    },
    Entry {
        terms: &["character map", "mapa de caracteres"],
        stem: "charmap",
    },
];

/// Leading words dropped once from a query ("la calculadora").
const ARTICLES: [&str; 7] = ["el", "la", "los", "las", "the", "mi", "my"];
/// Trailing words dropped once ("calculadora app").
const APP_WORDS: [&str; 2] = ["app", "aplicacion"];

fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        other => other,
    }
}

/// Lower case, accents folded, letters and digits only, single spaces. `+` and
/// `#` are spelled out first: "notepad++" is another app, not "notepad".
fn normalize(text: &str) -> String {
    let text = text.replace('+', " plus ").replace('#', " sharp ");
    let mut out = String::with_capacity(text.len());
    for c in text.chars().flat_map(char::to_lowercase).map(fold) {
        if c.is_alphanumeric() {
            out.push(c);
        } else if !out.ends_with(' ') && !out.is_empty() {
            out.push(' ');
        }
    }
    out.truncate(out.trim_end().len());
    out
}

/// The process stem for a built-in app named `query`, or `None` when the whole
/// query is not a known name (the query then goes to the gate unchanged).
pub fn process_stem_for(query: &str) -> Option<&'static str> {
    let normalized = normalize(query);
    let mut text = normalized.as_str();
    if let Some((first, rest)) = text.split_once(' ')
        && ARTICLES.contains(&first)
    {
        text = rest;
    }
    if let Some((rest, last)) = text.rsplit_once(' ')
        && APP_WORDS.contains(&last)
    {
        text = rest;
    }
    TABLE
        .iter()
        .find(|entry| entry.terms.contains(&text))
        .map(|entry| entry.stem)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::safety::{MIN_QUERY_CHARS, is_protected};

    #[test]
    fn spanish_and_english_names_map_to_the_process_stem() {
        for (query, stem) in [
            ("calculadora", "calculator"),
            ("Calculator", "calculator"),
            ("bloc de notas", "notepad"),
            ("Bloc de Notas", "notepad"),
            ("pintura", "mspaint"),
            ("Paint", "mspaint"),
            ("fotos", "photos"),
            ("cámara", "windowscamera"),
            ("herramienta de recortes", "snippingtool"),
            ("notas adhesivas", "stickynotes"),
            ("configuración", "systemsettings"),
            ("administrador de tareas", "taskmgr"),
        ] {
            assert_eq!(process_stem_for(query), Some(stem), "{query}");
        }
    }

    #[test]
    fn an_article_and_an_app_word_are_dropped_once() {
        assert_eq!(process_stem_for("la calculadora"), Some("calculator"));
        assert_eq!(process_stem_for("calculadora app"), Some("calculator"));
        assert_eq!(process_stem_for("la app"), None);
        assert_eq!(
            process_stem_for("el bloc de notas aplicacion"),
            Some("notepad")
        );
        assert_eq!(process_stem_for("la la calculadora"), None);
    }

    #[test]
    fn the_match_is_exact_never_a_substring_or_a_typo() {
        for query in [
            "calculadora cientifica",
            "calculadoras",
            "calculado",
            "notepad++",
            "bloc de",
            "paint 3d",
            "camara logitech",
            "fotos de verano",
            "chrome",
            "spotify",
            "",
            "  ",
        ] {
            assert_eq!(process_stem_for(query), None, "{query:?}");
        }
    }

    #[test]
    fn a_name_that_is_not_in_the_table_is_left_alone() {
        // No group for the shell, the browser or the terminal.
        for query in [
            "explorer",
            "explorador de archivos",
            "navegador",
            "terminal",
        ] {
            assert_eq!(process_stem_for(query), None, "{query}");
        }
    }

    #[test]
    fn every_term_is_normalized_and_every_stem_is_a_safe_needle() {
        for entry in TABLE {
            for term in entry.terms {
                assert_eq!(normalize(term), *term, "term {term:?} is not normalized");
            }
            assert!(
                entry.stem.chars().count() >= MIN_QUERY_CHARS,
                "{} is too short for the matcher",
                entry.stem
            );
            assert_eq!(entry.stem, entry.stem.to_lowercase());
            assert!(!entry.stem.contains(' '), "{}", entry.stem);
            assert!(
                !is_protected(&format!("{}.exe", entry.stem)),
                "{} is protected: the gate would never match it",
                entry.stem
            );
        }
    }

    #[test]
    fn no_term_belongs_to_two_apps() {
        let mut seen = std::collections::HashSet::new();
        for entry in TABLE {
            for term in entry.terms {
                assert!(seen.insert(*term), "{term:?} is listed twice");
            }
        }
    }
}
