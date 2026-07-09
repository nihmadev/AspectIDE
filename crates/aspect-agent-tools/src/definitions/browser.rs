use super::schema::{tool, req, opt, opt_int, req_str_arr, opt_str_arr};

pub fn register(tools: &mut Vec<serde_json::Value>, write: bool) {
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
                opt("selector", "string", "Scope the snapshot to a CSS selector."),
                opt("includeUrls", "boolean", "Include href URLs for link elements."),
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
        let mut doctor_params = vec![
            opt("offline", "boolean", "Skip network checks. DEFAULT true \u{2014} pass false only to include the registry/update probe (slow, needs network)."),
            opt("quick", "boolean", "Skip the live Chromium launch test. DEFAULT true \u{2014} pass false for a full launch check (30s+ cold start)."),
        ];
        if write {
            doctor_params.push(opt("fix", "boolean", "Attempt automatic repair."));
        }
        tools.push(tool(
            "BrowserDoctor",
            "agent-browser install/health diagnostics. Default run is offline+quick (fast: no Chromium launch, no network). Pass offline:false and/or quick:false for the full diagnostic.",
            &doctor_params,
        ));
    }
    if write {
        tools.push(tool(
            "BrowserOpen",
            "Open browser session.",
            &[
                opt("url", "string", "URL to navigate to on open."),
                opt("headed", "boolean", "Run headed (visible) instead of headless."),
            ],
        ));
        tools.push(tool(
            "BrowserAct",
            "Browser action against @refs from a snapshot. `command` is split on whitespace with NO quote handling \u{2014} use it only when no argument contains spaces. For any value with spaces (typing/filling multi-word text) use `batchCommands`, one pre-tokenized argument per array element. For SEVERAL sequential actions use `commands` (each element one full command string).",
            &[
                opt("command", "string", "Single action, split on whitespace into CLI args (no quotes). Use only when no argument has spaces."),
                opt_str_arr("batchCommands", "ONE action, pre-tokenized: one CLI token per element (e.g. [\"type\",\"#search\",\"hello world\"]); spaces inside an element are preserved. Preferred when any value has spaces."),
                opt_str_arr("commands", "SEVERAL actions run sequentially: each element is one complete command string (e.g. [\"click @e1\", \"wait 500\", \"snapshot\"])."),
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
            &[opt("all", "boolean", "Close all sessions, not just the active one.")],
        ));
        tools.push(tool(
            "BrowserChat",
            "Natural-language browser control via agent-browser's OWN cloud AI. Requires the AI_GATEWAY_API_KEY environment variable (Vercel AI Gateway) \u{2014} without it this tool ALWAYS fails; do not retry, use BrowserSnapshot + BrowserAct instead (same capability, no external key).",
            &[req("instruction", "string", "What to do in natural language.")],
        ));
        tools.push(tool(
            "BrowserDashboard",
            "Dashboard.",
            &[
                opt("action", "string", "Dashboard action (e.g. start/stop)."),
                opt_int("port", "Port to serve the dashboard on.", 1, 65535),
            ],
        ));
        tools.push(tool(
            "BrowserInstall",
            "Install agent-browser.",
            &[opt("withDeps", "boolean", "Also install system dependencies.")],
        ));
        tools.push(tool(
            "BrowserInvoke",
            "Run the agent-browser CLI. `args` is the argv array starting with the subcommand, e.g. [\"open\",\"https://example.com\"] or [\"click\",\"#submit\"]. Do NOT pass --json or --session \u{2014} AspectIDE injects those automatically; adding them yourself will conflict.",
            &[req_str_arr("args", "argv array: [subcommand, ...flags]. Omit --json/--session (injected).")],
        ));
    }
}
