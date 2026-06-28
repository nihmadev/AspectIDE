import type { AiChatMessage, AiChatTurnTokenUsage } from "./aiChatTypes";
import { resolveAssistantTurnUsage } from "./aiTurnUsage";
import {
  buildGoalObjectiveBlock,
  extractGoalBlockedReason,
  isExploratoryGoalRun,
  mergeGoalRunLimits,
  resolveDefaultGoalRunLimits,
  type GoalRunLimits,
} from "./aiGoalRunLimits";
import { buildGoalContinuationOrchestrationBlock } from "./aiGoalRunPromptBlocks";
import { getAiSessionGoal } from "./aiSessionGoal";
import { listAiSessionTodos } from "./aiSessionTodos";
import { isFullExecutionAgentMode, type AiAgentMode, type AiToolRoundLimit } from "./aiPreferences";
import { describeLoopSignature, recordTurnToolSignatures, resetToolLoopDetector } from "./aiLoopDetector";

export type GoalRunPhase = "running" | "paused" | "completed" | "stopped" | "max_rounds" | "blocked";

export type GoalRunCheckpoint = { summary: string; timestamp: number };
export type GoalRunHistoryEntry = { type: string; detail: string; timestamp: number };

export type GoalRunState = {
  sessionId: string;
  goal: string;
  phase: GoalRunPhase;
  startedAt: number;
  completedAt: number | null;
  round: number;
  /** @deprecated use limits.maxRounds */
  maxRounds: number;
  limits: GoalRunLimits;
  progress: number;
  promptTokens: number;
  completionTokens: number;
  completionSummary: string | null;
  blockedReason: string | null;
  stopReason: string | null;
  noProgressTurns: number;
  lastProgressAt: number;
  lastContinueAt: number;
  budgetWrapupSent: boolean;
  lastAssistantSnippet: string;
  lastCheckpoint: GoalRunCheckpoint | null;
  checkpoints: GoalRunCheckpoint[];
  history: GoalRunHistoryEntry[];
  /** Prevents double-counting when a turn is finalized after Goal tool already set progress. */
  lastAccountedAssistantMessageId: string | null;
  /** Shown in orchestration rail — why the next silent turn is running (Codex/Claude-style). */
  lastEvaluatorReason: string | null;
  /** True once any tool call has run during this goal run (used to reject premature completion). */
  everUsedTools: boolean;
  /** True once a premature [goal:complete] has been challenged, so we only challenge once. */
  completionChallenged: boolean;
  /** Automatic-mode wall-clock ceiling in minutes (from preferences); undefined = none. */
  hardStopMinutes?: number;
};

export type { GoalRunLimits };
export { isExploratoryGoalRun, resolveGoalRunMaxRounds } from "./aiGoalRunLimits";

const goalRuns = new Map<string, GoalRunState>();
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

export function subscribeAiSessionGoalRuns(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getAiSessionGoalRunsSnapshot() {
  return goalRuns.size;
}

export function getActiveGoalRun(sessionId: string): GoalRunState | null {
  const run = goalRuns.get(sessionId);
  if (!run || run.phase !== "running") return null;
  return run;
}

/** Running or paused — for orchestration rail UI. */
export function getDisplayGoalRun(sessionId: string): GoalRunState | null {
  const run = goalRuns.get(sessionId);
  if (!run || (run.phase !== "running" && run.phase !== "paused")) return null;
  return run;
}

export function getGoalRunSnapshot(sessionId: string): GoalRunState | null {
  return goalRuns.get(sessionId) ?? null;
}

export function getGoalRunEvaluatorReason(sessionId: string): string | null {
  return goalRuns.get(sessionId)?.lastEvaluatorReason ?? null;
}

const CHECKPOINT_CHAR_LIMIT = 280;
const MAX_CHECKPOINTS = 5;
const MAX_HISTORY = 20;

function summarizeCheckpoint(text: string) {
  const normalized = text.replace(/\s+/g, " ").trim();
  if (!normalized) return "";
  return normalized.length > CHECKPOINT_CHAR_LIMIT
    ? `${normalized.slice(0, CHECKPOINT_CHAR_LIMIT - 1)}…`
    : normalized;
}

function pushGoalHistory(run: GoalRunState, type: string, detail: string) {
  run.history = [
    ...run.history,
    { type, detail: detail.slice(0, 400), timestamp: Date.now() },
  ].slice(-MAX_HISTORY);
}

export function recordGoalCheckpoint(run: GoalRunState, assistantText: string) {
  const summary = summarizeCheckpoint(assistantText);
  if (!summary || run.lastCheckpoint?.summary === summary) return;
  const checkpoint: GoalRunCheckpoint = { summary, timestamp: Date.now() };
  run.lastCheckpoint = checkpoint;
  run.checkpoints = [...run.checkpoints, checkpoint].slice(-MAX_CHECKPOINTS);
}

export function formatGoalRunStatusText(run: GoalRunState) {
  const elapsed = Math.round(formatGoalRunElapsedMs(run) / 1000);
  const maxSeconds = Math.round(run.limits.maxDurationMs / 1000);
  const tokens = formatGoalRunTokenTotal(run);
  const lines = [
    `Goal: ${run.goal}`,
    `Phase: ${run.phase}`,
    `Rounds: ${run.round}/${run.limits.maxRounds}`,
    `Progress: ${run.progress}%`,
    `Tokens: ${tokens.toLocaleString()}/${run.limits.maxTokens.toLocaleString()}`,
    `Elapsed: ${elapsed}s/${maxSeconds}s`,
    run.lastCheckpoint ? `Checkpoint: ${run.lastCheckpoint.summary}` : "Checkpoint: none yet",
    run.lastEvaluatorReason ? `Next: ${run.lastEvaluatorReason}` : null,
    run.blockedReason ? `Blocked: ${run.blockedReason}` : null,
    run.stopReason ? `Stop: ${run.stopReason}` : null,
  ].filter((line): line is string => Boolean(line));
  return lines.join("\n");
}

export function pauseGoalRun(sessionId: string, reason = "Paused by user.") {
  const run = goalRuns.get(sessionId);
  if (!run || run.phase !== "running") return null;
  run.phase = "paused";
  run.stopReason = "paused";
  run.lastEvaluatorReason = reason;
  pushGoalHistory(run, "paused", reason);
  goalRuns.set(sessionId, run);
  emit();
  return run;
}

export function resumeGoalRun(sessionId: string) {
  const run = goalRuns.get(sessionId);
  if (!run || (run.phase !== "paused" && run.phase !== "stopped" && run.phase !== "blocked")) return null;
  run.phase = "running";
  run.round = 0;
  run.promptTokens = 0;
  run.completionTokens = 0;
  run.startedAt = Date.now();
  run.completedAt = null;
  run.noProgressTurns = 0;
  stallSignals.delete(`${sessionId}:stall`);
  resetToolLoopDetector(sessionId);
  run.budgetWrapupSent = false;
  run.blockedReason = null;
  run.stopReason = null;
  run.lastEvaluatorReason = "Goal resumed with a fresh local budget.";
  run.lastAccountedAssistantMessageId = null;
  pushGoalHistory(run, "resumed", "User resumed the goal with a fresh budget window.");
  goalRuns.set(sessionId, run);
  emit();
  return run;
}

export function startGoalRun(
  sessionId: string,
  goal: string,
  options: {
    agentMode: AiAgentMode;
    toolRoundLimit: AiToolRoundLimit;
    limits?: Partial<GoalRunLimits>;
    preferences?: { goalRunMaxTokens?: number | null; goalRunMaxRounds?: number | null; automaticModeHardStopMinutes?: number | null };
  },
): GoalRunState | null {
  if (!isFullExecutionAgentMode(options.agentMode)) return null;
  const trimmedGoal = goal.trim();
  if (!trimmedGoal) return null;
  const limits = mergeGoalRunLimits(
    resolveDefaultGoalRunLimits(options.toolRoundLimit, trimmedGoal, options.preferences),
    options.limits ?? {},
  );
  const run: GoalRunState = {
    sessionId,
    goal: trimmedGoal,
    phase: "running",
    startedAt: Date.now(),
    completedAt: null,
    round: 0,
    maxRounds: limits.maxRounds,
    limits,
    progress: 0,
    promptTokens: 0,
    completionTokens: 0,
    completionSummary: null,
    blockedReason: null,
    stopReason: null,
    noProgressTurns: 0,
    lastProgressAt: 0,
    lastContinueAt: 0,
    budgetWrapupSent: false,
    lastAssistantSnippet: "",
    lastCheckpoint: null,
    checkpoints: [],
    history: [],
    lastAccountedAssistantMessageId: null,
    lastEvaluatorReason: "Starting first turn toward the completion condition.",
    everUsedTools: false,
    completionChallenged: false,
    hardStopMinutes: options.preferences?.automaticModeHardStopMinutes ?? undefined,
  };
  pushGoalHistory(
    run,
    "set",
    `Limits: ${limits.maxRounds} rounds, ${Math.round(limits.maxDurationMs / 1000)}s, ${limits.maxTokens.toLocaleString()} tokens.`,
  );
  stallSignals.delete(`${sessionId}:stall`);
  resetToolLoopDetector(sessionId);
  goalRuns.set(sessionId, run);
  emit();
  return run;
}

export function stopGoalRun(sessionId: string) {
  const run = goalRuns.get(sessionId);
  if (!run) return;
  if (run.phase === "running") {
    run.phase = "stopped";
    run.completedAt = Date.now();
  }
  resetToolLoopDetector(sessionId);
  goalRuns.set(sessionId, run);
  emit();
}

function finishGoalRun(sessionId: string, phase: Exclude<GoalRunPhase, "running">, summary: string | null) {
  const run = goalRuns.get(sessionId);
  if (!run) return null;
  run.phase = phase;
  run.completedAt = Date.now();
  run.progress = phase === "completed" ? 100 : run.progress;
  run.completionSummary = summary;
  run.lastEvaluatorReason = summary;
  stallSignals.delete(`${sessionId}:stall`);
  goalRuns.set(sessionId, run);
  emit();
  return run;
}

/** Drop all state for a closed/deleted session so the maps don't grow unbounded. */
export function disposeGoalRun(sessionId: string) {
  goalRuns.delete(sessionId);
  stallSignals.delete(`${sessionId}:stall`);
  resetToolLoopDetector(sessionId);
  emit();
}

function resolveTurnUsage(
  usage: AiChatTurnTokenUsage | undefined,
  assistant: AiChatMessage | null,
) {
  return resolveAssistantTurnUsage(usage, assistant);
}

function shouldAccountGoalTurn(run: GoalRunState, assistant: AiChatMessage | null) {
  if (run.phase === "running") return true;
  // Goal tool may finish the run mid-turn before the chat finally block runs.
  if (run.phase === "completed" && run.round === 0 && assistant?.id) return true;
  return false;
}

export function recordGoalRunTurnUsage(
  sessionId: string,
  usage: AiChatTurnTokenUsage | undefined,
  assistant: AiChatMessage | null = null,
) {
  const run = goalRuns.get(sessionId);
  if (!run || !shouldAccountGoalTurn(run, assistant)) return;
  const assistantId = assistant?.id ?? null;
  if (assistantId && run.lastAccountedAssistantMessageId === assistantId) return;
  if (assistantId) run.lastAccountedAssistantMessageId = assistantId;

  run.round += 1;
  const turnUsage = resolveTurnUsage(usage, assistant);
  if (turnUsage) {
    run.promptTokens += Math.max(0, turnUsage.promptTokens);
    run.completionTokens += Math.max(0, turnUsage.completionTokens);
    if (turnUsage.promptTokens > 0 || turnUsage.completionTokens > 0) run.lastProgressAt = Date.now();
  }
  if (assistant?.content.trim()) {
    const snippet = summarizeCheckpoint(assistant.content);
    if (snippet !== run.lastAssistantSnippet) {
      run.lastAssistantSnippet = snippet;
      recordGoalCheckpoint(run, assistant.content);
    }
  }
  goalRuns.set(sessionId, run);
  emit();
}

export function applyGoalToolProgress(
  sessionId: string,
  input: { goal?: string; progress?: number; status?: string; summary?: string },
) {
  const run = goalRuns.get(sessionId);
  if (!run || run.phase !== "running") return;
  if (typeof input.progress === "number" && Number.isFinite(input.progress)) {
    run.progress = Math.min(100, Math.max(run.progress, Math.round(input.progress)));
  }
  if (input.status?.toLowerCase() === "completed") {
    run.progress = 100;
    const summary = input.summary?.trim();
    if (summary) run.completionSummary = summary;
    // Defer finishGoalRun until the turn is recorded in AiChatPanel finally + evaluateGoalCondition.
    goalRuns.set(sessionId, run);
    emit();
    return;
  }
  goalRuns.set(sessionId, run);
  emit();
}

/** First-turn directive — stored as internal history only, never shown in chat UI. */
export function buildGoalKickoffDirective(goal: string, extraMessage?: string) {
  const extra = extraMessage?.trim();
  const lines = [
    "<goal_kickoff>",
    buildGoalObjectiveBlock(goal),
    "",
    "Work toward this condition now. Orchestration is silent — no extra user chat messages.",
    "Use tools to verify state (tests, builds, reads) and surface evidence in your reply.",
    "In user-visible replies: cite only paths and facts confirmed by tool output in this session — never invent directories, archives, or a full project tree.",
    "Each turn: call Goal with honest progress (0–100).",
    'Call Goal with status "completed" and progress 100 only when the condition is fully satisfied.',
    "You may end a turn with a final line `[goal:complete]` or `[goal:blocked]` (stripped from chat UI; prefer the Goal tool for visible completion).",
    "Use TodoWrite for multi-step engineering work.",
    "</goal_kickoff>",
  ];
  if (extra) lines.splice(lines.length - 1, 0, "", "Additional user notes:", extra);
  return lines.join("\n");
}

/** @deprecated Use buildGoalKickoffDirective */
export const buildGoalKickoffMessage = buildGoalKickoffDirective;

/** Silent follow-up directive after evaluator says condition is not met yet. */
export function buildGoalContinuationDirective(sessionId: string, options: { budgetWrapup?: boolean } = {}) {
  const run = goalRuns.get(sessionId);
  if (!run) {
    return "Lux /goal orchestration — continue silently toward the pinned completion condition.";
  }
  return buildGoalContinuationOrchestrationBlock(run, {
    evaluatorNote: run.lastEvaluatorReason ?? undefined,
    budgetWrapup: options.budgetWrapup,
  });
}

export function resolveGoalContinuationDelayMs(sessionId: string) {
  return goalRuns.get(sessionId)?.limits.minDelayMs ?? 1500;
}

export function shouldSendBudgetWrapupContinuation(sessionId: string) {
  const run = goalRuns.get(sessionId);
  if (!run || run.phase !== "running" || run.budgetWrapupSent) return false;
  const used = formatGoalRunTokenTotal(run);
  return used >= Math.floor(run.limits.maxTokens * run.limits.budgetWrapupRatio);
}

export function markBudgetWrapupRequested(sessionId: string) {
  const run = goalRuns.get(sessionId);
  if (!run) return;
  run.budgetWrapupSent = true;
  pushGoalHistory(run, "budget-wrapup", "Requested final handoff near token budget.");
  goalRuns.set(sessionId, run);
  emit();
}

/** @deprecated Use buildGoalContinuationDirective */
export const buildGoalContinuationMessage = buildGoalContinuationDirective;

export function lastAssistantMessage(messages: AiChatMessage[]) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index]?.role === "assistant") return messages[index];
  }
  return null;
}

function parseGoalToolOutput(output: string | undefined) {
  if (!output?.trim()) return null;
  const trimmed = output.trim();
  const payloads = trimmed.includes("\n\n")
    ? [trimmed.slice(trimmed.indexOf("\n\n") + 2).trim(), trimmed]
    : [trimmed];
  for (const payload of payloads) {
    if (!payload) continue;
    try {
      const parsed = JSON.parse(payload) as Record<string, unknown>;
      const progress = typeof parsed.progress === "number" ? parsed.progress : undefined;
      const status = typeof parsed.status === "string" ? parsed.status : undefined;
      const summary = typeof parsed.summary === "string" ? parsed.summary : undefined;
      if (progress != null || status || summary) return { progress, status, summary };
    } catch {
      // try next payload shape
    }
  }
  return null;
}

export function syncGoalRunFromAssistantMessage(sessionId: string, message: AiChatMessage | null) {
  if (!message?.toolCalls?.length) return;
  const run = goalRuns.get(sessionId);
  if (run && !run.everUsedTools) {
    run.everUsedTools = true;
    goalRuns.set(sessionId, run);
  }
  for (const call of message.toolCalls) {
    if (call.tool !== "Goal" || call.status !== "success" || !call.output) continue;
    const parsed = parseGoalToolOutput(call.output);
    if (!parsed) continue;
    applyGoalToolProgress(sessionId, parsed);
  }
}

const GOAL_COMPLETE_MARKER = /(?:\[goal:complete\]|^goal:complete)\s*$/im;
const GOAL_BLOCKED_MARKER = /(?:\[goal:blocked\]|^goal:blocked)\s*$/im;

function detectGoalCompleteMarker(message: AiChatMessage | null) {
  if (!message?.content.trim()) return false;
  return GOAL_COMPLETE_MARKER.test(message.content.trim());
}

function detectGoalBlockedMarker(message: AiChatMessage | null) {
  if (!message?.content.trim()) return false;
  return GOAL_BLOCKED_MARKER.test(message.content.trim());
}

function goalRunDurationExceeded(run: GoalRunState) {
  return Date.now() - run.startedAt >= run.limits.maxDurationMs;
}

function goalRunTokensExceeded(run: GoalRunState) {
  return formatGoalRunTokenTotal(run) >= run.limits.maxTokens;
}

function buildEvaluatorReason(run: GoalRunState, assistant: AiChatMessage | null, messages: AiChatMessage[]) {
  if (detectGoalBlockedMarker(assistant)) {
    const blocked = assistant ? extractGoalBlockedReason(assistant.content) : "";
    return blocked || "Assistant reported a blocker — user input may be required.";
  }
  if (run.progress >= 100) return "Progress is 100% — verifying completion.";
  const todos = listAiSessionTodos(run.sessionId);
  const openTodos = todos.filter((todo) => todo.status !== "completed" && todo.status !== "cancelled");
  if (openTodos.length > 0) {
    return `Progress ${run.progress}% — ${openTodos.length} open task(s) remain.`;
  }
  if (assistant && (assistant.toolCalls?.length ?? 0) === 0 && /\b(done|complete|finished|готово)\b/i.test(assistant.content)) {
    return `Progress ${run.progress}% — assistant claimed done but the completion condition is not satisfied yet.`;
  }
  const visibleUserTurns = messages.filter((entry) => entry.visibility !== "internal" && entry.role === "user").length;
  if (visibleUserTurns === 0 && run.round <= 1) {
    return `Progress ${run.progress}% — initial verification pass; condition "${run.goal}" is not complete yet.`;
  }
  return `Progress ${run.progress}% — completion condition not met; continue with the next concrete step.`;
}

export type GoalEvaluation =
  | { status: "idle" }
  | { status: "satisfied"; reason: string }
  | { status: "blocked"; reason: string }
  | { status: "max_rounds"; reason: string }
  | { status: "stall"; reason: string }
  | { status: "paused"; reason: string }
  | { status: "budget_wrapup"; reason: string }
  | { status: "continue"; reason: string };

export function evaluateGoalCondition(
  sessionId: string,
  messages: AiChatMessage[],
  agentMode?: AiAgentMode,
): GoalEvaluation {
  const run = goalRuns.get(sessionId);
  if (!run || run.phase !== "running") return { status: "idle" };

  const assistant = lastAssistantMessage(messages);
  syncGoalRunFromAssistantMessage(sessionId, assistant);
  const refreshed = goalRuns.get(sessionId);
  if (!refreshed) return { status: "idle" };
  if (refreshed.phase === "completed") {
    return { status: "satisfied", reason: refreshed.completionSummary ?? "Goal completed." };
  }

  if (detectGoalBlockedMarker(assistant)) {
    const reason = buildEvaluatorReason(refreshed, assistant, messages);
    // Automatic mode is full autonomy: "blocked on the user" is not a valid stop —
    // the agent must decide and push on. Convert the block into a continue with a
    // self-decide nudge instead of pausing the run for human input.
    if (agentMode === "automatic") {
      refreshed.lastEvaluatorReason = `${AUTOMATIC_UNBLOCK_NUDGE} (was: ${reason})`;
      goalRuns.set(sessionId, refreshed);
      emit();
      return { status: "continue", reason: refreshed.lastEvaluatorReason };
    }
    refreshed.blockedReason = reason;
    finishGoalRun(sessionId, "blocked", reason);
    return { status: "blocked", reason };
  }

  if (detectGoalCompleteMarker(assistant) || refreshed.progress >= 100) {
    // Guard against premature completion: a [goal:complete] declared on a very early
    // round with no tool work at all is almost always the model claiming victory
    // without doing anything. Challenge it once and require a verification turn before
    // accepting. (progress >= 100 from the Goal tool is trusted — that's explicit.)
    const markerOnly = detectGoalCompleteMarker(assistant) && refreshed.progress < 100;
    const noWorkYet = !refreshed.everUsedTools
      && (assistant?.toolCalls?.length ?? 0) === 0
      && refreshed.round <= 2;
    if (markerOnly && noWorkYet && !refreshed.completionChallenged) {
      refreshed.completionChallenged = true;
      refreshed.lastEvaluatorReason =
        "You signaled completion, but no tools have run and the run just started. Do not declare done without evidence: make the actual changes, then read/test the result to verify, and only then mark complete.";
      goalRuns.set(sessionId, refreshed);
      emit();
      return { status: "continue", reason: refreshed.lastEvaluatorReason };
    }
    finishGoalRun(sessionId, "completed", refreshed.completionSummary ?? "Completion condition satisfied.");
    return { status: "satisfied", reason: refreshed.completionSummary ?? "Completion condition satisfied." };
  }

  // Tool-call loop guard — evaluated BEFORE the advisory-limit branch so it runs on
  // every turn, including automatic mode (which deliberately blows past advisory
  // round/token/duration budgets). recordTurnToolSignatures is the only place the
  // loop ring buffer is fed, so it must see every turn or the detector goes blind.
  // A warning-level loop gets a corrective nudge; a critical loop pauses the run
  // (non-automatic) or forces a strategy change (automatic) so an autonomous run
  // can't spin forever burning tokens.
  const loop = recordTurnToolSignatures(sessionId, assistant?.toolCalls);
  if (loop.critical) {
    resetToolLoopDetector(sessionId);
    const what = describeLoopSignature(loop.signature);
    if (agentMode === "automatic") {
      refreshed.lastEvaluatorReason =
        `Loop detected: you called ${what} ${loop.count}× with no progress. STOP repeating it — take a different approach, or if truly stuck, record the blocker and move to the next part of the task. ${AUTOMATIC_NO_STOP_NUDGE}`;
      goalRuns.set(sessionId, refreshed);
      emit();
      return { status: "continue", reason: refreshed.lastEvaluatorReason };
    }
    const reason = `Paused: detected a tool loop — ${what} repeated ${loop.count}×. Run /goal resume to continue.`;
    pauseGoalRun(sessionId, reason);
    return { status: "paused", reason };
  }
  if (loop.looping) {
    refreshed.lastEvaluatorReason =
      `You are repeating ${describeLoopSignature(loop.signature)} (${loop.count}×). Change approach — the same call keeps producing the same result.`;
    goalRuns.set(sessionId, refreshed);
    emit();
    return { status: "continue", reason: refreshed.lastEvaluatorReason };
  }

  // Hard wall-clock ceiling for automatic mode — a TRUE ceiling, evaluated every turn
  // independent of the advisory budgets (not only once another limit already tripped).
  const hardStopMinutes = refreshed.hardStopMinutes;
  if (agentMode === "automatic" && typeof hardStopMinutes === "number") {
    const elapsedMs = Date.now() - refreshed.startedAt;
    if (elapsedMs >= hardStopMinutes * 60_000) {
      const reason = `Hard stop: ${hardStopMinutes} minute${hardStopMinutes !== 1 ? "s" : ""} elapsed in automatic mode.`;
      finishGoalRun(sessionId, "max_rounds", reason);
      return { status: "max_rounds", reason };
    }
  }

  const limitReached = goalRunDurationExceeded(refreshed)
    || goalRunTokensExceeded(refreshed)
    || refreshed.round >= refreshed.limits.maxRounds;

  if (limitReached) {
    // Automatic mode is full autonomy: round/duration/token limits are advisory,
    // never terminal. The run ends only when the task is verified complete or the
    // user presses Stop — so keep going (no "final handoff" that could make the
    // model wrap up early).
    if (agentMode === "automatic") {
      refreshed.lastEvaluatorReason = AUTOMATIC_NO_STOP_NUDGE;
      goalRuns.set(sessionId, refreshed);
      emit();
      return { status: "continue", reason: refreshed.lastEvaluatorReason };
    }
    if (!refreshed.budgetWrapupSent) {
      refreshed.budgetWrapupSent = true;
      refreshed.lastEvaluatorReason = "Budget threshold reached — requesting final handoff.";
      pushGoalHistory(refreshed, "limit", refreshed.lastEvaluatorReason);
      goalRuns.set(sessionId, refreshed);
      emit();
      return { status: "budget_wrapup", reason: refreshed.lastEvaluatorReason };
    }
    const reason = goalRunTokensExceeded(refreshed)
      ? `Token budget reached (${formatGoalRunTokenTotal(refreshed).toLocaleString()}).`
      : goalRunDurationExceeded(refreshed)
        ? `Duration limit reached (${Math.round(refreshed.limits.maxDurationMs / 1000)}s).`
        : `Round limit reached (${refreshed.round}/${refreshed.limits.maxRounds}).`;
    finishGoalRun(sessionId, "max_rounds", reason);
    return { status: "max_rounds", reason };
  }

  let monitoring = false;
  if (assistant) {
    const outputTokens = assistant.turnUsage?.completionTokens ?? 0;
    const repeatedSnippet = refreshed.lastAssistantSnippet
      && summarizeCheckpoint(assistant.content) === refreshed.lastAssistantSnippet;
    const lowOutput = refreshed.round > 0
      && outputTokens < refreshed.limits.noProgressTokenThreshold
      && ((assistant.toolCalls?.length ?? 0) === 0 || repeatedSnippet);
    if (lowOutput) {
      refreshed.noProgressTurns += 1;
      // Automatic mode never auto-pauses (there is no user to run /goal resume) —
      // it keeps going with a stronger nudge instead.
      if (refreshed.noProgressTurns >= refreshed.limits.noProgressTurnsBeforePause && agentMode !== "automatic") {
        const reason = `Paused after ${refreshed.noProgressTurns} low-output turn(s) (${outputTokens} completion tokens). Run /goal resume to continue.`;
        pauseGoalRun(sessionId, reason);
        return { status: "paused", reason };
      }
      refreshed.lastEvaluatorReason = agentMode === "automatic"
        ? `Low-output turn ${refreshed.noProgressTurns} — ${AUTOMATIC_NO_STOP_NUDGE}`
        : `Low-output turn ${refreshed.noProgressTurns}/${refreshed.limits.noProgressTurnsBeforePause} — monitoring.`;
      monitoring = true;
      goalRuns.set(sessionId, refreshed);
      emit();
    } else if (outputTokens > 0 || (assistant.toolCalls?.length ?? 0) > 0) {
      refreshed.noProgressTurns = 0;
      goalRuns.set(sessionId, refreshed);
    }
  }

  if (assistant && isLowSignalAssistantTurn(assistant)) {
    const stallKey = `${sessionId}:stall`;
    const stallCount = (stallSignals.get(stallKey) ?? 0) + 1;
    stallSignals.set(stallKey, stallCount);
    if (stallCount >= 2) {
      stallSignals.delete(stallKey);
      // Automatic mode is never stopped by a stall — clear the counter and push on.
      if (agentMode !== "automatic") {
        const reason = "Stopped — assistant stalled without verifiable progress.";
        finishGoalRun(sessionId, "stopped", reason);
        return { status: "stall", reason };
      }
    }
  } else {
    stallSignals.delete(`${sessionId}:stall`);
  }

  if (shouldAutoCompleteExploratoryGoal(refreshed, assistant)) {
    const reason = "Smoke/test goal — verification recorded; marking complete.";
    finishGoalRun(sessionId, "completed", reason);
    return { status: "satisfied", reason };
  }

  const reason = monitoring
    ? refreshed.lastEvaluatorReason!
    : buildEvaluatorReason(refreshed, assistant, messages);
  refreshed.lastEvaluatorReason = reason;
  goalRuns.set(sessionId, refreshed);
  emit();
  return { status: "continue", reason };
}

export type GoalRunContinuationDecision =
  | { continue: true; reason: string; budgetWrapup?: boolean }
  | { continue: false; reason: "idle" | "completed" | "stopped" | "max_rounds" | "blocked" | "mode" | "stall" | "paused" };

export function applyGoalEvaluatorVerdict(
  sessionId: string,
  verdict: { satisfied: boolean; blocked: boolean; reason: string; source?: string },
  agentMode?: AiAgentMode,
): GoalEvaluation {
  const run = goalRuns.get(sessionId);
  if (!run || run.phase !== "running") return { status: "idle" };

  const prefix = verdict.source === "model" ? "Evaluator" : verdict.source === "heuristic" ? "Heuristic" : "Check";
  const reason = verdict.reason.trim() || "Completion condition update.";

  if (verdict.blocked) {
    // Full autonomy: an evaluator "blocked — needs user" verdict must not pause an
    // Automatic run. Convert to continue with a self-decide nudge (parity with the
    // marker path in evaluateGoalCondition).
    if (agentMode === "automatic") {
      run.lastEvaluatorReason = `${AUTOMATIC_UNBLOCK_NUDGE} (was: ${prefix}: ${reason})`;
      goalRuns.set(sessionId, run);
      emit();
      return { status: "continue", reason: run.lastEvaluatorReason };
    }
    finishGoalRun(sessionId, "blocked", `${prefix}: ${reason}`);
    return { status: "blocked", reason };
  }
  if (verdict.satisfied) {
    finishGoalRun(sessionId, "completed", `${prefix}: ${reason}`);
    return { status: "satisfied", reason };
  }

  run.lastEvaluatorReason = `${prefix}: ${reason}`;
  goalRuns.set(sessionId, run);
  emit();
  return { status: "continue", reason };
}

export function goalEvaluationToContinuation(
  evaluation: GoalEvaluation,
): GoalRunContinuationDecision {
  if (evaluation.status === "continue") {
    return { continue: true, reason: evaluation.reason };
  }
  if (evaluation.status === "budget_wrapup") {
    return { continue: true, reason: evaluation.reason, budgetWrapup: true };
  }
  if (evaluation.status === "blocked") return { continue: false, reason: "blocked" };
  if (evaluation.status === "max_rounds") return { continue: false, reason: "max_rounds" };
  if (evaluation.status === "stall") return { continue: false, reason: "stall" };
  if (evaluation.status === "paused") return { continue: false, reason: "paused" };
  if (evaluation.status === "satisfied") return { continue: false, reason: "completed" };
  return { continue: false, reason: "idle" };
}

const EVALUATOR_PENDING_REASON = "Evaluating completion condition…";

export function setGoalRunEvaluatorPending(sessionId: string, pending: boolean) {
  const run = goalRuns.get(sessionId);
  if (!run || run.phase !== "running") return;
  if (pending) {
    run.lastEvaluatorReason = EVALUATOR_PENDING_REASON;
    goalRuns.set(sessionId, run);
    emit();
  } else if (run.lastEvaluatorReason === EVALUATOR_PENDING_REASON) {
    // Clear the transient placeholder so an aborted/no-verdict evaluation doesn't leave
    // the run stuck showing "Evaluating…" forever. Only clears the placeholder — a real
    // reason written by applyGoalEvaluatorVerdict on the success path is preserved.
    run.lastEvaluatorReason = null;
    goalRuns.set(sessionId, run);
    emit();
  }
}

export function evaluateGoalRunContinuation(
  sessionId: string,
  messages: AiChatMessage[],
  agentMode: AiAgentMode,
): GoalRunContinuationDecision {
  const run = goalRuns.get(sessionId);
  if (!run || run.phase !== "running") {
    const snapshot = goalRuns.get(sessionId);
    if (snapshot?.phase === "completed") return { continue: false, reason: "completed" };
    if (snapshot?.phase === "blocked") return { continue: false, reason: "blocked" };
    if (snapshot?.phase === "max_rounds") return { continue: false, reason: "max_rounds" };
    if (snapshot?.phase === "paused") return { continue: false, reason: "paused" };
    if (snapshot?.phase === "stopped") return { continue: false, reason: "stall" };
    return { continue: false, reason: "idle" };
  }
  if (!isFullExecutionAgentMode(agentMode)) {
    finishGoalRun(sessionId, "stopped", null);
    return { continue: false, reason: "mode" };
  }

  return goalEvaluationToContinuation(evaluateGoalCondition(sessionId, messages, agentMode));
}

/** Nudge injected when Automatic mode overrides a "blocked — needs user" verdict. */
const AUTOMATIC_UNBLOCK_NUDGE =
  "Automatic mode: no user to unblock you. Choose the most reasonable option from the evidence, record it as an assumption, and continue toward the goal.";

/** Nudge injected when Automatic mode overrides a budget/stall/low-output stop. The
 *  run is fully autonomous: only task completion or the user's Stop ends it. */
const AUTOMATIC_NO_STOP_NUDGE =
  "Automatic mode: budgets are advisory and there is no user to stop you — keep working until the task is genuinely complete. Consolidate steps to stay efficient, verify with tools, and call Goal with status \"completed\" only when the completion condition is fully satisfied.";

const stallSignals = new Map<string, number>();

function shouldAutoCompleteExploratoryGoal(run: GoalRunState, assistant: AiChatMessage | null) {
  if (!isExploratoryGoalRun(run.goal)) return false;
  if (run.round < 2) return false;
  const successfulTools = assistant?.toolCalls?.filter((call) => call.status === "success").length ?? 0;
  if (successfulTools === 0) return false;
  if (run.round >= run.limits.maxRounds - 1) return true;
  return run.round >= 2 && run.progress >= 10;
}

function isLowSignalAssistantTurn(message: AiChatMessage) {
  const text = message.content.trim();
  const toolCount = message.toolCalls?.length ?? 0;
  if (toolCount > 0) return false;
  if (text.length > 400) return false;
  return /\b(done|complete|finished|готово|завершено)\b/i.test(text);
}

export function formatGoalRunElapsedMs(run: GoalRunState) {
  const end = run.completedAt ?? Date.now();
  return Math.max(0, end - run.startedAt);
}

export function formatGoalRunDuration(ms: number) {
  const totalSeconds = Math.floor(ms / 1000);
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  if (minutes <= 0) return `${seconds}s`;
  return `${minutes}m ${seconds}s`;
}

export function formatGoalRunTokenTotal(run: GoalRunState) {
  return run.promptTokens + run.completionTokens;
}