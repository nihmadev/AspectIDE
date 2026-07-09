import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { resolve, dirname } from "node:path";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const appPath = resolve(scriptDir, "../src/App.tsx");
const panelPath = resolve(scriptDir, "../src/components/AiChatPanel.tsx");
const [appSource, panelSource] = await Promise.all([
  readFile(appPath, "utf8"),
  readFile(panelPath, "utf8"),
]);
const errors = [];

if (!appSource.includes("aiChatHistoryLoadedRef")) {
  errors.push("AI chat history must be loaded from the stable app-level component (App), not from AiChatPanel.");
}

if (!appSource.includes("aiChatPersistTimerRef")) {
  errors.push("AI chat history persistence must be debounced from the app-level; aiChatPersistTimerRef was not found in App.");
}

if (!appSource.includes("isAiChatSessionBusyStatus(session.status)")) {
  errors.push("AI chat history must not persist while any session is busy (thinking/streaming/running-tools/waiting-approval).");
}

if (panelSource.includes("historyPersistTimerRef") || panelSource.includes("loadAiChatHistory")) {
  errors.push("AI chat history load/save must be removed from AiChatPanel to prevent stale-overwrite on remount.");
}

if (errors.length > 0) {
  throw new Error(`AI chat persistence verification failed:\n- ${errors.join("\n- ")}`);
}

console.log("AI chat persistence verification passed (sync is stable at the app level, guarded by busy status).");
