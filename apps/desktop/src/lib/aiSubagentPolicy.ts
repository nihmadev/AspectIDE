import type { AiPreferences } from "./aiPreferences";

export const defaultMaxParallelSubagents = 5;
export const maxParallelSubagentsMin = 1;
export const maxParallelSubagentsMax = 16;

export function resolveMaxParallelSubagents(preferences: Pick<AiPreferences, "maxParallelSubagents"> | number | undefined) {
  const raw = typeof preferences === "number" ? preferences : preferences?.maxParallelSubagents;
  const value = typeof raw === "number" && Number.isFinite(raw) ? Math.round(raw) : defaultMaxParallelSubagents;
  return Math.min(maxParallelSubagentsMax, Math.max(maxParallelSubagentsMin, value));
}