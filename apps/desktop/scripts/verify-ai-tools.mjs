import { readFile } from "node:fs/promises";
import { resolve } from "node:path";

const runtimePath = resolve("src/lib/aiChatRuntime.ts");
const approvalsRuntimePath = resolve("src/lib/aiRuntimeApprovals.ts");
const checkpointRuntimePath = resolve("src/lib/aiRuntimeCheckpoints.ts");
const runtimeToolsPath = resolve("src/lib/aiRuntimeTools.ts");
const toolCallPath = resolve("src/components/AiToolCall.tsx");
const toolsPanelPath = resolve("src/components/AiToolsPanel.tsx");

const [runtimeSource, approvalsRuntimeSource, checkpointRuntimeSource, runtimeToolsSource, toolCallSource, toolsPanelSource] = await Promise.all([
  readFile(runtimePath, "utf8"),
  readFile(approvalsRuntimePath, "utf8"),
  readFile(checkpointRuntimePath, "utf8"),
  readFile(runtimeToolsPath, "utf8"),
  readFile(toolCallPath, "utf8"),
  readFile(toolsPanelPath, "utf8"),
]);
const approvalSources = [runtimeSource, approvalsRuntimeSource, checkpointRuntimeSource];

const runtimeUnion = parseRuntimeToolUnion(runtimeToolsSource);
const runtimeDefinitions = parseRuntimeToolDefinitions(runtimeToolsSource);
const switchCases = parseSwitchCases(runtimeSource);
const toolCallIcons = parseRecordKeys(toolCallSource, "toolIcons");
const panelTools = parseUiToolNames(toolsPanelSource);

const errors = [
  ...sameSet("RuntimeToolName union", runtimeUnion, "runtimeTools definitions", runtimeDefinitions),
  ...sameSet("RuntimeToolName union", runtimeUnion, "runRuntimeTool switch", switchCases),
  ...sameSet("RuntimeToolName union", runtimeUnion, "AiToolsPanel", panelTools),
  ...sameSet("RuntimeToolName union", runtimeUnion, "AiToolCall icons", toolCallIcons),
  ...duplicates("runtimeTools definitions", runtimeDefinitions),
  ...duplicates("AiToolsPanel", panelTools),
];

for (const dangerous of ["Write", "StrReplace", "PatchEngine", "Checkpoint", "Delete", "Shell", "TerminalWrite"]) {
  if (!runtimeToolsSource.includes(`| "${dangerous}"`)) errors.push(`Runtime tool union is missing dangerous tool ${dangerous}.`);
  if (!approvalSources.some((source) => source.includes(`tool: "${dangerous}"`))) errors.push(`Approval request union is missing dangerous tool ${dangerous}.`);
}

if (!runtimeSource.includes("compactTerminalContext") || !runtimeSource.includes("terminalContext")) {
  errors.push("Terminal tools must be wired to structured terminal context, not only UI labels.");
}

if (errors.length > 0) {
  throw new Error(`AI tools verification failed:\n- ${errors.join("\n- ")}`);
}

console.log(`AI tools verification passed (${runtimeUnion.length} runtime tools, UI/catalog/icon coverage complete).`);

function parseRuntimeToolUnion(source) {
  const match = source.match(/type\s+RuntimeToolName\s*=\s*([^;]+);/s);
  if (!match) throw new Error("RuntimeToolName union not found.");
  return [...match[1].matchAll(/"([A-Za-z0-9_]+)"/g)].map((entry) => entry[1]);
}

function parseRuntimeToolDefinitions(source) {
  const match = source.match(/export\s+const\s+runtimeTools:\s*RuntimeToolDefinition\[\]\s*=\s*\[([\s\S]*?)\n\];/);
  if (!match) throw new Error("runtimeTools definition block not found.");
  return [...match[1].matchAll(/name:\s*"([A-Za-z0-9_]+)"/g)].map((entry) => entry[1]);
}

function parseSwitchCases(source) {
  const match = source.match(/async function runRuntimeTool[\s\S]*?switch \(name\) \{([\s\S]*?)\n\s*default:/);
  if (!match) throw new Error("runRuntimeTool switch not found.");
  return [...match[1].matchAll(/case\s+"([A-Za-z0-9_]+)"/g)].map((entry) => entry[1]);
}

function parseRecordKeys(source, recordName) {
  const match = source.match(new RegExp(`const\\s+${recordName}:\\s*Record<[^>]+>\\s*=\\s*\\{([\\s\\S]*?)\\n\\};`));
  if (!match) throw new Error(`${recordName} record not found.`);
  return [...match[1].matchAll(/^\s*([A-Za-z0-9_]+):/gm)]
    .map((entry) => entry[1])
    .filter((key) => key !== "default");
}

function parseUiToolNames(source) {
  return [...source.matchAll(/name:\s*"([A-Za-z0-9_]+)"/g)].map((entry) => entry[1]);
}

function sameSet(leftName, leftValues, rightName, rightValues) {
  const left = new Set(leftValues);
  const right = new Set(rightValues);
  const missing = [...left].filter((value) => !right.has(value));
  const extra = [...right].filter((value) => !left.has(value));
  const errors = [];
  if (missing.length > 0) errors.push(`${rightName} missing from ${leftName}: ${missing.join(", ")}.`);
  if (extra.length > 0) errors.push(`${rightName} has extra not in ${leftName}: ${extra.join(", ")}.`);
  return errors;
}

function duplicates(name, values) {
  const seen = new Set();
  const duplicateValues = new Set();
  for (const value of values) {
    if (seen.has(value)) duplicateValues.add(value);
    seen.add(value);
  }
  return duplicateValues.size > 0 ? [`${name} has duplicate tools: ${[...duplicateValues].join(", ")}.`] : [];
}
