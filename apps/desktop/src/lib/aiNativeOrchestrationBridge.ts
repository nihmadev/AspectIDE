// Bridges the native (Rust) turn loop's orchestration tool calls — TodoWrite,
// Goal, Task — into the frontend session stores that the Agent rail reads.
//
// The Rust loop executes these tools and returns their JSON result, but it has no
// way to write the TS-side stores (goal / todos / subagent runs). Without this
// bridge the Agent panel stays empty ("No tasks yet", "0/10 subagents") even while
// the agent is actively managing them. We parse the tool's input/output here and
// mirror it into the same stores the old TS dispatch fed.

import { setAiSessionGoal } from "./aiSessionGoal";
import { replaceAiSessionTodos, type AiSessionTodo } from "./aiSessionTodos";
import {
  appendSubagentTranscript,
  completeSubagentRun,
  getSubagentRun,
  registerSubagentRun,
  updateLastSubagentTranscript,
} from "./aiSubagentRuns";
import type { SubagentKind } from "./aiSubagents";

function parseJson(raw: string | undefined): Record<string, unknown> | null {
  if (!raw) return null;
  try {
    const value = JSON.parse(raw);
    return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
  } catch {
    return null;
  }
}

const KNOWN_SUBAGENT_KINDS = new Set<SubagentKind>([
  "generalPurpose",
  "codeReviewer",
  "testRunner",
  "explorer",
]);

function coerceSubagentKind(value: unknown): SubagentKind {
  return typeof value === "string" && KNOWN_SUBAGENT_KINDS.has(value as SubagentKind)
    ? (value as SubagentKind)
    : ("generalPurpose" as SubagentKind);
}

/** Mirror a started Task tool call into the subagent-runs store (shows a running row). */
export function bridgeNativeToolStarted(sessionId: string, callId: string, tool: string, input: string) {
  if (tool !== "Task") return;
  const args = parseJson(input);
  if (!args) return;
  registerSubagentRun({
    id: callId,
    sessionId,
    description: typeof args.description === "string" ? args.description : "Subagent task",
    subagentType: coerceSubagentKind(args.subagent_type),
    depth: 1,
    parentAgentId: null,
    abortController: new AbortController(),
  });
}

/**
 * Stream a native subagent's live progress into its run row (Agent rail).
 * stage "text" replaces the last assistant transcript entry in place with the
 * accumulated snapshot (the Rust side throttles to ~3 events/s); stage "tool"
 * appends a system line naming the tool the subagent just started; stage
 * "done"/"error" settles the run row with its summary — for a BACKGROUND task
 * this is the ONLY completion signal (the Task tool call itself returned
 * "started" long ago); for a foreground task the Task completion bridge below
 * writes the same summary right after (idempotent).
 */
export function bridgeNativeSubagentProgress(
  callId: string,
  stage: "text" | "tool" | "done" | "error" | "cancelled",
  content: string,
  tool: string,
) {
  const run = getSubagentRun(callId);
  if (!run) return;
  // A row the user already cancelled must not be revived by a late done/error.
  if (run.status !== "running") return;
  if (stage === "text" && content.trim()) {
    updateLastSubagentTranscript(callId, content);
    return;
  }
  if (stage === "tool" && tool.trim()) {
    const preview = content.trim();
    appendSubagentTranscript(
      callId,
      preview ? `→ ${tool} ${preview}` : `→ ${tool}`,
      "system",
    );
    return;
  }
  if (stage === "done") {
    completeSubagentRun(callId, content.trim() || "Done", "completed");
    return;
  }
  if (stage === "error") {
    completeSubagentRun(callId, content.trim() || "Failed", "failed");
    return;
  }
  // A whole-turn Stop (or a per-row Stop the UI hasn't applied yet) settled the
  // subagent on the Rust side — mirror it as cancelled, not completed.
  if (stage === "cancelled") {
    completeSubagentRun(callId, content.trim() || "Cancelled", "cancelled");
  }
}

/** Mirror a completed orchestration tool call into the matching session store. */
export function bridgeNativeToolCompleted(
  sessionId: string,
  callId: string,
  tool: string,
  status: string,
  output: string,
) {
  if (status === "error") {
    if (tool === "Task" && getSubagentRun(callId)) completeSubagentRun(callId, "Failed", "failed");
    return;
  }
  const result = parseJson(output);
  if (!result) return;

  if (tool === "TodoWrite") {
    const rawTodos = Array.isArray(result.todos) ? result.todos : [];
    const todos: AiSessionTodo[] = rawTodos
      .map((entry): AiSessionTodo | null => {
        if (!entry || typeof entry !== "object") return null;
        const todo = entry as Record<string, unknown>;
        const content = typeof todo.content === "string" ? todo.content.trim() : "";
        if (!content) return null;
        return {
          id: typeof todo.id === "string" && todo.id ? todo.id : `todo-${content.slice(0, 12)}`,
          content,
          status: normalizeTodoStatus(todo.status),
          priority: normalizeTodoPriority(todo.priority),
          source: "agent",
          notes: typeof todo.notes === "string" ? todo.notes : undefined,
        };
      })
      .filter((todo): todo is AiSessionTodo => todo !== null);
    if (todos.length > 0) replaceAiSessionTodos(sessionId, todos);
    return;
  }

  if (tool === "Goal") {
    const goal = typeof result.goal === "string" ? result.goal.trim() : "";
    if (goal) setAiSessionGoal(sessionId, goal);
    return;
  }

  if (tool === "Task") {
    // Background spawn: the tool result is just {status:"started"} — the run row
    // keeps streaming and is settled later by the SubagentProgress done/error
    // stage, not by this immediate completion.
    if (result.background === true) return;
    const summary = typeof result.summary === "string" ? result.summary : "Done";
    const run = getSubagentRun(callId);
    if (run && run.status === "running") completeSubagentRun(callId, summary, "completed");
  }
}

function normalizeTodoStatus(value: unknown): AiSessionTodo["status"] {
  switch (typeof value === "string" ? value.toLowerCase() : "") {
    case "completed":
    case "done":
      return "completed";
    case "in_progress":
    case "in-progress":
    case "active":
      return "in_progress";
    case "blocked":
    case "waiting":
      return "blocked";
    case "cancelled":
    case "canceled":
    case "skipped":
      return "cancelled";
    default:
      return "pending";
  }
}

function normalizeTodoPriority(value: unknown): AiSessionTodo["priority"] {
  switch (typeof value === "string" ? value.toLowerCase() : "") {
    case "high":
      return "high";
    case "low":
      return "low";
    default:
      return "medium";
  }
}
