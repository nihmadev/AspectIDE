import { Check, ChevronDown, Circle, Loader2, X } from "lucide-react";
import { useEffect, useState, useSyncExternalStore } from "react";
import { getActiveGoalRun, getAiSessionGoalRunsSnapshot, subscribeAiSessionGoalRuns } from "../../lib/aiSessionGoalRun";
import { getAiSessionTodosSnapshot, listAiSessionTodos, subscribeAiSessionTodos, type AiSessionTodo } from "../../lib/aiSessionTodos";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

/** A plan the user handed to Agent execution — rendered as a live checklist. */
export type ActivePlanRun = {
  planId: string;
  sessionId: string;
  title: string;
  /** Step count at Start time — used only as a floor so the bar never regresses
   *  if the agent's live task list temporarily has fewer items than the plan did
   *  (e.g. mid-rewrite via TodoWrite). */
  stepCount: number;
};

type AiPlanRunCardProps = {
  run: ActivePlanRun;
  /** True once the CURRENT turn has settled (no live send in flight). A plan that
   *  spans multiple turns (Automatic goal-run continuations) still counts as
   *  running between turns — this card folds in the goal-run store itself so a
   *  multi-turn plan doesn't flash "done" during the inter-turn delay. */
  turnSettled: boolean;
  onDismiss: () => void;
  t: TranslateFn;
};

const COLLAPSE_THRESHOLD = 6;

/**
 * Live "plan run" card. Mounted the moment the user presses Start on an
 * `AiPlanCard`, it renders the plan's steps as a checklist driven by the SAME
 * live todo store the orchestration rail and turn-summary card read (TodoWrite
 * updates land here with no polling — `subscribeAiSessionTodos`). Steps beyond
 * `COLLAPSE_THRESHOLD` collapse behind "Show all" once any are done, keeping a
 * long plan from dominating the transcript while it is still mostly pending.
 */
export function AiPlanRunCard({ run, turnSettled, onDismiss, t }: AiPlanRunCardProps) {
  useSyncExternalStore(subscribeAiSessionTodos, getAiSessionTodosSnapshot, getAiSessionTodosSnapshot);
  useSyncExternalStore(subscribeAiSessionGoalRuns, getAiSessionGoalRunsSnapshot, getAiSessionGoalRunsSnapshot);
  const [showAll, setShowAll] = useState(false);
  const todos = listAiSessionTodos(run.sessionId);
  // Start-click races the send pipeline: the card mounts synchronously while
  // handleSend is still snapshotting attachments/checkpoints, i.e. BEFORE the
  // session flips busy — so a purely render-time "not busy" must not read as
  // "complete". Only trust turnSettled after the turn was seen busy once.
  const [everBusy, setEverBusy] = useState(false);
  useEffect(() => {
    if (!turnSettled) {
      setEverBusy(true);
    }
  }, [turnSettled]);
  // A goal run spanning several turns (Automatic continuations) still owns this
  // plan between turns — only call it settled once BOTH the live turn and the
  // goal run have ended.
  const settled = everBusy && turnSettled && !getActiveGoalRun(run.sessionId);

  // The pinned task list is the live source of truth; fall back to the plan's own
  // step count only until the first TodoWrite lands (avoids a flash of "0/0").
  const total = Math.max(todos.length, run.stepCount);
  const done = todos.filter((todo) => todo.status === "completed").length;
  const failed = todos.filter((todo) => todo.status === "cancelled").length;
  const progressPct = total > 0 ? Math.round((done / total) * 100) : 0;
  const allDone = settled && total > 0 && done + failed >= total;

  const visibleTodos = showAll || todos.length <= COLLAPSE_THRESHOLD ? todos : todos.slice(0, COLLAPSE_THRESHOLD);
  const hiddenCount = todos.length - visibleTodos.length;

  return (
    <article className="ai-plan-run-card" role="region" aria-label={t("aiChat.plan.runAria")} data-settled={settled || undefined}>
      <header className="ai-plan-run-head">
        <div className="ai-plan-run-copy">
          <span className="ai-plan-run-status" data-done={allDone || undefined}>
            {settled ? <Check size={13} /> : <Loader2 size={13} className="spin-icon" />}
          </span>
          <strong title={run.title}>{run.title}</strong>
        </div>
        <span className="ai-plan-run-count">
          {t("aiChat.plan.runProgress", { done, total })}
        </span>
        {settled && (
          <button
            type="button"
            className="ai-plan-run-dismiss"
            onClick={onDismiss}
            title={t("aiChat.turnCheckpoint.dismiss")}
            aria-label={t("aiChat.turnCheckpoint.dismiss")}
          >
            <X size={13} />
          </button>
        )}
      </header>

      <div className="ai-plan-run-track" aria-hidden="true">
        <div
          className="ai-plan-run-fill"
          data-failed={failed > 0 || undefined}
          style={{ width: `${Math.max(total > 0 ? 3 : 0, progressPct)}%` }}
        />
      </div>

      {todos.length > 0 && (
        <ul className="ai-plan-run-steps">
          {visibleTodos.map((todo) => (
            <li key={todo.id} data-status={todo.status}>
              <PlanRunGlyph status={todo.status} />
              <span className="ai-plan-run-step-text" title={todo.content}>{todo.content}</span>
            </li>
          ))}
        </ul>
      )}

      {hiddenCount > 0 && (
        <button type="button" className="ai-plan-run-show-all" onClick={() => setShowAll(true)}>
          <ChevronDown size={12} />
          {t("aiChat.plan.runShowAll", { count: hiddenCount })}
        </button>
      )}

      {settled && (
        <footer className="ai-plan-run-foot">
          {failed > 0
            ? t("aiChat.plan.runSummaryFailed", { done, failed })
            : t("aiChat.plan.runSummaryDone", { done })}
        </footer>
      )}
    </article>
  );
}

function PlanRunGlyph({ status }: { status: AiSessionTodo["status"] }) {
  if (status === "completed") return <Check size={12} className="ai-plan-run-glyph-done" aria-hidden="true" />;
  if (status === "in_progress") return <Loader2 size={12} className="spin-icon ai-plan-run-glyph-active" aria-hidden="true" />;
  if (status === "cancelled") return <X size={12} className="ai-plan-run-glyph-failed" aria-hidden="true" />;
  return <Circle size={12} className="ai-plan-run-glyph-pending" aria-hidden="true" />;
}
