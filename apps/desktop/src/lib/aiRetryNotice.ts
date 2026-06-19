// Live "the turn is auto-retrying a transient failure" state, kept per session
// outside the chat store so a high-frequency retry/clear cycle never churns the
// persisted session list. Mirrors the aiTurnActivity store shape: a tiny
// external store the chat surfaces subscribe to via useSyncExternalStore.

export type AiRetryReason =
  | "rate-limited"
  | "server"
  | "forbidden"
  | "timeout"
  | "network"
  | "stream"
  | "generic";

export type AiRetryNotice = {
  /** 1-based number of the upcoming attempt (the first try is attempt 1). */
  attempt: number;
  /** Total attempts that will be made before giving up. */
  maxAttempts: number;
  /** Stable machine reason used to pick a localized label. */
  reason: AiRetryReason;
  /** Short human detail (e.g. "HTTP 429"). */
  detail: string;
  /** Backoff delay before the retry, in ms (0 when unknown). */
  delayMs: number;
  /** When the notice was recorded, for "x ago"/age-based pruning if needed. */
  updatedAt: number;
};

const noticeBySession = new Map<string, AiRetryNotice>();
const listeners = new Set<() => void>();
// Monotonic revision so a value-less getSnapshot can detect any change cheaply.
let revision = 0;

function emit() {
  revision += 1;
  listeners.forEach((listener) => listener());
}

export function subscribeAiRetryNotice(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Cheap change signal for useSyncExternalStore (server snapshot included). */
export function getAiRetryNoticeRevision() {
  return revision;
}

export function getAiRetryNotice(sessionId: string): AiRetryNotice | null {
  return noticeBySession.get(sessionId) ?? null;
}

const KNOWN_REASONS = new Set<AiRetryReason>([
  "rate-limited",
  "server",
  "forbidden",
  "timeout",
  "network",
  "stream",
  "generic",
]);

function normalizeReason(reason: string): AiRetryReason {
  return KNOWN_REASONS.has(reason as AiRetryReason) ? (reason as AiRetryReason) : "generic";
}

/** Record (or update) the active retry notice for a session. */
export function setAiRetryNotice(
  sessionId: string,
  notice: { attempt: number; maxAttempts: number; reason: string; detail: string; delayMs: number },
) {
  noticeBySession.set(sessionId, {
    attempt: notice.attempt,
    maxAttempts: notice.maxAttempts,
    reason: normalizeReason(notice.reason),
    detail: notice.detail,
    delayMs: notice.delayMs,
    updatedAt: Date.now(),
  });
  emit();
}

/** Drop the retry notice once the turn recovers, ends, or is cancelled. */
export function clearAiRetryNotice(sessionId: string) {
  if (!noticeBySession.delete(sessionId)) return;
  emit();
}
