import { X } from "lucide-react";
import { useEffect, useMemo, useRef, type CSSProperties } from "react";
import {
  formatAiChatContextValue,
  formatCompactTokens,
  type AiChatContextUsageMeta,
  type AiChatContextUsageRow,
  type AiChatContextUsageSummary,
} from "../../lib/aiChatContextUsage";
import type { AiChatContextDropSummary } from "../../lib/aiChatContextReport";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

const ringSize = 22;
const ringStroke = 2.5;
const ringRadius = (ringSize - ringStroke) / 2;
const ringCircumference = 2 * Math.PI * ringRadius;

type AiContextIndicatorProps = {
  contextOpen: boolean;
  contextTitle: string;
  contextUsage: AiChatContextUsageSummary & AiChatContextUsageMeta;
  contextDrops?: AiChatContextDropSummary | null;
  onCompact?: () => void;
  onOpenSettings?: () => void;
  setContextOpen: (open: boolean | ((open: boolean) => boolean)) => void;
  t: TranslateFn;
};

export function AiContextIndicator({
  contextOpen,
  contextTitle,
  contextUsage,
  contextDrops,
  onCompact,
  onOpenSettings,
  setContextOpen,
  t,
}: AiContextIndicatorProps) {
  const anchorRef = useRef<HTMLDivElement | null>(null);
  const level = contextLevel(contextUsage.percent);
  const visibleRows = useMemo(
    () => [...contextUsage.rows]
      .filter((row) => row.tokens > 0)
      .sort((left, right) => right.tokens - left.tokens),
    [contextUsage.rows],
  );
  const meterRows = useMemo(() => {
    const total = visibleRows.reduce((sum, row) => sum + row.tokens, 0);
    if (total <= 0) return visibleRows;
    return visibleRows.map((row) => ({
      ...row,
      percent: Math.max(1.5, (row.tokens / total) * 100),
    }));
  }, [visibleRows]);

  useEffect(() => {
    if (!contextOpen) return undefined;

    const onPointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (anchorRef.current?.contains(target)) return;
      setContextOpen(false);
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setContextOpen(false);
    };

    document.addEventListener("pointerdown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [contextOpen, setContextOpen]);

  return (
    <div className="ai-context-anchor" ref={anchorRef}>
      {contextOpen && (
        <AiContextPopover
          contextUsage={contextUsage}
          level={level}
          meterRows={meterRows}
          onCompact={onCompact}
          onOpenSettings={onOpenSettings}
          setContextOpen={setContextOpen}
          t={t}
          visibleRows={visibleRows}
          contextDrops={contextDrops}
        />
      )}
      <button
        type="button"
        className="ai-context-ring"
        aria-label={t("aiChat.context.label")}
        aria-expanded={contextOpen}
        aria-haspopup="dialog"
        title={contextTitle}
        data-active={contextOpen || undefined}
        data-level={level}
        onClick={() => setContextOpen((open) => !open)}
      >
        <ContextRingSvg
          className="ai-context-ring-svg"
          level={level}
          percent={contextUsage.percent}
          size={ringSize}
        />
        {(contextUsage.percent >= 10 || contextOpen) && (
          <span className="ai-context-ring-label" aria-hidden="true">
            {contextUsage.percent}
          </span>
        )}
      </button>
    </div>
  );
}

function AiContextPopover({
  contextUsage,
  level,
  meterRows,
  onCompact,
  onOpenSettings,
  setContextOpen,
  t,
  visibleRows,
  contextDrops,
}: {
  contextUsage: AiChatContextUsageSummary & AiChatContextUsageMeta;
  level: ContextLevel;
  meterRows: AiChatContextUsageRow[];
  onCompact?: () => void;
  onOpenSettings?: () => void;
  setContextOpen: (open: boolean | ((open: boolean) => boolean)) => void;
  t: TranslateFn;
  visibleRows: AiChatContextUsageRow[];
  contextDrops?: AiChatContextDropSummary | null;
}) {
  return (
    <div className="ai-context-panel" role="dialog" aria-label={t("aiChat.context.label")}>
      <header className="ai-context-panel-head">
        <div className="ai-context-panel-summary">
          <div className="ai-context-panel-ring" data-level={level} aria-hidden="true">
            <ContextRingSvg
              className="ai-context-panel-ring-svg"
              level={level}
              percent={contextUsage.percent}
              size={40}
            />
            <span className="ai-context-panel-ring-value">{contextUsage.percent}%</span>
          </div>
          <div className="ai-context-panel-stats">
            <span className="ai-context-panel-title">{t("aiChat.context.label")}</span>
            <strong>{t("aiChat.context.tokenUsage", {
              totalTokens: formatCompactTokens(contextUsage.totalTokens),
              tokenBudget: formatCompactTokens(contextUsage.tokenBudget),
            })}
            </strong>
            <p className="ai-context-panel-sub">
              {contextUsage.autoCompactEnabled
                ? t("aiChat.context.autoCompactOn", {
                  percent: contextUsage.autoCompactThresholdPercent,
                  trigger: formatCompactTokens(contextUsage.compactTriggerTokens),
                })
                : t("aiChat.context.autoCompactOff")}
            </p>
          </div>
        </div>
        <button
          type="button"
          className="ai-context-panel-close"
          aria-label={t("aiChat.context.closeAria")}
          title={t("common.close")}
          onClick={() => setContextOpen(false)}
        >
          <X size={14} />
        </button>
      </header>

      {meterRows.length > 0 ? (
        <div className="ai-context-panel-meter" aria-hidden="true">
          {meterRows.map((row) => (
            <span
              key={row.id}
              className="ai-context-panel-meter-segment"
              style={{ width: `${row.percent}%`, background: row.color } as CSSProperties}
            />
          ))}
        </div>
      ) : null}

      <div className="ai-context-panel-section">
        <span className="ai-context-panel-section-label">{t("aiChat.context.distribution")}</span>
        <span className="ai-context-panel-section-hint">{t("aiChat.context.estimatedUsage")}</span>
      </div>

      {visibleRows.length > 0 ? (
        <ul className="ai-context-panel-rows">
          {visibleRows.map((row) => (
            <ContextRow key={row.id} row={row} tokenBudget={contextUsage.tokenBudget} />
          ))}
        </ul>
      ) : (
        <p className="ai-context-panel-empty">{t("aiChat.context.empty")}</p>
      )}

      {contextDrops && contextDrops.totalDroppedCount > 0 && (
        <div className="ai-context-panel-section ai-context-panel-drops">
          <span className="ai-context-panel-section-label">{t("aiChat.context.dropsTitle")}</span>
          <span className="ai-context-panel-section-hint">
            {t("aiChat.context.dropsSummary", {
              count: contextDrops.totalDroppedCount,
              tokens: formatCompactTokens(contextDrops.totalDroppedTokens),
            })}
          </span>
        </div>
      )}
      {contextDrops && contextDrops.entries.length > 0 && (
        <ul className="ai-context-panel-drop-rows">
          {contextDrops.entries.slice(0, 24).map((entry) => (
            <li key={`${entry.id}-${entry.reason}`} className="ai-context-panel-drop-row">
              <div className="ai-context-panel-drop-head">
                <span>{entry.label}</span>
                <span>{formatCompactTokens(entry.tokens)}</span>
              </div>
              <p>{t(`aiChat.context.dropReason.${entry.reason}` as "aiChat.context.dropReason.budget-cap")}</p>
              {entry.detail ? <small title={entry.detail}>{entry.detail}</small> : null}
            </li>
          ))}
        </ul>
      )}

      {(onCompact || onOpenSettings) && (
        <footer className="ai-context-panel-footer">
          {onCompact && (
            <button type="button" onClick={() => { onCompact(); setContextOpen(false); }}>
              {t("aiChat.context.compactNow")}
            </button>
          )}
          {onOpenSettings && (
            <button type="button" onClick={() => { onOpenSettings(); setContextOpen(false); }}>
              {t("aiChat.context.openSettings")}
            </button>
          )}
        </footer>
      )}
    </div>
  );
}

/**
 * One distribution line, Codex-quiet: dot · label · tokens · share. The stacked
 * meter above already visualizes proportions, so rows skip per-row bars; the
 * long-form detail (file lists etc.) lives in the row tooltip.
 */
function ContextRow({
  row,
  tokenBudget,
}: {
  row: AiChatContextUsageRow;
  tokenBudget: number;
}) {
  const budgetPercent = tokenBudget > 0 ? Math.min(100, Math.round((row.tokens / tokenBudget) * 100)) : 0;

  return (
    <li className="ai-context-panel-row" title={row.detail || undefined}>
      <span className="ai-context-panel-row-dot" style={{ background: row.color }} aria-hidden="true" />
      <span className="ai-context-panel-row-label">{row.label}</span>
      <span className="ai-context-panel-row-metrics">
        <span className="ai-context-panel-row-value">{formatCompactTokens(row.tokens)}</span>
        <span className="ai-context-panel-row-percent">{budgetPercent > 0 ? `${budgetPercent}%` : "·"}</span>
      </span>
      <span className="sr-only">{formatAiChatContextValue(row)}</span>
    </li>
  );
}

function ContextRingSvg({
  className,
  level,
  percent,
  size,
}: {
  className?: string;
  level: ContextLevel;
  percent: number;
  size: number;
}) {
  const scale = size / ringSize;
  const stroke = ringStroke * scale;
  const radius = (size - stroke) / 2;
  const circumference = 2 * Math.PI * radius;
  const clamped = Math.min(100, Math.max(0, percent));
  const ringOffset = circumference - (clamped / 100) * circumference;

  return (
    <svg
      className={className}
      viewBox={`0 0 ${size} ${size}`}
      width={size}
      height={size}
      aria-hidden="true"
      data-level={level}
    >
      <circle
        className="ai-context-ring-track"
        cx={size / 2}
        cy={size / 2}
        r={radius}
        fill="none"
        strokeWidth={stroke}
      />
      <circle
        className="ai-context-ring-progress"
        cx={size / 2}
        cy={size / 2}
        r={radius}
        fill="none"
        strokeWidth={stroke}
        strokeLinecap="round"
        strokeDasharray={circumference}
        strokeDashoffset={ringOffset}
        transform={`rotate(-90 ${size / 2} ${size / 2})`}
      />
    </svg>
  );
}

type ContextLevel = "low" | "medium" | "high";

function contextLevel(percent: number): ContextLevel {
  if (percent >= 82) return "high";
  if (percent >= 58) return "medium";
  return "low";
}