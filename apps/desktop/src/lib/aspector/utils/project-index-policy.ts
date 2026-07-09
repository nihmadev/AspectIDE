import type { AiIndexState } from "./../../store/index";

export function isWeakProjectIndex(index: Pick<AiIndexState, "quality" | "status" | "indexedFiles">) {
  if (index.status === "disabled") return false;
  if (index.quality === "empty" || index.quality === "limited") return true;
  return index.status === "idle" && index.indexedFiles === 0;
}

