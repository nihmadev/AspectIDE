import type { AiChatMessage, AiChatSendInput } from "./../chat/types";
import {
  createRelatedFileDescriptor,
  isDocsContextPath,
  isMemoryContextPath,
  isRulesContextPath,
  scoreDocsFile,
  scoreMemoryFile,
  scoreRulesFile,
  tokenizeRelatedQuery,
  type RelatedFileDescriptor,
} from "./file-context";
import { booleanArg, clamp, isRecord, numberArg, readErrorMessage, stringArg, toolJson, truncateText, type ToolResult, type UnknownRecord } from "./shared";
import { luxCommands } from "./../../tauri/commands";

type ContextFile = {
  path: string;
  relativePath: string;
  size: number | null;
  truncated: boolean;
  text: string;
  error?: string;
};

type MemorySignal = {
  source: string;
  line: number;
  kind: "decision" | "preference" | "runtime" | "planning" | "heading";
  score: number;
  text: string;
};

export async function rulesContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const maxFiles = clamp(numberArg(args, "maxFiles", 12), 1, 40);
  const entries = await luxCommands.fsListFiles(clamp(input.preferences.maxIndexedFiles, 500, 20_000));
  const workspaceRoot = input.workspace?.root ?? "";
  const queryTokens = tokenizeRelatedQuery(query);
  const candidates = entries
    .filter((entry) => entry.kind === "file" && isRulesContextPath(entry.path, workspaceRoot))
    .map((entry) => createRelatedFileDescriptor(entry, workspaceRoot))
    .sort((left, right) => scoreRulesFile(right, queryTokens) - scoreRulesFile(left, queryTokens) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, maxFiles);
  const files = await readContextFiles(candidates, 10_000);
  return toolJson("RulesContext", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    count: files.length,
    files,
    notes: files.length > 0
      ? ["Follow these local rules when choosing tools, editing code, and explaining changes."]
      : ["No dedicated project rule files were found in the current workspace scan."],
  });
}

export async function docsContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const maxFiles = clamp(numberArg(args, "maxFiles", 12), 1, 40);
  const entries = await luxCommands.fsListFiles(clamp(input.preferences.maxIndexedFiles, 500, 20_000));
  const workspaceRoot = input.workspace?.root ?? "";
  const queryTokens = tokenizeRelatedQuery(query);
  const candidates = entries
    .filter((entry) => entry.kind === "file" && isDocsContextPath(entry.path, workspaceRoot))
    .map((entry) => createRelatedFileDescriptor(entry, workspaceRoot))
    .sort((left, right) => scoreDocsFile(right, queryTokens) - scoreDocsFile(left, queryTokens) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, maxFiles);
  const files = await readContextFiles(candidates, 12_000);
  return toolJson("DocsContext", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    dependencies: files
      .filter((file) => /(^|\/)(package\.json|cargo\.toml|pyproject\.toml|go\.mod|pom\.xml|build\.gradle)$/.test(file.relativePath.toLowerCase()))
      .map((file) => summarizeManifest(file.relativePath, file.text)),
    count: files.length,
    files,
  });
}

export async function memoryContext(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const query = stringArg(args, "query", input.message);
  const maxFiles = clamp(numberArg(args, "maxFiles", 14), 1, 40);
  const maxSignals = clamp(numberArg(args, "maxSignals", 40), 1, 120);
  const includeRecentChat = booleanArg(args, "includeRecentChat", true);
  const entries = await luxCommands.fsListFiles(clamp(input.preferences.maxIndexedFiles, 500, 20_000));
  const workspaceRoot = input.workspace?.root ?? "";
  const queryTokens = tokenizeRelatedQuery(query);
  const candidates = entries
    .filter((entry) => entry.kind === "file" && isMemoryContextPath(entry.path, workspaceRoot))
    .map((entry) => createRelatedFileDescriptor(entry, workspaceRoot))
    .sort((left, right) => scoreMemoryFile(right, queryTokens) - scoreMemoryFile(left, queryTokens) || left.relativeLower.localeCompare(right.relativeLower))
    .slice(0, maxFiles);
  const files = await readContextFiles(candidates, 14_000);
  const fileSignals = files.flatMap((file) => extractMemorySignals(file, queryTokens));
  const chatSignals = includeRecentChat ? extractChatMemorySignals(input, queryTokens) : [];
  const runtimeSignals = buildRuntimeMemorySignals(input, queryTokens);
  const signals = rankMemorySignals([...runtimeSignals, ...chatSignals, ...fileSignals], queryTokens).slice(0, maxSignals);

  return toolJson("MemoryContext", {
    workspaceRoot: input.workspace?.root ?? null,
    query,
    filesScanned: files.length,
    signalsReturned: signals.length,
    runtime: {
      provider: input.provider.name,
      protocol: input.provider.protocol,
      baseUrl: input.provider.baseUrl,
      model: input.selectedModel.alias || input.selectedModel.id,
      reasoningEffort: input.preferences.selectedEffortId,
      agent: input.selectedAgentName || input.preferences.agentMode,
      toolApprovalMode: input.preferences.toolApprovalMode,
      indexing: {
        enabled: input.preferences.projectIndexingEnabled,
        realtime: input.preferences.realtimeIndexing,
        maxIndexedFiles: input.preferences.maxIndexedFiles,
      },
    },
    files: files.map((file) => ({
      path: file.path,
      relativePath: file.relativePath,
      size: file.size,
      truncated: file.truncated,
      error: file.error,
      signalCount: fileSignals.filter((signal) => signal.source === file.relativePath).length,
    })),
    signals: signals.map(({ score: _score, ...signal }) => signal),
    notes: [
      "MemoryContext is read-only; it does not persist new memories.",
      files.length > 0 ? "Use high-signal decisions and preferences before changing code or tool behavior." : "No dedicated local memory files were found; runtime preferences and recent chat were used instead.",
    ],
  });
}

async function readContextFiles(files: RelatedFileDescriptor[], maxBytes: number): Promise<ContextFile[]> {
  const settled = await Promise.allSettled(files.map(async (file) => {
    const response = await luxCommands.fsReadText(file.path, maxBytes);
    return {
      path: response.path,
      relativePath: file.relativePath,
      size: response.size,
      truncated: response.truncated,
      text: truncateText(response.text, Math.min(maxBytes, 12_000)),
    } satisfies ContextFile;
  }));
  return settled.map((result, index): ContextFile => {
    if (result.status === "fulfilled") return result.value;
    return { path: files[index].path, relativePath: files[index].relativePath, size: files[index].entry?.size ?? null, truncated: false, error: readErrorMessage(result.reason), text: "" };
  });
}

function summarizeManifest(relativePath: string, text: string) {
  const lower = relativePath.toLowerCase();
  if (lower.endsWith("package.json")) return summarizePackageJson(relativePath, text);
  if (lower.endsWith("cargo.toml")) return summarizeCargoToml(relativePath, text);
  return { path: relativePath, kind: "manifest", summary: truncateText(text, 1200) };
}

function summarizePackageJson(relativePath: string, text: string) {
  try {
    const parsed = JSON.parse(text) as unknown;
    if (!isRecord(parsed)) throw new Error("package.json is not an object");
    return {
      path: relativePath,
      kind: "package.json",
      name: typeof parsed.name === "string" ? parsed.name : null,
      version: typeof parsed.version === "string" ? parsed.version : null,
      scripts: isRecord(parsed.scripts) ? Object.keys(parsed.scripts).slice(0, 20) : [],
      dependencies: packageDependencySummary(parsed),
    };
  } catch (error) {
    return { path: relativePath, kind: "package.json", error: readErrorMessage(error), summary: truncateText(text, 1200) };
  }
}

function packageDependencySummary(parsed: UnknownRecord) {
  const result: Array<{ name: string; version: string; scope: string }> = [];
  for (const scope of ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"]) {
    const dependencies = parsed[scope];
    if (!isRecord(dependencies)) continue;
    for (const [name, version] of Object.entries(dependencies).slice(0, 40)) {
      result.push({ name, version: String(version), scope });
    }
  }
  return result.slice(0, 80);
}

function summarizeCargoToml(relativePath: string, text: string) {
  const packageName = text.match(/^name\s*=\s*"([^"]+)"/m)?.[1] ?? null;
  const version = text.match(/^version\s*=\s*"([^"]+)"/m)?.[1] ?? null;
  const reservedKeys = new Set(["name", "version", "edition", "license", "authors"]);
  const isDependencyTable = (section: string) =>
    /^(?:dependencies|dev-dependencies|build-dependencies)$/.test(section) ||
    /^target\..+\.(?:dependencies|dev-dependencies|build-dependencies)$/.test(section) ||
    /^workspace\.dependencies$/.test(section);
  const dependencySubTable = (section: string): string | null =>
    /^(?:dependencies|dev-dependencies|build-dependencies)\.(.+)$/.exec(section)?.[1] ??
    /^target\..+\.(?:dependencies|dev-dependencies|build-dependencies)\.(.+)$/.exec(section)?.[1] ??
    /^workspace\.dependencies\.(.+)$/.exec(section)?.[1] ??
    null;
  const dependencies: Array<{ name: string; spec: string }> = [];
  const seen = new Set<string>();
  const addDependency = (name: string, spec: string) => {
    if (seen.has(name) || dependencies.length >= 80) return;
    seen.add(name);
    dependencies.push({ name, spec: truncateText(spec.trim(), 180) });
  };
  let section = "";
  for (const rawLine of text.split(/\r?\n/)) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;
    const header = /^\[\[?\s*([^\]]+?)\s*\]\]?/.exec(line);
    if (header) {
      section = header[1].trim();
      const sub = dependencySubTable(section);
      if (sub) addDependency(sub, "(table)");
      continue;
    }
    // Inside [dependencies.foo] the key=value lines are foo's attributes (version/features/path/...),
    // not new dependencies, so skip them. Only flat dependency tables emit deps directly.
    if (dependencySubTable(section) || !isDependencyTable(section)) continue;
    const kv = /^([A-Za-z0-9_-]+)\s*=\s*(.+)$/.exec(line);
    if (!kv || reservedKeys.has(kv[1])) continue;
    addDependency(kv[1], kv[2]);
  }
  return { path: relativePath, kind: "Cargo.toml", name: packageName, version, dependencies };
}

function extractMemorySignals(file: ContextFile, queryTokens: string[]): MemorySignal[] {
  if (file.error || !file.text.trim()) return [];
  const signals: MemorySignal[] = [];
  const lines = file.text.split(/\r?\n/);
  const windowLines = 1;
  for (let index = 0; index < lines.length; index += 1) {
    const rawLine = lines[index];
    const line = rawLine.trim();
    if (!line || line.length < 4) continue;
    const kind = classifyMemoryLine(line);
    if (!kind) continue;
    const contextStart = Math.max(0, index - windowLines);
    const contextEnd = Math.min(lines.length, index + windowLines + 1);
    const context = lines.slice(contextStart, contextEnd).map((candidate) => candidate.trim()).filter(Boolean).join("\n");
    signals.push({
      source: file.relativePath,
      line: index + 1,
      kind,
      score: scoreMemorySignal(file.relativePath, context || line, kind, queryTokens),
      text: truncateText(context || line, 700),
    });
  }
  return signals;
}

function extractChatMemorySignals(input: AiChatSendInput, queryTokens: string[]): MemorySignal[] {
  const recent = input.history.slice(-10);
  const signals: MemorySignal[] = [];
  for (const [index, message] of recent.entries()) {
    const content = message.content.trim();
    if (!content) continue;
    const kind = classifyChatMemory(content, message.role);
    if (!kind) continue;
    signals.push({
      source: `chat:${message.role}:${index + 1}`,
      line: 1,
      kind,
      score: scoreMemorySignal(`chat:${message.role}`, content, kind, queryTokens) + (message.role === "user" ? 16 : 6),
      text: truncateText(content, 900),
    });
  }

  const current = input.message.trim();
  if (current) {
    signals.push({
      source: "chat:current-user-message",
      line: 1,
      kind: "planning",
      score: scoreMemorySignal("chat:current-user-message", current, "planning", queryTokens) + 24,
      text: truncateText(current, 900),
    });
  }
  return signals;
}

function buildRuntimeMemorySignals(input: AiChatSendInput, queryTokens: string[]): MemorySignal[] {
  const model = input.selectedModel.alias || input.selectedModel.id;
  const approval = input.preferences.toolApprovalMode === "full-access"
    ? "Full Access: dangerous tools auto-approve, while workspace guards still apply."
    : "Default: dangerous tools require explicit approval.";
  const values = [
    `AI provider ${input.provider.name} (${input.provider.protocol}) base URL: ${input.provider.baseUrl}.`,
    `Selected model: ${model}; reasoning effort: ${input.preferences.selectedEffortId}.`,
    `Agent mode: ${input.selectedAgentName || input.preferences.agentMode}.`,
    approval,
    `Workspace indexing: enabled=${input.preferences.projectIndexingEnabled}, realtime=${input.preferences.realtimeIndexing}, maxIndexedFiles=${input.preferences.maxIndexedFiles}.`,
  ];
  return values.map((text, index) => ({
    source: "runtime-preferences",
    line: index + 1,
    kind: "runtime" as const,
    score: scoreMemorySignal("runtime-preferences", text, "runtime", queryTokens) + 30,
    text,
  }));
}

function classifyMemoryLine(line: string): MemorySignal["kind"] | null {
  const normalized = line.toLowerCase();
  if (/^#{1,6}\s+/.test(line)) {
    return /decision|preference|todo|roadmap|memory|rule|architecture|adr/.test(normalized) ? "heading" : null;
  }
  if (/\b(adr|decision|decided|chosen|choose|prefer|preference|convention|rule|policy|must|should|required|default|full access|approval mode)\b/i.test(line)) {
    return /prefer|preference|default|mode|setting|style|convention/i.test(line) ? "preference" : "decision";
  }
  if (/\b(todo|fixme|roadmap|next|planned|follow[- ]?up|remaining|blocked|in progress)\b/i.test(line)) return "planning";
  if (/^[-*]\s+\[[ xX-]\]/.test(line)) return "planning";
  return null;
}

function classifyChatMemory(content: string, role: AiChatMessage["role"]): MemorySignal["kind"] | null {
  if (role === "user") return /\b(need|нужно|сделай|добавь|не забудь|default|full access|proxy|model|reasoning|tools?)\b/i.test(content) ? "preference" : null;
  return /\b(done|implemented|changed|verified|remaining|blocked|todo|next)\b/i.test(content) ? "planning" : null;
}

function scoreMemorySignal(source: string, text: string, kind: MemorySignal["kind"], queryTokens: string[]) {
  const lower = `${source}\n${text}`.toLowerCase();
  let score = kind === "runtime" ? 70 : kind === "decision" ? 64 : kind === "preference" ? 60 : kind === "planning" ? 48 : 34;
  if (/full access|approval|proxy|model|reasoning|tool|test|production|prod/i.test(text)) score += 18;
  if (/\.codex|\.cursor|agents\.md|memory|decision|adr|roadmap|preference/i.test(source)) score += 14;
  for (const token of queryTokens) {
    if (lower.includes(token)) score += token.length >= 6 ? 22 : 12;
  }
  return score;
}

function rankMemorySignals(signals: MemorySignal[], queryTokens: string[]) {
  const seen = new Set<string>();
  return signals
    .map((signal) => ({ ...signal, score: signal.score + scoreMemorySignal(signal.source, signal.text, signal.kind, queryTokens) / 10 }))
    .filter((signal) => {
      const key = `${signal.source}:${signal.line}:${signal.text.slice(0, 120)}`.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => right.score - left.score || left.source.localeCompare(right.source) || left.line - right.line);
}
