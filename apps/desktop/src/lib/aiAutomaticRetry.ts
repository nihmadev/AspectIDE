// Per-session backoff state for Automatic mode's "never give up" retry loop. Lives
// outside the chat store (like aiRetryNotice) so the rapid retry/clear cycle never
// churns the persisted session list. Automatic mode retries every transient failure
// indefinitely — there is no attempt cap; the count only drives the backoff curve
// and the number shown in the live retry banner. The user's Stop button (which
// aborts the turn) is the only thing that ends the loop.

import type { AiChatErrorKind } from "./aiChatErrors";
import type { AiRetryReason } from "./aiRetryNotice";

/** First retry waits this long; each subsequent retry doubles up to the ceiling. */
const BASE_DELAY_MS = 2_000;
/** Backoff ceiling — keeps a permanently-broken setup pinging calmly, not hammering. */
const MAX_DELAY_MS = 60_000;

const attemptsBySession = new Map<string, number>();

/**
 * Record the next Automatic retry for a session and return its 1-based attempt
 * number and exponential-backoff delay (capped at MAX_DELAY_MS).
 */
export function nextAutomaticRetry(sessionId: string): { attempt: number; delayMs: number } {
  const attempt = (attemptsBySession.get(sessionId) ?? 0) + 1;
  attemptsBySession.set(sessionId, attempt);
  const delayMs = Math.min(BASE_DELAY_MS * 2 ** (attempt - 1), MAX_DELAY_MS);
  return { attempt, delayMs };
}

/** Current Automatic retry streak for a session (0 when none in flight). */
export function getAutomaticRetryAttempts(sessionId: string): number {
  return attemptsBySession.get(sessionId) ?? 0;
}

/** Clear the backoff streak — called on any successful turn, cancel, or run stop. */
export function resetAutomaticRetry(sessionId: string): void {
  attemptsBySession.delete(sessionId);
}

/** Map a classified chat error to the live retry banner's reason label. */
export function automaticRetryReason(kind: AiChatErrorKind): AiRetryReason {
  switch (kind) {
    case "rate-limit":
      return "rate-limited";
    case "timeout":
      return "timeout";
    case "provider":
      return "network";
    case "stream":
      return "stream";
    case "auth":
      return "forbidden";
    default:
      return "generic";
  }
}
