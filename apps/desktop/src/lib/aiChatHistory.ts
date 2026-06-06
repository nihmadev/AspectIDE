import type { AiChatMessage, AiChatMessageAttachment } from "./aiChatTypes";
import { hydrateAllAiSessionGoals } from "./aiSessionGoal";
import { hydrateAllAiSessionTodos } from "./aiSessionTodos";
import { hydratePendingFileReviews } from "./aiPendingFileReview";
import { attachAllSessionExtrasForPersist } from "./aiChatSessionExtras";
import type { AiChatSession, AiChatSessionState } from "./store";
import { isBrowserPreviewRuntime, isTauriRuntime, luxCommands } from "./tauri";

export const legacyAiChatSessionsStorageKey = "ai.chat.sessions.v1";

const historySchemaVersion = 1;
const maxPersistedSessions = 80;
const maxPersistedMessagesPerSession = 240;
const maxPersistedTextChars = 80_000;
const maxPersistedToolTextChars = 24_000;
const maxPersistedImagePreviewChars = 1_200_000;

type PersistedChatState = {
  activeSessionId: string;
  sessions: AiChatSession[];
};

export async function loadAiChatHistory(): Promise<AiChatSessionState | null> {
  if (isBrowserPreviewRuntime()) return loadLegacyAiChatHistory();

  const response = await luxCommands.aiChatHistoryLoad();
  if (response.schemaVersion !== historySchemaVersion || !Array.isArray(response.sessions) || response.sessions.length === 0) {
    const legacy = isTauriRuntime() ? loadLegacyAiChatHistory() : null;
    if (legacy && legacy.sessions.length > 0) await migrateLegacyAiChatHistory(legacy);
    return legacy;
  }
  const sessions = response.sessions.filter(isAiChatSession);
  hydrateChatSessionExtras(sessions);
  return {
    activeSessionId: response.activeSessionId,
    sessions,
  };
}

export async function saveAiChatHistory(state: AiChatSessionState): Promise<void> {
  const compacted = compactAiChatHistory({
    ...state,
    sessions: attachAllSessionExtrasForPersist(state.sessions),
  });
  if (isBrowserPreviewRuntime()) {
    window.localStorage.setItem(legacyAiChatSessionsStorageKey, JSON.stringify(compacted));
    return;
  }
  await luxCommands.aiChatHistorySave(compacted);
  if (isTauriRuntime()) removeLegacyAiChatHistory();
}

export async function migrateLegacyAiChatHistory(state: AiChatSessionState): Promise<void> {
  if (!isTauriRuntime()) return;
  await luxCommands.aiChatHistorySave(compactAiChatHistory(state));
  removeLegacyAiChatHistory();
}

export function loadLegacyAiChatHistory(): AiChatSessionState | null {
  const raw = window.localStorage.getItem(legacyAiChatSessionsStorageKey);
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as PersistedChatState;
    if (!parsed || !Array.isArray(parsed.sessions)) return null;
    const sessions = parsed.sessions.filter(isAiChatSession);
    hydrateChatSessionExtras(sessions);
    return {
      activeSessionId: typeof parsed.activeSessionId === "string" ? parsed.activeSessionId : "",
      sessions,
    };
  } catch {
    removeLegacyAiChatHistory();
    return null;
  }
}

function removeLegacyAiChatHistory() {
  window.localStorage.removeItem(legacyAiChatSessionsStorageKey);
}

function compactAiChatHistory(state: AiChatSessionState): AiChatSessionState {
  const sessions = state.sessions
    .slice()
    .sort(comparePersistedSessions)
    .slice(0, maxPersistedSessions)
    .map(compactSession);
  const activeSessionId = sessions.some((session) => session.id === state.activeSessionId)
    ? state.activeSessionId
    : sessions.find((session) => !session.closedAt)?.id ?? sessions[0]?.id ?? "";
  return { activeSessionId, sessions };
}

function compactSession(session: AiChatSession): AiChatSession {
  return {
    ...session,
    lastError: session.lastError ? truncate(session.lastError, 2_000) : null,
    messages: session.messages.slice(-maxPersistedMessagesPerSession).map(compactMessage),
    sessionTodos: session.sessionTodos?.slice(0, 48),
    sessionGoal: session.sessionGoal ? truncate(session.sessionGoal, 2_000) : undefined,
    pendingFileReviews: session.pendingFileReviews?.slice(0, 24).map((review) => ({
      ...review,
      beforeText: truncate(review.beforeText, 8_000),
      afterText: truncate(review.afterText, 8_000),
    })),
    contextBudgetReport: session.contextBudgetReport
      ? {
        ...session.contextBudgetReport,
        dropped: session.contextBudgetReport.dropped.slice(0, 32),
      }
      : session.contextBudgetReport,
  };
}

function hydrateChatSessionExtras(sessions: AiChatSession[]) {
  hydrateAllAiSessionGoals(sessions);
  hydrateAllAiSessionTodos(sessions);
  const reviews = sessions.flatMap((session) => session.pendingFileReviews ?? []);
  if (reviews.length > 0) hydratePendingFileReviews(reviews);
}

function compactMessage(message: AiChatMessage): AiChatMessage {
  const segments = message.segments?.map((segment) => {
    if (segment.kind === "tool") {
      return {
        ...segment,
        toolCall: {
          ...segment.toolCall,
          input: truncateOptional(segment.toolCall.input, 2_000),
          output: truncateOptional(segment.toolCall.output, maxPersistedToolTextChars),
          error: truncateOptional(segment.toolCall.error, 4_000),
          approval: segment.toolCall.approval ? {
            ...segment.toolCall.approval,
            summary: truncate(segment.toolCall.approval.summary, 4_000),
            preview: truncate(segment.toolCall.approval.preview, 12_000),
          } : undefined,
        },
      };
    }
    return { ...segment, text: truncate(segment.text, maxPersistedTextChars) };
  });
  return {
    ...message,
    content: truncate(message.content, maxPersistedTextChars),
    attachments: message.attachments?.map(compactMessageAttachment).filter((entry): entry is AiChatMessageAttachment => Boolean(entry)),
    reasoning: truncateOptional(message.reasoning, maxPersistedTextChars),
    toolCalls: message.toolCalls?.map((toolCall) => ({
      ...toolCall,
      input: truncateOptional(toolCall.input, 2_000),
      output: truncateOptional(toolCall.output, maxPersistedToolTextChars),
      error: truncateOptional(toolCall.error, 4_000),
    })),
    segments,
  };
}

function comparePersistedSessions(left: AiChatSession, right: AiChatSession) {
  if (!left.closedAt && right.closedAt) return -1;
  if (left.closedAt && !right.closedAt) return 1;
  return right.updatedAt - left.updatedAt;
}

function truncateOptional(value: string | undefined, maxChars: number) {
  return typeof value === "string" ? truncate(value, maxChars) : value;
}

function truncate(value: string, maxChars: number) {
  if (value.length <= maxChars) return value;
  return `${value.slice(0, maxChars)}\n...[truncated ${value.length - maxChars} chars]`;
}

function compactMessageAttachment(attachment: AiChatMessageAttachment): AiChatMessageAttachment | null {
  if (attachment.kind === "image" && attachment.previewUrl && attachment.previewUrl.length > maxPersistedImagePreviewChars) {
    return { ...attachment, previewUrl: undefined };
  }
  return attachment;
}

function isAiChatSession(value: unknown): value is AiChatSession {
  if (!value || typeof value !== "object") return false;
  const session = value as Partial<AiChatSession>;
  return typeof session.id === "string" && Array.isArray(session.messages);
}
