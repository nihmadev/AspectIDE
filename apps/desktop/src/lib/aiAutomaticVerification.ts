import type { AiChatSendInput } from "./aiChatTypes";
import { readLints, testHealth } from "./aiRuntimeDiagnostics";
import { toolJson, type ToolResult } from "./aiRuntimeShared";

const fileEditTools = new Set(["Write", "StrReplace", "PatchEngine", "Delete"]);

export function isFileEditToolName(name: string | undefined) {
  return Boolean(name && fileEditTools.has(name));
}

export async function runAutomaticPostEditVerification(
  input: AiChatSendInput,
  changedPaths: string[],
): Promise<ToolResult | null> {
  if (input.preferences.agentMode !== "automatic") return null;
  if (changedPaths.length === 0) return null;

  const uniquePaths = [...new Set(changedPaths.map((path) => path.trim()).filter(Boolean))];
  const lintPaths = uniquePaths.slice(0, 6);
  const lintResults = await Promise.all(
    lintPaths.map((path) => readLints({ path, maxResults: 60 }, input)),
  );
  const lintMerged = lintResults.map((result) => JSON.parse(result.content));
  const testResult = await testHealth();

  return toolJson("AutomaticVerification", {
    mode: "automatic",
    changedPaths: uniquePaths,
    readLints: lintMerged,
    testHealth: JSON.parse(testResult.content),
    notes: [
      "Lux Automatic mode ran post-edit verification without waiting for the model to request it.",
      "Treat failures as blockers; fix and re-verify before declaring the task complete.",
    ],
  });
}