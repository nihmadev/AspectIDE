import type { AiChatSendInput } from "./../chat/types";
import { readLints, testHealth } from "./../runtime/diagnostics";
import { toolJson, type ToolResult } from "./../runtime/shared";

const fileEditTools = new Set(["Write", "StrReplace", "PatchEngine", "Delete"]);

export function isFileEditToolName(name: string | undefined) {
  return Boolean(name && fileEditTools.has(name));
}

/**
 * Safely parse a JSON tool-result string that may be truncated by the tool output pipeline.
 * Returns the parsed value on success, or a bounded sentinel so the turn doesn't abort due
 * to a JSON parse failure on a successfully-edited file.
 */
function safeParseToolJson(content: string): unknown {
  try {
    return JSON.parse(content);
  } catch {
    // Detect truncation: well-formed JSON always ends with `}` or `]`.
    const truncated = content.length > 0 && content.at(-1) !== "}" && content.at(-1) !== "]";
    return { error: "Verification result could not be parsed.", truncated };
  }
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
  // Use safeParseToolJson to avoid crashing on truncated large diagnostic payloads —
  // a JSON.parse throw here would convert a successful edit into a failed Automatic turn.
  const lintMerged = lintResults.map((result) => safeParseToolJson(result.content));
  const testResult = await testHealth();

  return toolJson("AutomaticVerification", {
    mode: "automatic",
    changedPaths: uniquePaths,
    readLints: lintMerged,
    testHealth: safeParseToolJson(testResult.content),
    notes: [
      "Lux Automatic mode ran post-edit verification without waiting for the model to request it.",
      "Treat failures as blockers; fix and re-verify before declaring the task complete.",
    ],
  });
}