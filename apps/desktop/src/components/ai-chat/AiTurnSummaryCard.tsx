import { Check, ChevronDown, ChevronRight, Clock3, Coins, FilePenLine, SearchCheck, Zap } from "lucide-react";
import { useMemo, useState } from "react";
import type { AiChatMessage } from "../../lib/aiChatTypes";
import type { ContextCompactionState } from "../../lib/aiChatContextCompaction";
import { formatCompactTokens } from "../../lib/aiChatContextUsage";
import { buildTurnFileSummary } from "../../lib/aiTurnFileSummary";
import { openWorkspaceEditorPath } from "../../lib/openWorkspaceEditorPath";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

const maxVisibleFiles = 5;

type AiTurnSummaryCardProps = {
  message: AiChatMessage;
  compaction?: ContextCompactionState | null;
  workspaceRoot: string | null;
  t: TranslateFn;
  onReview?: () => void;
};

export function AiTurnSummaryCard({ message, compaction, workspaceRoot, t, onReview }: AiTurnSummaryCardProps) {
  const [filesExpanded, setFilesExpanded] = useState(false);
  const fileSummary = useMemo(() => buildTurnFileSummary(message, workspaceRoot), [message, workspaceRoot]);
  const usage = message.turnUsage;
  const timing = message.responseTiming;

  const hasFiles = Boolean(fileSummary && fileSummary.files.length > 0);
  const hasUsage = Boolean(usage && (usage.promptTokens > 0 || usage.completionTokens > 0));
  const hasTiming = Boolean(timing && timing.totalMs > 0);
  const hasCompaction = Boolean(compaction?.droppedItems && compaction.droppedItems.length > 0);
  const showReview = Boolean(onReview) && (hasFiles || hasUsage || hasTiming);

  if (!hasFiles && !hasUsage && !hasTiming && !hasCompaction) return null;

  const visibleFiles = fileSummary
    ? (filesExpanded ? fileSummary.files : fileSummary.files.slice(0, maxVisibleFiles))
    : [];
  const hiddenFileCount = fileSummary ? Math.max(0, fileSummary.files.length - maxVisibleFiles) : 0;

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
              title={t("aiChat.turnSummary.reviewHint")}
              aria-label={t("aiChat.turnSummary.review")}
            >
              <SearchCheck size={13} />
              <span>{t("aiChat.turnSummary.review")}</span>
            </button>
          )}
        </div>
        <div className="ai-turn-summary-metrics">
          {hasTiming && timing && (
            <span title={t("aiChat.turnUsage.timing", { total: timing.totalMs, model: timing.modelMs, tools: timing.toolMs })}>
              <Clock3 size={12} />
              {formatDuration(timing.totalMs)}
            </span>
          )}
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
          <div className="ai-turn-summary-files-head">
            <FilePenLine size={13} />
            <span>
              {t("aiChat.turnSummary.filesChanged", {
                count: fileSummary.files.length,
                added: fileSummary.totalLinesAdded,
                removed: fileSummary.totalLinesRemoved,
              })}
            </span>
          </div>
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
          {hiddenFileCount > 0 && !filesExpanded && (
            <button type="button" className="ai-turn-summary-more" onClick={() => setFilesExpanded(true)}>
              <ChevronDown size={13} />
              {t("aiChat.turnSummary.moreFiles", { count: hiddenFileCount })}
            </button>
          )}
          {filesExpanded && fileSummary.files.length > maxVisibleFiles && (
            <button type="button" className="ai-turn-summary-more" onClick={() => setFilesExpanded(false)}>
              <ChevronRight size={13} />
              {t("aiChat.turnSummary.collapseFiles")}
            </button>
          )}
        </section>
      )}

      {hasCompaction && compaction?.droppedItems && (
        <details className="ai-turn-summary-compact">
          <summary>
            {t("aiChat.compact.droppedSummary", {
              count: compaction.droppedItems.length,
              tokens: formatCompactTokens(compaction.droppedTokens ?? 0),
            })}
          </summary>
          <ul>
            {compaction.droppedItems.map((item) => (
              <li key={`${item.kind}-${item.label}-${item.tokens}`}>
                <span>{item.label}</span>
                <span>{formatCompactTokens(item.tokens)}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

function formatDuration(ms: number) {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.floor(ms / 60_000)}m ${Math.round((ms % 60_000) / 1000)}s`;
}