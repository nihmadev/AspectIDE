import { decodeChatDisplayText } from "./aiChatDisplayText";
import type { AiChatMessage, AiChatToolCall } from "./aiChatTypes";

const PATH_VERIFICATION_TOOLS = new Set([
  "Read",
  "Glob",
  "Grep",
  "InspectFile",
  "RepoMap",
  "SemanticSearch",
  "RelatedFiles",
  "SymbolContext",
  "FastContext",
  "WorkspaceIndex",
  "ActiveContext",
  "DiagnosticsContext",
  "GitContext",
  "ReviewDiff",
  "ImpactAnalysis",
  "TestHealth",
  "FailureAnalyzer",
]);

const filePathPattern = /(?:^|[\s"'`(])([\w./\\-]+\.(?:ts|tsx|js|jsx|rs|py|go|java|css|html|md|json|yaml|yml|toml|sql|cs|cpp|h|vue|svelte|db|csv|zip|tar|gz|pdf|proto))(?:$|[\s"'`,.)])/gi;
// Require at least two "/"-terminated segments: a real directory path has an
// internal separator ("src/components/"), while bare tokens ("read/", "shell/")
// and prose alternations ("read/write", "edit/apply") do not — those were being
// flagged as "unconfirmed paths" purely because a tool-name preceded a slash.
const dirPathPattern = /(?:^|[\s*`#>\-])([a-zA-Z][\w.-]*\/(?:[\w.-]+\/)+)/g;

function normalizeEvidencePath(path: string) {
  return path.replace(/\\/g, "/").replace(/^\.\//, "").toLowerCase();
}

function extractFilePaths(text: string) {
  const paths: string[] = [];
  filePathPattern.lastIndex = 0;
  for (const match of text.matchAll(filePathPattern)) {
    const path = match[1]?.replace(/\\/g, "/");
    if (path && path.length <= 180) paths.push(path);
  }
  return paths;
}

function extractDirPaths(text: string) {
  const paths: string[] = [];
  dirPathPattern.lastIndex = 0;
  for (const match of text.matchAll(dirPathPattern)) {
    const path = match[1]?.replace(/\\/g, "/");
    // Belt-and-suspenders vs future regex edits: a candidate must keep an
    // internal separator after trimming the trailing slash.
    if (path && path.length <= 120 && path.replace(/\/+$/, "").includes("/")) paths.push(path);
  }
  return paths;
}

function collectPathsFromToolText(text: string, verified: Set<string>) {
  for (const path of extractFilePaths(text)) verified.add(normalizeEvidencePath(path));
  for (const path of extractDirPaths(text)) verified.add(normalizeEvidencePath(path));
}

function collectVerifiedPathsFromToolCall(call: AiChatToolCall, verified: Set<string>) {
  if (!PATH_VERIFICATION_TOOLS.has(call.tool)) return;
  for (const text of [call.input, call.output, call.error]) {
    if (text) collectPathsFromToolText(text, verified);
  }
}

function isPathVerifiedByTools(path: string, verified: Set<string>) {
  const normalized = normalizeEvidencePath(path);
  if (verified.has(normalized)) return true;
  for (const entry of verified) {
    if (entry.startsWith(normalized) || normalized.startsWith(entry)) return true;
  }
  return false;
}

export function listUnverifiedPathsInAssistantMessage(message: AiChatMessage) {
  const verified = new Set<string>();
  for (const call of message.toolCalls ?? []) collectVerifiedPathsFromToolCall(call, verified);
  for (const segment of message.segments ?? []) {
    if (segment.kind === "tool") collectVerifiedPathsFromToolCall(segment.toolCall, verified);
  }

  const prose = decodeChatDisplayText(message.content);
  const cited = [...new Set([...extractFilePaths(prose), ...extractDirPaths(prose)])];
  return cited.filter((path) => !isPathVerifiedByTools(path, verified));
}

export function shouldShowPathEvidenceNotice(message: AiChatMessage, streaming: boolean) {
  if (streaming || message.role !== "assistant") return false;
  if (!(message.toolCalls?.length ?? 0) && !(message.segments?.some((segment) => segment.kind === "tool"))) {
    return false;
  }
  const unverified = listUnverifiedPathsInAssistantMessage(message);
  if (unverified.length === 0) return false;
  if (unverified.length >= 2) return true;
  return unverified.some((path) => path.endsWith("/"));
}