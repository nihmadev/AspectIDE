import { deriveSegmentToolCalls } from "./aiChatTypes";
import type { AiChatMessage, AiChatToolCall } from "./aiChatTypes";
import { extractReviewPathFromToolInput } from "./aiFileReviewBridge";

const fileEditTools = new Set(["Write", "StrReplace", "PatchEngine", "Delete"]);

export type TurnFileChangeEntry = {
  path: string;
  displayPath: string;
  linesAdded: number;
  linesRemoved: number;
  filesCreated: number;
  filesDeleted: number;
};

export type TurnFileSummary = {
  files: TurnFileChangeEntry[];
  totalLinesAdded: number;
  totalLinesRemoved: number;
  fileEditToolCount: number;
};

export function buildTurnFileSummary(
  message: AiChatMessage,
  workspaceRoot: string | null,
): TurnFileSummary | null {
  const toolCalls = message.segments?.length
    ? deriveSegmentToolCalls(message.segments)
    : message.toolCalls ?? [];
  const fileTools = toolCalls.filter((call) => fileEditTools.has(call.tool) && call.status === "success");
  if (fileTools.length === 0) return null;

  const byPath = new Map<string, TurnFileChangeEntry>();

  for (const call of fileTools) {
    ingestToolCall(byPath, call, workspaceRoot);
  }

  const files = [...byPath.values()]
    .filter((entry) => entry.linesAdded > 0 || entry.linesRemoved > 0 || entry.filesCreated > 0 || entry.filesDeleted > 0)
    .sort((left, right) => (right.linesAdded + right.linesRemoved) - (left.linesAdded + left.linesRemoved));

  if (files.length === 0) return null;

  return {
    files,
    totalLinesAdded: files.reduce((sum, file) => sum + file.linesAdded, 0),
    totalLinesRemoved: files.reduce((sum, file) => sum + file.linesRemoved, 0),
    fileEditToolCount: fileTools.length,
  };
}

function ingestToolCall(
  byPath: Map<string, TurnFileChangeEntry>,
  call: AiChatToolCall,
  workspaceRoot: string | null,
) {
  const parsed = parseToolOutput(call.output);
  const paths = parsed.paths.length > 0
    ? parsed.paths
    : [extractReviewPathFromToolInput(call.tool, call.input)].filter((path): path is string => Boolean(path));

  for (const path of paths) {
    const key = normalizePathKey(path);
    const existing = byPath.get(key) ?? {
      path,
      displayPath: toDisplayPath(path, workspaceRoot),
      linesAdded: 0,
      linesRemoved: 0,
      filesCreated: 0,
      filesDeleted: 0,
    };
    existing.linesAdded += parsed.stats?.linesAdded ?? call.stats?.linesAdded ?? 0;
    existing.linesRemoved += parsed.stats?.linesRemoved ?? call.stats?.linesRemoved ?? 0;
    existing.filesCreated += parsed.stats?.filesCreated ?? call.stats?.filesCreated ?? 0;
    existing.filesDeleted += parsed.stats?.filesDeleted ?? call.stats?.filesDeleted ?? 0;
    byPath.set(key, existing);
  }
}

function parseToolOutput(output?: string) {
  const empty = { paths: [] as string[], stats: null as TurnFileChangeEntry | null };
  if (!output?.trim()) return empty;
  try {
    const parsed = JSON.parse(output) as Record<string, unknown>;
    const paths: string[] = [];
    if (typeof parsed.path === "string") paths.push(parsed.path);
    if (Array.isArray(parsed.changedPaths)) {
      for (const entry of parsed.changedPaths) {
        if (typeof entry === "string") paths.push(entry);
      }
    }
    const rawStats = parsed.stats;
    const stats = isRecord(rawStats) ? {
      path: "",
      displayPath: "",
      linesAdded: readNumber(rawStats.linesAdded),
      linesRemoved: readNumber(rawStats.linesRemoved),
      filesCreated: readNumber(rawStats.filesCreated),
      filesDeleted: readNumber(rawStats.filesDeleted),
    } : null;
    return { paths, stats };
  } catch {
    return empty;
  }
}

function toDisplayPath(path: string, workspaceRoot: string | null) {
  if (!workspaceRoot) return path;
  const root = workspaceRoot.replace(/\\/g, "/").replace(/\/+$/, "");
  const normalized = path.replace(/\\/g, "/");
  if (normalized.toLowerCase().startsWith(root.toLowerCase() + "/")) {
    return normalized.slice(root.length + 1);
  }
  return path;
}

function normalizePathKey(path: string) {
  return path.replace(/\\/g, "/").toLowerCase();
}

function readNumber(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? Math.max(0, Math.round(value)) : 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}