// Live model sync for the bundled "LuxIDE" managed provider.
//
// The gateway's admin (via @LuxIDE_bot) can enable/disable models at runtime, and
// GET /v1/models returns only the currently-enabled ones. This module polls that
// endpoint while a LuxIDE provider is the active one and reconciles the local model
// list, so toggling a model in the bot makes it appear/disappear in the composer
// within seconds (immediately on window focus) — no restart, no manual Refresh.
//
// The reconcile is deliberately conservative: it never nukes the list to empty (a
// transient "all disabled" or a failed fetch must not strand the user with no
// models) and preserves the user's current selection across the refresh.

import { useEffect, useRef } from "react";

import {
  getAiProvider,
  type AiModelConfig,
  type AiPreferences,
} from "./aiPreferences";
import { fetchProviderModelConfigs, mergeRefreshedModels } from "./aiProviderModels";
import { getLinkedTokenSilent, isLuxideProvider } from "./luxideEnroll";
import { useLuxideUsageStore } from "./luxideUsageStore";
import { useLuxStore } from "./store";
import { isTauriRuntime, luxCommands } from "./tauri";

/** How often to re-pull the enabled-model list while LuxIDE is active. */
const SYNC_INTERVAL_MS = 45_000;

/** How often to re-pull per-model usage for the composer indicator. */
const USAGE_INTERVAL_MS = 30_000;

/** One rolling window's usage against its cap (cap 0 = uncapped). */
export type LuxideWindowUsage = { window: string; used: number; cap: number };

/** Per-model usage: all-time total plus each rolling window's used/cap. */
export type LuxideModelUsage = { total: number; windows: LuxideWindowUsage[] };

/** Rolling windows in display order, with short labels (mirrors the gateway). */
const WINDOW_LABELS: Record<string, string> = { "5h": "5h", day: "d", week: "wk" };
const WINDOW_ORDER = ["5h", "day", "week"];

/** True when two model lists have the same ids in the same order (a no-op refresh). */
function sameOrderedIds(a: AiModelConfig[], b: AiModelConfig[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((model, index) => model.id === b[index].id);
}

/**
 * Produce updated preferences after a live fetch of the LuxIDE provider's enabled
 * models, or `null` when nothing should change (empty fetch, unknown provider, or an
 * identical list). Pure — the caller persists the result.
 *
 * When the LuxIDE provider is the active one, the current selection is preserved:
 * kept as-is when the selected model still exists (matched by id, or case-insensitively
 * by alias to survive the one-time id→alias migration), otherwise reset to the first
 * model with its first effort level.
 */
export function reconcileLuxideModels(
  prefs: AiPreferences,
  providerId: string,
  fetched: AiModelConfig[],
): AiPreferences | null {
  if (fetched.length === 0) return null; // never strand the user with an empty list
  const provider = getAiProvider(prefs.providers, providerId);
  if (!provider) return null;

  const merged = mergeRefreshedModels(provider, fetched);
  if (sameOrderedIds(provider.models, merged.models)) return null;

  let selectedModelId = prefs.selectedModelId;
  let selectedEffortId = prefs.selectedEffortId;
  if (prefs.selectedProviderId === providerId) {
    const lowerSelected = selectedModelId.toLowerCase();
    const still =
      merged.models.find((model) => model.id === selectedModelId) ??
      merged.models.find((model) => model.alias.toLowerCase() === lowerSelected);
    if (still) {
      selectedModelId = still.id;
    } else {
      const first = merged.models[0];
      selectedModelId = first.id;
      selectedEffortId = first.effortLevels[0]?.id ?? selectedEffortId;
    }
  }

  const providers = prefs.providers.map((candidate) =>
    candidate.id === providerId ? merged : candidate,
  );
  return { ...prefs, providers, selectedModelId, selectedEffortId };
}

/**
 * Keep the active LuxIDE provider's model list in sync with the gateway. Runs on
 * mount, on an interval, and on window focus — but only does work when the selected
 * provider is a LuxIDE managed provider and we're in the desktop runtime (the
 * /v1/models call needs the native enrollment token and cross-origin fetch).
 */
export function useLuxideModelSync(): void {
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
  // Read prefs via a ref so the effect mounts once (a fresh subscription per
  // keystroke would tear down and rebuild the interval constantly).
  const prefsRef = useRef(useLuxStore.getState().aiPreferences);
  useEffect(() => useLuxStore.subscribe((state) => {
    prefsRef.current = state.aiPreferences;
  }), []);

  useEffect(() => {
    if (!isTauriRuntime()) return;
    let cancelled = false;
    let running = false;

    const sync = async () => {
      if (running || cancelled) return;
      const prefs = prefsRef.current;
      const provider = getAiProvider(prefs.providers, prefs.selectedProviderId);
      if (!provider || !isLuxideProvider(provider)) return;
      running = true;
      try {
        const token = await getLinkedTokenSilent();
        if (cancelled || !token) return; // not linked yet — stay silent
        const source = { ...provider, apiKey: token };
        const fetched = await fetchProviderModelConfigs(source);
        // The selection can change across the awaits — bail if it's no longer this
        // provider so we don't reconcile against a stale target.
        if (cancelled || prefsRef.current.selectedProviderId !== provider.id) return;
        const next = reconcileLuxideModels(prefsRef.current, provider.id, fetched);
        if (next) setAiPreferences(next);
      } catch {
        // Transient network / enrollment hiccup — the next tick retries.
      } finally {
        running = false;
      }
    };

    void sync();
    const interval = window.setInterval(() => void sync(), SYNC_INTERVAL_MS);
    const onFocus = () => void sync();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
    };
  }, [setAiPreferences]);
}

/** Compact token count: 1200 → "1.2k", 500_000_000 → "500M", 999_999_999 → "1B". */
export function formatCompactTokens(n: number): string {
  const unit = (value: number, suffix: string): string | null => {
    const rounded = value >= 100 ? Math.round(value) : Math.round(value * 10) / 10;
    // A round-up that reaches 1000 belongs to the next unit — let the caller retry.
    return rounded >= 1000 ? null : `${rounded}${suffix}`;
  };
  if (n >= 1_000_000_000) return unit(n / 1_000_000_000, "B") ?? `${Math.round(n / 1_000_000_000)}B`;
  if (n >= 1_000_000) return unit(n / 1_000_000, "M") ?? formatCompactTokens(1_000_000_000);
  if (n >= 1_000) return unit(n / 1_000, "k") ?? formatCompactTokens(1_000_000);
  return String(Math.max(0, Math.round(n)));
}

/**
 * Render the composer plaque label for a model's usage: each capped window as
 * "<label> used/cap" (e.g. "5h 12k/100k · wk 1.2M/2M") followed by the all-time total
 * "Σ 45M". Returns null when there's nothing to show (no caps and no spend, or the
 * model isn't a metered LuxIDE one).
 */
export function formatLuxideUsageLabel(usage: LuxideModelUsage | null): string | null {
  if (!usage) return null;
  const byKey = new Map(usage.windows.map((w) => [w.window, w]));
  const parts: string[] = [];
  for (const key of WINDOW_ORDER) {
    const win = byKey.get(key);
    if (win && win.cap > 0) {
      parts.push(`${WINDOW_LABELS[key] ?? key} ${formatCompactTokens(win.used)}/${formatCompactTokens(win.cap)}`);
    }
  }
  if (usage.total > 0) parts.push(`Σ ${formatCompactTokens(usage.total)}`);
  return parts.length ? parts.join(" · ") : null;
}

/**
 * Single poller for the active LuxIDE provider's per-user usage. Fetches ALL models'
 * usage in one /v1/usage request and writes it to the shared store (useLuxideUsageStore)
 * so every consumer — the composer plaque AND the model-picker decoration — reads one
 * map and one request runs per interval. Call this ONCE (in AiChatPanel).
 */
export function useLuxideUsagePoller(): void {
  const provider = useLuxStore((state) => getAiProvider(state.aiPreferences.providers, state.aiPreferences.selectedProviderId));
  const setMap = useLuxideUsageStore((state) => state.setMap);
  const isLuxide = !!provider && isLuxideProvider(provider);
  const baseUrl = provider?.baseUrl ?? "";

  useEffect(() => {
    if (!isTauriRuntime() || !isLuxide || !baseUrl) {
      setMap({});
      return;
    }
    let cancelled = false;
    let running = false;
    const sync = async () => {
      if (running || cancelled) return;
      running = true;
      try {
        const token = await getLinkedTokenSilent();
        if (cancelled || !token) return;
        const rows = await luxCommands.luxideUsage(baseUrl, token);
        if (cancelled) return;
        const map: Record<string, LuxideModelUsage> = {};
        for (const row of rows) map[row.id] = { total: row.total, windows: row.windows };
        setMap(map);
      } catch {
        // Transient failure — keep the last-known map; the next tick retries.
      } finally {
        running = false;
      }
    };
    void sync();
    const interval = window.setInterval(() => void sync(), USAGE_INTERVAL_MS);
    const onFocus = () => void sync();
    window.addEventListener("focus", onFocus);
    return () => {
      cancelled = true;
      window.clearInterval(interval);
      window.removeEventListener("focus", onFocus);
    };
  }, [isLuxide, baseUrl, setMap]);
}

/** Usage for one model alias from the shared store (null when absent). */
export function useLuxideModelUsage(alias: string | undefined): LuxideModelUsage | null {
  const map = useLuxideUsageStore((state) => state.map);
  return alias ? map[alias] ?? null : null;
}

/**
 * Usage for the currently-selected model when the active provider is a LuxIDE managed
 * one, else null. Reads the shared store, so switching models is instant (no fetch).
 */
export function useLuxideSelectedModelUsage(): LuxideModelUsage | null {
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const map = useLuxideUsageStore((state) => state.map);
  const provider = getAiProvider(aiPreferences.providers, aiPreferences.selectedProviderId);
  if (!provider || !isLuxideProvider(provider)) return null;
  const selected = provider.models.find((model) => model.id === aiPreferences.selectedModelId);
  const alias = selected?.alias ?? aiPreferences.selectedModelId;
  return map[alias] ?? null;
}

/** Weekly "used/cap" badge for a model, or null when the week window is uncapped. */
export function luxideWeeklyBadge(usage: LuxideModelUsage | null): string | null {
  const week = usage?.windows.find((w) => w.window === "week");
  if (!week || week.cap <= 0) return null;
  return `${formatCompactTokens(week.used)}/${formatCompactTokens(week.cap)}`;
}

/** Availability dot for a model: "blocked" when any capped window is exhausted, else "ok". */
export function luxideAvailability(usage: LuxideModelUsage | null): "ok" | "blocked" | null {
  if (!usage) return null;
  return usage.windows.some((w) => w.cap > 0 && w.used >= w.cap) ? "blocked" : "ok";
}
