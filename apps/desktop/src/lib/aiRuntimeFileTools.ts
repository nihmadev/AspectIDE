import type { AiChatSendInput } from "./aiChatTypes";
import { recordTurnCheckpointFileChanges } from "./aiChatTurnCheckpoints";
import { captureFileTextSnapshot, registerPendingFileReview } from "./aiPendingFileReview";
import { createDeleteApproval, createPatchApproval, createStrReplaceApproval, createWriteApproval } from "./aiRuntimeApprovals";
import { ensurePathsInFileCheckpoint } from "./aiRuntimeCheckpoints";
import { patchOperationsArg } from "./aiRuntimePatch";
import { booleanArg, clamp, maxToolOutputChars, normalizePathSlashes, numberArg, readErrorMessage, stringArg, truncateText, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { requireToolApproval, type ToolExecutionUi } from "./aiRuntimeToolApproval";
import { luxCommands } from "./tauri";

export type FileToolResult = Awaited<ReturnType<typeof luxCommands.aiFileWrite>>;

export function resolveFileEditSaveToDisk(input: AiChatSendInput, requested: boolean) {
  if (!requested) return false;
  if (input.preferences.agentMode === "automatic") return true;
  return input.preferences.fileEditTrustMode !== "preview-before-apply";
}

export function toolResultFromFileOperation(title: string, result: FileToolResult): ToolResult {
  return {
    title: result.message,
    content: truncateText(JSON.stringify({
      operation: result.operation,
      path: result.path,
      savedToDisk: result.savedToDisk,
      changedPaths: result.changedPaths,
      stats: result.stats,
      message: result.message,
    }, null, 2), maxToolOutputChars),
    stats: result.stats,
  };
}

export async function writeFileTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const path = stringArg(args, "path");
  const text = stringArg(args, "text");
  const overwrite = booleanArg(args, "overwrite", false);
  const saveToDisk = resolveFileEditSaveToDisk(input, booleanArg(args, "saveToDisk", true));
  const approval = createWriteApproval(input.locale, path, text, overwrite, saveToDisk);
  await requireToolApproval(input, ui, approval);
  await guardTurnFilePaths(input, [path]);
  const beforeText = await snapshotPathBeforeEdit(input, path);
  const result = await luxCommands.aiFileWrite(path, text, overwrite, saveToDisk);
  registerReviewFromFileOperation("Write", path, input, beforeText, result, ui.toolCallId, !saveToDisk);
  notifyFilePathsEdited(input, result.changedPaths);
  return toolResultFromFileOperation("Write", result);
}

export async function strReplaceTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const path = stringArg(args, "path");
  const oldText = stringArg(args, "oldText");
  const newText = stringArg(args, "newText");
  const expectedReplacements = clamp(numberArg(args, "expectedReplacements", 1), 1, 1000);
  const saveToDisk = resolveFileEditSaveToDisk(input, booleanArg(args, "saveToDisk", true));
  const approval = createStrReplaceApproval(input.locale, path, oldText, newText, expectedReplacements, saveToDisk);
  await requireToolApproval(input, ui, approval);
  await guardTurnFilePaths(input, [path]);
  const beforeText = await snapshotPathBeforeEdit(input, path);
  const result = await luxCommands.aiFileStrReplace(path, oldText, newText, expectedReplacements, saveToDisk);
  registerReviewFromFileOperation("StrReplace", path, input, beforeText, result, ui.toolCallId, !saveToDisk);
  notifyFilePathsEdited(input, result.changedPaths);
  return toolResultFromFileOperation("StrReplace", result);
}

export async function patchEngineTool(args: UnknownRecord, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const operations = patchOperationsArg(args);
  const saveToDisk = resolveFileEditSaveToDisk(input, booleanArg(args, "saveToDisk", true));
  const dryRun = booleanArg(args, "dryRun", false);
  const approval = createPatchApproval(input.locale, operations, saveToDisk, dryRun);
  await requireToolApproval(input, ui, approval);
  await guardTurnFilePaths(input, operations.map((operation) => operation.path));
  const paths = [...new Set(operations.map((operation) => operation.path))];
  const beforeByPath = new Map<string, string>();
  for (const path of paths) {
    beforeByPath.set(path, await snapshotPathBeforeEdit(input, path));
  }
  const result = await luxCommands.aiFilePatch(operations, saveToDisk, dryRun);
  for (const path of result.changedPaths) {
    registerReviewFromFileOperation("PatchEngine", path, input, beforeByPath.get(path) ?? "", result, ui.toolCallId, !saveToDisk);
  }
  notifyFilePathsEdited(input, result.changedPaths);
  return toolResultFromFileOperation("PatchEngine", result);
}

export async function deleteFileTool(path: string, input: AiChatSendInput, ui: ToolExecutionUi): Promise<ToolResult> {
  const approval = createDeleteApproval(input.locale, path);
  await requireToolApproval(input, ui, approval);
  await guardTurnFilePaths(input, [path]);
  const result = await luxCommands.aiFileDelete(path);
  notifyFilePathsEdited(input, result.changedPaths);
  return toolResultFromFileOperation("Delete", result);
}

async function guardTurnFilePaths(input: AiChatSendInput, paths: string[]) {
  const turn = input.turnCheckpoint;
  const workspaceRoot = input.workspace?.root;
  if (!turn || !workspaceRoot) return;
  const normalized = paths.map((path) => normalizePathSlashes(path)).filter(Boolean);
  if (normalized.length === 0) return;
  try {
    await ensurePathsInFileCheckpoint(workspaceRoot, turn.fileCheckpointId, normalized, input);
    recordTurnCheckpointFileChanges({
      sessionId: input.chatSessionId,
      turnCheckpointId: turn.turnCheckpointId,
      paths: normalized,
    });
  } catch (error) {
    console.warn("Turn file checkpoint extension failed:", readErrorMessage(error));
  }
}

async function snapshotPathBeforeEdit(input: AiChatSendInput, path: string) {
  const open = input.openDocuments.find((document) => document.path && normalizePathForCompare(document.path) === normalizePathForCompare(path));
  return captureFileTextSnapshot(path, open?.text);
}

function registerReviewFromFileOperation(
  toolName: string,
  path: string,
  input: AiChatSendInput,
  beforeText: string,
  result: FileToolResult,
  toolCallId: string,
  previewOnly: boolean,
) {
  if (!path || !result.changedPaths.includes(path)) return;
  const edited = result.editedDocuments.find((document) => document.path && normalizePathForCompare(document.path) === normalizePathForCompare(path));
  const afterText = edited?.text ?? beforeText;
  if (beforeText === afterText) return;
  registerPendingFileReview({
    sessionId: input.chatSessionId,
    path,
    relativePath: path,
    toolName,
    toolCallId,
    beforeText,
    afterText,
    previewOnly,
  });
}

function notifyFilePathsEdited(input: AiChatSendInput, paths: string[]) {
  if (paths.length === 0) return;
  input.onFilePathsEdited?.(paths);
}

function normalizePathForCompare(path: string) {
  return path.replaceAll("\\", "/").toLowerCase();
}