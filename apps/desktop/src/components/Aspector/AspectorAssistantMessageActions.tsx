import { Square } from "lucide-react";
import type { TranslateFn } from '../../lib/i18n/useTranslation';

type AspectorAssistantMessageActionsProps = {
  canMutate: boolean;
  canStopAfterTool: boolean;
  onStopAfterTool: () => void;
  t: TranslateFn;
};

export function AspectorAssistantMessageActions({
  canMutate,
  canStopAfterTool,
  onStopAfterTool,
  t,
}: AspectorAssistantMessageActionsProps) {
  if (!canMutate || !canStopAfterTool) return null;

  return (
    <div className="ai-chat-message-actions" data-role="assistant">
      <button type="button" title={t("aiChat.assistant.stopAfterTool")} onClick={onStopAfterTool}>
        <Square size={12} />
        <span>{t("aiChat.assistant.stopAfterTool")}</span>
      </button>
    </div>
  );
}