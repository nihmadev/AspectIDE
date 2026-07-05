import { ChevronRight, RotateCcw } from "lucide-react";
import type { AiChatErrorPresentation } from "../../lib/aiChatErrors";
import type { AiChatErrorHistoryEntry } from "../../lib/store";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

function formatErrorTime(timestamp: number) {
  return new Date(timestamp).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

/** Inline chat error with optional retry + open-settings actions, styled by error
 *  kind. When the turn failed more than once (auto-retry ladder, manual retries),
 *  a collapsible history of the previous attempts renders under the actions. */
export function AiChatError({
  canRetry,
  history,
  presentation,
  onRetry,
  onOpenSettings,
  t,
}: {
  canRetry: boolean;
  history?: AiChatErrorHistoryEntry[];
  presentation: AiChatErrorPresentation;
  onRetry: () => void;
  onOpenSettings?: () => void;
  t: TranslateFn;
}) {
  const retryLabel = presentation.kind === "approval"
    ? t("aiChat.error.action.retryApproval")
    : presentation.canRetryTools
      ? t("aiChat.error.action.retryTools")
      : t("aiChat.error.action.retry");
  const showRetry = canRetry && (presentation.canRetry || presentation.canRetryTools);
  // The card already shows the latest failure, so the disclosure only appears
  // once there is real history behind it: 2+ attempts overall.
  const attempts = (history ?? []).reduce((total, entry) => total + entry.count, 0);
  const showHistory = attempts > 1 && history !== undefined;

  return (
    <div className="ai-chat-error" role="status" data-kind={presentation.kind}>
      <span>{presentation.message}</span>
      <div className="ai-chat-error-actions">
        {showRetry && (
          <button type="button" onClick={onRetry}>
            <RotateCcw size={13} />
            <span>{retryLabel}</span>
          </button>
        )}
        {presentation.canOpenSettings && onOpenSettings && (
          <button type="button" onClick={onOpenSettings}>
            <span>{t("aiChat.error.action.openSettings")}</span>
          </button>
        )}
      </div>
      {showHistory && (
        <details className="ai-chat-error-history">
          <summary>
            <ChevronRight size={12} className="ai-chat-error-history-chevron" aria-hidden="true" />
            {t("aiChat.error.history", { count: attempts })}
          </summary>
          <ol className="ai-chat-error-history-list">
            {history.map((entry) => (
              <li key={`${entry.timestamp}-${entry.message}`}>
                <time dateTime={new Date(entry.timestamp).toISOString()}>{formatErrorTime(entry.timestamp)}</time>
                <span className="ai-chat-error-history-message">{entry.message}</span>
                {entry.count > 1 && <span className="ai-chat-error-history-count">×{entry.count}</span>}
              </li>
            ))}
          </ol>
        </details>
      )}
    </div>
  );
}
