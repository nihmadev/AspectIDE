import type { AiToolApprovalRequest } from "./aiChatTypes";
import { buildNumberedPreview, buildPatchPreview, buildReplacementPreview, countLines, patchOperationCounts, type RuntimePatchOperation } from "./aiRuntimePatch";
import { terminalShellName, terminalWritePreview, shortTerminalId } from "./aiRuntimeTerminal";
import { DEFAULT_LOCALE, translate, type Locale, type MessageKey, type MessageParams } from "./i18n";
import type { TerminalSessionInfo } from "./types";

export function createWriteApproval(locale: Locale, path: string, text: string, overwrite: boolean, saveToDisk: boolean): AiToolApprovalRequest {
  const lines = countLines(text);
  return {
    id: crypto.randomUUID(),
    tool: "Write",
    title: approvalText(locale, overwrite ? "aiApproval.write.rewrite.title" : "aiApproval.write.create.title"),
    path,
    summary: approvalText(locale, overwrite ? "aiApproval.write.rewrite.summary" : "aiApproval.write.create.summary", { path, lines, target: approvalTarget(locale, saveToDisk) }),
    preview: buildNumberedPreview(text, 80),
    risk: overwrite ? "modify" : "create",
    approveLabel: approvalText(locale, overwrite ? "aiApproval.write.rewrite.approve" : "aiApproval.write.create.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createStrReplaceApproval(locale: Locale, path: string, oldText: string, newText: string, expectedReplacements: number, saveToDisk: boolean): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "StrReplace",
    title: approvalText(locale, "aiApproval.strReplace.title"),
    path,
    summary: approvalText(locale, "aiApproval.strReplace.summary", { count: expectedReplacements, path, target: approvalTarget(locale, saveToDisk) }),
    preview: buildReplacementPreview(oldText, newText),
    risk: "modify",
    approveLabel: approvalText(locale, "aiApproval.strReplace.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createPatchApproval(locale: Locale, operations: RuntimePatchOperation[], saveToDisk: boolean, dryRun: boolean): AiToolApprovalRequest {
  const counts = patchOperationCounts(operations);
  const paths = [...new Set(operations.map((operation) => operation.path))];
  return {
    id: crypto.randomUUID(),
    tool: "PatchEngine",
    title: approvalText(locale, dryRun ? "aiApproval.patch.dryRun.title" : "aiApproval.patch.apply.title"),
    path: paths.length === 1 ? paths[0] : approvalText(locale, "aiApproval.paths", { count: paths.length }),
    summary: approvalText(locale, dryRun ? "aiApproval.patch.dryRun.summary" : "aiApproval.patch.apply.summary", {
      count: operations.length,
      create: counts.create,
      rewrite: counts.rewrite,
      replace: counts.replace,
      delete: counts.delete,
      target: approvalPatchTarget(locale, saveToDisk, dryRun),
    }),
    preview: buildPatchPreview(operations),
    risk: counts.delete > 0 ? "delete" : counts.create > 0 && counts.rewrite === 0 && counts.replace === 0 ? "create" : "modify",
    approveLabel: approvalText(locale, dryRun ? "aiApproval.patch.dryRun.approve" : "aiApproval.patch.apply.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createCheckpointRestoreApproval(locale: Locale, details: { id: string; label: string }, operations: RuntimePatchOperation[], saveToDisk: boolean, dryRun: boolean): AiToolApprovalRequest {
  const counts = patchOperationCounts(operations);
  const paths = [...new Set(operations.map((operation) => operation.path))];
  return {
    id: crypto.randomUUID(),
    tool: "Checkpoint",
    title: approvalText(locale, dryRun ? "aiApproval.checkpoint.dryRun.title" : "aiApproval.checkpoint.restore.title"),
    path: paths.length === 1 ? paths[0] : approvalText(locale, "aiApproval.paths", { count: paths.length }),
    summary: approvalText(locale, dryRun ? "aiApproval.checkpoint.dryRun.summary" : "aiApproval.checkpoint.restore.summary", {
      id: details.id,
      label: details.label,
      count: operations.length,
      create: counts.create,
      rewrite: counts.rewrite,
      replace: counts.replace,
      delete: counts.delete,
      target: approvalPatchTarget(locale, saveToDisk, dryRun),
    }),
    preview: buildPatchPreview(operations),
    risk: counts.delete > 0 ? "delete" : counts.create > 0 && counts.rewrite === 0 && counts.replace === 0 ? "create" : "modify",
    approveLabel: approvalText(locale, dryRun ? "aiApproval.patch.dryRun.approve" : "aiApproval.checkpoint.restore.approve"),
    rejectLabel: approvalText(locale, "aiApproval.checkpoint.keepCurrent"),
  };
}

export function createDeleteApproval(locale: Locale, path: string): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "Delete",
    title: approvalText(locale, "aiApproval.delete.title"),
    path,
    summary: approvalText(locale, "aiApproval.delete.summary", { path }),
    preview: `- ${path}`,
    risk: "delete",
    approveLabel: approvalText(locale, "aiApproval.delete.approve"),
    rejectLabel: approvalText(locale, "aiApproval.delete.keep"),
  };
}

export function createShellApproval(locale: Locale, command: string, cwd: string, timeoutSecs: number): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "Shell",
    title: approvalText(locale, "aiApproval.shell.title"),
    path: cwd || ".",
    summary: approvalText(locale, "aiApproval.shell.summary", { cwd: cwd || approvalText(locale, "aiApproval.workspace"), timeoutSecs }),
    preview: command,
    risk: "execute",
    approveLabel: approvalText(locale, "aiApproval.shell.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createBrowserOpenApproval(locale: Locale, url: string, session: string, headed: boolean): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "BrowserOpen",
    title: approvalText(locale, "aiApproval.browserOpen.title"),
    path: url || "about:blank",
    summary: approvalText(locale, "aiApproval.browserOpen.summary", { session, headed: headed ? "yes" : "no" }),
    preview: url || approvalText(locale, "aiApproval.browserOpen.emptyUrl"),
    risk: "execute",
    approveLabel: approvalText(locale, "aiApproval.browserOpen.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createBrowserActApproval(locale: Locale, command: string, session: string): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "BrowserAct",
    title: approvalText(locale, "aiApproval.browserAct.title"),
    path: session,
    summary: approvalText(locale, "aiApproval.browserAct.summary", { session }),
    preview: command,
    risk: "execute",
    approveLabel: approvalText(locale, "aiApproval.browserAct.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createBrowserChatApproval(locale: Locale, instruction: string, session: string): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "BrowserChat",
    title: approvalText(locale, "aiApproval.browserChat.title"),
    path: session,
    summary: approvalText(locale, "aiApproval.browserChat.summary", { session }),
    preview: instruction,
    risk: "execute",
    approveLabel: approvalText(locale, "aiApproval.browserChat.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createBrowserInstallApproval(locale: Locale, withDeps: boolean): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "BrowserInstall",
    title: approvalText(locale, "aiApproval.browserInstall.title"),
    path: "agent-browser",
    summary: approvalText(locale, withDeps ? "aiApproval.browserInstall.summaryDeps" : "aiApproval.browserInstall.summary"),
    preview: approvalText(locale, "aiApproval.browserInstall.preview"),
    risk: "execute",
    approveLabel: approvalText(locale, "aiApproval.browserInstall.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createBrowserCloseApproval(locale: Locale, all: boolean, session: string): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "BrowserAct",
    title: approvalText(locale, "aiApproval.browserClose.title"),
    path: session,
    summary: all
      ? approvalText(locale, "aiApproval.browserClose.allSummary")
      : approvalText(locale, "aiApproval.browserClose.summary", { session }),
    preview: all ? approvalText(locale, "aiApproval.browserClose.allPreview") : approvalText(locale, "aiApproval.browserClose.preview"),
    risk: "execute",
    approveLabel: approvalText(locale, "aiApproval.browserClose.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

export function createTerminalWriteApproval(locale: Locale, session: TerminalSessionInfo, data: string): AiToolApprovalRequest {
  return {
    id: crypto.randomUUID(),
    tool: "TerminalWrite",
    title: approvalText(locale, "aiApproval.terminalWrite.title"),
    path: session.cwd || ".",
    summary: approvalText(locale, "aiApproval.terminalWrite.summary", { count: data.length, terminal: shortTerminalId(session.id), shell: terminalShellName(session) }),
    preview: terminalWritePreview(data),
    risk: "execute",
    approveLabel: approvalText(locale, "aiApproval.terminalWrite.approve"),
    rejectLabel: approvalText(locale, "aiApproval.reject"),
  };
}

function approvalText(locale: Locale | undefined, key: MessageKey, params?: MessageParams) {
  return translate(locale ?? DEFAULT_LOCALE, key, params);
}

function approvalTarget(locale: Locale, saveToDisk: boolean) {
  return approvalText(locale, saveToDisk ? "aiApproval.target.disk" : "aiApproval.target.editor");
}

function approvalPatchTarget(locale: Locale, saveToDisk: boolean, dryRun: boolean) {
  return approvalText(locale, saveToDisk && !dryRun ? "aiApproval.target.disk" : "aiApproval.target.validation");
}
