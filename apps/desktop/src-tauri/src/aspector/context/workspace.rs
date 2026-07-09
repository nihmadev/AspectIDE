//! Native `RepoMap` + `WorkspaceIndex` tools — Stage 1.
//!
//! `RepoMap`: sorted list of important workspace files by path-score.
//! `WorkspaceIndex`: categorized snapshot (language mix, directories, important/test/
//! source/entrypoint/largest files). Both compose `aspect_fs` `list_files` natively.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::{workspace_root, SharedState};
use aspect_core::monaco_language_id_for_path;

// ── RepoMap ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMapFile {
    pub path: String,
    pub relative_path: String,
    pub size: u64,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRepoMapResponse {
    pub total_listed: usize,
    pub truncated: bool,
    /// Present only when the scan hit its cap: a human-readable caveat that the map
    /// ranks the lexicographically-first scanned files and may omit whole subtrees
    /// that sort after the cutoff.
    pub note: Option<String>,
    pub files: Vec<RepoMapFile>,
}

#[tauri::command]
pub async fn ai_repo_map(
    state: State<'_, SharedState>,
    max_files: Option<usize>,
) -> Result<AiRepoMapResponse, String> {
    let root = workspace_root(&state)?;
    // A6: capture the normalized workspace root before `root` is moved into the
    // blocking scan, so emitted paths can be presented forward-slashed +
    // workspace-relative exactly like WorkspaceIndex / the other orient tools.
    let root_str = crate::aspector::context::semantic::normalize_slashes_pub(&root.to_string_lossy());
    let max = max_files.unwrap_or(80).clamp(1, 500);
    // A5: use the scanned variant so we can tell the model when the map is a
    // lexicographically-first sample rather than the whole project.
    let scan_cap = max.max(500);
    let listing = spawn_list_files_scanned(root, scan_cap).await?;
    let entries = listing.entries;
    let mut files: Vec<_> = entries
        .iter()
        .filter(|e| matches!(e.kind, aspect_core::FsEntryKind::File))
        // F37: drop low-signal artifact paths (node_modules, dist/out, lockfiles, …)
        // exactly as `ai_workspace_index` already does — otherwise the repo map's
        // top-N is polluted by build output that `ai_workspace_index` filters out,
        // an inconsistency between the two views of the same tree.
        .filter_map(|e| {
            let path_str = crate::aspector::context::semantic::normalize_slashes_pub(&e.path.to_string_lossy());
            if crate::aspector::context::semantic::is_low_signal_path_pub(&path_str) {
                return None;
            }
            let score = crate::aspector::context::semantic::score_path_pub(&path_str);
            Some((e, score))
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));
    files.truncate(max);
    // A5: mirror ai_workspace_index — when the scan hit its cap the ranking only saw
    // the lexicographically-first slice of the tree, so signal that the map is partial.
    let truncated = listing.truncated;
    let note = truncated.then(|| {
        format!(
            "Workspace exceeds the {scan_cap}-file scan cap; the map ranks the \
             lexicographically-first {} files and may omit whole subtrees that sort later.",
            entries.len()
        )
    });
    let root_prefix = format!("{}/", root_str.trim_end_matches('/'));
    Ok(AiRepoMapResponse {
        total_listed: entries.len(),
        truncated,
        note,
        files: files
            .into_iter()
            .map(|(e, _)| {
                // A6: forward-slash + workspace-relative, matching WorkspaceIndexFile.
                let path = crate::aspector::context::semantic::normalize_slashes_pub(&e.path.to_string_lossy());
                let relative_path = path
                    .strip_prefix(&root_prefix)
                    .map_or_else(|| path.clone(), str::to_string);
                RepoMapFile {
                    path,
                    relative_path,
                    size: e.size,
                    modified_at: e.modified_at.map(|dt| dt.to_rfc3339()),
                }
            })
            .collect(),
    })
}

// ── WorkspaceIndex ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIndexFile {
    pub path: String,
    pub relative_path: String,
    pub language: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiWorkspaceIndexResponse {
    pub workspace_root: PathBuf,
    pub scanned: usize,
    pub indexed_files: usize,
    pub truncated: bool,
    /// Present only when the scan was truncated: a human-readable caveat that the
    /// `by_language`/`by_directory`/`largest` aggregates are computed over the
    /// lexicographically-first scanned files and may not represent the full project.
    pub aggregates_note: Option<String>,
    pub by_language: Vec<CountEntry>,
    pub by_directory: Vec<CountEntry>,
    pub important: Vec<WorkspaceIndexFile>,
    pub tests: Vec<WorkspaceIndexFile>,
    pub source: Vec<WorkspaceIndexFile>,
    pub entrypoints: Vec<WorkspaceIndexFile>,
    pub largest: Vec<WorkspaceIndexFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CountEntry {
    pub key: String,
    pub count: usize,
}

struct FileDesc {
    path: String,
    relative_path: String,
    relative_lower: String,
    basename_lower: String,
    extension: String,
    size: u64,
}

impl FileDesc {
    fn new(path: &str, root: &str, size: u64) -> Self {
        let path = crate::aspector::context::semantic::normalize_slashes_pub(path);
        let root_norm = crate::aspector::context::semantic::normalize_slashes_pub(root.trim_end_matches('/'));
        let root_lower = root_norm.to_lowercase();
        let relative_path = if !root_norm.is_empty()
            && path.to_lowercase().starts_with(&format!("{root_lower}/"))
        {
            path.get(root_norm.len() + 1..).unwrap_or(&path).to_string()
        } else {
            path.clone()
        };
        let basename = path.rsplit('/').next().unwrap_or(&path).to_string();
        let extension = crate::aspector::context::semantic::file_extension_pub(&basename.to_lowercase());
        Self {
            relative_lower: relative_path.to_lowercase(),
            basename_lower: basename.to_lowercase(),
            path,
            relative_path,
            extension,
            size,
        }
    }

    fn language(&self) -> String {
        crate::aspector::context::semantic::language_for_path_pub(&self.basename_lower)
    }

    fn top_directory(&self) -> String {
        let parts: Vec<&str> = self
            .relative_path
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        if parts.is_empty() {
            return ".".to_string();
        }
        if parts[0].starts_with('.') {
            return parts[0].to_string();
        }
        if parts[0] == "src" {
            return "src".to_string();
        }
        if parts.len() > 1 && matches!(parts[0], "apps" | "crates" | "packages") {
            return format!("{}/{}", parts[0], parts[1]);
        }
        parts[0].to_string()
    }

    fn is_important(&self) -> bool {
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
            .any(|n| self.relative_lower == *n || self.relative_lower.ends_with(&format!("/{n}")))
            || PREFIXES.iter().any(|p| {
                self.relative_lower.starts_with(p) || self.relative_lower.contains(&format!("/{p}"))
            })
            || self.relative_lower.contains("readme")
    }

    fn is_test(&self) -> bool {
        self.basename_lower
            .split(['.', '_', '-'])
            .any(|seg| matches!(seg, "test" | "spec" | "tests" | "specs"))
            || self
                .relative_lower
                .split('/')
                .any(|seg| matches!(seg, "__tests__" | "test" | "tests" | "spec" | "specs"))
    }

    fn is_source(&self) -> bool {
        self.relative_lower.contains("/src/")
            || self.relative_lower.starts_with("src/")
            || matches!(
                self.extension.as_str(),
                ".ts"
                    | ".tsx"
                    | ".js"
                    | ".jsx"
                    | ".rs"
                    | ".py"
                    | ".go"
                    | ".java"
                    | ".kt"
                    | ".cs"
                    | ".vue"
                    | ".svelte"
                    | ".astro"
            )
    }

    fn is_entrypoint(&self) -> bool {
        const ENTRYPOINTS: &[&str] = &[
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
        ENTRYPOINTS.contains(&self.basename_lower.as_str())
            || self.relative_lower.ends_with("src/main.rs")
            || self.relative_lower.ends_with("src-tauri/src/lib.rs")
    }

    fn to_index_file(&self) -> WorkspaceIndexFile {
        WorkspaceIndexFile {
            path: self.path.clone(),
            relative_path: self.relative_path.clone(),
            language: self.language(),
            size: self.size,
        }
    }
}

#[tauri::command]
pub async fn ai_workspace_index(
    state: State<'_, SharedState>,
    max_files: Option<usize>,
    max_scan: Option<usize>,
) -> Result<AiWorkspaceIndexResponse, String> {
    let root = workspace_root(&state)?;
    let root_str = crate::aspector::context::semantic::normalize_slashes_pub(&root.to_string_lossy());
    let max_files = max_files.unwrap_or(60).clamp(1, 180);
    let max_scan = max_scan.unwrap_or(5_000).clamp(500, 20_000);
    let entries = spawn_list_files(root.clone(), max_scan).await?;

    let descs: Vec<FileDesc> = entries
        .iter()
        .filter(|e| matches!(e.kind, aspect_core::FsEntryKind::File))
        .map(|e| {
            (
                crate::aspector::context::semantic::normalize_slashes_pub(&e.path.to_string_lossy()),
                e.size,
            )
        })
        .filter(|(p, _)| !crate::aspector::context::semantic::is_low_signal_path_pub(p))
        .map(|(p, size)| FileDesc::new(&p, &root_str, size))
        .collect();

    let by_language = top_counts(descs.iter().map(FileDesc::language), 20);
    let by_directory = top_counts(descs.iter().map(FileDesc::top_directory), 24);

    let mut important: Vec<&FileDesc> = descs.iter().filter(|d| d.is_important()).collect();
    important.sort_by(|a, b| {
        crate::aspector::context::semantic::score_path_pub(&b.relative_path)
            .cmp(&crate::aspector::context::semantic::score_path_pub(&a.relative_path))
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
    });
    important.truncate(max_files);

    let mut tests: Vec<&FileDesc> = descs.iter().filter(|d| d.is_test()).collect();
    tests.sort_by(|a, b| {
        crate::aspector::context::semantic::score_path_pub(&b.relative_path)
            .cmp(&crate::aspector::context::semantic::score_path_pub(&a.relative_path))
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
    });
    tests.truncate(max_files);

    let mut source: Vec<&FileDesc> = descs
        .iter()
        .filter(|d| d.is_source() && !d.is_test())
        .collect();
    source.sort_by(|a, b| {
        crate::aspector::context::semantic::score_path_pub(&b.relative_path)
            .cmp(&crate::aspector::context::semantic::score_path_pub(&a.relative_path))
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
    });
    source.truncate(max_files);

    let mut entrypoints: Vec<&FileDesc> = descs.iter().filter(|d| d.is_entrypoint()).collect();
    entrypoints.sort_by(|a, b| {
        crate::aspector::context::semantic::score_path_pub(&b.relative_path)
            .cmp(&crate::aspector::context::semantic::score_path_pub(&a.relative_path))
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
    });
    entrypoints.truncate(max_files);

    let mut largest: Vec<&FileDesc> = descs.iter().collect();
    largest.sort_by(|a, b| {
        b.size
            .cmp(&a.size)
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
    });
    largest.truncate(max_files.min(20));

    // F34: when the scan hit its cap the aggregates below are computed over the
    // lexicographically-first slice of the tree, not the whole project. Surface that
    // caveat instead of presenting a biased sample as authoritative.
    let truncated = entries.len() >= max_scan;
    let aggregates_note = truncated.then(|| {
        format!(
            "Workspace exceeds the {max_scan}-file scan cap; by_language, by_directory and \
             largest are computed over the {} lexicographically-first files and may not \
             represent the full project.",
            entries.len()
        )
    });

    Ok(AiWorkspaceIndexResponse {
        workspace_root: root,
        scanned: entries.len(),
        indexed_files: descs.len(),
        truncated,
        aggregates_note,
        by_language,
        by_directory,
        important: important.iter().map(|d| d.to_index_file()).collect(),
        tests: tests.iter().map(|d| d.to_index_file()).collect(),
        source: source.iter().map(|d| d.to_index_file()).collect(),
        entrypoints: entrypoints.iter().map(|d| d.to_index_file()).collect(),
        largest: largest.iter().map(|d| d.to_index_file()).collect(),
    })
}

fn top_counts(iter: impl Iterator<Item = String>, limit: usize) -> Vec<CountEntry> {
    let mut map: BTreeMap<String, usize> = BTreeMap::new();
    for item in iter {
        *map.entry(item).or_default() += 1;
    }
    let mut entries: Vec<CountEntry> = map
        .into_iter()
        .map(|(key, count)| CountEntry { key, count })
        .collect();
    entries.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    entries.truncate(limit);
    entries
}

async fn spawn_list_files(root: PathBuf, max: usize) -> Result<Vec<aspect_core::FsEntry>, String> {
    tokio::task::spawn_blocking(move || aspect_fs::list_files(root, max))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// Like [`spawn_list_files`] but preserves the exact `truncated` flag so callers can
/// tell the model when the listing is only the lexicographically-first slice of a
/// larger workspace. `list_files_scanned` returns [`aspect_fs::FileListing`] directly
/// (never `Err`), so only the join error is mapped.
async fn spawn_list_files_scanned(
    root: PathBuf,
    max: usize,
) -> Result<aspect_fs::FileListing, String> {
    tokio::task::spawn_blocking(move || aspect_fs::list_files_scanned(root, max))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resolve_file_languages(paths: Vec<String>) -> Result<Vec<String>, String> {
    Ok(paths
        .iter()
        .map(|p| monaco_language_id_for_path(std::path::Path::new(p)))
        .collect())
}

fn language_label(path: &str) -> String {
    let p = std::path::Path::new(path);
    let lower_name = p
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if matches!(lower_name.as_str(), "dockerfile" | "containerfile") {
        return "dockerfile".to_string();
    }
    match p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("rs") => "rust",
        Some("ts" | "tsx" | "mts" | "cts") => "typescript",
        Some("js" | "jsx" | "mjs" | "cjs") => "javascript",
        Some("py" | "pyw") => "python",
        Some("go" | "mod") => "go",
        Some("java") => "java",
        Some("kt" | "kts") => "kotlin",
        Some("cs") => "csharp",
        Some("fs" | "fsx") => "fsharp",
        Some("cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx") => "cpp",
        Some("c" | "h") => "c",
        Some("rb") => "ruby",
        Some("php") => "php",
        Some("swift") => "swift",
        Some("scala") => "scala",
        Some("dart") => "dart",
        Some("lua") => "lua",
        Some("zig") => "zig",
        Some("nim") => "nim",
        Some("r") => "r",
        Some("jl") => "julia",
        Some("hs" | "lhs") => "haskell",
        Some("clj" | "cljs" | "cljc") => "clojure",
        Some("ex" | "exs") => "elixir",
        Some("erl" | "hrl") => "erlang",
        Some("pl" | "pm") => "perl",
        Some("sh" | "bash" | "zsh" | "fish" | "ksh") => "shell",
        Some("ps1" | "psm1" | "psd1") => "powershell",
        Some("bat" | "cmd") => "bat",
        Some("tf" | "tfvars" | "hcl" | "nomad") => "hcl",
        Some("json" | "jsonc" | "json5" | "jsonl") => "json",
        Some("toml" | "tml") => "toml",
        Some("yaml" | "yml") => "yaml",
        Some("css" | "scss" | "sass" | "less") => "css",
        Some("html" | "htm" | "vue" | "svelte" | "astro") => "html",
        Some("sql" | "ddl" | "dml") => "sql",
        Some("xml" | "xsd" | "xsl" | "xslt") => "xml",
        Some("csv" | "tsv" | "psv") => "csv",
        Some("graphql" | "gql") => "graphql",
        Some("proto") => "proto",
        Some("prisma") => "prisma",
        Some("md" | "mdx" | "markdown" | "rst" | "org") => "markdown",
        Some("ini" | "cfg" | "conf" | "editorconfig") => "ini",
        Some(ext) => ext,
        None => "other",
    }
    .to_string()
}

#[tauri::command]
pub async fn ai_index_languages(
    state: State<'_, SharedState>,
) -> Result<Vec<CountEntry>, String> {
    let root = workspace_root(&state)?;
    let entries = spawn_list_files(root, 20_000).await?;
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in &entries {
        if entry.kind != aspect_core::FsEntryKind::File {
            continue;
        }
        let path = crate::aspector::context::semantic::normalize_slashes_pub(&entry.path.to_string_lossy());
        let lang = language_label(&path);
        *counts.entry(lang).or_default() += 1;
    }
    let mut result: Vec<CountEntry> = counts
        .into_iter()
        .map(|(key, count)| CountEntry { key, count })
        .collect();
    result.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.key.cmp(&b.key)));
    result.truncate(20);
    Ok(result)
}

