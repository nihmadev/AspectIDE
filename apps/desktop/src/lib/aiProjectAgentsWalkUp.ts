import { joinPath, parentPath } from "./fileTree";
import { normalizePathSlashes } from "./aiRuntimeShared";

/** Directories from workspace root toward the active file (root first, most specific last). */
export function walkUpAgentsDirectories(workspaceRoot: string, startDir: string): string[] {
  const root = normalizeDirectory(workspaceRoot);
  if (!root) return [];

  let current = normalizeDirectory(startDir);
  if (!isPathInsideWorkspace(root, current)) {
    current = root;
  }

  const fromSpecificToRoot: string[] = [];
  while (true) {
    fromSpecificToRoot.push(current);
    if (current === root) break;
    const parent = normalizeDirectory(parentPath(current));
    if (!parent || parent === current || !isPathInsideWorkspace(root, parent)) break;
    current = parent;
  }

  return fromSpecificToRoot.reverse();
}

export function resolveAgentsStartDirectory(workspaceRoot: string, activeDocumentPath: string | null) {
  const root = normalizeDirectory(workspaceRoot);
  if (!activeDocumentPath?.trim()) return root;
  const documentDir = normalizeDirectory(parentPath(activeDocumentPath));
  return isPathInsideWorkspace(root, documentDir) ? documentDir : root;
}

export function agentsFileCandidates(directory: string) {
  return (["AGENTS.md", "Agents.md", "agents.md"] as const).map((fileName) => joinPath(directory, fileName));
}

export function relativeDirectoryLabel(workspaceRoot: string, directory: string) {
  const root = normalizeDirectory(workspaceRoot);
  const current = normalizeDirectory(directory);
  if (current === root) return ".";
  const prefix = `${root}/`;
  return current.startsWith(prefix) ? current.slice(prefix.length) : current;
}

export function isPathInsideWorkspace(workspaceRoot: string, candidatePath: string) {
  const root = normalizeDirectory(workspaceRoot).toLowerCase();
  const candidate = normalizeDirectory(candidatePath).toLowerCase();
  return candidate === root || candidate.startsWith(`${root}/`);
}

function normalizeDirectory(path: string) {
  return normalizePathSlashes(path).replace(/\/+$/, "");
}