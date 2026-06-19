import { Loader2 } from "lucide-react";
import { useEffect, useState, useSyncExternalStore } from "react";
import { getAiRetryNotice, getAiRetryNoticeRevision, subscribeAiRetryNotice } from "../../lib/aiRetryNotice";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

/** Amber auto-retry banner with a live backoff countdown, driven by the retry-notice store. */
export function AiRetryBanner({ sessionId, t }: { sessionId: string; t: TranslateFn }) {
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
  return (
    <div className="ai-retry-notice" role="status" aria-live="polite" data-reason={notice.reason}>
      <Loader2 size={13} className="ai-retry-notice-spin" aria-hidden="true" />
      <span className="ai-retry-notice-text">
        <strong>{t("aiChat.retryNotice.title")}</strong>
        <span>{t("aiChat.retryNotice.body", { reason: reasonLabel, attempt: notice.attempt, max: notice.maxAttempts })}</span>
        <span className="ai-retry-notice-countdown">{countdown}</span>
      </span>
    </div>
  );
}
