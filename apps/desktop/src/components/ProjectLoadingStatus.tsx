import { Check, CircleAlert, FolderOpen, Loader2 } from "lucide-react";
import type { ProjectLoadSummary } from "../lib/projectLoadPresentation";
import { useTranslation } from "../lib/i18n/useTranslation";

type ProjectLoadingStatusProps = {
  onDismissError: () => void;
  summary: ProjectLoadSummary;
};

export function ProjectLoadingStatus({ onDismissError, summary }: ProjectLoadingStatusProps) {
  const { t } = useTranslation();
  if (!summary.active && summary.stage !== "error") return null;

  const boundedProgress = Math.max(0, Math.min(100, summary.progress));
  const progressLabel = t("projectLoading.progressLabel", { progress: Math.round(boundedProgress) });

  return (
    <div className="project-loading-overlay" data-stage={summary.stage} role="status" aria-live="polite" aria-busy={summary.active}>
      <div className="project-loading-backdrop" />
      <section className="project-loading-card" aria-label={t("projectLoading.screenLabel")}>
        <div className="project-loading-mark" aria-hidden="true">
          {summary.stage === "error" ? <CircleAlert size={22} /> : <FolderOpen size={22} />}
        </div>

        <div className="project-loading-copy">
          <span>{t("projectLoading.eyebrow")}</span>
          <h1>{t(summary.labelKey)}</h1>
          <p>{summary.workspaceName ?? summary.root ?? t("projectLoading.workspacePending")}</p>
        </div>

        <div className="project-loading-meter" aria-label={progressLabel} aria-valuemax={100} aria-valuemin={0} aria-valuenow={Math.round(boundedProgress)} role="progressbar">
          <div className="project-loading-meter-head">
            <span>{summary.stage === "error" ? t("projectLoading.errorDetail") : t("projectLoading.preparing")}</span>
            <strong>{progressLabel}</strong>
          </div>
          <div className="project-loading-progress"><span style={{ width: `${boundedProgress}%` }} /></div>
        </div>

        {summary.error ? (
          <>
            <p className="project-loading-error">{summary.error}</p>
            <button className="project-loading-dismiss" type="button" onClick={onDismissError}>
              {t("projectLoading.dismissError")}
            </button>
          </>
        ) : (
          <div className="project-loading-checklist">
            {summary.checklist.map((item, index) => (
              <span key={item.key} data-active={item.active} data-done={item.done}>
                <i>{item.done ? <Check size={12} /> : item.active ? <Loader2 size={12} className="spin-icon" /> : <em>{index + 1}</em>}</i>
                <b>{t(item.key)}</b>
              </span>
            ))}
          </div>
        )}
      </section>
    </div>
  );
}
