import { ArrowLeft, Ban, Check, ChevronRight, Circle, GripHorizontal, History, ListChecks, Loader2, Maximize2, Minus, Network, Pin, Square, Trash2, X, XCircle } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, useSyncExternalStore, type CSSProperties, type KeyboardEvent as ReactKeyboardEvent, type PointerEvent as ReactPointerEvent } from "react";
import { createPortal } from "react-dom";
import {
  cancelSubagentRun,
  clearFinishedSubagentRuns,
  getSubagentRun,
  listSubagentRunsForSession,
  removeSubagentRun,
  subscribeSubagentRuns,
  type SubagentRun,
} from '../../lib/aspector/subagents/runs';
import { formatCompactTokens } from '../../lib/aspector/chat/context-usage';
import { getAiSessionGoal, getAiSessionGoalsSnapshot, subscribeAiSessionGoals } from '../../lib/aspector/session/goal/session-goal';
import {
  formatGoalRunDuration,
  formatGoalRunElapsedMs,
  formatGoalRunTokenTotal,
  getDisplayGoalRun,
  getAiSessionGoalRunsSnapshot,
  getGoalRunEvaluatorReason,
  subscribeAiSessionGoalRuns,
} from '../../lib/aspector/session/goal/session-goal-run';
import { getAiSessionTodosSnapshot, listAiSessionTodos, subscribeAiSessionTodos, type AiSessionTodoStatus } from '../../lib/aspector/session/todos';
import { openWorkspaceEditorPath } from '../../lib/editor/open-workspace-editor-path';
import type { TranslateFn } from '../../lib/i18n/useTranslation';
import { isFullExecutionAgentMode, type AiPreferences } from '../../lib/aspector/utils/preferences';
import { resolveMaxParallelSubagents } from '../../lib/aspector/subagents/policy';
import type { AiChatSessionStatus } from '../../lib/store/index';
import { AspectorAgentNowBar } from "./AspectorAgentNowBar";

const ISLAND_FLOAT_POS_KEY = "aspect.agentIsland.floatPos";

type IslandFloatPos = { x: number; y: number };

/** Clamp a floating island position so the card stays fully inside its parent. */
export function clampAgentIslandPos(
  x: number,
  y: number,
  parentWidth: number,
  parentHeight: number,
  railWidth: number,
  railHeight: number,
): IslandFloatPos {
  const maxX = Math.max(0, parentWidth - railWidth);
  const maxY = Math.max(0, parentHeight - railHeight);
  return { x: Math.min(Math.max(0, x), maxX), y: Math.min(Math.max(0, y), maxY) };
}

function loadIslandFloatPos(): IslandFloatPos | null {
  if (typeof window === "undefined") return null;
  try {
    const raw = window.localStorage.getItem(ISLAND_FLOAT_POS_KEY);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<IslandFloatPos> | null;
    if (parsed && Number.isFinite(parsed.x) && Number.isFinite(parsed.y)) {
      return { x: parsed.x as number, y: parsed.y as number };
    }
  } catch {
    // Corrupted entry вЂ” fall back to the docked position.
  }
  return null;
}

function saveIslandFloatPos(pos: IslandFloatPos | null) {
  if (typeof window === "undefined") return;
  try {
    if (pos) window.localStorage.setItem(ISLAND_FLOAT_POS_KEY, JSON.stringify({ x: Math.round(pos.x), y: Math.round(pos.y) }));
    else window.localStorage.removeItem(ISLAND_FLOAT_POS_KEY);
  } catch {
    // Storage unavailable вЂ” the position simply won't survive a restart.
  }
}

type IslandDrag = {
  /** Callback ref вЂ” MUST be attached to whichever <aside> currently renders,
   *  so observers re-attach when the overview/subagent-window asides swap. */
  railRef: (el: HTMLElement | null) => void;
  /** True once the island is detached from the right-wall dock. */
  floating: boolean;
  /** Inline overrides that place the detached island; undefined while docked. */
  floatStyle: CSSProperties | undefined;
  onGripPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
  onGripPointerMove: (event: ReactPointerEvent<HTMLElement>) => void;
  onGripPointerEnd: (event: ReactPointerEvent<HTMLElement>) => void;
  /** Keyboard equivalent of dragging: Enter/Space detaches-in-place or redocks;
   *  arrows nudge a floating island (Shift = larger step). */
  onGripKeyDown: (event: ReactKeyboardEvent<HTMLElement>) => void;
  redock: () => void;
};

/**
 * Detachable island: dragging the grip pops the card out of its right-wall
 * dock into a free position, clamped to the chat area (.ai-chat-main вЂ” the
 * rail's direct DOM parent) so it can never escape the Agent panel. The
 * position persists across sessions; the Pin button snaps it back to the dock.
 *
 * The rail ref is a callback ref feeding state, because the <aside> mounts,
 * unmounts and swaps identity (hidden в†’ overview в†’ subagent window) while this
 * hook lives on вЂ” a plain ref + mount-time effect would attach the clamp
 * observers once (or never) and go permanently stale.
 */
function useIslandDrag(): IslandDrag {
  const railElRef = useRef<HTMLElement | null>(null);
  const [railEl, setRailEl] = useState<HTMLElement | null>(null);
  const railRef = useCallback((el: HTMLElement | null) => {
    railElRef.current = el;
    setRailEl(el);
  }, []);
  const [floatPos, setFloatPos] = useState<IslandFloatPos | null>(loadIslandFloatPos);
  const floatPosRef = useRef(floatPos);
  floatPosRef.current = floatPos;
  const dragState = useRef<{ pointerId: number; grabX: number; grabY: number } | null>(null);
  const floating = floatPos !== null;

  const clampToParent = useCallback((x: number, y: number): IslandFloatPos | null => {
    const rail = railElRef.current;
    // The narrow-panel container query hides the rail (display:none) вЂ” its
    // sizes read 0 then, so clamping would corrupt the stored position.
    // The rail's own ResizeObserver re-clamps once it becomes visible again.
    const parent = rail?.parentElement;
    if (!rail || !parent || rail.offsetWidth === 0) return null;
    return clampAgentIslandPos(x, y, parent.clientWidth, parent.clientHeight, rail.offsetWidth, rail.offsetHeight);
  }, []);

  const onGripPointerDown = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    if (event.button !== 0) return;
    // Buttons inside the drag surface (back / cancel / collapse) keep their clicks.
    if ((event.target as HTMLElement).closest("button, a, input, textarea, select")) return;
    const rail = railElRef.current;
    if (!rail) return;
    const rect = rail.getBoundingClientRect();
    dragState.current = { pointerId: event.pointerId, grabX: event.clientX - rect.left, grabY: event.clientY - rect.top };
    event.currentTarget.setPointerCapture(event.pointerId);
    event.preventDefault();
  }, []);

  const onGripPointerMove = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    const state = dragState.current;
    if (!state || state.pointerId !== event.pointerId) return;
    const parent = railElRef.current?.parentElement;
    if (!parent) return;
    const parentRect = parent.getBoundingClientRect();
    const next = clampToParent(event.clientX - parentRect.left - state.grabX, event.clientY - parentRect.top - state.grabY);
    if (next) setFloatPos(next);
  }, [clampToParent]);

  const onGripPointerEnd = useCallback((event: ReactPointerEvent<HTMLElement>) => {
    const state = dragState.current;
    if (!state || state.pointerId !== event.pointerId) return;
    dragState.current = null;
    saveIslandFloatPos(floatPosRef.current);
  }, []);

  const redock = useCallback(() => {
    dragState.current = null;
    setFloatPos(null);
    saveIslandFloatPos(null);
  }, []);

  // Keyboard-accessible detach: pop the docked card out at its current on-screen
  // spot (no jump), or redock if already floating вЂ” the keyboard twin of dragging
  // the grip, so the feature isn't pointer-only (WCAG 2.1.1).
  const toggleFloat = useCallback(() => {
    if (floatPosRef.current) {
      setFloatPos(null);
      saveIslandFloatPos(null);
      return;
    }
    const rail = railElRef.current;
    const parent = rail?.parentElement;
    if (!rail || !parent) return;
    const rect = rail.getBoundingClientRect();
    const parentRect = parent.getBoundingClientRect();
    const next = clampToParent(rect.left - parentRect.left, rect.top - parentRect.top);
    if (next) {
      setFloatPos(next);
      saveIslandFloatPos(next);
    }
  }, [clampToParent]);

  const onGripKeyDown = useCallback((event: ReactKeyboardEvent<HTMLElement>) => {
    // Don't hijack keys meant for a focused button inside the drag surface.
    if ((event.target as HTMLElement).closest("button, a, input, textarea, select")) return;
    if (event.key === "Enter" || event.key === " " || event.key === "Spacebar") {
      event.preventDefault();
      toggleFloat();
      return;
    }
    const pos = floatPosRef.current;
    if (!pos) return; // arrows only nudge an already-detached island
    const step = event.shiftKey ? 24 : 8;
    let dx = 0;
    let dy = 0;
    if (event.key === "ArrowLeft") dx = -step;
    else if (event.key === "ArrowRight") dx = step;
    else if (event.key === "ArrowUp") dy = -step;
    else if (event.key === "ArrowDown") dy = step;
    else return;
    event.preventDefault();
    const next = clampToParent(pos.x + dx, pos.y + dy);
    if (next) {
      setFloatPos(next);
      saveIslandFloatPos(next);
    }
  }, [toggleFloat, clampToParent]);

  // Keep a detached island inside the chat area: clamp synchronously on every
  // rail (re)mount вЂ” before paint, so a restored stale position never flashes
  // off-screen вЂ” then watch BOTH boxes. The parent resizes on window/panel
  // changes; the rail itself grows when tasks/subagents stream in or the
  // overview swaps with the (taller) subagent window, and each growth can push
  // a bottom-parked card past the parent's overflow:hidden edge.
  useLayoutEffect(() => {
    if (!floating || !railEl) return;
    const parent = railEl.parentElement;
    if (!parent) return;
    const clamp = () => {
      setFloatPos((pos) => {
        if (!pos) return pos;
        const next = clampToParent(pos.x, pos.y);
        return next && (next.x !== pos.x || next.y !== pos.y) ? next : pos;
      });
    };
    clamp();
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(clamp);
    observer.observe(parent);
    observer.observe(railEl);
    return () => observer.disconnect();
  }, [floating, railEl, clampToParent]);

  const floatStyle = floatPos
    ? ({ top: floatPos.y, left: floatPos.x, right: "auto", bottom: "auto", transform: "none" } as CSSProperties)
    : undefined;

  return { railRef, floating, floatStyle, onGripPointerDown, onGripPointerMove, onGripPointerEnd, onGripKeyDown, redock };
}

type AspectorAgentOrchestrationRailProps = {
  sessionId: string;
  agentMode: string;
  sessionStatus: AiChatSessionStatus;
  preferences: AiPreferences;
  t: TranslateFn;
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
};

export function AspectorAgentOrchestrationRail({ sessionId, agentMode, sessionStatus, preferences, t, collapsed, onToggleCollapsed }: AspectorAgentOrchestrationRailProps) {
  if (!isFullExecutionAgentMode(agentMode)) return null;
  return (
    <AspectorAgentOrchestrationRailBody
      sessionId={sessionId}
      sessionStatus={sessionStatus}
      preferences={preferences}
      t={t}
      collapsed={collapsed}
      onToggleCollapsed={onToggleCollapsed}
    />
  );
}

function AspectorAgentOrchestrationRailBody({
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
  const drag = useIslandDrag();

  const [selectedSubagentId, setSelectedSubagentId] = useState<string | null>(null);
  useEffect(() => {
    setSelectedSubagentId(null);
  }, [sessionId]);
  const maxParallelSubagents = resolveMaxParallelSubagents(preferences);

  const goal = getAiSessionGoal(sessionId);
  const goalRun = getDisplayGoalRun(sessionId);
  const evaluatorReason = getGoalRunEvaluatorReason(sessionId);
  const todos = listAiSessionTodos(sessionId);
  const allSubagentRuns = listSubagentRunsForSession(sessionId);
  // Live list: running now, or finished within the last 5 minutes.
  const subagentRuns = allSubagentRuns.filter(
    (run) => run.status === "running" || Date.now() - (run.endedAt ?? run.startedAt) < 300_000,
  );
  // Finished runs that aged out of the live window вЂ” this session's history
  // (the store keeps the last 32 finished runs, so this is already bounded).
  const liveIds = new Set(subagentRuns.map((run) => run.id));
  const historyRuns = allSubagentRuns.filter((run) => !liveIds.has(run.id));
  const runningSubagents = subagentRuns.filter((run) => run.status === "running").length;
  const completedTodos = todos.filter((todo) => todo.status === "completed").length;
  const activeTodos = todos.filter((todo) => todo.status === "in_progress").length;
  const [historyOpen, setHistoryOpen] = useState(false);
  // 1s heartbeat while any subagent runs so the per-row elapsed clocks stay live.
  const [, setClockTick] = useState(0);
  useEffect(() => {
    if (runningSubagents === 0) return;
    const timer = setInterval(() => setClockTick((tick) => tick + 1), 1000);
    return () => clearInterval(timer);
  }, [runningSubagents]);
  // The island is an overlay: it only earns screen space when the agent has
  // actually pinned a goal, opened tasks or spawned subagents (history counts вЂ”
  // finished runs stay reachable). File reviews live in the chat's review bar
  // and the editor diff, not here.
  const hasGoalContent = Boolean(goal) || Boolean(goalRun);
  const hasContent = hasGoalContent || todos.length > 0 || allSubagentRuns.length > 0;

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

  // Nothing to show вЂ” render nothing at all (no empty shell, no collapsed chip).
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
        aria-label="Agent panel collapsed вЂ” click to expand goal, tasks and status"
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
  // transcript and summary. Back returns to the overview. It inherits the
  // island's floating position so drilling in doesn't snap the card around.
  if (selectedSubagent) {
    return <SubagentWindow run={selectedSubagent} t={t} onBack={() => setSelectedSubagentId(null)} drag={drag} />;
  }

  return (
    <aside
      ref={drag.railRef}
      className="ai-agent-orchestration-rail"
      style={drag.floatStyle}
      data-floating={drag.floating || undefined}
      aria-label={t("aiChat.orchestration.aria")}
    >
      <div
        className="ai-agent-rail-grip"
        role="button"
        tabIndex={0}
        aria-label={t("aiChat.orchestration.dragHint")}
        title={t("aiChat.orchestration.dragHint")}
        onPointerDown={drag.onGripPointerDown}
        onPointerMove={drag.onGripPointerMove}
        onPointerUp={drag.onGripPointerEnd}
        onPointerCancel={drag.onGripPointerEnd}
        onKeyDown={drag.onGripKeyDown}
      >
        <GripHorizontal size={12} aria-hidden="true" />
      </div>
      {drag.floating && (
        <button
          type="button"
          className="ai-agent-rail-redock"
          onClick={drag.redock}
          title={t("aiChat.orchestration.redock")}
          aria-label={t("aiChat.orchestration.redock")}
        >
          <Pin size={9} />
        </button>
      )}
      <button
        type="button"
        className="ai-agent-rail-collapse"
        onClick={(e) => { e.stopPropagation(); onToggleCollapsed?.(); }}
        title="Collapse agent island (frees chat space)"
        aria-label="Collapse"
      >
        <Minus size={9} />
      </button>
      <AspectorAgentNowBar sessionId={sessionId} sessionStatus={sessionStatus} t={t} />

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
              {Number.isFinite(goalRun.limits.maxTokens) && goalRun.limits.maxTokens > 0 && (() => {
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
        <div
          className="ai-agent-task-progress"
          role="progressbar"
          aria-valuemin={0}
          aria-valuemax={todos.length}
          aria-valuenow={completedTodos}
          aria-label={t("aiChat.orchestration.tasksProgress", { done: completedTodos, total: todos.length })}
          title={t("aiChat.orchestration.tasksProgress", { done: completedTodos, total: todos.length })}
        >
          <span className="ai-agent-task-progress-done" style={{ width: `${(completedTodos / todos.length) * 100}%` }} />
          <span className="ai-agent-task-progress-active" style={{ width: `${(activeTodos / todos.length) * 100}%` }} />
        </div>
        <ul className="ai-agent-rail-tasks ai-agent-rail-tasks-compact">
          {todos.map((todo) => (
            <li key={todo.id} data-status={todo.status}>
              <TaskStatusGlyph status={todo.status} label={t(`aiChat.orchestration.status.${todo.status}`)} />
              <div className="ai-agent-rail-task-main">
                <div className="ai-agent-rail-task-line">
                  {todo.priority === "high" && (
                    <span
                      className="ai-agent-rail-task-prio"
                      role="img"
                      aria-label={t("aiChat.orchestration.priority.high")}
                      title={t("aiChat.orchestration.priority.high")}
                    />
                  )}
                  <span
                    className="ai-agent-rail-task-text"
                    title={todo.notes ? `${todo.content}\n\n${t("aiChat.orchestration.taskNotes", { notes: todo.notes })}` : todo.content}
                  >
                    {todo.content}
                  </span>
                </div>
                {todo.notes && <span className="ai-agent-rail-task-notes">{todo.notes}</span>}
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
              </div>
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
        <div className="ai-agent-rail-history-head">
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
          <button
            type="button"
            className="ai-subagent-cancel ai-subagent-remove"
            title={t("aiChat.subagents.clearHistory")}
            aria-label={t("aiChat.subagents.clearHistory")}
            onClick={() => clearFinishedSubagentRuns(sessionId)}
          >
            <Trash2 size={9} />
          </button>
        </div>
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

function TaskStatusGlyph({ status, label }: { status: AiSessionTodoStatus; label: string }) {
  if (status === "completed") return <Check size={11} role="img" aria-label={label}><title>{label}</title></Check>;
  if (status === "in_progress") return <Loader2 size={11} className="spin-icon" role="img" aria-label={label}><title>{label}</title></Loader2>;
  if (status === "blocked") return <Ban size={11} role="img" aria-label={label}><title>{label}</title></Ban>;
  if (status === "cancelled") return <XCircle size={11} data-muted="true" role="img" aria-label={label}><title>{label}</title></XCircle>;
  return <Circle size={11} role="img" aria-label={label}><title>{label}</title></Circle>;
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
  // Mini live console: the newest transcript entry (a "в†’ Tool вЂ¦" start line or
  // the streamed thinking/answer snapshot) shown under a RUNNING row, so the
  // rail narrates what each subagent is doing without drilling in. A FAILED row
  // keeps showing its error line (the transcript tail is the compact error) so
  // the failure is readable in the rail, not hidden behind a "failed" badge.
  const tail = run.status === "running"
    ? run.transcript.at(-1)
    : run.status === "failed"
      ? run.transcript.at(-1)
      : undefined;
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
        {run.status === "running" ? (
          <button type="button" className="ai-subagent-cancel" title={t("aiChat.subagents.cancel")} onClick={() => cancelSubagentRun(run.id)}>
            <Square size={9} />
          </button>
        ) : (
          <button type="button" className="ai-subagent-cancel ai-subagent-remove" title={t("aiChat.subagents.remove")} aria-label={t("aiChat.subagents.remove")} onClick={() => removeSubagentRun(run.id)}>
            <Trash2 size={9} />
          </button>
        )}
      </div>
      {tail && (
        <button
          type="button"
          className="ai-subagent-row-tail"
          data-role={tail.role}
          onClick={() => onSelect(run.id)}
          title={tail.content}
        >
          {tail.content}
        </button>
      )}
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
 * Dedicated subagent window: the island drills into one run вЂ” live status dot,
 * ticking clock, streamed transcript (auto-follows the bottom) and the final
 * summary. Back returns to the Goal/Tasks/Subagents overview. The header
 * doubles as the drag surface (its buttons keep their clicks).
 */
function SubagentWindow({ run, t, onBack, drag }: { run: SubagentRun; t: TranslateFn; onBack: () => void; drag: IslandDrag }) {
  // Tick the clock once a second while the run is live.
  const [, setClockTick] = useState(0);
  useEffect(() => {
    if (run.status !== "running") return;
    const timer = setInterval(() => setClockTick((tick) => tick + 1), 1000);
    return () => clearInterval(timer);
  }, [run.status]);
  // Expanded observation mode: the same live transcript in a large centered
  // modal (portal вЂ” escapes the island's clamped geometry). Esc collapses it.
  const [expanded, setExpanded] = useState(false);
  useEffect(() => {
    if (!expanded) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") setExpanded(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [expanded]);

  const elapsed = formatSubagentClock((run.endedAt ?? Date.now()) - run.startedAt);
  const summary = run.summary.trim();
  // The live transcript usually ends with the same final answer the summary
  // carries вЂ” only render the summary block when it adds new text.
  const showSummary = Boolean(summary) && summary !== run.transcript.at(-1)?.content.trim();

  if (expanded) {
    return createPortal(
      <div className="ai-subagent-modal-backdrop" onClick={() => setExpanded(false)} role="presentation">
        <section
          className="ai-subagent-modal"
          role="dialog"
          aria-modal="true"
          aria-label={t("aiChat.subagents.aria")}
          onClick={(event) => event.stopPropagation()}
        >
          <header className="ai-subagent-window-head ai-subagent-modal-head">
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
            <button type="button" className="ai-subagent-window-back" onClick={() => setExpanded(false)} title={t("aiChat.subagents.collapse")} aria-label={t("aiChat.subagents.collapse")}>
              <X size={12} />
            </button>
          </header>
          <p className="ai-subagent-window-title" title={run.description}>{run.description}</p>
          <SubagentTranscript run={run} t={t} showSummary={showSummary} summary={summary} follow />
        </section>
      </div>,
      document.body,
    );
  }

  return (
    <aside
      ref={drag.railRef}
      className="ai-agent-orchestration-rail ai-subagent-window"
      style={drag.floatStyle}
      data-floating={drag.floating || undefined}
      aria-label={t("aiChat.subagents.aria")}
    >
      <header
        className="ai-subagent-window-head"
        title={t("aiChat.orchestration.dragHint")}
        onPointerDown={drag.onGripPointerDown}
        onPointerMove={drag.onGripPointerMove}
        onPointerUp={drag.onGripPointerEnd}
        onPointerCancel={drag.onGripPointerEnd}
      >
        <button type="button" className="ai-subagent-window-back" onClick={onBack} title={t("aiChat.subagents.back")} aria-label={t("aiChat.subagents.back")}>
          <ArrowLeft size={12} />
        </button>
        <span className="ai-subagent-status-dot" data-status={run.status} aria-hidden="true" />
        <span className="ai-subagent-panel-type">{run.subagentType}</span>
        <span className="ai-subagent-window-clock">{elapsed}</span>
        <button type="button" className="ai-subagent-window-back ai-subagent-expand" onClick={() => setExpanded(true)} title={t("aiChat.subagents.expand")} aria-label={t("aiChat.subagents.expand")}>
          <Maximize2 size={11} />
        </button>
        {drag.floating && (
          <button
            type="button"
            className="ai-agent-rail-redock"
            onClick={drag.redock}
            title={t("aiChat.orchestration.redock")}
            aria-label={t("aiChat.orchestration.redock")}
          >
            <Pin size={9} />
          </button>
        )}
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
      <SubagentTranscript run={run} t={t} showSummary={showSummary} summary={summary} follow />
    </aside>
  );
}

/** The streamed transcript list, shared by the island window and the expanded
 *  modal. `follow` pins the scroll to the bottom on every new entry. */
function SubagentTranscript({
  run,
  t,
  showSummary,
  summary,
  follow,
}: {
  run: SubagentRun;
  t: TranslateFn;
  showSummary: boolean;
  summary: string;
  follow?: boolean;
}) {
  const transcriptRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!follow) return;
    const el = transcriptRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [follow, run.revision]);
  return (
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