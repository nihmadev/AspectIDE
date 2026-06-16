import { Check, ChevronRight, Circle, FileDiff, ListChecks, Loader2, Minus, Network, Square, X } from "lucide-react";
import { useCallback, useEffect, useState, useSyncExternalStore, type CSSProperties } from "react";
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
  const subagentRuns = listSubagentRunsForSession(sessionId).filter(
    (run) => run.status === "running" || Date.now() - (run.endedAt ?? run.startedAt) < 300_000,
  );
  const runningSubagents = subagentRuns.filter((run) => run.status === "running").length;
  const completedTodos = todos.filter((todo) => todo.status === "completed").length;

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

      <div className="ai-agent-rail-scroll">
      <section className="ai-agent-rail-block ai-agent-rail-block-compact" data-block="tasks">
        <header>
          <ListChecks size={12} />
          <strong>{t("aiChat.orchestration.tasksTitle")}</strong>
          <span className="ai-agent-rail-badge">{t("aiChat.orchestration.aiManaged")}</span>
          {todos.length > 0 && (
            <span className="ai-agent-rail-meta">{completedTodos}/{todos.length}</span>
          )}
        </header>
        {todos.length === 0 ? (
          <p className="ai-agent-rail-empty">{t("aiChat.orchestration.tasksEmpty")}</p>
        ) : (
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
        )}
      </section>

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

      <section className="ai-agent-rail-block ai-agent-rail-block-compact" data-block="subagents">
        <header>
          <Network size={12} />
          <strong>{t("aiChat.orchestration.subagentsTitle")}</strong>
          <span className="ai-agent-rail-meta">{runningSubagents}/{maxParallelSubagents}</span>
        </header>
        {subagentRuns.length === 0 ? (
          <p className="ai-agent-rail-empty">{t("aiChat.orchestration.subagentsEmpty")}</p>
        ) : (
          <ul className="ai-subagent-tree ai-agent-rail-subagents ai-agent-rail-subagents-compact">
            {buildSubagentTree(subagentRuns).map((node) => (
              <SubagentRailRow
                key={node.run.id}
                node={node}
                depth={0}
                selectedId={selectedSubagentId}
                t={t}
                onSelect={setSelectedSubagentId}
              />
            ))}
          </ul>
        )}
      </section>

      {selectedSubagent && (
        <section className="ai-agent-rail-transcript" aria-label={t("aiChat.orchestration.transcriptAria")}>
          <header>
            <strong>{selectedSubagent.description}</strong>
            <button type="button" className="ai-agent-rail-transcript-close" onClick={() => setSelectedSubagentId(null)}>
              <X size={12} />
            </button>
          </header>
          <div className="ai-agent-rail-transcript-body">
            {selectedSubagent.transcript.map((entry) => (
              <article key={entry.id} data-role={entry.role}>
                <p>{entry.content}</p>
              </article>
            ))}
            {selectedSubagent.summary && <p className="ai-agent-rail-transcript-summary">{selectedSubagent.summary}</p>}
          </div>
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

function SubagentRailRow({
  node,
  depth,
  selectedId,
  t,
  onSelect,
}: {
  node: SubagentTreeNode;
  depth: number;
  selectedId: string | null;
  t: TranslateFn;
  onSelect: (id: string) => void;
}) {
  const { run } = node;
  return (
    <li data-status={run.status} data-selected={selectedId === run.id || undefined} style={{ "--subagent-depth": depth } as CSSProperties}>
      <button type="button" className="ai-subagent-panel-row ai-subagent-panel-row-button" onClick={() => onSelect(run.id)}>
        <span className="ai-subagent-panel-type">{run.subagentType}</span>
        <span className="ai-subagent-panel-desc" title={run.description}>{run.description}</span>
      </button>
      {run.status === "running" && (
        <button type="button" className="ai-subagent-cancel" title={t("aiChat.subagents.cancel")} onClick={() => cancelSubagentRun(run.id)}>
          <Square size={10} />
        </button>
      )}
      {node.children.length > 0 && (
        <ul>
          {node.children.map((child) => (
            <SubagentRailRow key={child.run.id} node={child} depth={depth + 1} selectedId={selectedId} t={t} onSelect={onSelect} />
          ))}
        </ul>
      )}
    </li>
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