import { isBrowserPreviewRuntime, isTauriRuntime, luxCommands } from "./../../../tauri/commands";
import { workspaceInstructionsKey } from "./../preferences";

/**
 * Persisted per-request usage log. Each completed AI turn appends one entry so the
 * user can review history: which model, in which project, how fast, how many tokens,
 * and the (manual-price-based) cost. Stored as a capped ring buffer under the user
 * settings key, kept separate from chat history so it survives session pruning and
 * stays cheap to load for the Settings → AI Usage view.
 */
export const AI_USAGE_LOG_KEY = "ai.usageLog";

/** Hard cap on retained entries. Oldest are dropped first (ring buffer). */
export const maxUsageLogEntries = 1000;

const usageLogSchemaVersion = 1;

export type AiUsageLogEntry = {
  /** Stable id (uuid) — also the React list key. */
  id: string;
  /** Epoch ms when the turn completed. */
  timestamp: number;
  /** Normalized workspace key (forward-slashed root), or "" when no workspace was open. */
  workspaceKey: string;
  /** Human-readable workspace name for display, or "" when none. */
  workspaceName: string;
  /** Model alias/id used for the turn. */
  model: string;
  /** Provider display name. */
  provider: string;
  /** Agent mode at send time (agent/automatic/plan/ask). */
  agentMode: string;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  cachedPromptTokens: number;
  /** Estimated cost in USD, or null when no rate was resolvable. */
  estimatedCostUsd: number | null;
  /** Wall-clock turn duration in ms. */
  durationMs: number;
  /** Model API requests the turn issued (loop rounds + recovery synthesis), when known. */
  requestCount?: number;
};

type PersistedUsageLog = {
  schemaVersion: number;
  entries: AiUsageLogEntry[];
};

/**
 * In-memory mirror of the persisted log. The store is written through on every
 * append so a reload reflects the latest turn, but reads during a session hit this
 * cache instead of round-tripping settings. `null` = not loaded yet.
 */
let cache: AiUsageLogEntry[] | null = null;

function browserStorageKey() {
  return `lux.${AI_USAGE_LOG_KEY}`;
}

function readPersisted(value: unknown): AiUsageLogEntry[] {
  if (!value || typeof value !== "object") return [];
  const record = value as Partial<PersistedUsageLog>;
  if (!Array.isArray(record.entries)) return [];
  return record.entries.filter(isUsageLogEntry).slice(-maxUsageLogEntries);
}

/** Loads the usage log (cached after first call). Safe in any runtime. */
export async function loadAiUsageLog(): Promise<AiUsageLogEntry[]> {
  if (cache) return cache;
  if (isBrowserPreviewRuntime() && !isTauriRuntime()) {
    cache = readBrowserLog();
    return cache;
  }
  try {
    const setting = await luxCommands.settingsGet("user", AI_USAGE_LOG_KEY);
    cache = setting ? readPersisted(setting.value) : [];
  } catch {
    cache = [];
  }
  return cache;
}

function readBrowserLog(): AiUsageLogEntry[] {
  try {
    const raw = window.localStorage.getItem(browserStorageKey());
    return raw ? readPersisted(JSON.parse(raw)) : [];
  } catch {
    return [];
  }
}

async function persist(entries: AiUsageLogEntry[]): Promise<void> {
  const payload: PersistedUsageLog = { schemaVersion: usageLogSchemaVersion, entries };
  if (isBrowserPreviewRuntime() && !isTauriRuntime()) {
    try {
      window.localStorage.setItem(browserStorageKey(), JSON.stringify(payload));
    } catch {
      // Ignore quota failures: the log is best-effort, not critical state.
    }
    return;
  }
  try {
    await luxCommands.settingsSet("user", AI_USAGE_LOG_KEY, payload as unknown as Record<string, unknown>);
  } catch {
    // Best-effort: a failed write must never break the chat turn.
  }
}

export type AppendUsageLogInput = {
  workspaceRoot: string | null | undefined;
  workspaceName: string | null | undefined;
  model: string;
  provider: string;
  agentMode: string;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  cachedPromptTokens?: number;
  estimatedCostUsd: number | null;
  durationMs: number;
  requestCount?: number;
};

/**
 * Append one completed turn to the usage log and persist (write-through, ring-capped).
 * Returns the updated list. Entries with no tokens and no duration are skipped so
 * cancelled/no-op turns don't pollute history. Never throws.
 */
export async function appendAiUsageLogEntry(input: AppendUsageLogInput): Promise<AiUsageLogEntry[]> {
  const promptTokens = nonNegative(input.promptTokens);
  const completionTokens = nonNegative(input.completionTokens);
  const totalTokens = nonNegative(input.totalTokens) || promptTokens + completionTokens;
  const durationMs = nonNegative(input.durationMs);
  if (promptTokens + completionTokens <= 0 && durationMs <= 0) {
    return (await loadAiUsageLog()).slice();
  }

  const existing = await loadAiUsageLog();
  const entry: AiUsageLogEntry = {
    id: crypto.randomUUID(),
    timestamp: Date.now(),
    workspaceKey: workspaceInstructionsKey(input.workspaceRoot),
    workspaceName: input.workspaceName?.trim() || "",
    model: input.model.trim() || "unknown",
    provider: input.provider.trim() || "",
    agentMode: input.agentMode.trim() || "",
    promptTokens,
    completionTokens,
    totalTokens,
    cachedPromptTokens: nonNegative(input.cachedPromptTokens ?? 0),
    estimatedCostUsd: typeof input.estimatedCostUsd === "number" && input.estimatedCostUsd > 0 ? input.estimatedCostUsd : null,
    durationMs,
    ...(nonNegative(input.requestCount ?? 0) > 0 ? { requestCount: nonNegative(input.requestCount ?? 0) } : {}),
  };
  const next = [...existing, entry].slice(-maxUsageLogEntries);
  cache = next;
  await persist(next);
  return next.slice();
}

/** Clears the entire usage log. Returns the empty list. */
export async function clearAiUsageLog(): Promise<AiUsageLogEntry[]> {
  cache = [];
  await persist([]);
  return [];
}

/** Drops the in-memory cache and re-reads the persisted log (manual refresh). */
export async function reloadAiUsageLog(): Promise<AiUsageLogEntry[]> {
  cache = null;
  return loadAiUsageLog();
}

export type AiUsageProjectAggregate = {
  workspaceKey: string;
  workspaceName: string;
  requestCount: number;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  cachedPromptTokens: number;
  estimatedCostUsd: number;
  totalDurationMs: number;
  lastTimestamp: number;
};

/**
 * Group log entries by workspace, newest activity first. Used by the AI Usage view
 * to show per-project totals. The "" key (no workspace) is labeled by the caller.
 */
export function aggregateUsageByProject(entries: AiUsageLogEntry[]): AiUsageProjectAggregate[] {
  const byKey = new Map<string, AiUsageProjectAggregate>();
  for (const entry of entries) {
    const current = byKey.get(entry.workspaceKey) ?? {
      workspaceKey: entry.workspaceKey,
      workspaceName: entry.workspaceName,
      requestCount: 0,
      promptTokens: 0,
      completionTokens: 0,
      totalTokens: 0,
      cachedPromptTokens: 0,
      estimatedCostUsd: 0,
      totalDurationMs: 0,
      lastTimestamp: 0,
    };
    current.requestCount += 1;
    current.promptTokens += entry.promptTokens;
    current.completionTokens += entry.completionTokens;
    current.totalTokens += entry.totalTokens;
    current.cachedPromptTokens += entry.cachedPromptTokens;
    current.estimatedCostUsd += entry.estimatedCostUsd ?? 0;
    current.totalDurationMs += entry.durationMs;
    // Keep the most recent non-empty display name for the project.
    if (entry.workspaceName && entry.timestamp >= current.lastTimestamp) current.workspaceName = entry.workspaceName;
    current.lastTimestamp = Math.max(current.lastTimestamp, entry.timestamp);
    byKey.set(entry.workspaceKey, current);
  }
  return Array.from(byKey.values()).sort((left, right) => right.lastTimestamp - left.lastTimestamp);
}

/** Tokens-per-second throughput for a single entry (completion tokens / seconds). */
export function usageEntryTokensPerSecond(entry: AiUsageLogEntry): number {
  if (entry.durationMs <= 0) return 0;
  return entry.completionTokens / (entry.durationMs / 1000);
}

function nonNegative(value: number): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.round(value) : 0;
}

function isUsageLogEntry(value: unknown): value is AiUsageLogEntry {
  if (!value || typeof value !== "object") return false;
  const entry = value as Partial<AiUsageLogEntry>;
  return typeof entry.id === "string" && typeof entry.timestamp === "number" && typeof entry.model === "string";
}
