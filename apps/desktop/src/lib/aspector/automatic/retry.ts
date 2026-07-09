// Per-session backoff state for automatic retry loop. Lives outside the
// chat store (like aiRetryNotice) so rapid retry/clear cycles never churn the
// persisted session list. The user-facing budget is intentionally finite and
// predictable: 3s, 6s, 9s, ... up to 10 attempts. Goal orchestration may still
// continue after a successful retry; Stop is the manual escape hatch.

import type { AiChatErrorKind } from "./../chat/errors";
import type { AiRetryReason } from "./../utils/retry-notice";

/** Linear retry ladder requested by users: 3, 6, 9, ... seconds. */
const RETRY_STEP_MS = 3_000;
/** User-visible retry budget for transient provider/network failures. */
export const AUTOMATIC_RETRY_MAX_ATTEMPTS = 10;
/** Calm ceiling for broken local endpoints while preserving the linear ladder. */
const MAX_DELAY_MS = RETRY_STEP_MS * AUTOMATIC_RETRY_MAX_ATTEMPTS;

const attemptsBySession = new Map<string, number>();

export type AutomaticRetryPlan = {
  /** 1-based number shown to the user. */
  attempt: number;
  /** Total retry budget shown to the user. */
  maxAttempts: number;
  /** Linear backoff delay before the next attempt. */
  delayMs: number;
  /** True after the budget is consumed. Callers should surface the failure instead of scheduling another retry. */
  exhausted: boolean;
};

/**
 * Record the next retry for a session and return its 1-based attempt, max-attempt
 * budget and linear-backoff delay.
 */
export function nextAutomaticRetry(sessionId: string): AutomaticRetryPlan {
  const rawAttempt = (attemptsBySession.get(sessionId) ?? 0) + 1;
  attemptsBySession.set(sessionId, rawAttempt);
  const attempt = Math.min(rawAttempt, AUTOMATIC_RETRY_MAX_ATTEMPTS);
  const delayMs = Math.min(RETRY_STEP_MS * attempt, MAX_DELAY_MS);
  return {
    attempt,
    maxAttempts: AUTOMATIC_RETRY_MAX_ATTEMPTS,
    delayMs,
    exhausted: rawAttempt > AUTOMATIC_RETRY_MAX_ATTEMPTS,
  };
}

/** Current retry streak for a session (0 when none in flight). */
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

/**
 * Transient transport failures that genuinely benefit from a staggered retry in
 * ANY mode (not just Automatic): the provider dropped, the stream was interrupted,
 * the request timed out, or we hit a rate limit. These recover on their own with
 * backoff. Auth/tool/workspace errors are excluded — retrying won't help until the
 * user fixes something, so manual/plan modes surface them immediately instead.
 */
export function isTransientRetryKind(kind: AiChatErrorKind): boolean {
  return kind === "rate-limit"
    || kind === "timeout"
    || kind === "provider"
    || kind === "stream";
}
