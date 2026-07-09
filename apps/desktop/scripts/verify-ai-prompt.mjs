import { resolve } from "node:path";
import { build } from "esbuild";

const promptPath = resolve("src/lib/aiSystemPrompt.ts");
const bundle = await build({
  entryPoints: [promptPath],
  bundle: true,
  write: false,
  format: "esm",
  platform: "node",
  target: "es2022",
});
const moduleUrl = `data:text/javascript;base64,${Buffer.from(bundle.outputFiles[0].text).toString("base64")}`;
const { buildASPECTSystemPrompt } = await import(moduleUrl);

const baseContext = {
  preferences: {
    agentMode: "agent",
    selectedEffortId: "xhigh",
    toolRoundLimit: 100,
    toolApprovalMode: "full-access",
  },
  provider: {
    name: "Local",
    protocol: "local-proxy",
  },
  globalInstructions: "",
  projectInstructions: "",
  runtimeToolsAvailable: true,
  selectedAgentInstructions: "Work autonomously from evidence.",
  selectedAgentName: "Agent",
  selectedModel: {
    id: "gpt-5.5",
    alias: "gpt-5.5",
  },
  workspace: {
    root: "/workspace/aspect-ide",
  },
};

const prompt = buildASPECTSystemPrompt(baseContext);
const webPrompt = buildASPECTSystemPrompt({ ...baseContext, runtimeToolsAvailable: false });

const requiredPhrases = [
  "production coding agent built into AspectIDE",
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
  "TerminalContext",
  "TerminalWrite",
  "SecretGuard",
  "Default tool approval mode",
  "Tool round limit",
  "Full Access mode",
  "Agent mode",
  "Automatic mode",
  "Plan mode",
  "Ask mode",
  "Review behavior",
  "Review requests are read-only by default",
  "Do not run test/build/shell commands unless the user explicitly asks for verification",
  "Verification protocol",
  "Failure recovery",
  "GitHub-flavored Markdown",
  "fenced code blocks with language names",
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

const automaticPrompt = buildASPECTSystemPrompt({
  ...baseContext,
  agentBrowserEnabled: true,
  preferences: { ...baseContext.preferences, agentMode: "automatic" },
});
const planPrompt = buildASPECTSystemPrompt({ ...baseContext, preferences: { ...baseContext.preferences, agentMode: "plan" } });
const askPrompt = buildASPECTSystemPrompt({ ...baseContext, preferences: { ...baseContext.preferences, agentMode: "ask" } });
const planWithAgents = buildASPECTSystemPrompt({
  ...baseContext,
  preferences: { ...baseContext.preferences, agentMode: "plan" },
  projectAgentsSnip: "Project AGENTS guidance (auto-loaded, read-only)\n### . вЂ” AGENTS.md\nDo quality work.",
});
if (!automaticPrompt.includes("Vercel agent-browser is fully enabled")) {
  throw new Error("Automatic mode must expose full browser tools like Agent mode.");
}
for (const phrase of [
  "Automatic mode enforcement",
  "NEVER reply with only clarifying questions",
  "first model turn MUST include tool calls",
  "self-contained index.html",
]) {
  if (!automaticPrompt.includes(phrase)) {
    throw new Error(`Automatic mode prompt missing: ${phrase}`);
  }
}
if (automaticPrompt.includes("Shell, TerminalContext, and TerminalWrite are not available in Plan/Ask")) {
  throw new Error("Automatic mode must not disable terminal tools.");
}
for (const modePrompt of [planPrompt, askPrompt]) {
  if (!modePrompt.includes("Shell, TerminalContext, and TerminalWrite are not available in Plan/Ask")) {
    throw new Error("Plan/Ask runtime tool section must state terminal tools are unavailable.");
  }
  if (!modePrompt.includes("read-only Plan or Ask mode")) {
    throw new Error("Plan/Ask must use the compact read-only system prompt.");
  }
  if (modePrompt.includes("Use StrReplace for small exact edits")) {
    throw new Error("Plan/Ask compact prompt must not include full editing tool guidance.");
  }
}
if (!planWithAgents.includes("Project AGENTS guidance (auto-loaded, read-only)")) {
  throw new Error("Inlined project AGENTS snip must appear in the system prompt when provided.");
}
if (planPrompt.length >= prompt.length) {
  throw new Error(`Plan prompt (${planPrompt.length}) should be shorter than Agent prompt (${prompt.length}).`);
}

if (!webPrompt.includes("Runtime tools are not attached")) {
  throw new Error("AI system prompt must clearly explain web/dev chat requests without runtime tools.");
}

// Upper bounds carry headroom for the progress-narration guidance, the CodeGraph
// tool-map line, the numbered Precedence block, and the completion self-check
// (deliberate features); the floor still guards against an accidentally gutted
// prompt. Kept in lockstep with the Rust ceilings in ai_prompt.rs tests.
if (prompt.length < 6_000 || prompt.length > 17_700) {
  throw new Error(`AI system prompt length ${prompt.length} is outside the expected 6000-17700 character range.`);
}
if (automaticPrompt.length > 19_200) {
  throw new Error(`Automatic mode prompt length ${automaticPrompt.length} exceeds 19200 characters.`);
}

console.log(`AI prompt verification passed (${prompt.length} chars).`);
