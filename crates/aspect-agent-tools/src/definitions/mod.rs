mod schema;
mod context;
mod plan;
mod edit;
mod shell;
mod remote;
mod agent;
mod browser;

pub const MAX_RESULTS_SYMBOL: i64 = 300;

/// Produce the full tool-definitions array filtered by mode and settings.
pub fn runtime_tool_definitions(agent_mode: &str, browser_enabled: bool) -> Vec<serde_json::Value> {
    let full_exec = matches!(agent_mode, "agent" | "automatic");
    let plan_capable = full_exec || matches!(agent_mode, "plan");

    let mut tools = Vec::with_capacity(48);

    context::register(&mut tools);

    if plan_capable {
        plan::register(&mut tools);
    }

    if full_exec {
        edit::register(&mut tools);
        shell::register(&mut tools);
        remote::register(&mut tools);
        agent::register(&mut tools);
    }

    if browser_enabled {
        browser::register(&mut tools, full_exec);
    }

    tools
}

/// Inject the active provider's wire model ids into the Task tool's `model`
/// parameter description, so the orchestrating model only assigns subagent
/// models that actually exist on this provider (instead of guessing ids like
/// "haiku" that the provider will 400 on). No-op when the list is empty or the
/// Task tool is absent (read-only modes).
pub fn annotate_task_model_options(tools: &mut [serde_json::Value], available: &[String]) {
    if available.is_empty() {
        return;
    }
    let shown: Vec<&str> = available.iter().map(String::as_str).take(24).collect();
    let suffix = if available.len() > shown.len() {
        format!(" (+{} more)", available.len() - shown.len())
    } else {
        String::new()
    };
    for def in tools.iter_mut() {
        let is_task = def
            .pointer("/function/name")
            .and_then(|value| value.as_str())
            == Some("Task");
        if !is_task {
            continue;
        }
        if let Some(model_desc) =
            def.pointer_mut("/function/parameters/properties/model/description")
        {
            *model_desc = serde_json::Value::String(format!(
                "Optional model id override for this subagent; omit to inherit the current model. Valid ids on the current provider: {shown}{suffix}.",
                shown = shown.join(", "),
            ));
        }
        return;
    }
}
