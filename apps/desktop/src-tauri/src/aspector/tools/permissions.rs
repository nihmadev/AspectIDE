//! Declarative tool permission engine.
//!
//! Concept ported from claw-code (MIT) `permissions.rs` — allow/deny/ask rules
//! plus unconditional tool denials. Rules are evaluated in the Rust runtime (the
//! security foundation) before the TypeScript approval prompt runs, so a single
//! authoritative matcher governs which tool calls auto-run, prompt, or are
//! refused.
//!
//! Rule format (one per entry): `[allow|deny|ask:]Tool(glob)`
//!   - `allow:Shell(git *)`     → auto-run any git command   (also matches `Bash`)
//!   - `deny:Write(*.env)`      → never write .env files
//!   - `ask:Shell(rm *)`        → always prompt for rm
//!   - `Read`                   → bare tool name (no glob) matches every input
//!
//! A missing prefix defaults to `allow`. `*` is a wildcard in the glob.
//! Precedence: deny > ask > allow; first matching rule of the winning tier wins.
//!
//! SECURITY (finding #8): the Tauri command `ai_permission_decide` loads the
//! permission rules from the trusted `SettingsStore` (Rust-side app state) rather
//! than accepting them from the renderer. This prevents a compromised UI path
//! from injecting broad allow-rules or omitting deny-rules to bypass enforcement.

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
        Some((prefix, rest))
            if matches!(
                prefix.trim().to_lowercase().as_str(),
                "allow" | "deny" | "ask"
            ) =>
        {
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

/// Canonical shell tool name. SECURITY (finding #7): users write both `Bash`
/// and `Shell` in their rule lists (the docs show `Bash`, the native turn uses
/// `Shell`). Normalizing here ensures rules never silently fall through because
/// of the alias mismatch.
fn canonical_tool(name: &str) -> &str {
    match name.to_ascii_lowercase().as_str() {
        "bash" => "Shell",
        _ => name,
    }
}

/// True for the path-mutating tools whose permission input is a file path. SECURITY
/// (finding #26): for these, a basename-only deny pattern (no `/`) must match a file
/// in any directory, so `matches` falls back to testing the input's final segment.
/// Names mirror the dispatch arms in `ai_turn.rs` (`Write`/`StrReplace`/`Delete`/`PatchEngine`).
fn is_path_tool(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "write" | "strreplace" | "delete" | "patchengine"
    )
}

impl ParsedRule {
    fn matches(&self, tool: &str, input: &str) -> bool {
        // SECURITY (finding #7): canonicalize both the rule's tool name and the
        // incoming tool name so `Bash` rules match the `Shell` tool (and vice-versa).
        let rule_tool = canonical_tool(&self.tool);
        let query_tool = canonical_tool(tool);
        if !rule_tool.eq_ignore_ascii_case(query_tool) {
            return false;
        }
        match &self.pattern {
            None => true,
            Some(pattern) if pattern.is_empty() || pattern == "*" => true,
            Some(pattern) => {
                // SECURITY (finding #7): normalize whitespace in the command input
                // (collapse tabs/newlines to a single space) so rules cannot be
                // bypassed by embedding a tab or newline where a space is expected.
                let normalized_input = input.split_whitespace().collect::<Vec<_>>().join(" ");
                let pattern_lc = pattern.to_lowercase();
                let input_lc = normalized_input.to_lowercase();
                // Anchored match against the whole (normalized) input — keeps the
                // existing semantics for every current rule (e.g. `git *`).
                if glob_match(&pattern_lc, &input_lc) {
                    return true;
                }
                // SECURITY (finding #25): a deny like `Shell(rm *)` must also fire on a
                // dangerous command buried in a compound shell line (`ls && rm -rf /`,
                // `cd /tmp; rm secrets`, `true || rm -rf ~`). Re-test the pattern against
                // each `;`/`&`/`|`-delimited segment. Over-splitting only ADDS deny
                // matches, never removes one — fail-closed.
                if rule_tool == "Shell" {
                    return shell_segments(&input_lc)
                        .iter()
                        .any(|seg| glob_match(&pattern_lc, seg));
                }
                // SECURITY (finding #26): for path-mutating tools a deny like
                // `Write(.env)` must fire regardless of which directory the file lives
                // in (`/root/config/.env`). When the pattern has no `/` it targets a
                // basename, so also test it against the input's final path segment. A
                // pattern that DOES contain `/` stays anchored (keeps directory scoping
                // like `config/.env`). This shortcut is path-tool-only — Shell handled above.
                if is_path_tool(rule_tool) && !pattern_lc.contains('/') {
                    // `rsplit` always yields at least one element, so the basename is
                    // the whole string when the path has no `/`.
                    let basename = input_lc.rsplit('/').next().unwrap_or(&input_lc);
                    return glob_match(&pattern_lc, basename);
                }
                false
            }
        }
    }
}

/// Split a normalized, lowercased shell command into its top-level segments on the
/// chaining operators `;`, `&`, `|`. SECURITY (finding #25): a deny like `rm *` must
/// also fire on a dangerous command buried inside a compound line (`ls && rm -rf /`).
/// This splits on the individual characters rather than the `&&`/`||` token pairs, so
/// it over-splits (e.g. `&&` yields an empty middle) — but over-splitting only ADDS
/// candidate segments to test, it never merges two commands into one, so a deny can
/// never be hidden by chaining. Fail-closed by construction.
fn shell_segments(normalized_lower: &str) -> Vec<String> {
    normalized_lower
        .split([';', '&', '|'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
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

/// Key in `settings.json` where AI preferences (including `toolPermissionRules`)
/// are stored. Must match the TypeScript `AI_PREFERENCES_KEY` constant.
const AI_PREFERENCES_SETTINGS_KEY: &str = "ai.preferences";

/// Extract `toolPermissionRules: string[]` from the stored AI preferences blob,
/// or return an empty list if the key/field is absent.
fn load_permission_rules_from_settings(
    state: &tauri::State<'_, crate::SharedState>,
) -> Vec<String> {
    let Ok(settings) = state.settings.lock() else {
        return Vec::new();
    };
    let Some(store) = settings.as_ref() else {
        return Vec::new();
    };
    let Some(setting) = store.get(aspect_core::SettingsScope::User, AI_PREFERENCES_SETTINGS_KEY)
    else {
        return Vec::new();
    };
    // The value is the full `AiPreferences` JSON object; we only need the rules array.
    setting
        .value
        .get("toolPermissionRules")
        .and_then(|v| v.as_array())
        .map_or_else(Vec::new, |arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_string())
                .take(100) // match the TS-side limit
                .collect()
        })
}

/// Tauri command: decide a tool call against the configured permission rules.
///
/// SECURITY (finding #8): rules are loaded from the trusted Rust-side
/// `SettingsStore` rather than being accepted from the command caller. A
/// compromised renderer path cannot inject broad allow-rules or omit denies
/// to bypass safety enforcement. The `_rules` parameter is accepted but
/// intentionally **ignored** — it exists only to avoid a breaking API change
/// while the TypeScript side still passes the rules it built; the Rust side
/// always uses the authoritative persisted copy.
#[tauri::command]
pub fn ai_permission_decide(
    state: tauri::State<'_, crate::SharedState>,
    tool: String,
    input: String,
    // Intentionally ignored — loaded from SettingsStore instead (see above).
    _rules: Vec<String>,
) -> PermissionEvaluation {
    let rules = load_permission_rules_from_settings(&state);
    evaluate(&tool, &input, &rules)
}

