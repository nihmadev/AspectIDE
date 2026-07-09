use crate::probes::session_invoke_options;
use crate::process::run_json;
use crate::resolver::{resolve_binary, DEFAULT_MAX_OUTPUT};
use crate::types::{
    AgentBrowserDashboardRequest, AgentBrowserDashboardResponse, AgentBrowserSkillsRequest,
    AgentBrowserSkillsResponse, AgentBrowserStreamStatusRequest, AgentBrowserStreamStatusResponse,
};
use crate::validate::sanitize_session;

pub async fn stream_status(
    request: AgentBrowserStreamStatusRequest,
) -> Result<AgentBrowserStreamStatusResponse, String> {
    let binary = resolve_binary()?;
    let session = sanitize_session(&request.session);
    let enable_stream = request.enable == Some(true);
    let invoke_options = session_invoke_options(session.clone(), DEFAULT_MAX_OUTPUT);
    if enable_stream {
        let mut enable_args = vec!["stream".to_string(), "enable".to_string()];
        if let Some(port) = request.port {
            enable_args.push("--port".to_string());
            enable_args.push(port.to_string());
        }
        let enable_refs: Vec<&str> = enable_args.iter().map(String::as_str).collect();
        let _ = run_json(&binary, Some(invoke_options.clone()), &enable_refs, 45).await;
    }
    let status = run_json(&binary, Some(invoke_options), &["stream", "status"], 30).await?;
    let port = stream_port_from_data(&status.data);
    let enabled = status.success && port.is_some();
    let websocket_url = port.map(|value| format!("ws://127.0.0.1:{value}"));
    Ok(AgentBrowserStreamStatusResponse {
        session,
        enabled,
        port,
        websocket_url,
        data: status.data,
    })
}

fn stream_port_from_data(data: &serde_json::Value) -> Option<u16> {
    for key in ["port", "streamPort", "stream_port"] {
        if let Some(value) = data.get(key).and_then(parse_port_value) {
            return Some(value);
        }
    }
    if let Some(nested) = data.get("stream").and_then(serde_json::Value::as_object) {
        for key in ["port", "streamPort"] {
            if let Some(value) = nested.get(key).and_then(parse_port_value) {
                return Some(value);
            }
        }
    }
    None
}

fn parse_port_value(value: &serde_json::Value) -> Option<u16> {
    match value {
        serde_json::Value::Number(number) => {
            number.as_u64().and_then(|port| u16::try_from(port).ok())
        }
        serde_json::Value::String(text) => text.trim().parse::<u16>().ok(),
        _ => None,
    }
}

pub async fn dashboard(
    request: AgentBrowserDashboardRequest,
) -> Result<AgentBrowserDashboardResponse, String> {
    let binary = resolve_binary()?;
    let action = request.action.trim().to_ascii_lowercase();
    let port = request.port.unwrap_or(4848);
    let (args, url): (Vec<String>, Option<String>) = match action.as_str() {
        "start" => (
            vec![
                "dashboard".to_string(),
                "start".to_string(),
                "--port".to_string(),
                port.to_string(),
            ],
            Some(format!("http://127.0.0.1:{port}")),
        ),
        "stop" => (vec!["dashboard".to_string(), "stop".to_string()], None),
        "status" => (
            vec!["dashboard".to_string(), "status".to_string()],
            Some(format!("http://127.0.0.1:{port}")),
        ),
        other => return Err(format!("Unsupported dashboard action: {other}")),
    };
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let response = run_json(&binary, None, &arg_refs, 60).await?;
    let success = response.success;
    let detail = if success {
        format!("agent-browser dashboard {action} succeeded")
    } else {
        response.text.clone()
    };
    Ok(AgentBrowserDashboardResponse {
        action,
        success,
        port: Some(port),
        url,
        detail,
        data: response.data,
    })
}

pub async fn skills(
    request: AgentBrowserSkillsRequest,
) -> Result<AgentBrowserSkillsResponse, String> {
    let binary = resolve_binary()?;
    let args: Vec<String> = if request.all == Some(true) {
        vec!["skills".to_string(), "get".to_string(), "--all".to_string()]
    } else if let Some(name) = request
        .name
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        vec![
            "skills".to_string(),
            "get".to_string(),
            name.trim().to_string(),
            "--full".to_string(),
        ]
    } else {
        vec!["skills".to_string(), "list".to_string()]
    };
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let response = run_json(&binary, None, &arg_refs, 90).await?;
    Ok(AgentBrowserSkillsResponse {
        success: response.success,
        content: response.text.clone(),
        data: response.data,
    })
}
