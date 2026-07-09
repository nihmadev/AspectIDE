use std::path::Path;

use crate::json_helpers::json_str;

pub fn normalize_screenshot_path(raw: &str, root: &Path) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let candidate = Path::new(raw);
    if candidate.is_absolute() {
        let Ok(stripped) = candidate.strip_prefix(root) else {
            return raw.to_string();
        };
        stripped.to_string_lossy().replace('\\', "/")
    } else {
        raw.to_string()
    }
}

pub fn build_browser_args(tool_name: &str, args: &serde_json::Value) -> Vec<String> {
    let tool = if let Some(stripped) = tool_name.strip_prefix("Browser") {
        stripped
    } else {
        tool_name
    };

    match tool {
        "Status" => vec!["status".to_string()],
        "Open" => {
            let url = json_str(args, "url");
            let mut cmd = vec!["open".to_string(), url];
            if let Some(width) = args.get("width").and_then(|v| v.as_u64()) {
                cmd.push("--width".to_string());
                cmd.push(width.to_string());
            }
            if let Some(height) = args.get("height").and_then(|v| v.as_u64()) {
                cmd.push("--height".to_string());
                cmd.push(height.to_string());
            }
            cmd
        }
        "Act" => {
            let action = json_str(args, "action");
            let value = json_str(args, "value");
            vec!["act".to_string(), action, value]
        }
        "Snapshot" => vec!["snapshot".to_string()],
        "Screenshot" => {
            let mut cmd = vec!["screenshot".to_string()];
            if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                cmd.push("--path".to_string());
                cmd.push(path.to_string());
            }
            cmd
        }
        "Close" => vec!["close".to_string()],
        "Chat" => {
            let message = json_str(args, "message");
            vec!["chat".to_string(), message]
        }
        "Dashboard" => vec!["dashboard".to_string()],
        "Install" => {
            let version = json_str(args, "version");
            let mut cmd = vec!["install".to_string()];
            if !version.is_empty() {
                cmd.push("--version".to_string());
                cmd.push(version);
            }
            cmd
        }
        "Help" => vec!["help".to_string()],
        "Doctor" => vec!["doctor".to_string()],
        "Invoke" => {
            let action = json_str(args, "action");
            let mut cmd = vec!["invoke".to_string(), action];
            if let Some(payload) = args.get("payload") {
                cmd.push("--json".to_string());
                cmd.push(payload.to_string());
            }
            cmd
        }
        _ => vec![tool.to_lowercase()],
    }
}
