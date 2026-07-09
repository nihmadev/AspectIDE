import { getTurnCheckpoint, listTurnCheckpoints } from "./checkpoint-store";
import { findTurnCheckpointForUserMessage } from "./turn-checkpoints";
import { restoreFileCheckpointById } from "./../runtime/checkpoints";
import { normalizePathSlashes, readErrorMessage } from "./../runtime/shared";
import { isPathInsideWorkspace, resolveWorkspacePath } from "./../runtime/file-context";
import { setAiSessionGoal } from "./../session/goal/session-goal";
import { hydrateAiSessionTodos, type AiSessionTodo } from "./../session/todos";
import type { AiChatMessage, AiChatSendInput } from "./types";
import { isTauriRuntime, luxCommands } from "./../../tauri/commands";
import { useLuxStore } from "./../../store/index";
import { loadTurnCheckpointMessages, pruneTurnCheckpointsFromIndex } from "./turn-checkpoints";

export type RestoreChatTurnMode = "before-user" | "after-user";

export type RestoreChatTurnResult = {
  messages: AiChatMessage[];
  restoredFileCount: number;
  removedTurnCheckpoints: number;
  /** Files actually captured in the file checkpoint (0 = no snapshot ever existed
   *  for this turn — distinct from "captured but already matched disk"). Lets the
   *  caller show an honest toast instead of a blanket "Restored 0 files" success. */
  snapshotFileCount: number;
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
  restoreTurnSessionState(params.sessionId, turn.sessionGoal ?? "", turn.sessionTodos ?? []);

  const allTurns = listTurnCheckpoints(params.sessionId);
  const checkpointIndex = allTurns.findIndex((entry) => entry.id === turn.id);
  const removedTurnCheckpoints = checkpointIndex >= 0 ? allTurns.length - checkpointIndex : 0;
  if (checkpointIndex >= 0) pruneTurnCheckpointsFromIndex(params.sessionId, checkpointIndex);

  return {
    messages: nextMessages,
    restoredFileCount: restore.restoredPaths.length,
    removedTurnCheckpoints,
    snapshotFileCount: restore.snapshotFileCount,
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

/**
 * Roll back the session's orchestration state (pinned goal + task list) to what
 * the checkpoint captured. Without this a restore rewound messages and files but
 * left the goal rail and TodoWrite tasks from the rolled-back turn in place.
 * The native (Rust) session store is mirrored too, otherwise the next turn's
 * FastContext would re-inject the stale goal/tasks from Rust.
 */
function restoreTurnSessionState(sessionId: string, goal: string, todos: unknown[]) {
  const normalizedTodos = todos.filter(isPersistedTodo).map((todo) => ({ ...todo }));
  setAiSessionGoal(sessionId, goal);
  hydrateAiSessionTodos(sessionId, normalizedTodos);
  if (isTauriRuntime()) {
    void luxCommands.aiSessionGoalSet(sessionId, goal).catch(() => undefined);
    void luxCommands
      .aiSessionTodosSet(sessionId, normalizedTodos.map((todo) => ({
        id: todo.id,
        content: todo.content,
        status: todo.status,
        priority: todo.priority,
        notes: todo.notes,
      })))
      .catch(() => undefined);
  }
}

function isPersistedTodo(value: unknown): value is AiSessionTodo {
  if (!value || typeof value !== "object") return false;
  const todo = value as Partial<AiSessionTodo>;
  return typeof todo.id === "string" && typeof todo.content === "string" && typeof todo.status === "string";
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