use serde_json::{json, Value};

/// Convert an OpenAI tool definition into an Anthropic tool definition.
pub fn convert_tool(tool: &Value) -> Option<Value> {
    let function = tool.get("function")?;
    let name = function.get("name").and_then(Value::as_str)?;
    let mut out = json!({ "name": name });
    if let Some(description) = function.get("description").and_then(Value::as_str) {
        out["description"] = json!(description);
    }
    out["input_schema"] = function
        .get("parameters")
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!({ "type": "object" }));
    Some(out)
}

/// True when the OpenAI-shaped `tool_choice` would translate to Anthropic's
/// `any`/`tool` (forcing) shapes, which the API rejects alongside `thinking`.
pub fn tool_choice_forces_tool_use(choice: Option<&Value>) -> bool {
    match choice {
        Some(Value::String(value)) => value == "required" || value == "any",
        Some(Value::Object(object)) => object
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .is_some(),
        _ => false,
    }
}

/// Map an `OpenAI` `tool_choice` onto an Anthropic one.
pub fn convert_tool_choice(choice: Option<&Value>, disable_parallel: bool) -> Value {
    let mut out = match choice {
        Some(Value::String(value)) if value == "none" => {
            return json!({ "type": "none" });
        }
        Some(Value::String(value)) if value == "required" || value == "any" => {
            json!({ "type": "any" })
        }
        Some(Value::Object(object)) => object
            .get("function")
            .and_then(|f| f.get("name"))
            .and_then(Value::as_str)
            .map_or_else(
                || json!({ "type": "auto" }),
                |name| json!({ "type": "tool", "name": name }),
            ),
        _ => json!({ "type": "auto" }),
    };
    if disable_parallel {
        out["disable_parallel_tool_use"] = Value::Bool(true);
    }
    out
}
