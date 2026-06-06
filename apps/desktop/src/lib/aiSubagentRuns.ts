import type { SubagentKind } from "./aiSubagents";

import { defaultMaxParallelSubagents } from "./aiSubagentPolicy";

/** @deprecated Use resolveMaxParallelSubagents(preferences) */
export const MAX_PARALLEL_SUBAGENTS = defaultMaxParallelSubagents;

export type SubagentRunStatus = "running" | "completed" | "failed" | "cancelled";

export type SubagentTranscriptEntry = {
  id: string;
  role: "assistant" | "system";
  content: string;
  at: number;
};

export type SubagentRun = {
  id: string;
  sessionId: string;
  description: string;
  subagentType: SubagentKind;
  status: SubagentRunStatus;
  depth: number;
  parentAgentId: string | null;
  startedAt: number;
  endedAt: number | null;
  summary: string;
  transcript: SubagentTranscriptEntry[];
};

type SubagentListener = () => void;

const runs = new Map<string, SubagentRun>();
const abortControllers = new Map<string, AbortController>();
const listeners = new Set<SubagentListener>();

export function subscribeSubagentRuns(listener: SubagentListener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function listSubagentRunsForSession(sessionId: string) {
  return [...runs.values()]
    .filter((run) => run.sessionId === sessionId)
    .sort((left, right) => right.startedAt - left.startedAt);
}

export function countRunningSubagents(sessionId: string) {
  return listSubagentRunsForSession(sessionId).filter((run) => run.status === "running").length;
}

export function registerSubagentRun(input: {
  id: string;
  sessionId: string;
  description: string;
  subagentType: SubagentKind;
  depth: number;
  parentAgentId: string | null;
  abortController: AbortController;
}) {
  runs.set(input.id, {
    id: input.id,
    sessionId: input.sessionId,
    description: input.description,
    subagentType: input.subagentType,
    status: "running",
    depth: input.depth,
    parentAgentId: input.parentAgentId,
    startedAt: Date.now(),
    endedAt: null,
    summary: "",
    transcript: [],
  });
  abortControllers.set(input.id, input.abortController);
  emit();
}

export function appendSubagentTranscript(id: string, content: string, role: SubagentTranscriptEntry["role"] = "assistant") {
  const run = runs.get(id);
  if (!run || !content.trim()) return;
  const entry: SubagentTranscriptEntry = {
    id: `${id}-${run.transcript.length}`,
    role,
    content: content.trim(),
    at: Date.now(),
  };
  const transcript = [...run.transcript, entry].slice(-48);
  runs.set(id, { ...run, transcript });
  emit();
}

export function getSubagentRun(id: string) {
  return runs.get(id) ?? null;
}

const MAX_RUNS_PER_SESSION = 32;

export function completeSubagentRun(id: string, summary: string, status: SubagentRunStatus = "completed") {
  const run = runs.get(id);
  if (!run) return;
  runs.set(id, { ...run, status, summary, endedAt: Date.now() });
  abortControllers.delete(id);
  pruneSubagentRuns(run.sessionId);
  emit();
}

function pruneSubagentRuns(sessionId: string) {
  const sessionRuns = listSubagentRunsForSession(sessionId);
  const completed = sessionRuns.filter((run) => run.status !== "running");
  if (completed.length <= MAX_RUNS_PER_SESSION) return;
  const dropIds = new Set(completed.slice(MAX_RUNS_PER_SESSION).map((run) => run.id));
  for (const id of dropIds) {
    runs.delete(id);
    abortControllers.delete(id);
  }
}

export function cancelSubagentRun(id: string) {
  const controller = abortControllers.get(id);
  controller?.abort();
  const run = runs.get(id);
  if (run) {
    runs.set(id, { ...run, status: "cancelled", endedAt: Date.now(), summary: "Cancelled" });
  }
  abortControllers.delete(id);
  emit();
}

export function cancelAllSubagentRuns(sessionId: string) {
  for (const run of listSubagentRunsForSession(sessionId)) {
    if (run.status === "running") cancelSubagentRun(run.id);
  }
}

function emit() {
  for (const listener of listeners) listener();
}