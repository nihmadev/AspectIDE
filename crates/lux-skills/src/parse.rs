//! Frontmatter parsing + rendering for `SKILL.md` files.
//!
//! The frontmatter is a small YAML *subset* — `key: value` lines between two
//! `---` fences — deliberately hand-parsed to avoid a YAML dependency. Lists
//! accept either inline `[a, b]` or comma-separated `a, b`. Anything missing
//! falls back sensibly (e.g. `name` → slug, `description` → first body line).

use std::collections::HashMap;
use std::path::Path;

use crate::model::{Skill, SkillDraft, SkillScope};

/// Parse a skill from its on-disk Markdown `text`.
#[must_use]
pub fn parse_skill(slug: &str, scope: SkillScope, path: &Path, text: &str) -> Skill {
    let (front, body) = split_frontmatter(text);
    let name = front
        .get("name")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| slug.to_string());
    let description = front
        .get("description")
        .filter(|value| !value.is_empty())
        .cloned()
        .unwrap_or_else(|| first_meaningful_line(&body));
    let enabled = front.get("enabled").is_none_or(|value| parse_bool(value));
    Skill {
        slug: slug.to_string(),
        name,
        title: optional(&front, "title"),
        description,
        when_to_use: optional(&front, "when_to_use").or_else(|| optional(&front, "whentouse")),
        allowed_tools: front
            .get("allowed_tools")
            .or_else(|| front.get("allowedtools"))
            .map(|value| parse_list(value))
            .unwrap_or_default(),
        tags: front
            .get("tags")
            .map(|value| parse_list(value))
            .unwrap_or_default(),
        enabled,
        scope,
        path: path.to_path_buf(),
        body,
    }
}

/// Render a [`SkillDraft`] into canonical `SKILL.md` text (frontmatter + body).
#[must_use]
pub fn render_skill_markdown(draft: &SkillDraft) -> String {
    let mut out = String::from("---\n");
    out.push_str(&format!("name: {}\n", sanitize_scalar(draft.name.trim())));
    if let Some(title) = draft
        .title
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        out.push_str(&format!("title: {}\n", sanitize_scalar(title)));
    }
    out.push_str(&format!(
        "description: {}\n",
        sanitize_scalar(draft.description.trim())
    ));
    if let Some(when) = draft
        .when_to_use
        .as_ref()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        out.push_str(&format!("when_to_use: {}\n", sanitize_scalar(when)));
    }
    if !draft.allowed_tools.is_empty() {
        out.push_str(&format!(
            "allowed_tools: [{}]\n",
            join_list(&draft.allowed_tools)
        ));
    }
    if !draft.tags.is_empty() {
        out.push_str(&format!("tags: [{}]\n", join_list(&draft.tags)));
    }
    if !draft.enabled {
        out.push_str("enabled: false\n");
    }
    out.push_str("---\n\n");
    out.push_str(draft.body.trim());
    out.push('\n');
    out
}

/// Sanitize a scalar value for inline YAML frontmatter by collapsing newlines
/// (which would inject new keys or close the block early) into a single space.
/// The `---` fence pattern is also collapsed so it cannot close the frontmatter
/// block prematurely.
fn sanitize_scalar(value: &str) -> String {
    // Replace any form of line break with a space, then collapse runs.
    value
        .replace("\r\n", " ")
        .replace(['\r', '\n'], " ")
        // Squash `---` to avoid accidental fence close mid-value.
        .replace("---", "- - -")
}

/// Split leading `---`-fenced frontmatter from the body. Returns an empty map +
/// the whole text as body when there is no (well-formed) frontmatter.
fn split_frontmatter(text: &str) -> (HashMap<String, String>, String) {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let lines: Vec<&str> = normalized.lines().collect();
    if lines.first().map(|line| line.trim()) != Some("---") {
        return (HashMap::new(), normalized.trim().to_string());
    }
    let Some(close) = lines.iter().skip(1).position(|line| line.trim() == "---") else {
        // Opened but never closed — treat the whole text as body.
        return (HashMap::new(), normalized.trim().to_string());
    };
    let close = close + 1; // account for the skipped first line
    let mut map = HashMap::new();
    for line in &lines[1..close] {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_lowercase();
            if !key.is_empty() {
                map.insert(key, value.trim().to_string());
            }
        }
    }
    let body = lines[close + 1..].join("\n").trim().to_string();
    (map, body)
}

fn optional(front: &HashMap<String, String>, key: &str) -> Option<String> {
    front
        .get(key)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// Parse `[a, b]` or `a, b` into trimmed, unquoted, non-empty items.
fn parse_list(raw: &str) -> Vec<String> {
    raw.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn join_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| item.trim())
        .collect::<Vec<_>>()
        .join(", ")
}

fn parse_bool(raw: &str) -> bool {
    matches!(
        raw.trim().to_lowercase().as_str(),
        "true" | "yes" | "1" | "on"
    )
}

/// First non-empty body line with any leading Markdown heading markers stripped —
/// a reasonable description when the frontmatter omits one.
fn first_meaningful_line(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn parses_full_frontmatter() {
        let text = "---\nname: pdf-tools\ntitle: PDF Tools\ndescription: when working with PDF files\nallowed_tools: [Read, Shell]\ntags: pdf, docs\nenabled: false\n---\n\nDo the thing.\nMore.";
        let skill = parse_skill("pdf-tools", SkillScope::Project, &PathBuf::from("x"), text);
        assert_eq!(skill.name, "pdf-tools");
        assert_eq!(skill.title.as_deref(), Some("PDF Tools"));
        assert_eq!(skill.description, "when working with PDF files");
        assert_eq!(skill.allowed_tools, vec!["Read", "Shell"]);
        assert_eq!(skill.tags, vec!["pdf", "docs"]);
        assert!(!skill.enabled);
        assert_eq!(skill.body, "Do the thing.\nMore.");
    }

    #[test]
    fn falls_back_without_frontmatter() {
        let skill = parse_skill(
            "notes",
            SkillScope::Global,
            &PathBuf::from("x"),
            "# Heading\nbody text",
        );
        assert_eq!(skill.name, "notes");
        assert_eq!(skill.description, "Heading");
        assert!(skill.enabled);
        assert_eq!(skill.body, "# Heading\nbody text");
    }

    #[test]
    fn unterminated_frontmatter_is_all_body() {
        let skill = parse_skill(
            "x",
            SkillScope::Global,
            &PathBuf::from("x"),
            "---\nname: x\nno close",
        );
        assert_eq!(skill.body, "---\nname: x\nno close");
    }

    #[test]
    fn render_sanitizes_newlines_and_fence_in_scalars() {
        let draft = SkillDraft {
            name: "injected\n---\nevil: true".into(),
            description: "line1\r\nline2".into(),
            enabled: true,
            body: "body".into(),
            ..SkillDraft::default()
        };
        let rendered = render_skill_markdown(&draft);
        // The rendered YAML must have exactly two `---` lines (open + close fence).
        let fence_count = rendered.lines().filter(|line| line.trim() == "---").count();
        assert_eq!(fence_count, 2, "injected fence must not open extra blocks");
        // The injected content must NOT appear as a standalone YAML key line.
        // (It may still appear as text inside the `name:` value, which is safe.)
        assert!(
            !rendered.lines().any(|line| line.trim() == "evil: true"),
            "injected key must not appear as a standalone frontmatter line"
        );
        // After parsing the round-tripped skill, `evil` must not be a frontmatter key.
        let skill = parse_skill(
            "injected",
            SkillScope::Project,
            &PathBuf::from("x"),
            &rendered,
        );
        // The description newline must be collapsed to a space.
        assert!(
            !skill.description.contains('\n'),
            "description must not contain newlines after round-trip"
        );
        // The name must not contain a raw newline.
        assert!(
            !skill.name.contains('\n'),
            "name must not contain newlines after round-trip"
        );
    }

    #[test]
    fn render_then_parse_roundtrips() {
        let draft = SkillDraft {
            name: "git-flow".into(),
            description: "when managing branches".into(),
            allowed_tools: vec!["Shell".into()],
            tags: vec!["git".into()],
            enabled: true,
            body: "Steps:\n1. branch".into(),
            ..SkillDraft::default()
        };
        let rendered = render_skill_markdown(&draft);
        let skill = parse_skill(
            "git-flow",
            SkillScope::Project,
            &PathBuf::from("x"),
            &rendered,
        );
        assert_eq!(skill.name, "git-flow");
        assert_eq!(skill.description, "when managing branches");
        assert_eq!(skill.allowed_tools, vec!["Shell"]);
        assert_eq!(skill.tags, vec!["git"]);
        assert!(skill.enabled);
        assert_eq!(skill.body, "Steps:\n1. branch");
    }
}
