import { Check, ChevronDown, ChevronUp, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useSyncExternalStore } from "react";
import {
  acceptAllPendingFileReviews,
  acceptPendingFileReview,
  acceptPendingFileReviewHunk,
  listPendingFileReviewsForPath,
  rejectAllPendingFileReviews,
  rejectPendingFileReview,
  rejectPendingFileReviewHunk,
  subscribePendingFileReviews,
  getPendingFileReviewsSnapshot,
  type PendingFileReview,
} from "../../lib/aspector/utils/pending-file-review";
import { subscribeFileReviewFocus } from "../../lib/aspector/utils/file-review/bridge";
import { useTranslation } from "../../lib/i18n/useTranslation";
import { AspectorMonacoDiffReview } from "./AspectorMonacoDiffReview";

type AspectorFileReviewBarProps = {
  documentPath: string | null;
  sessionId: string | null;
};

export function AspectorFileReviewBar({ documentPath, sessionId }: AspectorFileReviewBarProps) {
  const { t } = useTranslation();
  const reviews = useSyncExternalStore(subscribePendingFileReviews, getPendingFileReviewsSnapshot, getPendingFileReviewsSnapshot);
  const [expanded, setExpanded] = useState(true);
  const [activeHunkId, setActiveHunkId] = useState<string | null>(null);
  const pending = useMemo(() => {
    if (!documentPath) return [];
    return listPendingFileReviewsForPath(documentPath);
  }, [documentPath, reviews]);
  const sessionPendingCount = useMemo(
    () => (sessionId ? reviews.filter((review) => review.sessionId === sessionId && review.status === "pending").length : 0),
    [reviews, sessionId],
  );

  useEffect(() => {
    const unsubscribe = subscribeFileReviewFocus((request) => {
      if (!documentPath || normalizePathKey(request.path) !== normalizePathKey(documentPath)) return;
      setExpanded(true);
      if (request.hunkId) setActiveHunkId(request.hunkId);
    });
    return () => {
      unsubscribe();
    };
  }, [documentPath]);

  if (!documentPath || pending.length === 0) return null;
  const review = pending[0];
  const language = inferLanguage(documentPath);
  // Resolve the selected hunk to its real modified-text start line so the diff
  // editor reveals the actual location instead of an estimated ordinal.
  const activeHunkLine = activeHunkId
    ? review.hunks.find((hunk) => hunk.id === activeHunkId)?.afterStartLine ?? null
    : null;

  return (
    <div className="ai-file-review-bar" data-expanded={expanded} data-preview={review.previewOnly || undefined}>
      <header className="ai-file-review-head">
        <div>
          <strong>{t("aiChat.review.title")}</strong>
          <span>
            {review.previewOnly
              ? t("aiChat.review.subtitlePreview", { tool: review.toolName })
              : t("aiChat.review.subtitle", { tool: review.toolName })}
          </span>
        </div>
        <div className="ai-file-review-head-actions">
          {sessionPendingCount > 1 && (
            <>
              <button type="button" onClick={() => void acceptAllPendingFileReviews(sessionId ?? undefined)}>
                {t("aiChat.review.acceptAll")}
              </button>
              <button type="button" onClick={() => void rejectAllPendingFileReviews(sessionId ?? undefined)}>
                {t("aiChat.review.rejectAll")}
              </button>
            </>
          )}
          <button type="button" aria-label={expanded ? t("aiChat.review.collapse") : t("aiChat.review.expand")} onClick={() => setExpanded((value) => !value)}>
            {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
          </button>
        </div>
      </header>
      {expanded && (
        <>
          {review.hunks.length > 1 && (
            <div className="ai-file-review-hunks" role="list">
              {review.hunks.map((hunk) => {
                const accepted = review.acceptedHunkIds.includes(hunk.id);
                return (
                  <div key={hunk.id} className="ai-file-review-hunk" data-kind={hunk.kind} data-active={activeHunkId === hunk.id || undefined}>
                    <button type="button" className="ai-file-review-hunk-jump" onClick={() => setActiveHunkId(hunk.id)}>
                      {t("aiChat.review.hunkLabel", {
                        start: hunk.afterStartLine,
                        added: hunk.afterLineCount,
                        removed: hunk.beforeLineCount,
                      })}
                    </button>
                    <div className="ai-file-review-hunk-actions">
                      <button
                        type="button"
                        className={accepted ? "active" : undefined}
                        onClick={() => void acceptPendingFileReviewHunk(review.id, hunk.id)}
                      >
                        {t("aiChat.review.acceptHunk")}
                      </button>
                      <button
                        type="button"
                        className={!accepted ? "active" : undefined}
                        onClick={() => void rejectPendingFileReviewHunk(review.id, hunk.id)}
                      >
                        {t("aiChat.review.rejectHunk")}
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
          <AspectorMonacoDiffReview
            beforeText={review.beforeText}
            afterText={review.afterText}
            language={language}
            activeHunkLine={activeHunkLine}
          />
          <div className="ai-file-review-actions">
            <button type="button" className="primary" onClick={() => void acceptPendingFileReview(review.id)}>
              <Check size={14} />
              <span>{t("aiChat.review.accept")}</span>
            </button>
            <button type="button" onClick={() => void rejectPendingFileReview(review.id)}>
              <X size={14} />
              <span>{t("aiChat.review.reject")}</span>
            </button>
          </div>
        </>
      )}
    </div>
  );
}

function inferLanguage(path: string) {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  const map: Record<string, string> = {
    ts: "typescript", tsx: "typescript", js: "javascript", jsx: "javascript",
    rs: "rust", py: "python", go: "go", json: "json", md: "markdown",
    css: "css", html: "html", sql: "sql", yaml: "yaml", yml: "yaml",
  };
  return map[ext] ?? "plaintext";
}

function normalizePathKey(path: string) {
  return path.replace(/\\/g, "/").toLowerCase();
}