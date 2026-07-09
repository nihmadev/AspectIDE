use super::schema::{tool, req, opt, opt_int, req_arr_items, opt_str_arr, todo_item_schema};

pub fn register(tools: &mut Vec<serde_json::Value>) {
    tools.push(tool(
        "Goal",
        "Pin session goal and progress.",
        &[
            opt("goal", "string", ""),
            opt_int("progress", "", 0, 100),
            opt("status", "string", ""),
            opt("summary", "string", ""),
        ],
    ));
    let todo_schema = todo_item_schema();
    tools.push(tool(
        "TodoWrite",
        "Replace the session task list. `todos` is an array of OBJECTS, each with a required `content` and optional id/status/priority/notes.",
        &[req_arr_items("todos", "Task items (objects, not strings).", todo_schema)],
    ));
    tools.push(tool(
        "Task",
        "Spawn an isolated subagent with its own model\u{2194}tool loop; blocks until it returns {agentId, subagentType, summary, boardPosts, boardTopics}. The subagent sees ONLY your prompt (no chat history) \u{2014} include all needed context and say exactly what to return. PARALLEL FAN-OUT: two or more Task calls issued in ONE response run CONCURRENTLY \u{2014} fan out independent work (explore several subsystems, review + test at once) instead of chaining sequential Tasks; parallel subagents that edit files must target DISJOINT files. BACKGROUND: background:true returns {agentId, status:\"started\"} immediately \u{2014} you keep working while the subagent runs; collect results with TaskWait. Prefer background for long/heavy briefs so you never sit blocked. Subagents coordinate through the shared AgentMessage board and cannot spawn further subagents.",
        &[
            req("description", "string", "Short title shown in the Agent rail."),
            req("prompt", "string", "Full task brief: context, goal, and the exact report you expect back."),
            opt("subagent_type", "string", "generalPurpose (default, full tools) | testRunner | codeReviewer (read-only) | explorer (read-only)."),
            opt("model", "string", "Optional model id override for this subagent; omit to inherit the current model."),
            opt("background", "boolean", "true = detached: returns {agentId, status:\"started\"} at once; collect with TaskWait. Use for heavy briefs while you do other work."),
        ],
    ));
    tools.push(tool(
        "TaskWait",
        "Wait for background subagents (Task background:true) and return their results: {tasks: [{agentId, subagentType, description, status: running|done|failed, summary, boardPosts, boardTopics}], stillRunning, timedOut}. Omit agentIds to wait for ALL background tasks in this session; pass agentIds to target specific ones. Blocks until every target settles or timeoutSecs passes (partial results are returned on timeout).",
        &[
            opt_str_arr("agentIds", "Agent ids to wait for (from Task background:true). Omit = all."),
            opt_int("timeoutSecs", "Max seconds to wait (default 600).", 5, 1800),
        ],
    ));
    tools.push(tool(
        "McpManage",
        "Manage Model Context Protocol (MCP) servers so you can extend your own toolset live. Actions: 'list' (configured servers + live state + their tools), 'add' (register a server by command/args/env and connect it \u{2014} its tools then become callable as mcp__<id>__<tool> on the NEXT round), 'connect'/'restart' (reconnect by id), 'disconnect' (stop a live session, keep config), 'enable'/'disable' (toggle + persist), 'remove' (delete config). MCP servers run real local processes (a command you specify, e.g. `npx -y @some/mcp-server`); treat them as trusted-but-side-effecting. After add/connect, call 'list' or check status to confirm 'connected' before relying on the new tools.",
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
