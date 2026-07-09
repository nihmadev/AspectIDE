use tauri::State;

use aspect_ai_core::*;

use crate::SharedState;

pub async fn execute_rules_context(
    state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let query = json_str_opt(&args, "query");
    let _max = json_usize(&args, "maxResults", 25);
    let result = crate::aspector::context::sources::ai_rules_context(
        state.clone(), query.clone(), None, None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_docs_context(
    state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let query = json_str_opt(&args, "query");
    let _max = json_usize(&args, "maxResults", 25);
    let result = crate::aspector::context::sources::ai_docs_context(
        state.clone(), query.clone(), None, None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_memory_context(
    state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let query = json_str_opt(&args, "query");
    let result = crate::aspector::context::sources::ai_memory_context(
        state.clone(), query.clone(), None, None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub fn execute_list_skills(
    app: &tauri::AppHandle,
    state: &State<'_, SharedState>,
    _tc: &ParsedToolCall,
) -> Result<String, String> {
    let skills = crate::platform::skills::skills_list(app.clone(), state.clone());
    Ok(serde_json::json!({ "skills": skills }).to_string())
}

pub async fn execute_use_skill(
    _app: &tauri::AppHandle,
    _state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let name = json_str(&args, "name");
    let _extra = args.get("extra").cloned();
    Err(format!("Skill '{name}' use not yet implemented"))
}

pub async fn execute_recall_memory(
    _app: &tauri::AppHandle,
    state: &State<'_, SharedState>,
    _input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let query = json_str(&args, "query");
    let result = crate::aspector::context::sources::ai_memory_context(
        state.clone(), Some(query), None, None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_relate_memories(
    _app: &tauri::AppHandle,
    _state: &State<'_, SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let _args = tc.args.clone();
    let _source = json_str(&_args, "sourceId");
    let _target = json_str(&_args, "targetId");
    let _relation = json_str_opt(&_args, "relation").unwrap_or("related".to_string());
    Ok(serde_json::json!({ "status": "not implemented" }).to_string())
}

pub async fn execute_remember_memory(
    _app: &tauri::AppHandle,
    _state: &State<'_, SharedState>,
    _input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let _args = tc.args.clone();
    let _text = json_str(&_args, "text");
    let _tags = _args.get("tags").and_then(|v| v.as_array()).map(|a| {
        a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()
    }).unwrap_or_default();
    let _sources = _args.get("sources").and_then(|v| v.as_array()).map(|a| {
        a.iter().filter_map(|v| v.as_str().map(String::from)).collect::<Vec<_>>()
    }).unwrap_or_default();
    Ok(serde_json::json!({ "status": "not implemented" }).to_string())
}

pub async fn execute_fast_context(
    _app: &tauri::AppHandle,
    _state: &State<'_, SharedState>,
    _input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let _args = tc.args.clone();
    let _target = json_str(&_args, "target");
    Ok(serde_json::json!({ "context": "not available" }).to_string())
}

pub async fn execute_context_budgeter(
    _app: &tauri::AppHandle,
    _state: &State<'_, SharedState>,
    _input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let approx_tokens = args.get("approxTokens").and_then(serde_json::Value::as_u64);
    Ok(serde_json::json!({ "budget": approx_tokens.unwrap_or(0) }).to_string())
}
