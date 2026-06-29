/**
 * Tracks the `turn_id` of the in-flight native turn for each chat session.
 *
 * The Rust inject command (`ai_inject_message`) is scoped by `session_id` +
 * `turn_id` so a restarted or concurrent turn cannot drain input that was staged
 * for a different (earlier) turn (F5 — misrouting live input between turns). The
 * UI stages mid-work messages from an effect that only knows the *session*, so it
 * needs to resolve the live `turn_id` here.
 *
 * In-memory only: a `turn_id` is meaningless across a reload — the turn it named
 * is no longer running — so there is nothing to persist.
 */
const activeTurnBySession = new Map<string, string>();

/** Record the turn that is now running for a session (replaces any prior entry). */
export function registerActiveTurn(sessionId: string, turnId: string) {
  activeTurnBySession.set(sessionId, turnId);
}

/**
 * Clear the active turn for a session, but ONLY if it still points at `turnId`.
 * A late settle from an older turn must not erase a newer turn that has already
 * registered for the same session (avoids a teardown race wiping live state).
 */
export function clearActiveTurn(sessionId: string, turnId: string) {
  if (activeTurnBySession.get(sessionId) === turnId) {
    activeTurnBySession.delete(sessionId);
  }
}

/** The `turn_id` currently running for a session, or `undefined` if none. */
export function getActiveTurnId(sessionId: string): string | undefined {
  return activeTurnBySession.get(sessionId);
}
