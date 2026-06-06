import type { AiChatSession } from "./store";
import type { AiChatToolCall } from "./aiChatTypes";

export type PendingToolApprovalRef = {
  sessionId: string;
  messageId: string;
  toolCall: AiChatToolCall;
};

export function findPendingToolApprovalInSession(session: AiChatSession): PendingToolApprovalRef | null {
  for (let index = session.messages.length - 1; index >= 0; index -= 1) {
    const message = session.messages[index];
    if (!message) continue;
    const toolCalls = message.toolCalls ?? message.segments?.filter((segment) => segment.kind === "tool").map((segment) => segment.toolCall) ?? [];
    for (let toolIndex = toolCalls.length - 1; toolIndex >= 0; toolIndex -= 1) {
      const toolCall = toolCalls[toolIndex];
      if (toolCall?.status === "approval" && toolCall.approval) {
        return { sessionId: session.id, messageId: message.id, toolCall };
      }
    }
  }
  return null;
}

export function findAnyPendingToolApproval(sessions: AiChatSession[]): PendingToolApprovalRef | null {
  for (const session of sessions) {
    const pending = findPendingToolApprovalInSession(session);
    if (pending) return pending;
  }
  return null;
}