use std::{collections::BTreeSet, env, path::Path};

use aspect_core::{
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
/// server started вЂ” the walk-truncation blind spot. Each extension here must
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
            "aspect-lsp: workspace language discovery hit its entry cap in {} вЂ” \
             languages without a root manifest may be undetected; \
             configure servers in Settings в†’ Language Servers if one is missing.",
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
                    "{} is not installed вЂ” install it from Settings в†’ Language Servers",
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
            source: "aspect-lsp".to_string(),
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
/// all of them we can stop walking вЂ” no point scanning a huge repo further.
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

/// Detect extensions, returning `(found, truncated)` where `truncated` is true if
/// the entry cap was hit before the tree was fully walked. Manifest-backed
/// languages are seeded first so they survive truncation; the flag lets callers
/// warn that extension-only languages past the cap may have been missed.
fn detect_extensions_bounded(root: &Path) -> (BTreeSet<String>, bool) {
    // Cap on directory entries visited вЂ” generous for real projects, but a hard
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
                        // Every server we could start is accounted for вЂ” stop early.
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

