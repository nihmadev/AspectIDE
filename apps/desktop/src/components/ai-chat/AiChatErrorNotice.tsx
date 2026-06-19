import { RotateCcw } from "lucide-react";
import type { AiChatErrorPresentation } from "../../lib/aiChatErrors";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

/** Inline chat error with optional retry + open-settings actions, styled by error kind. */
export function AiChatError({
  canRetry,
  presentation,
  onRetry,
  onOpenSettings,
  t,
}: {
  canRetry: boolean;
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
    </div>
  );
}
