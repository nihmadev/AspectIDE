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

/// Discovery roots in priority order: project first (so it shadows global).
fn discovery_roots(
    app: &AppHandle,
    state: &State<'_, SharedState>,
) -> Result<Vec<(SkillScope, PathBuf)>, String> {
    let mut roots = Vec::new();
    if let Ok(root) = workspace_root(state) {
        roots.push((SkillScope::Project, root.join(".lux").join("skills")));
    }
    roots.push((SkillScope::Global, global_root(app)?));
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

/// External skill directories to scan for importable skills (Claude Code / Codex,
/// global + project). Only existing dirs are returned by the caller.
fn external_skill_sources(
    app: &AppHandle,
    state: &State<'_, SharedState>,
) -> Vec<(String, SkillScope, PathBuf)> {
    let mut sources = Vec::new();
    if let Ok(home) = app.path().home_dir() {
        sources.push((
            "Claude (global)".to_string(),
            SkillScope::Global,
            home.join(".claude").join("skills"),
        ));
        sources.push((
            "Codex (global)".to_string(),
            SkillScope::Global,
            home.join(".codex").join("skills"),
        ));
    }
    if let Ok(root) = workspace_root(state) {
        sources.push((
            "Claude (project)".to_string(),
            SkillScope::Project,
            root.join(".claude").join("skills"),
        ));
        sources.push((
            "Codex (project)".to_string(),
            SkillScope::Project,
            root.join(".codex").join("skills"),
        ));
    }
    sources
}

/// Auto-discover skills sitting in other agents' folders (Claude/Codex) so the
/// user can import them with one click.
#[tauri::command]
pub fn skills_discover_importable(
    app: AppHandle,
    state: State<'_, SharedState>,
) -> Result<Vec<ImportableSkill>, String> {
    let mut out = Vec::new();
    for (source, hint, root) in external_skill_sources(&app, &state) {
        if !root.is_dir() {
            continue;
        }
        let skills = lux_skills::discover_skills(&[(hint, root)]).map_err(String::from)?;
        for skill in skills {
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
