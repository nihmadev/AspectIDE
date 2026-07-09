use super::schema::{
    tool, req, opt, opt_int, req_str_arr, opt_arr_items,
    ask_user_options_item_schema,
};

pub fn register(tools: &mut Vec<serde_json::Value>) {
    tools.push(tool(
        "FastContext",
        "Collect a compact workspace context packet.",
        &[req("query", "string", "Task or topic.")],
    ));
    tools.push(tool(
        "RepoMap",
        "Summarize workspace structure.",
        &[opt_int("maxFiles", "Max files, default 80.", 1, 500)],
    ));
    tools.push(tool(
        "WorkspaceIndex",
        "Indexed snapshot of the workspace. When the result is truncated, the language/directory/largest aggregates are a lexicographically-first sample (see aggregatesNote), not the whole project.",
        &[
            opt_int("maxFiles", "Max per section, default 60.", 1, 500),
            opt_int("maxScan", "Max files to scan before aggregating (default 5000; clamped to 500\u{2013}20000).", 500, 20_000),
        ],
    ));
    tools.push(tool(
        "ActiveContext",
        "Current IDE state.",
        &[
            opt("includeActiveText", "boolean", "Include active document text."),
            opt_int("maxOpenDocuments", "Max docs.", 1, 500),
        ],
    ));
    tools.push(tool(
        "RulesContext",
        "Read project guidance files.",
        &[
            opt("query", "string", "Topic."),
            opt_int("maxFiles", "Max files.", 1, 500),
        ],
    ));
    tools.push(tool(
        "DocsContext",
        "Local documentation and deps.",
        &[
            opt("query", "string", "Topic."),
            opt_int("maxFiles", "Max files.", 1, 500),
        ],
    ));
    tools.push(tool(
        "MemoryContext",
        "Durable project memory.",
        &[
            opt("query", "string", "Topic."),
            opt_int("maxFiles", "Max files.", 1, 500),
        ],
    ));
    tools.push(tool(
        "RecallMemory",
        "Search this project's durable memory \u{2014} facts, decisions, and conventions saved across sessions. Prefer this over re-deriving things you may have learned before. Each hit's `id` can be passed to RelateMemories to link it to other memories.",
        &[
            req("query", "string", "What to recall."),
            opt("category", "string", "Restrict to a category (core, episodic, semantic, procedural, or custom)."),
            opt_int("limit", "Max results, default 8.", 1, 500),
            opt("includeRelated", "boolean", "For the top 3 hits, also include their directly-related memories (1 hop, confidence >= 0.3) as a `related` array on each hit."),
        ],
    ));
    tools.push(tool(
        "RememberMemory",
        "Save a durable, project-scoped memory for future sessions (a stable fact, decision, or convention). Keep each memory one concise, self-contained statement. Re-remembering a byte-identical fact REINFORCES it (importance rises, no duplicate row); a near-duplicate fact in the same category SUPERSEDES the older one (the store keeps the newer wording and marks the old row stale) \u{2014} so re-saving an updated fact is correct behavior, not spam.",
        &[
            req("content", "string", "The fact to remember, as one self-contained sentence."),
            opt("category", "string", "core | episodic | semantic | procedural | custom (default semantic)."),
            opt("importance", "number", "0..1 relevance weight (default 0.5; out-of-range values are clamped)."),
            opt("pinned", "boolean", "Pin so it always surfaces first."),
            opt_int("ttlDays", "Optional time-to-live in days; the memory is auto-forgotten after it expires (pinning still wins over an expired TTL).", 0, 3650),
        ],
    ));
    tools.push(tool(
        "RelateMemories",
        "Link two memories in the knowledge graph \u{2014} mark one as superseding, extending, deriving from, contradicting, or otherwise related to the other. Get ids from RecallMemory results.",
        &[
            req("sourceId", "string", "Id of the source memory (from RecallMemory)."),
            req("targetId", "string", "Id of the target memory (from RecallMemory)."),
            req("relation", "string", "supersedes | extends | derives | contradicts | related."),
        ],
    ));
    tools.push(tool(
        "ListSkills",
        "List available skills \u{2014} reusable, vetted instruction modules for recurring tasks. Check here before improvising a procedure; an existing skill is more reliable.",
        &[
            opt("query", "string", "Rank skills by relevance to this topic; omit to list all."),
            opt_int("limit", "Max skills, default 20.", 1, 500),
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
        "Rank code locations by intent. The `path` filter is a case-insensitive SUBSTRING match on the workspace-relative path (not a glob) \u{2014} pass a directory fragment like `src/auth`, not `src/**/*.ts`.",
        &[
            req("query", "string", "Topic."),
            opt("path", "string", "Case-insensitive substring of the file path to restrict results (e.g. `src/auth`); NOT a glob."),
            opt_int("maxResults", "Max ranked results.", 1, 500),
            opt_int("maxFiles", "Max workspace files scanned as candidates (default 5000).", 1, 20_000),
        ],
    ));
    tools.push(tool(
        "CodeGraphDefinition",
        "Find where a symbol is defined via the precomputed whole-repo code graph \u{2014} instant and exact. PREFER over Grep for 'where is X defined'. One symbol per call.",
        &[req("symbol", "string", "Exact or partial symbol name.")],
    ));
    tools.push(tool(
        "CodeGraphCallers",
        "List every caller of a symbol (who depends on it) from the precomputed code graph \u{2014} exact, whole-repo, instant. PREFER over Grep for 'who calls/uses X'.",
        &[req("symbol", "string", "Symbol name.")],
    ));
    tools.push(tool(
        "CodeGraphCallees",
        "List every symbol a given symbol calls, from the precomputed code graph \u{2014} an instant map of what X depends on. PREFER over reading the body and chasing imports by hand.",
        &[req("symbol", "string", "Symbol name.")],
    ));
    tools.push(tool(
        "CodeGraphExplain",
        "Deep info about a symbol: kind, degree, neighbors, and connections sorted by relevance. First stop before refactoring or changing a signature \u{2014} shows the blast radius without grepping.",
        &[req("symbol", "string", "Symbol name.")],
    ));
    tools.push(tool(
        "CodeGraphOverview",
        "Overview of the code graph: total nodes, edges, community count, and top 10 god nodes (most connected symbols). Fastest orientation in an unfamiliar codebase \u{2014} run before a broad exploration.",
        &[],
    ));
    tools.push(tool(
        "Glob",
        "List files matching a glob pattern (`*`, `?`, `[...]`, `{a,b}`). NOTE: `*` matches across `/`, so `src/*.ts` also finds nested files (no `**` needed). Results EXCLUDE .gitignored paths (dist/, build/, node_modules/, \u{2026}) \u{2014} those return no match. A plain string with no wildcard is treated as a case-insensitive path substring.",
        &[
            req("pattern", "string", "Glob pattern, or a plain path substring."),
            opt_int("maxResults", "Max.", 1, 500),
        ],
    ));
    tools.push(tool(
        "Read",
        "Read a text file. To page a large file instead of re-reading a truncated head, pass startLine (1-based) and maxLines; the response reports totalLines so you can request the next window.",
        &[
            req("path", "string", "File path."),
            opt_int("maxBytes", "Max bytes.", 1, 10_485_760),
            opt_int("startLine", "1-based first line to return. Combine with maxLines to page a large file.", 1, 10_000_000),
            opt_int("maxLines", "Maximum number of lines to return starting at startLine.", 1, 1_000_000),
        ],
    ));
    tools.push(tool(
        "InspectFile",
        "Structured preview of any file type.",
        &[
            req("path", "string", "File path."),
            opt_int("maxRows", "", 1, 100_000),
            opt_int("maxColumns", "", 1, 1_000),
            opt_int("maxBytes", "", 1, 10_485_760),
        ],
    ));
    tools.push(tool(
        "Grep",
        "Search text in the workspace. The query is a LITERAL string by default \u{2014} set useRegex:true to use a regular expression, including alternation like `foo|bar|baz` (which matches 0 results as a literal).",
        &[
            req("query", "string", "Search text (literal unless useRegex is true)."),
            opt("useRegex", "boolean", "Treat query as a regex (needed for `a|b` alternation, anchors, character classes)."),
            opt("caseSensitive", "boolean", ""),
            opt_int("maxResults", "", 1, 500),
        ],
    ));
    tools.push(tool(
        "SymbolContext",
        "LSP symbols, hover, defs, refs.",
        &[
            opt("query", "string", "Symbol."),
            opt("path", "string", "File."),
            opt_int("line", "0-based line number (first line is 0), matching the editor's LSP coordinates.", 0, 2_000_000),
            opt_int("column", "0-based UTF-16 column offset within the line (first column is 0).", 0, 10_000),
            opt_int("maxResults", "Max results per symbol list.", 1, 300),
        ],
    ));
    tools.push(tool(
        "RelatedFiles",
        "Find related files.",
        &[
            opt("path", "string", "Target."),
            opt("query", "string", "Topic."),
            opt_int("maxResults", "", 1, 500),
            opt_int("maxFiles", "Max workspace files scanned as candidates (default 5000).", 1, 20_000),
        ],
    ));
    tools.push(tool(
        "DiagnosticsContext",
        "IDE diagnostics.",
        &[opt_int("maxResults", "", 1, 500)],
    ));
    tools.push(tool(
        "ReadLints",
        "Linter diagnostics with filters.",
        &[
            opt("path", "string", ""),
            opt("severity", "string", ""),
            opt("source", "string", ""),
            opt_int("maxResults", "", 1, 500),
        ],
    ));
    tools.push(tool("GitContext", "Git branch and changed files.", &[]));
    tools.push(tool(
        "ImpactAnalysis",
        "Blast radius for a change.",
        &[
            opt("path", "string", "Target."),
            opt("query", "string", "Change."),
            opt_int("maxResults", "", 1, 500),
        ],
    ));
    tools.push(tool(
        "ReviewDiff",
        "Quality gate on current diff.",
        &[
            opt("includePatch", "boolean", ""),
            opt_int("maxFindings", "", 1, 500),
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
            opt_int("maxFindings", "", 1, 500),
        ],
    ));
    tools.push(tool(
        "WebFetch",
        "Fetch ONE known URL's content. For an open-ended question across the web, use WebResearch instead.",
        &[
            req("url", "string", "URL."),
            opt_int("maxBytes", "Max response bytes to read (default 250000; clamped to 1024..1000000).", 1_024, 1_000_000),
            opt_int("timeoutSecs", "Request timeout in seconds (default 20, max 60).", 1, 60),
        ],
    ));
    tools.push(tool(
        "WebResearch",
        "Web research: searches the web (SearxNG or DuckDuckGo), fetches the top pages, reranks by relevance, and returns cited sources with extracted content. Two modes via `depth`: standard (fast, one query, ~6-8 sources) and deep (expands the query into several variants, merges all engines, follows one hop of in-page links, and returns more domain-diverse sources \u{2014} slower, ~30-60s, best for hard/open questions). Cite sources as [1], [2]. Prefer over WebFetch when you don't already have the exact URL.",
        &[
            req("query", "string", "The research question or topic."),
            opt("focus", "string", "web | academic | news | social | video | code (default web)."),
            opt("depth", "string", "standard (default, fast) | deep (query expansion + multi-engine + in-page link crawl + more diverse sources; slower)."),
            opt_int("maxSources", "How many ranked sources to return (default 6; up to 8 standard / 15 deep).", 1, 15),
        ],
    ));
    tools.push(tool(
        "MultiWebResearch",
        "Parallel multi-query web research: runs 2-6 searches CONCURRENTLY, merges + dedupes across queries, fetches the best pages, and returns ONE globally ranked, domain-diverse source list. Each source carries matchedQueries (which input queries surfaced it) and sources several queries agree on rank higher. Use for multi-facet questions \u{2014} comparisons (X vs Y), tradeoff surveys, or any topic needing several angles \u{2014} giving each query a DIFFERENT facet. Faster and broader than sequential WebResearch calls. Cite sources as [1], [2].",
        &[
            req_str_arr("queries", "2-6 distinct search queries, each covering a different facet of the question."),
            opt("focus", "string", "web | academic | news | social | video | code (default web); applies to all queries."),
            opt_int("maxSources", "Merged sources to return (default 10, up to 20).", 1, 20),
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
            opt_int("maxFindings", "", 1, 500),
        ],
    ));
    tools.push(tool(
        "AgentMessage",
        "Shared agent board for this session \u{2014} the main agent and every Task subagent read and post here (authors are agent ids; you post as 'main' from the main loop). Post findings other agents need (file locations, decisions, contracts, blockers) under a clear topic; read before duplicating work. Re-reads: pass sinceMs = the timestampMs of the last entry you saw to get only newer posts.",
        &[
            opt("action", "string", "read (default) or post."),
            opt("topic", "string", "Channel name (filter on read; required with content on post)."),
            opt("content", "string", "Message body (post)."),
            opt("author", "string", "Read filter: only this agent's posts (e.g. 'explorer-1a2b3c4d' or 'main')."),
            opt_int("sinceMs", "Read cursor: only entries with timestampMs strictly greater than this.", 0, 253_402_300_799_999),
            opt_int("limit", "Max read.", 1, 500),
        ],
    ));
    tools.push(tool(
        "AskUser",
        "Ask the user a question and wait for their answer. Use sparingly \u{2014} only for genuine decisions you cannot resolve from evidence (product/UX choices, ambiguous scope, credentials). Provide 0\u{2013}10 suggested `options`; the user can also type a custom answer unless allowCustom is false. Optionally render a self-contained HTML5 document via `htmlPreview` for visual choices (mockups, color/layout comparisons). In Automatic mode this returns immediately telling you to decide yourself \u{2014} never blocks.",
        &[
            req("question", "string", "The question to ask."),
            opt("detail", "string", "Optional clarifying context shown under the question."),
            opt_arr_items("options", "0\u{2013}10 suggested answers: strings or { label, description } objects.", ask_user_options_item_schema()),
            opt("multiSelect", "boolean", "Allow selecting more than one option."),
            opt("allowCustom", "boolean", "Offer a free-form answer field (default true)."),
            opt("htmlPreview", "string", "Optional self-contained HTML5 document rendered in a sandboxed preview pane."),
        ],
    ));
}
