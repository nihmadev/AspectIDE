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
  /** Monotonic count of transcript entries ever appended. Unlike transcript.length
   *  (capped at 48) this never decreases, so it yields collision-free entry ids. */
  transcriptSeq: number;
  /** Bumped on every mutation (append/status/summary). The rail's value-stable
   *  signature reads this so live updates keep flowing past the 48-entry cap. */
  revision: number;
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
    transcriptSeq: 0,
    revision: 0,
  });
  abortControllers.set(input.id, input.abortController);
  emit();
}

export function appendSubagentTranscript(id: string, content: string, role: SubagentTranscriptEntry["role"] = "assistant") {
  const run = runs.get(id);
  if (!run || !content.trim()) return;
  const seq = run.transcriptSeq + 1;
  const entry: SubagentTranscriptEntry = {
    // Monotonic seq, not transcript.length: the latter pins at 48 once capped and would
    // recycle ids, producing duplicate React keys in the transcript panel.
    id: `${id}-${seq}`,
    role,
    content: content.trim(),
    at: Date.now(),
  };
  const transcript = [...run.transcript, entry].slice(-48);
  runs.set(id, { ...run, transcript, transcriptSeq: seq, revision: run.revision + 1 });
  emit();
}

/**
 * Update the most recent assistant transcript entry in-place for streaming patches.
 * Instead of appending a growing prefix on every streaming chunk (causing duplicate/
 * balloon allocations), we replace the last assistant entry's content with the
 * accumulated full text and only emit once. If there is no existing assistant entry
 * to update, falls back to appendSubagentTranscript.
 */
export function updateLastSubagentTranscript(id: string, fullContent: string) {
  const run = runs.get(id);
  if (!run || !fullContent.trim()) return;
  const transcript = [...run.transcript];
  const lastIdx = transcript.length - 1;
  if (lastIdx >= 0 && transcript[lastIdx].role === "assistant") {
    // Replace in-place — same id/at so React doesn't unmount/remount the entry.
    transcript[lastIdx] = { ...transcript[lastIdx], content: fullContent.trim() };
    runs.set(id, { ...run, transcript, revision: run.revision + 1 });
    emit();
  } else {
    appendSubagentTranscript(id, fullContent);
  }
}

export function getSubagentRun(id: string) {
  return runs.get(id) ?? null;
}

const MAX_RUNS_PER_SESSION = 32;

export function completeSubagentRun(id: string, summary: string, status: SubagentRunStatus = "completed") {
  const run = runs.get(id);
  if (!run) return;
  runs.set(id, { ...run, status, summary, endedAt: Date.now(), revision: run.revision + 1 });
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
  // Native (Rust) subagents don't listen to the TS AbortController — the one
  // registered for them is a placeholder. Signal the Rust loop too so the
  // subagent actually STOPS (model stream aborted, remaining tools skipped)
  // instead of just flipping the UI row to "cancelled" while it keeps working.
  // Dynamic import keeps this store free of a static tauri.ts dependency
  // (vitest / non-Tauri runtimes); best-effort — the UI state settles either way.
  void import("./tauri")
    .then((m) => m.luxCommands.aiCancelSubagent(id))
    .catch(() => {});
  const run = runs.get(id);
  if (run) {
    runs.set(id, { ...run, status: "cancelled", endedAt: Date.now(), summary: "Cancelled", revision: run.revision + 1 });
  }
  abortControllers.delete(id);
  emit();
}

/** Manually remove a FINISHED run row (per-row delete in the rail). A running
 *  run is cancelled first so the native loop stops before the row disappears. */
export function removeSubagentRun(id: string) {
  const run = runs.get(id);
  if (!run) return;
  if (run.status === "running") cancelSubagentRun(id);
  runs.delete(id);
  abortControllers.delete(id);
  emit();
}

/** Clear every finished (non-running) run for a session — the history broom. */
export function clearFinishedSubagentRuns(sessionId: string) {
  let changed = false;
  for (const run of listSubagentRunsForSession(sessionId)) {
    if (run.status === "running") continue;
    runs.delete(run.id);
    abortControllers.delete(run.id);
    changed = true;
  }
  if (changed) emit();
}

export function cancelAllSubagentRuns(sessionId: string) {
  for (const run of listSubagentRunsForSession(sessionId)) {
    if (run.status === "running") cancelSubagentRun(run.id);
  }
}

function emit() {
  for (const listener of listeners) listener();
}