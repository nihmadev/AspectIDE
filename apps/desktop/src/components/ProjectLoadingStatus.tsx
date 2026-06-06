import { AlertCircle, Check, FolderOpen, Loader2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import type { ProjectLoadSummary } from "../lib/projectLoadPresentation";
import { useTranslation } from "../lib/i18n/useTranslation";

type ProjectLoadingStatusProps = {
  onDismissError: () => void;
  summary: ProjectLoadSummary;
};

const PROGRESS_SMOOTH_MS = 420;
const EXIT_MS = 260;

export function ProjectLoadingStatus({ onDismissError, summary }: ProjectLoadingStatusProps) {
  const { t } = useTranslation();
  const targetProgress = Math.max(0, Math.min(100, summary.progress));
  const [displayProgress, setDisplayProgress] = useState(targetProgress);
  const [overlayVisible, setOverlayVisible] = useState(summary.active || summary.stage === "error");
  const [exiting, setExiting] = useState(false);
  const progressFrameRef = useRef<number | null>(null);
  const displayProgressRef = useRef(displayProgress);

  useEffect(() => {
    const from = displayProgressRef.current;
    const startedAt = performance.now();
    if (progressFrameRef.current !== null) cancelAnimationFrame(progressFrameRef.current);

    const animate = (now: number) => {
      const elapsed = now - startedAt;
      const tNorm = Math.min(1, elapsed / PROGRESS_SMOOTH_MS);
      const eased = 1 - (1 - tNorm) ** 3;
      const next = from + (targetProgress - from) * eased;
      displayProgressRef.current = next;
      setDisplayProgress(next);
      if (tNorm < 1) progressFrameRef.current = requestAnimationFrame(animate);
      else progressFrameRef.current = null;
    };

    progressFrameRef.current = requestAnimationFrame(animate);
    return () => {
      if (progressFrameRef.current !== null) cancelAnimationFrame(progressFrameRef.current);
    };
  }, [targetProgress]);

  useEffect(() => {
    if (summary.active || summary.stage === "error") {
      setOverlayVisible(true);
      setExiting(false);
      return;
    }
    if (!overlayVisible) return;
    setExiting(true);
    const timer = window.setTimeout(() => {
      setOverlayVisible(false);
      setExiting(false);
    }, EXIT_MS);
    return () => window.clearTimeout(timer);
  }, [overlayVisible, summary.active, summary.stage]);

  if (!overlayVisible) return null;

  const boundedProgress = Math.max(0, Math.min(100, displayProgress));
  const progressLabel = t("projectLoading.progressLabel", { progress: Math.round(boundedProgress) });
  const workspaceTitle = summary.workspaceName ?? t("projectLoading.preparing");
  const workspacePath = summary.root ?? t("projectLoading.workspacePending");
  const detailText = summary.detail ? t(summary.detail.key, summary.detail.params) : null;

  return (
    <div
      className="project-load-overlay"
      data-stage={summary.stage}
      data-exiting={exiting || undefined}
      role="status"
      aria-live="polite"
      aria-busy={summary.active}
    >
      <div className="project-load-scrim" aria-hidden="true" />
      <article className="project-load-card" aria-label={t("projectLoading.screenLabel")}>
        <header className="project-load-header">
          <span className="project-load-icon" aria-hidden="true">
            <FolderOpen size={18} strokeWidth={2} />
          </span>
          <div className="project-load-header-copy">
            <span className="project-load-eyebrow">{t("projectLoading.eyebrow")}</span>
            <h2 className="project-load-title">{workspaceTitle}</h2>
            <p className="project-load-path" title={workspacePath}>
              {workspacePath}
            </p>
          </div>
        </header>

        <ol className="project-load-steps">
          {summary.checklist.map((step) => {
            const state = step.done ? "done" : step.active ? "active" : "pending";
            return (
              <li key={step.key} className="project-load-step" data-state={state}>
                <span className="project-load-step-marker" aria-hidden="true">
                  {state === "done" ? (
                    <Check size={13} strokeWidth={2.5} />
                  ) : state === "active" ? (
                    <Loader2 size={13} className="project-load-spin" />
                  ) : (
                    <span className="project-load-step-dot" />
                  )}
                </span>
                <span className="project-load-step-label">{t(step.key)}</span>
              </li>
            );
          })}
        </ol>

        <footer className="project-load-footer">
          <div className="project-load-footer-top">
            <span className="project-load-status-text">{t(summary.labelKey)}</span>
            <span className="project-load-percent">{progressLabel}</span>
          </div>
          {detailText ? <p className="project-load-detail">{detailText}</p> : null}
          <div
            className="project-load-bar-track"
            aria-label={progressLabel}
            aria-valuemax={100}
            aria-valuemin={0}
            aria-valuenow={Math.round(boundedProgress)}
            role="progressbar"
          >
            <div
              className="project-load-bar"
              data-indeterminate={summary.active && boundedProgress < 99 ? "true" : undefined}
              style={{ width: `${boundedProgress}%` }}
            />
          </div>
        </footer>
      </article>

      {summary.error && (
        <div className="project-load-error" role="alert">
          <AlertCircle size={16} aria-hidden="true" />
          <p>{summary.error}</p>
          <button type="button" onClick={onDismissError}>
            {t("projectLoading.dismissError")}
          </button>
        </div>
      )}
    </div>
  );
}