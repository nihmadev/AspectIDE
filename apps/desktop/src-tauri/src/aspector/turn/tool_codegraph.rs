use aspect_ai_core::*;

pub async fn execute_code_graph_definition(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let symbol = json_str(&args, "symbol");
    let result = crate::services::code_graph::code_graph_query(state.clone(), symbol.to_string()).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_code_graph_callers(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let symbol = json_str(&args, "symbol");
    let result = crate::services::code_graph::code_graph_query(state.clone(), symbol.to_string()).await?;
    Ok(serde_json::json!({ "callers": result.callers }).to_string())
}

pub async fn execute_code_graph_callees(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let symbol = json_str(&args, "symbol");
    let result = crate::services::code_graph::code_graph_query(state.clone(), symbol.to_string()).await?;
    Ok(serde_json::json!({ "callees": result.callees }).to_string())
}

pub async fn execute_code_graph_explain(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let symbol = json_str(&args, "symbol");
    let result = crate::services::code_graph::code_graph_query(state.clone(), symbol.to_string()).await?;
    Ok(serde_json::json!({ "explanation": result.explanation }).to_string())
}

pub async fn execute_code_graph_overview(
    state: &tauri::State<'_, crate::SharedState>,
    _tc: &ParsedToolCall,
) -> Result<String, String> {
    let result = crate::services::code_graph::code_graph_status(state.clone()).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}
