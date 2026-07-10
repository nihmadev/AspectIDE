import { Check, Lightbulb, ListPlus, Pencil, SendHorizontal, Trash2, X } from "lucide-react";
import { useState } from "react";
import {
  removeQueuedMessage,
  updateQueuedMessage,
  useQueuedMessages,
  type QueuedMessage,
} from '../../lib/aspector/chat/queue';
import type { TranslateFn } from '../../lib/i18n/useTranslation';

type AspectorChatQueuedMessagesProps = {
  sessionId: string | null;
  /** Force-send a single queued entry right now (parent removes it + sends). */
  onSendNow: (entry: QueuedMessage) => void;
  t: TranslateFn;
};

/**
 * Chips for messages staged while the agent is busy (see aiChatQueue). Each shows the
 * mode (queued vs recommendation) and offers edit / delete / send-now — like Codex's
 * recommendation queue, but editable and mode-aware. They auto-send when the turn ends.
 */
export function AspectorChatQueuedMessages({ sessionId, onSendNow, t }: AspectorChatQueuedMessagesProps) {
  const queued = useQueuedMessages(sessionId);
  if (queued.length === 0) return null;
  return (
    <div className="ai-chat-queue" aria-label={t("aiChat.queue.aria")}>
      <div className="ai-chat-queue-head">
        <ListPlus size={13} />
        <span>{t("aiChat.queue.title", { count: queued.length })}</span>
        <span className="ai-chat-queue-head-hint">{t("aiChat.queue.headHint")}</span>
      </div>
      <div className="ai-chat-queue-list">
        {queued.map((entry, index) => (
          <AspectorQueuedChip key={entry.id} entry={entry} index={index} onSendNow={onSendNow} t={t} />
        ))}
      </div>
    </div>
  );
}

function AspectorQueuedChip({ entry, index, onSendNow, t }: { entry: QueuedMessage; index: number; onSendNow: (entry: QueuedMessage) => void; t: TranslateFn }) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(entry.text);

  const commit = () => {
    const next = draft.trim();
    if (next && next !== entry.text) updateQueuedMessage(entry.id, { text: next });
    setEditing(false);
  };

  const isRecommendation = entry.mode === "recommendation";
  // Once a recommendation has been folded into the running turn, its chip must
  // stop offering Send-now / mode-toggle / edit — those would race the in-flight
  // injection into a duplicate turn. It stays visible (as "sending…") until Rust
  // confirms the fold-in and the entry is removed.
  const injected = Boolean(entry.injectedTurnId);

  if (injected) {
    return (
      <div className="ai-chat-queue-chip" data-mode={entry.mode} data-injected="true">
        <span className="ai-chat-queue-chip-index" aria-hidden="true">{index + 1}</span>
        <span className="ai-chat-queue-chip-text">
          <span className="ai-chat-queue-chip-tag">
            <Lightbulb size={11} />
            {t("aiChat.queue.sending")}
          </span>
          <span className="ai-chat-queue-chip-body" title={entry.text}>{entry.text}</span>
        </span>
      </div>
    );
  }

  return (
    <div className="ai-chat-queue-chip" data-mode={entry.mode} data-editing={editing || undefined}>
      <span className="ai-chat-queue-chip-index" aria-hidden="true">{index + 1}</span>
      {editing ? (
        <textarea
          className="ai-chat-queue-chip-edit"
          value={draft}
          autoFocus
          rows={1}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              commit();
            }
            if (event.key === "Escape") {
              setDraft(entry.text);
              setEditing(false);
            }
          }}
          onBlur={commit}
        />
      ) : (
        <span className="ai-chat-queue-chip-text">
          <span
            className="ai-chat-queue-chip-tag"
            title={isRecommendation ? t("aiChat.queue.modeRecommendationHint") : t("aiChat.queue.modeQueuedHint")}
          >
            {isRecommendation ? <Lightbulb size={11} /> : <ListPlus size={11} />}
            {isRecommendation ? t("aiChat.queue.modeRecommendation") : t("aiChat.queue.modeQueued")}
          </span>
          <span className="ai-chat-queue-chip-body" title={entry.text}>{entry.text}</span>
        </span>
      )}
      <div className="ai-chat-queue-chip-actions">
        {editing ? (
          <button type="button" title={t("aiChat.queue.save")} aria-label={t("aiChat.queue.save")} onClick={commit}>
            <Check size={13} />
          </button>
        ) : (
          <>
            <button
              type="button"
              className="ai-chat-queue-chip-mode"
              data-active={isRecommendation || undefined}
              title={isRecommendation ? t("aiChat.queue.markQueued") : t("aiChat.queue.markRecommendation")}
              aria-label={isRecommendation ? t("aiChat.queue.markQueued") : t("aiChat.queue.markRecommendation")}
              aria-pressed={isRecommendation}
              onClick={() => updateQueuedMessage(entry.id, { mode: isRecommendation ? "queued" : "recommendation" })}
            >
              <Lightbulb size={13} />
            </button>
            <button type="button" title={t("aiChat.queue.sendNow")} aria-label={t("aiChat.queue.sendNow")} onClick={() => onSendNow(entry)}>
              <SendHorizontal size={13} />
            </button>
            <button type="button" title={t("aiChat.queue.edit")} aria-label={t("aiChat.queue.edit")} onClick={() => { setDraft(entry.text); setEditing(true); }}>
              <Pencil size={13} />
            </button>
          </>
        )}
        <button
          type="button"
          className="ai-chat-queue-chip-danger"
          title={editing ? t("aiChat.queue.cancel") : t("aiChat.queue.delete")}
          aria-label={editing ? t("aiChat.queue.cancel") : t("aiChat.queue.delete")}
          onClick={() => (editing ? (setDraft(entry.text), setEditing(false)) : removeQueuedMessage(entry.id))}
        >
          {editing ? <X size={13} /> : <Trash2 size={13} />}
        </button>
      </div>
    </div>
  );
}
