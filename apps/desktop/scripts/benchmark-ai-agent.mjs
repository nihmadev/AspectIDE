import { exec as execCallback } from "node:child_process";
import { mkdtemp, readFile, readdir, rm, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve, relative, sep } from "node:path";
import { pathToFileURL } from "node:url";
import { promisify } from "node:util";
import { transform } from "esbuild";

const exec = promisify(execCallback);

const endpoint = normalizeEndpoint(process.env.LUX_AI_PROXY_URL ?? "http://127.0.0.1:8799/v1");
const model = process.env.LUX_AI_MODEL ?? "gpt-5.5";
const reasoningEffort = process.env.LUX_AI_REASONING ?? "xhigh";
const timeoutMs = Number(process.env.LUX_AI_AGENT_BENCH_TIMEOUT_MS ?? 180_000);
const keepWorkspace = process.env.LUX_AI_AGENT_BENCH_KEEP === "1";

const { buildLuxIdeSystemPrompt } = await loadPromptBuilder();

export const scenarios = [
  {
    id: "fix-cart-total",
    title: "Fix a failing implementation and verify tests",
    task: "Fix the failing cart total tests. Work autonomously: inspect the relevant files, edit the implementation, run the test command, and summarize what changed.",
    setup: setupCartWorkspace,
    evaluate: evaluateCartWorkspace,
  },
  {
    id: "review-access-control",
    title: "Review-only workflow reports findings without edits",
    task: "Review this workspace for security regressions. Do not modify files or run mutating commands. Lead with findings and include file evidence.",
    agentMode: "ask",
    selectedAgentName: "Ask",
    selectedAgentInstructions: "Review and explain from evidence. Do not modify files or run mutating commands.",
    setup: setupReviewWorkspace,
    evaluate: evaluateReviewWorkspace,
    evaluateTools: evaluateReviewToolDiscipline,
    maxRounds: 6,
  },
  {
    id: "fix-product-slugs",
    title: "Multi-file edit and failure recovery with tests",
    task: "Fix the failing product URL slug tests. Inspect implementation and tests, update all necessary source files, run the test command, and summarize the verified changes.",
    setup: setupProductSlugWorkspace,
    evaluate: evaluateProductSlugWorkspace,
    maxRounds: 12,
  },
];

if (isMain()) {
  const summary = await runLuxAgentBenchmark({ print: true });
  if (!summary.passedAll) process.exitCode = 1;
}

export async function runLuxAgentBenchmark({ print = true } = {}) {
  const results = [];
  for (const scenario of scenarios) {
    const workspaceRoot = await mkdtemp(resolve(tmpdir(), `lux-ai-agent-${scenario.id}-`));
    try {
      await scenario.setup(workspaceRoot);
      const result = await runLuxScenario(scenario, workspaceRoot);
      const evaluated = await scenario.evaluate(workspaceRoot, result);
      const combined = { ...result, ...evaluated, passed: result.passed && evaluated.passed, failures: [...result.failures, ...evaluated.failures] };
      results.push(combined);
      if (print) printScenarioResult(scenario, combined, workspaceRoot);
    } finally {
      if (!keepWorkspace) await rm(workspaceRoot, { force: true, recursive: true });
    }
  }

  const passed = results.filter((result) => result.passed).length;
  const score = results.reduce((sum, result) => sum + result.score, 0);
  const maxScore = results.reduce((sum, result) => sum + result.maxScore, 0);
  const percent = Math.round((score / maxScore) * 100);
  const passedAll = passed === results.length && percent >= 90;
  if (print) console.log(`AI agent benchmark: ${passed}/${results.length} scenarios passed, score ${score}/${maxScore} (${percent}%).`);
  return { maxScore, passed, passedAll, percent, results, score };
}

export async function runLuxScenario(scenario, workspaceRoot) {
  const toolLog = [];
  const messages = [
    { role: "system", content: buildLuxIdeSystemPrompt(context(workspaceRoot, scenario)) },
    { role: "user", content: scenario.task },
  ];
  let finalContent = "";
  const failures = [];
  const maxRounds = scenario.maxRounds ?? 10;

  for (let round = 0; round < maxRounds; round += 1) {
    const body = await requestCompletion(messages, scenario.id);
    const message = body?.choices?.[0]?.message ?? {};
    finalContent = typeof message.content === "string" ? message.content : "";
    const toolCalls = Array.isArray(message.tool_calls) ? normalizeToolCalls(message.tool_calls, round) : [];

    if (toolCalls.length === 0) {
      const evaluated = (scenario.evaluateTools ?? evaluateEditToolDiscipline)(toolLog);
      return { ...evaluated, finalContent, toolLog, failures: [...failures, ...evaluated.failures] };
    }

    messages.push({ role: "assistant", content: message.content ?? null, tool_calls: toolCalls });

    for (const call of toolCalls) {
      const name = call.function.name;
      const args = parseArguments(call.function.arguments);
      try {
        const result = await executeTool(name, args, workspaceRoot, toolLog);
        messages.push({ role: "tool", tool_call_id: call.id, content: JSON.stringify(result) });
      } catch (error) {
        const result = { error: error instanceof Error ? error.message : String(error) };
        messages.push({ role: "tool", tool_call_id: call.id, content: JSON.stringify(result) });
        toolLog.push({ name, status: "error", args, result });
      }
    }
  }

  failures.push(`model did not finish within ${maxRounds} tool rounds`);
  return { passed: false, score: 0, maxScore: 4, finalContent, toolLog, failures };
}

function printScenarioResult(scenario, combined, workspaceRoot) {
  const mark = combined.passed ? "PASS" : "FAIL";
  console.log(`${mark} ${scenario.id} (${combined.score}/${combined.maxScore}) ${scenario.title}`);
  if (!combined.passed) {
    for (const failure of combined.failures) console.log(`  - ${failure}`);
    console.log(`  tools: ${combined.toolLog.map((entry) => entry.name).join(" -> ") || "none"}`);
    console.log(`  workspace: ${workspaceRoot}`);
    console.log(`  final: ${truncate(combined.finalContent.replace(/\s+/g, " "), 500)}`);
  }
}

function isMain() {
  return Boolean(process.argv[1]) && import.meta.url === pathToFileURL(process.argv[1]).href;
}

async function requestCompletion(messages, scenarioId) {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), timeoutMs);
  try {
    const response = await fetch(`${endpoint}/chat/completions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        model,
        messages,
        temperature: 0,
        reasoning_effort: reasoningEffort,
        stream: false,
        tools: agentTools(),
        tool_choice: "auto",
      }),
      signal: controller.signal,
    });
    const body = await response.json().catch(async () => ({ error: { message: await response.text().catch(() => "non-JSON response") } }));
    if (!response.ok) throw new Error(`${scenarioId}: HTTP ${response.status}: ${body?.error?.message ?? JSON.stringify(body)}`);
    return body;
  } finally {
    clearTimeout(timeout);
  }
}

function agentTools() {
  return [
  functionTool("ActiveContext", "Return the current benchmark IDE state and workspace root.", {}),
  functionTool("ContextBudgeter", "Return ranked context for the benchmark workspace and task.", {
    query: stringParameter("Task or topic."),
  }),
  functionTool("Read", "Read a text file inside the benchmark workspace.", {
    path: stringParameter("Workspace-relative or absolute path."),
  }, ["path"]),
  functionTool("Grep", "Search text in workspace files.", {
    query: stringParameter("Text to search for."),
  }, ["query"]),
  functionTool("StrReplace", "Replace an exact text fragment in a workspace file.", {
    path: stringParameter("Workspace-relative or absolute path."),
    oldText: stringParameter("Exact text to replace."),
    newText: stringParameter("Replacement text."),
    expectedReplacements: { type: "number", description: "Expected replacement count, default 1." },
  }, ["path", "oldText", "newText"]),
  functionTool("PatchEngine", "Apply guarded create/rewrite/replace operations inside the benchmark workspace.", {
    operations: {
      type: "array",
      items: {
        type: "object",
        properties: {
          action: stringParameter("create, rewrite, or replace."),
          path: stringParameter("Workspace-relative or absolute path."),
          text: stringParameter("Complete file contents for create/rewrite."),
          oldText: stringParameter("Exact text for replace."),
          newText: stringParameter("Replacement text for replace."),
          expectedReplacements: { type: "number" },
        },
        required: ["action", "path"],
      },
    },
  }, ["operations"]),
  functionTool("Write", "Create or fully rewrite a text file inside the benchmark workspace.", {
    path: stringParameter("Workspace-relative or absolute path."),
    text: stringParameter("Complete file contents."),
  }, ["path", "text"]),
  functionTool("Shell", "Run a safe benchmark command. Allowed: npm test, npm run test, node --test.", {
    command: stringParameter("Command to run."),
  }, ["command"]),
  functionTool("TestHealth", "Run the benchmark test command and return compact output.", {}),
  functionTool("FailureAnalyzer", "Analyze compact failure output from the benchmark.", {
    log: stringParameter("Failure log."),
  }),
  functionTool("TodoWrite", "Update the benchmark task list. This does not inspect or edit files.", {
    todos: { type: "array", items: { type: "object" } },
  }),
  ];
}

async function executeTool(name, args, workspaceRoot, toolLog) {
  let result;
  switch (name) {
    case "ActiveContext":
      result = { workspaceRoot, activeDocument: null, openDocuments: [], approvalMode: "default" };
      break;
    case "ContextBudgeter":
      result = await contextBudgeter(workspaceRoot, String(args.query ?? ""));
      break;
    case "Read":
      result = { path: toRelativePath(resolveInsideWorkspace(workspaceRoot, stringArg(args, "path")), workspaceRoot), text: await readFile(resolveInsideWorkspace(workspaceRoot, stringArg(args, "path")), "utf8") };
      break;
    case "Grep":
      result = await grepWorkspace(workspaceRoot, stringArg(args, "query"));
      break;
    case "StrReplace":
      result = await strReplace(workspaceRoot, args);
      break;
    case "PatchEngine":
      result = await patchEngine(workspaceRoot, args);
      break;
    case "Write":
      result = await writeWorkspaceFile(workspaceRoot, stringArg(args, "path"), stringArg(args, "text"));
      break;
    case "Shell":
      result = await runAllowedCommand(workspaceRoot, stringArg(args, "command"));
      break;
    case "TestHealth":
      result = await runAllowedCommand(workspaceRoot, "npm test");
      break;
    case "FailureAnalyzer":
      result = failureAnalyzer(String(args.log ?? ""));
      break;
    case "TodoWrite":
      result = { ok: true, todos: Array.isArray(args.todos) ? args.todos : [] };
      break;
    default:
      throw new Error(`Unsupported benchmark tool: ${name}`);
  }
  toolLog.push({ name, status: "success", args, result });
  return result;
}

async function setupCartWorkspace(root) {
  await mkdir(resolve(root, "src"), { recursive: true });
  await mkdir(resolve(root, "tests"), { recursive: true });
  await writeFile(resolve(root, "package.json"), `${JSON.stringify({ type: "module", scripts: { test: "node --test" } }, null, 2)}\n`);
  await writeFile(resolve(root, "src/cart.mjs"), `export function cartTotal(items, taxRate = 0) {
  const subtotal = items.reduce((sum, item) => sum + item.price * (item.quantity || 0), 0);
  return Math.round((subtotal + subtotal * taxRate) * 100) / 100;
}
`);
  await writeFile(resolve(root, "tests/cart.test.mjs"), `import assert from "node:assert/strict";
import test from "node:test";
import { cartTotal } from "../src/cart.mjs";

test("cartTotal handles quantity and tax", () => {
  assert.equal(cartTotal([{ price: 12.5, quantity: 2 }, { price: 3, quantity: 1 }], 0.2), 33.6);
});

test("cartTotal treats missing quantity as one item", () => {
  assert.equal(cartTotal([{ price: 10 }, { price: 2.555, quantity: 2 }]), 15.11);
});
`);
}

async function evaluateCartWorkspace(root, result) {
  const failures = [];
  let score = result.score;
  let maxScore = result.maxScore + 3;
  const cartText = await readFile(resolve(root, "src/cart.mjs"), "utf8");
  const finalTest = await runAllowedCommand(root, "npm test");
  if (finalTest.exitCode === 0) score += 1;
  else failures.push(`final npm test failed: ${truncate(finalTest.output, 500)}`);
  if (!cartText.includes("quantity || 0")) score += 1;
  else failures.push("implementation still treats missing quantity as zero");
  if (hasAny(result.finalContent, ["test", "verify", "verified", "npm test", "passed"])) score += 1;
  else failures.push("final response did not mention verification");
  return { passed: failures.length === 0, score, maxScore, failures };
}

async function setupReviewWorkspace(root) {
  await mkdir(resolve(root, "src"), { recursive: true });
  await mkdir(resolve(root, "tests"), { recursive: true });
  await writeFile(resolve(root, "package.json"), `${JSON.stringify({ type: "module", scripts: { test: "node --test" } }, null, 2)}\n`);
  await writeFile(resolve(root, "src/access.mjs"), `export function canAccess(user, resource) {
  if (!user) return false;
  if (user.role === "admin") return true;
  return true;
}
`);
  await writeFile(resolve(root, "tests/access.test.mjs"), `import assert from "node:assert/strict";
import test from "node:test";
import { canAccess } from "../src/access.mjs";

test("non-admin cannot access private resources", () => {
  assert.equal(canAccess({ role: "viewer" }, { visibility: "private" }), false);
});
`);
}

async function evaluateReviewWorkspace(root, result) {
  const failures = [];
  let score = result.score;
  let maxScore = result.maxScore + 3;
  const accessText = await readFile(resolve(root, "src/access.mjs"), "utf8");
  if (accessText.includes("return true;\n}")) score += 1;
  else failures.push("review-only scenario modified src/access.mjs");
  const start = result.finalContent.trim().slice(0, 320).toLowerCase();
  if (hasAny(start, ["finding", "issue", "security", "severity", "problem", "vulnerability"])) score += 1;
  else failures.push("review did not lead with findings");
  if (hasAny(result.finalContent, ["src/access.mjs", "return true", "canAccess", "private", "viewer"])) score += 1;
  else failures.push("review did not include file/code evidence");
  return { passed: failures.length === 0, score, maxScore, failures };
}

async function setupProductSlugWorkspace(root) {
  await mkdir(resolve(root, "src"), { recursive: true });
  await mkdir(resolve(root, "tests"), { recursive: true });
  await writeFile(resolve(root, "package.json"), `${JSON.stringify({ type: "module", scripts: { test: "node --test" } }, null, 2)}\n`);
  await writeFile(resolve(root, "src/slug.mjs"), `export function slugify(value) {
  return String(value).trim().toLowerCase().replace(/\\s+/g, "-");
}
`);
  await writeFile(resolve(root, "src/product-url.mjs"), `import { slugify } from "./slug.mjs";

export function productUrl(product) {
  return "/products/" + slugify(product.name);
}
`);
  await writeFile(resolve(root, "tests/product-url.test.mjs"), `import assert from "node:assert/strict";
import test from "node:test";
import { slugify } from "../src/slug.mjs";
import { productUrl } from "../src/product-url.mjs";

test("slugify removes punctuation and collapses separators", () => {
  assert.equal(slugify("  ACME Deluxe, 20% Pack!! "), "acme-deluxe-20-pack");
});

test("productUrl includes stable id and slug", () => {
  assert.equal(productUrl({ id: 42, name: "ACME Deluxe, 20% Pack!!" }), "/products/42-acme-deluxe-20-pack");
});
`);
}

async function evaluateProductSlugWorkspace(root, result) {
  const failures = [];
  let score = result.score;
  let maxScore = result.maxScore + 4;
  const slugText = await readFile(resolve(root, "src/slug.mjs"), "utf8");
  const productText = await readFile(resolve(root, "src/product-url.mjs"), "utf8");
  const finalTest = await runAllowedCommand(root, "npm test");
  if (finalTest.exitCode === 0) score += 1;
  else failures.push(`final npm test failed: ${truncate(finalTest.output, 500)}`);
  if (/[^a-z0-9]+/.test(slugText) || slugText.includes("replace(/^-|-$/g")) score += 1;
  else failures.push("slugify does not appear to remove punctuation/collapse separators robustly");
  if (productText.includes("product.id") || productText.includes("${product.id}")) score += 1;
  else failures.push("productUrl does not include product id");
  if (hasAny(result.finalContent, ["src/slug.mjs", "src/product-url.mjs", "npm test", "passed", "verified"])) score += 1;
  else failures.push("final response did not summarize multi-file verification");
  return { passed: failures.length === 0, score, maxScore, failures };
}

function evaluateEditToolDiscipline(toolLog) {
  const names = toolLog.map((entry) => entry.name);
  const failures = [];
  let score = 0;
  const maxScore = 4;
  const firstMutatingIndex = names.findIndex((name) => ["StrReplace", "PatchEngine", "Write"].includes(name));
  const firstReadIndex = names.findIndex((name) => ["ContextBudgeter", "ActiveContext", "Read", "Grep"].includes(name));
  const verifyAfterEditIndex = names.findIndex((name, index) => index > firstMutatingIndex && ["Shell", "TestHealth"].includes(name));
  if (firstReadIndex >= 0) score += 1;
  else failures.push("model did not gather file/context evidence");
  if (firstMutatingIndex >= 0) score += 1;
  else failures.push("model did not edit the workspace");
  if (firstReadIndex >= 0 && firstMutatingIndex >= 0 && firstReadIndex < firstMutatingIndex) score += 1;
  else failures.push("model edited before reading/context gathering");
  if (verifyAfterEditIndex > firstMutatingIndex && firstMutatingIndex >= 0) score += 1;
  else failures.push("model did not verify after editing");
  return { passed: failures.length === 0, score, maxScore, failures };
}

function evaluateReviewToolDiscipline(toolLog) {
  const names = toolLog.map((entry) => entry.name);
  const failures = [];
  let score = 0;
  const maxScore = 3;
  if (names.some((name) => ["ContextBudgeter", "ActiveContext", "Read", "Grep"].includes(name))) score += 1;
  else failures.push("review did not gather workspace evidence");
  if (!names.some((name) => ["StrReplace", "PatchEngine", "Write", "Shell", "TestHealth"].includes(name))) score += 1;
  else failures.push("review-only scenario used mutating or command tools");
  if (!toolLog.some((entry) => entry.status === "error")) score += 1;
  else failures.push("review tool flow produced errors");
  return { passed: failures.length === 0, score, maxScore, failures };
}

async function contextBudgeter(root, query) {
  const files = await listWorkspaceFiles(root);
  return {
    query,
    selected: files.map((path) => ({ kind: classifyWorkspaceFile(path), path, reason: reasonForWorkspaceFile(path) })),
    packageJson: await readFile(resolve(root, "package.json"), "utf8"),
  };
}

async function grepWorkspace(root, query) {
  const files = await listWorkspaceFiles(root);
  const hits = [];
  for (const file of files) {
    const text = await readFile(resolve(root, file), "utf8");
    const lines = text.split(/\r?\n/);
    for (const [index, line] of lines.entries()) {
      if (line.toLowerCase().includes(query.toLowerCase())) hits.push({ path: file, line: index + 1, text: line });
    }
  }
  return { hits };
}

async function listWorkspaceFiles(root, dir = "") {
  const entries = await readdir(resolve(root, dir), { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    if (entry.name === "node_modules" || entry.name === ".git") continue;
    const child = dir ? `${dir}/${entry.name}` : entry.name;
    if (entry.isDirectory()) files.push(...await listWorkspaceFiles(root, child));
    else if (/\.(mjs|js|json|md)$/.test(entry.name)) files.push(child);
  }
  return files.sort();
}

function classifyWorkspaceFile(path) {
  if (path.startsWith("tests/") || /\.test\./.test(path)) return "test";
  if (path === "package.json") return "manifest";
  if (path.startsWith("src/")) return "source";
  return "file";
}

function reasonForWorkspaceFile(path) {
  if (path.startsWith("tests/") || /\.test\./.test(path)) return "Validation target.";
  if (path === "package.json") return "Contains test command.";
  if (path.startsWith("src/")) return "Source implementation.";
  return "Workspace file.";
}

async function strReplace(root, args) {
  const target = resolveInsideWorkspace(root, stringArg(args, "path"));
  const oldText = stringArg(args, "oldText");
  const newText = stringArg(args, "newText");
  const expected = Number.isFinite(Number(args.expectedReplacements)) ? Number(args.expectedReplacements) : 1;
  const text = await readFile(target, "utf8");
  const count = text.split(oldText).length - 1;
  if (count !== expected) throw new Error(`Expected ${expected} replacement(s), found ${count}.`);
  await writeFile(target, text.split(oldText).join(newText));
  return { path: toRelativePath(target, root), replacements: count };
}

async function patchEngine(root, args) {
  if (!Array.isArray(args.operations)) throw new Error("PatchEngine requires operations array.");
  const applied = [];
  for (const operation of args.operations) {
    const action = String(operation.action ?? "").toLowerCase();
    if (action === "replace") {
      applied.push(await strReplace(root, operation));
    } else if (action === "rewrite" || action === "create") {
      applied.push(await writeWorkspaceFile(root, stringArg(operation, "path"), stringArg(operation, "text")));
    } else {
      throw new Error(`Unsupported PatchEngine action: ${action}`);
    }
  }
  return { applied };
}

async function writeWorkspaceFile(root, path, text) {
  const target = resolveInsideWorkspace(root, path);
  await mkdir(dirname(target), { recursive: true });
  await writeFile(target, text);
  return { path: toRelativePath(target, root), bytes: Buffer.byteLength(text) };
}

async function runAllowedCommand(root, command) {
  const normalized = command.trim().replace(/\s+/g, " ");
  if (!["npm test", "npm run test", "node --test"].includes(normalized)) {
    throw new Error(`Command is not allowed in benchmark: ${command}`);
  }
  try {
    const { stdout, stderr } = await exec(normalized, { cwd: root, timeout: 30_000, windowsHide: true });
    return { command: normalized, exitCode: 0, output: truncate(`${stdout}\n${stderr}`.trim(), 4_000) };
  } catch (error) {
    return {
      command: normalized,
      exitCode: typeof error.code === "number" ? error.code : 1,
      output: truncate(`${error.stdout ?? ""}\n${error.stderr ?? ""}`.trim() || error.message, 4_000),
    };
  }
}

function failureAnalyzer(log) {
  const lines = log.split(/\r?\n/).filter((line) => /fail|error|expected|actual|assert|not ok/i.test(line)).slice(0, 20);
  return { findings: lines, nextAction: "Inspect src/cart.mjs and compare quantity handling with tests/cart.test.mjs." };
}

function context(workspaceRoot, scenario = {}) {
  const agentMode = scenario.agentMode ?? "agent";
  return {
    preferences: { agentMode, selectedEffortId: reasoningEffort, toolApprovalMode: agentMode === "agent" ? "full-access" : "default" },
    provider: { name: "Local", protocol: "local-proxy" },
    runtimeToolsAvailable: true,
    selectedAgentInstructions: scenario.selectedAgentInstructions ?? "Work autonomously from evidence. Edit the temp workspace and verify with the provided test command before reporting done.",
    selectedAgentName: scenario.selectedAgentName ?? "Agent",
    selectedModel: { id: model, alias: model },
    workspace: { root: workspaceRoot },
  };
}

async function loadPromptBuilder() {
  const promptPath = resolve("src/lib/aiSystemPrompt.ts");
  const source = await readFile(promptPath, "utf8");
  const { code } = await transform(source, { loader: "ts", format: "esm", target: "es2022" });
  return import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
}

function functionTool(name, description, properties, required = []) {
  return { type: "function", function: { name, description, parameters: { type: "object", properties, required, additionalProperties: false } } };
}

function stringParameter(description) {
  return { type: "string", description };
}

function normalizeToolCalls(toolCalls, round) {
  return toolCalls
    .map((call, index) => {
      const name = call?.function?.name;
      if (typeof name !== "string" || !name) return null;
      return {
        id: typeof call.id === "string" && call.id ? call.id : `call_${round}_${index}`,
        type: "function",
        function: { name, arguments: typeof call?.function?.arguments === "string" ? call.function.arguments : "{}" },
      };
    })
    .filter(Boolean);
}

function parseArguments(value) {
  try {
    const parsed = JSON.parse(value || "{}");
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function stringArg(args, key) {
  const value = args[key];
  if (typeof value !== "string" || !value) throw new Error(`Missing string argument: ${key}`);
  return value;
}

function resolveInsideWorkspace(root, path) {
  const target = resolve(root, path);
  const rel = relative(root, target);
  if (rel.startsWith("..") || rel === "" && target !== root || rel.includes(`..${sep}`)) throw new Error(`Path escapes workspace: ${path}`);
  return target;
}

function toRelativePath(path, root) {
  return relative(root, path).split(sep).join("/");
}

function normalizeEndpoint(value) {
  return value.trim().replace(/\/+$/g, "");
}

function hasAny(text, candidates) {
  const lower = text.toLowerCase();
  return candidates.some((candidate) => lower.includes(candidate.toLowerCase()));
}

function truncate(text, maxLength) {
  return text.length <= maxLength ? text : `${text.slice(0, maxLength - 3)}...`;
}
