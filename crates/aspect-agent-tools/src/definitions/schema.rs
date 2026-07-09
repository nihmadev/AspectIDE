use serde_json::json;

pub struct Param {
    pub name: &'static str,
    pub kind: &'static str,
    pub desc: &'static str,
    pub required: bool,
    pub items: Option<serde_json::Value>,
    pub min_val: Option<i64>,
    pub max_val: Option<i64>,
}

pub const fn req(name: &'static str, kind: &'static str, desc: &'static str) -> Param {
    Param { name, kind, desc, required: true, items: None, min_val: None, max_val: None }
}
pub const fn opt(name: &'static str, kind: &'static str, desc: &'static str) -> Param {
    Param { name, kind, desc, required: false, items: None, min_val: None, max_val: None }
}
pub const fn opt_int(name: &'static str, desc: &'static str, min: i64, max: i64) -> Param {
    Param { name, kind: "integer", desc, required: false, items: None, min_val: Some(min), max_val: Some(max) }
}
pub fn req_str_arr(name: &'static str, desc: &'static str) -> Param {
    Param { name, kind: "array", desc, required: true, items: Some(json!({ "type": "string" })), min_val: None, max_val: None }
}
pub fn opt_str_arr(name: &'static str, desc: &'static str) -> Param {
    Param { name, kind: "array", desc, required: false, items: Some(json!({ "type": "string" })), min_val: None, max_val: None }
}
pub const fn req_arr_items(name: &'static str, desc: &'static str, items: serde_json::Value) -> Param {
    Param { name, kind: "array", desc, required: true, items: Some(items), min_val: None, max_val: None }
}
pub const fn opt_arr_items(name: &'static str, desc: &'static str, items: serde_json::Value) -> Param {
    Param { name, kind: "array", desc, required: false, items: Some(items), min_val: None, max_val: None }
}

pub fn tool(name: &str, description: &str, params: &[Param]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for p in params {
        let mut schema = if let Some(ref items) = p.items {
            json!({ "type": "array", "description": p.desc, "items": items })
        } else {
            json!({ "type": p.kind, "description": p.desc })
        };
        if let (Some(min), Some(max)) = (p.min_val, p.max_val) {
            if let Some(obj) = schema.as_object_mut() {
                obj.insert("minimum".to_string(), json!(min));
                obj.insert("maximum".to_string(), json!(max));
            }
        }
        properties.insert(p.name.to_string(), schema);
        if p.required {
            required.push(json!(p.name));
        }
    }
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required,
                "additionalProperties": false,
            }
        }
    })
}

pub fn steps_item_schema() -> serde_json::Value {
    json!({
        "anyOf": [
            { "type": "string" },
            {
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "detail": { "type": "string" },
                    "file": { "type": "string" }
                },
                "required": ["title"]
            }
        ]
    })
}

pub fn alternatives_item_schema() -> serde_json::Value {
    json!({
        "anyOf": [
            { "type": "string" },
            {
                "type": "object",
                "properties": {
                    "option": { "type": "string" },
                    "tradeoff": { "type": "string" }
                },
                "required": ["option", "tradeoff"]
            }
        ]
    })
}

pub fn ask_user_options_item_schema() -> serde_json::Value {
    json!({
        "anyOf": [
            { "type": "string" },
            {
                "type": "object",
                "properties": {
                    "label": { "type": "string" },
                    "description": { "type": "string" }
                },
                "required": ["label"]
            }
        ]
    })
}

pub fn todo_item_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "content": { "type": "string", "description": "The task text" },
            "id": { "type": "string", "description": "Stable id (optional; auto-assigned if omitted)" },
            "status": { "type": "string", "enum": ["pending", "in_progress", "completed", "blocked", "cancelled"], "description": "Task status (default pending). Use 'blocked' when the task cannot proceed until something is resolved, 'cancelled' when it is no longer needed." },
            "priority": { "type": "string", "enum": ["low", "medium", "high"], "description": "Task priority (default medium)" },
            "notes": { "type": "string", "description": "Optional notes" }
        },
        "required": ["content"],
        "additionalProperties": false
    })
}

pub fn patch_operation_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "action": {
                "type": "string",
                "description": "Operation kind",
                "enum": [
                    "create", "write", "rewrite", "replacefile", "replace_file",
                    "strreplace", "str_replace", "replace", "delete", "remove"
                ]
            },
            "path": { "type": "string", "description": "Target file path" },
            "text": { "type": "string", "description": "New content (create/rewrite)" },
            "oldText": { "type": "string", "description": "Text to replace" },
            "newText": { "type": "string", "description": "Replacement text" },
            "expectedReplacements": {
                "type": "integer", "description": "Expected occurrence count",
                "minimum": 1, "maximum": 1000
            },
            "overwrite": { "type": "boolean", "description": "Allow overwrite existing" }
        },
        "required": ["action", "path"],
        "additionalProperties": false
    })
}
