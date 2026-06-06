import type { AiChatMessage } from "../../lib/aiChatTypes";
import type { ContextCompactionState } from "../../lib/aiChatContextCompaction";
import { formatCompactTokens } from "../../lib/aiChatContextUsage";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiTurnDiagnosticsProps = {
  message: AiChatMessage;
  compaction?: ContextCompactionState | null;
  t: TranslateFn;
};

export function AiTurnDiagnostics({ message, compaction, t }: AiTurnDiagnosticsProps) {
  const usage = message.turnUsage;
  const timing = message.responseTiming;
  const hasUsage = Boolean(usage && (usage.promptTokens > 0 || usage.completionTokens > 0));
  const hasCompaction = Boolean(compaction?.droppedItems && compaction.droppedItems.length > 0);
  if (!hasUsage && !timing && !hasCompaction) return null;

  return (
    <div className="ai-turn-diagnostics" role="status">
      {hasUsage && usage && (
        <span className="ai-turn-diagnostics-usage">
          {t("aiChat.turnUsage.summary", {
            input: formatCompactTokens(usage.promptTokens),
            output: formatCompactTokens(usage.completionTokens),
            total: formatCompactTokens(usage.totalTokens),
          })}
        </span>
      )}
      {timing && (
        <span className="ai-turn-diagnostics-timing">
          {t("aiChat.turnUsage.timing", {
            total: timing.totalMs,
            model: timing.modelMs,
            tools: timing.toolMs,
          })}
        </span>
      )}
      {hasCompaction && compaction?.droppedItems && (
        <details className="ai-turn-diagnostics-compact">
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