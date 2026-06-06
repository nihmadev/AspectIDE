import { isPollutedSessionGoal, sanitizeSessionGoal } from "./aiSessionOrchestrationSanitize";

const goalBySession = new Map<string, string>();
const listeners = new Set<() => void>();

export function subscribeAiSessionGoals(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getAiSessionGoalsSnapshot() {
  return goalBySession.size;
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
  listeners.forEach((listener) => listener());
  return "";
}

export function setAiSessionGoal(sessionId: string, goal: string) {
  const trimmed = goal.trim();
  if (!trimmed) {
    goalBySession.delete(sessionId);
  } else {
    goalBySession.set(sessionId, trimmed);
  }
  listeners.forEach((listener) => listener());
}

export function clearAiSessionGoal(sessionId: string) {
  goalBySession.delete(sessionId);
  listeners.forEach((listener) => listener());
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
  listeners.forEach((listener) => listener());
}