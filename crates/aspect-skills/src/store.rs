//! Filesystem discovery, CRUD, and relevance matching for skills across scopes.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use aspect_core::{AppError, AppResult};

use crate::model::{ScoredSkill, Skill, SkillScope};
use crate::parse::parse_skill;

const SKILL_FILE: &str = "SKILL.md";

/// Validate a skill slug: non-empty, в‰¤96 chars, `[A-Za-z0-9_-]` only (blocks path
/// traversal and odd filenames).
#[must_use]
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 96
        && slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Discover every skill across `roots` (in priority order вЂ” pass project before
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

/// Write a skill's Markdown into `root` using the same canonical path resolution
/// as `read_skill` and `set_skill_enabled`. The directory form
/// `<root>/<slug>/SKILL.md` is always preferred; the single-file form
/// `<root>/<slug>.md` is written only when it already exists *and* no directory
/// form is present, to avoid creating a shadowed copy that agents will never read.
///
/// This prevents the previous split where `write_skill` could update a single-file
/// copy while `read_skill`/`set_skill_enabled` kept loading the directory form.
pub fn write_skill(root: &Path, slug: &str, content: &str) -> AppResult<PathBuf> {
    if !is_valid_slug(slug) {
        return Err(AppError::InvalidPath(format!("invalid skill slug: {slug}")));
    }
    // Use `resolve_skill_path` so the write always targets the same file that
    // `read_skill` / `set_skill_enabled` would open. Fall back to the directory
    // form (preferred canonical) when no file exists yet.
    let target = resolve_skill_path(root, slug).unwrap_or_else(|| root.join(slug).join(SKILL_FILE));
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
/// byte of the file (unknown frontmatter keys, comments, body formatting) вЂ” unlike
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
        // No (well-formed) frontmatter вЂ” prepend a minimal block.
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

// в”Ђв”Ђ internals в”Ђв”Ђ

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

