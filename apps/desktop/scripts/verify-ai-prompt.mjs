import { readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { transform } from "esbuild";

const promptPath = resolve("src/lib/aiSystemPrompt.ts");
const source = await readFile(promptPath, "utf8");
const { code } = await transform(source, { loader: "ts", format: "esm", target: "es2022" });
const moduleUrl = `data:text/javascript;base64,${Buffer.from(code).toString("base64")}`;
const { buildLuxIdeSystemPrompt } = await import(moduleUrl);

const baseContext = {
  preferences: {
    agentMode: "agent",
    selectedEffortId: "xhigh",
    toolApprovalMode: "default",
  },
  provider: {
    name: "Local proxy",
    protocol: "local-proxy",
  },
  runtimeToolsAvailable: true,
  selectedAgentInstructions: "Work autonomously from evidence.",
  selectedAgentName: "Agent",
  selectedModel: {
    id: "gpt-5.5",
    alias: "gpt-5.5",
  },
  workspace: {
    root: "/workspace/lux-ide",
  },
};

const prompt = buildLuxIdeSystemPrompt(baseContext);
const webPrompt = buildLuxIdeSystemPrompt({ ...baseContext, runtimeToolsAvailable: false });

const requiredPhrases = [
  "production coding agent built into Lux IDE",
  "Work from evidence",
  "acceptance criteria",
  "ContextBudgeter",
  "FastContext",
  "ActiveContext",
  "RulesContext",
  "MemoryContext",
  "SemanticSearch",
  "SymbolContext",
  "RelatedFiles",
  "PatchEngine",
  "Checkpoint",
  "TodoWrite",
  "GitContext",
  "ReviewDiff",
  "TestHealth",
  "FailureAnalyzer",
  "SecretGuard",
  "Default tool approval mode",
  "Full Access mode",
  "Agent mode",
  "Plan mode",
  "Ask mode",
  "Review behavior",
  "Verification protocol",
  "Failure recovery",
  "Preserve user work",
  "Do not use TodoWrite as a substitute for evidence gathering",
  "one compact read-only context round",
  "Do not keep reading implementation files just to prepare edits",
  "Do not claim superiority",
];

const missing = requiredPhrases.filter((phrase) => !prompt.includes(phrase));
if (missing.length > 0) {
  throw new Error(`AI system prompt is missing required phrase(s): ${missing.join(", ")}`);
}

if (!webPrompt.includes("Runtime tools are not attached")) {
  throw new Error("AI system prompt must clearly explain web/dev chat requests without runtime tools.");
}

if (prompt.length < 6_000 || prompt.length > 12_000) {
  throw new Error(`AI system prompt length ${prompt.length} is outside the expected 6000-12000 character range.`);
}

console.log(`AI prompt verification passed (${prompt.length} chars).`);
