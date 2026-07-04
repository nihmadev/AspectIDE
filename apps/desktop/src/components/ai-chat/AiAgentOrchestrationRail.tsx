import { ArrowLeft, Check, ChevronRight, Circle, FileDiff, History, ListChecks, Loader2, Minus, Network, Square } from "lucide-react";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore, type CSSProperties } from "react";
import {
  cancelSubagentRun,
  getSubagentRun,
  listSubagentRunsForSession,
  subscribeSubagentRuns,
  type SubagentRun,
} from "../../lib/aiSubagentRuns";
import { formatCompactTokens } from "../../lib/aiChatContextUsage";
import { getAiSessionGoal, getAiSessionGoalsSnapshot, subscribeAiSessionGoals } from "../../lib/aiSessionGoal";
import {
  formatGoalRunDuration,
  formatGoalRunElapsedMs,
  formatGoalRunTokenTotal,
  getDisplayGoalRun,
  getAiSessionGoalRunsSnapshot,
  getGoalRunEvaluatorReason,
  subscribeAiSessionGoalRuns,
} from "../../lib/aiSessionGoalRun";
import { getAiSessionTodosSnapshot, listAiSessionTodos, subscribeAiSessionTodos, type AiSessionTodoStatus } from "../../lib/aiSessionTodos";
import {
  getPendingFileReviewsSnapshot,
  listPendingFileReviewsForSession,
  subscribePendingFileReviews,
} from "../../lib/aiPendingFileReview";
import { openWorkspaceEditorPath } from "../../lib/openWorkspaceEditorPath";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { isFullExecutionAgentMode, type AiPreferences } from "../../lib/aiPreferences";
import { resolveMaxParallelSubagents } from "../../lib/aiSubagentPolicy";
import type { AiChatSessionStatus } from "../../lib/store";
import { AiAgentNowBar } from "./AiAgentNowBar";

type AiAgentOrchestrationRailProps = {
  sessionId: string;
  agentMode: string;
  sessionStatus: AiChatSessionStatus;
  preferences: AiPreferences;
  t: TranslateFn;
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
};

export function AiAgentOrchestrationRail({ sessionId, agentMode, sessionStatus, preferences, t, collapsed, onToggleCollapsed }: AiAgentOrchestrationRailProps) {
  if (!isFullExecutionAgentMode(agentMode)) return null;
  return (
    <AiAgentOrchestrationRailBody
      sessionId={sessionId}
      sessionStatus={sessionStatus}
      preferences={preferences}
      t={t}
      collapsed={collapsed}
      onToggleCollapsed={onToggleCollapsed}
    />
  );
}

function AiAgentOrchestrationRailBody({
  sessionId,
  sessionStatus,
  preferences,
  t,
  collapsed = false,
  onToggleCollapsed,
}: {
  sessionId: string;
  sessionStatus: AiChatSessionStatus;
  preferences: AiPreferences;
  t: TranslateFn;
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
}) {
  useSyncExternalStore(subscribeAiSessionGoals, getAiSessionGoalsSnapshot, getAiSessionGoalsSnapshot);
  useSyncExternalStore(subscribeAiSessionGoalRuns, getAiSessionGoalRunsSnapshot, getAiSessionGoalRunsSnapshot);
  useSyncExternalStore(subscribeAiSessionTodos, getAiSessionTodosSnapshot, getAiSessionTodosSnapshot);
  const subagentRunsSnapshot = useCallback(() => subagentRunsSignature(sessionId), [sessionId]);
  useSyncExternalStore(subscribeSubagentRuns, subagentRunsSnapshot, subagentRunsSnapshot);
  useSyncExternalStore(subscribePendingFileReviews, getPendingFileReviewsSnapshot, getPendingFileReviewsSnapshot);

  const [selectedSubagentId, setSelectedSubagentId] = useState<string | null>(null);
  useEffect(() => {
    setSelectedSubagentId(null);
  }, [sessionId]);
  const maxParallelSubagents = resolveMaxParallelSubagents(preferences);

  const goal = getAiSessionGoal(sessionId);
  const goalRun = getDisplayGoalRun(sessionId);
  const evaluatorReason = getGoalRunEvaluatorReason(sessionId);
  const todos = listAiSessionTodos(sessionId);
  const fileReviews = listPendingFileReviewsForSession(sessionId);
  const allSubagentRuns = listSubagentRunsForSession(sessionId);
  // Live list: running now, or finished within the last 5 minutes.
  const subagentRuns = allSubagentRuns.filter(
    (run) => run.status === "running" || Date.now() - (run.endedAt ?? run.startedAt) < 300_000,
  );
  // Finished runs that aged out of the live window — this session's history
  // (the store keeps the last 32 finished runs, so this is already bounded).
  const liveIds = new Set(subagentRuns.map((run) => run.id));
  const historyRuns = allSubagentRuns.filter((run) => !liveIds.has(run.id));
  const runningSubagents = subagentRuns.filter((run) => run.status === "running").length;
  const completedTodos = todos.filter((todo) => todo.status === "completed").length;
  const [historyOpen, setHistoryOpen] = useState(false);
  // 1s heartbeat while any subagent runs so the per-row elapsed clocks stay live.
  const [, setClockTick] = useState(0);
  useEffect(() => {
    if (runningSubagents === 0) return;
    const timer = setInterval(() => setClockTick((tick) => tick + 1), 1000);
    return () => clearInterval(timer);
  }, [runningSubagents]);
  // The island is an overlay: it only earns screen space when the agent has
  // actually pinned a goal, opened tasks, produced reviews or spawned subagents
  // (history counts — finished runs stay reachable).
  const hasGoalContent = Boolean(goal) || Boolean(goalRun);
  const hasContent = hasGoalContent || todos.length > 0 || fileReviews.length > 0 || allSubagentRuns.length > 0;

  // Finished subagents are hidden once older than the 5-min window (see filter above), but that
  // boundary is only evaluated at render time. Schedule a refresh at the soonest expiry so the row
  // disappears on time instead of lingering until an unrelated re-render. Keyed on a stable string
  // (not the rebuilt array) to avoid a render loop.
  const [, forceExpiryTick] = useState(0);
  const finishedExpiryKey = subagentRuns
    .filter((run) => run.status !== "running")
    .map((run) => run.endedAt ?? run.startedAt)
    .join(",");
  useEffect(() => {
    if (!finishedExpiryKey) return;
    const timestamps = finishedExpiryKey.split(",").map(Number);
    const soonest = Math.min(...timestamps.map((at) => 300_000 - (Date.now() - at)));
    if (soonest <= 0) {
      forceExpiryTick((tick) => tick + 1);
      return;
    }
    const timer = setTimeout(() => forceExpiryTick((tick) => tick + 1), soonest + 50);
    return () => clearTimeout(timer);
  }, [finishedExpiryKey]);
  const selectedSubagentCandidate = selectedSubagentId ? getSubagentRun(selectedSubagentId) : null;
  const selectedSubagent =
    selectedSubagentCandidate && selectedSubagentCandidate.sessionId === sessionId ? selectedSubagentCandidate : null;

  // Nothing to show — render nothing at all (no empty shell, no collapsed chip).
  // An open subagent transcript pins the card even if its run row already aged
  // out of the 5-minute list, so the panel never vanishes mid-read.
  if (!hasContent && !selectedSubagent) return null;

  if (collapsed) {
    const busy = sessionStatus !== "idle" && sessionStatus !== "error";
    const taskCount = todos.length;
    const doneCount = completedTodos;
    return (
      <aside
        className="ai-agent-orchestration-rail ai-agent-rail-collapsed"
        data-collapsed="true"
        onClick={onToggleCollapsed}
        role="button"
        tabIndex={0}
        aria-label="Agent panel collapsed — click to expand goal, tasks and status"
        title="Click to expand Agent status, Goal and Tasks"
      >
        <div className="ai-agent-rail-mini">
          <div className="ai-agent-mini-icon" data-busy={busy || undefined}>
            {busy ? <Loader2 size={11} className="spin-icon" /> : <Check size={11} />}
          </div>
          <div className="ai-agent-mini-label">AI</div>
          {taskCount > 0 && (
            <div className="ai-agent-mini-tasks" title={`${doneCount}/${taskCount} tasks`}>
              {doneCount}/{taskCount}
            </div>
          )}
          <ChevronRight size={8} className="ai-agent-mini-expand" />
        </div>
      </aside>
    );
  }

  // A selected subagent takes over the island: its own window with live status,
  // transcript and summary. Back returns to the overview.
  if (selectedSubagent) {
    return <SubagentWindow run={selectedSubagent} t={t} onBack={() => setSelectedSubagentId(null)} />;
  }

  return (
    <aside className="ai-agent-orchestration-rail" aria-label={t("aiChat.orchestration.aria")}>
      <button
        type="button"
        className="ai-agent-rail-collapse"
        onClick={(e) => { e.stopPropagation(); onToggleCollapsed?.(); }}
        title="Collapse agent island (frees chat space)"
        aria-label="Collapse"
      >
        <Minus size={9} />
      </button>
      <AiAgentNowBar sessionId={sessionId} sessionStatus={sessionStatus} t={t} />

      {hasGoalContent && (
      <div className="ai-agent-rail-goal-dock" aria-label={t("aiChat.orchestration.goalTitle")}>
        <div className="ai-agent-goal-island" data-empty={goal ? undefined : true}>
          <span className="ai-agent-goal-island-label">{t("aiChat.orchestration.goalTitle")}</span>
          {goal ? (
            <p className="ai-agent-goal-island-text" title={goal}>{goal}</p>
          ) : (
            <p className="ai-agent-goal-island-hint">
              {t("aiChat.orchestration.goalEmpty")}{" "}
              <code className="ai-agent-goal-island-cmd">/goal</code>
            </p>
          )}
          {goalRun && (
            <div className="ai-agent-goal-run-meter" data-phase={goalRun.phase} data-paused={goalRun.phase === "paused" || undefined}>
              <div className="ai-agent-goal-run-meter-head">
                <span>{t("aiChat.orchestration.goalProgress", { progress: goalRun.progress })}</span>
                <span>{goalRun.round}/{goalRun.limits.maxRounds}</span>
              </div>
              <div className="ai-agent-goal-run-track" aria-hidden="true">
                <div className="ai-agent-goal-run-fill" style={{ width: `${Math.max(4, goalRun.progress)}%` }} />
              </div>
              <div className="ai-agent-goal-run-stats">
                <span>{formatGoalRunDuration(formatGoalRunElapsedMs(goalRun))}</span>
                <span>{formatCompactTokens(formatGoalRunTokenTotal(goalRun))}</span>
              </div>
              {goalRun.limits.maxTokens > 0 && (() => {
                const spent = formatGoalRunTokenTotal(goalRun);
                const pct = Math.min(100, Math.round((spent / goalRun.limits.maxTokens) * 100));
                const tone = pct >= 90 ? "high" : pct >= 70 ? "medium" : "low";
                return (
                  <div className="ai-agent-goal-budget" data-tone={tone} title={t("aiChat.orchestration.budgetTitle", { spent: formatCompactTokens(spent), total: formatCompactTokens(goalRun.limits.maxTokens) })}>
                    <div className="ai-agent-goal-budget-track" aria-hidden="true">
                      <div className="ai-agent-goal-budget-fill" style={{ width: `${Math.max(2, pct)}%` }} />
                    </div>
                    <span className="ai-agent-goal-budget-label">{t("aiChat.orchestration.budgetLabel", { pct })}</span>
                  </div>
                );
              })()}
              {goalRun.lastCheckpoint && (
                <p className="ai-agent-goal-checkpoint" title={goalRun.lastCheckpoint.summary}>
                  {t("aiChat.orchestration.goalCheckpoint", { summary: goalRun.lastCheckpoint.summary })}
                </p>
              )}
              {evaluatorReason && (goalRun.phase === "running" || goalRun.phase === "paused") && (
                <p className="ai-agent-goal-evaluator" title={evaluatorReason}>
                  {t("aiChat.orchestration.evaluatorReason", { reason: evaluatorReason })}
                </p>
              )}
            </div>
          )}
        </div>
      </div>
      )}

      <div className="ai-agent-rail-scroll">
      {todos.length > 0 && (
      <section className="ai-agent-rail-block ai-agent-rail-block-compact" data-block="tasks">
        <header>
          <ListChecks size={12} />
          <strong>{t("aiChat.orchestration.tasksTitle")}</strong>
          <span className="ai-agent-rail-meta">{completedTodos}/{todos.length}</span>
        </header>
        <ul className="ai-agent-rail-tasks ai-agent-rail-tasks-compact">
          {todos.map((todo) => (
            <li key={todo.id} data-status={todo.status}>
              <TaskStatusGlyph status={todo.status} />
              <span className="ai-agent-rail-task-text" title={todo.content}>{todo.content}</span>
              {todo.linkedFilePath && (
                <button
                  type="button"
                  className="ai-agent-rail-task-file"
                  onClick={() => void openWorkspaceEditorPath(todo.linkedFilePath!)}
                  title={todo.linkedFilePath}
                >
                  {basename(todo.linkedFilePath)}
                </button>
              )}
            </li>
          ))}
        </ul>
      </section>
      )}

      {fileReviews.length > 0 && (
        <section className="ai-agent-rail-block ai-agent-rail-block-compact" data-block="reviews">
          <header>
            <FileDiff size={12} />
            <strong>{t("aiChat.orchestration.reviewsTitle")}</strong>
            <span className="ai-agent-rail-meta">{fileReviews.length}</span>
          </header>
          <ul className="ai-agent-rail-reviews ai-agent-rail-reviews-compact">
            {fileReviews.map((review) => (
              <li key={review.id}>
                <button type="button" onClick={() => void openWorkspaceEditorPath(review.path)}>
                  <span>{review.relativePath || review.path}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      )}

      {subagentRuns.length > 0 && (
      <section className="ai-agent-rail-block ai-agent-rail-block-compact" data-block="subagents">
        <header>
          <Network size={12} />
          <strong>{t("aiChat.orchestration.subagentsTitle")}</strong>
          <span className="ai-agent-rail-meta">{runningSubagents}/{maxParallelSubagents}</span>
        </header>
        <ul className="ai-subagent-tree ai-agent-rail-subagents">
          {buildSubagentTree(subagentRuns).map((node) => (
            <SubagentRailRow key={node.run.id} node={node} depth={0} t={t} onSelect={setSelectedSubagentId} />
          ))}
        </ul>
      </section>
      )}

      {historyRuns.length > 0 && (
      <section className="ai-agent-rail-block ai-agent-rail-block-compact" data-block="subagent-history">
        <button
          type="button"
          className="ai-agent-rail-history-toggle"
          onClick={() => setHistoryOpen((open) => !open)}
          aria-expanded={historyOpen}
        >
          <History size={12} />
          <strong>{t("aiChat.subagents.history")}</strong>
          <span className="ai-agent-rail-meta">{historyRuns.length}</span>
          <ChevronRight size={10} className="ai-agent-rail-history-caret" data-open={historyOpen || undefined} />
        </button>
        {historyOpen && (
          <ul className="ai-subagent-tree ai-agent-rail-subagents">
            {historyRuns.map((run) => (
              <SubagentRailRow key={run.id} node={{ run, children: [] }} depth={0} t={t} onSelect={setSelectedSubagentId} />
            ))}
          </ul>
        )}
      </section>
      )}
      </div>
    </aside>
  );
}

function TaskStatusGlyph({ status }: { status: AiSessionTodoStatus }) {
  if (status === "completed") return <Check size={11} aria-hidden="true" />;
  if (status === "in_progress") return <Loader2 size={11} className="spin-icon" aria-hidden="true" />;
  if (status === "blocked" || status === "cancelled") return <Circle size={11} data-muted="true" aria-hidden="true" />;
  return <Circle size={11} aria-hidden="true" />;
}

type SubagentTreeNode = { run: SubagentRun; children: SubagentTreeNode[] };

/** Compact digital clock for run durations: `0:07`, `3:05`, `1:02:33`. */
function formatSubagentClock(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const seconds = String(total % 60).padStart(2, "0");
  if (hours > 0) return `${hours}:${String(minutes).padStart(2, "0")}:${seconds}`;
  return `${minutes}:${seconds}`;
}

/** Right-aligned row status: live elapsed while running, total duration when
 *  done, a short word for failed/cancelled. */
function subagentRowStatus(run: SubagentRun, t: TranslateFn): string {
  if (run.status === "running") return formatSubagentClock(Date.now() - run.startedAt);
  if (run.status === "completed") return formatSubagentClock((run.endedAt ?? run.startedAt) - run.startedAt);
  return t(`aiChat.subagents.status.${run.status}` as "aiChat.subagents.status.failed");
}

function SubagentRailRow({
  node,
  depth,
  t,
  onSelect,
}: {
  node: SubagentTreeNode;
  depth: number;
  t: TranslateFn;
  onSelect: (id: string) => void;
}) {
  const { run } = node;
  return (
    <li data-status={run.status} style={{ "--subagent-depth": depth } as CSSProperties}>
      <div className="ai-subagent-row-line">
        <button
          type="button"
          className="ai-subagent-panel-row-button ai-subagent-rail-row"
          onClick={() => onSelect(run.id)}
          title={run.description}
        >
          <span className="ai-subagent-status-dot" data-status={run.status} aria-hidden="true" />
          <span className="ai-subagent-panel-type">{run.subagentType}</span>
          <span className="ai-subagent-panel-desc">{run.description}</span>
          <span className="ai-subagent-row-status">{subagentRowStatus(run, t)}</span>
        </button>
        {run.status === "running" && (
          <button type="button" className="ai-subagent-cancel" title={t("aiChat.subagents.cancel")} onClick={() => cancelSubagentRun(run.id)}>
            <Square size={9} />
          </button>
        )}
      </div>
      {node.children.length > 0 && (
        <ul>
          {node.children.map((child) => (
            <SubagentRailRow key={child.run.id} node={child} depth={depth + 1} t={t} onSelect={onSelect} />
          ))}
        </ul>
      )}
    </li>
  );
}

/**
 * Dedicated subagent window: the island drills into one run — live status dot,
 * ticking clock, streamed transcript (auto-follows the bottom) and the final
 * summary. Back returns to the Goal/Tasks/Subagents overview.
 */
function SubagentWindow({ run, t, onBack }: { run: SubagentRun; t: TranslateFn; onBack: () => void }) {
  // Tick the clock once a second while the run is live.
  const [, setClockTick] = useState(0);
  useEffect(() => {
    if (run.status !== "running") return;
    const timer = setInterval(() => setClockTick((tick) => tick + 1), 1000);
    return () => clearInterval(timer);
  }, [run.status]);
  // Follow the live stream: pin the scroll to the bottom on every new entry.
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    const el = transcriptRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [run.revision]);

  const elapsed = formatSubagentClock((run.endedAt ?? Date.now()) - run.startedAt);
  const summary = run.summary.trim();
  // The live transcript usually ends with the same final answer the summary
  // carries — only render the summary block when it adds new text.
  const showSummary = Boolean(summary) && summary !== run.transcript.at(-1)?.content.trim();

  return (
    <aside className="ai-agent-orchestration-rail ai-subagent-window" aria-label={t("aiChat.subagents.aria")}>
      <header className="ai-subagent-window-head">
        <button type="button" className="ai-subagent-window-back" onClick={onBack} title={t("aiChat.subagents.back")} aria-label={t("aiChat.subagents.back")}>
          <ArrowLeft size={12} />
        </button>
        <span className="ai-subagent-status-dot" data-status={run.status} aria-hidden="true" />
        <span className="ai-subagent-panel-type">{run.subagentType}</span>
        <span className="ai-subagent-window-clock">{elapsed}</span>
        {run.status === "running" ? (
          <button type="button" className="ai-subagent-cancel" title={t("aiChat.subagents.cancel")} onClick={() => cancelSubagentRun(run.id)}>
            <Square size={9} />
          </button>
        ) : (
          <span className="ai-subagent-window-state" data-status={run.status}>
            {t(`aiChat.subagents.status.${run.status}` as "aiChat.subagents.status.completed")}
          </span>
        )}
      </header>
      <p className="ai-subagent-window-title" title={run.description}>{run.description}</p>
      <div className="ai-subagent-window-transcript" ref={transcriptRef}>
        {run.transcript.length === 0 && (
          <p className="ai-subagent-window-empty">{t("aiChat.subagents.transcriptEmpty")}</p>
        )}
        {run.transcript.map((entry) => (
          <article key={entry.id} data-role={entry.role}>
            <p>{entry.content}</p>
          </article>
        ))}
        {showSummary && (
          <article data-role="summary">
            <strong>{t("aiChat.subagents.summary")}</strong>
            <p>{summary}</p>
          </article>
        )}
      </div>
    </aside>
  );
}

/**
 * Value-stable snapshot of this session's subagent runs for useSyncExternalStore.
 * Returns a primitive string that changes on spawn/status/transcript-append/complete/cancel,
 * so React value-compares it and re-renders on live subagent updates (but stays equal when
 * nothing changed, avoiding an infinite re-render loop).
 */
function subagentRunsSignature(sessionId: string): string {
  return listSubagentRunsForSession(sessionId)
    .map((run) => `${run.id}:${run.status}:${run.revision}:${run.endedAt ?? ""}`)
    .join("|");
}

function buildSubagentTree(runs: SubagentRun[]): SubagentTreeNode[] {
  const byId = new Map(runs.map((run) => [run.id, { run, children: [] as SubagentTreeNode[] }]));
  const roots: SubagentTreeNode[] = [];
  for (const run of runs) {
    const node = byId.get(run.id);
    if (!node) continue;
    const parent = run.parentAgentId ? byId.get(run.parentAgentId) : null;
    if (parent) parent.children.push(node);
    else roots.push(node);
  }
  return roots;
}

function basename(path: string) {
  const normalized = path.replace(/\\/g, "/");
  const parts = normalized.split("/");
  return parts[parts.length - 1] || path;
}