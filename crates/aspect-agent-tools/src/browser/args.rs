use crate::json_utils::json_str_opt;

pub fn build_browser_args(tool_name: &str, args: &serde_json::Value) -> Vec<String> {
    match tool_name {
        "BrowserOpen" => {
            let mut a = vec!["open".to_string()];
            if let Some(url) = args.get("url").and_then(|v| v.as_str()) {
                a.push(url.to_string());
            }
            if args.get("headed").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                a.push("--headed".to_string());
            }
            a
        }
        "BrowserAct" => {
            if let Some(cmds) = args.get("commands").and_then(|v| v.as_array()) {
                let mut a = vec!["batch".to_string()];
                for cmd in cmds {
                    if let Some(s) = cmd.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                        a.push(s.to_string());
                    }
                }
                a
            } else if let Some(cmds) = args.get("batchCommands").and_then(|v| v.as_array()) {
                let tokens: Vec<String> = cmds.iter()
                    .filter_map(|c| c.as_str())
                    .map(str::to_string)
                    .filter(|s| !s.trim().is_empty())
                    .collect();
                let all_multiword = tokens.len() > 1
                    && tokens.iter().all(|t| t.trim().contains(char::is_whitespace));
                if all_multiword {
                    let mut a = vec!["batch".to_string()];
                    a.extend(tokens);
                    a
                } else {
                    tokens
                }
            } else {
                let cmd = args.get("command").and_then(|v| v.as_str()).unwrap_or("");
                cmd.split_whitespace().map(str::to_string).collect()
            }
        }
        "BrowserSnapshot" => {
            let mut a = vec!["snapshot".to_string()];
            if args.get("interactive").and_then(serde_json::Value::as_bool).unwrap_or(true) {
                a.push("-i".to_string());
            }
            if args.get("compact").and_then(serde_json::Value::as_bool).unwrap_or(true) {
                a.push("--compact".to_string());
            }
            if let Some(d) = args.get("depth").and_then(serde_json::Value::as_u64) {
                a.push("--depth".to_string());
                a.push(d.to_string());
            }
            if let Some(sel) = json_str_opt(args, "selector") {
                a.push("-s".to_string());
                a.push(sel);
            }
            if args.get("includeUrls").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                a.push("--urls".to_string());
            }
            a
        }
        "BrowserScreenshot" => {
            let mut a = vec!["screenshot".to_string()];
            if args.get("annotate").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                a.push("--annotate".to_string());
            }
            if args.get("fullPage").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                a.push("--full-page".to_string());
            }
            if let Some(p) = args.get("path").and_then(|v| v.as_str()) {
                a.push(p.to_string());
            }
            a
        }
        "BrowserClose" => {
            let mut a = vec!["close".to_string()];
            if args.get("all").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                a.push("--all".to_string());
            }
            a
        }
        "BrowserChat" => {
            let instruction = args.get("instruction").and_then(|v| v.as_str()).unwrap_or("");
            let quiet = args.get("quiet").and_then(serde_json::Value::as_bool).unwrap_or(true);
            if quiet {
                vec!["-q".to_string(), "chat".to_string(), instruction.to_string()]
            } else {
                vec!["chat".to_string(), instruction.to_string()]
            }
        }
        "BrowserDashboard" => {
            let action = args.get("action").and_then(|v| v.as_str()).unwrap_or("start");
            let sub = if action.eq_ignore_ascii_case("stop") { "stop" } else { "start" };
            let mut a = vec!["dashboard".to_string(), sub.to_string()];
            if let Some(port) = args.get("port").and_then(serde_json::Value::as_u64) {
                a.push("--port".to_string());
                a.push(port.to_string());
            }
            a
        }
        "BrowserInstall" => {
            let mut a = vec!["install".to_string()];
            if args.get("withDeps").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                a.push("--with-deps".to_string());
            }
            a
        }
        "BrowserHelp" => {
            const BROWSER_SKILLS: [&str; 6] = [
                "agentcore", "core", "dogfood", "electron", "slack", "vercel-sandbox",
            ];
            let all_skills = args.get("allSkills").and_then(serde_json::Value::as_bool).unwrap_or(false);
            let requested = json_str_opt(args, "skill")
                .or_else(|| json_str_opt(args, "topic"))
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty());
            if all_skills {
                vec!["skills".to_string(), "get".to_string(), "--all".to_string()]
            } else if let Some(name) = requested {
                let resolved = if BROWSER_SKILLS.contains(&name.as_str()) { name } else { "core".to_string() };
                let mut a = vec!["skills".to_string(), "get".to_string(), resolved];
                if args.get("full").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                    a.push("--full".to_string());
                }
                a
            } else {
                vec!["skills".to_string()]
            }
        }
        "BrowserDoctor" => {
            let mut a = vec!["doctor".to_string()];
            if args.get("offline").and_then(serde_json::Value::as_bool).unwrap_or(true) {
                a.push("--offline".to_string());
            }
            if args.get("quick").and_then(serde_json::Value::as_bool).unwrap_or(true) {
                a.push("--quick".to_string());
            }
            if args.get("fix").and_then(serde_json::Value::as_bool).unwrap_or(false) {
                a.push("--fix".to_string());
            }
            a
        }
        "BrowserInvoke" => args
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default(),
        _ => vec![],
    }
}
