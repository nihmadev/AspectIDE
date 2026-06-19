//! `lux-skills` — discoverable, reusable instruction modules for the agent.
//!
//! A *skill* is a Markdown file with YAML-style frontmatter (Claude-Code-style):
//! a `name`/`description` the model reads to decide *when* to apply it, optional
//! `when_to_use`/`allowed_tools`/`tags`/`enabled`, and a Markdown body holding the
//! actual instructions. Skills live in two scopes:
//!
//! * **Project** — `<workspace>/.lux/skills/` (travels with the repo, shareable).
//! * **Global** — `<app_data>/skills/` (the user's personal library).
//!
//! On a slug collision the project scope wins. The crate is pure file/logic with
//! no Tauri dependency; the desktop layer resolves the roots and exposes
//! commands + agent tools.

mod model;
mod parse;
mod store;

pub use model::{ScoredSkill, Skill, SkillDraft, SkillScope};
pub use parse::{parse_skill, render_skill_markdown};
pub use store::{
    delete_skill, discover_skills, is_valid_slug, match_skills, read_skill, set_skill_enabled,
    write_skill,
};
