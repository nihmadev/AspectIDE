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

const usage = await bundleModule("src/lib/aiTurnUsage.ts");

const fromBody = usage.extractTurnTokenUsage({
  usage: { prompt_tokens: 1200, completion_tokens: 340, reasoning_tokens: 90 },
});
if (!fromBody || fromBody.promptTokens !== 1200 || fromBody.completionTokens !== 430) {
  throw new Error(`extractTurnTokenUsage failed: ${JSON.stringify(fromBody)}`);
}

const estimated = usage.estimateTurnUsageFromAssistant({
  id: "a1",
  role: "assistant",
  content: "Hello ".repeat(200),
  toolCalls: [{
    id: "t1",
    tool: "Read",
    status: "success",
    input: "{}",
    output: "file ".repeat(300),
    startTime: Date.now(),
  }],
  timestamp: Date.now(),
});
if (!estimated || estimated.totalTokens <= 0) {
  throw new Error("estimateTurnUsageFromAssistant should return positive tokens");
}

// Simulate stream accumulator final body shape
const streamBody = {
  usage: { input_tokens: 900, output_tokens: 250 },
  choices: [{ message: { role: "assistant", content: "done" } }],
};
const streamParsed = usage.extractTurnTokenUsage(streamBody);
if (!streamParsed || streamParsed.promptTokens !== 900) {
  throw new Error(`stream-shaped usage parse failed: ${JSON.stringify(streamParsed)}`);
}

console.log("turn usage verification passed");