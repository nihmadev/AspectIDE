import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const panelPath = resolve("src/components/AiChatPanel.tsx");
const source = await readFile(panelPath, "utf8");
const errors = [];

if (!source.includes("historyPersistTimerRef")) {
  errors.push("AI chat history persistence must be debounced; historyPersistTimerRef was not found.");
}

if (!source.includes("if (sendingSessionId !== null) return;")) {
  errors.push("AI chat history must not persist during active streaming/generation.");
}

if (!source.includes("window.setTimeout")) {
  errors.push("AI chat history persistence must debounce rapid streaming updates.");
}

const persistEffect = source.match(/useEffect\(\(\) => \{\n\s*if \(!persistedSessionsLoadedRef\.current\)[\s\S]*?saveAiChatHistory\([\s\S]*?\n\s*}\, \[activeAiChatSessionId, aiChatSessions, sendingSessionId\]\);/);
if (!persistEffect) {
  errors.push("AI chat history persist effect must depend on sendingSessionId and save only from the debounced path.");
} else if (persistEffect[0].includes("saveAiChatHistory") && !persistEffect[0].includes("historyPersistTimerRef.current = window.setTimeout")) {
  errors.push("saveAiChatHistory must stay inside the debounced timer callback.");
}

if (errors.length > 0) {
  throw new Error(`AI chat persistence verification failed:\n- ${errors.join("\n- ")}`);
}

console.log("AI chat persistence verification passed (streaming updates are not persisted per chunk).");
