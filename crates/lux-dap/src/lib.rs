use std::{
    collections::BTreeSet,
    env,
    path::{Path, PathBuf},
};

use lux_core::{
    AppError, AppResult, DebugAdapterInfo, DebugAdapterStatus, DebugConfiguration,
    DebugConfigurationRequest, DebugWorkspaceInfo,
};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapFrame {
    pub content: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapResponse {
    pub request_seq: u64,
    pub success: bool,
    pub command: String,
    pub message: Option<String>,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DapEvent {
    pub event: String,
    pub body: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DapMessage {
    Response(DapResponse),
    Event(DapEvent),
    Request { command: String },
}

struct BuiltinDebugAdapter {
    id: &'static str,
    name: &'static str,
    command: &'static str,
    args: &'static [&'static str],
    marker_files: &'static [&'static str],
    marker_extensions: &'static [&'static str],
}

const BUILTIN_ADAPTERS: &[BuiltinDebugAdapter] = &[
    BuiltinDebugAdapter {
        id: "codelldb",
        name: "CodeLLDB",
        command: "codelldb",
        args: &["--port", "0"],
        marker_files: &["Cargo.toml"],
        marker_extensions: &["rs"],
    },
    BuiltinDebugAdapter {
        id: "js-debug",
        name: "JavaScript Debug Terminal",
        command: "js-debug-adapter",
        args: &[],
        marker_files: &["package.json", "tsconfig.json", "jsconfig.json"],
        marker_extensions: &["js", "jsx", "ts", "tsx"],
    },
    BuiltinDebugAdapter {
        id: "debugpy",
        name: "Python Debugpy",
        command: "python",
        args: &["-m", "debugpy.adapter"],
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

pub fn encode_dap_message(value: &Value) -> AppResult<Vec<u8>> {
    let content = serde_json::to_vec(value)?;
    let mut message = format!("Content-Length: {}\r\n\r\n", content.len()).into_bytes();
    message.extend_from_slice(&content);
    Ok(message)
}

pub fn drain_dap_frames(buffer: &mut Vec<u8>) -> AppResult<Vec<DapFrame>> {
    let mut frames = Vec::new();

    while let Some(header_end) = find_header_end(buffer) {
        let headers = std::str::from_utf8(&buffer[..header_end])
            .map_err(|error| AppError::Service(format!("invalid DAP header encoding: {error}")))?;
        let content_length = parse_content_length(headers)?;
        let frame_start = header_end + 4;
        let frame_end = frame_start + content_length;

        if buffer.len() < frame_end {
            break;
        }

        let content = buffer[frame_start..frame_end].to_vec();
        buffer.drain(..frame_end);
        frames.push(DapFrame { content });
    }

    Ok(frames)
}

pub fn parse_dap_message(frame: &DapFrame) -> AppResult<Option<DapMessage>> {
    let value: Value = serde_json::from_slice(&frame.content)?;
    Ok(parse_dap_message_value(&value))
}

pub fn initialize_request(seq: u64, adapter_id: &str) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "initialize",
        "arguments": {
            "clientID": "lux-ide",
            "clientName": "Lux IDE",
            "adapterID": adapter_id,
            "pathFormat": "path",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "supportsVariableType": true,
            "supportsVariablePaging": true,
            "supportsRunInTerminalRequest": true,
            "supportsProgressReporting": true,
            "supportsInvalidatedEvent": true,
        }
    })
}

pub fn launch_request(seq: u64, configuration: &DebugConfiguration) -> Value {
    debug_configuration_request(seq, "launch", configuration)
}

pub fn attach_request(seq: u64, configuration: &DebugConfiguration) -> Value {
    debug_configuration_request(seq, "attach", configuration)
}

pub fn configuration_done_request(seq: u64) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "configurationDone",
        "arguments": {}
    })
}

pub fn disconnect_request(seq: u64, terminate_debuggee: bool) -> Value {
    json!({
        "seq": seq,
        "type": "request",
        "command": "disconnect",
        "arguments": {
            "terminateDebuggee": terminate_debuggee,
        }
    })
}

fn debug_configuration_request(
    seq: u64,
    command: &str,
    configuration: &DebugConfiguration,
) -> Value {
    let mut arguments = serde_json::Map::new();
    for (key, value) in &configuration.raw {
        arguments.insert(key.clone(), value.clone());
    }
    arguments.insert(
        "name".to_string(),
        Value::String(configuration.name.clone()),
    );
    arguments.insert(
        "type".to_string(),
        Value::String(configuration.adapter_type.clone()),
    );
    arguments.insert("request".to_string(), Value::String(command.to_string()));

    json!({
        "seq": seq,
        "type": "request",
        "command": command,
        "arguments": arguments,
    })
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

fn parse_dap_message_value(value: &Value) -> Option<DapMessage> {
    match value.get("type")?.as_str()? {
        "response" => Some(DapMessage::Response(DapResponse {
            request_seq: value.get("request_seq")?.as_u64()?,
            success: value
                .get("success")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            command: value.get("command")?.as_str()?.to_string(),
            message: value
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string),
            body: value.get("body").cloned(),
        })),
        "event" => Some(DapMessage::Event(DapEvent {
            event: value.get("event")?.as_str()?.to_string(),
            body: value.get("body").cloned(),
        })),
        "request" => Some(DapMessage::Request {
            command: value.get("command")?.as_str()?.to_string(),
        }),
        _ => None,
    }
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
        let extensions = env::var_os("PATHEXT")
            .map(|value| {
                value
                    .to_string_lossy()
                    .split(';')
                    .filter(|extension| !extension.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_else(|| {
                vec![
                    ".COM".to_string(),
                    ".EXE".to_string(),
                    ".BAT".to_string(),
                    ".CMD".to_string(),
                ]
            });

        for extension in extensions {
            if dir.join(format!("{command}{extension}")).is_file() {
                return true;
            }
        }
    }

    false
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn parse_content_length(headers: &str) -> AppResult<usize> {
    for line in headers.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            return value.trim().parse::<usize>().map_err(|error| {
                AppError::Service(format!("invalid DAP Content-Length: {error}"))
            });
        }
    }

    Err(AppError::Service(
        "DAP frame is missing Content-Length header".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn drain_dap_frames_extracts_complete_frames_and_keeps_partial_tail() {
        let first = br#"{"seq":1,"type":"event","event":"initialized"}"#;
        let second =
            br#"{"seq":2,"type":"response","request_seq":1,"success":true,"command":"initialize"}"#;
        let mut buffer = Vec::new();
        buffer.extend_from_slice(format!("Content-Length: {}\r\n\r\n", first.len()).as_bytes());
        buffer.extend_from_slice(first);
        buffer.extend_from_slice(format!("Content-Length: {}\r\n\r\n", second.len()).as_bytes());
        buffer.extend_from_slice(&second[..8]);

        let frames = drain_dap_frames(&mut buffer).expect("valid frame should parse");

        assert_eq!(
            frames,
            vec![DapFrame {
                content: first.to_vec()
            }]
        );
        assert_eq!(
            buffer,
            [
                format!("Content-Length: {}\r\n\r\n", second.len()).as_bytes(),
                &second[..8]
            ]
            .concat()
        );
    }

    #[test]
    fn parse_dap_message_accepts_events_and_responses() {
        let event = DapFrame {
            content:
                br#"{"seq":1,"type":"event","event":"stopped","body":{"reason":"breakpoint"}}"#
                    .to_vec(),
        };
        let response = DapFrame {
            content: br#"{"seq":2,"type":"response","request_seq":1,"success":false,"command":"launch","message":"failed"}"#.to_vec(),
        };

        assert_eq!(
            parse_dap_message(&event).expect("event should parse"),
            Some(DapMessage::Event(DapEvent {
                event: "stopped".to_string(),
                body: Some(json!({"reason":"breakpoint"})),
            }))
        );
        assert_eq!(
            parse_dap_message(&response).expect("response should parse"),
            Some(DapMessage::Response(DapResponse {
                request_seq: 1,
                success: false,
                command: "launch".to_string(),
                message: Some("failed".to_string()),
                body: None,
            }))
        );
    }

    #[test]
    fn request_builders_emit_dap_initialize_launch_and_disconnect() {
        let configuration = DebugConfiguration {
            name: "Run binary".to_string(),
            adapter_type: "codelldb".to_string(),
            request: DebugConfigurationRequest::Launch,
            raw: serde_json::Map::from_iter([
                ("program".to_string(), json!("target/debug/app")),
                ("cwd".to_string(), json!("${workspaceFolder}")),
            ]),
        };

        let initialize = initialize_request(1, "codelldb");
        let launch = launch_request(2, &configuration);
        let disconnect = disconnect_request(3, true);

        assert_eq!(initialize["command"], "initialize");
        assert_eq!(initialize["arguments"]["adapterID"], "codelldb");
        assert_eq!(launch["command"], "launch");
        assert_eq!(launch["arguments"]["name"], "Run binary");
        assert_eq!(launch["arguments"]["type"], "codelldb");
        assert_eq!(launch["arguments"]["request"], "launch");
        assert_eq!(launch["arguments"]["program"], "target/debug/app");
        assert_eq!(disconnect["command"], "disconnect");
        assert_eq!(disconnect["arguments"]["terminateDebuggee"], true);
    }

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

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{nonce}"))
    }
}
