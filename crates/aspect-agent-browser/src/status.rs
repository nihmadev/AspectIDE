use crate::probes::{list_sessions, read_version};
use crate::resolver::{binary_source_label, resolve_binary_with_source};
use crate::process::run_json;
use crate::types::AgentBrowserStatusRequest;
use crate::types::AgentBrowserStatusResponse;
use crate::version::{normalize_agent_browser_version, version_is_older};

pub async fn status(
    request: AgentBrowserStatusRequest,
) -> Result<AgentBrowserStatusResponse, String> {
    let lightweight = request.lightweight == Some(true);
    status_inner(lightweight).await
}

#[allow(clippy::too_many_lines)]
async fn status_inner(lightweight: bool) -> Result<AgentBrowserStatusResponse, String> {
    let (binary, source) = resolve_binary_with_source()?;

    let version = read_version(&binary)
        .await
        .ok()
        .map(|text| normalize_agent_browser_version(&text));

    let latest_version: Option<String> = None;

    let sessions = if lightweight {
        Vec::new()
    } else {
        list_sessions(&binary).await.unwrap_or_default()
    };

    let doctor_outcome: Option<Result<serde_json::Value, String>> = if lightweight {
        None
    } else {
        Some(
            run_json(
                &binary,
                None,
                &["doctor", "--json", "--offline", "--quick"],
                12,
            )
            .await
            .map(|resp| resp.data),
        )
    };
    let doctor_hung = matches!(&doctor_outcome, Some(Err(error)) if error.contains("timed out"));
    let doctor: Option<serde_json::Value> = doctor_outcome.and_then(std::result::Result::ok);

    let available = if lightweight {
        version.is_some()
    } else if doctor.is_some() {
        doctor
            .as_ref()
            .and_then(|value| value.get("success"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    } else {
        version.is_some()
    };

    let detail = if available {
        if doctor_hung {
            format!(
                "agent-browser responds ({}), but its `doctor` subcommand hung and was skipped — \
                 a known CLI issue on some machines. Automation is likely functional.",
                version
                    .clone()
                    .unwrap_or_else(|| "version unknown".to_string())
            )
        } else {
            format!(
                "agent-browser is available ({})",
                version
                    .clone()
                    .unwrap_or_else(|| "version unknown".to_string())
            )
        }
    } else if lightweight {
        if version.is_some() {
            format!(
                "agent-browser {} (lightweight check)",
                version.as_deref().unwrap_or("version unknown")
            )
        } else {
            "agent-browser resolved but version unknown".to_string()
        }
    } else {
        "agent-browser responded, but doctor reported issues. Run `agent-browser doctor --fix` \
         in a terminal."
            .to_string()
    };

    let (update_performed, update_detail) =
        if let (Some(current), Some(latest)) = (version.as_ref(), latest_version.as_ref()) {
            if version_is_older(current, latest) {
                (
                    false,
                    Some(format!("Update available: {latest} (installed: {current})")),
                )
            } else {
                (false, None)
            }
        } else {
            (false, None)
        };

    Ok(AgentBrowserStatusResponse {
        available,
        command_path: Some(binary.display().to_string()),
        version,
        latest_version,
        update_performed,
        update_detail,
        detail,
        sessions,
        doctor,
        binary_source: Some(binary_source_label(source).to_string()),
    })
}
