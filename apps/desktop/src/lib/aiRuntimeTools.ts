export type RuntimeToolName =
  | "FastContext"
  | "RepoMap"
  | "SemanticSearch"
  | "Glob"
  | "Read"
  | "InspectFile"
  | "Write"
  | "StrReplace"
  | "PatchEngine"
  | "Checkpoint"
  | "Delete"
  | "Shell"
  | "TerminalContext"
  | "TerminalWrite"
  | "Grep"
  | "ReadLints"
  | "TodoWrite"
  | "WebFetch"
  | "SymbolContext"
  | "RelatedFiles"
  | "DiagnosticsContext"
  | "GitContext"
  | "TestHealth"
  | "FailureAnalyzer"
  | "WorkspaceIndex"
  | "ActiveContext"
  | "RulesContext"
  | "DocsContext"
  | "MemoryContext"
  | "ContextBudgeter"
  | "ImpactAnalysis"
  | "ReviewDiff"
  | "SecretGuard";

export type RuntimeToolDefinition = {
  type: "function";
  function: {
    name: RuntimeToolName;
    description: string;
    parameters: Record<string, unknown>;
  };
};

export const runtimeTools: RuntimeToolDefinition[] = [
  {
    type: "function",
    function: {
      name: "FastContext",
      description: "Collect a compact workspace context packet: active file, repo map, diagnostics, git state, and matching files for a query.",
      parameters: objectSchema({
        query: stringSchema("The task or topic to collect context for."),
      }, ["query"]),
    },
  },
  {
    type: "function",
    function: {
      name: "RepoMap",
      description: "Summarize the current workspace structure and important project files.",
      parameters: objectSchema({
        maxFiles: numberSchema("Maximum number of files to include, default 80."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "WorkspaceIndex",
      description: "Return a compact indexed snapshot of the workspace: file counts, language mix, important directories, configs, test files, source files, entrypoints, and largest files. Use to orient before broad changes or when deciding which tool to call next.",
      parameters: objectSchema({
        maxFiles: numberSchema("Maximum representative files per section, default 60."),
        maxScan: numberSchema("Maximum files to scan from the workspace index, default uses AI indexing settings."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ActiveContext",
      description: "Return the current IDE state available to the AI: active document, open editor tabs, dirty files, attached files, selected model/provider/agent, approval mode, workspace, and terminal session. Use before acting on the user's current editor state.",
      parameters: objectSchema({
        includeActiveText: booleanSchema("Include a truncated copy of the active document text. Default false."),
        maxOpenDocuments: numberSchema("Maximum open documents to return, default 24."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "RulesContext",
      description: "Read project guidance files such as AGENTS.md, CLAUDE.md, .cursorrules, .cursor/rules, and top-level README snippets. Use before editing to follow local conventions and constraints.",
      parameters: objectSchema({
        query: stringSchema("Optional task/topic used to prioritize matching rule files."),
        maxFiles: numberSchema("Maximum rule files to include, default 12."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "DocsContext",
      description: "Collect local documentation and dependency/version context from README/docs/package manifests. Use when answering framework/API questions or before relying on library behavior.",
      parameters: objectSchema({
        query: stringSchema("Library, framework, feature, or file topic to prioritize."),
        maxFiles: numberSchema("Maximum docs/manifests to include, default 12."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "MemoryContext",
      description: "Collect durable local project memory: decisions, preferences, TODOs, roadmap notes, rule files, recent chat instructions, and current AI runtime defaults. Read-only and local to the workspace.",
      parameters: objectSchema({
        query: stringSchema("Optional topic or current task used to prioritize memory signals."),
        maxFiles: numberSchema("Maximum memory/rule files to inspect, default 14."),
        maxSignals: numberSchema("Maximum extracted memory signals to return, default 40."),
        includeRecentChat: booleanSchema("Include recent user/assistant instructions from this chat. Default true."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ContextBudgeter",
      description: "Build a ranked, compressed context packet under a character budget from active editor state, files, diagnostics, git, rules, docs, memory, related files, and semantic search. Use before long or multi-file work to avoid noisy or oversized context.",
      parameters: objectSchema({
        query: stringSchema("Task, topic, symbol, or change description used to score context relevance."),
        targetChars: numberSchema("Approximate maximum characters for the returned context packet, default 16000, capped below the runtime output limit."),
        includeActiveText: booleanSchema("Include a trimmed excerpt from the active document. Default true."),
        includeOpenDocuments: booleanSchema("Include open editor tabs and dirty file excerpts. Default true."),
        includeToolContext: booleanSchema("Call read-only context tools such as MemoryContext, RulesContext, DocsContext, RelatedFiles, SemanticSearch, GitContext, and DiagnosticsContext. Default true."),
        maxItems: numberSchema("Maximum selected context items to return, default 28."),
      }, ["query"]),
    },
  },
  {
    type: "function",
    function: {
      name: "SemanticSearch",
      description: "Rank code locations by intent using language-server symbols, indexed text hits, and filename relevance. Use when the user asks where behavior is implemented, what owns a feature, or which files to inspect first.",
      parameters: objectSchema({
        query: stringSchema("Feature, symbol, API, error, or natural-language topic to search for."),
        path: stringSchema("Optional workspace-relative or absolute path fragment to prioritize or limit results."),
        maxResults: numberSchema("Maximum ranked results to return, default 24."),
      }, ["query"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Glob",
      description: "List workspace files whose full path contains a simple pattern or extension.",
      parameters: objectSchema({
        pattern: stringSchema("Case-insensitive path fragment, file name, or extension such as .tsx."),
        maxResults: numberSchema("Maximum number of files to return, default 80."),
      }, ["pattern"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Read",
      description: "Read a text file from disk without opening it in the editor.",
      parameters: objectSchema({
        path: stringSchema("Absolute path to the file."),
        maxBytes: numberSchema("Maximum bytes to read, default 120000."),
      }, ["path"]),
    },
  },
  {
    type: "function",
    function: {
      name: "InspectFile",
      description: "Inspect a file with Lux's structured preview engine without opening it in the editor. Prefer this over Read for tables, spreadsheets, PDFs, Office files, archives, notebooks, media, binary files, or when descriptor/metadata/AI context is needed.",
      parameters: objectSchema({
        path: stringSchema("Workspace-relative or absolute path to the file."),
        maxRows: numberSchema("Maximum data rows or structured entries to inspect, default 80."),
        maxColumns: numberSchema("Maximum table/spreadsheet/database columns to inspect, default 24."),
        maxBytes: numberSchema("Maximum inline text bytes to inspect, default 120000, capped at 1000000."),
      }, ["path"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Write",
      description: "Create or fully rewrite a text file inside the workspace. Creates parent directories when needed.",
      parameters: objectSchema({
        path: stringSchema("Workspace-relative or absolute path inside the workspace."),
        text: stringSchema("Complete file contents to write."),
        overwrite: booleanSchema("Allow replacing an existing file. Default false."),
        saveToDisk: booleanSchema("Persist to disk. Default true."),
      }, ["path", "text"]),
    },
  },
  {
    type: "function",
    function: {
      name: "StrReplace",
      description: "Replace an exact text fragment in a workspace file. Fails if the occurrence count does not match expectedReplacements.",
      parameters: objectSchema({
        path: stringSchema("Workspace-relative or absolute path inside the workspace."),
        oldText: stringSchema("Exact text to replace."),
        newText: stringSchema("Replacement text."),
        expectedReplacements: numberSchema("Expected occurrence count, default 1."),
        saveToDisk: booleanSchema("Persist to disk. Default true."),
      }, ["path", "oldText", "newText"]),
    },
  },
  {
    type: "function",
    function: {
      name: "PatchEngine",
      description: "Apply a guarded multi-file patch with full preflight validation, one approval, rollback on disk-write failure, exact replacement counts, and optional dry-run. Prefer this over many separate Write/StrReplace/Delete calls for coordinated edits.",
      parameters: objectSchema({
        operations: {
          type: "array",
          description: "Ordered patch operations. Actions: create, rewrite, replace, delete. Create/rewrite use text; replace uses oldText/newText/expectedReplacements; delete removes one file.",
          items: objectSchema({
            action: stringSchema("create, rewrite, replace, or delete."),
            path: stringSchema("Workspace-relative or absolute path inside the workspace."),
            text: stringSchema("Complete file contents for create/rewrite."),
            oldText: stringSchema("Exact text to replace for replace operations."),
            newText: stringSchema("Replacement text for replace operations."),
            expectedReplacements: numberSchema("Expected occurrence count for replace operations, default 1."),
            overwrite: booleanSchema("Allow create to overwrite an existing file. Default false."),
          }, ["action", "path"]),
        },
        saveToDisk: booleanSchema("Persist to disk. Default true."),
        dryRun: booleanSchema("Validate and summarize without modifying files. Default false."),
      }, ["operations"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Checkpoint",
      description: "Create, list, diff, delete, or restore in-session text snapshots for workspace files. Use create before risky edits and restore to roll back through the guarded PatchEngine approval path.",
      parameters: objectSchema({
        action: stringSchema("create, list, diff, delete, or restore."),
        id: stringSchema("Checkpoint id for diff, delete, or restore. Defaults to the latest checkpoint."),
        label: stringSchema("Optional short label for create."),
        paths: arraySchema("Workspace-relative or absolute file paths to snapshot, diff, or restore. For create, omitted paths default to changed/open/active files; diff/restore default to all checkpoint files."),
        includeOpenDocuments: booleanSchema("For create with omitted paths, include open editor documents. Default true."),
        includeGitChanges: booleanSchema("For create with omitted paths, include current git changed files. Default true."),
        maxFiles: numberSchema("Maximum files to snapshot or inspect, default 40, maximum 80."),
        maxBytesPerFile: numberSchema("Maximum bytes read per file, default 500000, maximum 1000000. Truncated files cannot be restored."),
        saveToDisk: booleanSchema("For restore, persist to disk. Default true."),
        dryRun: booleanSchema("For restore, validate and preview operations without modifying files. Default false."),
      }, ["action"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Delete",
      description: "Delete a file or directory inside the workspace. Use only when the requested change clearly requires removal.",
      parameters: objectSchema({
        path: stringSchema("Workspace-relative or absolute path inside the workspace."),
      }, ["path"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Shell",
      description: "Run a non-interactive shell command in the workspace after explicit user approval. Use for build, test, lint, and diagnostic commands. Do not use for interactive, long-running, network credential, or destructive commands unless the user clearly requested them.",
      parameters: objectSchema({
        command: stringSchema("The exact shell command to run."),
        cwd: stringSchema("Optional workspace-relative or absolute working directory inside the workspace."),
        timeoutSecs: numberSchema("Optional timeout in seconds, default 120, maximum 600."),
      }, ["command"]),
    },
  },
  {
    type: "function",
    function: {
      name: "TerminalContext",
      description: "Return current integrated terminal sessions, the active session, cwd/shell metadata, and recent redacted output tails. Use before referring to terminal state or before writing to an active terminal.",
      parameters: objectSchema({
        sessionId: stringSchema("Optional terminal session id. Defaults to all sessions with the active terminal first."),
        maxChars: numberSchema("Maximum output characters per terminal tail, default 12000, capped at 24000."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "TerminalWrite",
      description: "Write input to an existing integrated terminal after explicit user approval. Use for interactive terminal sessions, sending Enter, interrupts, or answering prompts. Prefer Shell for one-shot non-interactive commands.",
      parameters: objectSchema({
        data: stringSchema("Exact bytes/text to write to the terminal, for example 'npm run dev\\r' or '\\u0003' for Ctrl+C."),
        sessionId: stringSchema("Optional terminal session id. Defaults to the active terminal."),
      }, ["data"]),
    },
  },
  {
    type: "function",
    function: {
      name: "Grep",
      description: "Search text in the current workspace using the IDE search index.",
      parameters: objectSchema({
        query: stringSchema("Text or regex to search for."),
        useRegex: booleanSchema("Treat query as a regular expression."),
        caseSensitive: booleanSchema("Use case-sensitive matching."),
        includeGlobs: arraySchema("Optional include glob patterns."),
        maxResults: numberSchema("Maximum search hits, default 50."),
      }, ["query"]),
    },
  },
  {
    type: "function",
    function: {
      name: "DiagnosticsContext",
      description: "Return current IDE diagnostics grouped as compiler/language-server findings.",
      parameters: objectSchema({
        maxResults: numberSchema("Maximum diagnostics to return, default 80."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ReadLints",
      description: "Read current linter and language diagnostics with filters for path, severity, and source. Use after edits or before claiming code is clean.",
      parameters: objectSchema({
        path: stringSchema("Optional workspace-relative or absolute path filter."),
        severity: stringSchema("Optional severity filter: error, warning, information, or hint."),
        source: stringSchema("Optional diagnostic source filter such as eslint, typescript, rustc, or pylance."),
        maxResults: numberSchema("Maximum diagnostics to return, default 80."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "TodoWrite",
      description: "Replace the current AI session task list with structured todo items and progress states. Use for multi-step work to make progress visible; this does not edit project files.",
      parameters: objectSchema({
        todos: {
          type: "array",
          description: "Complete ordered task list for the current response.",
          items: objectSchema({
            id: stringSchema("Stable short id. If omitted, Lux creates one."),
            content: stringSchema("Concrete task description."),
            status: stringSchema("pending, in_progress, completed, blocked, or cancelled."),
            priority: stringSchema("low, medium, or high."),
            notes: stringSchema("Optional short context or result."),
          }, ["content", "status"]),
        },
      }, ["todos"]),
    },
  },
  {
    type: "function",
    function: {
      name: "WebFetch",
      description: "Fetch a specific HTTP/HTTPS URL and return cleaned text plus metadata. Use for current docs, release notes, error pages, and user-provided links. Private network hosts are blocked unless explicitly allowed.",
      parameters: objectSchema({
        url: stringSchema("The absolute HTTP or HTTPS URL to fetch."),
        maxBytes: numberSchema("Maximum response bytes to read, default 250000, maximum 1000000."),
        timeoutSecs: numberSchema("Request timeout in seconds, default 20, maximum 60."),
        allowPrivateHosts: booleanSchema("Allow localhost/private IP targets. Default false; use only for explicit local URLs."),
      }, ["url"]),
    },
  },
  {
    type: "function",
    function: {
      name: "SymbolContext",
      description: "Return semantic code intelligence from the active language servers: workspace symbols for a query, document symbols for a file, and hover/definition/reference/signature data for an exact position. Prefer this before editing unfamiliar code or when reasoning about APIs, call sites, or symbols.",
      parameters: objectSchema({
        query: stringSchema("Optional symbol name or topic to search in the workspace and filter document symbols."),
        path: stringSchema("Optional workspace-relative or absolute file path for document symbols or position context."),
        line: numberSchema("Optional 1-based line for hover/definition/references/signature context."),
        column: numberSchema("Optional 1-based UTF-16 column for hover/definition/references/signature context."),
        maxResults: numberSchema("Maximum symbols or locations to return, default 80."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "RelatedFiles",
      description: "Find files related to a target file or topic: tests, styles, types, stories, routes, schemas, configs, entrypoints, barrels, and nearby same-name modules. Use before editing to understand likely companions and validation targets.",
      parameters: objectSchema({
        path: stringSchema("Optional workspace-relative or absolute target file. Defaults to the active document."),
        query: stringSchema("Optional topic, symbol, feature, or filename fragments to prioritize."),
        maxResults: numberSchema("Maximum related files to return, default 40."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "GitContext",
      description: "Return the current git branch, ahead/behind counts, and changed files.",
      parameters: objectSchema({}),
    },
  },
  {
    type: "function",
    function: {
      name: "TestHealth",
      description: "Detect and run workspace tests and nearest validation commands across common languages and build systems, then return pass/fail status, command kind, duration, exit code, and compact logs.",
      parameters: objectSchema({}),
    },
  },
  {
    type: "function",
    function: {
      name: "FailureAnalyzer",
      description: "Analyze failing test output, compiler diagnostics, or pasted logs and return root-cause candidates, important evidence lines, affected files, and focused next actions. Use after TestHealth, Shell, or when the user provides error logs.",
      parameters: objectSchema({
        log: stringSchema("Optional raw test, build, CI, or terminal output to analyze."),
        includeTestHealth: booleanSchema("Run TestHealth and analyze its current output. Default true."),
        includeDiagnostics: booleanSchema("Include current IDE diagnostics in the analysis. Default true."),
        maxFindings: numberSchema("Maximum findings to return, default 12."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ImpactAnalysis",
      description: "Estimate blast radius for a planned or active change: related files, tests, diagnostics, configs, entrypoints, and likely validation commands. Use before broad edits.",
      parameters: objectSchema({
        path: stringSchema("Optional workspace-relative or absolute target file. Defaults to the active document."),
        query: stringSchema("Change description or topic to analyze."),
        maxResults: numberSchema("Maximum related files to include, default 32."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "ReviewDiff",
      description: "Review the current workspace diff as a quality gate: summarize changed files, risk signals, missing tests, diagnostics, and recommended verification. Read-only.",
      parameters: objectSchema({
        includePatch: booleanSchema("Include a truncated patch excerpt. Default true."),
        maxFindings: numberSchema("Maximum review findings to return, default 12."),
      }),
    },
  },
  {
    type: "function",
    function: {
      name: "SecretGuard",
      description: "Scan provided text and/or the current workspace diff for likely secrets, credentials, tokens, and private keys. Returns redacted previews and optional redacted text. Read-only.",
      parameters: objectSchema({
        text: stringSchema("Optional logs, shell output, patch, env text, or other content to scan."),
        path: stringSchema("Optional source path label for the provided text."),
        includeDiff: booleanSchema("Also scan the current workspace diff. Default true when text is empty, false otherwise."),
        returnRedactedText: booleanSchema("Return a redacted copy of the provided text. Default false."),
        maxFindings: numberSchema("Maximum findings to return, default 30."),
      }),
    },
  },
];

function objectSchema(properties: Record<string, unknown>, required: string[] = []) {
  return {
    type: "object",
    properties,
    required,
    additionalProperties: false,
  };
}

function stringSchema(description: string) {
  return { type: "string", description };
}

function numberSchema(description: string) {
  return { type: "number", description };
}

function booleanSchema(description: string) {
  return { type: "boolean", description };
}

function arraySchema(description: string) {
  return { type: "array", description, items: { type: "string" } };
}
