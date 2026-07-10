import { ArrowUpCircle, Download, Loader2, RefreshCw, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from '../../lib/i18n/useTranslation';
import type { UpdaterState } from '../../lib/hooks/use-updater';

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
const MB = 1024 * 1024;

function formatMb(bytes: number): string {
  return (bytes / MB).toFixed(1);
}

/** Rolling download speed (bytes/s) from progress samples; null until measurable. */
function useDownloadSpeed(downloadedBytes: number, active: boolean): number | null {
  const sample = useRef<{ at: number; bytes: number } | null>(null);
  const [speed, setSpeed] = useState<number | null>(null);

  useEffect(() => {
    if (!active) {
      sample.current = null;
      setSpeed(null);
      return;
    }
    const now = performance.now();
    const prev = sample.current;
    if (prev && downloadedBytes > prev.bytes) {
      const elapsed = (now - prev.at) / 1000;
      if (elapsed > 0.25) {
        const instant = (downloadedBytes - prev.bytes) / elapsed;
        // Light smoothing so the number doesn't jitter every event.
        setSpeed((current) => (current === null ? instant : current * 0.6 + instant * 0.4));
        sample.current = { at: now, bytes: downloadedBytes };
      }
    } else if (!prev) {
      sample.current = { at: now, bytes: downloadedBytes };
    }
  }, [downloadedBytes, active]);

  return speed;
}

export function UpdateNotice({ state, onInstall, onDismiss, onRetry }: UpdateNoticeProps) {
  const { t } = useTranslation();
  const [showNotes, setShowNotes] = useState(false);
  const speed = useDownloadSpeed(state.downloadedBytes, state.status === "downloading");

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
              <div className="update-notice-title-row">
                <strong className="update-notice-title">{t("update.downloading.title")}</strong>
                <span className="update-notice-percent">{percent === null ? "" : `${percent}%`}</span>
              </div>
              <p className="update-notice-text">
                {percent === null ? t("update.downloading.preparing") : `Aspect IDE ${version}`}
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
              <div className="update-notice-stats">
                <span className="update-notice-bytes">
                  {formatMb(state.downloadedBytes)}
                  {state.totalBytes !== null ? ` / ${formatMb(state.totalBytes)}` : ""} MB
                </span>
                {speed !== null && speed > 0 && (
                  <span className="update-notice-speed">{formatMb(speed)} MB/s</span>
                )}
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
