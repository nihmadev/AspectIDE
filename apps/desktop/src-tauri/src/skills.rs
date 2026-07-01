//! Tauri commands for skills: discovery, lookup, relevance matching, and CRUD
//! across the project (`<workspace>/.lux/skills`) and global (`<app_config>/skills`)
//! scopes. Project skills shadow global ones of the same slug.

use std::path::PathBuf;

use lux_skills::{ScoredSkill, Skill, SkillDraft, SkillScope};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

use crate::{workspace_root, SharedState};

/// The user's global skills library directory.
fn global_root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("skills"))
}

/// Discovery roots in priority order. Security: GLOBAL is listed first so a
/// `read_skill`/`discover_skills` slug collision resolves to the user's own global
/// skill — a repository-controlled `<workspace>/.lux/skills` skill can therefore no
/// longer SILENTLY shadow a trusted global skill of the same slug and be fed to the
/// agent as first-class instructions (a prompt-injection supply-chain vector when
/// opening an untrusted repo). Project skills are still discoverable (and badged
/// `Project` via their `SkillScope`) so the user can review/manage them, but they
/// cannot override a global skill without the user renaming/removing the global one.
/// Full per-workspace trust gating of automatic project-skill matching is tracked
/// as a followup (needs persistent trust state).
fn discovery_roots(
    app: &AppHandle,
    state: &State<'_, SharedState>,
) -> Result<Vec<(SkillScope, PathBuf)>, String> {
    let mut roots = vec![(SkillScope::Global, global_root(app)?)];
    if let Ok(root) = workspace_root(state) {
        roots.push((SkillScope::Project, root.join(".lux").join("skills")));
    }
    Ok(roots)
}

/// The writable root for a given scope (project requires an open workspace).
fn scope_root(
    app: &AppHandle,
    state: &State<'_, SharedState>,
    scope: SkillScope,
) -> Result<PathBuf, String> {
    match scope {
        SkillScope::Project => {
            let root = workspace_root(state)
                .map_err(|_| "open a workspace to manage project skills".to_string())?;
            Ok(root.join(".lux").join("skills"))
        }
        SkillScope::Global => global_root(app),
    }
}

#[tauri::command]
pub fn skills_list(app: AppHandle, state: State<'_, SharedState>) -> Result<Vec<Skill>, String> {
    let roots = discovery_roots(&app, &state)?;
    lux_skills::discover_skills(&roots).map_err(String::from)
}

#[tauri::command]
pub fn skills_get(
    app: AppHandle,
    state: State<'_, SharedState>,
    slug: String,
) -> Result<Option<Skill>, String> {
    let roots = discovery_roots(&app, &state)?;
    lux_skills::read_skill(&roots, &slug).map_err(String::from)
}

/// Load a skill for the agent-facing `UseSkill` tool, honouring the `enabled`
/// flag — unlike [`skills_get`], which the Settings UI relies on to load *any*
/// skill (disabled included) so the user can still edit it.
///
/// A disabled skill is deliberately inert: it is hidden from relevance matching
/// (`skills_match` / `lux_skills::match_skills` filter on `enabled`) and from
/// automatic injection, so a user who turned it off expects it to stay off.
/// Resolving one by slug for use must be equally inert — otherwise the model
/// could resurrect turned-off instructions from a slug it happened to see
/// earlier in the session. Returns `Ok(None)` when the skill is missing *or*
/// disabled so the caller reports it unavailable instead of running its body.
pub fn skill_for_use(
    app: AppHandle,
    state: State<'_, SharedState>,
    slug: String,
) -> Result<Option<Skill>, String> {
    Ok(skills_get(app, state, slug)?.filter(|skill| skill.enabled))
}

#[tauri::command]
pub fn skills_match(
    app: AppHandle,
    state: State<'_, SharedState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<ScoredSkill>, String> {
    let roots = discovery_roots(&app, &state)?;
    let skills = lux_skills::discover_skills(&roots).map_err(String::from)?;
    Ok(lux_skills::match_skills(
        &skills,
        &query,
        limit.unwrap_or(5),
    ))
}

#[tauri::command]
pub fn skills_save(
    app: AppHandle,
    state: State<'_, SharedState>,
    scope: SkillScope,
    slug: String,
    draft: SkillDraft,
) -> Result<Skill, String> {
    if !lux_skills::is_valid_slug(&slug) {
        return Err(format!("invalid skill name: {slug}"));
    }
    let root = scope_root(&app, &state, scope)?;
    let content = lux_skills::render_skill_markdown(&draft);
    lux_skills::write_skill(&root, &slug, &content).map_err(String::from)?;
    lux_skills::read_skill(&[(scope, root)], &slug)
        .map_err(String::from)?
        .ok_or_else(|| "saved skill could not be read back".to_string())
}

#[tauri::command]
pub fn skills_delete(
    app: AppHandle,
    state: State<'_, SharedState>,
    scope: SkillScope,
    slug: String,
) -> Result<bool, String> {
    let root = scope_root(&app, &state, scope)?;
    lux_skills::delete_skill(&root, &slug).map_err(String::from)
}

/// Toggle a skill's `enabled` flag in place — preserves the rest of the file
/// (unknown frontmatter, comments, body formatting) instead of re-rendering it.
#[tauri::command]
pub fn skills_set_enabled(
    app: AppHandle,
    state: State<'_, SharedState>,
    scope: SkillScope,
    slug: String,
    enabled: bool,
) -> Result<bool, String> {
    let root = scope_root(&app, &state, scope)?;
    lux_skills::set_skill_enabled(&root, &slug, enabled).map_err(String::from)
}

/// A skill found in an external tool's directory (Claude/Codex), offered for
/// one-click import into Lux's own skill library.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportableSkill {
    /// Human label of where it came from, e.g. "Claude (global)".
    pub source: String,
    pub slug: String,
    pub name: String,
    pub description: String,
    /// Suggested target scope (global source → Global, project source → Project).
    pub scope_hint: SkillScope,
    pub path: PathBuf,
    /// Raw `SKILL.md` text, copied verbatim on import (preserves frontmatter/body).
    pub content: String,
}

/// External skill directories to scan for importable skills (Claude Code, Codex,
/// `OpenClaw`, Hermes). All imports land in the user's global library, so every
/// source is hinted Global. Only existing dirs are scanned by the caller.
fn external_skill_sources(app: &AppHandle) -> Vec<(String, SkillScope, PathBuf)> {
    let mut sources = Vec::new();
    if let Ok(home) = app.path().home_dir() {
        let skills = |dir: &str| home.join(dir).join("skills");
        sources.push(("Claude".to_string(), SkillScope::Global, skills(".claude")));
        sources.push(("Codex".to_string(), SkillScope::Global, skills(".codex")));
        sources.push((
            "OpenClaw".to_string(),
            SkillScope::Global,
            skills(".openclaw"),
        ));
        sources.push(("Hermes".to_string(), SkillScope::Global, skills(".hermes")));
    }
    sources
}

/// Discover skills under `root`, descending one extra level so Hermes's
/// category-organized layout (`~/.hermes/skills/<category>/<name>/SKILL.md`) is
/// covered alongside the flat `<root>/<name>/SKILL.md` form. Deduped by slug
/// (the flat hit wins; on a leaf-name collision across categories the
/// first-scanned wins) — matching that import writes one file per slug anyway.
fn discover_external_root(hint: SkillScope, root: &PathBuf) -> Vec<lux_skills::Skill> {
    use std::collections::BTreeMap;
    let mut by_slug: BTreeMap<String, lux_skills::Skill> = BTreeMap::new();
    if let Ok(flat) = lux_skills::discover_skills(&[(hint, root.clone())]) {
        for skill in flat {
            by_slug.entry(skill.slug.clone()).or_insert(skill);
        }
    }
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            // Skip dirs that are themselves a skill (have a SKILL.md) — already
            // captured by the flat pass; only descend into category folders.
            if path.join("SKILL.md").is_file() || path.join("skill.md").is_file() {
                continue;
            }
            if let Ok(nested) = lux_skills::discover_skills(&[(hint, path)]) {
                for skill in nested {
                    by_slug.entry(skill.slug.clone()).or_insert(skill);
                }
            }
        }
    }
    by_slug.into_values().collect()
}

/// Auto-discover skills sitting in other agents' folders (Claude, Codex,
/// `OpenClaw`, Hermes) so the user can import them — individually or all at once —
/// into the global library.
// Tauri command: the Result is kept for IPC error-channel symmetry even though
// discovery swallows per-source errors and always succeeds.
#[allow(clippy::unnecessary_wraps)]
#[tauri::command]
pub fn skills_discover_importable(
    app: AppHandle,
    _state: State<'_, SharedState>,
) -> Result<Vec<ImportableSkill>, String> {
    let mut out = Vec::new();
    for (source, hint, root) in external_skill_sources(&app) {
        if !root.is_dir() {
            continue;
        }
        for skill in discover_external_root(hint, &root) {
            let content = std::fs::read_to_string(&skill.path).unwrap_or_default();
            if content.trim().is_empty() {
                continue;
            }
            out.push(ImportableSkill {
                source: source.clone(),
                slug: skill.slug,
                name: skill.name,
                description: skill.description,
                scope_hint: hint,
                path: skill.path,
                content,
            });
        }
    }
    Ok(out)
}

/// Import a skill into Lux's library from raw `SKILL.md` content (a discovered
/// candidate or a manual paste). Written verbatim, so frontmatter/body survive.
#[tauri::command]
pub fn skills_import(
    app: AppHandle,
    state: State<'_, SharedState>,
    scope: SkillScope,
    slug: String,
    content: String,
) -> Result<Skill, String> {
    if !lux_skills::is_valid_slug(&slug) {
        return Err(format!("invalid skill name: {slug}"));
    }
    if content.trim().is_empty() {
        return Err("skill content is empty".to_string());
    }
    let root = scope_root(&app, &state, scope)?;
    lux_skills::write_skill(&root, &slug, &content).map_err(String::from)?;
    lux_skills::read_skill(&[(scope, root)], &slug)
        .map_err(String::from)?
        .ok_or_else(|| "imported skill could not be read back".to_string())
}
