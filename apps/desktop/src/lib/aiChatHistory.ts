import type { AiChatMessage, AiChatMessageAttachment } from "./aiChatTypes";
import { hydrateAllAiSessionGoals, subscribeAiSessionGoals } from "./aiSessionGoal";
import { hydrateAllAiSessionTodos, subscribeAiSessionTodos } from "./aiSessionTodos";
import { hydratePendingFileReviews, subscribePendingFileReviews } from "./aiPendingFileReview";
import { attachAllSessionExtrasForPersist } from "./aiChatSessionExtras";
import type { AiChatSession, AiChatSessionState } from "./store";
import { type AiChatHistoryResponse, isBrowserPreviewRuntime, isTauriRuntime, luxCommands } from "./tauri";

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

// Cheap change-signal so the 450ms persist tick can skip serializing+IPC-ing the
// entire session list when nothing meaningful changed. `updatedAt` is bumped on every
// session mutation (see store.ts), so a digest of (id, updatedAt, message count,
// closedAt) + the active id captures all persistable session changes.
let lastPersistDigest: string | null = null;

// Extras (goal / todos / pending reviews) live in their own stores and persist by
// riding along on the session save — but they mutate WITHOUT bumping session.updatedAt.
// A monotonic revision, bumped on every extras-store emit, folds into the digest so an
// idle extras change still triggers a write instead of waiting for the next session edit.
// Stores never import this module, so the subscriptions add no import cycle.
let extrasRevision = 0;
let extrasSubscribed = false;

function ensureExtrasSubscription() {
  if (extrasSubscribed) return;
  extrasSubscribed = true;
  const bump = () => { extrasRevision += 1; };
  subscribeAiSessionGoals(bump);
  subscribeAiSessionTodos(bump);
  subscribePendingFileReviews(bump);
}

function computePersistDigest(state: AiChatSessionState): string {
  ensureExtrasSubscription();
  const parts = state.sessions.map(
    (session) => `${session.id}:${session.updatedAt}:${session.messages.length}:${session.closedAt ?? 0}`,
  );
  return `${state.activeSessionId}|${extrasRevision}|${parts.join(",")}`;
}

/** Reset the persist digest so the next save always writes (call after load/import). */
export function resetAiChatPersistDigest() {
  lastPersistDigest = null;
}

export async function loadAiChatHistory(): Promise<AiChatSessionState | null> {
  if (isBrowserPreviewRuntime()) return loadLegacyAiChatHistory();

  let response: AiChatHistoryResponse;
  try {
    response = await luxCommands.aiChatHistoryLoad();
  } catch (error) {
    const legacy = isTauriRuntime() ? loadLegacyAiChatHistory() : null;
    if (legacy && legacy.sessions.length > 0) {
      await migrateLegacyAiChatHistory(legacy);
      return legacy;
    }
    throw error;
  }
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

export async function saveAiChatHistory(state: AiChatSessionState, options: { force?: boolean } = {}): Promise<void> {
  // Skip the whole serialize+compact+IPC pass when the session signal is unchanged
  // since the last successful save. The 450ms debounce fires on unrelated store
  // churn too, so this turns most ticks into a cheap digest comparison.
  const digest = computePersistDigest(state);
  if (!options.force && digest === lastPersistDigest) return;

  const compacted = compactAiChatHistory({
    ...state,
    sessions: attachAllSessionExtrasForPersist(state.sessions),
  });
  if (isBrowserPreviewRuntime()) {
    try {
      window.localStorage.setItem(legacyAiChatSessionsStorageKey, JSON.stringify(compacted));
      lastPersistDigest = digest;
    } catch {
      // Likely QuotaExceededError: retry once with image previews dropped so textual history still persists.
      const stripped: AiChatSessionState = {
        ...compacted,
        sessions: compacted.sessions.map(stripSessionImagePreviews),
      };
      try {
        window.localStorage.setItem(legacyAiChatSessionsStorageKey, JSON.stringify(stripped));
        lastPersistDigest = digest;
      } catch (retryError) {
        console.warn("Failed to persist AI chat history to localStorage", retryError);
      }
    }
    return;
  }
  await luxCommands.aiChatHistorySave(compacted);
  // Only record the digest after a successful write so a thrown IPC error leaves the
  // digest stale and the next tick retries instead of silently dropping the change.
  lastPersistDigest = digest;
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

function stripSessionImagePreviews(session: AiChatSession): AiChatSession {
  return {
    ...session,
    messages: session.messages.map((message) =>
      message.attachments?.some((attachment) => attachment.kind === "image" && attachment.previewUrl)
        ? {
          ...message,
          attachments: message.attachments.map((attachment) =>
            attachment.kind === "image" && attachment.previewUrl
              ? { ...attachment, previewUrl: undefined }
              : attachment,
          ),
        }
        : message,
    ),
  };
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
