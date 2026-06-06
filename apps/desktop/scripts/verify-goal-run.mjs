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

const goalRun = await bundleModule("src/lib/aiSessionGoalRun.ts");
const orch = await bundleModule("src/lib/aiChatGoalOrchestration.ts");

const sessionId = "verify-goal-run";
goalRun.startGoalRun(sessionId, "Ship PDF preview routing", {
  agentMode: "agent",
  toolRoundLimit: 32,
  limits: { maxRounds: 4 },
});
goalRun.applyGoalToolProgress(sessionId, { progress: 40 });
const mid = goalRun.getActiveGoalRun(sessionId);
if (!mid || mid.progress !== 40) throw new Error("goal progress should update to 40");

goalRun.applyGoalToolProgress(sessionId, { progress: 100, status: "completed", summary: "Done" });
const afterGoalTool = goalRun.getActiveGoalRun(sessionId);
if (!afterGoalTool || afterGoalTool.phase !== "running" || afterGoalTool.progress !== 100) {
  throw new Error("Goal tool must defer finish until turn is accounted");
}
goalRun.recordGoalRunTurnUsage(sessionId, { promptTokens: 80, completionTokens: 40 });
const doneDecision = goalRun.evaluateGoalRunContinuation(sessionId, [], "agent");
if (doneDecision.continue || doneDecision.reason !== "completed") {
  throw new Error("evaluate should complete deferred goal run");
}
const done = goalRun.getGoalRunSnapshot(sessionId);
if (!done || done.phase !== "completed" || done.progress !== 100 || done.round !== 1) {
  throw new Error(`completed goal run expected with round 1, got phase=${done?.phase} round=${done?.round}`);
}
if (done.promptTokens + done.completionTokens !== 120) {
  throw new Error("goal run should retain token totals from recordGoalRunTurnUsage");
}

const earlyCompleteSession = "verify-goal-early-complete";
goalRun.startGoalRun(earlyCompleteSession, "Read README", { agentMode: "agent", toolRoundLimit: 32 });
goalRun.applyGoalToolProgress(earlyCompleteSession, { progress: 100, status: "completed", summary: "Done" });
goalRun.recordGoalRunTurnUsage(
  earlyCompleteSession,
  { promptTokens: 500, completionTokens: 200 },
  { id: "asst-early", role: "assistant", content: "Summary\n[goal:complete]", timestamp: Date.now() },
);
const earlyRun = goalRun.getGoalRunSnapshot(earlyCompleteSession);
if (!earlyRun || earlyRun.round !== 1 || earlyRun.promptTokens !== 500) {
  throw new Error("early Goal tool complete must still account the finishing turn");
}

const stallSession = "verify-goal-stall";
goalRun.startGoalRun(stallSession, "Refactor routing", { agentMode: "agent", toolRoundLimit: 32, limits: { maxRounds: 8 } });
goalRun.recordGoalRunTurnUsage(stallSession, { promptTokens: 10, completionTokens: 5 });
const stallMsg = {
  id: "a1",
  role: "assistant",
  content: "Done.",
  timestamp: Date.now(),
  turnUsage: { promptTokens: 100, completionTokens: 120, totalTokens: 220, estimatedCostUsd: null },
};
let decision = goalRun.evaluateGoalRunContinuation(stallSession, [stallMsg], "agent");
if (!decision.continue) throw new Error("first stall turn should continue");
goalRun.recordGoalRunTurnUsage(stallSession, { promptTokens: 10, completionTokens: 5 }, stallMsg);
decision = goalRun.evaluateGoalRunContinuation(stallSession, [stallMsg], "agent");
if (decision.continue || decision.reason !== "stall") {
  throw new Error(`second stall turn should stop with stall reason, got ${decision.reason}`);
}

if (!goalRun.isExploratoryGoalRun("тестирование goal")) {
  throw new Error("тестирование goal should be exploratory");
}
const exploratoryMax = goalRun.resolveGoalRunMaxRounds(32, "тестирование goal");
if (exploratoryMax !== 6) throw new Error(`exploratory max rounds expected 6, got ${exploratoryMax}`);

const markerSession = "verify-goal-marker";
goalRun.startGoalRun(markerSession, "Verify markers", { agentMode: "agent", toolRoundLimit: 32, limits: { maxRounds: 4 } });
goalRun.recordGoalRunTurnUsage(markerSession, { promptTokens: 1, completionTokens: 1 });
const markerAssistant = {
  id: "a-marker",
  role: "assistant",
  content: "All checks passed.\n[goal:complete]",
  timestamp: Date.now(),
};
const markerDecision = goalRun.evaluateGoalRunContinuation(markerSession, [markerAssistant], "agent");
if (markerDecision.continue || markerDecision.reason !== "completed") {
  throw new Error("goal:complete marker should finish the run");
}

const smokeSession = "verify-goal-smoke";
goalRun.startGoalRun(smokeSession, "тестирование goal", { agentMode: "agent", toolRoundLimit: 32 });
goalRun.recordGoalRunTurnUsage(smokeSession, { promptTokens: 5, completionTokens: 3 });
goalRun.recordGoalRunTurnUsage(smokeSession, { promptTokens: 5, completionTokens: 3 });
const smokeAssistant = {
  id: "a-smoke",
  role: "assistant",
  content: "Ran List and Read on workspace; goal path works.",
  timestamp: Date.now(),
  toolCalls: [{ tool: "List", status: "success", output: "ok" }],
};
goalRun.applyGoalToolProgress(smokeSession, { progress: 25 });
const smokeDecision = goalRun.evaluateGoalRunContinuation(smokeSession, [smokeAssistant], "agent");
if (smokeDecision.continue || smokeDecision.reason !== "completed") {
  throw new Error("exploratory smoke goal should auto-complete after verification tools");
}

const verdictSession = "verify-goal-verdict";
goalRun.startGoalRun(verdictSession, "Ship feature X", { agentMode: "agent", toolRoundLimit: 32, limits: { maxRounds: 8 } });
const verdictEval = goalRun.applyGoalEvaluatorVerdict(verdictSession, {
  satisfied: true,
  blocked: false,
  reason: "Model confirmed completion.",
  source: "model",
});
if (verdictEval.status !== "satisfied") {
  throw new Error("applyGoalEvaluatorVerdict should complete an active run");
}

const internal = orch.createInternalGoalOrchestrationMessage("silent", "continuation");
const visibleOnly = orch.filterVisibleChatMessages([
  internal,
  { id: "u1", role: "user", content: "hello", timestamp: 1 },
]);
if (visibleOnly.length !== 1 || visibleOnly[0].content !== "hello") {
  throw new Error("internal orchestration messages must be hidden from visible chat");
}

console.log("goal run verification passed");