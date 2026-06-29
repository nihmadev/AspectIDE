import { isPollutedSessionGoal, sanitizeSessionGoal } from "./aiSessionOrchestrationSanitize";

const goalBySession = new Map<string, string>();
const listeners = new Set<() => void>();

// Monotonic revision counter — incremented on every mutation so useSyncExternalStore
// subscribers re-render even when the Map size stays the same (e.g., goal text change
// within the same session, or a sanitize/replace cycle that keeps size == 1).
let goalsRevision = 0;

export function subscribeAiSessionGoals(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Returns the monotonic revision so useSyncExternalStore detects every mutation. */
export function getAiSessionGoalsSnapshot() {
  return goalsRevision;
}

export function getAiSessionGoal(sessionId: string) {
  const stored = goalBySession.get(sessionId)?.trim() ?? "";
  if (!stored || !isPollutedSessionGoal(stored)) return stored;
  const sanitized = sanitizeSessionGoal(stored);
  if (sanitized.ok) {
    goalBySession.set(sessionId, sanitized.value);
    return sanitized.value;
  }
  goalBySession.delete(sessionId);
  emitGoals();
  return "";
}

function emitGoals() {
  goalsRevision += 1;
  listeners.forEach((listener) => listener());
}

export function setAiSessionGoal(sessionId: string, goal: string) {
  const trimmed = goal.trim();
  if (!trimmed) {
    goalBySession.delete(sessionId);
  } else {
    goalBySession.set(sessionId, trimmed);
  }
  emitGoals();
}

export function clearAiSessionGoal(sessionId: string) {
  goalBySession.delete(sessionId);
  emitGoals();
}

export function hydrateAiSessionGoal(sessionId: string, goal: string | undefined) {
  const trimmed = goal?.trim();
  if (trimmed) goalBySession.set(sessionId, trimmed);
}

export function hydrateAllAiSessionGoals(sessions: Array<{ id: string; sessionGoal?: string }>) {
  goalBySession.clear();
  for (const session of sessions) {
    hydrateAiSessionGoal(session.id, session.sessionGoal);
  }
  emitGoals();
}