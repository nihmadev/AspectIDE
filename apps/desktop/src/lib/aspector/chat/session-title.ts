export const DEFAULT_CHAT_SESSION_TITLE = "New chat";
export const MAX_CHAT_SESSION_TITLE_CHARS = 42;

export function isDefaultChatSessionTitle(title: string) {
  return title.trim() === DEFAULT_CHAT_SESSION_TITLE;
}

export function normalizeChatSessionTitle(value: string) {
  const normalized = value.replace(/\s+/g, " ").trim();
  if (!normalized) return DEFAULT_CHAT_SESSION_TITLE;
  const stripped = normalized.replace(/^["'`]+|["'`]+$/g, "").replace(/[.!?]+$/g, "").trim();
  if (!stripped) return DEFAULT_CHAT_SESSION_TITLE;
  return stripped.length > MAX_CHAT_SESSION_TITLE_CHARS
    ? `${stripped.slice(0, MAX_CHAT_SESSION_TITLE_CHARS).trimEnd()}...`
    : stripped;
}

export function heuristicChatSessionTitle(firstUserMessage: string) {
  return normalizeChatSessionTitle(firstUserMessage);
}

// The chat session title is simply the user's first message (normalized/
// truncated). We intentionally do NOT call a model to invent a title: it cost an
// extra request per chat, leaked to whatever provider was active, and the first
// message is already the clearest label.
export function generateChatSessionTitle(input: { firstUserMessage: string }): string {
  return heuristicChatSessionTitle(input.firstUserMessage);
}

export function shouldRefreshChatSessionTitle(session: {
  title: string;
  messages: Array<{ role: string; visibility?: string; content: string }>;
}) {
  const visibleUserMessages = session.messages.filter(
    (message) => message.role === "user" && message.visibility !== "internal" && message.content.trim(),
  );
  if (visibleUserMessages.length !== 1) return false;
  const heuristic = heuristicChatSessionTitle(visibleUserMessages[0].content);
  return isDefaultChatSessionTitle(session.title) || session.title === heuristic;
}

export function scheduleChatSessionTitleRefresh(params: {
  sessionId: string;
  firstUserMessage: string;
  rename: (sessionId: string, title: string) => void;
  readSession: (sessionId: string) => { title: string; messages: Array<{ role: string; visibility?: string; content: string }> } | null;
}) {
  const trimmed = params.firstUserMessage.trim();
  if (!trimmed) return;

  // Title is derived synchronously from the first message — no request, no race.
  const session = params.readSession(params.sessionId);
  if (!session || !shouldRefreshChatSessionTitle(session)) return;
  const title = generateChatSessionTitle({ firstUserMessage: trimmed });
  if (title === session.title) return;
  params.rename(params.sessionId, title);
}

// Retained as a no-op so existing teardown call sites keep working now that title
// generation is synchronous (nothing in flight to cancel).
export function cancelChatSessionTitleRefresh(_sessionId: string) {}