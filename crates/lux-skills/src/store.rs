//! Filesystem discovery, CRUD, and relevance matching for skills across scopes.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use lux_core::{AppError, AppResult};

use crate::model::{ScoredSkill, Skill, SkillScope};
use crate::parse::parse_skill;

const SKILL_FILE: &str = "SKILL.md";

/// Validate a skill slug: non-empty, ≤96 chars, `[A-Za-z0-9_-]` only (blocks path
/// traversal and odd filenames).
#[must_use]
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 96
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Discover every skill across `roots` (in priority order — pass project before
/// global). On a slug collision the earlier scope wins. Results are sorted by name.
pub fn discover_skills(roots: &[(SkillScope, PathBuf)]) -> AppResult<Vec<Skill>> {
    let mut by_slug: BTreeMap<String, Skill> = BTreeMap::new();
    for (scope, root) in roots {
        if !root.is_dir() {
            continue;
        }
        for skill in discover_root(*scope, root)? {
            by_slug.entry(skill.slug.clone()).or_insert(skill);
        }
    }
    let mut skills: Vec<Skill> = by_slug.into_values().collect();
    skills.sort_by_key(|skill| skill.name.to_lowercase());
    Ok(skills)
}

/// Resolve a single skill by slug across `roots` (first scope to contain it wins).
pub fn read_skill(roots: &[(SkillScope, PathBuf)], slug: &str) -> AppResult<Option<Skill>> {
    if !is_valid_slug(slug) {
        return Ok(None);
    }
    for (scope, root) in roots {
        if let Some(path) = resolve_skill_path(root, slug) {
            let text = fs::read_to_string(&path)?;
            return Ok(Some(parse_skill(slug, *scope, &path, &text)));
        }
    }
    Ok(None)
}

/// Write a skill's Markdown into `root`. Overwrites an existing single-file form
/// if present, otherwise writes the directory form `<root>/<slug>/SKILL.md`.
pub fn write_skill(root: &Path, slug: &str, content: &str) -> AppResult<PathBuf> {
    if !is_valid_slug(slug) {
        return Err(AppError::InvalidPath(format!("invalid skill slug: {slug}")));
    }
    let single = root.join(format!("{slug}.md"));
    let target = if single.is_file() {
        single
    } else {
        root.join(slug).join(SKILL_FILE)
    };
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&target, content)?;
    Ok(target)
}

/// Delete a skill (both directory and single-file forms); returns whether anything
/// was removed.
pub fn delete_skill(root: &Path, slug: &str) -> AppResult<bool> {
    if !is_valid_slug(slug) {
        return Err(AppError::InvalidPath(format!("invalid skill slug: {slug}")));
    }
    let mut removed = false;
    let dir = root.join(slug);
    if dir.is_dir() {
        fs::remove_dir_all(&dir)?;
        removed = true;
    }
    let single = root.join(format!("{slug}.md"));
    if single.is_file() {
        fs::remove_file(&single)?;
        removed = true;
    }
    Ok(removed)
}

/// Flip a skill's `enabled` frontmatter flag *in place*, preserving every other
/// byte of the file (unknown frontmatter keys, comments, body formatting) — unlike
/// a full re-render. Returns whether the skill existed.
pub fn set_skill_enabled(root: &Path, slug: &str, enabled: bool) -> AppResult<bool> {
    if !is_valid_slug(slug) {
        return Err(AppError::InvalidPath(format!("invalid skill slug: {slug}")));
    }
    let Some(path) = resolve_skill_path(root, slug) else {
        return Ok(false);
    };
    let text = fs::read_to_string(&path)?;
    fs::write(&path, apply_enabled_flag(&text, enabled))?;
    Ok(true)
}

/// Set/insert the `enabled:` line inside a `SKILL.md`'s frontmatter, touching
/// nothing else. Prepends a minimal frontmatter block when none exists.
fn apply_enabled_flag(text: &str, enabled: bool) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let value = if enabled { "true" } else { "false" };
    let lines: Vec<&str> = normalized.lines().collect();
    let close = if lines.first().map(|line| line.trim()) == Some("---") {
        lines
            .iter()
            .skip(1)
            .position(|line| line.trim() == "---")
            .map(|rel| rel + 1)
    } else {
        None
    };
    let Some(close) = close else {
        // No (well-formed) frontmatter — prepend a minimal block.
        return format!("---\nenabled: {value}\n---\n\n{}", normalized.trim_start());
    };
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 1);
    out.push(lines[0].to_string());
    let mut wrote = false;
    for line in &lines[1..close] {
        if line
            .split_once(':')
            .is_some_and(|(key, _)| key.trim().eq_ignore_ascii_case("enabled"))
        {
            out.push(format!("enabled: {value}"));
            wrote = true;
        } else {
            out.push((*line).to_string());
        }
    }
    if !wrote {
        out.push(format!("enabled: {value}"));
    }
    for line in &lines[close..] {
        out.push((*line).to_string());
    }
    let mut result = out.join("\n");
    if normalized.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Rank enabled skills by relevance to `query` (token overlap over
/// name/title/description/when_to_use/tags). With an empty query, returns enabled
/// skills name-sorted (score 0) so callers can present the full catalog.
#[must_use]
pub fn match_skills(skills: &[Skill], query: &str, limit: usize) -> Vec<ScoredSkill> {
    let query_tokens = tokenize(query);
    let mut scored: Vec<ScoredSkill> = skills
        .iter()
        .filter(|skill| skill.enabled)
        .map(|skill| {
            let haystack = skill_haystack(skill);
            ScoredSkill {
                skill: skill.clone(),
                score: overlap_score(&query_tokens, &haystack),
            }
        })
        .filter(|scored| query_tokens.is_empty() || scored.score > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.skill
                    .name
                    .to_lowercase()
                    .cmp(&b.skill.name.to_lowercase())
            })
    });
    scored.truncate(limit);
    scored
}

// ── internals ──

fn discover_root(scope: SkillScope, root: &Path) -> AppResult<Vec<Skill>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            let Some(slug) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !is_valid_slug(slug) {
                continue;
            }
            if let Some(file) = skill_file_in_dir(&path) {
                let text = fs::read_to_string(&file)?;
                out.push(parse_skill(slug, scope, &file, &text));
            }
        } else if file_type.is_file() {
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            // A top-level SKILL.md is the marker file inside a dir form, not a skill on its own.
            if name.eq_ignore_ascii_case(SKILL_FILE) || !name.to_lowercase().ends_with(".md") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
                continue;
            };
            if !is_valid_slug(stem) {
                continue;
            }
            let text = fs::read_to_string(&path)?;
            out.push(parse_skill(stem, scope, &path, &text));
        }
    }
    Ok(out)
}

fn skill_file_in_dir(dir: &Path) -> Option<PathBuf> {
    let upper = dir.join(SKILL_FILE);
    if upper.is_file() {
        return Some(upper);
    }
    let lower = dir.join("skill.md");
    if lower.is_file() {
        return Some(lower);
    }
    None
}

fn resolve_skill_path(root: &Path, slug: &str) -> Option<PathBuf> {
    skill_file_in_dir(&root.join(slug)).or_else(|| {
        let single = root.join(format!("{slug}.md"));
        single.is_file().then_some(single)
    })
}

fn skill_haystack(skill: &Skill) -> HashSet<String> {
    let mut text = format!("{} {}", skill.name, skill.description);
    if let Some(title) = &skill.title {
        text.push(' ');
        text.push_str(title);
    }
    if let Some(when) = &skill.when_to_use {
        text.push(' ');
        text.push_str(when);
    }
    if !skill.tags.is_empty() {
        text.push(' ');
        text.push_str(&skill.tags.join(" "));
    }
    tokenize(&text).into_iter().collect()
}

fn overlap_score(query_tokens: &[String], haystack: &HashSet<String>) -> f64 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let matched = query_tokens
        .iter()
        .filter(|token| haystack.contains(*token))
        .count();
    matched as f64 / query_tokens.len() as f64
}

fn tokenize(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.chars().count() >= 2)
        .map(str::to_lowercase)
        .filter(|token| seen.insert(token.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn discovers_dir_and_single_file_forms() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("pdf/SKILL.md"),
            "---\nname: pdf\ndescription: pdf work\n---\nbody",
        );
        write(
            &root.join("git.md"),
            "---\nname: git\ndescription: git work\n---\nbody",
        );

        let skills = discover_skills(&[(SkillScope::Project, root.to_path_buf())]).unwrap();
        assert_eq!(skills.len(), 2);
        assert!(skills.iter().any(|s| s.slug == "pdf"));
        assert!(skills.iter().any(|s| s.slug == "git"));
    }

    #[test]
    fn project_scope_shadows_global() {
        let project = tempdir().unwrap();
        let global = tempdir().unwrap();
        write(
            &project.path().join("git/SKILL.md"),
            "---\nname: git-project\ndescription: project\n---\n",
        );
        write(
            &global.path().join("git/SKILL.md"),
            "---\nname: git-global\ndescription: global\n---\n",
        );

        let skills = discover_skills(&[
            (SkillScope::Project, project.path().to_path_buf()),
            (SkillScope::Global, global.path().to_path_buf()),
        ])
        .unwrap();
        let git = skills.iter().find(|s| s.slug == "git").unwrap();
        assert_eq!(git.name, "git-project");
        assert_eq!(git.scope, SkillScope::Project);
    }

    #[test]
    fn write_read_delete_roundtrip() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write_skill(
            root,
            "deploy",
            "---\nname: deploy\ndescription: ship it\n---\nsteps",
        )
        .unwrap();
        let roots = [(SkillScope::Global, root.to_path_buf())];
        let skill = read_skill(&roots, "deploy").unwrap().unwrap();
        assert_eq!(skill.name, "deploy");
        assert!(delete_skill(root, "deploy").unwrap());
        assert!(read_skill(&roots, "deploy").unwrap().is_none());
    }

    #[test]
    fn rejects_traversal_slug() {
        assert!(!is_valid_slug("../evil"));
        assert!(!is_valid_slug("a/b"));
        assert!(is_valid_slug("git-flow_2"));
        let dir = tempdir().unwrap();
        assert!(write_skill(dir.path(), "../evil", "x").is_err());
    }

    #[test]
    fn matches_by_relevance_and_skips_disabled() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("pdf/SKILL.md"),
            "---\nname: pdf\ndescription: extract text from pdf documents\ntags: pdf\n---\n",
        );
        write(
            &root.join("css/SKILL.md"),
            "---\nname: css\ndescription: style components\n---\n",
        );
        write(
            &root.join("old/SKILL.md"),
            "---\nname: old\ndescription: pdf legacy\nenabled: false\n---\n",
        );

        let skills = discover_skills(&[(SkillScope::Project, root.to_path_buf())]).unwrap();
        let hits = match_skills(&skills, "how to read a pdf", 5);
        assert_eq!(hits.first().unwrap().skill.slug, "pdf");
        assert!(
            hits.iter().all(|h| h.skill.slug != "old"),
            "disabled skills must be excluded"
        );
    }

    #[test]
    fn when_to_use_and_tags_feed_matching() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("a/SKILL.md"),
            "---\nname: a\ndescription: generic helper\nwhen_to_use: handling kubernetes deployments\ntags: k8s\n---\n",
        );
        write(
            &root.join("b/SKILL.md"),
            "---\nname: b\ndescription: generic helper\n---\n",
        );
        let skills = discover_skills(&[(SkillScope::Project, root.to_path_buf())]).unwrap();
        let hits = match_skills(&skills, "kubernetes", 5);
        assert_eq!(
            hits.len(),
            1,
            "only the skill whose when_to_use mentions kubernetes should match"
        );
        assert_eq!(hits[0].skill.slug, "a");
    }

    #[test]
    fn set_enabled_flips_in_place_and_preserves_content() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        write(
            &root.join("x/SKILL.md"),
            "---\nname: x\nauthor: jane\ndescription: d\n---\nbody line",
        );
        let roots = [(SkillScope::Global, root.to_path_buf())];

        assert!(set_skill_enabled(root, "x", false).unwrap());
        assert!(!read_skill(&roots, "x").unwrap().unwrap().enabled);
        let raw = fs::read_to_string(root.join("x/SKILL.md")).unwrap();
        assert!(
            raw.contains("author: jane"),
            "unknown frontmatter key must survive"
        );
        assert!(raw.contains("body line"), "body must survive");

        assert!(set_skill_enabled(root, "x", true).unwrap());
        assert!(read_skill(&roots, "x").unwrap().unwrap().enabled);
        assert!(!set_skill_enabled(root, "missing", true).unwrap());
    }
}
