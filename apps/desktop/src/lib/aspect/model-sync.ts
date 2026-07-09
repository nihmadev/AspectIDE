import { useEffect, useRef } from "react";

import {
  getAiProvider,
  type AiModelConfig,
  type AiPreferences,
} from "../aspector/utils/preferences";
import { fetchProviderModelConfigs, mergeRefreshedModels } from "../aspector/utils/provider-models";
import { getLinkedTokenSilent, isAspectProvider } from "./enroll";
import { useAspectUsageStore } from "./usage-store";
import { useLuxStore } from "../store/index";
import { isTauriRuntime, luxCommands } from "../tauri/commands";

const SYNC_INTERVAL_MS = 45_000;
const USAGE_INTERVAL_MS = 30_000;

export type AspectWindowUsage = { window: string; used: number; cap: number };
export type AspectModelUsage = { total: number; windows: AspectWindowUsage[] };

const WINDOW_LABELS: Record<string, string> = { "5h": "5h", day: "d", week: "wk" };
const WINDOW_ORDER = ["5h", "day", "week"];

function sameOrderedIds(a: AiModelConfig[], b: AiModelConfig[]): boolean {
  if (a.length !== b.length) return false;
  return a.every((model, index) => model.id === b[index].id);
}

export function reconcileAspectModels(
  prefs: AiPreferences,
  providerId: string,
  fetched: AiModelConfig[],
): AiPreferences | null {
  if (fetched.length === 0) return null;
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

export function useAspectModelSync(): void {
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
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
      if (!provider || !isAspectProvider(provider)) return;
      running = true;
      try {
        const token = await getLinkedTokenSilent();
        if (cancelled || !token) return;
        const source = { ...provider, apiKey: token };
        const fetched = await fetchProviderModelConfigs(source);
        if (cancelled || prefsRef.current.selectedProviderId !== provider.id) return;
        const next = reconcileAspectModels(prefsRef.current, provider.id, fetched);
        if (next) setAiPreferences(next);
      } catch {
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

export function formatCompactTokens(n: number): string {
  const unit = (value: number, suffix: string): string | null => {
    const rounded = value >= 100 ? Math.round(value) : Math.round(value * 10) / 10;
    return rounded >= 1000 ? null : `${rounded}${suffix}`;
  };
  if (n >= 1_000_000_000) return unit(n / 1_000_000_000, "B") ?? `${Math.round(n / 1_000_000_000)}B`;
  if (n >= 1_000_000) return unit(n / 1_000_000, "M") ?? formatCompactTokens(1_000_000_000);
  if (n >= 1_000) return unit(n / 1_000, "k") ?? formatCompactTokens(1_000_000);
  return String(Math.max(0, Math.round(n)));
}

export function formatAspectUsageLabel(usage: AspectModelUsage | null): string | null {
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

export function useAspectUsagePoller(): void {
  const provider = useLuxStore((state) => getAiProvider(state.aiPreferences.providers, state.aiPreferences.selectedProviderId));
  const setMap = useAspectUsageStore((state) => state.setMap);
  const isAspect = !!provider && isAspectProvider(provider);
  const baseUrl = provider?.baseUrl ?? "";

  useEffect(() => {
    if (!isTauriRuntime() || !isAspect || !baseUrl) {
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
        const map: Record<string, AspectModelUsage> = {};
        for (const row of rows) map[row.id] = { total: row.total, windows: row.windows };
        setMap(map);
      } catch {
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
  }, [isAspect, baseUrl, setMap]);
}

export function useAspectModelUsage(alias: string | undefined): AspectModelUsage | null {
  const map = useAspectUsageStore((state) => state.map);
  return alias ? map[alias] ?? null : null;
}

export function useAspectSelectedModelUsage(): AspectModelUsage | null {
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const map = useAspectUsageStore((state) => state.map);
  const provider = getAiProvider(aiPreferences.providers, aiPreferences.selectedProviderId);
  if (!provider || !isAspectProvider(provider)) return null;
  const selected = provider.models.find((model) => model.id === aiPreferences.selectedModelId);
  const alias = selected?.alias ?? aiPreferences.selectedModelId;
  return map[alias] ?? null;
}

export function aspectWeeklyBadge(usage: AspectModelUsage | null): string | null {
  const week = usage?.windows.find((w) => w.window === "week");
  if (!week || week.cap <= 0) return null;
  return `${formatCompactTokens(week.used)}/${formatCompactTokens(week.cap)}`;
}

export function aspectAvailability(usage: AspectModelUsage | null): "ok" | "blocked" | null {
  if (!usage) return null;
  return usage.windows.some((w) => w.cap > 0 && w.used >= w.cap) ? "blocked" : "ok";
}
