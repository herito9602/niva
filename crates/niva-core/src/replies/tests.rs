//! Task 2.1: a test for every D4 template case, in both languages, plus the
//! language heuristic (design D1, spec `language-handling`).

use super::*;
use crate::tools::{PowerKind, ReminderTime};

const LANGS: [Lang; 2] = [Lang::Es, Lang::En];

fn app() -> String {
    "Spotify.exe".into()
}

/// One sample of every [`Msg`] variant. `variant_name` below is exhaustive, so
/// a new variant does not compile until it is named, and the count test then
/// forces it into this list.
fn samples() -> Vec<Msg> {
    let mut all = vec![
        Msg::Opened { app: app() },
        Msg::AppNotFound { app: app() },
        Msg::AppIndexLoading,
        Msg::OpenFailed { app: app() },
        Msg::ConfirmClose { exe: app() },
        Msg::Closed { exe: app() },
        Msg::StillRunning { exe: app() },
        Msg::CloseNoMatch,
        Msg::CloseTooShort { min: 3 },
        Msg::CloseTargetChanged,
        Msg::CloseNotRunning,
        Msg::CloseNoWindow { exe: app() },
        Msg::ClosedKeepsRunning { exe: app() },
        Msg::AskWhichApp {
            candidates: vec!["notepad.exe".into(), "notepad++.exe".into()],
        },
        Msg::VolumeSet {
            level: 40,
            clamped: false,
        },
        Msg::Muted,
        Msg::Unmuted { level: 40 },
        Msg::ReminderCreated {
            text: "call mom".into(),
            when: ReminderTime::InMinutes(10),
        },
        Msg::AskReminderTime,
        Msg::AskReminderText,
        Msg::AskReminderBoth,
        Msg::ReminderTimeInvalid,
        Msg::Reminders {
            items: vec!["call mom".into(), "pay rent".into()],
        },
        Msg::NoReminders,
        Msg::RemindersUnavailable,
        Msg::SearchNothing,
        Msg::SearchNotConfigured,
        Msg::SearchKeyRejected,
        Msg::SearchQuotaUsed,
        Msg::ToolFailed,
        Msg::InvalidRequest,
        Msg::MixedSearchRisky,
        Msg::TooManyCalls { max: 5 },
        Msg::DuplicateSearch,
        Msg::CancelledClose { exe: app() },
        Msg::LlmUnavailable,
        Msg::NotAvailableYet,
        Msg::SleepNotAvailable,
        Msg::Busy,
    ];
    all.extend(PowerKind::ALL.map(|action| Msg::CancelledPower { action }));
    all.extend(PowerKind::ALL.map(|action| Msg::ConfirmPower { action }));
    all.extend(PowerKind::ALL.map(|action| Msg::PowerDone { action }));
    all
}

fn variant_name(msg: &Msg) -> &'static str {
    match msg {
        Msg::Opened { .. } => "Opened",
        Msg::AppNotFound { .. } => "AppNotFound",
        Msg::AppIndexLoading => "AppIndexLoading",
        Msg::OpenFailed { .. } => "OpenFailed",
        Msg::ConfirmClose { .. } => "ConfirmClose",
        Msg::Closed { .. } => "Closed",
        Msg::StillRunning { .. } => "StillRunning",
        Msg::CloseNoMatch => "CloseNoMatch",
        Msg::CloseTooShort { .. } => "CloseTooShort",
        Msg::CloseTargetChanged => "CloseTargetChanged",
        Msg::CloseNotRunning => "CloseNotRunning",
        Msg::CloseNoWindow { .. } => "CloseNoWindow",
        Msg::ClosedKeepsRunning { .. } => "ClosedKeepsRunning",
        Msg::AskWhichApp { .. } => "AskWhichApp",
        Msg::ConfirmPower { .. } => "ConfirmPower",
        Msg::PowerDone { .. } => "PowerDone",
        Msg::VolumeSet { .. } => "VolumeSet",
        Msg::Muted => "Muted",
        Msg::Unmuted { .. } => "Unmuted",
        Msg::ReminderCreated { .. } => "ReminderCreated",
        Msg::AskReminderTime => "AskReminderTime",
        Msg::AskReminderText => "AskReminderText",
        Msg::AskReminderBoth => "AskReminderBoth",
        Msg::ReminderTimeInvalid => "ReminderTimeInvalid",
        Msg::Reminders { .. } => "Reminders",
        Msg::NoReminders => "NoReminders",
        Msg::RemindersUnavailable => "RemindersUnavailable",
        Msg::SearchNothing => "SearchNothing",
        Msg::SearchNotConfigured => "SearchNotConfigured",
        Msg::SearchKeyRejected => "SearchKeyRejected",
        Msg::SearchQuotaUsed => "SearchQuotaUsed",
        Msg::ToolFailed => "ToolFailed",
        Msg::InvalidRequest => "InvalidRequest",
        Msg::MixedSearchRisky => "MixedSearchRisky",
        Msg::TooManyCalls { .. } => "TooManyCalls",
        Msg::DuplicateSearch => "DuplicateSearch",
        Msg::CancelledClose { .. } => "CancelledClose",
        Msg::CancelledPower { .. } => "CancelledPower",
        Msg::LlmUnavailable => "LlmUnavailable",
        Msg::NotAvailableYet => "NotAvailableYet",
        Msg::SleepNotAvailable => "SleepNotAvailable",
        Msg::Busy => "Busy",
    }
}

#[test]
fn every_message_variant_has_a_sample() {
    // 42 variants; the power messages are sampled once per action.
    let mut names: Vec<&str> = samples().iter().map(variant_name).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 42, "a variant has no sample: {names:?}");
}

#[test]
fn the_search_setup_replies_say_the_literal_truth_in_both_languages() {
    assert_eq!(
        render(&Msg::SearchNotConfigured, Lang::En),
        "Web search needs a key. See the setup note."
    );
    assert_eq!(
        render(&Msg::SearchNotConfigured, Lang::Es),
        "La búsqueda web necesita una clave. Consulta la nota de configuración."
    );
    for lang in LANGS {
        let texts: Vec<String> = [
            Msg::SearchNotConfigured,
            Msg::SearchKeyRejected,
            Msg::SearchQuotaUsed,
            Msg::SearchNothing,
        ]
        .iter()
        .map(|msg| render(msg, lang))
        .collect();
        let mut unique = texts.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), 4, "{lang:?}: each case reads differently");
        assert!(texts.iter().all(|t| t.chars().count() <= 80), "{texts:?}");
    }
}

#[test]
fn every_message_renders_in_both_languages_and_the_languages_differ() {
    for msg in samples() {
        let es = render(&msg, Lang::Es);
        let en = render(&msg, Lang::En);
        assert!(!es.trim().is_empty(), "{msg:?} has no Spanish text");
        assert!(!en.trim().is_empty(), "{msg:?} has no English text");
        assert_ne!(es, en, "{msg:?} is not translated");
    }
}

#[test]
fn values_appear_in_the_text_in_both_languages() {
    for lang in LANGS {
        for (msg, needle) in [
            (Msg::Opened { app: app() }, "Spotify.exe"),
            (Msg::AppNotFound { app: app() }, "Spotify.exe"),
            (Msg::OpenFailed { app: app() }, "Spotify.exe"),
            (Msg::ConfirmClose { exe: app() }, "Spotify.exe"),
            (Msg::Closed { exe: app() }, "Spotify.exe"),
            (Msg::StillRunning { exe: app() }, "Spotify.exe"),
            (Msg::CloseNoWindow { exe: app() }, "Spotify.exe"),
            (Msg::ClosedKeepsRunning { exe: app() }, "Spotify.exe"),
            (Msg::CloseTooShort { min: 3 }, "3"),
            (Msg::TooManyCalls { max: 5 }, "5"),
            (Msg::CancelledClose { exe: app() }, "Spotify.exe"),
            (
                Msg::VolumeSet {
                    level: 40,
                    clamped: false,
                },
                "40",
            ),
            (Msg::Unmuted { level: 55 }, "55"),
        ] {
            assert!(
                render(&msg, lang).contains(needle),
                "{lang:?} {msg:?} lost {needle}"
            );
        }
    }
}

#[test]
fn a_which_one_question_lists_every_candidate_in_both_languages() {
    for lang in LANGS {
        let text = render(
            &Msg::AskWhichApp {
                candidates: vec!["a.exe".into(), "b.exe".into(), "c.exe".into()],
            },
            lang,
        );
        for name in ["a.exe", "b.exe", "c.exe"] {
            assert!(text.contains(name), "{lang:?}: {text}");
        }
        assert!(text.ends_with('?'), "{text}");
    }
}

#[test]
fn power_prompts_and_outcomes_name_each_action_differently() {
    for lang in LANGS {
        let prompts: Vec<String> = PowerKind::ALL
            .map(|action| render(&Msg::ConfirmPower { action }, lang))
            .to_vec();
        let done: Vec<String> = PowerKind::ALL
            .map(|action| render(&Msg::PowerDone { action }, lang))
            .to_vec();
        let cancelled: Vec<String> = PowerKind::ALL
            .map(|action| render(&Msg::CancelledPower { action }, lang))
            .to_vec();
        for texts in [prompts, done, cancelled] {
            let mut unique = texts.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(unique.len(), 4, "{lang:?}: {texts:?}");
        }
    }
}

#[test]
fn a_clamped_volume_says_so() {
    for lang in LANGS {
        let plain = render(
            &Msg::VolumeSet {
                level: 100,
                clamped: false,
            },
            lang,
        );
        let clamped = render(
            &Msg::VolumeSet {
                level: 100,
                clamped: true,
            },
            lang,
        );
        assert_ne!(plain, clamped, "{lang:?}");
        assert!(clamped.contains("100"));
    }
}

#[test]
fn reminder_confirmation_states_the_time_the_way_it_was_given() {
    let at = |hour, minute, date| Msg::ReminderCreated {
        text: "call mom".into(),
        when: ReminderTime::AtLocal { hour, minute, date },
    };
    let date = Some(CalendarDate {
        year: 2026,
        month: 11,
        day: 4,
    });
    for lang in LANGS {
        let in_ten = render(
            &Msg::ReminderCreated {
                text: "call mom".into(),
                when: ReminderTime::InMinutes(10),
            },
            lang,
        );
        assert!(
            in_ten.contains("10") && in_ten.contains("call mom"),
            "{in_ten}"
        );
        let one = render(
            &Msg::ReminderCreated {
                text: "x".into(),
                when: ReminderTime::InMinutes(1),
            },
            lang,
        );
        assert!(
            !one.contains("1 minutes") && !one.contains("1 minutos"),
            "{one}"
        );
        let nine = render(&at(9, 5, None), lang);
        assert!(
            nine.contains("09:05") && nine.contains("call mom"),
            "{nine}"
        );
        let dated = render(&at(9, 5, date), lang);
        assert!(
            dated.contains("2026-11-04") && dated.contains("09:05"),
            "{dated}"
        );
    }
}

#[test]
fn the_reminder_list_counts_and_names_the_reminders() {
    for lang in LANGS {
        let two = render(
            &Msg::Reminders {
                items: vec!["call mom".into(), "pay rent".into()],
            },
            lang,
        );
        assert!(two.contains('2') && two.contains("call mom") && two.contains("pay rent"));
        let one = render(
            &Msg::Reminders {
                items: vec!["call mom".into()],
            },
            lang,
        );
        assert!(one.contains('1') && one.contains("call mom"), "{one}");
        assert_ne!(one, two);
    }
}

#[test]
fn the_nine_reducer_templates_render_in_both_languages() {
    let all = [
        Template::VoiceNotReady,
        Template::Error,
        Template::Cancelled,
        Template::ActionDone,
        Template::SearchUnavailable,
        Template::ReminderAlert,
        Template::SleepCountdown,
        Template::SleepCancelled,
        Template::SleepNotWatched,
    ];
    for t in all {
        let es = template(t, Lang::Es);
        let en = template(t, Lang::En);
        assert!(!es.is_empty() && !en.is_empty(), "{t:?}");
        assert_ne!(es, en, "{t:?} is not translated");
    }
}

#[test]
fn english_templates_keep_the_texts_the_host_showed_before() {
    assert_eq!(
        template(Template::VoiceNotReady, Lang::En),
        "Voice is not ready. Type a command instead."
    );
    assert_eq!(template(Template::Error, Lang::En), "Something went wrong.");
    assert_eq!(template(Template::Cancelled, Lang::En), "Cancelled.");
    assert_eq!(template(Template::ActionDone, Lang::En), "Done.");
    assert_eq!(
        template(Template::SearchUnavailable, Lang::En),
        "I can't search right now."
    );
    assert_eq!(template(Template::ReminderAlert, Lang::En), "Reminder.");
}

#[test]
fn the_sleep_countdown_texts_are_truthful_in_both_languages() {
    assert_eq!(
        template(Template::SleepCountdown, Lang::En),
        "Suspending in 10 seconds. Press a key or move the mouse to cancel."
    );
    assert_eq!(
        template(Template::SleepCountdown, Lang::Es),
        "Suspendiendo en 10 segundos. Presiona una tecla o mueve el mouse para cancelar."
    );
    // A cancel says it was cancelled, never "done".
    assert_eq!(
        template(Template::SleepCancelled, Lang::En),
        "Suspend cancelled."
    );
    assert_eq!(
        template(Template::SleepCancelled, Lang::Es),
        "Suspensión cancelada."
    );
    for lang in [Lang::En, Lang::Es] {
        let cancelled = template(Template::SleepCancelled, lang);
        assert!(!cancelled.contains(template(Template::ActionDone, lang)));
        // The failure to watch says the PC was NOT suspended.
        let unwatched = template(Template::SleepNotWatched, lang);
        assert!(
            unwatched.contains("not suspend") || unwatched.contains("no suspendí"),
            "{unwatched}"
        );
    }
}

#[test]
fn the_island_countdown_text_has_the_seconds_placeholder_in_both_languages() {
    for lang in [Lang::En, Lang::Es] {
        let text = countdown_display(lang);
        assert_eq!(text.matches("{n}").count(), 1, "{text}");
        assert!(
            text.contains("cancel") || text.contains("cancelar"),
            "{text}"
        );
    }
}

#[test]
fn the_confirmation_buttons_have_labels_in_both_languages() {
    assert_eq!(confirm_labels(Lang::En), ("Confirm", "Cancel"));
    assert_eq!(confirm_labels(Lang::Es), ("Confirmar", "Cancelar"));
}

// --- language detection (D1 text heuristic) -----------------------------

#[test]
fn clear_spanish_and_english_commands_are_detected() {
    for (text, lang) in [
        ("open notepad", Lang::En),
        ("close the browser please", Lang::En),
        ("what time is it", Lang::En),
        ("turn the volume up", Lang::En),
        ("abre el bloc de notas", Lang::Es),
        ("sube el volumen al 30", Lang::Es),
        ("hola, cómo estás", Lang::Es),
        ("apaga la computadora", Lang::Es),
    ] {
        assert_eq!(detect_language(text), Some(lang), "{text}");
    }
}

#[test]
fn accented_or_spanish_punctuation_decides_even_with_an_english_app_name() {
    // Spec: an English app name must not change the reply language.
    for text in ["abrí Spotify", "cerrá Spotify", "¿qué hora es?", "mañana"] {
        assert_eq!(detect_language(text), Some(Lang::Es), "{text}");
    }
}

#[test]
fn a_tie_or_no_signal_is_none_so_the_caller_applies_the_fixed_default() {
    for text in ["", "   ", "spotify", "notepad", "ok", "12:30", "abre open"] {
        assert_eq!(detect_language(text), None, "{text:?}");
    }
}

#[test]
fn detection_is_per_text_with_no_memory() {
    assert_eq!(detect_language("abre notepad"), Some(Lang::Es));
    assert_eq!(detect_language("open notepad"), Some(Lang::En));
    assert_eq!(detect_language("abre notepad"), Some(Lang::Es));
}

#[test]
fn language_codes() {
    assert_eq!(Lang::Es.code(), "es");
    assert_eq!(Lang::En.code(), "en");
    assert_eq!(Lang::default(), DEFAULT_LANG);
}

// ---------------------------------------------------------------------------
// Task 2.4a: replies are short (spec `assistant-core`, "Replies are short")
// ---------------------------------------------------------------------------

fn chars(text: &str) -> usize {
    text.chars().count()
}

#[test]
fn a_short_reply_is_returned_untouched_apart_from_trimming() {
    assert_eq!(bound_reply("  Hello there.  "), "Hello there.");
    let exact = "a".repeat(REPLY_MAX_CHARS);
    assert_eq!(bound_reply(&exact), exact);
}

#[test]
fn a_1500_character_reply_is_cut_to_the_bound() {
    let long = "The quick brown fox jumps over the lazy dog and keeps running. ".repeat(24);
    assert!(chars(&long) > 1400);
    let bounded = bound_reply(&long);
    assert!(chars(&bounded) <= REPLY_MAX_CHARS, "{}", chars(&bounded));
    assert!(chars(&bounded) > REPLY_MAX_CHARS / 2, "cut far too early");
}

#[test]
fn the_cut_prefers_a_sentence_boundary_and_adds_no_ellipsis() {
    let sentence = "This is a complete sentence about nothing in particular. ";
    let long = sentence.repeat(30);
    let bounded = bound_reply(&long);
    assert!(bounded.ends_with("particular."), "{bounded:?}");
    assert!(!bounded.contains('…'));
    assert!(chars(&bounded) <= REPLY_MAX_CHARS);
}

#[test]
fn without_a_usable_sentence_end_the_cut_is_on_a_word_with_an_ellipsis() {
    let long = "word ".repeat(300);
    let bounded = bound_reply(&long);
    assert!(bounded.ends_with("word…"), "{bounded:?}");
    assert!(chars(&bounded) <= REPLY_MAX_CHARS);
}

#[test]
fn the_cut_never_ends_inside_a_word() {
    // Vary the length so the bound lands at every offset inside a long word.
    for pad in 0..40 {
        let long = format!("{}{}", "a ".repeat(pad), "internationalization ".repeat(40));
        let bounded = bound_reply(&long);
        let body = bounded.trim_end_matches('…');
        let last = body.split_whitespace().last().unwrap();
        assert!(
            last == "a" || last == "internationalization",
            "pad {pad}: ends with {last:?}"
        );
        assert!(chars(&bounded) <= REPLY_MAX_CHARS, "pad {pad}");
    }
}

#[test]
fn a_sentence_end_too_early_to_be_useful_is_not_used() {
    // One short sentence, then a long run-on: keeping only "Hi." would be silly.
    let long = format!("Hi. {}", "and then something else happened ".repeat(40));
    let bounded = bound_reply(&long);
    assert!(chars(&bounded) > REPLY_MAX_CHARS / 2, "{bounded:?}");
    assert!(bounded.ends_with('…'));
}

#[test]
fn decimals_and_abbreviations_without_a_following_space_are_not_sentence_ends() {
    let long = format!("{}3.5 is the value we want ", "number ".repeat(52));
    let long = long.repeat(4);
    let bounded = bound_reply(&long);
    assert!(
        !bounded.trim_end_matches('…').ends_with("3."),
        "{bounded:?}"
    );
}

#[test]
fn a_closing_quote_stays_with_its_sentence() {
    let long = format!(
        "{} \"This is the end of the quote.\" {}",
        "filler words ".repeat(20),
        "more text after the quote ".repeat(40)
    );
    let bounded = bound_reply(&long);
    assert!(bounded.ends_with("quote.\""), "{bounded:?}");
}

#[test]
fn accents_and_multibyte_text_are_cut_on_character_boundaries() {
    let long = "¿Qué tal estás? Todo está muy bien, gracias por preguntar. ".repeat(30);
    let bounded = bound_reply(&long);
    assert!(chars(&bounded) <= REPLY_MAX_CHARS);
    assert!(bounded.ends_with('.') || bounded.ends_with('?') || bounded.ends_with('…'));
}

#[test]
fn one_giant_token_is_hard_cut_with_an_ellipsis_within_the_bound() {
    let bounded = bound_reply(&"x".repeat(1000));
    assert_eq!(chars(&bounded), REPLY_MAX_CHARS);
    assert!(bounded.ends_with('…'));
}

#[test]
fn trailing_commas_are_dropped_before_the_ellipsis() {
    let long = "alpha, beta, gamma, delta, ".repeat(40);
    let bounded = bound_reply(&long);
    assert!(!bounded.ends_with(",…"), "{bounded:?}");
    assert!(bounded.ends_with('…'));
}
