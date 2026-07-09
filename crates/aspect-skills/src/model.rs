//! Skill data model: the parsed [`Skill`], its [`SkillScope`], the create/edit
//! [`SkillDraft`], and the [`ScoredSkill`] relevance result. All `serde`
//! (camelCase) so the desktop layer returns them straight across the Tauri bridge.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where a skill is stored. Project skills shadow global skills of the same slug.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SkillScope {
    /// `<workspace>/.aspect/skills/` вЂ” travels with the repository.
    Project,
    /// `<app_data>/skills/` вЂ” the user's personal, cross-project library.
    Global,
}

/// A fully resolved skill (frontmatter + Markdown body).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    /// Folder/file slug вЂ” the stable identifier within a scope (kebab/snake case).
    pub slug: String,
    /// `name` from frontmatter, falling back to the slug.
    pub name: String,
    /// Optional human-facing title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// One-line "use this whenвЂ¦" description the model reads to decide relevance.
    pub description: String,
    /// Optional longer trigger description.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when_to_use: Option<String>,
    /// Tools this skill expects to use (advisory metadata).
    pub allowed_tools: Vec<String>,
    /// Free-form tags, used for matching and filtering.
    pub tags: Vec<String>,
    /// Disabled skills are hidden from matching/injection but kept on disk.
    pub enabled: bool,
    /// Which scope this skill was discovered in.
    pub scope: SkillScope,
    /// Absolute path to the skill's Markdown file.
    pub path: PathBuf,
    /// The Markdown instructions (everything after the frontmatter).
    pub body: String,
}

/// Structured fields for creating/editing a skill; rendered to a `SKILL.md`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillDraft {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub when_to_use: Option<String>,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub body: String,
}

fn default_true() -> bool {
    true
}

/// A skill with its relevance score for a query (used by [`crate::match_skills`]).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredSkill {
    #[serde(flatten)]
    pub skill: Skill,
    pub score: f64,
}
