import { Pencil } from "lucide-react";
import { useState } from "react";
import type { AiChatMessage } from "../../lib/aiChatTypes";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiChatMessageActionsProps = {
  canMutate: boolean;
  canRestoreUser: boolean;
  message: AiChatMessage;
  onEditUserMessage: (messageId: string, nextContent: string) => void;
  t: TranslateFn;
};

export function AiChatMessageActions({
  canMutate,
  canRestoreUser,
  message,
  onEditUserMessage,
  t,
}: AiChatMessageActionsProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);

  if (!canMutate) return null;

  if (message.role === "user" && canRestoreUser) {
    return (
      <div className="ai-chat-message-actions" data-role="user">
        {editing ? (
          <div className="ai-chat-message-edit">
            <textarea value={draft} onChange={(event) => setDraft(event.target.value)} rows={3} />
            <div className="ai-chat-message-edit-actions">
              <button type="button" onClick={() => setEditing(false)}>{t("common.cancel")}</button>
              <button
                type="button"
                className="primary"
                disabled={!draft.trim()}
                onClick={() => {
                  setEditing(false);
                  onEditUserMessage(message.id, draft.trim());
                }}
              >
                {t("aiChat.turnCheckpoint.editResend")}
              </button>
            </div>
            <p className="ai-chat-message-edit-hint">{t("aiChat.turnCheckpoint.editHint")}</p>
          </div>
        ) : (
          <button
            type="button"
            title={t("aiChat.turnCheckpoint.edit")}
            onClick={() => { setDraft(message.content); setEditing(true); }}
          >
            <Pencil size={12} />
            <span>{t("aiChat.turnCheckpoint.edit")}</span>
          </button>
        )}
      </div>
    );
  }

  return null;
}
