//! Runtime tool definitions for `OpenAI` function-calling — the Rust source of
//! truth that replaces the TS `runtimeTools` array in `aiRuntimeTools.ts`.
//!
//! Returns a `Vec<serde_json::Value>` matching the `OpenAI` tools format so the
//! native turn loop can include them in the completion payload without crossing
//! the IPC bridge.

use serde_json::json;

/// Produce the full tool-definitions array filtered by mode and settings.
pub fn runtime_tool_definitions(agent_mode: &str, browser_enabled: bool) -> Vec<serde_json::Value> {
    let full_exec = matches!(agent_mode, "agent" | "automatic");
    // Plan mode is read-only for files but still presents plans; full-exec modes do too.
    let plan_capable = full_exec || matches!(agent_mode, "plan");

    let mut tools = Vec::with_capacity(48);

    // ── Context / search (always available) ──
    tools.push(tool(
        "FastContext",
        "Collect a compact workspace context packet.",
        &[req("query", "string", "Task or topic.")],
    ));
    tools.push(tool(
        "RepoMap",
        "Summarize workspace structure.",
        &[opt("maxFiles", "number", "Max files, default 80.")],
    ));
    tools.push(tool(
        "WorkspaceIndex",
        "Indexed snapshot of the workspace.",
        &[
            opt("maxFiles", "number", "Max per section, default 60."),
            opt("maxScan", "number", "Max scan."),
        ],
    ));
    tools.push(tool(
        "ActiveContext",
        "Current IDE state.",
        &[
            opt(
                "includeActiveText",
                "boolean",
                "Include active document text.",
            ),
            opt("maxOpenDocuments", "number", "Max docs."),
        ],
    ));
    tools.push(tool(
        "RulesContext",
        "Read project guidance files.",
        &[
            opt("query", "string", "Topic."),
            opt("maxFiles", "number", "Max files."),
        ],
    ));
    tools.push(tool(
        "DocsContext",
        "Local documentation and deps.",
        &[
            opt("query", "string", "Topic."),
            opt("maxFiles", "number", "Max files."),
        ],
    ));
    tools.push(tool(
        "MemoryContext",
        "Durable project memory.",
        &[
            opt("query", "string", "Topic."),
            opt("maxFiles", "number", "Max."),
            opt("maxSignals", "number", "Max."),
            opt("includeRecentChat", "boolean", "Include chat."),
        ],
    ));
    tools.push(tool(
        "ContextBudgeter",
        "Ranked context under a char budget.",
        &[
            req("query", "string", "Task."),
            opt("targetChars", "number", "Budget."),
            opt("includeActiveText", "boolean", ""),
            opt("includeOpenDocuments", "boolean", ""),
            opt("includeToolContext", "boolean", ""),
            opt("maxItems", "number", ""),
        ],
    ));
    tools.push(tool(
        "SemanticSearch",
        "Rank code locations by intent.",
        &[
            req("query", "string", "Topic."),
            opt("path", "string", "Path filter."),
            opt("maxResults", "number", "Max."),
        ],
    ));
    tools.push(tool(
        "Glob",
        "List files matching a pattern.",
        &[
            req("pattern", "string", "Pattern."),
            opt("maxResults", "number", "Max."),
        ],
    ));
    tools.push(tool(
        "Read",
        "Read a text file.",
        &[
            req("path", "string", "File path."),
            opt("maxBytes", "number", "Max bytes."),
        ],
    ));
    tools.push(tool(
        "InspectFile",
        "Structured preview of any file type.",
        &[
            req("path", "string", "File path."),
            opt("maxRows", "number", ""),
            opt("maxColumns", "number", ""),
            opt("maxBytes", "number", ""),
        ],
    ));
    tools.push(tool(
        "Grep",
        "Search text in workspace.",
        &[
            req("query", "string", "Search text."),
            opt("useRegex", "boolean", ""),
            opt("caseSensitive", "boolean", ""),
            opt("maxResults", "number", ""),
        ],
    ));
    tools.push(tool(
        "SymbolContext",
        "LSP symbols, hover, defs, refs.",
        &[
            opt("query", "string", "Symbol."),
            opt("path", "string", "File."),
            opt("line", "number", ""),
            opt("column", "number", ""),
            opt("maxResults", "number", ""),
        ],
    ));
    tools.push(tool(
        "RelatedFiles",
        "Find related files.",
        &[
            opt("path", "string", "Target."),
            opt("query", "string", "Topic."),
            opt("maxResults", "number", ""),
        ],
    ));
    tools.push(tool(
        "DiagnosticsContext",
        "IDE diagnostics.",
        &[opt("maxResults", "number", "")],
    ));
    tools.push(tool(
        "ReadLints",
        "Linter diagnostics with filters.",
        &[
            opt("path", "string", ""),
            opt("severity", "string", ""),
            opt("source", "string", ""),
            opt("maxResults", "number", ""),
        ],
    ));
    tools.push(tool("GitContext", "Git branch and changed files.", &[]));
    tools.push(tool(
        "ImpactAnalysis",
        "Blast radius for a change.",
        &[
            opt("path", "string", "Target."),
            opt("query", "string", "Change."),
            opt("maxResults", "number", ""),
        ],
    ));
    tools.push(tool(
        "ReviewDiff",
        "Quality gate on current diff.",
        &[
            opt("includePatch", "boolean", ""),
            opt("maxFindings", "number", ""),
        ],
    ));
    tools.push(tool(
        "SecretGuard",
        "Scan for secrets.",
        &[
            opt("text", "string", ""),
            opt("path", "string", ""),
            opt("includeDiff", "boolean", ""),
            opt("returnRedactedText", "boolean", ""),
            opt("maxFindings", "number", ""),
        ],
    ));
    tools.push(tool(
        "WebFetch",
        "Fetch a URL.",
        &[
            req("url", "string", "URL."),
            opt("maxBytes", "number", ""),
            opt("timeoutSecs", "number", ""),
            opt("allowPrivateHosts", "boolean", ""),
        ],
    ));
    tools.push(tool("TestHealth", "Run workspace tests.", &[]));
    tools.push(tool(
        "FailureAnalyzer",
        "Root-cause failing output.",
        &[
            opt("log", "string", "Raw output."),
            opt("includeTestHealth", "boolean", ""),
            opt("includeDiagnostics", "boolean", ""),
            opt("maxFindings", "number", ""),
        ],
    ));
    tools.push(tool(
        "AgentMessage",
        "Agent-to-agent coordination board.",
        &[
            opt("action", "string", "post or read."),
            opt("topic", "string", "Channel."),
            opt("content", "string", "Message."),
            opt("limit", "number", "Max read."),
        ],
    ));
    tools.push(tool(
        "AskUser",
        "Ask the user a question and wait for their answer. Use sparingly — only for genuine decisions you cannot resolve from evidence (product/UX choices, ambiguous scope, credentials). Provide 0–10 suggested `options`; the user can also type a custom answer unless allowCustom is false. Optionally render a self-contained HTML5 document via `htmlPreview` for visual choices (mockups, color/layout comparisons). In Automatic mode this returns immediately telling you to decide yourself — never blocks.",
        &[
            req("question", "string", "The question to ask."),
            opt("detail", "string", "Optional clarifying context shown under the question."),
            opt_arr("options", "0–10 suggested answers: strings or { label, description } objects."),
            opt("multiSelect", "boolean", "Allow selecting more than one option."),
            opt("allowCustom", "boolean", "Offer a free-form answer field (default true)."),
            opt("htmlPreview", "string", "Optional self-contained HTML5 document rendered in a sandboxed preview pane."),
        ],
    ));

    // ── Plan presentation (plan + agent + automatic) ──
    if plan_capable {
        tools.push(tool(
            "PresentPlan",
            "Present a structured, reviewable execution plan to the user. Renders an expandable plan card and pins the plan as the session goal + task list. In Plan/Agent mode the user presses Start to hand it to Agent execution (do not edit before that). In Automatic mode execution auto-starts. Prefer this over a plain prose checklist when proposing multi-step work.",
            &[
                req_arr("steps", "Ordered steps: strings or { title, detail, file } objects."),
                opt("title", "string", "Short plan title."),
                opt("summary", "string", "One-paragraph summary of the goal/approach."),
            ],
        ));
    }

    // ── Edit / execute / orchestrate (agent/automatic only; unknown modes fail safe) ──
    if full_exec {
        tools.push(tool(
            "Write",
            "Create or rewrite a file.",
            &[
                req("path", "string", "File path."),
                req("text", "string", "Contents."),
                opt("overwrite", "boolean", ""),
                opt("saveToDisk", "boolean", ""),
            ],
        ));
        tools.push(tool(
            "StrReplace",
            "Replace exact text in a file.",
            &[
                req("path", "string", ""),
                req("oldText", "string", ""),
                req("newText", "string", ""),
                opt("expectedReplacements", "number", ""),
                opt("saveToDisk", "boolean", ""),
            ],
        ));
        tools.push(tool(
            "PatchEngine",
            "Multi-file patch.",
            &[
                req_arr("operations", "Patch ops."),
                opt("saveToDisk", "boolean", ""),
                opt("dryRun", "boolean", ""),
            ],
        ));
        tools.push(tool(
            "Delete",
            "Delete a file.",
            &[req("path", "string", "")],
        ));
        tools.push(tool(
            "Checkpoint",
            "File snapshots.",
            &[
                req("action", "string", "create/list/diff/delete/restore."),
                opt("id", "string", ""),
                opt("label", "string", ""),
                opt("paths", "string", ""),
                opt("saveToDisk", "boolean", ""),
                opt("dryRun", "boolean", ""),
            ],
        ));
        tools.push(tool(
            "Shell",
            "Run a shell command.",
            &[
                req("command", "string", "Command."),
                opt("cwd", "string", ""),
                opt("timeoutSecs", "number", ""),
            ],
        ));
        tools.push(tool(
            "TerminalContext",
            "Terminal sessions and output.",
            &[
                opt("sessionId", "string", ""),
                opt("maxChars", "number", ""),
            ],
        ));
        tools.push(tool(
            "TerminalWrite",
            "Write to a terminal.",
            &[
                req("data", "string", "Text."),
                opt("sessionId", "string", ""),
            ],
        ));
        tools.push(tool(
            "Goal",
            "Pin session goal and progress.",
            &[
                opt("goal", "string", ""),
                opt("progress", "number", ""),
                opt("status", "string", ""),
                opt("summary", "string", ""),
            ],
        ));
        tools.push(tool(
            "TodoWrite",
            "Replace session task list.",
            &[req_arr("todos", "Task list.")],
        ));
        tools.push(tool(
            "Task",
            "Spawn a subagent.",
            &[
                req("description", "string", "Title."),
                req("prompt", "string", "Task."),
                opt("subagent_type", "string", ""),
                opt("model", "string", ""),
                opt("resume", "string", ""),
            ],
        ));
    }

    // ── Browser (requires agent_browser_enabled) ──
    if browser_enabled {
        let browser_write = full_exec;
        let browser_read = true;
        if browser_read {
            tools.push(tool("BrowserStatus", "Check agent-browser.", &[]));
            tools.push(tool(
                "BrowserSnapshot",
                "Accessibility tree with refs.",
                &[
                    opt("interactive", "boolean", ""),
                    opt("compact", "boolean", ""),
                    opt("depth", "number", ""),
                    opt("selector", "string", ""),
                    opt("includeUrls", "boolean", ""),
                ],
            ));
            tools.push(tool(
                "BrowserHelp",
                "agent-browser help.",
                &[
                    opt("topic", "string", ""),
                    opt("skill", "string", ""),
                    opt("allSkills", "boolean", ""),
                ],
            ));
            // `fix` triggers a side-effecting repair, so expose it only when execute-capable;
            // diagnostics-only params stay available in read-only modes.
            let mut doctor_params = vec![
                opt("offline", "boolean", ""),
                opt("quick", "boolean", ""),
            ];
            if browser_write {
                doctor_params.push(opt("fix", "boolean", ""));
            }
            tools.push(tool("BrowserDoctor", "Diagnostics.", &doctor_params));
        }
        if browser_write {
            tools.push(tool(
                "BrowserOpen",
                "Open browser session.",
                &[opt("url", "string", ""), opt("headed", "boolean", "")],
            ));
            tools.push(tool(
                "BrowserAct",
                "Browser action.",
                &[opt("command", "string", "")],
            ));
            tools.push(tool(
                "BrowserScreenshot",
                "Screenshot.",
                &[
                    opt("path", "string", ""),
                    opt("annotate", "boolean", ""),
                    opt("fullPage", "boolean", ""),
                    opt("attachVision", "boolean", ""),
                ],
            ));
            tools.push(tool(
                "BrowserClose",
                "Close browser.",
                &[opt("all", "boolean", "")],
            ));
            tools.push(tool(
                "BrowserChat",
                "Natural-language browser.",
                &[
                    req("instruction", "string", ""),
                    opt("quiet", "boolean", ""),
                ],
            ));
            tools.push(tool(
                "BrowserDashboard",
                "Dashboard.",
                &[
                    opt("action", "string", ""),
                    opt("port", "number", ""),
                    opt("openInBrowser", "boolean", ""),
                ],
            ));
            tools.push(tool(
                "BrowserInstall",
                "Install agent-browser.",
                &[opt("withDeps", "boolean", "")],
            ));
            tools.push(tool(
                "BrowserInvoke",
                "Raw CLI.",
                &[req_arr("args", "CLI args.")],
            ));
        }
    }

    tools
}

// ── Builders ──

struct Param {
    name: &'static str,
    kind: &'static str,
    desc: &'static str,
    required: bool,
    is_array: bool,
}

const fn req(name: &'static str, kind: &'static str, desc: &'static str) -> Param {
    Param {
        name,
        kind,
        desc,
        required: true,
        is_array: false,
    }
}
const fn opt(name: &'static str, kind: &'static str, desc: &'static str) -> Param {
    Param {
        name,
        kind,
        desc,
        required: false,
        is_array: false,
    }
}
const fn req_arr(name: &'static str, desc: &'static str) -> Param {
    Param {
        name,
        kind: "array",
        desc,
        required: true,
        is_array: true,
    }
}
const fn opt_arr(name: &'static str, desc: &'static str) -> Param {
    Param {
        name,
        kind: "array",
        desc,
        required: false,
        is_array: true,
    }
}

fn tool(name: &str, description: &str, params: &[Param]) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    let mut required = Vec::new();
    for p in params {
        let schema = if p.is_array {
            json!({ "type": "array", "description": p.desc, "items": { "type": "object" } })
        } else {
            json!({ "type": p.kind, "description": p.desc })
        };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_defs_agent_mode_has_write_tools() {
        let defs = runtime_tool_definitions("agent", false);
        let names: Vec<String> = defs
            .iter()
            .filter_map(|t| t.get("function")?.get("name")?.as_str().map(str::to_string))
            .collect();
        assert!(names.contains(&"Write".to_string()));
        assert!(names.contains(&"Shell".to_string()));
        assert!(names.contains(&"SemanticSearch".to_string()));
    }

    #[test]
    fn tool_defs_plan_mode_blocks_write() {
        let defs = runtime_tool_definitions("plan", false);
        let names: Vec<String> = defs
            .iter()
            .filter_map(|t| t.get("function")?.get("name")?.as_str().map(str::to_string))
            .collect();
        assert!(!names.contains(&"Write".to_string()));
        assert!(!names.contains(&"Shell".to_string()));
        assert!(names.contains(&"SemanticSearch".to_string()));
    }

    #[test]
    fn tool_defs_browser_disabled() {
        let defs = runtime_tool_definitions("agent", false);
        let has_browser_open = defs
            .iter()
            .filter_map(|t| t.get("function")?.get("name")?.as_str())
            .any(|name| name == "BrowserOpen");
        assert!(!has_browser_open);
    }

    #[test]
    fn tool_defs_browser_enabled() {
        let defs = runtime_tool_definitions("agent", true);
        let names: Vec<String> = defs
            .iter()
            .filter_map(|t| t.get("function")?.get("name")?.as_str().map(str::to_string))
            .collect();
        assert!(names.contains(&"BrowserOpen".to_string()));
        assert!(names.contains(&"BrowserSnapshot".to_string()));
    }
}
