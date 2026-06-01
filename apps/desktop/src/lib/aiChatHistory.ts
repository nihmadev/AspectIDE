import type { AiChatMessage } from "./aiChatTypes";
import type { AiChatSession, AiChatSessionState } from "./store";
import { isTauriRuntime, luxCommands } from "./tauri";

export const legacyAiChatSessionsStorageKey = "ai.chat.sessions.v1";

const historySchemaVersion = 1;
const maxPersistedSessions = 80;
const maxPersistedMessagesPerSession = 240;
const maxPersistedTextChars = 80_000;
const maxPersistedToolTextChars = 24_000;

type PersistedChatState = {
  activeSessionId: string;
  sessions: AiChatSession[];
};

export async function loadAiChatHistory(): Promise<AiChatSessionState | null> {
  if (!isTauriRuntime()) return loadLegacyAiChatHistory();
  try {
    const response = await luxCommands.aiChatHistoryLoad();
    if (response.schemaVersion !== historySchemaVersion || !Array.isArray(response.sessions) || response.sessions.length === 0) {
      const legacy = loadLegacyAiChatHistory();
      if (legacy && legacy.sessions.length > 0) await migrateLegacyAiChatHistory(legacy);
      return legacy;
    }
    return {
      activeSessionId: response.activeSessionId,
      sessions: response.sessions.filter(isAiChatSession),
    };
  } catch {
    return loadLegacyAiChatHistory();
  }
}

export async function saveAiChatHistory(state: AiChatSessionState): Promise<void> {
  const compacted = compactAiChatHistory(state);
  if (isTauriRuntime()) {
    await luxCommands.aiChatHistorySave(compacted);
    removeLegacyAiChatHistory();
    return;
  }
  window.localStorage.setItem(legacyAiChatSessionsStorageKey, JSON.stringify(compacted));
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
    return {
      activeSessionId: typeof parsed.activeSessionId === "string" ? parsed.activeSessionId : "",
      sessions: parsed.sessions.filter(isAiChatSession),
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
  };
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

function isAiChatSession(value: unknown): value is AiChatSession {
  if (!value || typeof value !== "object") return false;
  const session = value as Partial<AiChatSession>;
  return typeof session.id === "string" && Array.isArray(session.messages);
}
