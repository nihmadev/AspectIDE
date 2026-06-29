//! Workspace adapter discovery: detects which built-in debug adapters apply to a
//! project, reads `.vscode/launch.json` (JSONC tolerant), and exposes adapter
//! metadata. Pure filesystem inspection — no session or protocol state.
#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeSet;
use std::env;
use std::path::{Path, PathBuf};

use lux_core::{
    AppResult, DebugAdapterInfo, DebugAdapterStatus, DebugAdapterTransport, DebugConfiguration,
    DebugConfigurationRequest, DebugWorkspaceInfo,
};
use serde_json::Value;

// ── Bounded walk limits ──────────────────────────────────────────────────
const WALK_MAX_DEPTH: usize = 12;
const WALK_MAX_FILES: usize = 500_000;

// ── Ignored directory names for extension detection ──────────────────────
const IGNORE_DIRS: &[&str] = &[
    "node_modules", "target", ".git", ".venv", "venv", "dist", "build",
    "vendor", "__pycache__", ".next", ".nuxt", "out", ".cache",
    ".bundle", "coverage", ".terraform", ".serverless",
];

struct BuiltinDebugAdapter {
    id: &'static str,
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    configuration_types: &'static [&'static str],
    transport: DebugAdapterTransport,
    marker_files: &'static [&'static str],
    marker_extensions: &'static [&'static str],
}

const BUILTIN_ADAPTERS: &[BuiltinDebugAdapter] = &[
    BuiltinDebugAdapter {
        id: "codelldb",
        name: "CodeLLDB",
        command: "codelldb",
        args: &["--port", "0"],
        configuration_types: &["codelldb", "lldb"],
        transport: DebugAdapterTransport::TcpServer,
        marker_files: &["Cargo.toml"],
        marker_extensions: &["rs"],
    },
    BuiltinDebugAdapter {
        id: "js-debug",
        name: "JavaScript Debug Terminal",
        command: "js-debug-adapter",
        args: &[],
        configuration_types: &[
            "js-debug",
            "node",
            "pwa-node",
            "node-terminal",
            "extensionHost",
        ],
        transport: DebugAdapterTransport::Stdio,
        marker_files: &["package.json", "tsconfig.json", "jsconfig.json"],
        marker_extensions: &["js", "jsx", "ts", "tsx"],
    },
    BuiltinDebugAdapter {
        id: "debugpy",
        name: "Python Debugpy",
        command: "python",
        args: &["-m", "debugpy.adapter"],
        configuration_types: &["debugpy", "python"],
        transport: DebugAdapterTransport::Stdio,
        marker_files: &["pyproject.toml", "requirements.txt", "setup.py"],
        marker_extensions: &["py"],
    },
];

pub fn workspace_debug_info(root: impl AsRef<Path>) -> AppResult<DebugWorkspaceInfo> {
    let root = root.as_ref().canonicalize()?;
    let adapters = workspace_debug_adapters(&root)?;
    let (launch_json_path, configurations) = read_launch_configurations(&root)?;
    Ok(DebugWorkspaceInfo {
        adapters,
        configurations,
        launch_json_path,
    })
}

pub fn workspace_debug_adapters(root: impl AsRef<Path>) -> AppResult<Vec<DebugAdapterInfo>> {
    let root = root.as_ref().canonicalize()?;
    let detected_files = detect_files(&root)?;
    let detected_extensions = detect_extensions(&root)?;
    let mut adapters = Vec::new();

    for adapter in BUILTIN_ADAPTERS {
        let matches_file = adapter
            .marker_files
            .iter()
            .any(|file| detected_files.contains(*file));
        let matches_extension = adapter
            .marker_extensions
            .iter()
            .any(|extension| detected_extensions.contains(*extension));
        if !matches_file && !matches_extension {
            continue;
        }

        let available = command_available(adapter.command);
        adapters.push(DebugAdapterInfo {
            id: adapter.id.to_string(),
            name: adapter.name.to_string(),
            command: adapter.command.to_string(),
            args: adapter.args.iter().map(|arg| (*arg).to_string()).collect(),
            configuration_types: adapter
                .configuration_types
                .iter()
                .map(|value| (*value).to_string())
                .collect(),
            transport: adapter.transport,
            workspace_root: root.clone(),
            status: if available {
                DebugAdapterStatus::Available
            } else {
                DebugAdapterStatus::Missing
            },
            error: if available {
                None
            } else {
                Some(format!("{} was not found in PATH", adapter.command))
            },
        });
    }

    Ok(adapters)
}

#[must_use]
pub fn adapter_matches_configuration(
    adapter: &DebugAdapterInfo,
    configuration: &DebugConfiguration,
) -> bool {
    adapter
        .configuration_types
        .iter()
        .any(|adapter_type| adapter_type.eq_ignore_ascii_case(&configuration.adapter_type))
        || adapter.id.eq_ignore_ascii_case(&configuration.adapter_type)
        || adapter
            .command
            .eq_ignore_ascii_case(&configuration.adapter_type)
}

pub fn workspace_debug_adapter_for_configuration(
    root: impl AsRef<Path>,
    configuration: &DebugConfiguration,
) -> AppResult<Option<DebugAdapterInfo>> {
    Ok(workspace_debug_adapters(root)?
        .into_iter()
        .find(|adapter| adapter_matches_configuration(adapter, configuration)))
}

fn read_launch_configurations(
    root: &Path,
) -> AppResult<(Option<PathBuf>, Vec<DebugConfiguration>)> {
    let path = root.join(".vscode").join("launch.json");
    if !path.is_file() {
        return Ok((None, Vec::new()));
    }

    let contents = std::fs::read_to_string(&path)?;
    let value: Value = serde_json::from_str(&jsonc_to_json(&contents))?;
    let configurations = value
        .get("configurations")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(parse_debug_configuration).collect())
        .unwrap_or_default();
    Ok((Some(path), configurations))
}

fn parse_debug_configuration(value: &Value) -> Option<DebugConfiguration> {
    let object = value.as_object()?;
    let name = object.get("name")?.as_str()?.trim();
    let adapter_type = object.get("type")?.as_str()?.trim();
    let request = parse_debug_request(object.get("request")?.as_str()?)?;
    if name.is_empty() || adapter_type.is_empty() {
        return None;
    }

    Some(DebugConfiguration {
        name: name.to_string(),
        adapter_type: adapter_type.to_string(),
        request,
        raw: object.clone(),
    })
}

fn parse_debug_request(value: &str) -> Option<DebugConfigurationRequest> {
    match value {
        "launch" => Some(DebugConfigurationRequest::Launch),
        "attach" => Some(DebugConfigurationRequest::Attach),
        _ => None,
    }
}

fn jsonc_to_json(value: &str) -> String {
    remove_trailing_commas(&strip_jsonc_comments(value))
}

fn strip_jsonc_comments(value: &str) -> String {
    let mut result = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            result.push(ch);
            continue;
        }

        if ch == '/' {
            match chars.peek().copied() {
                Some('/') => {
                    chars.next();
                    for next in chars.by_ref() {
                        if next == '\n' {
                            result.push('\n');
                            break;
                        }
                    }
                    continue;
                }
                Some('*') => {
                    chars.next();
                    let mut previous = '\0';
                    for next in chars.by_ref() {
                        if next == '\n' {
                            result.push('\n');
                        }
                        if previous == '*' && next == '/' {
                            break;
                        }
                        previous = next;
                    }
                    continue;
                }
                _ => {}
            }
        }

        result.push(ch);
    }

    result
}

fn remove_trailing_commas(value: &str) -> String {
    let chars = value.chars().collect::<Vec<_>>();
    let mut result = String::with_capacity(value.len());
    let mut index = 0;
    let mut in_string = false;
    let mut escaped = false;

    while index < chars.len() {
        let ch = chars[index];
        if in_string {
            result.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }

        if ch == '"' {
            in_string = true;
            result.push(ch);
            index += 1;
            continue;
        }

        if ch == ',' {
            let mut next_index = index + 1;
            while next_index < chars.len() && chars[next_index].is_whitespace() {
                next_index += 1;
            }
            if matches!(chars.get(next_index), Some(']' | '}')) {
                index += 1;
                continue;
            }
        }

        result.push(ch);
        index += 1;
    }

    result
}

fn detect_files(root: &Path) -> AppResult<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.insert(entry.file_name().to_string_lossy().to_string());
        }
    }
    Ok(files)
}

fn detect_extensions(root: &Path) -> AppResult<BTreeSet<String>> {
    let mut extensions = BTreeSet::new();
    let mut stack = vec![(root.to_path_buf(), 0_usize)];
    let mut visited = BTreeSet::new();
    let mut total_files = 0_usize;

    while let Some((path, depth)) = stack.pop() {
        if depth >= WALK_MAX_DEPTH || total_files >= WALK_MAX_FILES {
            continue;
        }
        // Symlink protection: canonicalize and deduplicate.
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        if !visited.insert(canonical) {
            continue;
        }

        let Ok(children) = std::fs::read_dir(&path) else {
            continue;
        };

        for child in children {
            if total_files >= WALK_MAX_FILES {
                break;
            }
            let child = child?;
            let file_name = child.file_name();
            let file_name = file_name.to_string_lossy();
            // Skip ignored directories.
            if IGNORE_DIRS.contains(&file_name.as_ref()) {
                continue;
            }

            let file_type = child.file_type()?;
            if file_type.is_symlink() {
                // Skip symlinks to avoid cycles.
                continue;
            }
            if file_type.is_dir() {
                stack.push((child.path(), depth + 1));
            } else if file_type.is_file() {
                total_files += 1;
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
    use crate::session::tcp_server_port_arg_indices;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn launch_json_parser_accepts_jsonc_comments_and_trailing_commas() {
        let root = unique_temp_dir("lux-dap-jsonc");
        std::fs::create_dir_all(root.join(".vscode")).expect("vscode dir should be created");
        std::fs::write(
            root.join(".vscode").join("launch.json"),
            r#"{
                // Cursor and VS Code launch files allow comments.
                "version": "0.2.0",
                "configurations": [
                    {
                        "name": "Run API",
                        "type": "debugpy",
                        "request": "launch",
                        "program": "${workspaceFolder}/app.py",
                    },
                ],
            }"#,
        )
        .expect("launch.json should be written");

        let info = workspace_debug_info(&root).expect("debug info should load");

        assert_eq!(info.configurations.len(), 1);
        assert_eq!(info.configurations[0].name, "Run API");
        assert_eq!(info.configurations[0].adapter_type, "debugpy");

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn workspace_debug_info_reads_launch_json_and_detects_adapters() {
        let root = unique_temp_dir("lux-dap-workspace");
        std::fs::create_dir_all(root.join(".vscode")).expect("vscode dir should be created");
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"debug-test\"\n",
        )
        .expect("cargo manifest should be written");
        std::fs::write(
            root.join(".vscode").join("launch.json"),
            r#"{
                "version": "0.2.0",
                "configurations": [
                    {"name": "Run app", "type": "codelldb", "request": "launch", "program": "target/debug/app"},
                    {"name": "Attach app", "type": "codelldb", "request": "attach", "pid": 1234},
                    {"name": "Ignored", "type": "codelldb", "request": "unsupported"}
                ]
            }"#,
        )
        .expect("launch.json should be written");

        let info = workspace_debug_info(&root).expect("debug info should load");

        assert_eq!(info.configurations.len(), 2);
        assert_eq!(info.configurations[0].name, "Run app");
        assert_eq!(
            info.configurations[0].request,
            DebugConfigurationRequest::Launch
        );
        assert_eq!(
            info.configurations[1].request,
            DebugConfigurationRequest::Attach
        );
        assert_eq!(
            info.launch_json_path
                .as_ref()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str()),
            Some("launch.json")
        );
        assert!(info.adapters.iter().any(|adapter| adapter.id == "codelldb"));

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn adapters_expose_transport_and_configuration_aliases() {
        let root = unique_temp_dir("lux-dap-adapter-aliases");
        std::fs::create_dir_all(&root).expect("test root should be created");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"rust-app\"\n")
            .expect("cargo manifest should be written");
        std::fs::write(root.join("pyproject.toml"), "[project]\nname = \"api\"\n")
            .expect("pyproject should be written");

        let adapters = workspace_debug_adapters(&root).expect("adapters should load");
        let debugpy = adapters
            .iter()
            .find(|adapter| adapter.id == "debugpy")
            .expect("debugpy should be detected");

        assert_eq!(debugpy.transport, DebugAdapterTransport::Stdio);
        assert!(debugpy
            .configuration_types
            .iter()
            .any(|adapter_type| adapter_type == "python"));
        let codelldb = adapters
            .iter()
            .find(|adapter| adapter.id == "codelldb")
            .expect("codelldb should be detected");
        assert_eq!(codelldb.transport, DebugAdapterTransport::TcpServer);
        assert_eq!(tcp_server_port_arg_indices(&codelldb.args), Some((0, 1)));

        let configuration = DebugConfiguration {
            name: "Run API".to_string(),
            adapter_type: "python".to_string(),
            request: DebugConfigurationRequest::Launch,
            raw: serde_json::Map::new(),
        };

        assert!(adapter_matches_configuration(debugpy, &configuration));

        let _ = std::fs::remove_dir_all(root);
    }

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nonce}"))
    }
}
