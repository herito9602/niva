//! `eval/commands.yaml` (E1 cases) into typed cases. The file format is
//! documented at the top of that file; the matcher semantics live in
//! [`matcher`](super::matcher). Parsing is strict: an unknown matcher, a
//! malformed range or a tool or argument the schemas do not have is an error
//! that names the case, so a typo cannot silently make a case pass.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use niva_core::prompt::TOOL_SPECS;
use niva_core::replies::Lang;
use serde::Deserialize;
use serde_norway::Value;

use super::matcher::{Expected, ExpectedCall, Matcher, Scalar, Sign};

/// Categories the runner knows (the file's header lists them).
const CATEGORIES: &[&str] = &[
    "open_app",
    "close_app",
    "volume",
    "reminder",
    "reminder_followup",
    "list_reminders",
    "web_search",
    "power",
    "multi",
    "chat",
    "adversarial",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Case {
    pub id: String,
    pub category: String,
    pub lang: Lang,
    /// The right answer involves a risky tool.
    pub risky: bool,
    pub utterance: String,
    pub expected: Expected,
}

#[derive(Deserialize)]
struct File {
    cases: Vec<RawCase>,
}

#[derive(Deserialize)]
struct RawCase {
    id: String,
    category: String,
    lang: String,
    #[serde(default)]
    risky: bool,
    utterance: String,
    /// `none` or a list of calls; read by hand so an error can name what is wrong.
    expected: Value,
}

#[derive(Deserialize)]
struct RawCall {
    tool: String,
    #[serde(default)]
    args: BTreeMap<String, Value>,
}

#[derive(Deserialize, Clone)]
#[serde(untagged)]
enum RawScalar {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDict {
    contains: Option<Vec<RawScalar>>,
    contains_any: Option<Vec<RawScalar>>,
    one_of: Option<Vec<RawScalar>>,
    sign: Option<String>,
    range: Option<Vec<f64>>,
    absent: Option<bool>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawMatcher {
    Scalar(RawScalar),
    Dict(RawDict),
}

fn scalar(raw: &RawScalar) -> Scalar {
    match raw {
        RawScalar::Bool(b) => Scalar::Bool(*b),
        RawScalar::Int(i) => Scalar::Num(*i as f64),
        RawScalar::Float(f) => Scalar::Num(*f),
        RawScalar::Str(s) => Scalar::Str(s.clone()),
    }
}

/// A number in a `contains` list (a year, say) is matched as its text.
fn text(raw: &RawScalar) -> String {
    match raw {
        RawScalar::Bool(b) => b.to_string(),
        RawScalar::Int(i) => i.to_string(),
        RawScalar::Float(f) => f.to_string(),
        RawScalar::Str(s) => s.clone(),
    }
}

fn matcher(raw: &RawMatcher) -> Result<Matcher, String> {
    let dict = match raw {
        RawMatcher::Scalar(raw) => return Ok(Matcher::Equals(scalar(raw))),
        RawMatcher::Dict(dict) => dict,
    };
    let mut found = Vec::new();
    if let Some(items) = &dict.contains {
        found.push(Matcher::Contains(items.iter().map(text).collect()));
    }
    if let Some(items) = &dict.contains_any {
        found.push(Matcher::ContainsAny(items.iter().map(text).collect()));
    }
    if let Some(items) = &dict.one_of {
        found.push(Matcher::OneOf(items.iter().map(scalar).collect()));
    }
    if let Some(sign) = &dict.sign {
        found.push(Matcher::Sign(match sign.as_str() {
            "pos" => Sign::Pos,
            "neg" => Sign::Neg,
            other => return Err(format!("sign must be pos or neg, not {other}")),
        }));
    }
    if let Some(range) = &dict.range {
        match range.as_slice() {
            [lo, hi] => found.push(Matcher::Range(*lo, *hi)),
            _ => return Err("range needs exactly [lo, hi]".into()),
        }
    }
    if dict.absent == Some(true) {
        found.push(Matcher::Absent);
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => Err("empty or unknown matcher".into()),
        _ => Err("exactly one matcher per argument".into()),
    }
}

fn convert(raw: RawCase) -> Result<Case, String> {
    let at = |why: String| format!("case {}: {why}", raw.id);
    let lang = match raw.lang.as_str() {
        "es" => Lang::Es,
        "en" => Lang::En,
        other => return Err(at(format!("lang must be es or en, not {other}"))),
    };
    let expected = match &raw.expected {
        Value::String(word) if word == "none" => Expected::NoTools,
        Value::Sequence(items) => Expected::Calls(
            items
                .iter()
                .map(|item| {
                    let call: RawCall = serde_norway::from_value(item.clone())
                        .map_err(|e| at(format!("an expected call is malformed: {e}")))?;
                    let args = call
                        .args
                        .iter()
                        .map(|(name, value)| {
                            let read = serde_norway::from_value::<RawMatcher>(value.clone())
                                .map_err(|_| {
                                    let shown = serde_norway::to_string(value).unwrap_or_default();
                                    format!("cannot read the matcher {}", shown.trim())
                                })
                                .and_then(|raw| matcher(&raw));
                            read.map(|m| (name.clone(), m))
                                .map_err(|why| at(format!("{}.{name}: {why}", call.tool)))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    Ok(ExpectedCall {
                        tool: call.tool.clone(),
                        args,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?,
        ),
        other => {
            let shown = serde_norway::to_string(other).unwrap_or_default();
            return Err(at(format!(
                "expected must be none or a list, not {}",
                shown.trim()
            )));
        }
    };
    Ok(Case {
        id: raw.id.clone(),
        category: raw.category.clone(),
        lang,
        risky: raw.risky,
        utterance: raw.utterance.clone(),
        expected,
    })
}

pub fn parse_cases(yaml: &str) -> Result<Vec<Case>, String> {
    let file: File = serde_norway::from_str(yaml).map_err(|e| format!("commands.yaml: {e}"))?;
    file.cases.into_iter().map(convert).collect()
}

/// Checks the cases against the tool schemas the model is shown: ids are
/// unique, categories are known, every expected tool exists and every
/// expected argument is one of its parameters.
pub fn validate(cases: &[Case]) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for case in cases {
        if !seen.insert(case.id.as_str()) {
            return Err(format!("duplicate case id {}", case.id));
        }
        if !CATEGORIES.contains(&case.category.as_str()) {
            return Err(format!(
                "case {}: unknown category {}",
                case.id, case.category
            ));
        }
        let Expected::Calls(calls) = &case.expected else {
            continue;
        };
        for call in calls {
            let Some(spec) = TOOL_SPECS.iter().find(|s| s.name == call.tool) else {
                return Err(format!("case {}: no tool named {}", case.id, call.tool));
            };
            for (arg, _) in &call.args {
                if !spec.params.iter().any(|p| p.name == arg) {
                    return Err(format!(
                        "case {}: {} has no argument {arg}",
                        case.id, call.tool
                    ));
                }
            }
        }
    }
    Ok(())
}

pub fn load(path: &Path) -> Result<Vec<Case>, String> {
    let yaml = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    parse_cases(&yaml)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::matcher::{ExpectedCall, Matcher, Scalar, Sign};

    const REAL_FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../eval/commands.yaml");

    fn one(yaml_case: &str) -> Result<Vec<Case>, String> {
        parse_cases(&format!("cases:\n{yaml_case}"))
    }

    #[test]
    fn a_case_with_every_matcher_kind_parses() {
        let cases = one(r#"
  - id: all-matchers
    category: reminder
    lang: es
    risky: true
    utterance: "Recordame a las 9"
    expected:
      - {tool: create_reminder, args: {text: {contains: [Mama, Llamar]}, delay_minutes: 20, at_local_time: {one_of: ["21:00", "21"]}, date: {absent: true}}}
      - {tool: set_volume, args: {delta: {sign: neg}, level: {range: [10, 20]}, mute: true}}
      - {tool: open_app, args: {name: {contains_any: [a, b]}}}
      - {tool: power_action, args: {action: lock}}
      - {tool: list_reminders}
"#)
        .unwrap();
        assert_eq!(cases.len(), 1);
        let case = &cases[0];
        assert_eq!(case.id, "all-matchers");
        assert_eq!(case.category, "reminder");
        assert_eq!(case.lang, Lang::Es);
        assert!(case.risky);
        let Expected::Calls(calls) = &case.expected else {
            panic!("calls expected");
        };
        assert_eq!(calls.len(), 5);
        let reminder: &ExpectedCall = &calls[0];
        assert_eq!(reminder.tool, "create_reminder");
        let get = |name: &str| {
            reminder
                .args
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, m)| m.clone())
                .unwrap()
        };
        assert_eq!(
            get("text"),
            Matcher::Contains(vec!["Mama".into(), "Llamar".into()])
        );
        assert_eq!(get("delay_minutes"), Matcher::Equals(Scalar::Num(20.0)));
        assert_eq!(
            get("at_local_time"),
            Matcher::OneOf(vec![Scalar::Str("21:00".into()), Scalar::Str("21".into())])
        );
        assert_eq!(get("date"), Matcher::Absent);
        let volume = &calls[1];
        assert!(
            volume
                .args
                .contains(&("delta".into(), Matcher::Sign(Sign::Neg)))
        );
        assert!(
            volume
                .args
                .contains(&("level".into(), Matcher::Range(10.0, 20.0)))
        );
        assert!(
            volume
                .args
                .contains(&("mute".into(), Matcher::Equals(Scalar::Bool(true))))
        );
        assert!(calls[2].args.contains(&(
            "name".into(),
            Matcher::ContainsAny(vec!["a".into(), "b".into()])
        )));
        assert!(
            calls[3]
                .args
                .contains(&("action".into(), Matcher::Equals(Scalar::Str("lock".into()))))
        );
        assert!(calls[4].args.is_empty());
    }

    #[test]
    fn expected_none_means_no_tool_may_be_called() {
        let cases = one(r#"
  - id: chat
    category: chat
    lang: en
    utterance: "Hi"
    expected: none
"#)
        .unwrap();
        assert_eq!(cases[0].expected, Expected::NoTools);
        assert!(!cases[0].risky, "risky defaults to false");
    }

    #[test]
    fn a_false_boolean_and_a_negative_number_are_scalars() {
        let cases = one(r#"
  - id: scalars
    category: volume
    lang: en
    utterance: "x"
    expected:
      - {tool: set_volume, args: {mute: false}}
      - {tool: set_volume, args: {delta: -20}}
"#)
        .unwrap();
        let Expected::Calls(calls) = &cases[0].expected else {
            panic!()
        };
        assert!(
            calls[0]
                .args
                .contains(&("mute".into(), Matcher::Equals(Scalar::Bool(false))))
        );
        assert!(
            calls[1]
                .args
                .contains(&("delta".into(), Matcher::Equals(Scalar::Num(-20.0))))
        );
    }

    #[test]
    fn mistakes_name_the_case() {
        for (yaml, needle) in [
            (
                "  - id: bad-lang\n    category: chat\n    lang: fr\n    utterance: x\n    expected: none\n",
                "fr",
            ),
            (
                "  - id: bad-none\n    category: chat\n    lang: en\n    utterance: x\n    expected: maybe\n",
                "maybe",
            ),
            (
                "  - id: bad-matcher\n    category: volume\n    lang: en\n    utterance: x\n    expected:\n      - {tool: set_volume, args: {level: {roughly: 3}}}\n",
                "roughly",
            ),
            (
                "  - id: bad-sign\n    category: volume\n    lang: en\n    utterance: x\n    expected:\n      - {tool: set_volume, args: {delta: {sign: sideways}}}\n",
                "sideways",
            ),
            (
                "  - id: bad-range\n    category: volume\n    lang: en\n    utterance: x\n    expected:\n      - {tool: set_volume, args: {level: {range: [1]}}}\n",
                "range",
            ),
            (
                "  - id: two-keys\n    category: volume\n    lang: en\n    utterance: x\n    expected:\n      - {tool: set_volume, args: {level: {contains: [a], absent: true}}}\n",
                "one matcher",
            ),
        ] {
            let error = one(yaml).unwrap_err();
            assert!(error.contains(needle), "{error}");
        }
    }

    #[test]
    fn broken_yaml_is_an_error_not_a_panic() {
        assert!(parse_cases("cases: [").is_err());
        assert!(parse_cases("nothing: 1").is_err());
        assert!(parse_cases("").is_err());
    }

    fn case(id: &str, tool: &str, arg: &str) -> Case {
        Case {
            id: id.into(),
            category: "open_app".into(),
            lang: Lang::En,
            risky: false,
            utterance: "x".into(),
            expected: Expected::Calls(vec![ExpectedCall {
                tool: tool.into(),
                args: vec![(arg.into(), Matcher::Absent)],
            }]),
        }
    }

    #[test]
    fn validation_rejects_duplicate_ids_unknown_tools_and_unknown_arguments() {
        assert!(validate(&[case("a", "open_app", "name")]).is_ok());
        let duplicate = validate(&[case("a", "open_app", "name"), case("a", "open_app", "name")]);
        assert!(duplicate.unwrap_err().contains("duplicate"));
        let tool = validate(&[case("a", "format_disk", "name")]).unwrap_err();
        assert!(tool.contains("format_disk"), "{tool}");
        let arg = validate(&[case("a", "open_app", "path")]).unwrap_err();
        assert!(arg.contains("path"), "{arg}");
    }

    // --- the real file ----------------------------------------------------------

    #[test]
    fn the_real_eval_file_parses_and_matches_the_tool_schemas() {
        let cases = load(Path::new(REAL_FILE)).expect("eval/commands.yaml");
        assert_eq!(cases.len(), 51);
        validate(&cases).expect("every expected tool and argument exists in the schemas");
    }

    #[test]
    fn the_real_file_has_the_e1_mix() {
        let cases = load(Path::new(REAL_FILE)).unwrap();
        let count = |category: &str| cases.iter().filter(|c| c.category == category).count();
        assert_eq!(count("adversarial"), 6);
        assert_eq!(count("chat"), 4);
        assert_eq!(count("volume"), 10);
        assert_eq!(count("reminder_followup"), 2);
        assert_eq!(cases.iter().filter(|c| c.risky).count(), 9);
        assert_eq!(cases.iter().filter(|c| c.lang == Lang::Es).count(), 25);
        // Every `none` case is chat or adversarial.
        for case in &cases {
            if case.expected == Expected::NoTools {
                assert!(
                    ["chat", "adversarial"].contains(&case.category.as_str()),
                    "{}",
                    case.id
                );
            }
        }
    }

    #[test]
    fn the_follow_up_cases_expect_a_reminder_without_a_time() {
        // Design "Validation before execution": the model calls create_reminder
        // WITHOUT a time and the core asks. The file says so, and this test
        // pins it so nobody edits the cases to chase a number.
        let cases = load(Path::new(REAL_FILE)).unwrap();
        for case in cases.iter().filter(|c| c.category == "reminder_followup") {
            let Expected::Calls(calls) = &case.expected else {
                panic!("{}", case.id)
            };
            assert_eq!(calls[0].tool, "create_reminder");
            for time in ["delay_minutes", "at_local_time", "date"] {
                assert!(
                    calls[0].args.contains(&(time.into(), Matcher::Absent)),
                    "{} must require {time} absent",
                    case.id
                );
            }
        }
    }
}
