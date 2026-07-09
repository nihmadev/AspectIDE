import { exec as execCallback } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { promisify } from "node:util";
import { runAspectScenario, scenarios } from "./benchmark-ai-agent.mjs";

const exec = promisify(execCallback);

const includeExternal = process.env.ASPECT_AI_COMPARE_EXTERNAL === "1";
const keepWorkspace = process.env.ASPECT_AI_AGENT_BENCH_KEEP === "1";
const externalTimeoutMs = Number(process.env.ASPECT_AI_COMPARE_TIMEOUT_MS ?? 300_000);

const adapters = [
  { id: "Aspect", label: "AspectIDE AI", kind: "Aspect" },
  { id: "codex", label: "Codex CLI", kind: "external", command: "codex", versionCommand: "codex --version", run: runCodex },
  { id: "claude", label: "Claude Code", kind: "external", command: "claude", versionCommand: "claude --version", run: runClaude },
  { id: "opencode", label: "OpenCode", kind: "external", command: "opencode", versionCommand: "opencode --version", run: runOpenCode },
];

const selectedAdapters = includeExternal ? adapters : adapters.filter((adapter) => adapter.id === "Aspect");

if (!includeExternal) {
  console.log("External CLI adapters are available but disabled. Set ASPECT_AI_COMPARE_EXTERNAL=1 to run Codex/Claude/OpenCode.");
}

const scorecards = [];
for (const adapter of selectedAdapters) {
  const availability = await adapterAvailability(adapter);
  if (!availability.available) {
    scorecards.push({ adapter, skipped: true, reason: availability.reason, results: [], score: 0, maxScore: 0, percent: 0, passed: 0 });
    console.log(`SKIP ${adapter.label}: ${availability.reason}`);
    continue;
  }

  const results = [];
  for (const scenario of scenarios) {
    const workspaceRoot = await mkdtemp(resolve(tmpdir(), `aspect-ai-compare-${adapter.id}-${scenario.id}-`));
    try {
      await scenario.setup(workspaceRoot);
      const runResult = adapter.kind === "Aspect"
        ? await runAspectScenario(scenario, workspaceRoot)
        : await adapter.run(scenario, workspaceRoot);
      const evaluated = await scenario.evaluate(workspaceRoot, runResult);
      const combined = { ...runResult, ...evaluated, passed: runResult.passed && evaluated.passed, failures: [...runResult.failures, ...evaluated.failures] };
      results.push(combined);
      printScenario(adapter, scenario, combined);
    } finally {
      if (!keepWorkspace) await rm(workspaceRoot, { force: true, recursive: true });
    }
  }

  const score = results.reduce((sum, result) => sum + result.score, 0);
  const maxScore = results.reduce((sum, result) => sum + result.maxScore, 0);
  const passed = results.filter((result) => result.passed).length;
  const percent = maxScore > 0 ? Math.round((score / maxScore) * 100) : 0;
  scorecards.push({ adapter, skipped: false, version: availability.version, results, score, maxScore, percent, passed });
}

console.log("\nAI agent comparison scorecard");
for (const card of scorecards) {
  if (card.skipped) {
    console.log(`- ${card.adapter.label}: SKIPPED (${card.reason})`);
    continue;
  }
  console.log(`- ${card.adapter.label}: ${card.passed}/${scenarios.length} scenarios, ${card.score}/${card.maxScore} (${card.percent}%)${card.version ? `, ${card.version}` : ""}`);
}

const aspectCard = scorecards.find((card) => card.adapter.id === "Aspect");
const externalCards = scorecards.filter((card) => !card.skipped && card.adapter.id !== "Aspect");
if (aspectCard && externalCards.length > 0) {
  const bestExternal = externalCards.reduce((best, card) => card.percent > best.percent ? card : best, externalCards[0]);
  console.log(`Best external: ${bestExternal.adapter.label} ${bestExternal.percent}%. Aspect: ${aspectCard.percent}%.`);
  if (aspectCard.percent < bestExternal.percent) process.exitCode = 1;
} else if (!aspectCard || aspectCard.percent < 90) {
  process.exitCode = 1;
}

async function adapterAvailability(adapter) {
  if (adapter.kind === "Aspect") return { available: true, version: "local prompt/runtime" };
  const version = await safeExec(adapter.versionCommand, process.cwd(), 20_000);
  if (version.exitCode !== 0) return { available: false, reason: `${adapter.command} not available: ${version.output}` };
  return { available: true, version: firstLine(version.output) };
}

async function runCodex(scenario, workspaceRoot) {
  const prompt = externalPrompt(scenario);
  const command = `codex exec --skip-git-repo-check --ephemeral --sandbox workspace-write --ask-for-approval never --cd ${quote(workspaceRoot)} ${quote(prompt)}`;
  return runExternalCommand(command, workspaceRoot);
}

async function runClaude(scenario, workspaceRoot) {
  const prompt = externalPrompt(scenario);
  const permissionMode = scenario.agentMode === "ask" ? "default" : "bypassPermissions";
  const command = `claude --print --permission-mode ${permissionMode} --model sonnet --effort xhigh ${quote(prompt)}`;
  return runExternalCommand(command, workspaceRoot);
}

async function runOpenCode(scenario, workspaceRoot) {
  const prompt = externalPrompt(scenario);
  const command = `opencode run --dir ${quote(workspaceRoot)} --format default --dangerously-skip-permissions --variant max ${quote(prompt)}`;
  return runExternalCommand(command, workspaceRoot);
}

async function runExternalCommand(command, workspaceRoot) {
  const result = await safeExec(command, workspaceRoot, externalTimeoutMs);
  return {
    finalContent: result.output,
    failures: result.exitCode === 0 ? [] : [`external command exited ${result.exitCode}: ${truncate(result.output, 700)}`],
    maxScore: 1,
    passed: result.exitCode === 0,
    score: result.exitCode === 0 ? 1 : 0,
    toolLog: [{ name: "ExternalCLI", status: result.exitCode === 0 ? "success" : "error", args: { command }, result }],
  };
}

async function safeExec(command, cwd, timeout) {
  try {
    const { stdout, stderr } = await exec(command, { cwd, timeout, windowsHide: true, maxBuffer: 1024 * 1024 * 16 });
    return { exitCode: 0, output: `${stdout}\n${stderr}`.trim() };
  } catch (error) {
    return {
      exitCode: typeof error.code === "number" ? error.code : 1,
      output: `${error.stdout ?? ""}\n${error.stderr ?? ""}`.trim() || error.message,
    };
  }
}

function externalPrompt(scenario) {
  return [
    scenario.task,
    "",
    "Benchmark rules:",
    "- Work only inside this temporary workspace.",
    "- Use the repository files and tests as the source of truth.",
    "- For edit tasks, inspect files before editing and run npm test before final response.",
    "- For review-only tasks, do not edit files and lead with findings.",
    "- Keep the final response concise and include verification evidence.",
  ].join("\n");
}

function printScenario(adapter, scenario, result) {
  const mark = result.passed ? "PASS" : "FAIL";
  console.log(`${mark} ${adapter.id}/${scenario.id} (${result.score}/${result.maxScore})`);
  if (!result.passed) {
    for (const failure of result.failures) console.log(`  - ${failure}`);
    console.log(`  final: ${truncate(result.finalContent.replace(/\s+/g, " "), 500)}`);
  }
}

function quote(value) {
  return `"${String(value).replace(/"/g, "\\\"")}"`;
}

function firstLine(value) {
  return value.split(/\r?\n/).find(Boolean)?.trim() ?? "";
}

function truncate(text, maxLength) {
  return text.length <= maxLength ? text : `${text.slice(0, maxLength - 3)}...`;
}
