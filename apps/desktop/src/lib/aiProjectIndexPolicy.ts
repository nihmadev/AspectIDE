import type { AiIndexState } from "./store";

export function isWeakProjectIndex(index: Pick<AiIndexState, "quality" | "status" | "indexedFiles">) {
  if (index.status === "disabled") return false;
  if (index.quality === "empty" || index.quality === "limited") return true;
  return index.status === "idle" && index.indexedFiles === 0;
}

export function shouldAutoRefreshIndexForAutomatic(
  indexingEnabled: boolean,
  index: Pick<AiIndexState, "quality" | "status" | "indexedFiles">,
) {
  // Automatic background indexing whenever enabled and index is weak.
  // No more user-facing nags — it should just work silently in the background.
  return indexingEnabled && isWeakProjectIndex(index);
}