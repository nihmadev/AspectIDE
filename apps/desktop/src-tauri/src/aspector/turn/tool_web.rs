use aspect_ai_core::*;

pub async fn execute_web_fetch(
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let url = json_str(&args, "url");
    let max_chars = json_usize(&args, "maxChars", 5000) as u64;
    let result = crate::network::web_fetch::fetch(url, Some(max_chars), None).await?;
    Ok(serde_json::to_string(&result).map_err(|e| e.to_string())?)
}

pub async fn execute_web_research(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let query = json_str(&args, "query");
    let _max = json_usize(&args, "maxResults", 7);
    let result = crate::network::research::web_research(
        state.clone(), query.to_string(), None,
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_multi_web_research(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let queries: Vec<String> = serde_json::from_value(
        args.get("queries").cloned().unwrap_or(serde_json::Value::Array(vec![]))
    ).map_err(|e| format!("MultiWebResearch requires `queries` string[]: {e}"))?;
    let _max = json_usize(&args, "maxResults", 7);
    let mut results = Vec::with_capacity(queries.len());
    for q in &queries {
        let try_res = crate::network::research::web_research(state.clone(), q.clone(), None).await;
        match try_res {
            Ok(r) => results.push(serde_json::json!({ "query": q, "ok": true, "results": r })),
            Err(e) => results.push(serde_json::json!({ "query": q, "ok": false, "error": e })),
        }
    }
    serde_json::to_string(&serde_json::json!({ "results": results })).map_err(|e| e.to_string())
}
