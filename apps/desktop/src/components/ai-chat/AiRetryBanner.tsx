import { ChevronRight, Loader2 } from "lucide-react";
import { useEffect, useState, useSyncExternalStore } from "react";
import { getAiRetryNotice, getAiRetryNoticeRevision, subscribeAiRetryNotice } from "../../lib/aiRetryNotice";
import type { AiChatErrorHistoryEntry } from "../../lib/store";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

function formatRetryErrorTime(timestamp: number) {
  return new Date(timestamp).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

/** Amber auto-retry banner with a live backoff countdown, driven by the retry-notice
 *  store. While it hammers, an expandable "error history" discloses the failures the
 *  ladder has hit so far (the scary error card is hidden during backoff). */
export function AiRetryBanner({ sessionId, history, t }: { sessionId: string; history?: AiChatErrorHistoryEntry[]; t: TranslateFn }) {
  useSyncExternalStore(subscribeAiRetryNotice, getAiRetryNoticeRevision, getAiRetryNoticeRevision);
  const notice = getAiRetryNotice(sessionId);
  const [now, setNow] = useState(() => Date.now());

  // While a backoff is counting down, tick a few times a second so the live
  // "retrying in Ns" stays accurate. The interval only runs while a notice with a
  // real delay is present, and resets whenever a fresh retry arrives (updatedAt).
  const updatedAt = notice?.updatedAt ?? 0;
  const delayMs = notice?.delayMs ?? 0;
  useEffect(() => {
    if (!updatedAt || delayMs <= 0) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 250);
    return () => window.clearInterval(id);
  }, [updatedAt, delayMs]);

  if (!notice) return null;
  const reasonLabel = t(`aiChat.retryNotice.reason.${notice.reason}` as "aiChat.retryNotice.reason.generic");
  const remainingMs = Math.max(0, notice.delayMs - (now - notice.updatedAt));
  const seconds = Math.ceil(remainingMs / 1000);
  const countdown = remainingMs > 0
    ? t("aiChat.retryNotice.countdown", { seconds })
    : t("aiChat.retryNotice.reconnecting");
  const attempts = (history ?? []).reduce((total, entry) => total + entry.count, 0);
  const showHistory = attempts > 1 && history !== undefined;
  return (
    <div className="ai-retry-notice" role="status" aria-live="polite" data-reason={notice.reason}>
      <Loader2 size={13} className="ai-retry-notice-spin" aria-hidden="true" />
      <span className="ai-retry-notice-text">
        <strong>{t("aiChat.retryNotice.title")}</strong>
        <span>{t("aiChat.retryNotice.body", { reason: reasonLabel, attempt: notice.attempt, max: notice.maxAttempts })}</span>
        <span className="ai-retry-notice-countdown">{countdown}</span>
      </span>
      {showHistory && (
        <details className="ai-chat-error-history ai-retry-notice-history">
          <summary>
            <ChevronRight size={12} className="ai-chat-error-history-chevron" aria-hidden="true" />
            {t("aiChat.error.history", { count: attempts })}
          </summary>
          <ol className="ai-chat-error-history-list">
            {history.map((entry) => (
              <li key={`${entry.timestamp}-${entry.message}`}>
                <time dateTime={new Date(entry.timestamp).toISOString()}>{formatRetryErrorTime(entry.timestamp)}</time>
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
