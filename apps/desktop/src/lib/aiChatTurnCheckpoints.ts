import {
  appendTurnCheckpointChangedPaths,
  getTurnCheckpoint,
  listTurnCheckpoints,
  removeTurnCheckpointsAtAndAfter,
  upsertTurnCheckpoint,
  type PersistedTurnCheckpoint,
} from "./aiChatCheckpointStore";
import { createFileCheckpointForTurn } from "./aiRuntimeCheckpoints";
import type { AiChatMessage, AiChatSendInput } from "./aiChatTypes";

export type TurnCheckpointSummary = {
  id: string;
  userMessageId: string;
  label: string;
  createdAt: number;
  messageCount: number;
  fileCheckpointId: string;
  changedPaths: string[];
};

export async function createTurnCheckpointBeforeSend(params: {
  input: AiChatSendInput;
  label: string;
  messages: AiChatMessage[];
  sessionId: string;
  userMessageId: string;
}): Promise<TurnCheckpointSummary> {
  const fileCheckpoint = await createFileCheckpointForTurn(params.input, params.label);
  const turn: PersistedTurnCheckpoint = {
    id: `turn-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 8)}`,
    sessionId: params.sessionId,
    userMessageId: params.userMessageId,
    label: params.label,
    createdAt: Date.now(),
    messageCount: params.messages.length,
    fileCheckpointId: fileCheckpoint.id,
    changedPaths: [],
    messages: params.messages.map(cloneMessageForCheckpoint),
  };
  upsertTurnCheckpoint(turn);
  return summarizeTurnCheckpoint(turn);
}

export function findTurnCheckpointForUserMessage(sessionId: string, userMessageId: string) {
  return listTurnCheckpoints(sessionId).find((entry) => entry.userMessageId === userMessageId) ?? null;
}

export function hasUserTurnCheckpoint(sessionId: string, userMessageId: string) {
  return findTurnCheckpointForUserMessage(sessionId, userMessageId) !== null;
}

/** Re-link persisted messages to local turn checkpoints (e.g. after reload in Agent mode). */
export function repairMessageTurnCheckpointIds(sessionId: string, messages: AiChatMessage[]): AiChatMessage[] {
  let changed = false;
  const next = messages.map((message) => {
    if (message.role !== "user" || message.turnCheckpointId) return message;
    const turn = findTurnCheckpointForUserMessage(sessionId, message.id);
    if (!turn) return message;
    changed = true;
    return { ...message, turnCheckpointId: turn.id };
  });
  return changed ? next : messages;
}

export function findTurnCheckpointForMessageIndex(sessionId: string, messageIndex: number, messages: AiChatMessage[]) {
  const userMessage = messages[messageIndex];
  if (!userMessage || userMessage.role !== "user") return null;
  return findTurnCheckpointForUserMessage(sessionId, userMessage.id);
}

export function listTurnCheckpointSummaries(sessionId: string) {
  return listTurnCheckpoints(sessionId).map(summarizeTurnCheckpoint);
}

export function loadTurnCheckpointMessages(sessionId: string, turnCheckpointId: string): AiChatMessage[] {
  const turn = getTurnCheckpoint(sessionId, turnCheckpointId);
  if (!turn) throw new Error("Turn checkpoint not found.");
  return turn.messages.filter(isAiChatMessage);
}

export function pruneTurnCheckpointsFromIndex(sessionId: string, fromIndex: number) {
  removeTurnCheckpointsAtAndAfter(sessionId, fromIndex);
}

export function recordTurnCheckpointFileChanges(params: {
  sessionId: string;
  turnCheckpointId: string;
  paths: string[];
}) {
  appendTurnCheckpointChangedPaths(params.sessionId, params.turnCheckpointId, params.paths);
}

function summarizeTurnCheckpoint(turn: PersistedTurnCheckpoint): TurnCheckpointSummary {
  return {
    id: turn.id,
    userMessageId: turn.userMessageId,
    label: turn.label,
    createdAt: turn.createdAt,
    messageCount: turn.messageCount,
    fileCheckpointId: turn.fileCheckpointId,
    changedPaths: turn.changedPaths,
  };
}

function cloneMessageForCheckpoint(message: AiChatMessage): AiChatMessage {
  return {
    ...message,
    attachments: message.attachments?.map((attachment) => ({ ...attachment })),
    toolCalls: message.toolCalls?.map((toolCall) => ({ ...toolCall })),
    segments: message.segments?.map((segment) => (
      segment.kind === "tool"
        ? { ...segment, toolCall: { ...segment.toolCall } }
        : { ...segment }
    )),
  };
}

function isAiChatMessage(value: unknown): value is AiChatMessage {
  if (!value || typeof value !== "object") return false;
  const message = value as Partial<AiChatMessage>;
  return typeof message.id === "string" && (message.role === "user" || message.role === "assistant") && typeof message.content === "string";
}