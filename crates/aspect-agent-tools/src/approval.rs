#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalGate {
    Blocked(String),
    Allowed,
    RejectedNonInteractive(String),
    Prompt,
}

pub enum PermissionDecision {
    Deny,
    Allow,
    Ask,
    Default,
}

pub struct PermissionEval {
    pub decision: PermissionDecision,
    pub matched_rule: Option<String>,
}

pub fn evaluate_permission(tool: &str, input: &str, rules: &[String]) -> PermissionEval {
    let mut result = PermissionEval {
        decision: PermissionDecision::Default,
        matched_rule: None,
    };
    for rule in rules {
        let trimmed = rule.trim();
        let (kind, pattern) = match trimmed.split_once(':') {
            Some((k, p)) => (k.trim().to_ascii_lowercase(), p.trim()),
            None => continue,
        };
        let (tool_pat, _rest) = match pattern.split_once('(') {
            Some((t, r)) => (t.trim(), r.trim_end_matches(')').trim()),
            None => (pattern.trim(), ""),
        };
        let tool_match = tool_pat == tool
            || (tool_pat == "Bash" && tool == "Shell")
            || (tool_pat == "Shell" && tool == tool);
        let input_val = pattern
            .strip_prefix(tool_pat)
            .and_then(|s| s.strip_prefix('('))
            .and_then(|s| s.strip_suffix(')'));
        let input_match = input_val
            .map(|v| input.contains(v.trim().trim_matches('*')))
            .unwrap_or(false);
        if !tool_match || !input_match {
            continue;
        }
        let priority = match kind.as_str() {
            "deny" => 3,
            "ask" => 2,
            "allow" => 1,
            _ => 0,
        };
        let current = match result.decision {
            PermissionDecision::Deny => 3,
            PermissionDecision::Ask => 2,
            PermissionDecision::Allow => 1,
            PermissionDecision::Default => 0,
        };
        if priority > current {
            result = PermissionEval {
                decision: match kind.as_str() {
                    "deny" => PermissionDecision::Deny,
                    "allow" => PermissionDecision::Allow,
                    "ask" => PermissionDecision::Ask,
                    _ => PermissionDecision::Default,
                },
                matched_rule: Some(trimmed.to_string()),
            };
        }
    }
    result
}

pub fn resolve_approval_gate(
    tool: &str,
    permission_input: &str,
    rules: &[String],
    approval_mode: &str,
    interactive: bool,
    auto_approve: bool,
) -> ApprovalGate {
    let ev = evaluate_permission(tool, permission_input, rules);
    let force_ask = match ev.decision {
        PermissionDecision::Deny => {
            return ApprovalGate::Blocked(ev.matched_rule.map_or_else(
                || format!("{tool} is blocked by a permission rule."),
                |rule| format!("{tool} is blocked by permission rule `{rule}`."),
            ));
        }
        PermissionDecision::Allow => return ApprovalGate::Allowed,
        PermissionDecision::Ask => true,
        PermissionDecision::Default => false,
    };
    if !force_ask && (auto_approve || approval_mode == "full-access") {
        return ApprovalGate::Allowed;
    }
    if !interactive {
        return ApprovalGate::RejectedNonInteractive(format!(
            "{tool} requires approval and is unavailable to subagents."
        ));
    }
    ApprovalGate::Prompt
}
