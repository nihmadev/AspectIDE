import type { AiChatSendInput } from "./../chat/types";
import { publicSecretFinding, scanSecrets } from "./secret-guard";
import { truncateText } from "./shared";
import type { TerminalSessionInfo } from "./../../types/index";

export function compactTerminalContext(input: AiChatSendInput, maxChars: number, onlySessionId?: string) {
  const sessions = terminalSessionsForContext(input, onlySessionId);
  return {
    activeTerminalId: input.terminalContext.activeTerminalId,
    sessionCount: input.terminalContext.sessions.length,
    sessions: sessions.map((session) => compactTerminalSession(session, input, maxChars)),
    notes: sessions.length > 0
      ? ["Terminal output is a recent UI buffer tail, not a full persistent log."]
      : ["No integrated terminal sessions are currently registered in the UI."],
  };
}

export function terminalSessionsForContext(input: AiChatSendInput, onlySessionId?: string) {
  const sessions = input.terminalContext.sessions.length > 0
    ? input.terminalContext.sessions
    : input.terminal
      ? [input.terminal]
      : [];
  const activeId = input.terminalContext.activeTerminalId ?? input.terminal?.id ?? null;
  const filtered = onlySessionId ? sessions.filter((session) => session.id === onlySessionId) : sessions;
  return [...filtered].sort((left, right) => Number(right.id === activeId) - Number(left.id === activeId));
}

export function compactTerminalSession(session: TerminalSessionInfo, input: AiChatSendInput, maxChars: number) {
  const buffer = input.terminalContext.outputBuffers[session.id];
  const rawTail = buffer?.text ?? "";
  const secretScan = scanSecrets(rawTail, `terminal.${shortTerminalId(session.id)}`);
  return {
    id: session.id,
    shortId: shortTerminalId(session.id),
    active: session.id === (input.terminalContext.activeTerminalId ?? input.terminal?.id),
    shell: session.shell,
    shellName: terminalShellName(session),
    cwd: session.cwd,
    createdAt: session.created_at,
    output: {
      chars: rawTail.length,
      chunks: buffer?.chunks ?? 0,
      bytesSeen: buffer?.bytes ?? 0,
      updatedAt: buffer?.updatedAt ?? null,
      truncated: Boolean(buffer?.truncated),
      tail: truncateText(secretScan.redactedText, maxChars),
      secretGuard: {
        redacted: secretScan.findings.length > 0,
        findingCount: secretScan.findings.length,
        findings: secretScan.findings.slice(0, 12).map(publicSecretFinding),
      },
    },
  };
}

export function selectTerminalSession(input: AiChatSendInput, sessionId: string) {
  const requested = sessionId.trim();
  if (requested) return input.terminalContext.sessions.find((session) => session.id === requested) ?? null;
  const activeId = input.terminalContext.activeTerminalId ?? input.terminal?.id ?? null;
  return input.terminalContext.sessions.find((session) => session.id === activeId) ?? input.terminal ?? null;
}

export function terminalShellName(session: TerminalSessionInfo) {
  const normalized = session.shell.replaceAll("\\", "/");
  return normalized.split("/").pop()?.replace(/\.exe$/i, "") || session.shell;
}

export function shortTerminalId(sessionId: string) {
  return sessionId.slice(0, 8);
}

export function terminalWritePreview(data: string) {
  const visible = data
    .replaceAll("\r", "\\r")
    .replaceAll("\n", "\\n")
    .replaceAll("\t", "\\t")
    .replaceAll("\x03", "^C");
  return truncateText(visible, 1_200);
}
