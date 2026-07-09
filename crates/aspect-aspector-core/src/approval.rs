use crate::types::{ApprovalGate, ParsedToolCall};

pub const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

pub const QUESTION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

pub const TOOL_OUTPUT_CHAR_LIMIT: usize = 32_000;

pub const TURN_OUTPUT_BYTE_BUDGET: usize = 600_000;

pub const TURN_TOOL_CALL_BUDGET: usize = 200;

pub fn resolve_approval_gate(
    tool: &ParsedToolCall,
    permission_input: &str,
    rules: &[String],
    approval_mode: &str,
    interactive: bool,
    auto_approve: &[String],
) -> ApprovalGate {
    let is_read_tool = is_read_tool_name(&tool.name);
    if is_read_tool {
        return ApprovalGate::Allowed;
    }

    if let Some(rule) = find_matching_rule(&tool.name, permission_input, rules) {
        return match rule.as_str() {
            "deny" => ApprovalGate::Blocked,
            "allow" => ApprovalGate::Allowed,
            _ => {
                if !interactive {
                    ApprovalGate::RejectedNonInteractive
                } else {
                    ApprovalGate::Prompt
                }
            }
        };
    }

    if auto_approve.iter().any(|a| tool.name.contains(a)) {
        return ApprovalGate::Allowed;
    }

    match approval_mode {
        "auto" | "automatic" | "full" => ApprovalGate::Allowed,
        "always" | "always_ask" => {
            if interactive {
                ApprovalGate::Prompt
            } else {
                ApprovalGate::RejectedNonInteractive
            }
        }
        _ => {
            if interactive {
                ApprovalGate::Prompt
            } else {
                ApprovalGate::RejectedNonInteractive
            }
        }
    }
}

fn find_matching_rule(tool_name: &str, permission_input: &str, rules: &[String]) -> Option<String> {
    for rule in rules {
        let rule = rule.trim();
        if let Some(stripped) = rule.strip_prefix("deny ") {
            if matches_pattern(tool_name, permission_input, stripped) {
                return Some("deny".to_string());
            }
        } else if let Some(stripped) = rule.strip_prefix("allow ") {
            if matches_pattern(tool_name, permission_input, stripped) {
                return Some("allow".to_string());
            }
        } else if let Some(stripped) = rule.strip_prefix("ask ") {
            if matches_pattern(tool_name, permission_input, stripped) {
                return Some("ask".to_string());
            }
        }
    }
    None
}

fn matches_pattern(tool_name: &str, permission_input: &str, pattern: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    if tool_name == pattern {
        return true;
    }
    let globbed = if pattern.contains('*') || pattern.contains('?') {
        glob_match(pattern, tool_name)
    } else {
        false
    };
    if globbed {
        return true;
    }
    if permission_input.contains(pattern) {
        return true;
    }
    false
}

fn glob_match(pattern: &str, name: &str) -> bool {
    let regex_pattern = pattern
        .replace('.', "\\.")
        .replace('*', ".*")
        .replace('?', ".");
    regex::Regex::new(&format!("^{regex_pattern}$"))
        .is_ok_and(|re| re.is_match(name))
}

fn is_read_tool_name(name: &str) -> bool {
    matches!(
        name,
        "Read"
            | "Grep"
            | "Glob"
            | "SemanticSearch"
            | "RelatedFiles"
            | "RepoMap"
            | "WorkspaceIndex"
            | "SymbolContext"
            | "GitContext"
            | "DiagnosticsContext"
            | "ReadLints"
            | "InspectFile"
            | "TerminalContext"
            | "FastContext"
            | "ActiveContext"
            | "RulesContext"
            | "DocsContext"
            | "MemoryContext"
            | "RecallMemory"
            | "WebFetch"
            | "BrowserStatus"
            | "BrowserSnapshot"
            | "BrowserScreenshot"
            | "BrowserList"
            | "CodeGraphQuery"
            | "CodeGraphCallers"
            | "CodeGraphCallees"
            | "CodeGraphExplain"
            | "CodeGraphCommunity"
    )
}
