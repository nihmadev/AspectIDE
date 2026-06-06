#!/usr/bin/env node
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const src = path.join(root, "src");

const checks = [
  ["aiFileDiffHunks", path.join(src, "lib", "aiFileDiffHunks.ts")],
  ["aiSubagents", path.join(src, "lib", "aiSubagents.ts")],
  ["aiTurnUsage", path.join(src, "lib", "aiTurnUsage.ts")],
  ["AiMonacoDiffReview", path.join(src, "components", "ai-chat", "AiMonacoDiffReview.tsx")],
  ["Task tool", path.join(src, "lib", "aiRuntimeTools.ts")],
  ["transport retry", path.join(src, "lib", "aiChatTransport.ts")],
];

for (const [label, filePath] of checks) {
  const text = await readFile(filePath, "utf8");
  assert.ok(text.length > 20, `${label} missing at ${filePath}`);
}

const tools = await readFile(path.join(src, "lib", "aiRuntimeTools.ts"), "utf8");
assert.match(tools, /name: "Task"/, "Task tool must be registered");

const transport = await readFile(path.join(src, "lib", "aiChatTransport.ts"), "utf8");
assert.match(transport, /requestChatCompletionWithRetry/, "provider retry helper required");
assert.match(transport, /no model fallback/, "retry must not silently switch models");

const runtime = await readFile(path.join(src, "lib", "aiChatRuntime.ts"), "utf8");
const dispatch = await readFile(path.join(src, "lib", "aiRuntimeToolDispatch.ts"), "utf8");
const sessionTools = await readFile(path.join(src, "lib", "aiRuntimeToolSession.ts"), "utf8");
assert.match(runtime, /extractTurnTokenUsage/, "turn usage must be parsed");
assert.match(runtime, /runAutomaticPostEditVerification/, "automatic post-edit verification required");
assert.match(runtime, /runRuntimeTool/, "turn loop must delegate tools to dispatch");
assert.match(dispatch, /case "Task":/, "Task runtime handler required");
assert.match(sessionTools, /resolveMaxParallelSubagents/, "parallel subagent limit required");
assert.match(sessionTools, /countRunningSubagents/, "parallel subagent limit guard required");

const fileTools = await readFile(path.join(src, "lib", "aiRuntimeFileTools.ts"), "utf8");
assert.match(fileTools, /resolveFileEditSaveToDisk/, "file edit save-to-disk gate required");
assert.match(fileTools, /agentMode === "automatic"/, "automatic mode must force disk persistence");

const panel = await readFile(path.join(src, "components", "AiChatPanel.tsx"), "utf8");
assert.match(panel, /buildContextDropSummary/, "context drop summary must be wired in chat panel");
assert.match(panel, /shouldAutoRefreshIndexForAutomatic/, "automatic index refresh policy required");
assert.match(panel, /classifyAiChatError/, "structured chat errors required");
assert.match(panel, /AiSubagentPanel/, "subagent panel must be visible in chat");

const enforcement = await readFile(path.join(src, "lib", "aiAutomaticModeEnforcement.ts"), "utf8");
assert.match(enforcement, /first model turn MUST include tool calls/, "automatic enforcement must require tool execution");
const semantic = await readFile(path.join(src, "lib", "aiRuntimeSemanticSearch.ts"), "utf8");
assert.match(semantic, /export async function semanticSearch/, "semantic search must live in dedicated module");

assert.match(dispatch, /export async function runRuntimeTool/, "tool dispatch must be isolated from aiChatRuntime");

const runtimeLines = runtime.replace(/^\uFEFF/, "").split("\n").length;
assert.ok(runtimeLines < 520, `aiChatRuntime.ts should be slim after extraction (got ${runtimeLines} lines)`);

const decode = await readFile(path.join(src, "lib", "decodeChatDisplayText.ts"), "utf8").catch(() => "");
assert.ok(decode.includes("decodeChatDisplayText"), "decodeChatDisplayText module must exist");

console.log("verify-ai-chat-pipeline: ok");