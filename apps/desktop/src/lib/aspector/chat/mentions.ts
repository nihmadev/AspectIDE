import { displayPath } from "./../../explorer/file-tree";
import { luxCommands } from "./../../tauri/commands";
import type { DocumentSnapshot, FsEntry, LspWorkspaceSymbol } from "./../../types/index";

export type AiMentionKind = "file" | "folder" | "symbol" | "codebase" | "docs";

export type AiMentionCandidate = {
  kind: AiMentionKind;
  id: string;
  label: string;
  detail: string;
  path?: string;
  symbolName?: string;
  line?: number;
  column?: number;
  score: number;
};

export type ParsedMentionQuery = {
  triggerIndex: number;
  query: string;
  kindFilter: AiMentionKind | null;
};

const mentionKindPrefixes: Record<string, AiMentionKind> = {
  file: "file",
  files: "file",
  folder: "folder",
  folders: "folder",
  dir: "folder",
  symbol: "symbol",
  symbols: "symbol",
  codebase: "codebase",
  code: "codebase",
  docs: "docs",
  doc: "docs",
  rules: "docs",
};

const staticMentions: AiMentionCandidate[] = [
  { kind: "codebase", id: "mention:codebase", label: "@codebase", detail: "Semantic workspace search", score: 200 },
  { kind: "docs", id: "mention:docs", label: "@docs", detail: "Project rules and documentation", score: 190 },
];

export function parseMentionQuery(message: string, caretIndex = message.length): ParsedMentionQuery | null {
  const before = message.slice(0, caretIndex);
  const at = before.lastIndexOf("@");
  if (at < 0) return null;
  const prefix = before.slice(at + 1);
  if (prefix.includes("\n") || prefix.includes(" ")) return null;
  const colon = prefix.indexOf(":");
  const kindFilter = colon >= 0 ? mentionKindPrefixes[prefix.slice(0, colon).toLowerCase()] ?? null : null;
  const query = colon >= 0 ? prefix.slice(colon + 1) : prefix;
  return { triggerIndex: at, query, kindFilter };
}

export function mentionMenuVisible(message: string, caretIndex = message.length) {
  const parsed = parseMentionQuery(message, caretIndex);
  return Boolean(parsed && !message.includes("\n", parsed.triggerIndex));
}

export async function searchMentionCandidates(input: {
  query: string;
  kindFilter: AiMentionKind | null;
  workspaceRoot: string | null;
  openDocuments: DocumentSnapshot[];
  fileEntries: FsEntry[];
  limit?: number;
}): Promise<AiMentionCandidate[]> {
  const limit = input.limit ?? 12;
  const normalized = input.query.trim().toLowerCase();
  const matches: AiMentionCandidate[] = [];

  for (const entry of staticMentions) {
    if (input.kindFilter && input.kindFilter !== entry.kind) continue;
    if (!normalized || entry.label.toLowerCase().includes(normalized) || entry.kind.includes(normalized)) {
      matches.push({ ...entry, score: entry.score + (normalized ? 20 : 0) });
    }
  }

  if (!input.kindFilter || input.kindFilter === "file") {
    matches.push(...searchFileMentions(normalized, input.openDocuments, input.fileEntries, input.workspaceRoot));
  }
  if ((!input.kindFilter || input.kindFilter === "folder") && input.workspaceRoot) {
    matches.push(...searchFolderMentions(normalized, input.fileEntries, input.workspaceRoot));
  }
  if ((!input.kindFilter || input.kindFilter === "symbol") && input.workspaceRoot && normalized.length >= 1) {
    matches.push(...await searchSymbolMentions(normalized));
  }

  return matches
    .sort((left, right) => right.score - left.score || left.label.localeCompare(right.label))
    .slice(0, limit);
}

export function applyMentionSelection(message: string, parsed: ParsedMentionQuery) {
  const before = message.slice(0, parsed.triggerIndex);
  const tail = message.slice(parsed.triggerIndex);
  const consumed = tail.match(/^@[^\s]*/)?.[0]?.length ?? 1;
  const after = message.slice(parsed.triggerIndex + consumed);
  const lead = before.trimEnd();
  return `${lead}${lead.length > 0 && after.length > 0 ? " " : ""}${after.trimStart()}`.trimStart();
}

function searchFileMentions(
  query: string,
  openDocuments: DocumentSnapshot[],
  fileEntries: FsEntry[],
  workspaceRoot: string | null,
) {
  const results: AiMentionCandidate[] = [];
  const seen = new Set<string>();

  for (const document of openDocuments) {
    if (!document.path) continue;
    const label = displayPath(document.path);
    const score = scoreText(label, query) + 40;
    if (score <= 0 && query) continue;
    const key = document.path.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    results.push({
      kind: "file",
      id: `file:${document.path}`,
      label: label.split(/[/\\]/).pop() ?? label,
      detail: label,
      path: document.path,
      score,
    });
  }

  for (const entry of fileEntries) {
    if (entry.kind !== "file") continue;
    const label = displayPath(entry.path);
    const score = scoreText(label, query);
    if (score <= 0 && query) continue;
    const key = entry.path.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    results.push({
      kind: "file",
      id: `file:${entry.path}`,
      label: label.split(/[/\\]/).pop() ?? label,
      detail: label,
      path: entry.path,
      score,
    });
    if (results.length >= 24) break;
  }

  return results;
}

function searchFolderMentions(query: string, fileEntries: FsEntry[], workspaceRoot: string) {
  const results: AiMentionCandidate[] = [];
  const seen = new Set<string>();
  for (const entry of fileEntries) {
    if (entry.kind !== "directory") continue;
    const label = displayPath(entry.path);
    const score = scoreText(label, query);
    if (score <= 0 && query) continue;
    const key = entry.path.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    results.push({
      kind: "folder",
      id: `folder:${entry.path}`,
      label: label.split(/[/\\]/).pop() ?? label,
      detail: label,
      path: entry.path,
      score,
    });
    if (results.length >= 16) break;
  }
  return results;
}

async function searchSymbolMentions(query: string): Promise<AiMentionCandidate[]> {
  try {
    const symbols = await luxCommands.lspWorkspaceSymbols(query);
    return symbols.slice(0, 16).map((symbol, index) => workspaceSymbolToCandidate(symbol, query, index));
  } catch {
    return [];
  }
}

function workspaceSymbolToCandidate(symbol: LspWorkspaceSymbol, query: string, index: number): AiMentionCandidate {
  const path = symbol.location.path;
  const line = symbol.location.range.start_line + 1;
  const column = symbol.location.range.start_column + 1;
  const label = symbol.name;
  const detail = `${path.split(/[/\\]/).pop() ?? path}:${line}`;
  return {
    kind: "symbol",
    id: `symbol:${path}:${line}:${column}:${symbol.name}`,
    label,
    detail,
    path,
    symbolName: symbol.name,
    line,
    column,
    score: scoreText(`${symbol.name} ${path}`, query) + Math.max(0, 30 - index),
  };
}

function scoreText(value: string, query: string) {
  const haystack = value.toLowerCase();
  const needle = query.toLowerCase();
  if (!needle) return 10;
  if (haystack === needle) return 120;
  if (haystack.startsWith(needle)) return 90;
  if (haystack.includes(needle)) return 60;
  return 0;
}