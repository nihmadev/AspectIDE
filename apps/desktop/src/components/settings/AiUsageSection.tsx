import { ChevronRight, RefreshCw, Trash2 } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { formatCompactTokens } from '../../lib/aspector/chat/context-usage';
import {
  aggregateUsageByProject,
  clearAiUsageLog,
  loadAiUsageLog,
  reloadAiUsageLog,
  usageEntryTokensPerSecond,
  type AiUsageLogEntry,
} from '../../lib/aspector/utils/usage/usage-log';
import { workspaceInstructionsKey } from '../../lib/aspector/utils/preferences';
import type { MessageKey } from '../../lib/i18n';
import type { TranslateFn } from '../../lib/i18n/useTranslation';
import type { WorkspaceInfo } from '../../lib/types';

/** Time windows for the filter pills, in hours (null = everything). */
const TIME_FILTERS: Array<{ id: string; labelKey: MessageKey; hours: number | null }> = [
  { id: "today", labelKey: "settings.usage.filter.today", hours: 0 },
  { id: "24h", labelKey: "settings.usage.filter.h24", hours: 24 },
  { id: "48h", labelKey: "settings.usage.filter.h48", hours: 48 },
  { id: "72h", labelKey: "settings.usage.filter.h72", hours: 72 },
  { id: "7d", labelKey: "settings.usage.filter.d7", hours: 24 * 7 },
  { id: "all", labelKey: "settings.usage.filter.all", hours: null },
];

/** Rendered log rows are capped; aggregates always cover the whole filter window. */
const MAX_LOG_ROWS = 200;

/**
 * AI Usage — request-log console (FreeModel/observability style): time-window
 * pills, stat cards for the window, and a reverse-chronological request log
 * where each row expands into the full request breakdown.
 */
export function AiUsageSection({ t, workspace }: { t: TranslateFn; workspace: WorkspaceInfo | null }) {
  const [entries, setEntries] = useState<AiUsageLogEntry[] | null>(null);
  const [filterId, setFilterId] = useState("24h");
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const currentKey = workspaceInstructionsKey(workspace?.root);

  useEffect(() => {
    let active = true;
    void loadAiUsageLog().then((loaded) => { if (active) setEntries(loaded); });
    return () => { active = false; };
  }, []);

  const filtered = useMemo(() => {
    if (!entries) return [];
    const filter = TIME_FILTERS.find((candidate) => candidate.id === filterId) ?? TIME_FILTERS[5];
    if (filter.hours === null) return entries;
    const cutoff = filter.id === "today"
      ? new Date(new Date().setHours(0, 0, 0, 0)).getTime()
      : Date.now() - filter.hours * 60 * 60 * 1000;
    return entries.filter((entry) => entry.timestamp >= cutoff);
  }, [entries, filterId]);

  const stats = useMemo(() => filtered.reduce((sum, entry) => ({
    requests: sum.requests + 1,
    input: sum.input + entry.promptTokens,
    output: sum.output + entry.completionTokens,
    cached: sum.cached + entry.cachedPromptTokens,
    costUsd: sum.costUsd + (entry.estimatedCostUsd ?? 0),
    durationMs: sum.durationMs + entry.durationMs,
  }), { requests: 0, input: 0, output: 0, cached: 0, costUsd: 0, durationMs: 0 }), [filtered]);

  const rows = useMemo(() => filtered.slice(-MAX_LOG_ROWS).reverse(), [filtered]);
  const projects = useMemo(() => aggregateUsageByProject(filtered), [filtered]);

  const refresh = () => { void reloadAiUsageLog().then(setEntries); };
  const clearLog = () => { void clearAiUsageLog().then(setEntries); };

  if (entries === null) {
    return <div className="settings-empty-note">{t("settings.usage.loading")}</div>;
  }

  return (
    <div className="settings-section-stack ai-usage-console">
      <div className="ai-usage-toolbar">
        <div className="ai-usage-filters" role="radiogroup" aria-label={t("settings.usage.filter.aria")}>
          {TIME_FILTERS.map((filter) => (
            <button
              key={filter.id}
              type="button"
              role="radio"
              aria-checked={filterId === filter.id}
              data-active={filterId === filter.id || undefined}
              onClick={() => setFilterId(filter.id)}
            >
              {t(filter.labelKey)}
            </button>
          ))}
        </div>
        <div className="ai-usage-toolbar-actions">
          <button type="button" onClick={refresh} title={t("settings.usage.refresh")}>
            <RefreshCw size={13} />
          </button>
          <button type="button" className="ai-usage-clear" onClick={clearLog} title={t("settings.usage.clear")}>
            <Trash2 size={13} />
          </button>
        </div>
      </div>

      <div className="ai-usage-cards">
        <UsageCard label={t("settings.usage.card.requests")} value={formatInteger(stats.requests)} sub={t("settings.usage.card.requestsSub")} />
        <UsageCard label={t("settings.usage.card.input")} value={formatCompactTokens(stats.input)} sub={t("settings.usage.card.tokens")} />
        <UsageCard label={t("settings.usage.card.output")} value={formatCompactTokens(stats.output)} sub={t("settings.usage.card.tokens")} />
        <UsageCard label={t("settings.usage.card.cached")} value={formatCompactTokens(stats.cached)} sub={t("settings.usage.card.tokens")} />
        <UsageCard label={t("settings.usage.card.cost")} value={formatUsageCost(stats.costUsd, t)} sub={t("settings.usage.card.costSub")} />
        <UsageCard label={t("settings.usage.card.time")} value={formatUsageDuration(stats.durationMs)} sub={t("settings.usage.card.timeSub")} />
      </div>

      {rows.length === 0 ? (
        <div className="settings-empty-note">{t("settings.usage.emptyWindow")}</div>
      ) : (
        <div className="ai-usage-log" role="list" aria-label={t("settings.usage.recent.title")}>
          {rows.map((entry) => {
            const expanded = expandedId === entry.id;
            return (
              <div key={entry.id} className="ai-usage-log-item" data-open={expanded || undefined} role="listitem">
                <button
                  type="button"
                  className="ai-usage-log-row"
                  aria-expanded={expanded}
                  onClick={() => setExpandedId(expanded ? null : entry.id)}
                >
                  <span className="ai-usage-log-time">{formatLogTime(entry.timestamp)}</span>
                  <span className="ai-usage-log-badge" data-tone="ok">{t("settings.usage.status.ok")}</span>
                  <span className="ai-usage-log-model" title={entry.model}>{entry.model}</span>
                  <span className="ai-usage-log-tokens">
                    <span data-kind="in" title={t("settings.usage.detail.input")}>↑ {formatCompactTokens(entry.promptTokens)}</span>
                    <span data-kind="out" title={t("settings.usage.detail.output")}>↓ {formatCompactTokens(entry.completionTokens)}</span>
                    {entry.cachedPromptTokens > 0 && (
                      <span data-kind="cache" title={t("settings.usage.detail.cached")}>⚡ {formatCompactTokens(entry.cachedPromptTokens)}</span>
                    )}
                  </span>
                  <span className="ai-usage-log-duration">{formatInteger(entry.durationMs)}ms</span>
                  <ChevronRight size={13} className="ai-usage-log-caret" data-open={expanded || undefined} />
                </button>
                {expanded && (
                  <dl className="ai-usage-log-detail">
                    <div><dt>{t("settings.usage.detail.provider")}</dt><dd>{entry.provider || "—"}</dd></div>
                    <div><dt>{t("settings.usage.detail.model")}</dt><dd>{entry.model}</dd></div>
                    <div><dt>{t("settings.usage.detail.mode")}</dt><dd>{entry.agentMode || "—"}</dd></div>
                    <div><dt>{t("settings.usage.detail.project")}</dt><dd>{entry.workspaceName || projectKeyLabel(entry.workspaceKey, t)}</dd></div>
                    <div><dt>{t("settings.usage.detail.requests")}</dt><dd>{entry.requestCount && entry.requestCount > 0 ? formatInteger(entry.requestCount) : "—"}</dd></div>
                    <div><dt>{t("settings.usage.detail.input")}</dt><dd>{formatInteger(entry.promptTokens)}</dd></div>
                    <div><dt>{t("settings.usage.detail.output")}</dt><dd>{formatInteger(entry.completionTokens)}</dd></div>
                    <div><dt>{t("settings.usage.detail.cached")}</dt><dd>{formatInteger(entry.cachedPromptTokens)}</dd></div>
                    <div><dt>{t("settings.usage.detail.total")}</dt><dd>{formatInteger(entry.totalTokens)}</dd></div>
                    <div><dt>{t("settings.usage.detail.speed")}</dt><dd>{formatUsageSpeed(usageEntryTokensPerSecond(entry), t)}</dd></div>
                    <div><dt>{t("settings.usage.detail.cost")}</dt><dd>{formatUsageCost(entry.estimatedCostUsd, t)}</dd></div>
                    <div><dt>{t("settings.usage.detail.when")}</dt><dd>{new Date(entry.timestamp).toLocaleString()}</dd></div>
                    <div><dt>{t("settings.usage.detail.duration")}</dt><dd>{formatUsageDuration(entry.durationMs)}</dd></div>
                  </dl>
                )}
              </div>
            );
          })}
        </div>
      )}

      {projects.length > 1 && (
        <div className="ai-usage-projects">
          <h4>{t("settings.usage.byProject.title")}</h4>
          {projects.map((project) => (
            <div className="ai-usage-project-row" key={project.workspaceKey || "__none__"} data-active={project.workspaceKey === currentKey || undefined}>
              <div className="ai-usage-project-main">
                <strong>{project.workspaceName || projectKeyLabel(project.workspaceKey, t)}</strong>
                <small>{t("settings.usage.byProject.requests", { count: project.requestCount })}</small>
              </div>
              <div className="ai-usage-project-stats">
                <span>{formatCompactTokens(project.totalTokens)} {t("settings.usage.tok")}</span>
                <span>{formatUsageCost(project.estimatedCostUsd, t)}</span>
                <span>{formatUsageDuration(project.totalDurationMs)}</span>
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function UsageCard({ label, sub, value }: { label: string; sub: string; value: string }) {
  return (
    <div className="ai-usage-card">
      <span className="ai-usage-card-label">{label}</span>
      <strong className="ai-usage-card-value">{value}</strong>
      <span className="ai-usage-card-sub">{sub}</span>
    </div>
  );
}

function projectKeyLabel(key: string, t: TranslateFn) {
  if (!key) return t("settings.usage.noProject");
  const segments = key.split("/").filter(Boolean);
  return segments[segments.length - 1] || key;
}

function formatInteger(value: number) {
  return new Intl.NumberFormat("en-US").format(Math.round(value));
}

function formatLogTime(timestamp: number) {
  return new Date(timestamp).toLocaleTimeString(undefined, { hour12: false });
}

function formatUsageCost(usd: number | null, t: TranslateFn) {
  if (usd === null || usd <= 0) return t("settings.usage.costUnknown");
  if (usd < 0.01) return "<$0.01";
  return `$${usd.toFixed(usd < 1 ? 3 : 2)}`;
}

function formatUsageDuration(ms: number) {
  if (ms <= 0) return "—";
  const seconds = ms / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m ${Math.round(seconds % 60)}s`;
  const hours = Math.floor(minutes / 60);
  return `${hours}h ${minutes % 60}m`;
}

function formatUsageSpeed(tokensPerSecond: number, t: TranslateFn) {
  if (tokensPerSecond <= 0) return "—";
  return t("settings.usage.speedValue", { speed: tokensPerSecond.toFixed(1) });
}
