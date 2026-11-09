//! Pure wire format of the Ollama adapter, against fixtures. No network.

use niva_core::ports::{ChatRequest, LlmError};
use niva_core::tools::ArgValue;
use serde_json::{Value, json};

use super::*;

fn request(allow_tools: bool) -> ChatRequest {
    ChatRequest {
        system: "You are Niva.".into(),
        user: "open notepad".into(),
        allow_tools,
    }
}

fn body(request: &ChatRequest) -> Value {
    serde_json::from_str(&chat_body("qwen3.5:4b-q4_K_M", request)).expect("valid JSON")
}

// --- request ------------------------------------------------------------------

#[test]
fn the_request_uses_the_settings_e1_measured() {
    let body = body(&request(true));
    assert_eq!(body["model"], "qwen3.5:4b-q4_K_M");
    assert_eq!(body["stream"], false);
    assert_eq!(body["think"], false);
    assert_eq!(body["keep_alive"], "30m");
    assert_eq!(body["options"]["temperature"], 0);
    assert_eq!(body["options"]["num_ctx"], 4096);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][0]["content"], "You are Niva.");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["messages"][1]["content"], "open notepad");
    assert_eq!(body["messages"].as_array().unwrap().len(), 2);
}

#[test]
fn a_tool_request_offers_the_seven_schemas_in_e1_order() {
    let body = body(&request(true));
    let names: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
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
    assert!(body["tools"][0]["type"] == "function");
}

#[test]
fn the_no_tools_summary_call_sends_no_tool_schema_at_all() {
    let body = body(&request(false));
    assert!(body.get("tools").is_none(), "{body}");
}

#[test]
fn set_volume_is_a_schema_with_three_optional_arguments() {
    // Verify 2a S6.
    let body = body(&request(true));
    let volume = &body["tools"][2]["function"];
    assert_eq!(volume["name"], "set_volume");
    let params = &volume["parameters"];
    assert_eq!(params["required"], json!([]));
    assert!(params.get("additionalProperties").is_none());
    assert_eq!(params["properties"]["level"]["type"], "integer");
    assert_eq!(params["properties"]["delta"]["type"], "integer");
    assert_eq!(params["properties"]["mute"]["type"], "boolean");
}

#[test]
fn schemas_follow_the_json_schema_shape_ollama_expects() {
    let body = body(&request(true));
    let power = &body["tools"][4]["function"];
    assert_eq!(power["name"], "power_action");
    assert_eq!(
        power["parameters"]["properties"]["action"],
        json!({"type": "string", "enum": ["lock", "sleep", "restart", "shutdown"]})
    );
    assert_eq!(power["parameters"]["required"], json!(["action"]));
    let list = &body["tools"][6]["function"];
    assert_eq!(list["parameters"]["properties"], json!({}));
    let reminder = &body["tools"][5]["function"]["parameters"];
    assert_eq!(reminder["required"], json!(["text"]));
    assert_eq!(
        reminder["properties"]["delay_minutes"]["description"],
        "Minutes from now, for relative requests ('in 20 minutes')."
    );
}

/// The tools exactly as the Python client sent them in E1 (captured with
/// `ollama.Client`'s own serialization; see the module docs).
const E1_TOOLS: &str = include_str!("e1_tools.json");

/// E1's tools with the deliberate changes since: the `close_app` (E1 attempt
/// 3), `power_action` (attempt 4) and `set_volume` (Batch 38) descriptions
/// gained sentences. Nothing else differs, and the E1 text is still the start
/// of each description.
fn e1_with_known_changes() -> Value {
    let mut theirs: Value = serde_json::from_str(E1_TOOLS).unwrap();
    for (index, name) in [(1, "close_app"), (2, "set_volume"), (4, "power_action")] {
        let function = &mut theirs[index]["function"];
        assert_eq!(function["name"], name);
        let e1_text = function["description"].as_str().unwrap().to_string();
        let ours = niva_core::prompt::TOOL_SPECS
            .iter()
            .find(|spec| spec.name == name)
            .unwrap()
            .description;
        assert!(ours.starts_with(&e1_text), "only appended to: {ours}");
        function["description"] = json!(ours);
    }
    theirs
}

#[test]
fn the_tool_schemas_are_what_the_e1_client_put_on_the_wire() {
    let ours = body(&request(true))["tools"].clone();
    assert_eq!(ours, e1_with_known_changes());
}

#[test]
fn the_tool_schemas_have_the_e1_key_order_too() {
    // Value equality ignores order; the model reads the text, so compare it.
    let ours = serde_json::to_string(&body(&request(true))["tools"]).unwrap();
    assert_eq!(
        ours,
        serde_json::to_string(&e1_with_known_changes()).unwrap()
    );
    assert!(ours.find("\"required\"").unwrap() < ours.find("\"properties\"").unwrap());
}

#[test]
fn user_text_with_quotes_and_newlines_stays_one_valid_string() {
    let mut req = request(true);
    req.user = "say \"hi\"\nthen \\ close".into();
    assert_eq!(
        body(&req)["messages"][1]["content"],
        "say \"hi\"\nthen \\ close"
    );
}

#[test]
fn the_warm_up_body_is_a_tiny_tool_request_with_the_same_settings() {
    let body: Value = serde_json::from_str(&warm_up_body("m")).unwrap();
    assert_eq!(body["messages"][0]["content"], "Reply with OK.");
    assert_eq!(body["messages"][1]["content"], "ping");
    assert_eq!(body["think"], false);
    assert_eq!(body["keep_alive"], "30m");
    assert_eq!(body["tools"].as_array().unwrap().len(), 7);
}

// --- chat response ------------------------------------------------------------

fn calls_of(body: &str) -> Vec<(String, Vec<(String, ArgValue)>)> {
    parse_chat_response(body)
        .expect("parses")
        .calls
        .into_iter()
        .map(|c| (c.name, c.args.into_iter().collect()))
        .collect()
}

fn reply(message: Value) -> String {
    json!({"model": "m", "created_at": "t", "message": message, "done": true}).to_string()
}

#[test]
fn a_tool_call_with_object_arguments() {
    let body = reply(json!({
        "role": "assistant", "content": "",
        "tool_calls": [{"function": {"name": "open_app", "arguments": {"name": "Spotify"}}}]
    }));
    assert_eq!(
        calls_of(&body),
        vec![(
            "open_app".to_string(),
            vec![("name".to_string(), ArgValue::Str("Spotify".into()))]
        )]
    );
}

#[test]
fn integral_floats_become_ints_before_validation() {
    // Verify 2a S6: `30` and `30.0` must both reach the validator as Int(30).
    for raw in [
        r#"{"level": 30}"#,
        r#"{"level": 30.0}"#,
        r#"{"level": 3e1}"#,
        r#"{"level": 30.00}"#,
    ] {
        let body = format!(
            r#"{{"message":{{"content":"","tool_calls":[{{"function":{{"name":"set_volume","arguments":{raw}}}}}]}}}}"#
        );
        assert_eq!(
            calls_of(&body)[0].1,
            vec![("level".to_string(), ArgValue::Int(30))],
            "{raw}"
        );
    }
}

#[test]
fn negative_and_zero_floats_are_ints_too() {
    let body = reply(json!({
        "content": "",
        "tool_calls": [{"function": {"name": "set_volume", "arguments": {"delta": -20.0}}},
                       {"function": {"name": "set_volume", "arguments": {"level": 0.0}}}]
    }));
    let calls = calls_of(&body);
    assert_eq!(calls[0].1, vec![("delta".to_string(), ArgValue::Int(-20))]);
    assert_eq!(calls[1].1, vec![("level".to_string(), ArgValue::Int(0))]);
}

#[test]
fn a_real_fraction_stays_a_float_so_the_validator_rejects_it() {
    let body = reply(json!({
        "content": "",
        "tool_calls": [{"function": {"name": "set_volume", "arguments": {"level": 30.5}}}]
    }));
    assert_eq!(
        calls_of(&body)[0].1,
        vec![("level".to_string(), ArgValue::Float(30.5))]
    );
}

#[test]
fn numbers_too_big_to_be_exact_ints_stay_floats() {
    let body = r#"{"message":{"content":"","tool_calls":[{"function":{"name":"set_volume","arguments":{"level":1e300}}}]}}"#;
    assert_eq!(
        calls_of(body)[0].1,
        vec![("level".to_string(), ArgValue::Float(1e300))]
    );
}

#[test]
fn arguments_may_arrive_as_a_json_string() {
    let body = reply(json!({
        "content": "",
        "tool_calls": [{"function": {"name": "set_volume", "arguments": "{\"level\": 30.0, \"x\": null}"}}]
    }));
    assert_eq!(
        calls_of(&body)[0].1,
        vec![
            ("level".to_string(), ArgValue::Int(30)),
            ("x".to_string(), ArgValue::Null)
        ]
    );
}

#[test]
fn missing_null_and_empty_arguments_are_no_arguments() {
    for arguments in [json!(null), json!({}), json!(""), json!("   ")] {
        let body = reply(json!({
            "content": "",
            "tool_calls": [{"function": {"name": "list_reminders", "arguments": arguments}}]
        }));
        assert_eq!(
            calls_of(&body),
            vec![("list_reminders".to_string(), vec![])]
        );
    }
    let body = reply(json!({
        "content": "",
        "tool_calls": [{"function": {"name": "list_reminders"}}]
    }));
    assert_eq!(calls_of(&body).len(), 1);
}

#[test]
fn bools_strings_and_nulls_are_kept_as_they_are() {
    let body = reply(json!({
        "content": "",
        "tool_calls": [{"function": {"name": "set_volume",
            "arguments": {"mute": true, "level": null, "name": "ñandú"}}}]
    }));
    assert_eq!(
        calls_of(&body)[0].1,
        vec![
            ("level".to_string(), ArgValue::Null),
            ("mute".to_string(), ArgValue::Bool(true)),
            ("name".to_string(), ArgValue::Str("ñandú".into())),
        ]
    );
}

#[test]
fn several_calls_keep_their_order() {
    let body = reply(json!({
        "content": "",
        "tool_calls": [
            {"function": {"name": "open_app", "arguments": {"name": "Chrome"}}},
            {"function": {"name": "set_volume", "arguments": {"mute": true}}}
        ]
    }));
    let names: Vec<String> = calls_of(&body).into_iter().map(|c| c.0).collect();
    assert_eq!(names, ["open_app", "set_volume"]);
}

#[test]
fn text_and_calls_come_back_together() {
    let body = reply(json!({
        "content": "Opening it.",
        "tool_calls": [{"function": {"name": "open_app", "arguments": {"name": "x"}}}]
    }));
    let parsed = parse_chat_response(&body).unwrap();
    assert_eq!(parsed.text, "Opening it.");
    assert_eq!(parsed.calls.len(), 1);
}

#[test]
fn empty_content_without_calls_is_an_empty_reply_not_an_error() {
    for message in [
        json!({"role": "assistant", "content": ""}),
        json!({"role": "assistant"}),
    ] {
        let parsed = parse_chat_response(&reply(message)).unwrap();
        assert_eq!(parsed.text, "");
        assert!(parsed.calls.is_empty());
    }
}

#[test]
fn extra_fields_are_ignored() {
    let body = reply(json!({
        "role": "assistant", "content": "Hi", "thinking": "hmm", "images": null,
        "tool_calls": [{"id": "call_1", "function": {"index": 0, "name": "list_reminders", "arguments": {}}}]
    }));
    assert_eq!(parse_chat_response(&body).unwrap().calls.len(), 1);
}

fn is_failed(body: &str) -> bool {
    matches!(parse_chat_response(body), Err(LlmError::Failed(_)))
}

#[test]
fn malformed_json_is_a_failure() {
    for body in ["", "not json", "{\"message\": ", "[1,2,3]", "null"] {
        assert!(is_failed(body), "{body:?}");
    }
}

#[test]
fn a_response_without_a_message_is_a_failure() {
    assert!(is_failed(r#"{"done": true}"#));
    assert!(is_failed(r#"{"message": "text"}"#));
}

#[test]
fn an_error_body_is_a_failure_that_says_why() {
    match parse_chat_response(r#"{"error":"model requires more system memory"}"#) {
        Err(LlmError::Failed(why)) => assert!(why.contains("more system memory"), "{why}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_call_without_a_function_or_a_name_is_a_failure() {
    assert!(is_failed(&reply(
        json!({"content": "", "tool_calls": [{}]})
    )));
    assert!(is_failed(&reply(
        json!({"content": "", "tool_calls": [{"function": {"arguments": {}}}]})
    )));
    assert!(is_failed(&reply(
        json!({"content": "", "tool_calls": [{"function": {"name": 7, "arguments": {}}}]})
    )));
    assert!(is_failed(&reply(
        json!({"content": "", "tool_calls": "oops"})
    )));
}

#[test]
fn arguments_that_are_not_an_object_are_a_failure() {
    for arguments in [
        json!([1, 2]),
        json!(5),
        json!(true),
        json!("not json"),
        json!("[1]"),
    ] {
        let body = reply(json!({
            "content": "",
            "tool_calls": [{"function": {"name": "open_app", "arguments": arguments}}]
        }));
        assert!(is_failed(&body), "{arguments}");
    }
}

#[test]
fn nested_values_have_no_place_in_any_schema_and_are_rejected() {
    for value in [json!([1]), json!({"a": 1})] {
        let body = reply(json!({
            "content": "",
            "tool_calls": [{"function": {"name": "open_app", "arguments": {"name": value}}}]
        }));
        assert!(is_failed(&body), "{value}");
    }
}

// --- tags and ps ---------------------------------------------------------------

#[test]
fn tags_lists_the_installed_model_names() {
    let body = json!({"models": [
        {"name": "llama3.2:3b", "model": "llama3.2:3b", "size": 1},
        {"name": "qwen3.5:4b-q4_K_M", "model": "qwen3.5:4b-q4_K_M", "size": 2}
    ]})
    .to_string();
    assert_eq!(
        parse_tags(&body).unwrap(),
        ["llama3.2:3b", "qwen3.5:4b-q4_K_M"]
    );
    assert_eq!(
        parse_tags(r#"{"models": []}"#).unwrap(),
        Vec::<String>::new()
    );
    assert!(parse_tags("nope").is_err());
    assert!(parse_tags(r#"{"nothing": 1}"#).is_err());
}

#[test]
fn ps_lists_loaded_models_with_their_sizes() {
    let body = json!({"models": [{
        "name": "qwen3.5:4b-q4_K_M", "model": "qwen3.5:4b-q4_K_M",
        "size": 5_000_000_000u64, "size_vram": 5_000_000_000u64,
        "digest": "d8b0f5e9760c", "expires_at": "2026-11-04T10:00:00-05:00"
    }]})
    .to_string();
    let loaded = parse_ps(&body).unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].name, "qwen3.5:4b-q4_K_M");
    assert_eq!(loaded[0].size, 5_000_000_000);
    assert_eq!(loaded[0].size_vram, 5_000_000_000);
    assert_eq!(loaded[0].digest, "d8b0f5e9760c");
    assert!(parse_ps(r#"{"models": []}"#).unwrap().is_empty());
    assert!(parse_ps("nope").is_err());
}

#[test]
fn ps_tolerates_a_model_without_vram_fields() {
    let loaded = parse_ps(r#"{"models":[{"name":"m"}]}"#).unwrap();
    assert_eq!((loaded[0].size, loaded[0].size_vram), (0, 0));
}

#[test]
fn the_version_is_read_from_api_version() {
    assert_eq!(
        parse_version(r#"{"version":"0.40.14"}"#).unwrap(),
        "0.40.14"
    );
    assert!(parse_version(r#"{"nothing":1}"#).is_err());
    assert!(parse_version("nope").is_err());
}

#[test]
fn the_digest_of_an_installed_model_is_read_from_tags() {
    let body = json!({"models": [
        {"name": "llama3.2:3b", "digest": "a80c"},
        {"name": "qwen3.5:4b-q4_K_M", "digest": "d8b0f5e9760c"}
    ]})
    .to_string();
    assert_eq!(
        parse_digest(&body, "qwen3.5:4b-q4_K_M").unwrap().as_deref(),
        Some("d8b0f5e9760c")
    );
    assert_eq!(parse_digest(&body, "other:1b").unwrap(), None);
    assert!(parse_digest("nope", "x").is_err());
}

#[test]
fn an_error_message_is_pulled_out_of_an_error_body() {
    assert_eq!(
        error_message(r#"{"error":"model 'x' not found"}"#),
        "model 'x' not found"
    );
    assert_eq!(error_message("plain text"), "plain text");
    assert_eq!(error_message(""), "");
}
