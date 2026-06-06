import { getTurnCheckpoint, listTurnCheckpoints } from "./aiChatCheckpointStore";
import { findTurnCheckpointForUserMessage } from "./aiChatTurnCheckpoints";
import { restoreFileCheckpointById } from "./aiRuntimeCheckpoints";
import { normalizePathSlashes, readErrorMessage } from "./aiRuntimeShared";
import { isPathInsideWorkspace, resolveWorkspacePath } from "./aiRuntimeFileContext";
import type { AiChatMessage, AiChatSendInput } from "./aiChatTypes";
import { luxCommands } from "./tauri";
import { useLuxStore } from "./store";
import { loadTurnCheckpointMessages, pruneTurnCheckpointsFromIndex } from "./aiChatTurnCheckpoints";

export type RestoreChatTurnMode = "before-user" | "after-user";

export type RestoreChatTurnResult = {
  messages: AiChatMessage[];
  restoredFileCount: number;
  removedTurnCheckpoints: number;
};

export async function restoreChatToTurnCheckpoint(params: {
  input: AiChatSendInput;
  mode: RestoreChatTurnMode;
  sessionId: string;
  turnCheckpointId: string;
  currentMessages: AiChatMessage[];
}): Promise<RestoreChatTurnResult> {
  const workspaceRoot = params.input.workspace?.root;
  if (!workspaceRoot) throw new Error("Open a workspace to restore checkpoints.");

  const turn = getTurnCheckpoint(params.sessionId, params.turnCheckpointId);
  if (!turn) throw new Error("Turn checkpoint not found.");

  const userIndex = params.currentMessages.findIndex((message) => message.id === turn.userMessageId);
  const baseMessages = loadTurnCheckpointMessages(params.sessionId, turn.id);
  const nextMessages = params.mode === "before-user"
    ? baseMessages
    : userIndex >= 0
      ? params.currentMessages.slice(0, userIndex + 1)
      : baseMessages;

  const applyPatch = async (operations: Parameters<typeof luxCommands.aiFilePatch>[0], saveToDisk: boolean, dryRun: boolean) => {
    const result = await luxCommands.aiFilePatch(operations, saveToDisk, dryRun);
    return { title: result.message, content: result.message };
  };

  const restore = await restoreFileCheckpointById(workspaceRoot, turn.fileCheckpointId, params.input, applyPatch);
  await reloadOpenDocumentsFromPaths(workspaceRoot, restore.restoredPaths);

  const allTurns = listTurnCheckpoints(params.sessionId);
  const checkpointIndex = allTurns.findIndex((entry) => entry.id === turn.id);
  const removedTurnCheckpoints = checkpointIndex >= 0 ? allTurns.length - checkpointIndex : 0;
  if (checkpointIndex >= 0) pruneTurnCheckpointsFromIndex(params.sessionId, checkpointIndex);

  return {
    messages: nextMessages,
    restoredFileCount: restore.restoredPaths.length,
    removedTurnCheckpoints,
  };
}

export async function restoreChatBeforeUserMessage(params: {
  currentMessages: AiChatMessage[];
  input: AiChatSendInput;
  sessionId: string;
  userMessageId: string;
}) {
  const turn = listTurnCheckpoints(params.sessionId).find((entry) => entry.userMessageId === params.userMessageId);
  if (!turn) throw new Error("No turn checkpoint for this message.");
  return restoreChatToTurnCheckpoint({
    currentMessages: params.currentMessages,
    input: params.input,
    mode: "before-user",
    sessionId: params.sessionId,
    turnCheckpointId: turn.id,
  });
}

/** OpenCode-style /undo: roll back the latest agent turn's file edits; keep the user message. */
export function findLastUndoableUserTurn(messages: AiChatMessage[], sessionId: string) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    if (message.role !== "user" || message.visibility === "internal") continue;
    const turn = findTurnCheckpointForUserMessage(sessionId, message.id);
    if (!turn) continue;
    const hasAgentAfter = messages.slice(index + 1).some((entry) => entry.role === "assistant" && entry.visibility !== "internal");
    if (!hasAgentAfter) continue;
    return { userMessage: message, turn };
  }
  return null;
}

export async function undoLastAgentTurn(params: {
  currentMessages: AiChatMessage[];
  input: AiChatSendInput;
  sessionId: string;
}) {
  const target = findLastUndoableUserTurn(params.currentMessages, params.sessionId);
  if (!target) {
    throw new Error("Nothing to undo — send a message with a workspace checkpoint first.");
  }
  return restoreChatToTurnCheckpoint({
    currentMessages: params.currentMessages,
    input: params.input,
    mode: "after-user",
    sessionId: params.sessionId,
    turnCheckpointId: target.turn.id,
  });
}

async function reloadOpenDocumentsFromPaths(workspaceRoot: string, paths: string[]) {
  if (paths.length === 0) return;
  const normalizedTargets = new Set(paths.map((path) => normalizePathSlashes(path).toLowerCase()));
  const state = useLuxStore.getState();
  const maxBytes = 2_000_000;

  for (const document of state.openDocuments) {
    if (!document.path) continue;
    const resolved = normalizePathSlashes(resolveWorkspacePath(document.path, workspaceRoot)).toLowerCase();
    if (!isPathInsideWorkspace(resolved, workspaceRoot) || !normalizedTargets.has(resolved)) continue;
    try {
      const response = await luxCommands.fsReadText(resolved, maxBytes);
      state.replaceDocumentSnapshot({
        ...document,
        text: response.text,
        is_dirty: false,
        version: document.version + 1,
      });
    } catch (error) {
      console.warn(`Failed to reload ${resolved} after checkpoint restore: ${readErrorMessage(error)}`);
    }
  }
}