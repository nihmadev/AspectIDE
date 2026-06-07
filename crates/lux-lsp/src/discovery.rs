use std::{collections::BTreeSet, env, path::Path};

use lux_core::{
    AppResult, DiagnosticSeverity, LanguageServerInfo, LanguageServerStatus, WorkspaceDiagnostic,
};

#[derive(Debug, Clone, Copy)]
struct BuiltinServer {
    language_id: &'static str,
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    extensions: &'static [&'static str],
}

const BUILTIN_SERVERS: &[BuiltinServer] = &[
    BuiltinServer {
        language_id: "rust",
        name: "rust-analyzer",
        command: "rust-analyzer",
        args: &[],
        extensions: &["rs"],
    },
    BuiltinServer {
        language_id: "typescript",
        name: "TypeScript Language Server",
        command: "typescript-language-server",
        args: &["--stdio"],
        extensions: &["ts", "tsx", "js", "jsx"],
    },
    BuiltinServer {
        language_id: "json",
        name: "JSON Language Server",
        command: "vscode-json-language-server",
        args: &["--stdio"],
        extensions: &["json"],
    },
];

pub fn workspace_language_servers(root: impl AsRef<Path>) -> AppResult<Vec<LanguageServerInfo>> {
    let root = root.as_ref().canonicalize()?;
    let detected_extensions = detect_extensions(&root);
    let mut servers = Vec::new();

    for server in BUILTIN_SERVERS {
        if !server
            .extensions
            .iter()
            .any(|extension| detected_extensions.contains(*extension))
        {
            continue;
        }

        let available = command_available(server.command);
        servers.push(LanguageServerInfo {
            language_id: server.language_id.to_string(),
            name: server.name.to_string(),
            command: server.command.to_string(),
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
                Some(format!("{} was not found in PATH", server.command))
            },
        });
    }

    Ok(servers)
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
fn detect_extensions(root: &Path) -> BTreeSet<String> {
    // Cap on directory entries visited — generous for real projects, but a hard
    // ceiling so discovery returns promptly no matter the repo size.
    const MAX_ENTRIES: usize = 20_000;

    let wanted = relevant_extensions();
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];
    let mut visited = 0_usize;

    while let Some(path) = stack.pop() {
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };

        for child in children.flatten() {
            visited += 1;
            if visited > MAX_ENTRIES {
                return found;
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
                            return found;
                        }
                    }
                }
            }
        }
    }

    found
}

fn command_available(command: &str) -> bool {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 {
        return command_path.is_file();
    }

    let Some(paths) = env::var_os("PATH") else {
        return false;
    };

    env::split_paths(&paths).any(|path| command_exists_in_dir(&path, command))
}

fn command_exists_in_dir(dir: &Path, command: &str) -> bool {
    let direct = dir.join(command);
    if direct.is_file() {
        return true;
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
            if dir.join(format!("{command}{extension}")).is_file() {
                return true;
            }
        }
    }

    false
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
    fn detect_extensions_is_empty_for_unrelated_files() {
        let root = temp_dir("unrelated");
        std::fs::write(root.join("notes.md"), "# notes").unwrap();
        std::fs::write(root.join("data.bin"), [0_u8; 4]).unwrap();

        // No builtin server handles .md/.bin, so nothing is detected.
        assert!(detect_extensions(&root).is_empty());

        std::fs::remove_dir_all(&root).ok();
    }
}
