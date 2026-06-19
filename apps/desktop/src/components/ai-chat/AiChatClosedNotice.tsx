import { MessageSquarePlus } from "lucide-react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

/** Notice shown when the active chat session has been closed, offering to restore it. */
export function AiChatClosedNotice({ onRestore, t }: { onRestore: () => void; t: TranslateFn }) {
  return (
    <div className="ai-chat-closed-notice" role="status">
      <span>{t("aiChat.closedNotice")}</span>
      <button type="button" onClick={onRestore}>
        <MessageSquarePlus size={13} />
        <span>{t("aiChat.restoreChat")}</span>
      </button>
    </div>
  );
}
