use aspect_ai_core::*;

use super::approval::require_tool_approval;
use super::helpers::{augment_checkpoint_before_edit, require_file_read_before_edit};

pub async fn execute_read(
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let path = json_str(&args, "path");
    let max_bytes = args.get("maxBytes").and_then(serde_json::Value::as_u64);
    let start_line = args.get("startLine").and_then(serde_json::Value::as_u64).and_then(|v| u32::try_from(v).ok());
    let max_lines = args.get("maxLines").and_then(serde_json::Value::as_u64).and_then(|v| u32::try_from(v).ok());
    let result = crate::aspector::tools::executors::ai_read_file(
        state.clone(), std::path::PathBuf::from(path), max_bytes, start_line, max_lines,
    ).await?;
    crate::aspector::session::store::mark_file_read(&input.session_id, &result.path);
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_glob(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let pattern = json_str(&args, "pattern");
    let max = json_usize(&args, "maxResults", 80);
    let result = crate::aspector::tools::executors::ai_glob(state.clone(), pattern, Some(max)).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_symbol_context(
    state: &tauri::State<'_, crate::SharedState>,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let query = json_str_opt(&args, "query");
    let path = json_str_opt(&args, "path").map(std::path::PathBuf::from);
    let line = args.get("line").and_then(serde_json::Value::as_u64).and_then(|v| u32::try_from(v).ok()).map(|v| v.saturating_add(1));
    let column = args.get("column").and_then(serde_json::Value::as_u64).and_then(|v| u32::try_from(v).ok()).map(|v| v.saturating_add(1));
    let max = json_usize(&args, "maxResults", 80);
    let result = crate::aspector::tools::executors::ai_symbol_context(
        state.clone(), query, path, line, column, Some(max),
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_write(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let path = json_str(&args, "path");
    let text = json_str(&args, "text");
    let overwrite = args.get("overwrite").and_then(serde_json::Value::as_bool);
    if overwrite.unwrap_or(false) {
        require_file_read_before_edit(state, &input.session_id, "Write", &path)?;
    }
    let save = if is_automatic { Some(true) } else { args.get("saveToDisk").and_then(serde_json::Value::as_bool) };
    let resolved_path = crate::resolve_workspace_path(state, std::path::Path::new(&path))
        .map_or_else(|_| path.clone(), |p| p.to_string_lossy().replace('\\', "/"));
    let effective = if is_automatic { "full-access" } else { input.tool_approval_mode.as_str() };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "Write", &format!("Write to {path}"),
        &text.chars().take(400).collect::<String>(),
        if overwrite.unwrap_or(false) { "modify" } else { "create" },
        &input.tool_permission_rules, &resolved_path, false,
    ).await?;
    augment_checkpoint_before_edit(state, input, &path).await;
    let result = crate::aspector::tools::executors::ai_file_write(
        app.clone(), state.clone(), std::path::PathBuf::from(&path), text, overwrite, save,
    ).await?;
    if let Ok(resolved) = crate::resolve_workspace_path(state, std::path::Path::new(&path)) {
        crate::aspector::session::store::mark_file_read(&input.session_id, &resolved);
    }
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_str_replace(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let path = json_str(&args, "path");
    require_file_read_before_edit(state, &input.session_id, "StrReplace", &path)?;
    let resolved_str_path = crate::resolve_workspace_path(state, std::path::Path::new(&path))
        .map_or_else(|_| path.clone(), |p| p.to_string_lossy().replace('\\', "/"));
    let old_text = json_str(&args, "oldText");
    let new_text = json_str(&args, "newText");
    let expected = args.get("expectedReplacements").and_then(serde_json::Value::as_u64).and_then(|v| usize::try_from(v).ok());
    let save = if is_automatic { Some(true) } else { args.get("saveToDisk").and_then(serde_json::Value::as_bool) };
    let effective = if is_automatic { "full-access" } else { input.tool_approval_mode.as_str() };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "StrReplace", &format!("Replace in {path}"),
        &format!("-{}\n+{}", old_text.chars().take(200).collect::<String>(), new_text.chars().take(200).collect::<String>()),
        "modify", &input.tool_permission_rules, &resolved_str_path, false,
    ).await?;
    augment_checkpoint_before_edit(state, input, &path).await;
    let result = crate::aspector::tools::executors::ai_file_str_replace(
        app.clone(), state.clone(), std::path::PathBuf::from(&path), old_text, new_text, expected, save,
    ).await?;
    if let Ok(resolved) = crate::resolve_workspace_path(state, std::path::Path::new(&path)) {
        crate::aspector::session::store::mark_file_read(&input.session_id, &resolved);
    }
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_delete(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let path = json_str(&args, "path");
    let resolved_delete_path = crate::resolve_workspace_path(state, std::path::Path::new(&path))
        .map_or_else(|_| path.clone(), |p| p.to_string_lossy().replace('\\', "/"));
    let effective = if is_automatic { "full-access" } else { input.tool_approval_mode.as_str() };
    require_tool_approval(
        app, turn_id, tc, effective, interactive,
        "Delete", &format!("Delete {path}"), &resolved_delete_path,
        "delete", &input.tool_permission_rules, &resolved_delete_path, false,
    ).await?;
    augment_checkpoint_before_edit(state, input, &path).await;
    let result = crate::aspector::tools::executors::ai_file_delete(
        app.clone(), state.clone(), std::path::PathBuf::from(path),
    ).await?;
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_inspect_file(
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    tc: &ParsedToolCall,
) -> Result<String, String> {
    let args = tc.args.clone();
    let path = json_str(&args, "path");
    let mut options = aspect_core::FileInspectionOptions::default();
    if let Some(v) = args.get("maxRows").and_then(serde_json::Value::as_u64) {
        options.max_rows = usize::try_from(v).unwrap_or(options.max_rows);
    }
    let max_columns_requested = args.get("maxColumns").and_then(serde_json::Value::as_u64).is_some();
    if let Some(v) = args.get("maxColumns").and_then(serde_json::Value::as_u64) {
        options.max_columns = usize::try_from(v).unwrap_or(options.max_columns);
    }
    if let Some(v) = args.get("maxBytes").and_then(serde_json::Value::as_u64) {
        options.max_text_bytes = v;
    }
    let mut result = crate::files::file_intel::file_inspect(
        state.clone(), std::path::PathBuf::from(path), Some(options),
    ).await?;
    if max_columns_requested && matches!(result.preview, aspect_core::FilePreview::Text { .. }) {
        result.warnings.push(
            "maxColumns applies only to tabular/spreadsheet/notebook previews; it was ignored for this plain-text/source file.".to_string(),
        );
    }
    crate::aspector::session::store::mark_file_read(&input.session_id, &result.path);
    serde_json::to_string(&result).map_err(|e| e.to_string())
}

pub async fn execute_patch_engine(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    is_automatic: bool,
) -> Result<String, String> {
    let args = tc.args.clone();
    let operations_raw = args.get("operations").cloned().unwrap_or(serde_json::json!([]));
    let mut guarded_paths: Vec<String> = Vec::new();
    let mut checkpoint_paths: Vec<String> = Vec::new();
    if let Some(ops) = operations_raw.as_array() {
        for op in ops {
            let action = op.get("action").or_else(|| op.get("kind")).or_else(|| op.get("operation"))
                .and_then(|v| v.as_str()).unwrap_or("").trim().to_ascii_lowercase();
            let overwrite_flag = op.get("overwrite").and_then(serde_json::Value::as_bool).unwrap_or(false);
            let is_create = matches!(action.as_str(), "create");
            let mutates_existing = matches!(action.as_str(),
                "write" | "rewrite" | "replacefile" | "replace_file" | "strreplace"
                | "str_replace" | "replace" | "delete" | "remove"
            ) || (is_create && overwrite_flag);
            let Some(path) = op.get("path").and_then(|v| v.as_str()) else { continue; };
            if mutates_existing || is_create { checkpoint_paths.push(path.to_string()); }
            if !mutates_existing { continue; }
            require_file_read_before_edit(state, &input.session_id, "PatchEngine", path)?;
            guarded_paths.push(path.to_string());
        }
    }
    let serde_json::Value::Array(raw_ops) = operations_raw else {
        return Err("PatchEngine `operations` must be an array.".to_string());
    };
    let mut operations: Vec<crate::aspector::tools::executors::AiFilePatchOperation> = Vec::with_capacity(raw_ops.len());
    for (index, raw) in raw_ops.into_iter().enumerate() {
        let op = serde_json::from_value(raw).map_err(|e| {
            format!("PatchEngine operation[{index}] is invalid: {e}. Every operation needs at least `action` and `path`.")
        })?;
        operations.push(op);
    }
    let save = if is_automatic { Some(true) } else { args.get("saveToDisk").and_then(serde_json::Value::as_bool) };
    let dry_run = args.get("dryRun").and_then(serde_json::Value::as_bool);
    if !dry_run.unwrap_or(false) {
        let resolved_patch_targets: Vec<String> = guarded_paths.iter().map(|p|
            crate::resolve_workspace_path(state, std::path::Path::new(p))
                .map_or_else(|_| p.clone(), |r| r.to_string_lossy().replace('\\', "/"))
        ).collect();
        let effective = if is_automatic { "full-access" } else { input.tool_approval_mode.as_str() };
        for resolved_target in &resolved_patch_targets {
            require_tool_approval(app, turn_id, tc, effective, interactive,
                "PatchEngine", &format!("{} operations", operations.len()), resolved_target,
                "modify", &input.tool_permission_rules, resolved_target, false,
            ).await?;
        }
        if resolved_patch_targets.is_empty() {
            require_tool_approval(app, turn_id, tc, effective, interactive,
                "PatchEngine", &format!("{} operations", operations.len()), "multi-file patch",
                "modify", &input.tool_permission_rules, "patch", false,
            ).await?;
        }
    }
    if !dry_run.unwrap_or(false) {
        for p in &checkpoint_paths {
            augment_checkpoint_before_edit(state, input, p).await;
        }
    }
    let result = crate::aspector::tools::executors::ai_file_patch(
        app.clone(), state.clone(), operations, save, dry_run,
    ).await?;
    if !dry_run.unwrap_or(false) {
        for path in &guarded_paths {
            if let Ok(resolved) = crate::resolve_workspace_path(state, std::path::Path::new(path)) {
                crate::aspector::session::store::mark_file_read(&input.session_id, &resolved);
            }
        }
    }
    serde_json::to_string(&result).map_err(|e| e.to_string())
}
