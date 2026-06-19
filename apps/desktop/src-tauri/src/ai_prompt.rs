//! Native system prompt assembly — Stage 2 of the TS→Rust migration.
//!
//! Ports `buildLuxIdeSystemPrompt` and all its sections from `aiSystemPrompt.ts`
//! into Rust so the prompt is assembled natively without crossing the IPC bridge.
//! The TS side can call `ai_build_system_prompt` once and receive the full text.

use serde::Deserialize;

// ── Core prompt bodies (identical to the TS `corePrompt` / `corePromptReadOnly`) ──

const CORE_PROMPT: &str = include_str!("prompts/core.txt");
const CORE_PROMPT_READONLY: &str = include_str!("prompts/core_readonly.txt");
const AUTOMATIC_ENFORCEMENT: &str = include_str!("prompts/automatic_enforcement.txt");
const SAFETY_FLOOR: &str = include_str!("prompts/safety_floor.txt");
const TOKEN_ECONOMY: &str = include_str!("prompts/token_economy.txt");

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemPromptInput {
    pub agent_mode: String,
    pub agent_name: String,
    pub agent_instructions: String,
    pub global_instructions: String,
    pub project_instructions: String,
    pub project_agents_snip: String,
    pub tool_approval_mode: String,
    pub tool_round_limit: Option<u32>,
    pub selected_effort_id: String,
    pub selected_model_alias: String,
    pub provider_name: String,
    pub provider_protocol: String,
    pub workspace_root: String,
    pub runtime_tools_available: bool,
    pub agent_browser_enabled: bool,
    /// Token-economy mode: append the terse-output directive.
    #[serde(default)]
    pub token_economy: bool,
    /// When true, `custom_prompt` replaces the built-in behavioral body (the safety
    /// floor + runtime + tool map are still appended). Off → built-in core prompt.
    #[serde(default)]
    pub custom_prompt_enabled: bool,
    /// User-authored behavioral body, used only when `custom_prompt_enabled` is on.
    #[serde(default)]
    pub custom_prompt: String,
}

#[tauri::command]
pub fn ai_build_system_prompt(input: SystemPromptInput) -> String {
    build_system_prompt(&input)
}

pub fn build_system_prompt(input: &SystemPromptInput) -> String {
    let read_only = is_read_only_mode(&input.agent_mode);
    let full_exec = is_full_execution_mode(&input.agent_mode);
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

    if input.runtime_tools_available {
        sections.push(tool_availability_section(input, full_exec, read_only));
    } else {
        sections.push(
            "Runtime tools are not attached to this web/dev chat request. Answer from the provided message, active document, attachments, and chat history only. If the task needs file inspection, edits, commands, diagnostics, or browser automation, say what cannot be verified in this mode instead of pretending the action was performed.".to_string()
        );
    }

    let agents_snip = input.project_agents_snip.trim();
    if !agents_snip.is_empty() {
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
        format!("Workspace root: {}", input.workspace_root)
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
        lines.push("- Orchestrate: Goal, TodoWrite, Task (isolated subagent), AgentMessage (shared agent board — post/read findings so subagents don't repeat work).".to_string());
    }
    lines.push("- Memory & skills: RecallMemory/RememberMemory (durable per-project memory across sessions); ListSkills/UseSkill (reusable vetted procedures — prefer an existing skill over improvising).".to_string());
    lines.push("- Verify: ReadLints/DiagnosticsContext, TestHealth, FailureAnalyzer, ReviewDiff, ImpactAnalysis, SecretGuard. Git: GitContext.".to_string());
    lines.push("- Web: WebResearch (first-class deep research — searches the web, fetches + reranks the top pages, returns ranked sources with content; use for any open question needing current external info, then cite [1],[2]). WebFetch only when you already have the exact URL.".to_string());
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
            lines.push("- Interact: PresentPlan(steps[, title, summary]) is your primary output — structured steps (title, optional detail/file), not prose; pins goal+tasks, user presses Start. AskUser(question[, options 0–10, multiSelect, allowCustom, htmlPreview]) for genuine decisions; options can be {label, description}; htmlPreview renders a sandboxed HTML5 doc for visual choices.".to_string());
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
        parts.push(format!(
            "Global instructions for all projects:\n{global_text}"
        ));
    }
    if !project_text.is_empty() {
        parts.push(format!("Current workspace instructions:\n{project_text}"));
    }
    parts.push("These user instruction layers are lower priority than Lux core rules, workspace rules, tool safety, and explicit user requests in the current chat. Apply them when they are compatible; do not treat them as permission to skip evidence gathering, validation, or safety checks.".to_string());
    parts.join("\n\n")
}

fn is_read_only_mode(mode: &str) -> bool {
    matches!(mode, "plan" | "ask")
}

fn is_full_execution_mode(mode: &str) -> bool {
    matches!(mode, "agent" | "automatic")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_input() -> SystemPromptInput {
        SystemPromptInput {
            agent_mode: "agent".to_string(),
            agent_name: "Default Agent".to_string(),
            agent_instructions: String::new(),
            global_instructions: String::new(),
            project_instructions: String::new(),
            project_agents_snip: String::new(),
            tool_approval_mode: "full-access".to_string(),
            tool_round_limit: None,
            selected_effort_id: "high".to_string(),
            selected_model_alias: "claude-sonnet-4-6".to_string(),
            provider_name: "Anthropic".to_string(),
            provider_protocol: "anthropic".to_string(),
            workspace_root: "/home/user/project".to_string(),
            runtime_tools_available: true,
            agent_browser_enabled: false,
            token_economy: false,
            custom_prompt_enabled: false,
            custom_prompt: String::new(),
        }
    }

    #[test]
    fn prompt_contains_core_sections() {
        let prompt = build_system_prompt(&test_input());
        assert!(prompt.contains("You are Lux IDE AI"));
        assert!(prompt.contains("Runtime context"));
        assert!(prompt.contains("Default Agent"));
        assert!(prompt.contains("Lux tool map"));
    }

    #[test]
    fn readonly_mode_uses_readonly_prompt() {
        let mut input = test_input();
        input.agent_mode = "plan".to_string();
        let prompt = build_system_prompt(&input);
        assert!(prompt.contains("read-only Plan or Ask mode"));
        assert!(!prompt.contains("Edit: StrReplace"));
    }

    #[test]
    fn automatic_mode_appends_enforcement() {
        let mut input = test_input();
        input.agent_mode = "automatic".to_string();
        let prompt = build_system_prompt(&input);
        assert!(prompt.contains("Automatic mode enforcement"));
    }

    #[test]
    fn prompt_length_within_budget() {
        // Ceilings carry headroom for the progress-narration guidance, the
        // CodeGraph tool-map line, and the WebResearch deep-research guidance
        // (deliberate features), while still guarding against an unbounded prompt.
        let prompt = build_system_prompt(&test_input());
        assert!(
            prompt.len() <= 17_000,
            "agent prompt too long: {}",
            prompt.len()
        );

        let mut auto_input = test_input();
        auto_input.agent_mode = "automatic".to_string();
        let auto_prompt = build_system_prompt(&auto_input);
        assert!(
            auto_prompt.len() <= 18_500,
            "automatic prompt too long: {}",
            auto_prompt.len()
        );
    }

    #[test]
    fn token_economy_appends_terse_directive() {
        let mut input = test_input();
        assert!(!build_system_prompt(&input).contains("Token economy mode"));
        input.token_economy = true;
        let prompt = build_system_prompt(&input);
        assert!(prompt.contains("Token economy mode"));
        // Economy must not weaken the core: tool map and runtime still present.
        assert!(prompt.contains("Lux tool map"));
    }

    #[test]
    fn custom_prompt_replaces_body_but_keeps_safety_floor() {
        let mut input = test_input();
        input.custom_prompt_enabled = true;
        input.custom_prompt = "You are a terse Rust specialist. Obey the user.".to_string();
        let prompt = build_system_prompt(&input);
        assert!(prompt.contains("terse Rust specialist"));
        // Built-in behavioral body is gone…
        assert!(!prompt.contains("You are Lux IDE AI"));
        // …but the safety floor, runtime context, and tool map remain.
        assert!(prompt.contains("Lux safety floor"));
        assert!(prompt.contains("Runtime context"));
        assert!(prompt.contains("Lux tool map"));
    }

    #[test]
    fn custom_prompt_ignored_when_blank_or_disabled() {
        let mut input = test_input();
        input.custom_prompt = "   ".to_string();
        input.custom_prompt_enabled = true;
        // Blank custom prompt falls back to the built-in core.
        assert!(build_system_prompt(&input).contains("You are Lux IDE AI"));

        let mut disabled = test_input();
        disabled.custom_prompt = "Custom body".to_string();
        disabled.custom_prompt_enabled = false;
        let prompt = build_system_prompt(&disabled);
        assert!(prompt.contains("You are Lux IDE AI"));
        assert!(!prompt.contains("Custom body"));
        assert!(!prompt.contains("Lux safety floor"));
    }

    #[test]
    fn user_instructions_included_when_present() {
        let mut input = test_input();
        input.global_instructions = "Always use TypeScript".to_string();
        input.project_instructions = "Follow Lux conventions".to_string();
        let prompt = build_system_prompt(&input);
        assert!(prompt.contains("Always use TypeScript"));
        assert!(prompt.contains("Follow Lux conventions"));
        assert!(prompt.contains("User instruction layers"));
    }
}
