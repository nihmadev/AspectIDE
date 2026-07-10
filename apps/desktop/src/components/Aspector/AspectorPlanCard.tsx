import {
  AlertTriangle,
  ArrowRight,
  Bot,
  CheckCircle2,
  ChevronRight,
  FileCode,
  GitBranch,
  Gauge,
  ListChecks,
  Loader2,
  NotebookPen,
  Play,
  Sparkles,
} from "lucide-react";
import { useState } from "react";
import type { TranslateFn } from '../../lib/i18n/useTranslation';
import type { PendingPlan } from '../../lib/aspector/utils/pending-plan';
import { openWorkspaceEditorPath } from '../../lib/editor/open-workspace-editor-path';

type AspectorPlanCardProps = {
  plan: PendingPlan;
  /** Hand the plan to Agent mode and begin execution (manual plans only). */
  onStart: () => void;
  /** Disable Start while a turn is already running. */
  busy: boolean;
  /** Live agent mode. In Automatic the plan always auto-runs — never show a Start button. */
  agentMode?: string;
  t: TranslateFn;
};

/** Below this 5-phase quality score, the card surfaces the gate's coaching so the
 * user can ask for a stronger plan before Start (soft gate — Start stays enabled). */
const QUALITY_WARN_THRESHOLD = 0.75;

/**
 * `PresentPlan` card. Renders a titled, expandable list of structured steps plus
 * the think-mcp reasoning phases when present — the key decision (alternatives),
 * risks/failure modes, and verification. In Plan/Agent mode it shows a primary
 * "Start" button that hands the plan to Agent execution; in Automatic mode the plan
 * auto-starts, so it shows a running indicator instead. When the deterministic
 * quality gate scored the plan low (and it is not auto-running), the card surfaces
 * the coaching so the user can push for a sharper plan — a soft gate, never a block.
 */
export function AspectorPlanCard({ plan, onStart, busy, agentMode, t }: AspectorPlanCardProps) {
  const [expanded, setExpanded] = useState<Set<number>>(() => new Set([0]));
  // Automatic mode never hands a plan to the user to start — it auto-executes.
  // Trust the live mode too, not only the (snapshotted) plan.autoStart, so a plan
  // proposed while the backend saw a non-automatic mode still renders as auto here.
  const autoStart = plan.autoStart || agentMode === "automatic";

  const alternatives = plan.alternatives ?? [];
  const risks = plan.risks ?? [];
  const verification = plan.verification ?? [];
  const coaching = plan.coaching ?? [];
  const quality = typeof plan.quality === "number" ? plan.quality : 1;
  const showCoaching = !autoStart && coaching.length > 0 && quality < QUALITY_WARN_THRESHOLD;

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

      {alternatives.length > 0 && (
        <section className="ai-plan-section ai-plan-section-decision">
          <h4>
            <GitBranch size={12} aria-hidden="true" />
            {t("aiChat.plan.decision")}
          </h4>
          <ul>
            {alternatives.map((decision, index) => (
              <li key={`alt-${index}`}>
                <strong>{decision.option}</strong>
                {decision.tradeoff && <span> — {decision.tradeoff}</span>}
              </li>
            ))}
          </ul>
        </section>
      )}

      {risks.length > 0 && (
        <section className="ai-plan-section ai-plan-section-risks">
          <h4>
            <AlertTriangle size={12} aria-hidden="true" />
            {t("aiChat.plan.risks")}
          </h4>
          <ul>
            {risks.map((risk, index) => (
              <li key={`risk-${index}`}>{risk}</li>
            ))}
          </ul>
        </section>
      )}

      {verification.length > 0 && (
        <section className="ai-plan-section ai-plan-section-verify">
          <h4>
            <CheckCircle2 size={12} aria-hidden="true" />
            {t("aiChat.plan.verification")}
          </h4>
          <ul>
            {verification.map((check, index) => (
              <li key={`verify-${index}`}>{check}</li>
            ))}
          </ul>
        </section>
      )}

      {showCoaching && (
        <section className="ai-plan-coaching" role="note">
          <span className="ai-plan-quality-badge" title={t("aiChat.plan.qualityLabel", { percent: Math.round(quality * 100) })}>
            <Gauge size={12} aria-hidden="true" />
            {t("aiChat.plan.qualityLabel", { percent: Math.round(quality * 100) })}
          </span>
          <p className="ai-plan-coaching-hint">{t("aiChat.plan.coachingHint")}</p>
          <ul>
            {coaching.map((tip, index) => (
              <li key={`coach-${index}`}>{tip}</li>
            ))}
          </ul>
        </section>
      )}

      <footer className="ai-plan-card-foot">
        {autoStart ? (
          <span className="ai-plan-card-auto">
            <Loader2 size={12} className="spin-icon" />
            {t("aiChat.plan.autoRunning")}
          </span>
        ) : (
          <>
            <span className="ai-plan-card-tip">{t("aiChat.plan.startHint")}</span>
            {/* Mode-transition chip: starting a plan visibly hands the chat from
                Plan (read-only) to Agent (execution) — the switch used to be
                invisible until the user noticed the mode selector had changed. */}
            <span className="ai-plan-mode-switch" title={t("aiChat.plan.modeSwitch")}>
              <span className="ai-plan-mode" data-mode="plan">
                <NotebookPen size={11} aria-hidden="true" />
                Plan
              </span>
              <ArrowRight size={11} aria-hidden="true" className="ai-plan-mode-arrow" />
              <span className="ai-plan-mode" data-mode="agent">
                <Bot size={11} aria-hidden="true" />
                Agent
              </span>
            </span>
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
