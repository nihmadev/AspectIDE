//! Declarative tool permission engine.
//!
//! Concept ported from claw-code (MIT) `permissions.rs` — allow/deny/ask rules
//! plus unconditional tool denials. Rules are evaluated in the Rust runtime (the
//! security foundation) before the TypeScript approval prompt runs, so a single
//! authoritative matcher governs which tool calls auto-run, prompt, or are
//! refused.
//!
//! Rule format (one per entry): `[allow|deny|ask:]Tool(glob)`
//!   - `allow:Bash(git *)`      → auto-run any git command
//!   - `deny:Write(*.env)`      → never write .env files
//!   - `ask:Bash(rm *)`         → always prompt for rm
//!   - `Read`                   → bare tool name (no glob) matches every input
//! A missing prefix defaults to `allow`. `*` is a wildcard in the glob.
//! Precedence: deny > ask > allow; first matching rule of the winning tier wins.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PermissionDecision {
    Allow,
    Deny,
    Ask,
    /// No rule matched — caller falls back to its default approval behaviour.
    Default,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionEvaluation {
    pub decision: PermissionDecision,
    /// The rule string that produced a non-default decision, for transparency.
    pub matched_rule: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuleTier {
    Allow,
    Deny,
    Ask,
}

struct ParsedRule {
    raw: String,
    tier: RuleTier,
    tool: String,
    pattern: Option<String>,
}

fn parse_rule(raw: &str) -> Option<ParsedRule> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // Optional `tier:` prefix. Only split on the first colon when the prefix is a
    // known tier, so `Bash(curl http://x)` isn't mis-split.
    let (tier, body) = match raw.split_once(':') {
        Some((prefix, rest)) if matches!(prefix.trim().to_lowercase().as_str(), "allow" | "deny" | "ask") => {
            let tier = match prefix.trim().to_lowercase().as_str() {
                "deny" => RuleTier::Deny,
                "ask" => RuleTier::Ask,
                _ => RuleTier::Allow,
            };
            (tier, rest.trim())
        }
        _ => (RuleTier::Allow, raw),
    };

    let (tool, pattern) = match body.split_once('(') {
        Some((tool, rest)) => {
            let pattern = rest.strip_suffix(')').unwrap_or(rest);
            (tool.trim(), Some(pattern.trim().to_string()))
        }
        None => (body.trim(), None),
    };
    if tool.is_empty() {
        return None;
    }
    Some(ParsedRule {
        raw: raw.to_string(),
        tier,
        tool: tool.to_string(),
        pattern,
    })
}

impl ParsedRule {
    fn matches(&self, tool: &str, input: &str) -> bool {
        if !self.tool.eq_ignore_ascii_case(tool) {
            return false;
        }
        match &self.pattern {
            None => true,
            Some(pattern) if pattern.is_empty() || pattern == "*" => true,
            Some(pattern) => glob_match(&pattern.to_lowercase(), &input.trim().to_lowercase()),
        }
    }
}

/// Minimal glob matcher supporting `*` (any run, including empty) and `?` (one char).
/// Iterative backtracking — no regex, no catastrophic blow-up.
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);

    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some(p);
            mark = t;
            p += 1;
        } else if let Some(star_pos) = star {
            p = star_pos + 1;
            mark += 1;
            t = mark;
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

/// Evaluate `tool` + `input` against the rule set. Precedence: deny > ask > allow.
#[must_use]
pub fn evaluate(tool: &str, input: &str, rules: &[String]) -> PermissionEvaluation {
    let parsed: Vec<ParsedRule> = rules.iter().filter_map(|rule| parse_rule(rule)).collect();

    for wanted in [RuleTier::Deny, RuleTier::Ask, RuleTier::Allow] {
        if let Some(rule) = parsed
            .iter()
            .find(|rule| rule.tier == wanted && rule.matches(tool, input))
        {
            let decision = match wanted {
                RuleTier::Deny => PermissionDecision::Deny,
                RuleTier::Ask => PermissionDecision::Ask,
                RuleTier::Allow => PermissionDecision::Allow,
            };
            return PermissionEvaluation {
                decision,
                matched_rule: Some(rule.raw.clone()),
            };
        }
    }

    PermissionEvaluation {
        decision: PermissionDecision::Default,
        matched_rule: None,
    }
}

/// Tauri command: decide a tool call against the configured permission rules.
#[tauri::command]
pub fn ai_permission_decide(tool: String, input: String, rules: Vec<String>) -> PermissionEvaluation {
    evaluate(&tool, &input, &rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decide(tool: &str, input: &str, rules: &[&str]) -> PermissionDecision {
        let owned: Vec<String> = rules.iter().map(|s| s.to_string()).collect();
        evaluate(tool, input, &owned).decision
    }

    #[test]
    fn allow_glob_matches() {
        assert_eq!(decide("Shell", "git status", &["allow:Shell(git *)"]), PermissionDecision::Allow);
        assert_eq!(decide("Shell", "npm install", &["allow:Shell(git *)"]), PermissionDecision::Default);
    }

    #[test]
    fn deny_beats_allow() {
        let rules = &["allow:Shell(*)", "deny:Shell(rm *)"];
        assert_eq!(decide("Shell", "rm -rf node_modules", rules), PermissionDecision::Deny);
        assert_eq!(decide("Shell", "ls", rules), PermissionDecision::Allow);
    }

    #[test]
    fn ask_beats_allow_below_deny() {
        let rules = &["allow:Shell(*)", "ask:Shell(git push *)"];
        assert_eq!(decide("Shell", "git push --force", rules), PermissionDecision::Ask);
    }

    #[test]
    fn bare_tool_matches_any_input() {
        assert_eq!(decide("Read", "/etc/hosts", &["allow:Read"]), PermissionDecision::Allow);
        assert_eq!(decide("Write", "x", &["deny:Write"]), PermissionDecision::Deny);
    }

    #[test]
    fn no_prefix_defaults_to_allow() {
        assert_eq!(decide("Shell", "git status", &["Shell(git *)"]), PermissionDecision::Allow);
    }

    #[test]
    fn path_glob_for_writes() {
        let rules = &["deny:Write(*.env)"];
        assert_eq!(decide("Write", "config/.env", rules), PermissionDecision::Deny);
        assert_eq!(decide("Write", "src/app.ts", rules), PermissionDecision::Default);
    }

    #[test]
    fn empty_rules_is_default() {
        assert_eq!(decide("Shell", "anything", &[]), PermissionDecision::Default);
    }

    #[test]
    fn colon_in_input_not_mis_split() {
        // A URL with a colon in the pattern must still parse as Shell(...).
        assert_eq!(
            decide("Shell", "curl http://x", &["deny:Shell(curl http://*)"]),
            PermissionDecision::Deny,
        );
    }
}
