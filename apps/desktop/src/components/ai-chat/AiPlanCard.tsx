import { ChevronRight, FileCode, ListChecks, Loader2, Play, Sparkles } from "lucide-react";
import { useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { PendingPlan } from "../../lib/aiPendingPlan";
import { openWorkspaceEditorPath } from "../../lib/openWorkspaceEditorPath";

type AiPlanCardProps = {
  plan: PendingPlan;
  /** Hand the plan to Agent mode and begin execution (manual plans only). */
  onStart: () => void;
  /** Disable Start while a turn is already running. */
  busy: boolean;
  t: TranslateFn;
};

/**
 * `PresentPlan` card. Renders a titled, expandable list of structured steps with
 * optional detail + file link per step. In Plan/Agent mode it shows a primary
 * "Start" button that hands the plan to Agent execution; in Automatic mode the
 * plan auto-starts, so instead of a button it shows a running indicator (the card
 * is purely informational there).
 */
export function AiPlanCard({ plan, onStart, busy, t }: AiPlanCardProps) {
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set([0]));

  const toggleStep = (index: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  return (
    <article className="ai-plan-card" role="region" aria-label={t("aiChat.plan.aria")}>
      <header className="ai-plan-card-head">
        <span className="ai-plan-card-icon" aria-hidden="true">
          <Sparkles size={15} />
        </span>
        <div className="ai-plan-card-copy">
          <strong>{plan.title}</strong>
          {plan.summary && <p>{plan.summary}</p>}
        </div>
        <span className="ai-plan-card-count" title={t("aiChat.plan.stepCount", { count: plan.steps.length })}>
          <ListChecks size={12} />
          {plan.steps.length}
        </span>
      </header>

      <ol className="ai-plan-steps">
        {plan.steps.map((step, index) => {
          const isOpen = expanded.has(index);
          const hasBody = Boolean(step.detail || step.file);
          return (
            <li key={`${step.title}-${index}`} className="ai-plan-step" data-open={isOpen || undefined}>
              <button
                type="button"
                className="ai-plan-step-head"
                onClick={() => hasBody && toggleStep(index)}
                data-static={!hasBody || undefined}
                aria-expanded={hasBody ? isOpen : undefined}
              >
                <span className="ai-plan-step-index">{index + 1}</span>
                <span className="ai-plan-step-title">{step.title}</span>
                {hasBody && (
                  <span className="ai-plan-step-chevron" aria-hidden="true">
                    <ChevronRight size={13} />
                  </span>
                )}
              </button>
              {isOpen && hasBody && (
                <div className="ai-plan-step-body">
                  {step.detail && <p>{step.detail}</p>}
                  {step.file && (
                    <button
                      type="button"
                      className="ai-plan-step-file"
                      onClick={() => void openWorkspaceEditorPath(step.file)}
                      title={step.file}
                    >
                      <FileCode size={11} />
                      {basename(step.file)}
                    </button>
                  )}
                </div>
              )}
            </li>
          );
        })}
      </ol>

      <footer className="ai-plan-card-foot">
        {plan.autoStart ? (
          <span className="ai-plan-card-auto">
            <Loader2 size={12} className="spin-icon" />
            {t("aiChat.plan.autoRunning")}
          </span>
        ) : (
          <>
            <span className="ai-plan-card-tip">{t("aiChat.plan.startHint")}</span>
            <button type="button" className="ai-plan-start" onClick={onStart} disabled={busy}>
              <Play size={12} />
              {t("aiChat.plan.start")}
            </button>
          </>
        )}
      </footer>
    </article>
  );
}

function basename(path: string) {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}
