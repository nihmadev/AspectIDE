import type { AiChatMessage, AiChatMessageKind } from "./aiChatTypes";

export type GoalOrchestrationKind = "kickoff" | "continuation";

export function isVisibleChatMessage(message: AiChatMessage): boolean {
  return message.visibility !== "internal";
}

export function filterVisibleChatMessages(messages: AiChatMessage[]): AiChatMessage[] {
  return messages.filter(isVisibleChatMessage);
}

export function createInternalGoalOrchestrationMessage(
  content: string,
  orchestration: GoalOrchestrationKind,
): AiChatMessage {
  return {
    id: crypto.randomUUID(),
    role: "user",
    visibility: "internal",
    kind: "goal-orchestration" satisfies AiChatMessageKind,
    content,
    timestamp: Date.now(),
  };
}

export function isGoalOrchestrationMessage(message: AiChatMessage): boolean {
  return message.kind === "goal-orchestration" || message.visibility === "internal";
}