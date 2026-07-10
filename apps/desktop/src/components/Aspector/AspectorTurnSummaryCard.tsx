import { Check, ChevronRight, Clock3, Coins, FilePenLine, Repeat2, SearchCheck, Zap } from "lucide-react";
import { useMemo, useState } from "react";
import type { AiChatMessage } from '../../lib/aspector/chat/types';
import { formatCompactTokens } from '../../lib/aspector/chat/context-usage';
import { buildTurnFileSummary } from '../../lib/aspector/utils/turn-file-summary';
import { openWorkspaceEditorPath } from '../../lib/editor/open-workspace-editor-path';
import type { TranslateFn } from '../../lib/i18n/useTranslation';

type AspectorTurnSummaryCardProps = {
  message: AiChatMessage;
  workspaceRoot: string | null;
  t: TranslateFn;
  onReview?: () => void;
  /** Grey out (never hide) Review while a turn is running — a vanishing button reads as broken. */
  reviewDisabled?: boolean;
};

export function AspectorTurnSummaryCard({ message, workspaceRoot, t, onReview, reviewDisabled = false }: AspectorTurnSummaryCardProps) {
  const [filesExpanded, setFilesExpanded] = useState(false);
  const fileSummary = useMemo(() => buildTurnFileSummary(message, workspaceRoot), [message, workspaceRoot]);
  const usage = message.turnUsage;
  const timing = message.responseTiming;
  // The native turn-loop reports only responseDurationMs (no per-phase timing),
  // so fall back to it for the total when responseTiming is absent.
  const totalDurationMs = timing?.totalMs ?? message.responseDurationMs ?? 0;

  const hasFiles = Boolean(fileSummary && fileSummary.files.length > 0);
  const hasUsage = Boolean(usage && (usage.promptTokens > 0 || usage.completionTokens > 0));
  const hasTiming = totalDurationMs > 0;
  const showReview = Boolean(onReview) && (hasFiles || hasUsage || hasTiming);

  if (!hasFiles && !hasUsage && !hasTiming) return null;

  // The file list is collapsed by default behind the header (filesExpanded), so the
  // summary stays a single quiet line; clicking the header toggles it open/closed.
  const visibleFiles = fileSummary?.files ?? [];

  return (
    <div className="ai-turn-summary-card" role="status">
      <header className="ai-turn-summary-head">
        <div className="ai-turn-summary-status-row">
          <span className="ai-turn-summary-status">
            <Check size={14} />
            {t("aiChat.turnSummary.done")}
          </span>
          {showReview && onReview && (
            <button
              type="button"
              className="ai-turn-summary-review"
              onClick={onReview}
              disabled={reviewDisabled}
              title={t("aiChat.turnSummary.reviewHint")}
              aria-label={t("aiChat.turnSummary.review")}
            >
              <SearchCheck size={13} />
              <span>{t("aiChat.turnSummary.review")}</span>
            </button>
          )}
        </div>
        <div className="ai-turn-summary-metrics">
          {hasTiming && (
            <span title={timing
              ? t("aiChat.turnUsage.timing", { total: timing.totalMs, model: timing.modelMs, tools: timing.toolMs })
              : formatDuration(totalDurationMs)}>
              <Clock3 size={12} />
              {formatDuration(totalDurationMs)}
            </span>
          )}
          {usage?.requestCount && usage.requestCount > 0 ? (
            <span title={t("aiChat.turnUsage.requests", { count: usage.requestCount })}>
              <Repeat2 size={12} />
              {usage.requestCount} req
            </span>
          ) : null}
          {hasUsage && usage && (
            <span title={t("aiChat.turnUsage.summary", {
              input: formatCompactTokens(usage.promptTokens),
              output: formatCompactTokens(usage.completionTokens),
              total: formatCompactTokens(usage.totalTokens),
            })}
            >
              <Coins size={12} />
              {formatCompactTokens(usage.promptTokens)} in · {formatCompactTokens(usage.completionTokens)} out · {formatCompactTokens(usage.totalTokens)} tot
            </span>
          )}
          {usage?.cachedPromptTokens && usage.cachedPromptTokens > 0 ? (
            <span
              className="ai-turn-summary-cache"
              title={t("aiChat.turnUsage.cacheHit", {
                cached: formatCompactTokens(usage.cachedPromptTokens),
                percent: usage.promptTokens > 0 ? Math.round((usage.cachedPromptTokens / usage.promptTokens) * 100) : 0,
              })}
            >
              <Zap size={12} />
              {formatCompactTokens(usage.cachedPromptTokens)} cached
            </span>
          ) : null}
        </div>
      </header>

      {hasFiles && fileSummary && (
        <section className="ai-turn-summary-files">
          <button
            type="button"
            className="ai-turn-summary-files-head"
            onClick={() => setFilesExpanded((value) => !value)}
            aria-expanded={filesExpanded}
          >
            <FilePenLine size={13} />
            <span>
              {t("aiChat.turnSummary.filesChanged", {
                count: fileSummary.files.length,
                added: fileSummary.totalLinesAdded,
                removed: fileSummary.totalLinesRemoved,
              })}
            </span>
            <ChevronRight className="ai-turn-summary-files-caret" size={13} />
          </button>
          {filesExpanded && (
          <ul>
            {visibleFiles.map((file) => (
              <li key={file.path}>
                <button type="button" className="ai-turn-summary-file" onClick={() => void openWorkspaceEditorPath(file.path)}>
                  <span className="ai-turn-summary-file-path" title={file.path}>{file.displayPath}</span>
                  <span className="ai-turn-summary-file-stats">
                    {file.linesAdded > 0 && <span data-kind="add">+{file.linesAdded}</span>}
                    {file.linesRemoved > 0 && <span data-kind="remove">−{file.linesRemoved}</span>}
                    {file.filesCreated > 0 && <span data-kind="create">new</span>}
                    {file.filesDeleted > 0 && <span data-kind="delete">del</span>}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          )}
        </section>
      )}

    </div>
  );
}

// Human-readable elapsed time. Scales the unit up as needed:
//   <1s → ms, <60s → s, <60m → m s, <24h → h m, else → d h.
function formatDuration(ms: number) {
  if (ms < 1000) return `${ms}ms`;
  const totalSeconds = Math.round(ms / 1000);
  if (totalSeconds < 60) return `${(ms / 1000).toFixed(1)}s`;

  const seconds = totalSeconds % 60;
  const totalMinutes = Math.floor(totalSeconds / 60);
  if (totalMinutes < 60) return `${totalMinutes}m ${seconds}s`;

  const minutes = totalMinutes % 60;
  const totalHours = Math.floor(totalMinutes / 60);
  if (totalHours < 24) return `${totalHours}h ${minutes}m`;

  const hours = totalHours % 24;
  const days = Math.floor(totalHours / 24);
  return `${days}d ${hours}h`;
}