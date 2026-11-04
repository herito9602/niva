//! The arg matchers of `eval/commands.yaml`, ported from the E1 spike
//! (`spikes/eval/run_e1.py`, `arg_matches`/`call_matches`/`judge_tools`) with
//! the same semantics, so a number measured here compares with E1's:
//!
//! - scalar: equality; numbers are coerced (`"50"`, `50.0` and `"50%"` equal
//!   50), booleans accept `"true"`/`"false"`, strings compare trimmed and
//!   case-insensitively;
//! - `contains` (all) / `contains_any` (one): case-insensitive substrings of
//!   the argument's text form;
//! - `one_of`: the trimmed, lowercased text is one of the values;
//! - `sign`: a non-zero number with that sign; `range`: inclusive;
//! - `absent`: the argument is missing, null or an empty string;
//! - arguments the case does not list are not checked; expected calls match
//!   in any order, one call each, and any call left over fails the case.

use niva_core::tools::{ArgValue, ToolCall};

#[derive(Debug, Clone, PartialEq)]
pub enum Scalar {
    Bool(bool),
    Num(f64),
    Str(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sign {
    Pos,
    Neg,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Matcher {
    Absent,
    Contains(Vec<String>),
    ContainsAny(Vec<String>),
    OneOf(Vec<Scalar>),
    Sign(Sign),
    Range(f64, f64),
    Equals(Scalar),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExpectedCall {
    pub tool: String,
    pub args: Vec<(String, Matcher)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expected {
    /// `expected: none`: no tool call allowed.
    NoTools,
    Calls(Vec<ExpectedCall>),
}

/// The argument as Python's `str()` would show it, lowercased by the callers.
fn text_of(value: &ArgValue) -> String {
    match value {
        ArgValue::Str(s) => s.clone(),
        ArgValue::Int(i) => i.to_string(),
        ArgValue::Float(f) => f.to_string(),
        ArgValue::Bool(b) => b.to_string(),
        ArgValue::Null => String::new(),
    }
}

/// `_num` of the spike: a bool is not a number; a string may carry a `%`.
fn number_of(value: &ArgValue) -> Option<f64> {
    match value {
        ArgValue::Int(i) => Some(*i as f64),
        ArgValue::Float(f) => Some(*f),
        ArgValue::Str(s) => s.trim().replace('%', "").parse().ok(),
        ArgValue::Bool(_) | ArgValue::Null => None,
    }
}

fn bool_of(value: &ArgValue) -> Option<bool> {
    match value {
        ArgValue::Bool(b) => Some(*b),
        ArgValue::Str(s) => match s.trim().to_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn scalar_text(scalar: &Scalar) -> String {
    match scalar {
        Scalar::Bool(b) => b.to_string(),
        Scalar::Num(n) if n.fract() == 0.0 && n.abs() < 1e15 => (*n as i64).to_string(),
        Scalar::Num(n) => n.to_string(),
        Scalar::Str(s) => s.clone(),
    }
}

/// `actual` is `None` when the model did not send the argument.
pub fn arg_matches(matcher: &Matcher, actual: Option<&ArgValue>) -> bool {
    let present = actual.filter(|value| **value != ArgValue::Null);
    if *matcher == Matcher::Absent {
        return present.is_none_or(|value| matches!(value, ArgValue::Str(s) if s.is_empty()));
    }
    let Some(value) = present else {
        return false;
    };
    let text = text_of(value).to_lowercase();
    match matcher {
        Matcher::Absent => unreachable!("handled above"),
        Matcher::Contains(needles) => needles.iter().all(|n| text.contains(&n.to_lowercase())),
        Matcher::ContainsAny(needles) => needles.iter().any(|n| text.contains(&n.to_lowercase())),
        Matcher::OneOf(values) => values
            .iter()
            .any(|v| scalar_text(v).to_lowercase() == text.trim()),
        Matcher::Sign(sign) => {
            number_of(value).is_some_and(|n| n != 0.0 && (n > 0.0) == (*sign == Sign::Pos))
        }
        Matcher::Range(lo, hi) => number_of(value).is_some_and(|n| *lo <= n && n <= *hi),
        Matcher::Equals(Scalar::Bool(want)) => bool_of(value) == Some(*want),
        Matcher::Equals(Scalar::Num(want)) => number_of(value).is_some_and(|n| n == *want),
        Matcher::Equals(Scalar::Str(want)) => text.trim() == want.trim().to_lowercase(),
    }
}

pub fn call_matches(expected: &ExpectedCall, call: &ToolCall) -> bool {
    expected.tool == call.name
        && expected
            .args
            .iter()
            .all(|(key, matcher)| arg_matches(matcher, call.args.get(key)))
}

#[derive(Debug, Clone, PartialEq)]
pub struct Judgement {
    pub passed: bool,
    /// Calls no expectation took: for a `none` case all of them.
    pub unmatched: Vec<ToolCall>,
}

pub fn judge_tools(expected: &Expected, calls: &[ToolCall]) -> Judgement {
    let wanted = match expected {
        Expected::NoTools => {
            return Judgement {
                passed: calls.is_empty(),
                unmatched: calls.to_vec(),
            };
        }
        Expected::Calls(wanted) => wanted,
    };
    let mut remaining: Vec<ToolCall> = calls.to_vec();
    let mut all_found = true;
    for want in wanted {
        match remaining.iter().position(|call| call_matches(want, call)) {
            Some(at) => {
                remaining.remove(at);
            }
            None => all_found = false,
        }
    }
    Judgement {
        passed: all_found && remaining.is_empty(),
        unmatched: remaining,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> ArgValue {
        ArgValue::Str(v.into())
    }

    fn call(name: &str, args: &[(&str, ArgValue)]) -> ToolCall {
        ToolCall {
            name: name.into(),
            args: args
                .iter()
                .map(|(k, v)| (k.to_string(), v.clone()))
                .collect(),
        }
    }

    fn expect(tool: &str, args: &[(&str, Matcher)]) -> ExpectedCall {
        ExpectedCall {
            tool: tool.into(),
            args: args
                .iter()
                .map(|(k, m)| (k.to_string(), m.clone()))
                .collect(),
        }
    }

    fn strs(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // --- scalars: equality with coercion ----------------------------------------

    #[test]
    fn numbers_are_coerced_from_ints_floats_and_strings() {
        let m = Matcher::Equals(Scalar::Num(50.0));
        assert!(arg_matches(&m, Some(&ArgValue::Int(50))));
        assert!(arg_matches(&m, Some(&ArgValue::Float(50.0))));
        assert!(arg_matches(&m, Some(&s("50"))));
        assert!(arg_matches(&m, Some(&s(" 50 "))));
        assert!(arg_matches(&m, Some(&s("50%"))));
        assert!(!arg_matches(&m, Some(&ArgValue::Int(51))));
        assert!(!arg_matches(&m, Some(&s("fifty"))));
        assert!(
            !arg_matches(&m, Some(&ArgValue::Bool(true))),
            "a bool is not a number"
        );
        assert!(!arg_matches(&m, None));
        assert!(!arg_matches(&m, Some(&ArgValue::Null)));
    }

    #[test]
    fn negative_numbers_match_exactly() {
        let m = Matcher::Equals(Scalar::Num(-20.0));
        assert!(arg_matches(&m, Some(&ArgValue::Int(-20))));
        assert!(!arg_matches(&m, Some(&ArgValue::Int(20))));
    }

    #[test]
    fn booleans_accept_true_false_and_their_string_forms() {
        let yes = Matcher::Equals(Scalar::Bool(true));
        let no = Matcher::Equals(Scalar::Bool(false));
        assert!(arg_matches(&yes, Some(&ArgValue::Bool(true))));
        assert!(arg_matches(&yes, Some(&s("true"))));
        assert!(arg_matches(&yes, Some(&s(" TRUE "))));
        assert!(arg_matches(&no, Some(&ArgValue::Bool(false))));
        assert!(arg_matches(&no, Some(&s("false"))));
        assert!(!arg_matches(&yes, Some(&ArgValue::Bool(false))));
        assert!(!arg_matches(&no, Some(&ArgValue::Bool(true))));
        assert!(!arg_matches(&yes, Some(&s("yes"))));
        assert!(!arg_matches(&yes, Some(&ArgValue::Int(1))));
        assert!(!arg_matches(&no, None), "false is not 'missing'");
    }

    #[test]
    fn strings_compare_trimmed_and_case_insensitive() {
        let m = Matcher::Equals(Scalar::Str("Lock".into()));
        assert!(arg_matches(&m, Some(&s("lock"))));
        assert!(arg_matches(&m, Some(&s("  LOCK "))));
        assert!(!arg_matches(&m, Some(&s("lock now"))));
        assert!(!arg_matches(&m, None));
    }

    // --- dict matchers ----------------------------------------------------------

    #[test]
    fn contains_needs_every_substring_case_insensitively() {
        let m = Matcher::Contains(strs(&["capital", "Australia"]));
        assert!(arg_matches(
            &m,
            Some(&s("What is the CAPITAL of australia?"))
        ));
        assert!(!arg_matches(&m, Some(&s("capital of France"))));
        assert!(!arg_matches(&m, None));
    }

    #[test]
    fn contains_any_needs_one_substring() {
        let m = Matcher::ContainsAny(strs(&["calculadora", "calculator"]));
        assert!(arg_matches(&m, Some(&s("Calculator"))));
        assert!(arg_matches(&m, Some(&s("la calculadora"))));
        assert!(!arg_matches(&m, Some(&s("notepad"))));
    }

    #[test]
    fn contains_looks_at_the_text_form_of_numbers_and_bools() {
        assert!(arg_matches(
            &Matcher::Contains(strs(&["2022"])),
            Some(&ArgValue::Int(2022))
        ));
        assert!(arg_matches(
            &Matcher::Contains(strs(&["true"])),
            Some(&ArgValue::Bool(true))
        ));
    }

    #[test]
    fn one_of_is_exact_after_trimming_and_lowercasing() {
        let m = Matcher::OneOf(vec![
            Scalar::Str("21:00".into()),
            Scalar::Str("21:00:00".into()),
            Scalar::Str("21".into()),
        ]);
        assert!(arg_matches(&m, Some(&s("21:00"))));
        assert!(arg_matches(&m, Some(&s(" 21:00:00 "))));
        assert!(arg_matches(&m, Some(&ArgValue::Int(21))));
        assert!(!arg_matches(&m, Some(&s("9:00 pm"))));
        assert!(!arg_matches(&m, Some(&s("21:00 and more"))));
    }

    #[test]
    fn sign_needs_a_non_zero_number_with_that_sign() {
        let pos = Matcher::Sign(Sign::Pos);
        let neg = Matcher::Sign(Sign::Neg);
        assert!(arg_matches(&pos, Some(&ArgValue::Int(10))));
        assert!(arg_matches(&pos, Some(&s("5"))));
        assert!(arg_matches(&neg, Some(&ArgValue::Int(-10))));
        assert!(arg_matches(&neg, Some(&ArgValue::Float(-0.5))));
        assert!(
            !arg_matches(&pos, Some(&ArgValue::Int(0))),
            "zero has no sign"
        );
        assert!(!arg_matches(&neg, Some(&ArgValue::Int(0))));
        assert!(!arg_matches(&pos, Some(&ArgValue::Int(-3))));
        assert!(!arg_matches(&neg, Some(&ArgValue::Int(3))));
        assert!(!arg_matches(&pos, Some(&s("up"))));
        assert!(!arg_matches(&pos, Some(&ArgValue::Bool(true))));
        assert!(!arg_matches(&pos, None));
    }

    #[test]
    fn range_is_inclusive() {
        let m = Matcher::Range(10.0, 20.0);
        assert!(arg_matches(&m, Some(&ArgValue::Int(10))));
        assert!(arg_matches(&m, Some(&ArgValue::Int(20))));
        assert!(arg_matches(&m, Some(&s("15"))));
        assert!(!arg_matches(&m, Some(&ArgValue::Int(9))));
        assert!(!arg_matches(&m, Some(&ArgValue::Int(21))));
        assert!(!arg_matches(&m, Some(&s("many"))));
    }

    #[test]
    fn absent_means_missing_null_or_empty() {
        let m = Matcher::Absent;
        assert!(arg_matches(&m, None));
        assert!(arg_matches(&m, Some(&ArgValue::Null)));
        assert!(arg_matches(&m, Some(&s(""))));
        assert!(!arg_matches(&m, Some(&s("21:00"))));
        assert!(!arg_matches(&m, Some(&ArgValue::Int(0))), "0 was invented");
        assert!(!arg_matches(&m, Some(&ArgValue::Bool(false))));
    }

    // --- calls ----------------------------------------------------------------

    #[test]
    fn a_call_matches_by_tool_and_by_every_listed_argument() {
        let want = expect(
            "set_volume",
            &[("level", Matcher::Equals(Scalar::Num(30.0)))],
        );
        assert!(call_matches(
            &want,
            &call("set_volume", &[("level", ArgValue::Int(30))])
        ));
        assert!(!call_matches(
            &want,
            &call("set_volume", &[("level", ArgValue::Int(40))])
        ));
        assert!(!call_matches(
            &want,
            &call("open_app", &[("level", ArgValue::Int(30))])
        ));
        assert!(!call_matches(&want, &call("set_volume", &[])));
    }

    #[test]
    fn arguments_the_case_does_not_list_are_not_checked() {
        let want = expect(
            "set_volume",
            &[("level", Matcher::Equals(Scalar::Num(30.0)))],
        );
        let extra = call(
            "set_volume",
            &[
                ("level", ArgValue::Int(30)),
                ("mute", ArgValue::Bool(false)),
            ],
        );
        // E1 scored this as a pass; the real validator rejects it (Conflicting).
        // The runner reports that separately as `plan_rejected`.
        assert!(call_matches(&want, &extra));
    }

    #[test]
    fn an_expected_call_without_arguments_matches_any_arguments() {
        let want = expect("list_reminders", &[]);
        assert!(call_matches(&want, &call("list_reminders", &[])));
    }

    #[test]
    fn an_absent_expectation_on_a_missing_argument_passes() {
        let want = expect("create_reminder", &[("at_local_time", Matcher::Absent)]);
        assert!(call_matches(
            &want,
            &call("create_reminder", &[("text", s("x"))])
        ));
        assert!(!call_matches(
            &want,
            &call("create_reminder", &[("at_local_time", s("09:00"))])
        ));
    }

    // --- judging a response -----------------------------------------------------

    #[test]
    fn none_passes_only_with_zero_calls() {
        let passed = judge_tools(&Expected::NoTools, &[]);
        assert!(passed.passed && passed.unmatched.is_empty());
        let calls = [call("power_action", &[("action", s("sleep"))])];
        let failed = judge_tools(&Expected::NoTools, &calls);
        assert!(!failed.passed);
        assert_eq!(failed.unmatched, calls);
    }

    #[test]
    fn expected_calls_match_in_any_order() {
        let want = Expected::Calls(vec![
            expect(
                "open_app",
                &[("name", Matcher::Contains(strs(&["spotify"])))],
            ),
            expect(
                "set_volume",
                &[("level", Matcher::Equals(Scalar::Num(30.0)))],
            ),
        ]);
        let calls = [
            call("set_volume", &[("level", ArgValue::Int(30))]),
            call("open_app", &[("name", s("Spotify"))]),
        ];
        let judged = judge_tools(&want, &calls);
        assert!(judged.passed, "{judged:?}");
        assert!(judged.unmatched.is_empty());
    }

    #[test]
    fn an_extra_call_fails_the_case_and_is_reported_as_unmatched() {
        let want = Expected::Calls(vec![expect(
            "set_volume",
            &[("delta", Matcher::Sign(Sign::Neg))],
        )]);
        let calls = [
            call("set_volume", &[("delta", ArgValue::Int(-10))]),
            call("close_app", &[("name", s("spotify"))]),
        ];
        let judged = judge_tools(&want, &calls);
        assert!(!judged.passed);
        assert_eq!(judged.unmatched, [calls[1].clone()]);
    }

    #[test]
    fn a_missing_call_fails_the_case() {
        let want = Expected::Calls(vec![expect("open_app", &[]), expect("set_volume", &[])]);
        let judged = judge_tools(&want, &[call("open_app", &[("name", s("x"))])]);
        assert!(!judged.passed);
        assert!(judged.unmatched.is_empty());
    }

    #[test]
    fn one_call_cannot_satisfy_two_expectations() {
        let want = Expected::Calls(vec![
            expect(
                "open_app",
                &[("name", Matcher::Contains(strs(&["chrome"])))],
            ),
            expect(
                "open_app",
                &[("name", Matcher::Contains(strs(&["chrome"])))],
            ),
        ]);
        let judged = judge_tools(&want, &[call("open_app", &[("name", s("chrome"))])]);
        assert!(!judged.passed);
    }

    #[test]
    fn a_risky_call_that_nobody_expected_is_unmatched() {
        let calls = [call("power_action", &[("action", s("sleep"))])];
        let judged = judge_tools(&Expected::NoTools, &calls);
        assert_eq!(judged.unmatched[0].name, "power_action");
    }
}
