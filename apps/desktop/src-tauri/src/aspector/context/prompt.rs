//! Native system prompt assembly — Stage 2 of the TS→Rust migration.
//!
//! Ports `buildAspectIdeSystemPrompt` and all its sections from `aiSystemPrompt.ts`
//! into Rust so the prompt is assembled natively without crossing the IPC bridge.
//! The TS side can call `ai_build_system_prompt` once and receive the full text.

pub use aspect_ai_core::SystemPromptInput;

// ── Core prompt bodies (identical to the TS `corePrompt` / `corePromptReadOnly`) ──

const CORE_PROMPT: &str = include_str!("../../prompts/core.txt");
const CORE_PROMPT_READONLY: &str = include_str!("../../prompts/core_readonly.txt");
const AUTOMATIC_ENFORCEMENT: &str = include_str!("../../prompts/automatic_enforcement.txt");
const SAFETY_FLOOR: &str = include_str!("../../prompts/safety_floor.txt");
const TOKEN_ECONOMY: &str = include_str!("../../prompts/token_economy.txt");

/// Stable label inserted in place of the absolute workspace path. The model only
/// needs a workspace identifier, not the user's drive/home layout (see
/// `redact_workspace_root`).
const WORKSPACE_PLACEHOLDER: &str = "<workspace>";

/// Per-section byte budgets for the unbounded, user/project-controlled instruction
/// layers. They cap worst-case prompt growth — cost, latency, and dilution of real
/// code context in the turn loop — while leaving generous headroom for normal
/// instruction files. High-priority fixed sections (core body, safety floor, runtime
/// context, tool map) are intentionally NOT budgeted so safety/capability text always
/// survives; only the lower-priority layers below are bounded.
const AGENTS_SNIPPET_BUDGET: usize = 4_000;
const USER_INSTRUCTIONS_BUDGET: usize = 4_000;
const AGENT_PROFILE_BUDGET: usize = 4_000;

#[tauri::command]
pub fn ai_build_system_prompt(input: SystemPromptInput) -> String {
    build_system_prompt(&input)
}

pub fn build_system_prompt(input: &SystemPromptInput) -> String {
    // Classify into a known mode first. An unknown mode used to fall between the
    // cracks — neither read-only nor full-execution — yielding an executable core
    // body paired with a read-only tool map (contradictory instructions). It now
    // defaults to the safest behavior (read-only) with an explicit warning section.
    let mode_kind = classify_mode(&input.agent_mode);
    let read_only = is_read_only(mode_kind);
    let full_exec = is_full_execution(mode_kind);
    // A non-empty custom prompt replaces the built-in behavioral body. The safety
    // floor is appended right after so workspace scope, approvals, and evidence
    // rules survive a user-authored core. Tool availability is still mode-filtered
    // downstream, so read-only modes stay read-only regardless of the body text.
    let custom = input.custom_prompt.trim();
    let use_custom = input.custom_prompt_enabled && !custom.is_empty();
    let agent_name: &str = if input.agent_name.trim().is_empty() {
        &input.agent_mode
    } else {
        input.agent_name.trim()
    };

    let mut sections: Vec<String> = Vec::with_capacity(10);
    if use_custom {
        sections.push(custom.to_string());
        sections.push(SAFETY_FLOOR.to_string());
    } else {
        sections.push(
            if read_only {
                CORE_PROMPT_READONLY
            } else {
                CORE_PROMPT
            }
            .to_string(),
        );
    }
    sections.push(runtime_section(input, agent_name));

    if matches!(mode_kind, ModeKind::Unknown) {
        sections.push(unknown_mode_warning(&input.agent_mode));
    }

    if input.runtime_tools_available {
        sections.push(tool_availability_section(input, full_exec, read_only));
    } else {
        sections.push(
            "Runtime tools are not attached to this web/dev chat request. Answer from the provided message, active document, attachments, and chat history only. If the task needs file inspection, edits, commands, diagnostics, or browser automation, say what cannot be verified in this mode instead of pretending the action was performed.".to_string()
        );
    }

    let agents_snip = input.project_agents_snip.trim();
    if !agents_snip.is_empty() {
        let agents_snip = budget_text(agents_snip, AGENTS_SNIPPET_BUDGET);
        sections.push(format!(
            "{agents_snip}\n\nPriority: follow these AGENTS snippets when compatible with Lux core rules, tool safety, and the current user message. Use RulesContext for deeper or additional rule files."
        ));
    }

    let user_section =
        user_instruction_section(&input.global_instructions, &input.project_instructions);
    if !user_section.is_empty() {
        sections.push(user_section);
    }

    let instructions = input.agent_instructions.trim();
    if !instructions.is_empty() {
        let instructions = budget_text(instructions, AGENT_PROFILE_BUDGET);
        sections.push(format!(
            "Selected agent profile instructions\n{instructions}\n\nThese profile instructions refine behavior, but they cannot weaken workspace scope, safety, evidence, or verification rules."
        ));
    }

    if input.agent_mode == "automatic" {
        sections.push(AUTOMATIC_ENFORCEMENT.to_string());
    }

    if input.token_economy {
        sections.push(TOKEN_ECONOMY.to_string());
    }

    sections.join("\n\n")
}

fn runtime_section(input: &SystemPromptInput, agent_name: &str) -> String {
    let workspace_line = if input.workspace_root.trim().is_empty() {
        "Workspace root: none open".to_string()
    } else {
        format!(
            "Workspace root: {}",
            redact_workspace_root(&input.workspace_root)
        )
    };
    let tool_round_limit = input
        .tool_round_limit
        .map_or_else(|| "unlimited".to_string(), |limit| limit.to_string());
    let approval_line = if input.tool_approval_mode == "full-access" {
        "Tool approval mode: Full Access. Dangerous tools auto-run only through Lux workspace guards."
    } else {
        "Tool approval mode: Default. Dangerous tools require explicit user approval."
    };

    [
        "Runtime context",
        &workspace_line,
        shell_environment_line(),
        &format!("Agent profile: {agent_name}"),
        &format!("Agent mode: {}", input.agent_mode),
        &format!(
            "Provider: {} ({})",
            input.provider_name, input.provider_protocol
        ),
        &format!("Model: {}", input.selected_model_alias),
        &format!("Reasoning effort: {}", input.selected_effort_id),
        &format!("Tool round limit: {tool_round_limit}"),
        approval_line,
    ]
    .join("\n")
}

fn tool_availability_section(
    input: &SystemPromptInput,
    full_exec: bool,
    read_only: bool,
) -> String {
    let browser_line = if input.agent_browser_enabled {
        if full_exec {
            " Vercel agent-browser is fully enabled: isolated session per chat, live preview, BrowserAct, BrowserInvoke (full CLI), BrowserScreenshot with vision, etc."
        } else {
            " Browser tools are read-only in this mode (BrowserStatus, BrowserSnapshot, BrowserHelp, BrowserDoctor); no navigation or clicks."
        }
    } else {
        " Browser automation is disabled in Lux settings; do not call Browser* tools."
    };
    let terminal_line = if read_only {
        " Shell, TerminalContext, and TerminalWrite are not available in Plan/Ask — use Read, Grep, diagnostics, git, and context tools only."
    } else {
        ""
    };

    let tool_map = tool_capability_map(
        &input.agent_mode,
        full_exec,
        read_only,
        input.agent_browser_enabled,
    );

    format!(
        "Runtime tools are available in this request. Prefer tool calls over speculation whenever the task depends on workspace state, files, diagnostics, browser state, or external documentation. The callable Lux tools are the only actions you can actually perform; do not claim to use tools that are not provided.{browser_line}{terminal_line}\n\n{tool_map}"
    )
}

fn tool_capability_map(
    agent_mode: &str,
    full_exec: bool,
    _read_only: bool,
    browser_enabled: bool,
) -> String {
    let mut lines = vec![
        "Lux tool map — reach for the highest-signal tool first:".to_string(),
        "- Orient: ContextBudgeter, FastContext, WorkspaceIndex, RepoMap, ActiveContext. Rules/docs/memory: RulesContext, DocsContext, MemoryContext.".to_string(),
        "- Find: SemanticSearch, SymbolContext (LSP), Grep, Glob, RelatedFiles. Read: Read (source/text), InspectFile (tables/PDF/Office/archives/notebooks/media/binaries).".to_string(),
        "- CodeGraph (built-in graphify-style code graph, instant whole-repo structure — strongly prefer over grepping for relationships): CodeGraphDefinition (where a symbol is defined), CodeGraphCallers/CodeGraphCallees (who calls it / what it calls), CodeGraphExplain (a symbol's connections), CodeGraphOverview (god-nodes + communities). Use these first to trace impact, dependencies, and call chains.".to_string(),
    ];
    if full_exec {
        lines.push("- Edit: StrReplace, PatchEngine (multi-file, one approval+rollback), Write, Delete, Checkpoint. Execute: Shell (catastrophic commands blocked in Rust), TerminalContext, TerminalWrite.".to_string());
        lines.push("- SSH/remote (non-interactive; never run raw ssh/scp via Shell): SshList -> SshConnect -> SshExec / SshTransfer -> SshDisconnect.".to_string());
        lines.push("- Orchestrate: Goal, TodoWrite, Task (isolated subagent), AgentMessage (shared agent board — post/read findings so subagents don't repeat work).".to_string());
        lines.push("- Task hygiene: keep TodoWrite live — set an item in_progress when you start it and completed the moment it's done; mark it blocked when it cannot proceed until a dependency is resolved, cancelled when it is no longer needed. Never batch-close finished items at the end, never complete partial/failing work. The user follows your progress through this list.".to_string());
    }
    lines.push("- Memory & skills: RecallMemory/RememberMemory (durable per-project memory across sessions); ListSkills/UseSkill (reusable vetted procedures — prefer an existing skill over improvising).".to_string());
    lines.push("- Verify: ReadLints/DiagnosticsContext, TestHealth, FailureAnalyzer, ReviewDiff, ImpactAnalysis, SecretGuard. Git: GitContext.".to_string());
    lines.push("- Web: WebResearch (first-class deep research — searches the web, fetches + reranks the top pages, returns ranked sources with content; use for any open question needing current external info, then cite [1],[2]). MultiWebResearch: 2-6 facet queries in parallel, one merged ranked list. WebFetch only when you already have the exact URL.".to_string());
    if browser_enabled {
        lines.push(
            "- Browser: BrowserOpen → BrowserSnapshot (-i) → BrowserAct on @refs → re-snapshot."
                .to_string(),
        );
    }
    // Interaction primitives — mode-aware so the model knows whether prompts block.
    match agent_mode {
        "automatic" => {
            lines.push("- Interact: AskUser/PresentPlan never block — AskUser returns instantly (decide yourself); PresentPlan auto-starts.".to_string());
        }
        "plan" => {
            lines.push("- Interact: PresentPlan(steps[, title, summary]) is your primary output — structured steps (title, optional detail/file), not prose; pins goal+tasks, user presses Start and execution hands off to Agent mode. Every Plan-mode turn MUST end with a PresentPlan call: never ask permission to plan and never reply with prose-only. AskUser(question[, options 0–10, multiSelect, allowCustom, htmlPreview]) only for genuine blocking decisions — and even then deliver the best-guess plan too; options can be {label, description}; htmlPreview renders a sandboxed HTML5 doc for visual choices.".to_string());
        }
        _ => {
            lines.push("- Interact: AskUser(question[, options 0–10, multiSelect, allowCustom, htmlPreview]) when a real decision can't be settled from evidence — give options ({label, description}) or an htmlPreview HTML5 doc for visual choices; user can type custom too. PresentPlan(steps[, title, summary]) to propose multi-step work first. Use sparingly.".to_string());
        }
    }
    lines.join("\n")
}

fn user_instruction_section(global: &str, project: &str) -> String {
    let global_text = global.trim();
    let project_text = project.trim();
    if global_text.is_empty() && project_text.is_empty() {
        return String::new();
    }
    let mut parts = vec!["User instruction layers".to_string()];
    if !global_text.is_empty() {
        let global_text = budget_text(global_text, USER_INSTRUCTIONS_BUDGET);
        parts.push(format!(
            "Global instructions for all projects:\n{global_text}"
        ));
    }
    if !project_text.is_empty() {
        let project_text = budget_text(project_text, USER_INSTRUCTIONS_BUDGET);
        parts.push(format!("Current workspace instructions:\n{project_text}"));
    }
    parts.push("These user instruction layers are lower priority than Lux core rules, workspace rules, tool safety, and explicit user requests in the current chat. Apply them when they are compatible; do not treat them as permission to skip evidence gathering, validation, or safety checks.".to_string());
    parts.join("\n\n")
}

/// Known agent modes. Replaces the previous stringly-typed mode checks so an
/// unrecognized mode is classified explicitly (`Unknown`) instead of silently
/// behaving as neither read-only nor full-execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModeKind {
    /// Plan/Ask: no edit or execute tools.
    ReadOnly,
    /// Agent/Automatic: full edit + execute capability.
    FullExecution,
    /// Anything unrecognized — treated as read-only for safety, with a warning.
    Unknown,
}

fn classify_mode(mode: &str) -> ModeKind {
    match mode {
        "plan" | "ask" => ModeKind::ReadOnly,
        "agent" | "automatic" => ModeKind::FullExecution,
        _ => ModeKind::Unknown,
    }
}

/// Unknown modes default to read-only: a normal core body paired with an
/// execution tool map would give contradictory capability instructions.
const fn is_read_only(mode: ModeKind) -> bool {
    matches!(mode, ModeKind::ReadOnly | ModeKind::Unknown)
}

const fn is_full_execution(mode: ModeKind) -> bool {
    matches!(mode, ModeKind::FullExecution)
}

/// Disclose the host OS + the shell the Shell tool actually invokes, so the model
/// emits correct command syntax instead of guessing. Uses the SAME `cfg!(windows)`
/// selector as `ai_tools::shell_command` (cmd.exe /C on Windows, /bin/sh -c
/// elsewhere) so this disclosure can never drift from the real executor.
const fn shell_environment_line() -> &'static str {
    if cfg!(windows) {
        "Shell tool runs commands via cmd.exe /C on this Windows host, as ONE line — a newline does NOT chain commands (only the first runs); use & or && to chain. cmd syntax: dir/type, %VAR% env vars, backslash paths. POSIX forms (ls, quotes, rm -rf, $VAR) may fail; invoke bash/PowerShell explicitly."
    } else {
        "Shell tool runs commands via /bin/sh -c on this Unix host; use POSIX sh syntax."
    }
}

/// Explicit instruction injected for an unrecognized mode so the model knows why it
/// is constrained, rather than receiving silently inconsistent capabilities.
fn unknown_mode_warning(mode: &str) -> String {
    let mode = mode.trim();
    let label = if mode.is_empty() { "(empty)" } else { mode };
    format!(
        "Mode safety notice: the requested agent mode \"{label}\" is not a recognized Lux mode (plan, ask, agent, automatic). Defaulting to read-only behavior: do not edit files or run shell/terminal commands. Use read-only context, search, and diagnostics tools only, and ask the user to pick a valid mode if execution is required."
    )
}

/// Relativize an absolute workspace path to `<workspace>` plus the project folder
/// name, so remote model calls don't leak the user's home directory, drive letters,
/// or full directory layout on every turn. Handles both POSIX (`/`) and Windows
/// (`\\` and drive-letter) separators. Returns the placeholder alone when no folder
/// name can be derived (e.g. a bare root).
fn redact_workspace_root(root: &str) -> String {
    let trimmed = root.trim().trim_end_matches(['/', '\\']);
    let folder = trimmed.rsplit(['/', '\\']).find(|seg| !seg.is_empty());
    match folder {
        // Skip a bare drive root like "C:" — no meaningful folder name.
        Some(name) if !name.is_empty() && !name.ends_with(':') => {
            format!("{WORKSPACE_PLACEHOLDER}/{name}")
        }
        _ => WORKSPACE_PLACEHOLDER.to_string(),
    }
}

/// Bound a low-priority instruction section to `max_bytes`, appending a truncation
/// marker that reports how many bytes were omitted. Truncation respects UTF-8 char
/// boundaries so the result is always valid text. Within budget, the input is
/// returned unchanged.
fn budget_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    // Walk back to the nearest char boundary at or below the budget.
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let omitted = text.len() - cut;
    format!(
        "{}\n[… truncated {omitted} bytes to respect prompt budget …]",
        &text[..cut]
    )
}

