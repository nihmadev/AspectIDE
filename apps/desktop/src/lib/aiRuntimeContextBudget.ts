import type { AiChatSendInput } from "./aiChatTypes";
import { countLines } from "./aiRuntimePatch";
import { clamp, isRecord, readErrorMessage, truncateText, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { compactTerminalSession, terminalSessionsForContext } from "./aiRuntimeTerminal";

export type ContextBudgetItem = {
  id: string;
  kind: string;
  source: string;
  score: number;
  reason: string;
  content: string;
  path?: string;
  line?: number;
};

export function addDirectContextBudgetItems(items: ContextBudgetItem[], input: AiChatSendInput, query: string, queryTokens: string[], includeActiveText: boolean, includeOpenDocuments: boolean) {
  items.push({
    id: "intent:user-message",
    kind: "intent",
    source: "current-user-message",
    score: 130,
    reason: "The current user request defines the task.",
    content: truncateText(input.message.trim() || query, 1_800),
  });

  const recentUser = [...input.history].reverse().find((message) => message.role === "user" && message.content.trim());
  if (recentUser) {
    items.push({
      id: "intent:recent-user-message",
      kind: "intent",
      source: "recent-user-message",
      score: 88,
      reason: "Recent user instruction may constrain the current task.",
      content: truncateText(recentUser.content, 1_200),
    });
  }

  items.push({
    id: "runtime:ai-settings",
    kind: "runtime",
    source: "ai-runtime-preferences",
    score: 92,
    reason: "Model, provider, reasoning, and tool approval mode affect AI behavior.",
    content: [
      `provider=${input.provider.name}`,
      `protocol=${input.provider.protocol}`,
      `baseUrl=${input.provider.baseUrl}`,
      `model=${input.selectedModel.alias || input.selectedModel.id}`,
      `reasoning=${input.preferences.selectedEffortId}`,
      `agent=${input.selectedAgentName || input.preferences.agentMode}`,
      `toolApprovalMode=${input.preferences.toolApprovalMode}`,
    ].join("\n"),
  });

  if (input.activeDocument) {
    const path = input.activeDocument.path ?? input.activeDocument.title;
    items.push({
      id: `active:${input.activeDocument.id}:metadata`,
      kind: "active-document",
      source: path,
      path,
      score: 112 + scoreContextTokens(`${path}\n${input.activeDocument.language_id}`, queryTokens),
      reason: "The active editor is the strongest local signal for the user's current focus.",
      content: [
        `path=${path}`,
        `language=${input.activeDocument.language_id}`,
        `dirty=${input.activeDocument.is_dirty}`,
        `lines=${countLines(input.activeDocument.text)}`,
      ].join("\n"),
    });
    if (includeActiveText && input.activeDocument.text.trim()) {
      items.push({
        id: `active:${input.activeDocument.id}:excerpt`,
        kind: "file-excerpt",
        source: path,
        path,
        score: 104 + scoreContextTokens(`${path}\n${input.activeDocument.text.slice(0, 3_000)}`, queryTokens),
        reason: "Active document excerpt provides immediate code context.",
        content: truncateContextAroundTokens(input.activeDocument.text, queryTokens, 2_800),
      });
    }
  }

  if (includeOpenDocuments) {
    for (const document of input.openDocuments.slice(0, 24)) {
      if (input.activeDocument?.id === document.id) continue;
      const path = document.path ?? document.title;
      const score = (document.is_dirty ? 88 : 58) + scoreContextTokens(`${path}\n${document.text.slice(0, 2_000)}`, queryTokens);
      items.push({
        id: `open:${document.id}`,
        kind: document.is_dirty ? "dirty-document" : "open-document",
        source: path,
        path,
        score,
        reason: document.is_dirty ? "Dirty open file may contain unsaved user work." : "Open editor tab may be relevant to the task.",
        content: [
          `path=${path}`,
          `language=${document.language_id}`,
          `dirty=${document.is_dirty}`,
          truncateContextAroundTokens(document.text, queryTokens, document.is_dirty ? 1_800 : 900),
        ].join("\n"),
      });
    }
  }

  for (const attachment of input.attachments.slice(0, 12)) {
    items.push({
      id: `attachment:${attachment.name}`,
      kind: "attachment",
      source: attachment.name,
      score: 82 + scoreContextTokens(`${attachment.name}\n${attachment.text.slice(0, 2_000)}`, queryTokens),
      reason: "User-attached files are explicit task context.",
      content: truncateContextAroundTokens(attachment.text, queryTokens, 1_800),
    });
  }

  for (const session of terminalSessionsForContext(input).slice(0, 4)) {
    const compact = compactTerminalSession(session, input, 2_400);
    const terminalText = `${compact.shellName}\n${compact.cwd}\n${compact.output.tail}`;
    items.push({
      id: `terminal:${session.id}`,
      kind: "terminal",
      source: `terminal:${compact.shortId}`,
      score: (compact.active ? 86 : 54) + scoreContextTokens(terminalText, queryTokens),
      reason: compact.active ? "Active integrated terminal may contain live command state." : "Open integrated terminal may contain relevant command output.",
      content: truncateText([
        `id=${session.id}`,
        `active=${compact.active}`,
        `shell=${compact.shellName}`,
        `cwd=${compact.cwd}`,
        `updatedAt=${compact.output.updatedAt ?? "never"}`,
        compact.output.tail,
      ].join("\n"), 3_000),
    });
  }
}

export function addToolContextBudgetItems(items: ContextBudgetItem[], toolResults: PromiseSettledResult<ToolResult>[], queryTokens: string[]) {
  const [memory, rules, docs, related, semantic, diagnostics, git] = toolResults.map(parseToolContent);
  addMemoryBudgetItems(items, memory, queryTokens);
  addContextFilesBudgetItems(items, rules, "rule", "Project rule file constrains code/tool behavior.", queryTokens);
  addContextFilesBudgetItems(items, docs, "doc", "Local documentation or manifest grounds framework/API assumptions.", queryTokens);
  addRelatedBudgetItems(items, related, queryTokens);
  addSemanticBudgetItems(items, semantic, queryTokens);
  addDiagnosticsBudgetItems(items, diagnostics, queryTokens);
  addGitBudgetItems(items, git, queryTokens);
}

export function rankContextBudgetItems(items: ContextBudgetItem[], queryTokens: string[]) {
  const seen = new Set<string>();
  return items
    .map((item) => ({ ...item, score: item.score + scoreContextTokens(`${item.source}\n${item.content}`, queryTokens) }))
    .filter((item) => {
      const key = `${item.kind}:${item.source}:${item.line ?? 0}:${item.content.slice(0, 120)}`.toLowerCase();
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .sort((left, right) => right.score - left.score || contextKindRank(right.kind) - contextKindRank(left.kind) || left.source.localeCompare(right.source));
}

export function selectContextBudgetItems(items: ContextBudgetItem[], targetChars: number, maxItems: number) {
  const selected: ContextBudgetItem[] = [];
  let remaining = targetChars;
  for (const item of items) {
    if (selected.length >= maxItems || remaining <= 0) break;
    const reserved = selected.length === 0 ? 600 : 240;
    const maxItemChars = clamp(Math.min(Math.max(600, remaining - reserved), item.kind === "file-excerpt" ? 2_800 : 1_800), 300, 3_200);
    const content = truncateText(item.content, maxItemChars);
    const overhead = item.source.length + item.kind.length + item.reason.length + 160;
    if (content.length + overhead > remaining && selected.length > 0) continue;
    selected.push({ ...item, content });
    remaining -= content.length + overhead;
  }
  return selected;
}

export function buildContextBudgeterNextActions(selected: ContextBudgetItem[]) {
  const kinds = new Set(selected.map((item) => item.kind));
  const actions = ["Use the packet in ranked order; earlier items are higher signal for this task."];
  if (kinds.has("diagnostic")) actions.push("Resolve or account for diagnostics before claiming the code is clean.");
  if (kinds.has("related-file") || kinds.has("semantic-hit")) actions.push("Read the highest-ranked related files before editing unfamiliar code.");
  if (kinds.has("terminal")) actions.push("Use TerminalContext before relying on live terminal state; use TerminalWrite only for intentional interactive input.");
  if (kinds.has("git")) actions.push("Preserve existing changed files and avoid overwriting unrelated work.");
  if (kinds.has("rule") || kinds.has("memory")) actions.push("Apply project rules and remembered preferences when choosing tools or implementation style.");
  return actions.slice(0, 6);
}

export function contextBudgeterUnavailable(toolResults: PromiseSettledResult<ToolResult>[]) {
  const names = ["MemoryContext", "RulesContext", "DocsContext", "RelatedFiles", "SemanticSearch", "DiagnosticsContext", "GitContext"];
  return toolResults
    .map((result, index) => result.status === "rejected" ? { tool: names[index] ?? `tool-${index + 1}`, error: readErrorMessage(result.reason) } : null)
    .filter((value): value is { tool: string; error: string } => Boolean(value));
}

export function parseToolContent(result: PromiseSettledResult<ToolResult>) {
  if (result.status !== "fulfilled") return { error: readErrorMessage(result.reason) };
  try {
    const parsed = JSON.parse(result.value.content) as unknown;
    return isRecord(parsed) ? parsed : { value: parsed };
  } catch {
    return { text: result.value.content };
  }
}

function addMemoryBudgetItems(items: ContextBudgetItem[], memory: unknown, queryTokens: string[]) {
  if (!isRecord(memory) || !Array.isArray(memory.signals)) return;
  for (const signal of memory.signals.filter(isRecord).slice(0, 24)) {
    const source = stringField(signal, "source", "memory");
    const text = stringField(signal, "text", "");
    if (!text.trim()) continue;
    items.push({
      id: `memory:${source}:${numberField(signal, "line", 0)}:${items.length}`,
      kind: "memory",
      source,
      line: numberField(signal, "line", 0) || undefined,
      score: 96 + scoreContextTokens(`${source}\n${text}`, queryTokens),
      reason: `Project memory signal: ${stringField(signal, "kind", "memory")}.`,
      content: truncateText(text, 900),
    });
  }
}

function addContextFilesBudgetItems(items: ContextBudgetItem[], value: unknown, kind: string, reason: string, queryTokens: string[]) {
  if (!isRecord(value) || !Array.isArray(value.files)) return;
  for (const file of value.files.filter(isRecord).slice(0, 12)) {
    const source = stringField(file, "relativePath", stringField(file, "path", kind));
    const text = stringField(file, "text", "");
    if (!text.trim()) continue;
    items.push({
      id: `${kind}:${source}`,
      kind,
      source,
      path: stringField(file, "path", source),
      score: (kind === "rule" ? 92 : 76) + scoreContextTokens(`${source}\n${text.slice(0, 2_000)}`, queryTokens),
      reason,
      content: truncateContextAroundTokens(text, queryTokens, kind === "rule" ? 1_200 : 1_000),
    });
  }
}

function addRelatedBudgetItems(items: ContextBudgetItem[], related: unknown, queryTokens: string[]) {
  if (!isRecord(related) || !Array.isArray(related.files)) return;
  for (const file of related.files.filter(isRecord).slice(0, 24)) {
    const source = stringField(file, "relativePath", stringField(file, "path", "related-file"));
    const relations = Array.isArray(file.relations) ? file.relations.filter((relation): relation is string => typeof relation === "string") : [];
    items.push({
      id: `related:${source}`,
      kind: "related-file",
      source,
      path: stringField(file, "path", source),
      score: 62 + numberField(file, "score", 0) / 2 + scoreContextTokens(`${source}\n${relations.join(" ")}`, queryTokens),
      reason: relations.length > 0 ? `Related by ${relations.join(", ")}.` : "Related file candidate from project structure.",
      content: [`path=${source}`, `relations=${relations.join(", ") || "none"}`, `size=${numberField(file, "size", 0) || "unknown"}`].join("\n"),
    });
  }
}

function addSemanticBudgetItems(items: ContextBudgetItem[], semantic: unknown, queryTokens: string[]) {
  if (!isRecord(semantic) || !Array.isArray(semantic.results)) return;
  for (const result of semantic.results.filter(isRecord).slice(0, 24)) {
    const source = stringField(result, "relativePath", stringField(result, "path", "semantic-result"));
    const preview = stringField(result, "preview", stringField(result, "name", source));
    items.push({
      id: `semantic:${stringField(result, "type", "result")}:${source}:${numberField(result, "line", 0)}`,
      kind: "semantic-hit",
      source,
      path: stringField(result, "path", source),
      line: numberField(result, "line", 0) || undefined,
      score: 72 + numberField(result, "score", 0) / 4 + scoreContextTokens(`${source}\n${preview}`, queryTokens),
      reason: `SemanticSearch ${stringField(result, "type", "hit")} hit from ${stringField(result, "source", "workspace")}.`,
      content: truncateText([
        `path=${source}`,
        `line=${numberField(result, "line", 0) || "unknown"}`,
        `name=${stringField(result, "name", "")}`,
        `preview=${preview}`,
      ].filter(Boolean).join("\n"), 900),
    });
  }
}

function addDiagnosticsBudgetItems(items: ContextBudgetItem[], diagnostics: unknown, queryTokens: string[]) {
  if (!isRecord(diagnostics) || !Array.isArray(diagnostics.diagnostics)) return;
  for (const diagnostic of diagnostics.diagnostics.filter(isRecord).slice(0, 40)) {
    const path = stringField(diagnostic, "path", "diagnostic");
    const message = stringField(diagnostic, "message", "");
    const severity = stringField(diagnostic, "severity", "diagnostic");
    items.push({
      id: `diagnostic:${path}:${numberField(diagnostic, "line", 0)}:${items.length}`,
      kind: "diagnostic",
      source: path,
      path,
      line: numberField(diagnostic, "line", 0) || undefined,
      score: (severity === "error" ? 96 : 72) + scoreContextTokens(`${path}\n${message}`, queryTokens),
      reason: `${severity} diagnostic can affect correctness or validation.`,
      content: `${severity} ${path}:${numberField(diagnostic, "line", 0) || "?"}:${numberField(diagnostic, "column", 0) || "?"} ${message}`,
    });
  }
}

function addGitBudgetItems(items: ContextBudgetItem[], git: unknown, queryTokens: string[]) {
  if (!isRecord(git)) return;
  const changedFiles = Array.isArray(git.changedFiles) ? git.changedFiles.filter(isRecord).slice(0, 60) : [];
  if (changedFiles.length === 0 && !stringField(git, "branch", "")) return;
  const content = [
    `branch=${stringField(git, "branch", "unknown")}`,
    `ahead=${numberField(git, "ahead", 0)}`,
    `behind=${numberField(git, "behind", 0)}`,
    ...changedFiles.map((file) => `${stringField(file, "indexStatus", " ")}${stringField(file, "worktreeStatus", " ")} ${stringField(file, "path", "")}`),
  ].join("\n");
  items.push({
    id: "git:status",
    kind: "git",
    source: "git-status",
    score: 78 + changedFiles.length * 2 + scoreContextTokens(content, queryTokens),
    reason: "Git status identifies changed files and branch state that should not be overwritten accidentally.",
    content: truncateText(content, 1_600),
  });
}

function truncateContextAroundTokens(text: string, queryTokens: string[], maxChars: number) {
  if (text.length <= maxChars) return text;
  const lower = text.toLowerCase();
  const firstHit = queryTokens.map((token) => lower.indexOf(token)).filter((index) => index >= 0).sort((left, right) => left - right)[0];
  if (firstHit === undefined) return truncateText(text, maxChars);
  const start = Math.max(0, firstHit - Math.floor(maxChars * 0.35));
  const end = Math.min(text.length, start + maxChars);
  const prefix = start > 0 ? `...[truncated ${start} chars before]\n` : "";
  const suffix = end < text.length ? `\n...[truncated ${text.length - end} chars after]` : "";
  return `${prefix}${text.slice(start, end)}${suffix}`;
}

function scoreContextTokens(text: string, queryTokens: string[]) {
  if (queryTokens.length === 0) return 0;
  const lower = text.toLowerCase();
  let score = 0;
  for (const token of queryTokens) {
    if (lower.includes(token)) score += token.length >= 6 ? 18 : 10;
  }
  return score;
}

function contextKindRank(kind: string) {
  switch (kind) {
    case "intent":
      return 12;
    case "active-document":
    case "file-excerpt":
      return 11;
    case "diagnostic":
      return 10;
    case "memory":
    case "rule":
      return 9;
    case "semantic-hit":
    case "related-file":
      return 8;
    case "git":
      return 7;
    case "doc":
      return 6;
    default:
      return 0;
  }
}

function stringField(value: UnknownRecord, key: string, fallback = "") {
  const field = value[key];
  return typeof field === "string" ? field : fallback;
}

function numberField(value: UnknownRecord, key: string, fallback = 0) {
  const field = value[key];
  const numeric = typeof field === "number" ? field : Number(field);
  return Number.isFinite(numeric) ? numeric : fallback;
}
