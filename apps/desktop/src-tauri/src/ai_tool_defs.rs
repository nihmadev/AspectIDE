//! Runtime tool definitions for `OpenAI` function-calling — the Rust source of
//! truth that replaces the TS `runtimeTools` array in `aiRuntimeTools.ts`.
//!
//! Returns a `Vec<serde_json::Value>` matching the `OpenAI` tools format so the
//! native turn loop can include them in the completion payload without crossing
//! the IPC bridge.

use serde_json::json;

/// Max result bounds reused across many tools.
const MAX_RESULTS_DEFAULT: i64 = 500;
const MAX_RESULTS_SYMBOL: i64 = 300;
const MAX_RESULTS_FINDINGS: i64 = 500;
/// Max byte limits for reading content.
const MAX_BYTES_DEFAULT: i64 = 10_485_760;
/// Timeout bound (1..600 seconds).
const TIMEOUT_MIN: i64 = 1;
const TIMEOUT_MAX: i64 = 600;
/// Port range.
const PORT_MIN: i64 = 1;
const PORT_MAX: i64 = 65535;
/// Line/column bounds.
const LINE_MAX: i64 = 2_000_000;
const COLUMN_MAX: i64 = 10_000;
/// Progress bounds.
const PROGRESS_MIN: i64 = 0;
const PROGRESS_MAX: i64 = 100;
/// Expected replacement count.
const REPLACEMENT_MIN: i64 = 1;
const REPLACEMENT_MAX: i64 = 1000;

/// Returns a JSON Schema `anyOf` for PresentPlan.steps: a string or object.
fn steps_item_schema() -> serde_json::Value {
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

/// Returns a JSON Schema `anyOf` for PresentPlan.alternatives: a string or decision object.
fn alternatives_item_schema() -> serde_json::Value {
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

/// Returns a JSON Schema `anyOf` for AskUser.options: a string or label+description object.
fn ask_user_options_item_schema() -> serde_json::Value {
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

/// Returns the JSON Schema for one TodoWrite.todos item — an object with a required
/// `content` plus optional id/status/priority/notes. MUST match the handler in
/// `ai_turn.rs`, which reads objects (not bare strings).
fn todo_item_schema() -> serde_json::Value {
    json!({
        "type": "object",
        "properties": {
            "content": { "type": "string", "description": "The task text" },
            "id": { "type": "string", "description": "Stable id (optional; auto-assigned if omitted)" },
            "status": { "type": "string", "enum": ["pending", "in_progress", "completed"], "description": "Task status (default pending)" },
            "priority": { "type": "string", "enum": ["low", "medium", "high"], "description": "Task priority (default medium)" },
            "notes": { "type": "string", "description": "Optional notes" }
        },
        "required": ["content"],
        "additionalProperties": false
    })
}

/// Returns a full nested JSON Schema for PatchEngine.operations array items.
fn patch_operation_schema() -> serde_json::Value {
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
                "minimum": REPLACEMENT_MIN, "maximum": REPLACEMENT_MAX
            },
            "overwrite": { "type": "boolean", "description": "Allow overwrite existing" }
        },
        "required": ["action", "path"],
        "additionalProperties": false
    })
}

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
        &[opt_int(
            "maxFiles",
            "Max files, default 80.",
            1,
            MAX_RESULTS_DEFAULT,
        )],
    ));
    tools.push(tool(
        "WorkspaceIndex",
        "Indexed snapshot of the workspace. When the result is truncated, the language/directory/largest aggregates are a lexicographically-first sample (see aggregatesNote), not the whole project.",
        &[
            opt_int(
                "maxFiles",
                "Max per section, default 60.",
                1,
                MAX_RESULTS_DEFAULT,
            ),
            opt_int(
                "maxScan",
                "Max files to scan before aggregating (default 5000; clamped to 500–20000).",
                500,
                20_000,
            ),
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
            opt_int("maxOpenDocuments", "Max docs.", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "RulesContext",
        "Read project guidance files.",
        &[
            opt("query", "string", "Topic."),
            opt_int("maxFiles", "Max files.", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "DocsContext",
        "Local documentation and deps.",
        &[
            opt("query", "string", "Topic."),
            opt_int("maxFiles", "Max files.", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "MemoryContext",
        "Durable project memory.",
        &[
            opt("query", "string", "Topic."),
            opt_int("maxFiles", "Max files.", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "RecallMemory",
        "Search this project's durable memory — facts, decisions, and conventions saved across sessions. Prefer this over re-deriving things you may have learned before.",
        &[
            req("query", "string", "What to recall."),
            opt("category", "string", "Restrict to a category (core, episodic, semantic, procedural, or custom)."),
            opt_int("limit", "Max results, default 8.", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "RememberMemory",
        "Save a durable, project-scoped memory for future sessions (a stable fact, decision, or convention). Keep each memory one concise, self-contained statement.",
        &[
            req("content", "string", "The fact to remember, as one self-contained sentence."),
            opt("category", "string", "core | episodic | semantic | procedural | custom (default semantic)."),
            opt("importance", "number", "0..1 relevance weight (default 0.5; out-of-range values are clamped)."),
            opt("pinned", "boolean", "Pin so it always surfaces first."),
        ],
    ));
    tools.push(tool(
        "ListSkills",
        "List available skills — reusable, vetted instruction modules for recurring tasks. Check here before improvising a procedure; an existing skill is more reliable.",
        &[
            opt("query", "string", "Rank skills by relevance to this topic; omit to list all."),
            opt_int("limit", "Max skills, default 20.", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "UseSkill",
        "Fetch a skill's full instructions by slug (from ListSkills) and follow them for the current task.",
        &[req("slug", "string", "The skill slug to load.")],
    ));
    tools.push(tool(
        "ContextBudgeter",
        "Ranked context under a char budget.",
        &[
            req("query", "string", "Task."),
            opt_int("targetChars", "Target character budget for the packet (default ~12000; clamped to 2000..22000).", 2_000, 22_000),
            opt("includeActiveText", "boolean", "Include a trimmed excerpt from the active document. Default false."),
            opt("includeOpenDocuments", "boolean", "Include open editor tabs and dirty-file excerpts. Default false."),
            opt("includeToolContext", "boolean", "Gather ranked context from read-only tools (rules, memory, related files, diagnostics). Default true."),
            opt_int("maxItems", "Hard cap on packet items. Default 28.", 4, 80),
        ],
    ));
    tools.push(tool(
        "SemanticSearch",
        "Rank code locations by intent. The `path` filter is a case-insensitive SUBSTRING match on the workspace-relative path (not a glob) — pass a directory fragment like `src/auth`, not `src/**/*.ts`.",
        &[
            req("query", "string", "Topic."),
            opt("path", "string", "Case-insensitive substring of the file path to restrict results (e.g. `src/auth`); NOT a glob."),
            opt_int("maxResults", "Max ranked results.", 1, MAX_RESULTS_DEFAULT),
            opt_int("maxFiles", "Max workspace files scanned as candidates (default 5000).", 1, 20_000),
        ],
    ));
    tools.push(tool(
        "CodeGraphDefinition",
        "Find definition(s) of a symbol in the code graph. Only one symbol at a time.",
        &[req("symbol", "string", "Exact or partial symbol name.")],
    ));
    tools.push(tool(
        "CodeGraphCallers",
        "List all callers of a symbol (who depends on it).",
        &[req("symbol", "string", "Symbol name.")],
    ));
    tools.push(tool(
        "CodeGraphCallees",
        "List all symbols a given symbol calls.",
        &[req("symbol", "string", "Symbol name.")],
    ));
    tools.push(tool(
        "CodeGraphExplain",
        "Deep info about a symbol: degree, neighbors, and connections sorted by relevance.",
        &[req("symbol", "string", "Symbol name.")],
    ));
    tools.push(tool(
        "CodeGraphOverview",
        "Overview of the code graph: total nodes, edges, community count, and top 10 god nodes (most connected symbols).",
        &[],
    ));
    tools.push(tool(
        "Glob",
        "List files matching a glob pattern (`*`, `?`, `[...]`, `{a,b}`). NOTE: `*` matches across `/`, so `src/*.ts` also finds nested files (no `**` needed). Results EXCLUDE .gitignored paths (dist/, build/, node_modules/, …) — those return no match. A plain string with no wildcard is treated as a case-insensitive path substring.",
        &[
            req("pattern", "string", "Glob pattern, or a plain path substring."),
            opt_int("maxResults", "Max.", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "Read",
        "Read a text file. To page a large file instead of re-reading a truncated head, pass startLine (1-based) and maxLines; the response reports totalLines so you can request the next window.",
        &[
            req("path", "string", "File path."),
            opt_int("maxBytes", "Max bytes.", 1, MAX_BYTES_DEFAULT),
            opt_int(
                "startLine",
                "1-based first line to return. Combine with maxLines to page a large file.",
                1,
                10_000_000,
            ),
            opt_int(
                "maxLines",
                "Maximum number of lines to return starting at startLine.",
                1,
                1_000_000,
            ),
        ],
    ));
    tools.push(tool(
        "InspectFile",
        "Structured preview of any file type.",
        &[
            req("path", "string", "File path."),
            opt_int("maxRows", "", 1, 100_000),
            opt_int("maxColumns", "", 1, 1_000),
            opt_int("maxBytes", "", 1, MAX_BYTES_DEFAULT),
        ],
    ));
    tools.push(tool(
        "Grep",
        "Search text in the workspace. The query is a LITERAL string by default — set useRegex:true to use a regular expression, including alternation like `foo|bar|baz` (which matches 0 results as a literal).",
        &[
            req("query", "string", "Search text (literal unless useRegex is true)."),
            opt("useRegex", "boolean", "Treat query as a regex (needed for `a|b` alternation, anchors, character classes)."),
            opt("caseSensitive", "boolean", ""),
            opt_int("maxResults", "", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "SymbolContext",
        "LSP symbols, hover, defs, refs.",
        &[
            opt("query", "string", "Symbol."),
            opt("path", "string", "File."),
            opt_int(
                "line",
                "0-based line number (first line is 0), matching the editor's LSP coordinates.",
                0,
                LINE_MAX,
            ),
            opt_int(
                "column",
                "0-based UTF-16 column offset within the line (first column is 0).",
                0,
                COLUMN_MAX,
            ),
            opt_int(
                "maxResults",
                "Max results per symbol list.",
                1,
                MAX_RESULTS_SYMBOL,
            ),
        ],
    ));
    tools.push(tool(
        "RelatedFiles",
        "Find related files.",
        &[
            opt("path", "string", "Target."),
            opt("query", "string", "Topic."),
            opt_int("maxResults", "", 1, MAX_RESULTS_DEFAULT),
            opt_int(
                "maxFiles",
                "Max workspace files scanned as candidates (default 5000).",
                1,
                20_000,
            ),
        ],
    ));
    tools.push(tool(
        "DiagnosticsContext",
        "IDE diagnostics.",
        &[opt_int("maxResults", "", 1, MAX_RESULTS_DEFAULT)],
    ));
    tools.push(tool(
        "ReadLints",
        "Linter diagnostics with filters.",
        &[
            opt("path", "string", ""),
            opt("severity", "string", ""),
            opt("source", "string", ""),
            opt_int("maxResults", "", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool("GitContext", "Git branch and changed files.", &[]));
    tools.push(tool(
        "ImpactAnalysis",
        "Blast radius for a change.",
        &[
            opt("path", "string", "Target."),
            opt("query", "string", "Change."),
            opt_int("maxResults", "", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "ReviewDiff",
        "Quality gate on current diff.",
        &[
            opt("includePatch", "boolean", ""),
            opt_int("maxFindings", "", 1, MAX_RESULTS_FINDINGS),
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
            opt_int("maxFindings", "", 1, MAX_RESULTS_FINDINGS),
        ],
    ));
    tools.push(tool(
        "WebFetch",
        "Fetch ONE known URL's content. For an open-ended question across the web, use WebResearch instead.",
        &[
            req("url", "string", "URL."),
            opt_int("maxBytes", "Max response bytes to read (default 250000; clamped to 1024..1000000).", 1_024, 1_000_000),
            opt_int("timeoutSecs", "Request timeout in seconds (default 20, max 60).", TIMEOUT_MIN, 60),
        ],
    ));
    tools.push(tool(
        "WebResearch",
        "Web research: searches the web (SearxNG or DuckDuckGo), fetches the top pages, reranks by relevance, and returns cited sources with extracted content. Two modes via `depth`: standard (fast, one query, ~6-8 sources) and deep (expands the query into several variants, merges all engines, follows one hop of in-page links, and returns more domain-diverse sources — slower, ~30-60s, best for hard/open questions). Cite sources as [1], [2]. Prefer over WebFetch when you don't already have the exact URL.",
        &[
            req("query", "string", "The research question or topic."),
            opt("focus", "string", "web | academic | news | social | video | code (default web)."),
            opt("depth", "string", "standard (default, fast) | deep (query expansion + multi-engine + in-page link crawl + more diverse sources; slower)."),
            opt_int("maxSources", "How many ranked sources to return (default 6; up to 8 standard / 15 deep).", 1, 15),
        ],
    ));
    tools.push(tool(
        "SshList",
        "List active SSH sessions and the hosts defined in ~/.ssh/config, plus whether the OpenSSH client is available. Read-only discovery of hosts; opening a session (SshConnect) is only available in Agent/Automatic mode.",
        &[],
    ));
    tools.push(tool("TestHealth", "Run workspace tests.", &[]));
    tools.push(tool(
        "FailureAnalyzer",
        "Root-cause failing output.",
        &[
            opt("log", "string", "Raw output."),
            opt("includeTestHealth", "boolean", ""),
            opt("includeDiagnostics", "boolean", ""),
            opt_int("maxFindings", "", 1, MAX_RESULTS_FINDINGS),
        ],
    ));
    tools.push(tool(
        "AgentMessage",
        "Agent-to-agent coordination board.",
        &[
            opt("action", "string", "post or read."),
            opt("topic", "string", "Channel."),
            opt("content", "string", "Message."),
            opt_int("limit", "Max read.", 1, MAX_RESULTS_DEFAULT),
        ],
    ));
    tools.push(tool(
        "AskUser",
        "Ask the user a question and wait for their answer. Use sparingly — only for genuine decisions you cannot resolve from evidence (product/UX choices, ambiguous scope, credentials). Provide 0–10 suggested `options`; the user can also type a custom answer unless allowCustom is false. Optionally render a self-contained HTML5 document via `htmlPreview` for visual choices (mockups, color/layout comparisons). In Automatic mode this returns immediately telling you to decide yourself — never blocks.",
        &[
            req("question", "string", "The question to ask."),
            opt("detail", "string", "Optional clarifying context shown under the question."),
            opt_arr_items("options", "0–10 suggested answers: strings or { label, description } objects.", ask_user_options_item_schema()),
            opt("multiSelect", "boolean", "Allow selecting more than one option."),
            opt("allowCustom", "boolean", "Offer a free-form answer field (default true)."),
            opt("htmlPreview", "string", "Optional self-contained HTML5 document rendered in a sandboxed preview pane."),
        ],
    ));

    // ── Plan presentation (plan + agent + automatic) ──
    if plan_capable {
        tools.push(tool(
            "PresentPlan",
            "Present a structured, reviewable execution plan to the user. Renders an expandable plan card and pins the plan as the session goal + task list. In Plan/Agent mode the user presses Start to hand it to Agent execution (do not edit before that). In Automatic mode execution auto-starts. Scale the plan to the task's complexity and risk — it is NOT a flat list of phases. A strong plan covers five reasoning phases (a deterministic quality gate scores them and coaches whatever is missing): (1) DECOMPOSE into concrete file-level `steps` (each = a specific action on a named file/module with its acceptance check, never vague labels like 'implement business logic'); (2) ALTERNATIVES — in `alternatives`, name the key decision(s): the approach you chose and why it wins over the option you rejected (the tradeoff); (3) CRITIQUE — in `risks`, the failure modes and hidden assumptions of the riskiest step (what breaks, under what input/timing); (4) SYNTHESIS — the chosen path's rationale in `summary`; (5) VERIFY — in `verification`, the tests/build/checks that prove it works, plus a rollback/recovery trigger for risky changes. Riskier work (auth, payments, migrations, concurrency, data-loss, public APIs) earns more steps, an explicit decision, named risks, and verification; trivial work stays terse (steps alone are fine). Prefer this over a plain prose checklist for multi-step work.",
            &[
                req_arr_items("steps", "Ordered steps: strings or { title, detail, file } objects.", steps_item_schema()),
                opt("title", "string", "Short plan title."),
                opt("summary", "string", "One-paragraph summary of the goal/approach + why this path (synthesis)."),
                opt_arr_items("alternatives", "Key decisions: strings or { option, tradeoff } objects — the approach chosen and why it beats the rejected one.", alternatives_item_schema()),
                opt_str_arr("risks", "Failure modes / hidden assumptions of the riskiest steps (strings)."),
                opt_str_arr("verification", "Checks that prove it works + rollback trigger (strings)."),
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
            "Replace exact text in a file. Line endings are matched tolerantly (LF vs CRLF), so `\\n`-only oldText matches a Windows `\\r\\n` file. Re-issuing an insert that is already present is a safe no-op, not a duplicate.",
            &[
                req("path", "string", "File path."),
                req("oldText", "string", "Exact text to find (whitespace-significant; EOL-tolerant)."),
                req("newText", "string", "Replacement text; empty string deletes the matched text."),
                opt_int("expectedReplacements", "", REPLACEMENT_MIN, REPLACEMENT_MAX),
                opt("saveToDisk", "boolean", ""),
            ],
        ));
        tools.push(tool(
            "PatchEngine",
            "Apply many file edits in one atomic batch (all-or-nothing; rolls back on any failure). Each op needs `action` and `path`; an invalid op is reported by its index. StrReplace-style ops are EOL-tolerant like StrReplace.",
            &[
                req_arr_items("operations", "Patch ops.", patch_operation_schema()),
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
            "Snapshot file contents so risky edits can be rolled back. action=create captures the given paths (or all open editor files when paths is omitted); list/diff/delete/restore manage them.",
            &[
                req("action", "string", "create/list/diff/delete/restore."),
                opt("id", "string", "Checkpoint id (for diff/delete/restore)."),
                opt("label", "string", "Human label for a created checkpoint."),
                opt_str_arr("paths", "Array of workspace file paths to snapshot on create. Omit to snapshot all open editor files."),
                opt("saveToDisk", "boolean", ""),
                opt("dryRun", "boolean", ""),
            ],
        ));
        tools.push(tool(
            "Shell",
            "Run ONE shell command and get {exitCode, stdout, stderr, timedOut, stdoutTruncated, stderrTruncated}. Runs non-interactively in the workspace root (override with cwd); default timeout 120s (max 600). Output over ~24k chars is head+tail truncated (see the *Truncated flags). On Windows it runs via cmd.exe /C as a SINGLE line — chain steps with `&&`, never a newline; use cmd syntax (dir/type, %VAR%, backslash paths). On Unix it is /bin/sh. Catastrophic commands are refused.",
            &[
                req("command", "string", "The command line (single line; chain with && or ;)."),
                opt("cwd", "string", "Working directory (workspace-relative); defaults to the workspace root."),
                opt_int("timeoutSecs", "Timeout in seconds (default 120).", TIMEOUT_MIN, TIMEOUT_MAX),
            ],
        ));
        tools.push(tool(
            "TerminalContext",
            "Terminal sessions and output.",
            &[
                opt("sessionId", "string", ""),
                opt_int("maxChars", "", 1, 100_000),
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
            "SshConnect",
            "Open a non-interactive SSH session to a remote host and verify it. Use the host alias from ~/.ssh/config (see SshList), a hostname/IP, or user@host. Auth uses ssh-agent / default keys / an explicit identityFile — never an interactive password (Lux runs in BatchMode). Returns a sessionId for SshExec/SshTransfer plus the remote OS and home directory. This is the ONLY correct way to start SSH work; do not run `ssh` through Shell/TerminalWrite.",
            &[
                req("host", "string", "ssh_config alias, hostname/IP, or user@host."),
                opt("user", "string", "Login user (overrides host/config)."),
                opt_int("port", "Port (default 22 or per ssh_config).", PORT_MIN, PORT_MAX),
                opt("identityFile", "string", "Path to a private key to use exclusively."),
                opt("label", "string", "Friendly label for the session."),
            ],
        ));
        tools.push(tool(
            "SshExec",
            "Run a command on an SSH session (from SshConnect) and return structured { exitCode, stdout, stderr, durationMs }. Runs in the session's sticky working directory; pass cwd to change it for this and following commands. Non-interactive and catastrophic-command guarded. Prefer this over Shell `ssh ...` for every remote command.",
            &[
                req("session", "string", "sessionId from SshConnect."),
                req("command", "string", "Remote command to run."),
                opt("cwd", "string", "Remote working directory (sticky for the session)."),
                opt_int("timeoutSecs", "Timeout in seconds, default 120, max 600.", TIMEOUT_MIN, TIMEOUT_MAX),
            ],
        ));
        tools.push(tool(
            "SshTransfer",
            "Copy a file or directory between the workspace and a remote host over scp, for an SSH session. The local path is confined to the workspace.",
            &[
                req("session", "string", "sessionId from SshConnect."),
                req("direction", "string", "\"upload\" (local→remote) or \"download\" (remote→local)."),
                req("localPath", "string", "Workspace-relative or absolute path inside the workspace."),
                req("remotePath", "string", "Absolute or login-relative path on the remote host."),
                opt("recursive", "boolean", "Copy directories recursively."),
            ],
        ));
        tools.push(tool(
            "SshDisconnect",
            "Close an SSH session (by sessionId) or every session (all=true).",
            &[
                opt("session", "string", "sessionId to close."),
                opt("all", "boolean", "Close all sessions."),
            ],
        ));
        tools.push(tool(
            "Goal",
            "Pin session goal and progress.",
            &[
                opt("goal", "string", ""),
                opt_int("progress", "", PROGRESS_MIN, PROGRESS_MAX),
                opt("status", "string", ""),
                opt("summary", "string", ""),
            ],
        ));
        tools.push(tool(
            "TodoWrite",
            "Replace the session task list. `todos` is an array of OBJECTS, each with a required `content` and optional id/status/priority/notes.",
            &[req_arr_items("todos", "Task items (objects, not strings).", todo_item_schema())],
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

        // ── MCP self-management (agent/automatic): install, inspect, restart servers ──
        tools.push(tool(
            "McpManage",
            "Manage Model Context Protocol (MCP) servers so you can extend your own toolset live. Actions: 'list' (configured servers + live state + their tools), 'add' (register a server by command/args/env and connect it — its tools then become callable as mcp__<id>__<tool> on the NEXT round), 'connect'/'restart' (reconnect by id), 'disconnect' (stop a live session, keep config), 'enable'/'disable' (toggle + persist), 'remove' (delete config). MCP servers run real local processes (a command you specify, e.g. `npx -y @some/mcp-server`); treat them as trusted-but-side-effecting. After add/connect, call 'list' or check status to confirm 'connected' before relying on the new tools.",
            &[
                req("action", "string", "list | add | connect | restart | disconnect | enable | disable | remove"),
                opt("id", "string", "Server id (lowercase letters/digits/-/_, no '__'). Required for all actions except 'list'."),
                opt("name", "string", "Human-readable name (add)."),
                opt("command", "string", "Executable to spawn for the stdio transport, e.g. 'npx' (add)."),
                opt_str_arr("args", "Command arguments, e.g. ['-y','@modelcontextprotocol/server-filesystem','.'] (add)."),
                opt("env", "object", "Environment variables for the server process as a JSON object (add)."),
                opt("enabled", "boolean", "Enable flag for enable/disable, or initial state for add (default true)."),
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
                    opt("interactive", "boolean", "Only interactive elements."),
                    opt("compact", "boolean", "Condensed output."),
                    opt_int("depth", "Max tree depth.", 1, 100),
                    opt(
                        "selector",
                        "string",
                        "Scope the snapshot to a CSS selector.",
                    ),
                    opt(
                        "includeUrls",
                        "boolean",
                        "Include href URLs for link elements.",
                    ),
                ],
            ));
            tools.push(tool(
                "BrowserHelp",
                "agent-browser usage guide. No args = list available skills. Pass skill=<name> for that skill's docs; skill='core' is the command reference and snapshot/@ref workflow. Valid skills: core | electron | slack | dogfood | agentcore | vercel-sandbox. There are no other help topics.",
                &[
                    opt("skill", "string", "Skill name: core | electron | slack | dogfood | agentcore | vercel-sandbox. Unknown names fall back to 'core'."),
                    opt("full", "boolean", "Append the full command reference/templates (long; may truncate)."),
                    opt("allSkills", "boolean", "Fetch every skill's docs at once (very long)."),
                ],
            ));
            // `fix` triggers a side-effecting repair, so expose it only when execute-capable;
            // diagnostics-only params stay available in read-only modes.
            let mut doctor_params = vec![
                opt("offline", "boolean", "Skip network checks. DEFAULT true — pass false only to include the registry/update probe (slow, needs network)."),
                opt("quick", "boolean", "Skip the live Chromium launch test. DEFAULT true — pass false for a full launch check (30s+ cold start)."),
            ];
            if browser_write {
                doctor_params.push(opt("fix", "boolean", "Attempt automatic repair."));
            }
            tools.push(tool(
                "BrowserDoctor",
                "agent-browser install/health diagnostics. Default run is offline+quick (fast: no Chromium launch, no network). Pass offline:false and/or quick:false for the full diagnostic.",
                &doctor_params,
            ));
        }
        if browser_write {
            tools.push(tool(
                "BrowserOpen",
                "Open browser session.",
                &[
                    opt("url", "string", "URL to navigate to on open."),
                    opt(
                        "headed",
                        "boolean",
                        "Run headed (visible) instead of headless.",
                    ),
                ],
            ));
            tools.push(tool(
                "BrowserAct",
                "Browser action against @refs from a snapshot. `command` is split on whitespace with NO quote handling — use it only when no argument contains spaces. For any value with spaces (typing/filling multi-word text) use `batchCommands`, one pre-tokenized argument per array element.",
                &[
                    opt(
                        "command",
                        "string",
                        "Single action, split on whitespace into CLI args (no quotes). Use only when no argument has spaces.",
                    ),
                    opt_str_arr(
                        "batchCommands",
                        "Pre-tokenized args, one CLI token per element (e.g. [\"type\",\"#search\",\"hello world\"]); spaces inside an element are preserved. Preferred when any value has spaces.",
                    ),
                ],
            ));
            tools.push(tool(
                "BrowserScreenshot",
                "Capture a screenshot to a file (returns the saved path; the image is not fed into vision).",
                &[
                    opt("path", "string", "Output file path (.png/.jpg/.jpeg/.webp). Relative paths resolve against the workspace root; passing a directory saves screenshot-<timestamp>.png inside it; a missing extension gets .png appended."),
                    opt("annotate", "boolean", "Annotate interactive elements."),
                    opt("fullPage", "boolean", "Capture the full scrollable page."),
                ],
            ));
            tools.push(tool(
                "BrowserClose",
                "Close browser.",
                &[opt(
                    "all",
                    "boolean",
                    "Close all sessions, not just the active one.",
                )],
            ));
            tools.push(tool(
                "BrowserChat",
                "Natural-language browser.",
                &[
                    req("instruction", "string", "What to do in natural language."),
                    opt(
                        "quiet",
                        "boolean",
                        "Suppress step-by-step narration (default true).",
                    ),
                ],
            ));
            tools.push(tool(
                "BrowserDashboard",
                "Dashboard.",
                &[
                    opt("action", "string", "Dashboard action (e.g. start/stop)."),
                    opt_int(
                        "port",
                        "Port to serve the dashboard on.",
                        PORT_MIN,
                        PORT_MAX,
                    ),
                ],
            ));
            tools.push(tool(
                "BrowserInstall",
                "Install agent-browser.",
                &[opt(
                    "withDeps",
                    "boolean",
                    "Also install system dependencies.",
                )],
            ));
            tools.push(tool(
                "BrowserInvoke",
                "Run the agent-browser CLI. `args` is the argv array starting with the subcommand, e.g. [\"open\",\"https://example.com\"] or [\"click\",\"#submit\"]. Do NOT pass --json or --session — Lux injects those automatically; adding them yourself will conflict.",
                &[req_str_arr("args", "argv array: [subcommand, ...flags]. Omit --json/--session (injected).")],
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
    /// Item schema for array parameters (replaces the old boolean `is_array`).
    items: Option<serde_json::Value>,
    /// Minimum value constraint for integer parameters.
    min_val: Option<i64>,
    /// Maximum value constraint for integer parameters.
    max_val: Option<i64>,
}

const fn req(name: &'static str, kind: &'static str, desc: &'static str) -> Param {
    Param {
        name,
        kind,
        desc,
        required: true,
        items: None,
        min_val: None,
        max_val: None,
    }
}
const fn opt(name: &'static str, kind: &'static str, desc: &'static str) -> Param {
    Param {
        name,
        kind,
        desc,
        required: false,
        items: None,
        min_val: None,
        max_val: None,
    }
}

const fn opt_int(name: &'static str, desc: &'static str, min: i64, max: i64) -> Param {
    Param {
        name,
        kind: "integer",
        desc,
        required: false,
        items: None,
        min_val: Some(min),
        max_val: Some(max),
    }
}

fn req_str_arr(name: &'static str, desc: &'static str) -> Param {
    Param {
        name,
        kind: "array",
        desc,
        required: true,
        items: Some(json!({ "type": "string" })),
        min_val: None,
        max_val: None,
    }
}
fn opt_str_arr(name: &'static str, desc: &'static str) -> Param {
    Param {
        name,
        kind: "array",
        desc,
        required: false,
        items: Some(json!({ "type": "string" })),
        min_val: None,
        max_val: None,
    }
}
const fn req_arr_items(name: &'static str, desc: &'static str, items: serde_json::Value) -> Param {
    Param {
        name,
        kind: "array",
        desc,
        required: true,
        items: Some(items),
        min_val: None,
        max_val: None,
    }
}
const fn opt_arr_items(name: &'static str, desc: &'static str, items: serde_json::Value) -> Param {
    Param {
        name,
        kind: "array",
        desc,
        required: false,
        items: Some(items),
        min_val: None,
        max_val: None,
    }
}

fn tool(name: &str, description: &str, params: &[Param]) -> serde_json::Value {
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

    #[test]
    fn integer_params_have_bounds() {
        let defs = runtime_tool_definitions("agent", true);
        for tool_val in &defs {
            let Some(func) = tool_val.get("function") else {
                continue;
            };
            let Some(name) = func.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(params) = func.pointer("/parameters/properties") else {
                continue;
            };
            let Some(obj) = params.as_object() else {
                continue;
            };
            for (prop_name, schema) in obj {
                let Some(ty) = schema.get("type").and_then(|v| v.as_str()) else {
                    continue;
                };
                if ty == "integer" {
                    let has_min = schema.get("minimum").is_some();
                    let has_max = schema.get("maximum").is_some();
                    assert!(
                        has_min && has_max,
                        "integer param {name}/{prop_name} missing minimum/maximum"
                    );
                }
            }
        }
    }

    #[test]
    fn string_arrays_have_string_items() {
        let defs = runtime_tool_definitions("agent", true);
        for tool_val in &defs {
            let Some(func) = tool_val.get("function") else {
                continue;
            };
            let Some(params) = func.pointer("/parameters/properties") else {
                continue;
            };
            let Some(obj) = params.as_object() else {
                continue;
            };
            for (prop_name, schema) in obj {
                let Some(ty) = schema.get("type").and_then(|v| v.as_str()) else {
                    continue;
                };
                if ty != "array" {
                    continue;
                }
                let Some(items) = schema.get("items") else {
                    panic!("array param without items: {prop_name}");
                };
                let items_obj = items
                    .as_object()
                    .unwrap_or_else(|| panic!("array param {prop_name} has non-object items"));
                // If it's an anyOf, that's a union item schema – skip items.type check.
                if items_obj.contains_key("anyOf") {
                    continue;
                }
                let item_type = items
                    .get("type")
                    .and_then(|v| v.as_str())
                    .unwrap_or("<missing>");
                // Properties-only arrays (like PatchEngine.operations) have object items.
                // String-only arrays have string items.
                // Both are valid — just assert there's something reasonable.
                assert!(
                    item_type == "string" || item_type == "object",
                    "array param {prop_name} has unexpected items.type: {item_type}"
                );
            }
        }
    }
}
