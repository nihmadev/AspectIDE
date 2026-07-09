import type { AiChatSendInput } from "./../chat/types";
import { createShellApproval, createTerminalWriteApproval } from "./approvals";
import { publicSecretFinding, scanSecrets } from "./secret-guard";
import { requireToolApproval, type ToolExecutionUi } from "./tool-approval";
import { clamp, numberArg, stringArg, toolJson, type ToolResult, type UnknownRecord } from "./shared";
import { compactTerminalContext, compactTerminalSession, selectTerminalSession, terminalWritePreview } from "./terminal";
import { luxCommands } from "./../../tauri/commands";

export async function shellTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const command = stringArg(args, "command");
  const cwd = stringArg(args, "cwd", input.workspace?.root ?? "");
  const timeoutSecs = clamp(numberArg(args, "timeoutSecs", 120), 1, 600);

  // Rust is the authoritative safety classifier; classify before prompting so we
  // can reject catastrophic commands up front and auto-approve read-only ones.
  const classification = await luxCommands.aiShellClassify(command).catch(() => null);
  if (classification?.blocked) {
    throw new Error(`Lux blocked this command for safety (${classification.blocked}). Run it manually in the integrated terminal if it is genuinely intended.`);
  }

  const approval = createShellApproval(input.locale, command, cwd, timeoutSecs);
  // Permission rules win; otherwise a classified read-only command (ls, git
  // status, cat …) auto-approves without a prompt.
  await requireToolApproval(input, ui, approval, { autoApproveOnDefault: Boolean(classification?.readOnly) });
  const result = await luxCommands.aiShell(command, cwd || null, timeoutSecs);
  const stdoutScan = scanSecrets(result.stdout, "shell.stdout");
  const stderrScan = scanSecrets(result.stderr, "shell.stderr");
  const secretFindings = [...stdoutScan.findings, ...stderrScan.findings];
  return toolJson("Shell", {
    workspaceRoot: result.workspaceRoot,
    cwd: result.cwd,
    command: result.command,
    exitCode: result.exitCode,
    durationMs: result.durationMs,
    timedOut: result.timedOut,
    stdout: stdoutScan.redactedText,
    stderr: stderrScan.redactedText,
    ...(result.warnings && result.warnings.length > 0 ? { safetyWarnings: result.warnings } : {}),
    secretGuard: {
      redacted: secretFindings.length > 0,
      findingCount: secretFindings.length,
      findings: secretFindings.slice(0, 20).map(publicSecretFinding),
    },
  });
}

export function terminalContextTool(args: UnknownRecord, input: AiChatSendInput): ToolResult {
  const sessionId = stringArg(args, "sessionId", "").trim();
  const maxChars = clamp(numberArg(args, "maxChars", 12_000), 500, 24_000);
  return toolJson("TerminalContext", compactTerminalContext(input, maxChars, sessionId || undefined));
}

export async function terminalWriteTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const data = stringArg(args, "data");
  if (!data) throw new Error("TerminalWrite requires non-empty data.");
  const session = selectTerminalSession(input, stringArg(args, "sessionId", ""));
  if (!session) throw new Error("TerminalWrite requires an active terminal session.");
  const approval = createTerminalWriteApproval(input.locale, session, data);
  await requireToolApproval(input, ui, approval);
  await luxCommands.terminalWrite(session.id, data);
  return toolJson("TerminalWrite", {
    session: compactTerminalSession(session, input, 1_200),
    bytesWritten: data.length,
    preview: terminalWritePreview(data),
  });
}