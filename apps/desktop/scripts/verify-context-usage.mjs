import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as esbuild from "esbuild";

const desktopRoot = resolve(fileURLToPath(new URL(".", import.meta.url)), "..");

async function bundleModule(relativePath) {
  const result = await esbuild.build({
    entryPoints: [resolve(desktopRoot, relativePath)],
    bundle: true,
    write: false,
    format: "esm",
    platform: "node",
    target: "es2022",
  });
  const code = result.outputFiles[0]?.text;
  if (!code) throw new Error(`failed to bundle ${relativePath}`);
  return import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
}

const usage = await bundleModule("src/lib/aiChatContextUsage.ts");

const t = (key, params) => {
  if (key === "aiChat.attachment.count") return `${params.count} attachments`;
  if (key === "aiChat.context.messageCount") return `${params.count} messages`;
  if (key === "aiTools.summary.ran") return `${params.count} tools`;
  return key;
};

const hugeOldToolOutput = "x".repeat(1_200_000);
const conversation = [
  { id: "u1", role: "user", content: "Inspect project", timestamp: 1 },
  {
    id: "a1",
    role: "assistant",
    content: "Ran tool",
    timestamp: 2,
    toolCalls: [{ id: "t1", tool: "Read", status: "success", input: "{}", output: hugeOldToolOutput, startTime: 2 }],
  },
  { id: "u2", role: "user", content: "Continue", timestamp: 3 },
  { id: "a2", role: "assistant", content: "Ok", timestamp: 4, toolCalls: [{ id: "t2", tool: "Grep", status: "success", input: "{}", output: "small", startTime: 4 }] },
  { id: "u3", role: "user", content: "Next", timestamp: 5 },
  { id: "a3", role: "assistant", content: "Ok", timestamp: 6, toolCalls: [{ id: "t3", tool: "Read", status: "success", input: "{}", output: "small", startTime: 6 }] },
  { id: "u4", role: "user", content: "Next", timestamp: 7 },
  { id: "a4", role: "assistant", content: "Ok", timestamp: 8, toolCalls: [{ id: "t4", tool: "Read", status: "success", input: "{}", output: "small", startTime: 8 }] },
];

const summary = usage.buildAiChatContextUsageSummary({
  pinnedEditorPaths: [],
  aiIndexStatus: "disabled",
  agentInstruction: "",
  agentName: "Lux",
  attachments: [],
  conversation,
  message: "",
  preferences: {
    agentMode: "automatic",
    toolApprovalMode: "full-access",
    projectIndexingEnabled: false,
    maxIndexedFiles: 0,
    contextAutoCompactEnabled: true,
    contextAutoCompactThreshold: 0.7,
  },
  selectedModel: { id: "test", alias: "test", contextTokens: 239_000 },
  selectedModelAlias: "test",
  t,
});

if (summary.totalTokens >= 100_000) {
  throw new Error(`context usage counts raw stale tool output: ${summary.totalTokens}`);
}

if (summary.percent >= 100) {
  throw new Error(`context usage falsely reports full context: ${summary.percent}%`);
}

console.log(`context usage verification passed: ${summary.totalTokens} tokens, ${summary.percent}%`);
