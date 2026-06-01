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
    let detected_extensions = detect_extensions(&root)?;
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

fn detect_extensions(root: &Path) -> AppResult<BTreeSet<String>> {
    let mut extensions = BTreeSet::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(path) = stack.pop() {
        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };

        for child in children {
            let child = child?;
            let file_name = child.file_name();
            let file_name = file_name.to_string_lossy();
            if file_name == "node_modules" || file_name == "target" || file_name == ".git" {
                continue;
            }

            let file_type = child.file_type()?;
            if file_type.is_dir() {
                stack.push(child.path());
            } else if file_type.is_file() {
                if let Some(extension) = child.path().extension().and_then(|value| value.to_str()) {
                    extensions.insert(extension.to_ascii_lowercase());
                }
            }
        }
    }

    Ok(extensions)
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
}
