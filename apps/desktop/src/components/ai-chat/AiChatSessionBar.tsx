import { Check, RotateCcw } from "lucide-react";
import { aiChatSessionTitle, aiChatStatusLabel } from "../../lib/aiChatPresentation";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { AiChatSession } from "../../lib/store";

type AiChatSessionBarProps = {
  activeSessionId: string;
  closeSession: (sessionId: string) => void;
  restoreSession: (sessionId: string) => void;
  sessions: AiChatSession[];
  setActiveSession: (sessionId: string) => void;
  t: TranslateFn;
};

export function AiChatSessionBar({ activeSessionId, closeSession, restoreSession, sessions, setActiveSession, t }: AiChatSessionBarProps) {
  return (
    <div className="ai-chat-session-bar" aria-label={t("aiChat.history.aria")}>
      {sessions.map((session) => {
        const active = session.id === activeSessionId;
        const title = aiChatSessionTitle(session.title, t);
        return (
          <div key={session.id} className="ai-chat-session-tab-wrap" data-active={active} data-closed={Boolean(session.closedAt)}>
            <button
              type="button"
              className="ai-chat-session-tab"
              data-active={active}
              data-closed={Boolean(session.closedAt)}
              onClick={() => setActiveSession(session.id)}
              onMouseDown={(event) => {
                if (event.button !== 1) return;
                event.preventDefault();
                closeSession(session.id);
              }}
              title={session.closedAt ? `${title} (${t("agent.chat.closed")})` : title}
            >
              <span className="ai-chat-session-check" aria-hidden="true">{active && <Check size={11} />}</span>
              <span>{title}</span>
              {session.status !== "idle" && <small>{aiChatStatusLabel(session.status, true, t)}</small>}
              {session.closedAt && <small>{t("agent.chat.closed")}</small>}
            </button>
            {session.closedAt ? (
              <button className="ai-chat-session-close" type="button" aria-label={t("aiChat.restoreChat")} title={t("aiChat.restoreChat")} onClick={() => restoreSession(session.id)}>
                <RotateCcw size={11} />
              </button>
            ) : (
              <button className="ai-chat-session-close" type="button" aria-label={t("aiChat.closeChat")} title={t("aiChat.closeChat")} onClick={() => closeSession(session.id)}>
                <Check size={11} />
              </button>
            )}
          </div>
        );
      })}
    </div>
  );
}

export function compareAiChatSessions(left: { closedAt: number | null; updatedAt: number }, right: { closedAt: number | null; updatedAt: number }) {
  if (!left.closedAt && right.closedAt) return -1;
  if (left.closedAt && !right.closedAt) return 1;
  return right.updatedAt - left.updatedAt;
}
