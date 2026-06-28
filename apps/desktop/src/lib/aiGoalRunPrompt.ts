import { buildGoalLimitWarning, buildGoalObjectiveBlock } from "./aiGoalRunLimits";
import { getActiveGoalRun } from "./aiSessionGoalRun";
import { listAiSessionTodos } from "./aiSessionTodos";

/** Extra system-facing hint while a /goal autonomous run is active. */
export function buildGoalRunPromptSection(chatSessionId: string) {
  const run = getActiveGoalRun(chatSessionId);
  if (!run) return null;
  const todos = listAiSessionTodos(chatSessionId);
  const openTodos = todos.filter((todo) => todo.status !== "completed" && todo.status !== "cancelled");
  const todoHint = openTodos.length > 0
    ? `Open TodoWrite tasks (${openTodos.length}): ${openTodos.slice(0, 6).map((todo) => todo.content).join("; ")}`
    : "No open TodoWrite tasks — create/update if the goal has multiple steps.";

  const evaluatorNote = run.lastEvaluatorReason?.trim();
  const checkpoint = run.lastCheckpoint?.summary;
  return [
    "Active /goal autonomous run (session-scoped):",
    buildGoalObjectiveBlock(run.goal),
    `- Reported progress: ${run.progress}% — reach 100% only when the condition truly holds.`,
    `- Orchestration round ${run.round + 1} of ${run.limits.maxRounds} (silent follow-ups — no extra user chat messages).`,
    `- Budget: ${(run.promptTokens + run.completionTokens).toLocaleString()}/${run.limits.maxTokens.toLocaleString()} tokens · ${Math.round((Date.now() - run.startedAt) / 1000)}s elapsed.`,
    evaluatorNote ? `- Evaluator note: ${evaluatorNote}` : null,
    checkpoint ? `- Latest checkpoint: ${checkpoint}` : null,
    todoHint,
    "- Each turn: call Goal with honest `progress` (0–99 while work/verification/final reporting remains).",
    "- Do NOT mark TodoWrite items `completed` immediately after starting them; mark completed only after the concrete action is done and verified or its result is reported.",
    '- When done: call Goal with `status: "completed"` and `progress: 100` only after all open tasks are completed/cancelled, verification evidence exists, and the final user-visible summary is ready. Otherwise continue.',
    "- Never use `completed` as a planning placeholder. Use `in_progress` during execution and update after the tool result/diagnostic/test confirms the step.",
    "- If blocked on user input: final line `[goal:blocked]` plus a one-line explanation on the line before it.",
    "- User-visible answers: only paths and facts from tool output — do not invent folders, archives, or project structure.",
    "- Multi-step work: keep TodoWrite current; verify with tools before declaring complete.",
    buildGoalLimitWarning(run),
  ].filter((line): line is string => Boolean(line)).join("\n");
}
