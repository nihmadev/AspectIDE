export type AiTurnActivityPhase =
  | "idle"
  | "thinking"
  | "streaming"
  | "running-tools"
  | "waiting-approval"
  | "subagent";

export type AiTurnActivity = {
  phase: AiTurnActivityPhase;
  toolName: string | null;
  filePath: string | null;
  subagentLabel: string | null;
  updatedAt: number;
};

const activityBySession = new Map<string, AiTurnActivity>();
const listeners = new Set<() => void>();

const idleActivity = (): AiTurnActivity => ({
  phase: "idle",
  toolName: null,
  filePath: null,
  subagentLabel: null,
  updatedAt: Date.now(),
});

export function subscribeAiTurnActivity(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getAiTurnActivitySnapshot() {
  return activityBySession.size;
}

export function getAiTurnActivity(sessionId: string): AiTurnActivity {
  return activityBySession.get(sessionId) ?? idleActivity();
}

export function setAiTurnActivity(sessionId: string, patch: Partial<AiTurnActivity>) {
  const current = activityBySession.get(sessionId) ?? idleActivity();
  activityBySession.set(sessionId, {
    ...current,
    ...patch,
    updatedAt: Date.now(),
  });
  listeners.forEach((listener) => listener());
}

export function clearAiTurnActivity(sessionId: string) {
  if (!activityBySession.has(sessionId)) return;
  activityBySession.set(sessionId, idleActivity());
  listeners.forEach((listener) => listener());
}

export function deleteAiTurnActivity(sessionId: string) {
  if (!activityBySession.delete(sessionId)) return;
  listeners.forEach((listener) => listener());
}

export function extractToolPath(toolName: string, input: string | undefined) {
  if (!input) return null;
  try {
    const parsed = JSON.parse(input) as Record<string, unknown>;
    const keys = ["path", "file", "filePath", "target", "uri"];
    for (const key of keys) {
      const value = parsed[key];
      if (typeof value === "string" && value.trim()) return value.trim();
    }
  } catch {
    const match = input.match(/["']?(?:path|file)["']?\s*[:=]\s*["']([^"']+)["']/i);
    if (match?.[1]) return match[1];
  }
  return null;
}