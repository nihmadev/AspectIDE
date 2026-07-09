use aspect_ai_core::*;

use super::commands::emit_turn_event;

#[allow(clippy::too_many_arguments)]
pub async fn require_tool_approval(
    app: &tauri::AppHandle,
    turn_id: &str,
    tc: &ParsedToolCall,
    approval_mode: &str,
    interactive: bool,
    tool: &str,
    summary: &str,
    preview: &str,
    risk: &str,
    rules: &[String],
    permission_input: &str,
    auto_approve: bool,
) -> Result<(), String> {
    let auto_approve_list: Vec<String> = if auto_approve {
        vec![tc.name.clone()]
    } else {
        Vec::new()
    };
    match resolve_approval_gate(
        tc, permission_input, rules, approval_mode, interactive, &auto_approve_list,
    ) {
        ApprovalGate::Blocked => return Err(format!("{tool} blocked by permission rules")),
        ApprovalGate::RejectedNonInteractive => return Err(format!("{tool} requires interactive approval")),
        ApprovalGate::Allowed => return Ok(()),
        ApprovalGate::Prompt => {}
    }

    let rx = register_approval(turn_id, &tc.id);
    if is_turn_cancelled(turn_id) {
        cancel_approvals_for_turn(turn_id);
        return Err(format!("{tool} cancelled: the turn was stopped by the user."));
    }
    if let Err(emit_err) = emit_turn_event(
        app,
        &TurnEvent::ApprovalRequired {
            turn_id: turn_id.to_string(),
            request_id: tc.id.clone(),
            tool: tool.to_string(),
            title: format!("Approve {tool}"),
            summary: summary.to_string(),
            preview: preview.to_string(),
            risk: risk.to_string(),
        },
    ) {
        cancel_approvals_for_turn(turn_id);
        return Err(format!("{tool} approval could not be delivered to the UI ({emit_err}); tool skipped."));
    }

    const APPROVAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);
    match tokio::time::timeout(APPROVAL_TIMEOUT, rx).await {
        Ok(Ok(ApprovalDecision::Approved)) => Ok(()),
        Ok(_) => Err(format!("{tool} was rejected by the user.")),
        Err(_elapsed) => {
            cancel_approvals_for_turn(turn_id);
            Err(format!(
                "{tool} approval timed out after {}s. If the approval card disappeared, retry the action.",
                APPROVAL_TIMEOUT.as_secs()
            ))
        }
    }
}
