use std::{collections::BTreeSet, env, path::Path};

use lux_core::{
    AppResult, DiagnosticSeverity, LanguageServerInfo, LanguageServerStatus, WorkspaceDiagnostic,
};

#[derive(Debug, Clone, Copy)]
pub struct BuiltinServer {
    pub language_id: &'static str,
    pub name: &'static str,
    pub command: &'static str,
    pub args: &'static [&'static str],
    pub extensions: &'static [&'static str],
}

/// Popular-language server catalog.
///
/// The single source of truth shared by discovery (what to look for) and the
/// managed installer (what to install and how). Keep `command` aligned with the
/// binary each install method produces.
pub const BUILTIN_SERVERS: &[BuiltinServer] = &[
    BuiltinServer {
        language_id: "rust",
        name: "Rust Language Server",
        command: "rust-analyzer",
        args: &[],
        extensions: &["rs"],
    },
    BuiltinServer {
        language_id: "typescript",
        name: "TypeScript Language Server",
        command: "typescript-language-server",
        args: &["--stdio"],
        extensions: &["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"],
    },
    BuiltinServer {
        language_id: "python",
        name: "Python Language Server",
        command: "ty",
        args: &["server"],
        extensions: &["py", "pyi"],
    },
    BuiltinServer {
        language_id: "go",
        name: "Go Language Server",
        command: "gopls",
        args: &[],
        extensions: &["go"],
    },
    BuiltinServer {
        language_id: "json",
        name: "JSON Language Server",
        command: "vscode-json-language-server",
        args: &["--stdio"],
        extensions: &["json", "jsonc"],
    },
    BuiltinServer {
        language_id: "html",
        name: "HTML Language Server",
        command: "vscode-html-language-server",
        args: &["--stdio"],
        extensions: &["html", "htm"],
    },
    BuiltinServer {
        language_id: "css",
        name: "CSS Language Server",
        command: "vscode-css-language-server",
        args: &["--stdio"],
        extensions: &["css", "scss", "less"],
    },
    BuiltinServer {
        language_id: "yaml",
        name: "YAML Language Server",
        command: "yaml-language-server",
        args: &["--stdio"],
        extensions: &["yaml", "yml"],
    },
    BuiltinServer {
        language_id: "bash",
        name: "Bash Language Server",
        command: "bash-language-server",
        args: &["start"],
        extensions: &["sh", "bash", "zsh"],
    },
    BuiltinServer {
        language_id: "lua",
        name: "Lua Language Server",
        command: "lua-language-server",
        args: &[],
        extensions: &["lua"],
    },
    BuiltinServer {
        language_id: "cpp",
        name: "C/C++ Language Server",
        command: "clangd",
        args: &[],
        extensions: &["c", "h", "cc", "cpp", "cxx", "hpp", "hxx"],
    },
];

/// Well-known manifest files that definitively imply a language, mapped to a
/// representative extension one of its builtin servers handles. Probed at the
/// workspace root BEFORE the capped walk so a language whose source files happen
/// to sort after the entry cap (in a large monorepo) is still detected and its
/// server started — the walk-truncation blind spot. Each extension here must
/// appear in some [`BUILTIN_SERVERS`] entry.
const MANIFEST_EXTENSION_HINTS: &[(&str, &str)] = &[
    ("Cargo.toml", "rs"),
    ("rust-toolchain.toml", "rs"),
    ("tsconfig.json", "ts"),
    ("jsconfig.json", "js"),
    ("package.json", "js"),
    ("pyproject.toml", "py"),
    ("setup.py", "py"),
    ("requirements.txt", "py"),
    ("Pipfile", "py"),
    ("go.mod", "go"),
];

pub fn workspace_language_servers(root: impl AsRef<Path>) -> AppResult<Vec<LanguageServerInfo>> {
    workspace_language_servers_with_dirs(root, &[])
}

/// Discover servers for the workspace, resolving each command against `extra_dirs`
/// (e.g. the IDE's managed LSP bin directory) FIRST, then PATH.
///
/// This is how on-demand-installed servers become "Available" without the user
/// touching PATH.
pub fn workspace_language_servers_with_dirs(
    root: impl AsRef<Path>,
    extra_dirs: &[std::path::PathBuf],
) -> AppResult<Vec<LanguageServerInfo>> {
    // dunce, not std: a Windows `\\?\` verbatim root would flow into every
    // server's `initialize` rootUri/workspaceFolders as `file:////%3F/...`,
    // silently breaking project indexing (workspace/symbol returns nothing).
    let root = dunce::canonicalize(root.as_ref())?;
    let (detected_extensions, truncated) = detect_extensions_bounded(&root);
    if truncated {
        // Not silent: a too-large tree means an extension-only language past the
        // entry cap could be missed. Manifest-backed languages are still detected.
        eprintln!(
            "lux-lsp: workspace language discovery hit its entry cap in {} — \
             languages without a root manifest may be undetected; \
             configure servers in Settings → Language Servers if one is missing.",
            root.display()
        );
    }
    let mut servers = Vec::new();

    for server in BUILTIN_SERVERS {
        if !server
            .extensions
            .iter()
            .any(|extension| detected_extensions.contains(*extension))
        {
            continue;
        }

        let resolved = resolve_command(server.command, extra_dirs);
        let available = resolved.is_some();
        servers.push(LanguageServerInfo {
            language_id: server.language_id.to_string(),
            name: server.name.to_string(),
            // Use the absolute managed-dir path when found there, so the manager
            // spawns the installed binary directly regardless of PATH.
            command: resolved.map_or_else(
                || server.command.to_string(),
                |path| path.to_string_lossy().to_string(),
            ),
            args: server.args.iter().map(|arg| (*arg).to_string()).collect(),
            workspace_root: root.clone(),
            status: if available {
                LanguageServerStatus::Available
            } else {
                LanguageServerStatus::Missing
            },
            error: if available {
                None
            } else {
                Some(format!(
                    "{} is not installed — install it from Settings → Language Servers",
                    server.command
                ))
            },
        });
    }

    Ok(servers)
}

/// Resolve a server command to an absolute executable path, searching `extra_dirs`
/// (managed install location) before PATH. Returns None if not found anywhere.
fn resolve_command(command: &str, extra_dirs: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 {
        return command_path.is_file().then(|| command_path.to_path_buf());
    }
    for dir in extra_dirs {
        if let Some(path) = executable_in_dir(dir, command) {
            return Some(path);
        }
    }
    let paths = env::var_os("PATH")?;
    env::split_paths(&paths).find_map(|dir| executable_in_dir(&dir, command))
}

#[must_use]
pub fn language_server_diagnostics(servers: &[LanguageServerInfo]) -> Vec<WorkspaceDiagnostic> {
    servers
        .iter()
        .filter(|server| server.status == LanguageServerStatus::Missing)
        .map(|server| WorkspaceDiagnostic {
            path: diagnostic_anchor_path(server),
            line: 1,
            column: 1,
            severity: DiagnosticSeverity::Warning,
            source: "lux-lsp".to_string(),
            message: server
                .error
                .clone()
                .unwrap_or_else(|| format!("{} is configured but unavailable", server.command)),
        })
        .collect()
}

pub fn diagnostic_anchor_path(server: &LanguageServerInfo) -> std::path::PathBuf {
    let candidates: &[&str] = match server.language_id.as_str() {
        "rust" => &["rust-toolchain.toml", "Cargo.toml"],
        "typescript" | "javascript" => &["tsconfig.json", "jsconfig.json", "package.json"],
        "json" => &["package.json"],
        _ => &[],
    };

    for candidate in candidates {
        let path = server.workspace_root.join(candidate);
        if path.is_file() {
            return path;
        }
    }

    server.workspace_root.clone()
}

/// Set of every file extension any builtin server cares about. Once we've seen
/// all of them we can stop walking — no point scanning a huge repo further.
fn relevant_extensions() -> BTreeSet<&'static str> {
    BUILTIN_SERVERS
        .iter()
        .flat_map(|server| server.extensions.iter().copied())
        .collect()
}

/// Directory names that never contain workspace source we'd start a server for,
/// and that are commonly enormous. Skipping them keeps discovery fast and bounded.
fn is_ignored_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules"
            | "target"
            | ".git"
            | "dist"
            | "build"
            | "out"
            | ".next"
            | ".turbo"
            | ".cache"
            | "vendor"
            | ".venv"
            | "venv"
            | "__pycache__"
    )
}

/// Detects which builtin-relevant file extensions exist in the workspace.
///
/// Infallible (a bad entry is skipped, never aborts the scan) and **bounded**:
/// it stops as soon as every extension a builtin server handles has been seen,
/// and caps the total number of entries visited so a giant or pathological tree
/// can never stall language-service startup. This is the dominant fix for the
/// "language service hangs on load" symptom on large repos.
///
/// Convenience wrapper that drops the `truncated` flag; only the tests need the
/// flag-free form (production discovery uses [`detect_extensions_bounded`]).
#[cfg(test)]
fn detect_extensions(root: &Path) -> BTreeSet<String> {
    detect_extensions_bounded(root).0
}

/// Detect extensions, returning `(found, truncated)` where `truncated` is true if
/// the entry cap was hit before the tree was fully walked. Manifest-backed
/// languages are seeded first so they survive truncation; the flag lets callers
/// warn that extension-only languages past the cap may have been missed.
fn detect_extensions_bounded(root: &Path) -> (BTreeSet<String>, bool) {
    // Cap on directory entries visited — generous for real projects, but a hard
    // ceiling so discovery returns promptly no matter the repo size.
    const MAX_ENTRIES: usize = 20_000;

    let wanted = relevant_extensions();
    let mut found: BTreeSet<String> = BTreeSet::new();

    // Manifest-first: a root manifest is authoritative and cheap, so detect those
    // languages up front. This closes the gap where a language's source files sort
    // after MAX_ENTRIES in a huge repo and would otherwise be reported as absent.
    for (manifest, extension) in MANIFEST_EXTENSION_HINTS {
        if root.join(manifest).is_file() && wanted.contains(extension) {
            found.insert((*extension).to_string());
        }
    }
    if found.len() == wanted.len() {
        return (found, false);
    }

    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0_usize;

    while let Some(path) = stack.pop() {
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };

        for child in children.flatten() {
            visited += 1;
            if visited > MAX_ENTRIES {
                // Walk truncated: anything not already seeded above may be missing.
                return (found, true);
            }

            let file_name = child.file_name();
            let file_name = file_name.to_string_lossy();
            if is_ignored_dir(&file_name) || file_name.starts_with('.') {
                continue;
            }

            let Ok(file_type) = child.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(child.path());
            } else if file_type.is_file() {
                if let Some(extension) = child.path().extension().and_then(|value| value.to_str()) {
                    let extension = extension.to_ascii_lowercase();
                    if wanted.contains(extension.as_str()) {
                        found.insert(extension);
                        // Every server we could start is accounted for — stop early.
                        if found.len() == wanted.len() {
                            return (found, false);
                        }
                    }
                }
            }
        }
    }

    (found, false)
}

/// Whether `path` points at a file we could actually execute. On Unix this
/// requires at least one execute bit to be set, so a same-named *non-executable*
/// data file sitting in PATH is not mistaken for an available server. On other
/// platforms executability is conferred by the extension, so `is_file()` alone
/// is the right check (the Windows PATHEXT loop handles extensions separately).
#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_file()
        && path
            .metadata()
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

/// Resolve `command` to an absolute executable path within `dir` (applying the
/// matched Windows extension), so callers can spawn it directly. None if absent.
fn executable_in_dir(dir: &Path, command: &str) -> Option<std::path::PathBuf> {
    let direct = dir.join(command);
    if is_executable_file(&direct) {
        return Some(direct);
    }

    #[cfg(windows)]
    {
        let extensions = env::var_os("PATHEXT").map_or_else(
            || {
                vec![
                    ".COM".to_string(),
                    ".EXE".to_string(),
                    ".BAT".to_string(),
                    ".CMD".to_string(),
                ]
            },
            |value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            },
        );

        for extension in extensions {
            let candidate = dir.join(format!("{command}{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_server_diagnostics_reports_missing_servers() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let servers = vec![LanguageServerInfo {
            language_id: "typescript".to_string(),
            name: "TypeScript Language Server".to_string(),
            command: "typescript-language-server".to_string(),
            args: vec!["--stdio".to_string()],
            workspace_root: root.clone(),
            status: LanguageServerStatus::Missing,
            error: Some("typescript-language-server was not found in PATH".to_string()),
        }];

        let diagnostics = language_server_diagnostics(&servers);

        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(diagnostics[0].source, "lux-lsp");
        assert_eq!(diagnostics[0].line, 1);
        assert!(diagnostics[0]
            .message
            .contains("typescript-language-server"));
        assert_eq!(diagnostics[0].path, root);
    }

    #[test]
    fn language_server_diagnostics_ignores_available_servers() {
        let servers = vec![LanguageServerInfo {
            language_id: "rust".to_string(),
            name: "rust-analyzer".to_string(),
            command: "rust-analyzer".to_string(),
            args: Vec::new(),
            workspace_root: std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            status: LanguageServerStatus::Available,
            error: None,
        }];

        assert!(language_server_diagnostics(&servers).is_empty());
    }

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let unique = format!(
            "lux-lsp-disco-{tag}-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let dir = env::temp_dir().join(unique);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn detect_extensions_finds_relevant_and_skips_ignored_dirs() {
        let root = temp_dir("relevant");
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/app.ts"), "export {}").unwrap();
        // A relevant file buried in an ignored dir must NOT be detected — that's
        // what keeps discovery from walking huge node_modules / target trees.
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "module.exports={}").unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("target/debug/build.rs"), "fn main() {}").unwrap();

        let found = detect_extensions(&root);
        assert!(found.contains("rs"), "should detect rust source");
        assert!(found.contains("ts"), "should detect typescript source");
        // The .js only exists under node_modules, which is skipped.
        assert!(
            !found.contains("js"),
            "ignored-dir files must not be detected"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn detect_extensions_skips_hidden_directories() {
        let root = temp_dir("hidden");
        std::fs::create_dir_all(root.join(".hidden")).unwrap();
        std::fs::write(root.join(".hidden/buried.rs"), "fn main() {}").unwrap();

        let found = detect_extensions(&root);
        assert!(
            !found.contains("rs"),
            "hidden-dir files must not be detected"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn detect_extensions_seeds_languages_from_root_manifests() {
        // A root manifest implies the language even when NO matching source file
        // is present in the (small) tree — covering the large-repo case where
        // source would sort after the entry cap.
        let root = temp_dir("manifest");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"x\"").unwrap();
        std::fs::write(root.join("go.mod"), "module x").unwrap();
        std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"x\"").unwrap();

        let found = detect_extensions(&root);
        assert!(found.contains("rs"), "Cargo.toml should imply rust");
        assert!(found.contains("go"), "go.mod should imply go");
        assert!(found.contains("py"), "pyproject.toml should imply python");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn detect_extensions_bounded_reports_no_truncation_for_small_tree() {
        let root = temp_dir("untruncated");
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();

        let (found, truncated) = detect_extensions_bounded(&root);
        assert!(found.contains("rs"));
        assert!(!truncated, "a tiny tree must not report truncation");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn detect_extensions_is_empty_for_unrelated_files() {
        let root = temp_dir("unrelated");
        std::fs::write(root.join("notes.md"), "# notes").unwrap();
        std::fs::write(root.join("data.bin"), [0_u8; 4]).unwrap();

        // No builtin server handles .md/.bin, so nothing is detected.
        assert!(detect_extensions(&root).is_empty());

        std::fs::remove_dir_all(&root).ok();
    }
}
