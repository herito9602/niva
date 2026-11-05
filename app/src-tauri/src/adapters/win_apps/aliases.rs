//! Spanish and English names of the built-in Windows apps.
//!
//! The Start menu shows an app under one language, and a PC can mix them
//! (`Calculator` next to `Configuración`). The model, or the user, may use the
//! other one. Each group below lists the names of ONE app in both languages;
//! `find_app` also scores an entry against every term of the group the query
//! belongs to.
//!
//! The lookup is by WHOLE query on purpose: a substring trigger would turn
//! "camara logitech" into Windows Camera instead of "Configuración de cámara
//! Logitech". The alias only widens what a query is compared with; the model's
//! text is still never used as anything but a comparison key against index
//! names. There is deliberately no browser, terminal, paint or regedit group.

use super::normalize;

/// Groups of terms for one app. Every term is exactly what [`normalize`]
/// produces (lower case, no accents, single spaces); a test enforces it.
pub(super) const GROUPS: &[&[&str]] = &[
    &["calculator", "calculadora"],
    &["notepad", "bloc de notas"],
    &["calendar", "calendario"],
    &["camera", "camara"],
    &["clock", "reloj", "alarmas", "alarms clock"],
    &["mail", "correo"],
    &["maps", "mapas"],
    &["photos", "fotos"],
    &["settings", "configuracion", "ajustes"],
    &[
        "file explorer",
        "explorador de archivos",
        "explorador",
        "file manager",
        "administrador de archivos",
    ],
    &["control panel", "panel de control"],
    &["command prompt", "simbolo del sistema"],
    &["snipping tool", "recortes", "herramienta de recortes"],
    &["sticky notes", "notas adhesivas", "notas rapidas"],
    &["weather", "clima"],
    &["news", "noticias"],
    &[
        "sound recorder",
        "voice recorder",
        "grabadora de sonidos",
        "grabadora de voz",
    ],
    &["movies tv", "peliculas y tv", "peliculas"],
    &["media player", "reproductor multimedia"],
    &["microsoft store", "store", "tienda", "tienda de microsoft"],
    &["phone link", "enlace movil"],
    &["get help", "obtener ayuda"],
    &["magnifier", "lupa"],
    &["narrator", "narrador"],
    &["on screen keyboard", "teclado en pantalla"],
    &["character map", "mapa de caracteres"],
    &["windows security", "seguridad de windows"],
];

/// Leading words dropped once from a query ("la calculadora").
const ARTICLES: [&str; 7] = ["el", "la", "los", "las", "the", "mi", "my"];
/// Trailing words dropped once ("calculadora app").
const APP_WORDS: [&str; 2] = ["app", "aplicacion"];

/// The terms of the group whose term EQUALS the query (normalised here, then
/// stripped of one leading article and one trailing `app` / `aplicacion`).
/// Empty when no group matches. The group includes the query's own term.
pub fn alias_terms(query: &str) -> &'static [&'static str] {
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
    GROUPS
        .iter()
        .find(|group| group.contains(&text))
        .copied()
        .unwrap_or(&[])
}
