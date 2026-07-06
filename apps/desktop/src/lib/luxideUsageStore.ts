import { create } from "zustand";

import type { LuxideModelUsage } from "./luxideModelSync";

type LuxideUsageStore = {
  /** alias → this user's usage (all-time total + rolling windows), refreshed by the poller. */
  map: Record<string, LuxideModelUsage>;
  setMap: (map: Record<string, LuxideModelUsage>) => void;
};

/**
 * One shared usage map for the active LuxIDE provider, populated by a single poller
 * (useLuxideUsagePoller) and read by every consumer — the composer plaque AND the
 * model-picker decoration — so switching models is instant and only one /v1/usage
 * request runs per interval.
 */
export const useLuxideUsageStore = create<LuxideUsageStore>((set) => ({
  map: {},
  setMap: (map) => set({ map }),
}));
