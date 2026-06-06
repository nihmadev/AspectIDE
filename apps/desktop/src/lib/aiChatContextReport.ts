import type { ContextCompactionDroppedItem, ContextCompactionState } from "./aiChatContextCompaction";
import type { ContextBudgetItem } from "./aiRuntimeContextBudget";

export type AiContextDropReason =
  | "budget-cap"
  | "max-items"
  | "low-score"
  | "compaction"
  | "truncated";

export type AiContextDropEntry = {
  id: string;
  kind: string;
  label: string;
  reason: AiContextDropReason;
  detail: string;
  tokens: number;
};

export type AiChatContextBudgetReport = {
  updatedAt: number;
  query: string;
  targetChars: number;
  selectedCount: number;
  droppedCount: number;
  dropped: AiContextDropEntry[];
};

export type AiChatContextDropSummary = {
  budgetReport: AiChatContextBudgetReport | null;
  compaction: ContextCompactionState | null;
  totalDroppedTokens: number;
  totalDroppedCount: number;
  entries: AiContextDropEntry[];
};

export function recordContextBudgetReport(
  sessionId: string,
  query: string,
  ranked: ContextBudgetItem[],
  selected: ContextBudgetItem[],
  targetChars: number,
  maxItems: number,
  onRecord: (sessionId: string, report: AiChatContextBudgetReport) => void,
) {
  const selectedIds = new Set(selected.map((item) => item.id));
  const dropped = ranked
    .filter((item) => !selectedIds.has(item.id))
    .slice(0, 48)
    .map((item, index) => toBudgetDropEntry(item, index, ranked.length, maxItems));

  onRecord(sessionId, {
    updatedAt: Date.now(),
    query,
    targetChars,
    selectedCount: selected.length,
    droppedCount: dropped.length,
    dropped,
  });
}

function toBudgetDropEntry(
  item: ContextBudgetItem,
  index: number,
  candidateCount: number,
  maxItems: number,
): AiContextDropEntry {
  const reason: AiContextDropReason = index >= maxItems
    ? "max-items"
    : item.content.includes("...[truncated ")
      ? "truncated"
      : candidateCount > maxItems
        ? "low-score"
        : "budget-cap";
  return {
    id: item.id,
    kind: item.kind,
    label: item.path || item.source || item.kind,
    reason,
    detail: item.reason,
    tokens: Math.ceil(item.content.length / 4),
  };
}

export function buildContextDropSummary(
  budgetReport: AiChatContextBudgetReport | null | undefined,
  compaction: ContextCompactionState | null | undefined,
): AiChatContextDropSummary {
  const compactionEntries: AiContextDropEntry[] = (compaction?.droppedItems ?? []).map((item, index) => ({
    id: `compact-${index}`,
    kind: item.kind,
    label: item.label,
    reason: "compaction" as const,
    detail: item.kind,
    tokens: item.tokens,
  }));

  const budgetEntries = budgetReport?.dropped ?? [];
  const entries = [...budgetEntries, ...compactionEntries];
  const compactionTokens = compaction?.droppedTokens ?? 0;
  const budgetTokens = budgetEntries.reduce((sum, entry) => sum + entry.tokens, 0);

  return {
    budgetReport: budgetReport ?? null,
    compaction: compaction ?? null,
    totalDroppedTokens: budgetTokens + compactionTokens,
    totalDroppedCount: entries.length,
    entries,
  };
}

export function compactionItemsToDropEntries(items: ContextCompactionDroppedItem[]): AiContextDropEntry[] {
  return items.map((item, index) => ({
    id: `compact-${index}`,
    kind: item.kind,
    label: item.label,
    reason: "compaction",
    detail: item.kind,
    tokens: item.tokens,
  }));
}