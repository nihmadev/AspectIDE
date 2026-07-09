import type { AiChatSendInput, AiToolApprovalDecision, AiToolApprovalRequest, AiToolApprovalState } from "./../chat/types";
import { luxCommands } from "./../../tauri/commands";

export type ToolExecutionUi = {
  toolCallId: string;
  setApproval: (approval: AiToolApprovalState) => void;
  setRunning: (approval?: AiToolApprovalState) => void;
};

export class ToolApprovalRejectedError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "ToolApprovalRejectedError";
  }
}

/** Tools whose permission pattern matches the target path rather than the command/preview. */
const PATH_MATCHED_TOOLS = new Set(["Write", "StrReplace", "PatchEngine", "Delete", "Checkpoint"]);

function permissionMatchInput(approval: AiToolApprovalRequest): string {
  if (PATH_MATCHED_TOOLS.has(approval.tool)) return approval.path ?? "";
  return approval.preview ?? approval.path ?? "";
}

/**
 * Evaluate the user's declarative permission rules (Rust engine) for this call.
 * deny → refuse before prompting; allow → skip the prompt; ask/default → fall
 * through to mode-based behaviour. Never throws on engine failure (fail-open to
 * the normal approval flow).
 */
async function evaluatePermissionRules(
  input: AiChatSendInput,
  approval: AiToolApprovalRequest,
): Promise<"allow" | "deny" | "ask" | "default"> {
  const rules = input.preferences.toolPermissionRules;
  if (!rules || rules.length === 0) return "default";
  try {
    const result = await luxCommands.aiPermissionDecide(approval.tool, permissionMatchInput(approval), rules);
    return result.decision;
  } catch {
    return "default";
  }
}

export async function requireToolApproval(
  input: AiChatSendInput,
  ui: ToolExecutionUi,
  approval: AiToolApprovalRequest,
  options?: {
    /** When no rule matches, auto-run instead of prompting (e.g. classified read-only commands). */
    autoApproveOnDefault?: boolean;
  },
) {
  if (input.abortSignal.aborted) throw new DOMException("AI request was cancelled", "AbortError");

  const ruleDecision = await evaluatePermissionRules(input, approval);
  if (input.abortSignal.aborted) throw new DOMException("AI request was cancelled", "AbortError");

  // A deny rule blocks unconditionally — even in Full Access mode.
  if (ruleDecision === "deny") {
    const denied = { ...approval, decision: "rejected" as AiToolApprovalDecision };
    ui.setApproval(denied);
    throw new ToolApprovalRejectedError(`${approval.tool} is blocked by a permission rule.`);
  }

  // Automatic mode is full autonomy: never prompt for approval (deny rules above
  // still block). Full Access or an explicit allow rule also run without prompting.
  if (
    input.preferences.agentMode === "automatic"
    || input.preferences.toolApprovalMode === "full-access"
    || ruleDecision === "allow"
  ) {
    ui.setRunning({ ...approval, decision: "approved" });
    return;
  }

  // No matching rule, but the caller vouched the call is safe (read-only) → run.
  if (ruleDecision !== "ask" && options?.autoApproveOnDefault) {
    ui.setRunning({ ...approval, decision: "approved" });
    return;
  }

  ui.setApproval(approval);
  const decision: AiToolApprovalDecision = await input.onToolApproval(approval);
  if (input.abortSignal.aborted) throw new DOMException("AI request was cancelled", "AbortError");
  const approvalState = { ...approval, decision };
  if (decision !== "approved") {
    ui.setApproval(approvalState);
    throw new ToolApprovalRejectedError(`${approval.tool} was rejected by the user.`);
  }
  ui.setRunning(approvalState);
}