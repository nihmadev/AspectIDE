import type { AiChatSendInput, AiToolApprovalRequest } from "./../chat/types";
import { publicSecretFinding, scanSecrets } from "./secret-guard";
import { clamp, numberArg, stringArg, toolJson, type ToolResult, type UnknownRecord } from "./shared";
import { requireToolApproval, type ToolExecutionUi } from "./tool-approval";
import { luxCommands, type SshTransferDirection } from "./../../tauri/commands";

/**
 * Web/dev fallback execution for the `Ssh*` tools (the desktop runtime drives the
 * native turn loop in Rust). Mirrors the Shell/Terminal tools: explicit approval
 * for side-effecting calls, the same catastrophic-command classifier for remote
 * commands, and secret redaction on captured output.
 */

function sshApproval(
  tool: AiToolApprovalRequest["tool"],
  title: string,
  path: string,
  summary: string,
  preview: string,
): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool,
    title,
    path,
    summary,
    preview,
    risk: "execute",
    approveLabel: "Approve",
    rejectLabel: "Reject",
  };
}

export async function sshConnectTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const host = stringArg(args, "host").trim();
  if (!host) throw new Error("SshConnect requires a host (an ~/.ssh/config alias, a hostname/IP, or user@host).");
  const user = stringArg(args, "user", "").trim() || null;
  const port = numberArg(args, "port", 0);
  const identityFile = stringArg(args, "identityFile", "").trim() || null;
  const label = stringArg(args, "label", "").trim() || null;

  await requireToolApproval(input, ui, sshApproval(
    "SshConnect",
    "Open SSH connection",
    host,
    `Connect to ${host} over SSH (non-interactive, key/agent auth).`,
    host,
  ));
  const result = await luxCommands.sshConnect(host, user, port > 0 ? port : null, identityFile, label);
  return toolJson("SshConnect", result);
}

export async function sshExecTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const session = stringArg(args, "session").trim();
  if (!session) throw new Error("SshExec requires a sessionId from SshConnect.");
  const command = stringArg(args, "command").trim();
  if (!command) throw new Error("SshExec requires a non-empty command.");
  const cwd = stringArg(args, "cwd", "").trim() || null;
  const timeoutSecs = clamp(numberArg(args, "timeoutSecs", 120), 1, 600);

  // Rust is the authoritative safety classifier; classify first so a catastrophic
  // remote command is rejected up front and a read-only one auto-approves.
  const classification = await luxCommands.aiShellClassify(command).catch(() => null);
  if (classification?.blocked) {
    throw new Error(`Lux blocked this remote command for safety (${classification.blocked}). If genuinely intended, run it yourself.`);
  }
  await requireToolApproval(
    input,
    ui,
    sshApproval("SshExec", "Run remote command", session, `Run on SSH session ${session}.`, command),
    { autoApproveOnDefault: Boolean(classification?.readOnly) },
  );
  const result = await luxCommands.sshExec(session, command, cwd, timeoutSecs);
  const stdoutScan = scanSecrets(result.stdout, "ssh.stdout");
  const stderrScan = scanSecrets(result.stderr, "ssh.stderr");
  const secretFindings = [...stdoutScan.findings, ...stderrScan.findings];
  return toolJson("SshExec", {
    ...result,
    stdout: stdoutScan.redactedText,
    stderr: stderrScan.redactedText,
    secretGuard: {
      redacted: secretFindings.length > 0,
      findingCount: secretFindings.length,
      findings: secretFindings.slice(0, 20).map(publicSecretFinding),
    },
  });
}

export async function sshTransferTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const session = stringArg(args, "session").trim();
  if (!session) throw new Error("SshTransfer requires a sessionId from SshConnect.");
  const direction = stringArg(args, "direction").trim().toLowerCase();
  if (direction !== "upload" && direction !== "download") {
    throw new Error('SshTransfer direction must be "upload" or "download".');
  }
  const localPath = stringArg(args, "localPath").trim();
  const remotePath = stringArg(args, "remotePath").trim();
  if (!localPath || !remotePath) throw new Error("SshTransfer requires localPath and remotePath.");
  const recursive = args.recursive === true;

  const arrow = direction === "upload" ? "→" : "←";
  await requireToolApproval(input, ui, sshApproval(
    "SshTransfer",
    "Transfer file over SSH",
    session,
    `scp ${direction}: ${localPath} ${arrow} ${remotePath}`,
    `${localPath}  ${remotePath}`,
  ));
  const result = await luxCommands.sshTransfer(session, direction as SshTransferDirection, localPath, remotePath, recursive);
  const stdoutScan = scanSecrets(result.stdout, "ssh.transfer.stdout");
  const stderrScan = scanSecrets(result.stderr, "ssh.transfer.stderr");
  const secretFindings = [...stdoutScan.findings, ...stderrScan.findings];
  return toolJson("SshTransfer", {
    ...result,
    stdout: stdoutScan.redactedText,
    stderr: stderrScan.redactedText,
    secretGuard: {
      redacted: secretFindings.length > 0,
      findingCount: secretFindings.length,
      findings: secretFindings.slice(0, 20).map(publicSecretFinding),
    },
  });
}

export async function sshListTool(): Promise<ToolResult> {
  const result = await luxCommands.sshList();
  return toolJson("SshList", result);
}

export async function sshDisconnectTool(args: UnknownRecord): Promise<ToolResult> {
  const session = stringArg(args, "session", "").trim() || null;
  const all = args.all === true;
  if (!session && !all) throw new Error("SshDisconnect requires a sessionId or all=true.");
  const result = await luxCommands.sshDisconnect(session, all);
  return toolJson("SshDisconnect", result);
}
