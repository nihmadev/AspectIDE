import { ArrowUpCircle, Download, Loader2, RefreshCw, X } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "../lib/i18n/useTranslation";
import type { UpdaterState } from "../lib/useUpdater";

type UpdateNoticeProps = {
  state: UpdaterState;
  onInstall: () => void;
  onDismiss: () => void;
  onRetry: () => void;
};

/**
 * Transient bottom-right card surfacing the auto-update lifecycle. Renders only
 * for the actionable states (available / downloading / relaunching / error);
 * idle / up-to-date / checking stay silent so the UI is never noisy. The full
 * controls (manual check, current version) live in Settings → Updates.
 */
export function UpdateNotice({ state, onInstall, onDismiss, onRetry }: UpdateNoticeProps) {
  const { t } = useTranslation();
  const [showNotes, setShowNotes] = useState(false);

  const visible =
    state.status === "available" ||
    state.status === "downloading" ||
    state.status === "relaunching" ||
    state.status === "error";
  if (!visible) return null;

  const version = state.availableVersion ?? "";
  const percent = state.progress === null ? null : Math.round(state.progress * 100);

  return (
    <div className="update-notice" data-status={state.status} role="status" aria-live="polite">
      <div className="update-notice-glow" aria-hidden="true" />
      <div className="update-notice-body">
        <span className="update-notice-icon" aria-hidden="true">
          {state.status === "available" && <ArrowUpCircle size={18} strokeWidth={2} />}
          {state.status === "downloading" && <Download size={18} strokeWidth={2} />}
          {state.status === "relaunching" && <Loader2 size={18} strokeWidth={2} className="update-notice-spin" />}
          {state.status === "error" && <RefreshCw size={18} strokeWidth={2} />}
        </span>

        <div className="update-notice-content">
          {state.status === "available" && (
            <>
              <strong className="update-notice-title">{t("update.available.title")}</strong>
              <p className="update-notice-text">{t("update.available.body", { version })}</p>
              {state.notes && showNotes && (
                <pre className="update-notice-notes">{state.notes}</pre>
              )}
              <div className="update-notice-actions">
                <button type="button" className="update-notice-primary" onClick={onInstall}>
                  {t("update.action.install")}
                </button>
                {state.notes && (
                  <button type="button" className="update-notice-ghost" onClick={() => setShowNotes((value) => !value)}>
                    {showNotes ? t("update.action.hideNotes") : t("update.action.viewNotes")}
                  </button>
                )}
                <button type="button" className="update-notice-ghost" onClick={onDismiss}>
                  {t("update.action.later")}
                </button>
              </div>
            </>
          )}

          {state.status === "downloading" && (
            <>
              <strong className="update-notice-title">{t("update.downloading.title")}</strong>
              <p className="update-notice-text">
                {percent === null
                  ? t("update.downloading.preparing")
                  : t("update.downloading.body", { version, percent })}
              </p>
              <div
                className="update-notice-bar-track"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={100}
                aria-valuenow={percent ?? undefined}
              >
                <div
                  className="update-notice-bar"
                  data-indeterminate={percent === null ? "true" : undefined}
                  style={percent === null ? undefined : { width: `${percent}%` }}
                />
              </div>
            </>
          )}

          {state.status === "relaunching" && (
            <>
              <strong className="update-notice-title">{t("update.relaunching.title")}</strong>
              <p className="update-notice-text">{t("update.relaunching.body")}</p>
            </>
          )}

          {state.status === "error" && (
            <>
              <strong className="update-notice-title">{t("update.error.title")}</strong>
              <p className="update-notice-text update-notice-error-text">{state.error}</p>
              <div className="update-notice-actions">
                <button type="button" className="update-notice-primary" onClick={onRetry}>
                  {t("update.error.retry")}
                </button>
                <button type="button" className="update-notice-ghost" onClick={onDismiss}>
                  {t("update.action.later")}
                </button>
              </div>
            </>
          )}
        </div>

        {(state.status === "available" || state.status === "error") && (
          <button
            type="button"
            className="update-notice-close"
            aria-label={t("update.action.later")}
            title={t("update.action.later")}
            onClick={onDismiss}
          >
            <X size={14} />
          </button>
        )}
      </div>
    </div>
  );
}
