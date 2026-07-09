use std::collections::HashSet;

use aspect_ai_core::*;

use super::tool_mcp::{execute_mcp_proxy, execute_mcp_manage};
use super::tool_files::{
    execute_read, execute_glob, execute_symbol_context,
    execute_write, execute_str_replace, execute_delete, execute_inspect_file, execute_patch_engine,
};
use super::tool_shell::{execute_shell, execute_shell_output};
use super::tool_search::{
    execute_semantic_search, execute_related_files, execute_repo_map, execute_workspace_index,
    execute_grep, execute_git_context,
};
use super::tool_web::{execute_web_fetch, execute_web_research, execute_multi_web_research};
use super::tool_ssh::{execute_ssh_connect, execute_ssh_exec, execute_ssh_transfer, execute_ssh_list, execute_ssh_disconnect};
use super::tool_browser::execute_browser_tool;
use super::tool_diag::{
    execute_diagnostics_context, execute_read_lints, execute_test_health,
    execute_failure_analyzer, execute_impact_analysis, execute_review_diff,
    execute_terminal_context, execute_terminal_write,
};
use super::tool_session::{
    execute_goal, execute_todo_write, execute_agent_message, execute_ask_user,
    execute_present_plan, execute_active_context, execute_secret_guard,
};
use super::tool_sources::{
    execute_rules_context, execute_docs_context, execute_memory_context,
    execute_list_skills, execute_use_skill, execute_recall_memory,
    execute_relate_memories, execute_remember_memory, execute_fast_context,
    execute_context_budgeter,
};
use super::tool_task::{execute_task, execute_task_wait};
use super::tool_codegraph::{
    execute_code_graph_definition, execute_code_graph_callers, execute_code_graph_callees,
    execute_code_graph_explain, execute_code_graph_overview,
};

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
pub async fn execute_tool(
    app: &tauri::AppHandle,
    state: &tauri::State<'_, crate::SharedState>,
    input: &TurnInput,
    turn_id: &str,
    interactive: bool,
    tc: &ParsedToolCall,
    allowed_tool_names: &HashSet<String>,
) -> Result<String, String> {
    if !allowed_tool_names.is_empty() && !allowed_tool_names.contains(&tc.name) {
        return Err(tool_rejection_error(
            &tc.name, &input.agent_mode, input.agent_browser_enabled, allowed_tool_names,
        ));
    }

    let is_automatic = input.agent_mode == "automatic";

    let _file_edit_guard = if matches!(
        tc.name.as_str(),
        "Write" | "StrReplace" | "PatchEngine" | "Delete" | "Checkpoint"
    ) {
        Some(file_edit_lock().lock().await)
    } else {
        None
    };

    match tc.name.as_str() {
        name if name.starts_with("mcp__") => execute_mcp_proxy(app, state, input, turn_id, interactive, tc, is_automatic).await,
        "McpManage" => execute_mcp_manage(app, state, input, turn_id, interactive, tc, is_automatic).await,
        "SemanticSearch" => execute_semantic_search(state, tc).await,
        "RelatedFiles" => execute_related_files(state, input, tc).await,
        "RepoMap" => execute_repo_map(state, tc).await,
        "WorkspaceIndex" => execute_workspace_index(state, tc).await,
        "Shell" => execute_shell(app, state, input, turn_id, interactive, tc, is_automatic).await,
        "ShellOutput" => execute_shell_output(app, state, input, turn_id, tc, interactive).await,
        "Read" => execute_read(state, input, tc).await,
        "Glob" => execute_glob(state, tc).await,
        "SymbolContext" => execute_symbol_context(state, tc).await,
        "Write" => execute_write(app, state, input, turn_id, interactive, tc, is_automatic).await,
        "StrReplace" => execute_str_replace(app, state, input, turn_id, interactive, tc, is_automatic).await,
        "Delete" => execute_delete(app, state, input, turn_id, interactive, tc, is_automatic).await,
        "Grep" => execute_grep(state, tc).await,
        "GitContext" => execute_git_context(state).await,
        "DiagnosticsContext" => execute_diagnostics_context(state, tc).await,
        "ReadLints" => execute_read_lints(state, tc).await,
        "AgentMessage" => execute_agent_message(input, turn_id, interactive, tc).await,
        "PatchEngine" => execute_patch_engine(app, state, input, turn_id, interactive, tc, is_automatic).await,
        "InspectFile" => execute_inspect_file(state, input, tc).await,
        "WebFetch" => execute_web_fetch(tc).await,
        "WebResearch" => execute_web_research(state, tc).await,
        "MultiWebResearch" => execute_multi_web_research(state, tc).await,
        "SshConnect" => execute_ssh_connect(app, turn_id, interactive, tc, is_automatic, state).await,
        "SshExec" => execute_ssh_exec(app, turn_id, interactive, tc, is_automatic, state).await,
        "SshTransfer" => execute_ssh_transfer(app, turn_id, interactive, tc, is_automatic, state).await,
        "SshList" => execute_ssh_list(state).await,
        "SshDisconnect" => execute_ssh_disconnect(state, tc).await,
        "TestHealth" => execute_test_health(app, state, turn_id, interactive, tc, is_automatic).await,
        name if name.starts_with("Browser") => execute_browser_tool(app, state, input, turn_id, interactive, tc).await,
        "Goal" => execute_goal(input, tc),
        "TodoWrite" => execute_todo_write(input, tc),
        "AskUser" => execute_ask_user(app, input, turn_id, interactive, tc).await,
        "PresentPlan" => execute_present_plan(app, input, turn_id, tc).await,
        "ActiveContext" => execute_active_context(state, input, tc),
        "SecretGuard" => execute_secret_guard(tc),
        "RulesContext" => execute_rules_context(state, tc).await,
        "DocsContext" => execute_docs_context(state, tc).await,
        "MemoryContext" => execute_memory_context(state, tc).await,
        "RecallMemory" => execute_recall_memory(app, state, input, tc).await,
        "RelateMemories" => execute_relate_memories(app, state, tc).await,
        "RememberMemory" => execute_remember_memory(app, state, input, tc).await,
        "ListSkills" => execute_list_skills(app, state, tc),
        "UseSkill" => execute_use_skill(app, state, tc).await,
        "FastContext" => execute_fast_context(app, state, input, tc).await,
        "ReviewDiff" => execute_review_diff(app, state).await,
        "FailureAnalyzer" => execute_failure_analyzer(app, state, turn_id, interactive, tc, is_automatic).await,
        "ImpactAnalysis" => execute_impact_analysis(state, input, tc).await,
        "TerminalContext" => execute_terminal_context(input, tc).await,
        "TerminalWrite" => execute_terminal_write(app, state, turn_id, interactive, tc, is_automatic).await,
        "Task" => execute_task(app, state, input, turn_id, tc, interactive).await,
        "TaskWait" => execute_task_wait(input, turn_id, tc).await,
        "ContextBudgeter" => execute_context_budgeter(app, state, input, tc).await,
        "Checkpoint" => execute_mcp_manage(app, state, input, turn_id, interactive, tc, is_automatic).await, // TODO: extract
        name if name.starts_with("CodeGraph") => execute_code_graph_tool(state, tc).await,
        other => Err(format!("Unknown tool: {other}")),
    }
}

async fn execute_code_graph_tool(state: &tauri::State<'_, crate::SharedState>, tc: &ParsedToolCall) -> Result<String, String> {
    match tc.name.as_str() {
        "CodeGraphDefinition" => execute_code_graph_definition(state, tc).await,
        "CodeGraphCallers" => execute_code_graph_callers(state, tc).await,
        "CodeGraphCallees" => execute_code_graph_callees(state, tc).await,
        "CodeGraphExplain" => execute_code_graph_explain(state, tc).await,
        "CodeGraphOverview" => execute_code_graph_overview(state, tc).await,
        other => Err(format!("Unknown CodeGraph tool: {other}")),
    }
}
