//! The real system prompt and tool schemas (task 2.10, verify 2a S6).

use super::*;
use crate::ports::LocalDateTime;
use crate::replies::Lang;
use crate::tools::{PowerKind, ToolName};

fn spec(name: &str) -> &'static ToolSpec {
    TOOL_SPECS
        .iter()
        .find(|spec| spec.name == name)
        .unwrap_or_else(|| panic!("no spec for {name}"))
}

fn sorted(mut names: Vec<&'static str>) -> Vec<&'static str> {
    names.sort_unstable();
    names
}

#[test]
fn there_is_one_schema_per_registered_tool() {
    let specs = sorted(TOOL_SPECS.iter().map(|s| s.name).collect());
    let tools = sorted(ToolName::ALL.iter().map(|t| t.as_str()).collect());
    assert_eq!(specs, tools);
}

#[test]
fn each_schema_offers_exactly_the_arguments_the_validator_accepts() {
    for tool in ToolName::ALL {
        let params = sorted(spec(tool.as_str()).params.iter().map(|p| p.name).collect());
        let allowed = sorted(tool.allowed_args().to_vec());
        assert_eq!(params, allowed, "{}", tool.as_str());
    }
}

#[test]
fn required_arguments_are_declared_parameters() {
    for spec in &TOOL_SPECS {
        for required in spec.required {
            assert!(
                spec.params.iter().any(|p| p.name == *required),
                "{}: {required} is required but not declared",
                spec.name
            );
        }
        assert!(!spec.description.is_empty(), "{}", spec.name);
    }
}

#[test]
fn set_volume_requires_nothing_so_the_model_sends_one_of_three() {
    // Verify 2a S6: the spike required both `level` and `mute`, so a model
    // that filled both hit `Conflicting`. All three are optional here.
    let volume = spec("set_volume");
    assert!(volume.required.is_empty());
    let names: Vec<_> = volume.params.iter().map(|p| p.name).collect();
    assert_eq!(names, ["level", "delta", "mute"]);
    assert!(volume.description.contains("exactly ONE"));
    let kinds: Vec<_> = volume.params.iter().map(|p| p.kind).collect();
    assert_eq!(
        kinds,
        [ParamKind::Integer, ParamKind::Integer, ParamKind::Boolean]
    );
}

#[test]
fn required_arguments_match_the_validator() {
    assert_eq!(spec("open_app").required, ["name"]);
    assert_eq!(spec("close_app").required, ["name"]);
    assert_eq!(spec("web_search").required, ["query"]);
    assert_eq!(spec("power_action").required, ["action"]);
    // The text is required; the time is deliberately not: a reminder without
    // one is a question for the core, not an error.
    assert_eq!(spec("create_reminder").required, ["text"]);
    assert!(spec("list_reminders").required.is_empty());
    assert!(spec("list_reminders").params.is_empty());
}

#[test]
fn power_action_offers_exactly_the_power_kinds() {
    let action = spec("power_action").params[0];
    let expected: Vec<&str> = PowerKind::ALL.iter().map(|k| k.as_str()).collect();
    match action.kind {
        ParamKind::OneOf(values) => assert_eq!(values, expected.as_slice()),
        other => panic!("action is {other:?}"),
    }
}

#[test]
fn the_tools_keep_the_order_e1_measured() {
    let order: Vec<_> = TOOL_SPECS.iter().map(|s| s.name).collect();
    assert_eq!(
        order,
        [
            "open_app",
            "close_app",
            "set_volume",
            "web_search",
            "power_action",
            "create_reminder",
            "list_reminders"
        ]
    );
}

fn now() -> LocalDateTime {
    LocalDateTime {
        year: 2026,
        month: 11,
        day: 4,
        hour: 14,
        minute: 5,
    }
}

#[test]
fn weekdays_are_computed_from_the_date() {
    assert_eq!(weekday_name(2026, 11, 4), "Wednesday");
    assert_eq!(weekday_name(2026, 1, 1), "Thursday");
    assert_eq!(weekday_name(2024, 2, 29), "Thursday");
    assert_eq!(weekday_name(2000, 1, 1), "Saturday");
    assert_eq!(weekday_name(2023, 12, 31), "Sunday");
    assert_eq!(weekday_name(2021, 3, 1), "Monday");
}

#[test]
fn the_prompt_states_the_local_time_and_the_reply_language() {
    let en = system_prompt(Lang::En, Some(&now()));
    assert!(en.contains("Current local time: Wednesday 2026-11-04 14:05."));
    assert!(en.contains("reply in English"));
    assert!(!en.contains("Spanish"));
    let es = system_prompt(Lang::Es, Some(&now()));
    assert!(es.contains("reply in Spanish"));
    assert!(!es.contains("English"));
}

#[test]
fn without_a_clock_the_time_line_is_left_out() {
    let prompt = system_prompt(Lang::En, None);
    assert!(!prompt.contains("Current local time"));
    assert!(prompt.starts_with("You are Niva"));
}

#[test]
fn the_time_is_zero_padded() {
    let early = LocalDateTime {
        year: 2026,
        month: 1,
        day: 2,
        hour: 0,
        minute: 7,
    };
    assert!(system_prompt(Lang::En, Some(&early)).contains("Friday 2026-01-02 00:07."));
}

#[test]
fn the_prompt_carries_the_e1_rules_that_keep_risky_calls_out() {
    let prompt = system_prompt(Lang::En, None);
    for rule in [
        "Only do what the user asked.",
        "Never lock, sleep, restart or shut down the PC unless the user explicitly asked for it.",
        "Never close an app unless the user explicitly asked for it.",
        "negates them, or asks how to do them, do not call a tool.",
        "you do not need to ask first.",
        "For chit-chat, answer briefly without tools.",
    ] {
        assert!(prompt.contains(rule), "missing rule: {rule}");
    }
}

#[test]
fn small_talk_about_sleeping_is_not_a_power_request() {
    // E1 attempt 3: the model turned "I'm going to bed" / "me voy a dormir"
    // into power_action sleep (9 of 16 held-out phrasings, every run).
    let prompt = system_prompt(Lang::En, None);
    assert!(
        prompt.contains("goodnight, being tired or going to bed is only conversation"),
        "{prompt}"
    );
}

#[test]
fn set_volume_says_that_silencing_the_computer_is_a_mute() {
    // Batch 38, the first real run: "Silenciá la compu" came back from Whisper
    // as a phrase with "la computadora" and the model took "the computer" for
    // the PC of power_action (lock or sleep). Of 19 spoken ways to say "mute
    // the computer" (compu, computadora, PC, muteá, callá, en silencio), 7 went
    // to power_action before this sentence in the set_volume description and
    // 0 after it.
    let volume = spec("set_volume");
    assert!(
        volume.description.ends_with(
            "Also use it to mute or silence the computer's sound ('silenciá la compu', 'mute the PC')."
        ),
        "{}",
        volume.description
    );
    // The power tool keeps its E1 wording: the fix lives in the other tool.
    assert!(
        !spec("power_action").description.contains("set_volume"),
        "{}",
        spec("power_action").description
    );
}

#[test]
fn power_action_is_only_for_an_explicit_command_about_the_pc() {
    // E1 attempt 4: the held-out phrasings ("I'm done for today", "me voy a
    // la cama") still produced power_action sleep after the prompt line of
    // attempt 3, so the tool itself says when it applies.
    let power = spec("power_action");
    assert!(
        power.description.starts_with(
            "Lock, sleep, restart or shut down the PC. Asks the user for confirmation."
        )
    );
    assert!(
        power.description.contains("explicit command"),
        "{}",
        power.description
    );
    assert!(power.description.contains("never"), "{}", power.description);
    assert!(
        power.description.contains("going to bed"),
        "{}",
        power.description
    );
}

#[test]
fn close_app_asks_for_the_short_name_the_user_said() {
    // E1 attempt 3: "Quit Chrome" became close_app("Google Chrome"), which no
    // process stem contains, so the real path answered "no running app".
    let close = spec("close_app");
    assert!(
        close.description.starts_with(
            "Close a running application by its name. Asks the user for confirmation."
        )
    );
    assert!(
        close.description.contains("short name"),
        "{}",
        close.description
    );
    assert!(close.description.contains("chrome"));
}

#[test]
fn the_prompt_asks_for_short_replies_and_states_the_bound() {
    // Task 2.4a: the soft half of "Replies are short" (the hard cap is in
    // `replies::bound_reply`).
    let prompt = system_prompt(Lang::En, None);
    assert!(prompt.contains("one short sentence"));
    assert!(prompt.contains("300 characters"));
    const { assert!(300 < crate::replies::REPLY_MAX_CHARS) };
}

#[test]
fn the_prompt_is_one_message_without_markup_the_model_could_echo() {
    let prompt = system_prompt(Lang::Es, Some(&now()));
    assert!(!prompt.contains("```"));
    assert!(!prompt.contains('<'));
    assert!(prompt.lines().count() >= 6);
}
