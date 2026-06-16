//! Native `RepoMap` + `WorkspaceIndex` tools — Stage 1.
//!
//! `RepoMap`: sorted list of important workspace files by path-score.
//! `WorkspaceIndex`: categorized snapshot (language mix, directories, important/test/
//! source/entrypoint/largest files). Both compose `lux_fs` `list_files` natively.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use tauri::State;

use crate::ai_semantic;
use crate::{workspace_root, SharedState};

// ── RepoMap ──

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoMapFile {
    pub path: PathBuf,
    pub size: u64,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRepoMapResponse {
    pub total_listed: usize,
    pub files: Vec<RepoMapFile>,
}

#[tauri::command]
pub async fn ai_repo_map(
    state: State<'_, SharedState>,
    max_files: Option<usize>,
) -> Result<AiRepoMapResponse, String> {
    let root = workspace_root(&state)?;
    let max = max_files.unwrap_or(80).clamp(1, 500);
    let entries = spawn_list_files(root, max.max(500)).await?;
    let mut files: Vec<_> = entries
        .iter()
        .filter(|e| matches!(e.kind, lux_core::FsEntryKind::File))
        .map(|e| {
            let path_str = ai_semantic::normalize_slashes_pub(&e.path.to_string_lossy());
            let score = ai_semantic::score_path_pub(&path_str);
            (e, score)
        })
        .collect();
    files.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.path.cmp(&b.0.path)));
    files.truncate(max);
    Ok(AiRepoMapResponse {
        total_listed: entries.len(),
        files: files
            .into_iter()
            .map(|(e, _)| RepoMapFile {
                path: e.path.clone(),
                size: e.size,
                modified_at: e.modified_at.map(|dt| dt.to_rfc3339()),
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
        let path = ai_semantic::normalize_slashes_pub(path);
        let root_norm = ai_semantic::normalize_slashes_pub(root.trim_end_matches('/'));
        let root_lower = root_norm.to_lowercase();
        let relative_path = if !root_norm.is_empty()
            && path.to_lowercase().starts_with(&format!("{root_lower}/"))
        {
            path.get(root_norm.len() + 1..).unwrap_or(&path).to_string()
        } else {
            path.clone()
        };
        let basename = path.rsplit('/').next().unwrap_or(&path).to_string();
        let extension = ai_semantic::file_extension_pub(&basename.to_lowercase());
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
        ai_semantic::language_for_path_pub(&self.basename_lower)
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
    let root_str = ai_semantic::normalize_slashes_pub(&root.to_string_lossy());
    let max_files = max_files.unwrap_or(60).clamp(1, 180);
    let max_scan = max_scan.unwrap_or(5_000).clamp(500, 20_000);
    let entries = spawn_list_files(root.clone(), max_scan).await?;

    let descs: Vec<FileDesc> = entries
        .iter()
        .filter(|e| matches!(e.kind, lux_core::FsEntryKind::File))
        .map(|e| {
            (
                ai_semantic::normalize_slashes_pub(&e.path.to_string_lossy()),
                e.size,
            )
        })
        .filter(|(p, _)| !ai_semantic::is_low_signal_path_pub(p))
        .map(|(p, size)| FileDesc::new(&p, &root_str, size))
        .collect();

    let by_language = top_counts(descs.iter().map(FileDesc::language), 20);
    let by_directory = top_counts(descs.iter().map(FileDesc::top_directory), 24);

    let mut important: Vec<&FileDesc> = descs.iter().filter(|d| d.is_important()).collect();
    important.sort_by(|a, b| {
        ai_semantic::score_path_pub(&b.relative_path)
            .cmp(&ai_semantic::score_path_pub(&a.relative_path))
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
    });
    important.truncate(max_files);

    let mut tests: Vec<&FileDesc> = descs.iter().filter(|d| d.is_test()).collect();
    tests.sort_by(|a, b| {
        ai_semantic::score_path_pub(&b.relative_path)
            .cmp(&ai_semantic::score_path_pub(&a.relative_path))
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
    });
    tests.truncate(max_files);

    let mut source: Vec<&FileDesc> = descs
        .iter()
        .filter(|d| d.is_source() && !d.is_test())
        .collect();
    source.sort_by(|a, b| {
        ai_semantic::score_path_pub(&b.relative_path)
            .cmp(&ai_semantic::score_path_pub(&a.relative_path))
            .then_with(|| a.relative_lower.cmp(&b.relative_lower))
    });
    source.truncate(max_files);

    let mut entrypoints: Vec<&FileDesc> = descs.iter().filter(|d| d.is_entrypoint()).collect();
    entrypoints.sort_by(|a, b| {
        ai_semantic::score_path_pub(&b.relative_path)
            .cmp(&ai_semantic::score_path_pub(&a.relative_path))
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

    Ok(AiWorkspaceIndexResponse {
        workspace_root: root,
        scanned: entries.len(),
        indexed_files: descs.len(),
        truncated: entries.len() >= max_scan,
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

async fn spawn_list_files(root: PathBuf, max: usize) -> Result<Vec<lux_core::FsEntry>, String> {
    tokio::task::spawn_blocking(move || lux_fs::list_files(root, max))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_counts_sorts_and_truncates() {
        let items = vec![
            "rust",
            "rust",
            "typescript",
            "typescript",
            "typescript",
            "python",
        ]
        .into_iter()
        .map(str::to_string);
        let counts = top_counts(items, 2);
        assert_eq!(counts.len(), 2);
        assert_eq!(counts[0].key, "typescript");
        assert_eq!(counts[0].count, 3);
        assert_eq!(counts[1].key, "rust");
    }

    #[test]
    fn file_desc_categorization() {
        let d = FileDesc::new("/root/src/app.test.tsx", "/root", 100);
        assert!(d.is_test());
        assert!(d.is_source());
        assert!(!d.is_entrypoint());

        let e = FileDesc::new("/root/src/main.rs", "/root", 200);
        assert!(e.is_entrypoint());
        assert!(!e.is_test());

        let p = FileDesc::new("/root/package.json", "/root", 50);
        assert!(p.is_important());
    }
}
