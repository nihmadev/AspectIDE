//! Native `RelatedFiles` tool — Stage 1 of the TS→Rust migration.
//!
//! Finds files related to a target (tests, styles, types, routes, configs,
//! stories, barrels, entrypoints) plus query-token hits. Composes `lux_fs`
//! `list_files` entirely in Rust — no IPC for file lists.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::ai_semantic::{self};
use crate::{workspace_root, SharedState};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedFileResult {
    pub path: String,
    pub relative_path: String,
    pub relations: Vec<String>,
    pub query_hits: Vec<String>,
    pub score: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRelatedFilesResponse {
    pub workspace_root: PathBuf,
    pub target: Option<TargetInfo>,
    pub query: String,
    pub scanned: usize,
    pub count: usize,
    /// True when the underlying workspace listing hit its file cap, so `files` is
    /// drawn from a lexicographically-first sample of the project rather than the
    /// whole tree — related-file ranking may therefore miss late-sorting matches.
    #[serde(default)]
    pub truncated: bool,
    pub files: Vec<RelatedFileResult>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    pub path: String,
    pub relative_path: String,
    pub basename: String,
    pub family_stem: String,
}

struct Desc {
    path: String,
    relative_path: String,
    lower: String,
    relative_lower: String,
    dir: String,
    relative_dir: String,
    basename: String,
    basename_lower: String,
    extension: String,
    stem_lower: String,
    family_stem_lower: String,
}

impl Desc {
    fn new(path: &str, root: &str) -> Self {
        let path = ai_semantic::normalize_slashes_pub(path);
        let root = ai_semantic::normalize_slashes_pub(root.trim_end_matches('/'));
        let relative_path = if !root.is_empty()
            && path
                .to_lowercase()
                .starts_with(&format!("{}/", root.to_lowercase()))
        {
            path.get(root.len() + 1..).unwrap_or(&path).to_string()
        } else {
            path.clone()
        };
        let basename = path.rsplit('/').next().unwrap_or(&path).to_string();
        let dir = if path.contains('/') {
            path[..path.rfind('/').unwrap()].to_string()
        } else {
            String::new()
        };
        let relative_dir = if relative_path.contains('/') {
            relative_path[..relative_path.rfind('/').unwrap()].to_string()
        } else {
            String::new()
        };
        let basename_lower = basename.to_lowercase();
        let extension = ai_semantic::file_extension_pub(&basename_lower);
        let stem_lower =
            basename_lower[..basename_lower.len().saturating_sub(extension.len())].to_string();
        let family_stem = ai_semantic::family_stem_pub(&basename);
        Self {
            lower: path.to_lowercase(),
            relative_lower: relative_path.to_lowercase(),
            path,
            relative_path,
            dir,
            relative_dir,
            basename_lower,
            basename,
            extension,
            stem_lower,
            family_stem_lower: family_stem.to_lowercase(),
        }
    }
}

#[tauri::command]
pub async fn ai_related_files(
    state: State<'_, SharedState>,
    path: Option<String>,
    query: Option<String>,
    max_results: Option<usize>,
    max_files: Option<usize>,
) -> Result<AiRelatedFilesResponse, String> {
    let root = workspace_root(&state)?;
    let root_str = ai_semantic::normalize_slashes_pub(&root.to_string_lossy());
    let query_str = query.unwrap_or_default().trim().to_string();
    let max_results = max_results.unwrap_or(40).clamp(1, 120);
    let file_cap = max_files.unwrap_or(5_000).clamp(500, 20_000);
    let tokens = ai_semantic::tokenize_pub(&query_str);

    let target_path = path
        .map(|p| ai_semantic::normalize_slashes_pub(p.trim()))
        .filter(|p| !p.is_empty())
        .map(|p| resolve_workspace_path_simple(&p, &root_str));
    let target = target_path.as_deref().map(|p| Desc::new(p, &root_str));

    let listing = {
        let root = root.clone();
        tokio::task::spawn_blocking(move || lux_fs::list_files_scanned(root, file_cap))
            .await
            .map_err(|e| e.to_string())?
    };
    let files = listing.entries;
    let truncated = listing.truncated;

    let file_count = files
        .iter()
        .filter(|e| matches!(e.kind, lux_core::FsEntryKind::File))
        .count();
    let mut matches: BTreeMap<String, RelatedFileResult> = BTreeMap::new();

    for entry in &files {
        if !matches!(entry.kind, lux_core::FsEntryKind::File) {
            continue;
        }
        let entry_path = ai_semantic::normalize_slashes_pub(&entry.path.to_string_lossy());
        if ai_semantic::is_low_signal_path_pub(&entry_path) {
            continue;
        }
        let desc = Desc::new(&entry_path, &root_str);
        if let Some(tgt) = &target {
            if desc.lower == tgt.lower {
                continue;
            }
        }
        let (score, relations, query_hits) = score_related(&desc, target.as_ref(), &tokens);
        if score <= 0 {
            continue;
        }
        let key = desc.path.clone();
        let existing = matches.get(&key);
        if existing.is_none_or(|e| e.score < score) {
            matches.insert(
                key,
                RelatedFileResult {
                    path: desc.path,
                    relative_path: desc.relative_path,
                    relations: relations.into_iter().collect(),
                    query_hits,
                    score,
                },
            );
        }
    }

    let mut ranked: Vec<RelatedFileResult> = matches.into_values().collect();
    ranked.sort_by(|a, b| {
        b.score.cmp(&a.score).then_with(|| {
            a.relative_path
                .to_lowercase()
                .cmp(&b.relative_path.to_lowercase())
        })
    });
    ranked.truncate(max_results);

    let target_info = target.map(|t| TargetInfo {
        path: t.path,
        relative_path: t.relative_path,
        basename: t.basename,
        family_stem: ai_semantic::family_stem_pub(&t.basename_lower),
    });

    Ok(AiRelatedFilesResponse {
        workspace_root: root,
        target: target_info,
        query: query_str,
        scanned: file_count,
        count: ranked.len(),
        truncated,
        files: ranked,
    })
}

fn resolve_workspace_path_simple(path: &str, root: &str) -> String {
    let normalized = ai_semantic::normalize_slashes_pub(path.trim());
    if root.is_empty() || normalized.starts_with('/') || normalized.chars().nth(1) == Some(':') {
        return normalized;
    }
    format!(
        "{}/{}",
        root.trim_end_matches('/'),
        normalized.trim_start_matches('/')
    )
}

fn score_related(
    desc: &Desc,
    target: Option<&Desc>,
    tokens: &[String],
) -> (i64, BTreeSet<String>, Vec<String>) {
    let mut relations = BTreeSet::new();
    let mut query_hits = Vec::new();
    let mut score: i64 = 0;

    if let Some(tgt) = target {
        let same_dir = desc.dir == tgt.dir;
        let same_family = !desc.family_stem_lower.is_empty()
            && !tgt.family_stem_lower.is_empty()
            && desc.family_stem_lower == tgt.family_stem_lower;
        let sibling_family = !desc.family_stem_lower.is_empty()
            && !tgt.family_stem_lower.is_empty()
            && (desc.stem_lower == tgt.family_stem_lower
                || desc.family_stem_lower.contains(&tgt.family_stem_lower)
                || tgt.family_stem_lower.contains(&desc.family_stem_lower));

        if same_dir {
            relations.insert("same-directory".to_string());
            score += 16;
        }
        if same_family {
            relations.insert("same-family".to_string());
            score += 42;
        } else if same_dir && sibling_family {
            score += 24;
        }

        let dir_dist = directory_distance(&tgt.relative_dir, &desc.relative_dir);
        score += (18 - dir_dist * 4).max(0);

        if same_dir && is_barrel(desc) {
            relations.insert("barrel".to_string());
            score += 25;
        }
        if same_dir
            && !tgt.family_stem_lower.is_empty()
            && desc.stem_lower.contains(&tgt.family_stem_lower)
            && desc.family_stem_lower != tgt.family_stem_lower
        {
            relations.insert("nearby-name".to_string());
            score += 12;
        }

        // Only award strong kind-relation boosts (test/style/type/route/story counterparts)
        // when the stem relationship is established.  `same_dir` alone is too broad:
        // it caused src/bar.test.ts, styles, routes, etc. to rank as companions of any
        // same-directory file even when they share nothing in common.  We keep same-dir
        // as a small proximity signal (the +16 above) but gate the larger boosts on a
        // real family/stem/source-extension relationship.
        let stem_related = same_family || sibling_family || is_source_counterpart(desc, tgt);
        if stem_related {
            score += add_kind_relations(desc, &mut relations);
        }

        if same_family && is_source_counterpart(desc, tgt) {
            relations.insert("nearby-name".to_string());
            score += 18;
        }
    } else {
        let kind_score = add_kind_relations(desc, &mut relations);
        score += kind_score.clamp(0, 20);
        if is_important(desc) {
            score += 35;
        }
    }

    for token in tokens {
        if desc.relative_lower.contains(token) {
            query_hits.push(token.clone());
            relations.insert("query-match".to_string());
            score += if token.len() >= 6 { 18 } else { 12 };
            if desc.basename_lower.contains(token) {
                score += 10;
            }
        }
    }

    if is_important(desc) {
        add_important_relation(desc, &mut relations);
        score += if target.is_some() { 14 } else { 30 };
    }
    if target.is_some() && query_hits.is_empty() && relations.is_empty() {
        return (0, relations, query_hits);
    }
    if desc.relative_lower.contains("/src/") || desc.relative_lower.starts_with("src/") {
        score += 4;
    }
    if desc.relative_lower.contains("/test") || desc.relative_lower.contains("/spec") {
        score += 4;
    }
    if std::path::Path::new(&desc.basename_lower)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("lock"))
    {
        score -= 20;
    }

    (score, relations, query_hits)
}

fn directory_distance(left: &str, right: &str) -> i64 {
    if left == right {
        return 0;
    }
    let lp: Vec<&str> = left.split('/').filter(|s| !s.is_empty()).collect();
    let rp: Vec<&str> = right.split('/').filter(|s| !s.is_empty()).collect();
    let mut common = 0;
    while common < lp.len() && common < rp.len() && lp[common] == rp[common] {
        common += 1;
    }
    i64::try_from((lp.len() - common) + (rp.len() - common)).unwrap_or(i64::MAX)
}

fn add_kind_relations(desc: &Desc, relations: &mut BTreeSet<String>) -> i64 {
    let mut score = 0;
    if is_test(desc) {
        relations.insert("test".into());
        score += 35;
    }
    if is_style(desc) {
        relations.insert("style".into());
        score += 30;
    }
    if is_type_def(desc) {
        relations.insert("type-definition".into());
        score += 28;
    }
    if is_route(desc) {
        relations.insert("route".into());
        score += 24;
    }
    if is_schema(desc) {
        relations.insert("schema".into());
        score += 24;
    }
    if is_config(desc) {
        relations.insert("config".into());
        score += 18;
    }
    if is_entrypoint(desc) {
        relations.insert("entrypoint".into());
        score += 18;
    }
    if is_story(desc) {
        relations.insert("story".into());
        score += 22;
    }
    if is_barrel(desc) {
        relations.insert("barrel".into());
        score += 14;
    }
    score
}

fn add_important_relation(desc: &Desc, relations: &mut BTreeSet<String>) {
    if is_config(desc) {
        relations.insert("config".into());
    }
    if is_entrypoint(desc) {
        relations.insert("entrypoint".into());
    }
    if desc.basename_lower.contains("readme")
        || desc.basename_lower.contains("license")
        || desc.basename_lower.contains("notice")
    {
        relations.insert("nearby-name".into());
    }
}

fn has_segment(base: &str, words: &[&str]) -> bool {
    base.split(['.', '_', '-']).any(|seg| words.contains(&seg))
}

fn is_test(d: &Desc) -> bool {
    has_segment(&d.basename_lower, &["test", "spec", "tests", "specs"])
        || d.relative_lower
            .split('/')
            .any(|seg| matches!(seg, "__tests__" | "test" | "tests" | "spec" | "specs"))
}
fn is_style(d: &Desc) -> bool {
    matches!(d.extension.as_str(), ".css" | ".scss" | ".sass" | ".less")
        || has_segment(&d.basename_lower, &["styles", "style", "theme", "tokens"])
}
fn is_type_def(d: &Desc) -> bool {
    d.basename_lower.ends_with(".d.ts")
        || d.basename_lower.ends_with(".d.mts")
        || d.basename_lower.ends_with(".d.cts")
        || has_segment(
            &d.basename_lower,
            &["types", "type", "interfaces", "interface", "dto", "defs"],
        )
}
fn is_route(d: &Desc) -> bool {
    has_segment(
        &d.basename_lower,
        &["route", "routes", "router", "page", "layout"],
    ) || d
        .relative_lower
        .split('/')
        .any(|seg| matches!(seg, "app" | "pages" | "routes" | "route"))
}
fn is_schema(d: &Desc) -> bool {
    has_segment(
        &d.basename_lower,
        &[
            "schema",
            "schemas",
            "model",
            "models",
            "entity",
            "entities",
            "migration",
            "prisma",
            "graphql",
            "proto",
        ],
    ) || matches!(
        d.extension.as_str(),
        ".graphql" | ".gql" | ".proto" | ".sql"
    )
}
fn is_config(d: &Desc) -> bool {
    has_segment(
        &d.basename_lower,
        &[
            "config",
            "conf",
            "rc",
            "settings",
            "eslint",
            "prettier",
            "vite",
            "webpack",
            "rollup",
            "tsconfig",
            "jsconfig",
            "cargo",
            "package",
            "pyproject",
        ],
    ) || d.relative_lower.contains("package.json")
        || d.relative_lower.contains("cargo.toml")
}
fn is_entrypoint(d: &Desc) -> bool {
    let patterns = [
        "main.ts",
        "main.tsx",
        "main.js",
        "main.jsx",
        "main.rs",
        "main.go",
        "main.py",
        "main.java",
        "index.ts",
        "index.tsx",
        "index.js",
        "index.jsx",
        "app.ts",
        "app.tsx",
        "app.js",
        "app.jsx",
        "lib.ts",
        "lib.rs",
        "mod.rs",
    ];
    patterns.iter().any(|p| d.basename_lower == *p)
        || d.relative_lower.ends_with("src/main.rs")
        || d.relative_lower.ends_with("src-tauri/src/lib.rs")
}
fn is_story(d: &Desc) -> bool {
    has_segment(&d.basename_lower, &["stories", "story"])
}
fn is_barrel(d: &Desc) -> bool {
    matches!(
        d.basename_lower.as_str(),
        "index.ts" | "index.tsx" | "index.js" | "index.jsx" | "mod.rs" | "lib.rs"
    )
}
fn is_important(d: &Desc) -> bool {
    const NAMES: &[&str] = &[
        "package.json",
        "cargo.toml",
        "pyproject.toml",
        "go.mod",
        "pom.xml",
        "build.gradle",
        "dockerfile",
        "makefile",
        ".env.example",
    ];
    const PREFIXES: &[&str] = &["vite.config.", "tsconfig.", "jsconfig."];
    NAMES
        .iter()
        .any(|n| d.relative_lower == *n || d.relative_lower.ends_with(&format!("/{n}")))
        || PREFIXES
            .iter()
            .any(|p| d.relative_lower.starts_with(p) || d.relative_lower.contains(&format!("/{p}")))
        || d.relative_lower.contains("readme")
}
fn is_source_counterpart(a: &Desc, b: &Desc) -> bool {
    const RELATED: &[&str] = &[
        ".ts", ".tsx", ".js", ".jsx", ".css", ".scss", ".sass", ".less", ".d.ts",
    ];
    if a.extension.to_lowercase() == b.extension.to_lowercase() {
        return false;
    }
    RELATED.contains(&a.extension.to_lowercase().as_str())
        && RELATED.contains(&b.extension.to_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn related_scoring_same_family() {
        let target = Desc::new("/root/src/userProfile.tsx", "/root");
        let test = Desc::new("/root/src/userProfile.test.tsx", "/root");
        let (score, relations, _) = score_related(&test, Some(&target), &[]);
        assert!(score > 50, "test companion should score high: {score}");
        assert!(relations.contains("test"));
    }

    #[test]
    fn related_scoring_query_hit() {
        let desc = Desc::new("/root/src/auth/login.ts", "/root");
        let (score, relations, hits) = score_related(&desc, None, &["login".to_string()]);
        assert!(score > 0);
        assert!(relations.contains("query-match"));
        assert!(hits.contains(&"login".to_string()));
    }

    #[test]
    fn related_scoring_unrelated_file_zero() {
        let target = Desc::new("/root/src/app.ts", "/root");
        let unrelated = Desc::new("/root/docs/changelog.md", "/root");
        let (score, _, _) = score_related(&unrelated, Some(&target), &[]);
        assert_eq!(score, 0, "unrelated file with no hits should be 0");
    }

    #[test]
    fn same_dir_unrelated_test_file_no_kind_boost() {
        // A test file in the same directory as `app.ts` should NOT receive the
        // strong "test" kind-relation boost unless it shares the family stem.
        // Before the fix, `src/bar.test.ts` alongside `src/app.ts` would get +35
        // from add_kind_relations even though the stems are unrelated.
        let target = Desc::new("/root/src/app.ts", "/root");
        let unrelated_test = Desc::new("/root/src/bar.test.ts", "/root");
        let (score, relations, _) = score_related(&unrelated_test, Some(&target), &[]);
        assert!(
            !relations.contains("test"),
            "unrelated same-dir test should not carry 'test' relation, got {relations:?}"
        );
        // The file is still in the same dir so it gets the small proximity signal,
        // but not the large kind-relation boost.
        assert!(
            score < 50,
            "unrelated same-dir test companion score should be modest, got {score}"
        );
    }

    #[test]
    fn same_family_test_file_keeps_kind_boost() {
        // Companion test with the same family stem must still get the full boost.
        let target = Desc::new("/root/src/userProfile.tsx", "/root");
        let companion_test = Desc::new("/root/src/userProfile.test.tsx", "/root");
        let (score, relations, _) = score_related(&companion_test, Some(&target), &[]);
        assert!(
            relations.contains("test"),
            "'test' relation must be present for family companion"
        );
        assert!(score > 50, "family test companion must score high: {score}");
    }
}
