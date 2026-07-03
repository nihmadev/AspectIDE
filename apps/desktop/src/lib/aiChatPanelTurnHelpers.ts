import { getAiModel, getAiProvider } from "./aiPreferences";
import type { AiChatMessage } from "./aiChatTypes";
import { appendAiUsageLogEntry } from "./aiUsageLog";
import { useLuxStore, type AiChatSessionStatus } from "./store";

/**
 * Append a completed assistant turn to the persisted usage log (model, project,
 * speed, tokens, cost). Reads provider/model/workspace fresh from the store so it
 * is closure-safe inside the turn `finally` block. Best-effort: never throws into
 * the turn lifecycle.
 */
export function recordAiUsageLogEntry(assistant: AiChatMessage | null | undefined) {
  const usage = assistant?.turnUsage;
  if (!usage) return;
  const state = useLuxStore.getState();
  const prefs = state.aiPreferences;
  const provider = getAiProvider(prefs.providers, prefs.selectedProviderId) ?? prefs.providers[0] ?? null;
  const model = getAiModel(provider, prefs.selectedModelId) ?? provider?.models[0] ?? null;
  void appendAiUsageLogEntry({
    workspaceRoot: state.workspace?.root,
    workspaceName: state.workspace?.name,
    model: model?.alias || model?.id || prefs.selectedModelId,
    provider: provider?.name ?? "",
    agentMode: prefs.agentMode,
    promptTokens: usage.promptTokens,
    completionTokens: usage.completionTokens,
    totalTokens: usage.totalTokens,
    cachedPromptTokens: usage.cachedPromptTokens,
    estimatedCostUsd: usage.estimatedCostUsd,
    durationMs: assistant?.responseTiming?.totalMs ?? assistant?.responseDurationMs ?? 0,
  });
}

export function statusToSessionStatus(
  status: "thinking" | "streaming" | "preparing" | "running-tools" | "waiting-approval",
): AiChatSessionStatus {
  return status;
}

/** Drop a trailing assistant shell that produced nothing (no text/reasoning/tools/segments). */
export function trimCancelledAssistantShell(
  sessionId: string,
  replaceMessages: (sessionId: string, messages: AiChatMessage[]) => void,
) {
  const session = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === sessionId);
  if (!session) return;
  const last = session.messages[session.messages.length - 1];
  if (last?.role !== "assistant") return;
  const hasContent = Boolean(
    last.content.trim()
    || last.reasoning?.trim()
    || (last.toolCalls?.length ?? 0) > 0
    || (last.segments?.length ?? 0) > 0,
  );
  if (!hasContent) replaceMessages(sessionId, session.messages.slice(0, -1));
}

/** Fold an error into a trailing empty assistant shell, else append it as a new bubble. */
export function replaceEmptyAssistantTail(messages: AiChatMessage[], assistantError: AiChatMessage) {
  const last = messages[messages.length - 1];
  if (
    last?.role === "assistant"
    && !last.content.trim()
    && !last.reasoning?.trim()
    && (last.toolCalls?.length ?? 0) === 0
    && (last.segments?.length ?? 0) === 0
  ) {
    return [...messages.slice(0, -1), { ...last, content: assistantError.content, timestamp: assistantError.timestamp }];
  }
  return [...messages, assistantError];
}

export function findLastUserMessageIndex(messages: AiChatMessage[]) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index].role === "user") return index;
  }
  return -1;
}

/** Real model output worth resuming from — reasoning, tool calls, or streamed
 *  text segments. A bare error bubble has none of these. */
export function messageHasAssistantWork(message: AiChatMessage) {
  return Boolean(
    (message.segments?.length ?? 0) > 0
    || (message.toolCalls?.length ?? 0) > 0
    || message.reasoning?.trim(),
  );
}

/** Drop the synthetic error bubble a failed turn appended (its content is the
 *  session's `lastError`), so a retry resumes from the AI's preserved reasoning
 *  and tool calls instead of replaying from scratch. */
export function stripTrailingErrorBubble(messages: AiChatMessage[], lastError: string | null) {
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && lastError && last.content === lastError && !messageHasAssistantWork(last)) {
    return messages.slice(0, -1);
  }
  return messages;
}

export function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

export function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
