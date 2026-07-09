import type { GoalRunState } from "./session-goal-run";
import { buildGoalLimitWarning, buildGoalObjectiveBlock, isGoalTokenBudgetEnabled } from "./run-limits";

export function buildGoalContinuationOrchestrationBlock(
  run: GoalRunState,
  options: { evaluatorNote?: string; budgetWrapup?: boolean } = {},
) {
  const usedTokens = run.promptTokens + run.completionTokens;
  const budgetOn = isGoalTokenBudgetEnabled(run.limits);
  const remainingTokens = Math.max(0, run.limits.maxTokens - usedTokens);
  const remainingTurns = Math.max(0, run.limits.maxRounds - run.round);
  const elapsedSeconds = Math.round((Date.now() - run.startedAt) / 1000);
  const evaluatorNote = options.evaluatorNote?.trim();

  const lines = [
    "<goal_continuation>",
    buildGoalObjectiveBlock(run.goal),
    "",
    "<progress_budget>",
    `rounds_used: ${run.round}`,
    `rounds_remaining: ${remainingTurns}`,
    `tracked_tokens_used: ${usedTokens}`,
    `tracked_tokens_remaining: ${budgetOn ? remainingTokens : "unlimited"}`,
    `elapsed_seconds: ${elapsedSeconds}`,
    `progress_percent: ${run.progress}`,
    "</progress_budget>",
    "",
  ];

  if (evaluatorNote) {
    lines.push("<evaluator_note>", evaluatorNote, "</evaluator_note>", "");
  }

  if (run.lastCheckpoint?.summary) {
    lines.push("<recent_checkpoint>", run.lastCheckpoint.summary, "</recent_checkpoint>", "");
  }

  if (options.budgetWrapup) {
    lines.push(
      "<budget_wrapup>",
      "This goal is near its tracked token or time budget. Finish the current step if it is small and safe.",
      "Then write a concise handoff: what is done, what remains, and the next concrete command or file.",
      "Do not output [goal:complete] unless the goal is actually finished and verified.",
      "After the handoff, stop.",
      "</budget_wrapup>",
    );
  } else {
    lines.push(
      "<next_step>",
      "Continue working toward the active goal. Take the next concrete step.",
      "Prefer verifying actual current state over assuming prior work succeeded.",
      "In user-visible replies, cite only tool-confirmed paths and facts — never invent directories or file trees.",
      "If a check fails, repair the issue rather than shrinking the scope.",
      "</next_step>",
    );
  }

  lines.push(
    "",
    "<completion_audit>",
    "Before outputting [goal:complete], treat completion as unproven.",
    "Verify the result against the goal objective and the current project state.",
    "Only mark complete when every requirement is satisfied and relevant checks passed or absence is justified.",
    "If user input is required, explain the specific blocker in the line immediately before [goal:blocked].",
    "</completion_audit>",
    "",
    "End with [goal:complete] only when the goal is fully satisfied.",
    "End with [goal:blocked] only if user input is required.",
    buildGoalLimitWarning(run),
    "</goal_continuation>",
  );

  return lines.filter(Boolean).join("\n");
}