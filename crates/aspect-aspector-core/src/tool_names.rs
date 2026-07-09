use std::collections::HashSet;

fn normalize_tool_name(name: &str) -> String {
    let mut normalized = name.to_lowercase();
    if let Some(stripped) = normalized.strip_prefix("ai_") {
        normalized = stripped.to_string();
    }
    normalized
}

fn harness_alias_hint(normalized: &str) -> Option<&'static str> {
    let hint = match normalized {
        "search" | "semanticsearch" => "SemanticSearch",                             // not harness — AI SDK name
        "file_search" | "grep" => "Grep",
        "read_file" | "browser" | "fetch" => "Read",                                 // read_file is the old TS name
        "glob" | "list_files" => "Glob",
        "symbol_context" | "symbolsearch" => "SymbolContext",
        "create_file" | "write_file" | "write" => "Write",
        "str_replace_editor" | "edit_file" | "str_replace" => "StrReplace",
        "patch_engine" | "patch" | "apply_patch" => "PatchEngine",
        "delete_file" | "delete" => "Delete",
        "run_terminal" | "terminal" | "shell" => "Shell",
        "run_in_terminal" | "execute_command" => "Shell",
        "web_search" | "web_fetch" => "WebFetch",
        "web_research" | "research" => "WebResearch",
        "multi_web_research" | "multi_research" => "MultiWebResearch",
        "related_code" | "related" => "RelatedFiles",
        "diagnostics" | "problems" => "DiagnosticsContext",
        "file_inspect" | "inspect" => "InspectFile",
        "test_health" | "tests" => "TestHealth",
        "code_graph" | "codebase_graph" => "CodeGraph",
        "context_budget" | "budget" => "ContextBudgeter",
        "secret_guard" | "secrets" => "SecretGuard",
        "goal" | "set_goal" => "Goal",
        "agent_message" | "message_user" | "tell_user" => "AgentMessage",
        "subagent" | "task" | "agent" => "Task",
        "checkpoint" | "snapshot" => "Checkpoint",
        "rules_context" | "rules" => "RulesContext",
        "docs_context" | "docs" => "DocsContext",
        "memory_context" | "memory" => "MemoryContext",
        "active_context" | "active" => "ActiveContext",
        "fast_context" | "fast" => "FastContext",
        "git_context" | "git" => "GitContext",
        "workspace_index" | "workspace" => "WorkspaceIndex",
        "repo_map" | "map" => "RepoMap",
        "impact_analysis" | "impact" => "ImpactAnalysis",
        "review_diff" | "diff_review" => "ReviewDiff",
        "failure_analyzer" | "analyze_failure" => "FailureAnalyzer",
        "ask_user" | "ask" | "question" => "AskUser",
        "present_plan" | "plan" => "PresentPlan",
        "todo" | "todos" | "todo_write" => "TodoWrite",
        "terminal_context" => "TerminalContext",
        "terminal_write" | "write_terminal" => "TerminalWrite",
        "ssh_connect" | "ssh" => "SshConnect",
        "ssh_exec" | "remote_exec" => "SshExec",
        "ssh_transfer" | "transfer" => "SshTransfer",
        "ssh_list" | "list_remote" => "SshList",
        "ssh_disconnect" | "disconnect" => "SshDisconnect",
        "browser_status" | "status" => "BrowserStatus",
        "browser_open" | "open" => "BrowserOpen",
        "browser_act" | "act" => "BrowserAct",
        "browser_snapshot" => "BrowserSnapshot",
        "browser_screenshot" | "screenshot" => "BrowserScreenshot",
        "browser_close" | "close" => "BrowserClose",
        "browser_chat" | "chat" => "BrowserChat",
        "browser_dashboard" | "dashboard" => "BrowserDashboard",
        "browser_install" | "install" => "BrowserInstall",
        "browser_help" | "help" => "BrowserHelp",
        "browser_doctor" | "doctor" => "BrowserDoctor",
        "browser_invoke" | "invoke" => "BrowserInvoke",
        "codebase_query" | "query" => "CodeGraphQuery",
        "codebase_callers" | "callers" => "CodeGraphCallers",
        "codebase_callees" | "callees" => "CodeGraphCallees",
        "codebase_explain" | "explain" => "CodeGraphExplain",
        "codebase_community" | "community" | "god_nodes" => "CodeGraphCommunity",
        "remember_memory" | "remember" => "RememberMemory",
        "recall_memory" | "recall" => "RecallMemory",
        "relate_memories" | "relate" => "RelateMemories",
        "list_skills" | "skills" => "ListSkills",
        "use_skill" | "skill" => "UseSkill",
        "mcp" | "mcp_call" => "Mcp",
        "mcp_manage" | "manage_mcp" => "McpManage",
        "task_wait" | "wait_task" => "TaskWait",
        _ => return None,
    };
    Some(hint)
}

pub fn tool_names_from_defs(tools: &[serde_json::Value]) -> HashSet<String> {
    tools
        .iter()
        .filter_map(|t| {
            t.get("function")
                .and_then(|f| f.get("name"))
                .or_else(|| t.get("name"))
                .and_then(|n| n.as_str())
                .map(|n| n.to_string())
        })
        .collect()
}

pub fn closest_tool_names(target: &str, candidates: &HashSet<String>) -> Vec<String> {
    let normalized = normalize_tool_name(target);
    let hint = harness_alias_hint(&normalized).map(|s| s.to_string());

    let mut scored: Vec<(i32, &String)> = candidates
        .iter()
        .filter_map(|c| {
            let cn = normalize_tool_name(c);
            if cn == normalized {
                Some((100, c))
            } else if hint.as_ref().is_some_and(|h| cn == h.to_lowercase()) {
                Some((90, c))
            } else if c.to_lowercase() == target.to_lowercase() {
                Some((80, c))
            } else {
                let dist = edit_distance(&normalized, &cn);
                let len_cmp = normalized.len().max(cn.len());
                let sim = if len_cmp == 0 {
                    0.0
                } else {
                    1.0 - (dist as f64 / len_cmp as f64)
                };
                if sim >= 0.5 {
                    Some(((sim * 50.0) as i32, c))
                } else {
                    None
                }
            }
        })
        .collect();

    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.truncate(3);
    scored.into_iter().map(|(_, c)| c.clone()).collect()
}

pub fn tool_rejection_error(
    name: &str,
    agent_mode: &str,
    browser_enabled: bool,
    allowed: &HashSet<String>,
) -> String {
    let mode_hint = if agent_mode == "code" || agent_mode == "full" {
        " Note: only Plan/AskUser and agent-mode tools are available in Plan/Ask mode."
    } else {
        ""
    };
    let browser_hint = if !browser_enabled && name.starts_with("Browser") {
        " Enable the agent-browser in Settings → AI → Browser to use Browser tools."
    } else {
        ""
    };

    let closest = closest_tool_names(name, allowed);
    let did_you_mean = if closest.is_empty() {
        String::new()
    } else {
        format!(" Did you mean: {}?", closest.join(", "))
    };

    format!(
        "Tool '{name}' is not available in {agent_mode} mode.{mode_hint}{browser_hint}{did_you_mean}"
    )
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a_len = a.len();
    let b_len = b.len();
    if a_len == 0 {
        return b_len;
    }
    if b_len == 0 {
        return a_len;
    }
    let mut prev: Vec<usize> = (0..=b_len).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = vec![i + 1];
        for (j, cb) in b.chars().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let val = (current[j] + 1)
                .min(prev[j + 1] + 1)
                .min(prev[j] + cost);
            current.push(val);
        }
        prev = current;
    }
    prev[b_len]
}
