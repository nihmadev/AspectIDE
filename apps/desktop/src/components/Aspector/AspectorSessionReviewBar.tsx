import { Check, ChevronDown, ChevronUp, FileDiff, Loader2, X } from "lucide-react";
import { useMemo, useState, useSyncExternalStore } from "react";
import {
  acceptAllPendingFileReviews,
  acceptPendingFileReview,
  getPendingFileReviewsSnapshot,
  rejectAllPendingFileReviews,
  rejectPendingFileReview,
  subscribePendingFileReviews,
  type PendingFileReview,
} from "../../lib/aspector/utils/pending-file-review";
import { requestFileReviewFocus } from "../../lib/aspector/utils/file-review/bridge";
import { openWorkspaceEditorPath } from "../../lib/editor/open-workspace-editor-path";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AspectorSessionReviewBarProps = {
  sessionId: string | null;
  t: TranslateFn;
};

/**
 * Chat-side review surface for the AI's pending file edits (Cursor-style).
 * The editor's AiFileReviewBar only appears when the touched file is OPEN in a
 * tab — in the dedicated Agent workspace there is no editor at all, which left
 * edits impossible to accept from the chat. This bar lives above the composer,
 * lists every pending edit of the active session with +/− stats, and offers
 * per-file Accept/Reject plus Accept-all/Reject-all. Clicking a row opens the
 * file in the editor and focuses its diff review.
 */
export function AspectorSessionReviewBar({ sessionId, t }: AspectorSessionReviewBarProps) {
  const reviews = useSyncExternalStore(subscribePendingFileReviews, getPendingFileReviewsSnapshot, getPendingFileReviewsSnapshot);
  const [collapsed, setCollapsed] = useState(false);
  const [busyIds, setBusyIds] = useState<ReadonlySet<string>>(new Set());
  const [bulkBusy, setBulkBusy] = useState(false);

  const pending = useMemo(
    () => (sessionId ? reviews.filter((review) => review.sessionId === sessionId && review.status === "pending") : []),
    [reviews, sessionId],
  );
  const totals = useMemo(() => {
    let added = 0;
    let removed = 0;
    for (const review of pending) {
      for (const hunk of review.hunks) {
        added += hunk.afterLineCount;
        removed += hunk.beforeLineCount;
      }
    }
    return { added, removed };
  }, [pending]);

  if (!sessionId || pending.length === 0) return null;

  const markBusy = (id: string, busy: boolean) => {
    setBusyIds((prev) => {
      const next = new Set(prev);
      if (busy) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  const acceptOne = async (review: PendingFileReview) => {
    markBusy(review.id, true);
    try {
      await acceptPendingFileReview(review.id);
    } catch {
      /* failure keeps the review pending; the editor bar shows details */
    } finally {
      markBusy(review.id, false);
    }
  };

  const rejectOne = async (review: PendingFileReview) => {
    markBusy(review.id, true);
    try {
      await rejectPendingFileReview(review.id);
    } catch {
      /* failure keeps the review pending */
    } finally {
      markBusy(review.id, false);
    }
  };

  const runBulk = async (action: (sessionId?: string) => Promise<void>) => {
    setBulkBusy(true);
    try {
      await action(sessionId);
    } finally {
      setBulkBusy(false);
    }
  };

  const openReview = (review: PendingFileReview) => {
    void openWorkspaceEditorPath(review.path);
    requestFileReviewFocus({ path: review.path, toolCallId: review.toolCallId });
  };

  return (
    <section className="ai-session-review" role="region" aria-label={t("aiChat.sessionReview.aria")}>
      <header className="ai-session-review-head">
        <span className="ai-session-review-icon" aria-hidden="true">
          <FileDiff size={13} />
        </span>
        <strong>{t("aiChat.sessionReview.title", { count: pending.length })}</strong>
        <span className="ai-session-review-stats">
          <span data-kind="add">+{totals.added}</span>
          <span data-kind="remove">−{totals.removed}</span>
        </span>
        <div className="ai-session-review-bulk">
          {bulkBusy ? (
            <Loader2 size={13} className="spin-icon" />
          ) : (
            <>
              <button type="button" className="primary" onClick={() => void runBulk(acceptAllPendingFileReviews)}>
                <Check size={12} />
                {t("aiChat.sessionReview.acceptAll")}
              </button>
              <button type="button" onClick={() => void runBulk(rejectAllPendingFileReviews)}>
                <X size={12} />
                {t("aiChat.sessionReview.rejectAll")}
              </button>
            </>
          )}
          <button
            type="button"
            className="ai-session-review-collapse"
            aria-label={collapsed ? t("aiChat.review.expand") : t("aiChat.review.collapse")}
            onClick={() => setCollapsed((value) => !value)}
          >
            {collapsed ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
          </button>
        </div>
      </header>
      {!collapsed && (
        <ul className="ai-session-review-list">
          {pending.map((review) => {
            const busy = busyIds.has(review.id) || bulkBusy;
            let added = 0;
            let removed = 0;
            for (const hunk of review.hunks) {
              added += hunk.afterLineCount;
              removed += hunk.beforeLineCount;
            }
            return (
              <li key={review.id} className="ai-session-review-row" data-busy={busy || undefined}>
                <button
                  type="button"
                  className="ai-session-review-file"
                  title={t("aiChat.sessionReview.open", { path: review.relativePath || review.path })}
                  onClick={() => openReview(review)}
                >
                  <span className="ai-session-review-name">{basename(review.path)}</span>
                  <span className="ai-session-review-dir">{parentDir(review.relativePath || review.path)}</span>
                  <span className="ai-session-review-row-stats">
                    {added > 0 && <span data-kind="add">+{added}</span>}
                    {removed > 0 && <span data-kind="remove">−{removed}</span>}
                  </span>
                </button>
                <div className="ai-session-review-row-actions">
                  {busy ? (
                    <Loader2 size={13} className="spin-icon" />
                  ) : (
                    <>
                      <button
                        type="button"
                        data-action="accept"
                        title={t("aiChat.sessionReview.accept")}
                        aria-label={t("aiChat.sessionReview.accept")}
                        onClick={() => void acceptOne(review)}
                      >
                        <Check size={13} />
                      </button>
                      <button
                        type="button"
                        data-action="reject"
                        title={t("aiChat.sessionReview.reject")}
                        aria-label={t("aiChat.sessionReview.reject")}
                        onClick={() => void rejectOne(review)}
                      >
                        <X size={13} />
                      </button>
                    </>
                  )}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}

function basename(path: string) {
  const parts = path.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] || path;
}

function parentDir(path: string) {
  const normalized = path.replace(/\\/g, "/");
  const index = normalized.lastIndexOf("/");
  return index > 0 ? normalized.slice(0, index) : "";
}
