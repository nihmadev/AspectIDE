import { AlertTriangle, Loader2, X } from "lucide-react";
import { useState } from "react";
import { isWeakProjectIndex } from "../../lib/aiProjectIndexPolicy";
import type { AiIndexState } from "../../lib/store";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiIndexStatusBannerProps = {
  index: AiIndexState;
  indexingEnabled: boolean;
  onReindex?: () => void;
  onOpenSettings?: () => void;
  t: TranslateFn;
};

export function AiIndexStatusBanner({ index, indexingEnabled, onReindex, onOpenSettings, t }: AiIndexStatusBannerProps) {
  const [limitedDismissed, setLimitedDismissed] = useState(false);

  if (!indexingEnabled) {
    return (
      <div className="ai-index-banner" data-level="off">
        <AlertTriangle size={14} />
        <span>{t("aiChat.index.disabled")}</span>
        {onOpenSettings && (
          <button type="button" onClick={onOpenSettings}>{t("aiChat.index.openSettings")}</button>
        )}
      </div>
    );
  }

  if (index.status === "indexing") {
    return (
      <div className="ai-index-banner" data-level="busy">
        <Loader2 size={14} className="spin-icon" />
        <span>{t("aiChat.index.indexing", { progress: Math.round(index.progress * 100), files: index.indexedFiles })}</span>
      </div>
    );
  }

  if (index.lastError) {
    return (
      <div className="ai-index-banner" data-level="error">
        <AlertTriangle size={14} />
        <span>{t("aiChat.index.error")}</span>
        {onReindex && (
          <button type="button" onClick={onReindex}>{t("aiChat.index.reindex")}</button>
        )}
        {onOpenSettings && (
          <button type="button" onClick={onOpenSettings}>{t("aiChat.index.openSettings")}</button>
        )}
      </div>
    );
  }

  if (isWeakProjectIndex(index)) {
    if (limitedDismissed) return null;
    const limited = index.quality === "limited";
    return (
      <div className="ai-index-banner" data-level={limited ? "limited" : "stale"}>
        <AlertTriangle size={14} />
        <span>{limited ? t("aiChat.index.limited") : t("aiChat.index.notReady")}</span>
        {onReindex && (
          <button type="button" onClick={onReindex}>{t("aiChat.index.reindex")}</button>
        )}
        <button
          type="button"
          className="ai-index-banner-close"
          onClick={() => setLimitedDismissed(true)}
          title="Dismiss warning"
        >
          <X size={10} />
        </button>
      </div>
    );
  }

  return null;
}