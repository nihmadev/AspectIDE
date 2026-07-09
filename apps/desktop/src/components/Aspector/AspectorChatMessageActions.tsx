import { Pencil, Undo2 } from "lucide-react";
import type { AiChatMessage } from "../../lib/aspector/chat/types";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AspectorChatMessageActionsProps = {
  canMutate: boolean;
  canRestoreUser: boolean;
  /** Inline edit is active — the bubble itself is the editor, so hide the triggers. */
  editing: boolean;
  message: AiChatMessage;
  onStartEdit: () => void;
  onRestore: () => void;
  t: TranslateFn;
};

/**
 * Compact per-message affordances for a checkpointed user message: start the
 * in-place edit (the bubble text itself becomes editable — no separate framed
 * editor), or roll straight back to the pre-turn snapshot without resending.
 */
export function AspectorChatMessageActions({
  canMutate,
  canRestoreUser,
  editing,
  message,
  onStartEdit,
  onRestore,
  t,
}: AspectorChatMessageActionsProps) {
  if (!canMutate || editing) return null;
  if (message.role !== "user" || !canRestoreUser) return null;

  return (
    <div className="ai-chat-message-actions" data-role="user">
      <button type="button" title={t("aiChat.turnCheckpoint.editHint")} onClick={onStartEdit}>
        <Pencil size={12} />
        <span>{t("aiChat.turnCheckpoint.edit")}</span>
      </button>
      <button type="button" title={t("aiChat.turnCheckpoint.restoreHere")} onClick={onRestore}>
        <Undo2 size={12} />
        <span>{t("aiChat.turnCheckpoint.restore")}</span>
      </button>
    </div>
  );
}
