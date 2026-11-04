//! The Ollama wire format: request bodies out, responses in. Pure functions
//! with no I/O, tested against fixtures (`wire/tests.rs`).
//!
//! The tool schemas are byte-for-byte what the Python client put on the wire in
//! E1 (`wire/e1_tools.json`, captured from `run_e1.py` through the client's own
//! serialization): the client's models drop `additionalProperties` and put
//! `required` before `properties`, and Ollama renders the JSON into the prompt,
//! so even the key order changes what the model reads. The first port sent
//! them alphabetically and lost a few percent of tool selection.
//!
//! Settings are exactly what E1 measured (`spikes/eval/run_e1.py`):
//! `think: false`, `temperature: 0`, `keep_alive: "30m"`, plus an explicit
//! `num_ctx` of 4096. E1 left `num_ctx` at the server default of that run;
//! 4096 is the value the S1 co-residency measurement used, pinned here so a
//! change of Ollama's default cannot move the context size (and the VRAM) under
//! the product.

use niva_core::ports::{ChatReply, ChatRequest, LlmError};
use niva_core::prompt::{ParamKind, TOOL_SPECS, ToolSpec};
use niva_core::tools::{ArgValue, Args, ToolCall};
use serde_json::{Map, Value, json};

#[cfg(test)]
mod tests;

/// Context window sent with every request.
pub const NUM_CTX: u32 = 4096;
/// How long Ollama keeps the model loaded after a request.
pub const KEEP_ALIVE: &str = "30m";
/// Integers up to 2^53 are exact in an f64; beyond that a float is not an int.
const MAX_EXACT_INT: f64 = 9_007_199_254_740_992.0;

fn tool_json(spec: &ToolSpec) -> Value {
    let mut properties = Map::new();
    for param in spec.params {
        let mut schema = Map::new();
        match param.kind {
            ParamKind::Integer => schema.insert("type".into(), json!("integer")),
            ParamKind::Boolean => schema.insert("type".into(), json!("boolean")),
            ParamKind::Text => schema.insert("type".into(), json!("string")),
            ParamKind::OneOf(values) => {
                schema.insert("type".into(), json!("string"));
                schema.insert("enum".into(), json!(values))
            }
        };
        if !param.description.is_empty() {
            schema.insert("description".into(), json!(param.description));
        }
        properties.insert(param.name.into(), Value::Object(schema));
    }
    json!({
        "type": "function",
        "function": {
            "name": spec.name,
            "description": spec.description,
            // Key order and the absence of `additionalProperties` are what the
            // Python client sent in E1 (see `wire/e1_tools.json`).
            "parameters": {
                "type": "object",
                "required": spec.required,
                "properties": properties,
            },
        },
    })
}

fn chat_json(model: &str, system: &str, user: &str, with_tools: bool) -> Value {
    let mut body = json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "stream": false,
        "think": false,
        "keep_alive": KEEP_ALIVE,
        "options": {"temperature": 0, "num_ctx": NUM_CTX},
    });
    if with_tools {
        body["tools"] = Value::Array(TOOL_SPECS.iter().map(tool_json).collect());
    }
    body
}

/// The body of one `/api/chat` call. The no-tools summary call
/// (`allow_tools: false`) sends no `tools` key at all.
pub fn chat_body(model: &str, request: &ChatRequest) -> String {
    chat_json(model, &request.system, &request.user, request.allow_tools).to_string()
}

/// The warm-up call of the spike: a trivial request that loads the model with
/// the same tool schemas and settings.
pub fn warm_up_body(model: &str) -> String {
    chat_json(model, "Reply with OK.", "ping", true).to_string()
}

fn failed(why: impl Into<String>) -> LlmError {
    LlmError::Failed(why.into())
}

/// A JSON number as the validator wants it: an integral float (`30.0`, `3e1`)
/// is an int (verify 2a S6), a real fraction or an inexact giant stays a float.
fn decode_number(number: &serde_json::Number) -> ArgValue {
    if let Some(int) = number.as_i64() {
        return ArgValue::Int(int);
    }
    match number.as_f64() {
        Some(float) if float.is_finite() && float.fract() == 0.0 && float.abs() < MAX_EXACT_INT => {
            ArgValue::Int(float as i64)
        }
        Some(float) => ArgValue::Float(float),
        None => ArgValue::Null,
    }
}

fn decode_arg(value: &Value) -> Option<ArgValue> {
    match value {
        Value::Null => Some(ArgValue::Null),
        Value::Bool(b) => Some(ArgValue::Bool(*b)),
        Value::Number(n) => Some(decode_number(n)),
        Value::String(s) => Some(ArgValue::Str(s.clone())),
        // No tool schema has an array or object argument.
        Value::Array(_) | Value::Object(_) => None,
    }
}

/// The `arguments` of a call: an object, or a string holding one, or nothing.
fn decode_args(name: &str, arguments: Option<&Value>) -> Result<Args, LlmError> {
    let parsed;
    let object = match arguments {
        None | Some(Value::Null) => return Ok(Args::new()),
        Some(Value::Object(object)) => object,
        Some(Value::String(text)) if text.trim().is_empty() => return Ok(Args::new()),
        Some(Value::String(text)) => {
            parsed = serde_json::from_str::<Value>(text)
                .map_err(|_| failed(format!("{name}: arguments are not valid JSON")))?;
            parsed
                .as_object()
                .ok_or_else(|| failed(format!("{name}: arguments are not an object")))?
        }
        Some(_) => return Err(failed(format!("{name}: arguments are not an object"))),
    };
    let mut args = Args::new();
    for (key, value) in object {
        let value = decode_arg(value)
            .ok_or_else(|| failed(format!("{name}: argument {key} has an unsupported type")))?;
        args.insert(key.clone(), value);
    }
    Ok(args)
}

fn decode_call(call: &Value) -> Result<ToolCall, LlmError> {
    let function = call
        .get("function")
        .and_then(Value::as_object)
        .ok_or_else(|| failed("a tool call has no function"))?;
    let name = function
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| failed("a tool call has no name"))?;
    Ok(ToolCall {
        name: name.to_string(),
        args: decode_args(name, function.get("arguments"))?,
    })
}

/// Parses an `/api/chat` response (non-streaming). Unknown fields are
/// ignored; empty content with no calls is an empty reply (the assistant turns
/// it into an error), not a failure here.
pub fn parse_chat_response(body: &str) -> Result<ChatReply, LlmError> {
    let value: Value =
        serde_json::from_str(body).map_err(|e| failed(format!("malformed response: {e}")))?;
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        return Err(failed(error));
    }
    let message = value
        .get("message")
        .and_then(Value::as_object)
        .ok_or_else(|| failed("response has no message"))?;
    let text = message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let calls = match message.get("tool_calls") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(calls)) => calls.iter().map(decode_call).collect::<Result<_, _>>()?,
        Some(_) => return Err(failed("tool_calls is not a list")),
    };
    Ok(ChatReply { text, calls })
}

/// Model names of an `/api/tags` response.
pub fn parse_tags(body: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or("no models list")?;
    Ok(models
        .iter()
        .filter_map(|m| m.get("name").and_then(Value::as_str))
        .map(str::to_string)
        .collect())
}

/// The server version of `/api/version`.
pub fn parse_version(body: &str) -> Result<String, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    value
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "no version".to_string())
}

/// The digest of the installed model called `model` (`/api/tags`), if any.
/// The tag is mutable, so results are only comparable by digest.
pub fn parse_digest(body: &str, model: &str) -> Result<Option<String>, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or("no models list")?;
    Ok(models
        .iter()
        .find(|m| m.get("name").and_then(Value::as_str) == Some(model))
        .and_then(|m| m.get("digest").and_then(Value::as_str))
        .map(str::to_string))
}

/// A model Ollama has loaded (`/api/ps`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loaded {
    pub name: String,
    /// Total bytes the loaded model takes.
    pub size: u64,
    /// Bytes of it in GPU memory.
    pub size_vram: u64,
    pub digest: String,
}

pub fn parse_ps(body: &str) -> Result<Vec<Loaded>, String> {
    let value: Value = serde_json::from_str(body).map_err(|e| e.to_string())?;
    let models = value
        .get("models")
        .and_then(Value::as_array)
        .ok_or("no models list")?;
    let text = |m: &Value, key: &str| m.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let number = |m: &Value, key: &str| m.get(key).and_then(Value::as_u64).unwrap_or(0);
    Ok(models
        .iter()
        .map(|m| Loaded {
            name: text(m, "name"),
            size: number(m, "size"),
            size_vram: number(m, "size_vram"),
            digest: text(m, "digest"),
        })
        .collect())
}

/// The `error` of an Ollama error body, or the body itself if it is not one.
pub fn error_message(body: &str) -> String {
    serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_string))
        .unwrap_or_else(|| body.trim().to_string())
}
