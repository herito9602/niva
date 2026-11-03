//! Registry classification and per-tool argument validation. Specs:
//! `action-safety` (risk classification), `pc-actions` (volume clamp).

use super::*;

fn s(text: &str) -> ArgValue {
    ArgValue::Str(text.into())
}

fn call(name: &str, args: &[(&str, ArgValue)]) -> ToolCall {
    ToolCall {
        name: name.into(),
        args: args
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect(),
    }
}

fn parse(name: &str, args: &[(&str, ArgValue)]) -> Result<ParsedCall, ToolError> {
    parse_call(&call(name, args))
}

fn safe(action: SafeAction) -> Result<ParsedCall, ToolError> {
    Ok(ParsedCall::Safe(action))
}

fn invalid(name: &'static str, problem: ArgProblem) -> Result<ParsedCall, ToolError> {
    Err(ToolError::InvalidArgument { name, problem })
}

// Registry

#[test]
fn close_app_and_power_action_are_risky() {
    assert_eq!(ToolName::CloseApp.risk(), Risk::Risky);
    assert_eq!(ToolName::PowerAction.risk(), Risk::Risky);
}

#[test]
fn every_other_tool_is_safe() {
    for tool in ToolName::ALL {
        if !matches!(tool, ToolName::CloseApp | ToolName::PowerAction) {
            assert_eq!(tool.risk(), Risk::Safe, "{}", tool.as_str());
        }
    }
}

#[test]
fn the_registry_has_the_seven_spec_tools_with_unique_wire_names() {
    let names: Vec<&str> = ToolName::ALL.iter().map(|t| t.as_str()).collect();
    assert_eq!(
        names,
        [
            "open_app",
            "close_app",
            "set_volume",
            "create_reminder",
            "list_reminders",
            "web_search",
            "power_action"
        ]
    );
    for tool in ToolName::ALL {
        assert_eq!(ToolName::from_name(tool.as_str()), Some(tool));
    }
}

#[test]
fn tool_lookup_is_exact() {
    assert_eq!(ToolName::from_name("Close_App"), None);
    assert_eq!(ToolName::from_name("shutdown"), None);
    assert_eq!(ToolName::from_name(""), None);
}

#[test]
fn an_unknown_tool_is_an_error() {
    assert_eq!(
        parse("format_disk", &[]),
        Err(ToolError::UnknownTool("format_disk".into()))
    );
}

#[test]
fn power_restart_and_shutdown_are_click_only_lock_and_sleep_are_not() {
    assert!(PowerKind::Restart.click_only());
    assert!(PowerKind::Shutdown.click_only());
    assert!(!PowerKind::Lock.click_only());
    assert!(!PowerKind::Sleep.click_only());
}

// Unknown arguments (additionalProperties: false)

#[test]
fn an_argument_outside_the_schema_is_rejected() {
    assert_eq!(
        parse(
            "open_app",
            &[("name", s("notepad")), ("confirmed", ArgValue::Bool(true))]
        ),
        Err(ToolError::UnknownArgument("confirmed".into()))
    );
    assert_eq!(
        parse("list_reminders", &[("limit", ArgValue::Int(3))]),
        Err(ToolError::UnknownArgument("limit".into()))
    );
}

// open_app, web_search

#[test]
fn open_app_takes_a_name() {
    assert_eq!(
        parse("open_app", &[("name", s("  Notepad "))]),
        safe(SafeAction::OpenApp {
            name: "Notepad".into()
        })
    );
}

#[test]
fn open_app_without_a_name_or_with_a_blank_one_is_invalid() {
    assert_eq!(
        parse("open_app", &[]),
        Err(ToolError::Missing(vec!["name"]))
    );
    assert_eq!(
        parse("open_app", &[("name", ArgValue::Null)]),
        Err(ToolError::Missing(vec!["name"]))
    );
    assert_eq!(
        parse("open_app", &[("name", s("   "))]),
        invalid("name", ArgProblem::Empty)
    );
    assert_eq!(
        parse("open_app", &[("name", ArgValue::Int(4))]),
        invalid("name", ArgProblem::WrongType)
    );
}

#[test]
fn web_search_takes_a_query() {
    assert_eq!(
        parse("web_search", &[("query", s("weather in Lima"))]),
        safe(SafeAction::WebSearch {
            query: "weather in Lima".into()
        })
    );
    assert_eq!(
        parse("web_search", &[]),
        Err(ToolError::Missing(vec!["query"]))
    );
}

// set_volume

#[test]
fn set_volume_sets_a_level() {
    assert_eq!(
        parse("set_volume", &[("level", ArgValue::Int(30))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Set(30),
            clamped: false
        })
    );
}

#[test]
fn a_level_above_100_is_clamped_and_reported() {
    assert_eq!(
        parse("set_volume", &[("level", ArgValue::Int(150))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Set(100),
            clamped: true
        })
    );
}

#[test]
fn a_negative_level_is_clamped_to_zero() {
    assert_eq!(
        parse("set_volume", &[("level", ArgValue::Int(-5))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Set(0),
            clamped: true
        })
    );
}

#[test]
fn level_100_and_0_are_not_clamped() {
    for level in [0, 100] {
        assert_eq!(
            parse("set_volume", &[("level", ArgValue::Int(level))]),
            safe(SafeAction::SetVolume {
                change: VolumeChange::Set(level as u8),
                clamped: false
            })
        );
    }
}

#[test]
fn set_volume_supports_delta_and_mute() {
    assert_eq!(
        parse("set_volume", &[("delta", ArgValue::Int(-20))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Delta(-20),
            clamped: false
        })
    );
    assert_eq!(
        parse("set_volume", &[("mute", ArgValue::Bool(true))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Mute(true),
            clamped: false
        })
    );
}

#[test]
fn a_delta_outside_the_schema_range_is_clamped_and_reported() {
    assert_eq!(
        parse("set_volume", &[("delta", ArgValue::Int(150))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Delta(100),
            clamped: true
        })
    );
    assert_eq!(
        parse("set_volume", &[("delta", ArgValue::Int(-101))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Delta(-100),
            clamped: true
        })
    );
    assert_eq!(
        parse("set_volume", &[("delta", ArgValue::Int(i64::MIN))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Delta(-100),
            clamped: true
        })
    );
    assert_eq!(
        parse("set_volume", &[("delta", ArgValue::Int(100))]),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Delta(100),
            clamped: false
        })
    );
}

#[test]
fn set_volume_takes_exactly_one_of_level_delta_mute() {
    assert_eq!(
        parse("set_volume", &[]),
        Err(ToolError::Missing(vec!["level", "delta", "mute"]))
    );
    assert_eq!(
        parse(
            "set_volume",
            &[
                ("level", ArgValue::Int(30)),
                ("mute", ArgValue::Bool(false))
            ]
        ),
        Err(ToolError::Conflicting(vec!["level", "mute"]))
    );
}

#[test]
fn null_means_absent_so_the_spike_schema_shape_still_works() {
    assert_eq!(
        parse(
            "set_volume",
            &[("level", ArgValue::Int(20)), ("mute", ArgValue::Null)]
        ),
        safe(SafeAction::SetVolume {
            change: VolumeChange::Set(20),
            clamped: false
        })
    );
}

#[test]
fn set_volume_rejects_wrong_types() {
    assert_eq!(
        parse("set_volume", &[("level", s("30"))]),
        invalid("level", ArgProblem::WrongType)
    );
    assert_eq!(
        parse("set_volume", &[("level", ArgValue::Float(30.5))]),
        invalid("level", ArgProblem::WrongType)
    );
    assert_eq!(
        parse("set_volume", &[("mute", ArgValue::Int(1))]),
        invalid("mute", ArgProblem::WrongType)
    );
}

// power_action, close_app

#[test]
fn power_action_accepts_only_the_four_actions() {
    for kind in PowerKind::ALL {
        assert_eq!(
            parse("power_action", &[("action", s(kind.as_str()))]),
            Ok(ParsedCall::Power(kind))
        );
    }
    assert_eq!(
        parse("power_action", &[("action", s("hibernate"))]),
        invalid("action", ArgProblem::NotOneOf)
    );
    assert_eq!(
        parse("power_action", &[("action", s("Shutdown"))]),
        invalid("action", ArgProblem::NotOneOf)
    );
    assert_eq!(
        parse("power_action", &[]),
        Err(ToolError::Missing(vec!["action"]))
    );
}

#[test]
fn close_app_parses_to_an_unresolved_query() {
    assert_eq!(
        parse("close_app", &[("name", s(" Spotify "))]),
        Ok(ParsedCall::Close {
            query: "Spotify".into()
        })
    );
    assert_eq!(
        parse("close_app", &[]),
        Err(ToolError::Missing(vec!["name"]))
    );
}

// create_reminder, list_reminders

#[test]
fn list_reminders_takes_no_arguments() {
    assert_eq!(
        parse("list_reminders", &[]),
        safe(SafeAction::ListReminders)
    );
}

#[test]
fn a_reminder_with_a_delay() {
    assert_eq!(
        parse(
            "create_reminder",
            &[("text", s("tea")), ("delay_minutes", ArgValue::Int(10))]
        ),
        safe(SafeAction::CreateReminder {
            text: "tea".into(),
            when: ReminderTime::InMinutes(10)
        })
    );
}

#[test]
fn a_non_positive_delay_is_rejected() {
    for delay in [0, -5] {
        assert_eq!(
            parse(
                "create_reminder",
                &[("text", s("tea")), ("delay_minutes", ArgValue::Int(delay))]
            ),
            invalid("delay_minutes", ArgProblem::NotPositive)
        );
    }
}

#[test]
fn a_reminder_at_a_local_time_with_an_optional_date() {
    assert_eq!(
        parse(
            "create_reminder",
            &[("text", s("call")), ("at_local_time", s("09:05"))]
        ),
        safe(SafeAction::CreateReminder {
            text: "call".into(),
            when: ReminderTime::AtLocal {
                hour: 9,
                minute: 5,
                date: None
            }
        })
    );
    assert_eq!(
        parse(
            "create_reminder",
            &[
                ("text", s("call")),
                ("at_local_time", s("23:59")),
                ("date", s("2028-02-29"))
            ]
        ),
        safe(SafeAction::CreateReminder {
            text: "call".into(),
            when: ReminderTime::AtLocal {
                hour: 23,
                minute: 59,
                date: Some(CalendarDate {
                    year: 2028,
                    month: 2,
                    day: 29
                })
            }
        })
    );
}

#[test]
fn malformed_times_and_dates_are_rejected() {
    for bad in [
        "25:00", "24", "24:00", "09:60", "18:60", "ab:cd", "09:5", "09-05", "9:", ":30", "1:2:3",
        "09:00:60", "09:00:5", "009", "9.30",
    ] {
        assert_eq!(
            parse(
                "create_reminder",
                &[("text", s("x")), ("at_local_time", s(bad))]
            ),
            invalid("at_local_time", ArgProblem::BadFormat),
            "{bad}"
        );
    }
    for bad in [
        "2027-02-29",
        "2027-13-01",
        "2027-00-10",
        "27-01-01",
        "2027/01/01",
    ] {
        assert_eq!(
            parse(
                "create_reminder",
                &[
                    ("text", s("x")),
                    ("at_local_time", s("09:00")),
                    ("date", s(bad))
                ]
            ),
            invalid("date", ArgProblem::BadFormat),
            "{bad}"
        );
    }
}

#[test]
fn a_reminder_without_a_time_reports_what_is_missing() {
    assert_eq!(
        parse("create_reminder", &[("text", s("tea"))]),
        Err(ToolError::Missing(vec!["time"]))
    );
    assert_eq!(
        parse("create_reminder", &[("at_local_time", s("09:00"))]),
        Err(ToolError::Missing(vec!["text"]))
    );
    assert_eq!(
        parse("create_reminder", &[]),
        Err(ToolError::Missing(vec!["text", "time"]))
    );
}

#[test]
fn a_date_without_a_time_is_missing_the_time() {
    assert_eq!(
        parse(
            "create_reminder",
            &[("text", s("x")), ("date", s("2028-01-01"))]
        ),
        Err(ToolError::Missing(vec!["time"]))
    );
}

#[test]
fn a_delay_and_a_local_time_together_conflict() {
    assert_eq!(
        parse(
            "create_reminder",
            &[
                ("text", s("x")),
                ("delay_minutes", ArgValue::Int(5)),
                ("at_local_time", s("09:00"))
            ]
        ),
        Err(ToolError::Conflicting(vec![
            "delay_minutes",
            "at_local_time"
        ]))
    );
    assert_eq!(
        parse(
            "create_reminder",
            &[
                ("text", s("x")),
                ("delay_minutes", ArgValue::Int(5)),
                ("date", s("2028-01-01"))
            ]
        ),
        Err(ToolError::Conflicting(vec!["delay_minutes", "date"]))
    );
}

fn reminder_at(at: &str) -> Result<ParsedCall, ToolError> {
    parse(
        "create_reminder",
        &[("text", s("call")), ("at_local_time", s(at))],
    )
}

fn at_local(hour: u8, minute: u8) -> Result<ParsedCall, ToolError> {
    safe(SafeAction::CreateReminder {
        text: "call".into(),
        when: ReminderTime::AtLocal {
            hour,
            minute,
            date: None,
        },
    })
}

#[test]
fn local_times_accept_the_formats_the_eval_scores_as_correct() {
    // eval/commands.yaml accepts "21:00", "21:00:00" and "21" for 9 pm.
    assert_eq!(reminder_at("18"), at_local(18, 0));
    assert_eq!(reminder_at("9"), at_local(9, 0));
    assert_eq!(reminder_at("09"), at_local(9, 0));
    assert_eq!(reminder_at("0"), at_local(0, 0));
    assert_eq!(reminder_at("23"), at_local(23, 0));
    assert_eq!(reminder_at("9:05"), at_local(9, 5));
    assert_eq!(reminder_at("09:05"), at_local(9, 5));
    assert_eq!(reminder_at("18:00:00"), at_local(18, 0));
    assert_eq!(reminder_at("18:30:45"), at_local(18, 30));
    assert_eq!(reminder_at(" 18:00 "), at_local(18, 0));
}

#[test]
fn every_tool_risk_matches_what_parse_call_produces() {
    // One source of truth: Risky tools parse to a pending risky call, Safe
    // tools to a ready safe action.
    let minimal = |tool: ToolName| match tool {
        ToolName::OpenApp | ToolName::CloseApp => call(tool.as_str(), &[("name", s("abc"))]),
        ToolName::SetVolume => call(tool.as_str(), &[("mute", ArgValue::Bool(true))]),
        ToolName::CreateReminder => call(
            tool.as_str(),
            &[("text", s("x")), ("delay_minutes", ArgValue::Int(5))],
        ),
        ToolName::ListReminders => call(tool.as_str(), &[]),
        ToolName::WebSearch => call(tool.as_str(), &[("query", s("q"))]),
        ToolName::PowerAction => call(tool.as_str(), &[("action", s("lock"))]),
    };
    for tool in ToolName::ALL {
        let parsed = parse_call(&minimal(tool)).unwrap_or_else(|e| panic!("{tool:?}: {e:?}"));
        let is_safe = matches!(parsed, ParsedCall::Safe(_));
        assert_eq!(is_safe, tool.risk() == Risk::Safe, "{tool:?}");
    }
}

#[test]
fn a_delay_above_seven_days_is_rejected() {
    let delay = |minutes: i64| {
        parse(
            "create_reminder",
            &[
                ("text", s("tea")),
                ("delay_minutes", ArgValue::Int(minutes)),
            ],
        )
    };
    assert_eq!(
        delay(10_080),
        safe(SafeAction::CreateReminder {
            text: "tea".into(),
            when: ReminderTime::InMinutes(10_080)
        })
    );
    assert_eq!(
        delay(10_081),
        invalid("delay_minutes", ArgProblem::OutOfRange)
    );
    assert_eq!(
        delay(i64::MAX),
        invalid("delay_minutes", ArgProblem::OutOfRange)
    );
}

#[test]
fn free_text_is_capped_at_200_characters() {
    let ok = "ñ".repeat(200);
    let long = "ñ".repeat(201);
    assert_eq!(
        parse("open_app", &[("name", s(&ok))]),
        safe(SafeAction::OpenApp { name: ok.clone() })
    );
    for (tool, key) in [
        ("open_app", "name"),
        ("close_app", "name"),
        ("web_search", "query"),
    ] {
        assert_eq!(
            parse(tool, &[(key, s(&long))]),
            invalid(key_static(key), ArgProblem::TooLong),
            "{tool}"
        );
    }
    assert_eq!(
        parse(
            "create_reminder",
            &[("text", s(&long)), ("delay_minutes", ArgValue::Int(5))]
        ),
        invalid("text", ArgProblem::TooLong)
    );
}

fn key_static(key: &str) -> &'static str {
    match key {
        "name" => "name",
        "query" => "query",
        other => panic!("unexpected key {other}"),
    }
}
