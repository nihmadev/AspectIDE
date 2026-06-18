import type { FsEntry } from "./types";

export type FileTreeDirectories = Record<string, FsEntry[]>;

export function cleanPath(path: string) {
  return stripWindowsExtendedPathPrefix(path).replace(/\\/g, "/");
}

export function normalizePath(path: string) {
  return cleanPath(path).replace(/\/+$/, "").toLowerCase();
}

/** Robust workspace-root equality: the same folder can arrive with different slash
 *  direction, trailing slash, or letter case (Windows), so a raw `===` wrongly
 *  scatters a project's chats. Compare on the normalized form (null = no workspace). */
export function sameWorkspaceRoot(a: string | null | undefined, b: string | null | undefined) {
  const left = a ?? null;
  const right = b ?? null;
  if (left === right) return true;
  if (left === null || right === null) return false;
  return normalizePath(left) === normalizePath(right);
}

export function displayPath(path: string) {
  return cleanPath(path);
}

export function parentPath(path: string) {
  const normalized = displayPath(path).replace(/\/+$/, "");
  const index = normalized.lastIndexOf("/");
  return index > 0 ? normalized.slice(0, index) : normalized;
}

export function joinPath(parent: string, child: string) {
  const normalizedParent = stripWindowsExtendedPathPrefix(parent);
  const separator = normalizedParent.includes("\\") ? "\\" : "/";
  return `${normalizedParent.replace(/[\\/]+$/, "")}${separator}${child}`;
}

function stripWindowsExtendedPathPrefix(path: string) {
  return path
    .replace(/^\\\\\?\\UNC\\/i, "\\\\")
    .replace(/^\\\\\?\\/, "");
}

export function buildFileTreeDirectories(root: string, entries: FsEntry[]) {
  const directories: FileTreeDirectories = { [normalizePath(root)]: [] };

  for (const entry of entries) {
    const parentKey = normalizePath(parentPath(entry.path));
    directories[parentKey] ??= [];
    directories[parentKey].push(entry);

    if (entry.kind === "directory") {
      directories[normalizePath(entry.path)] ??= [];
    }
  }

  for (const key of Object.keys(directories)) {
    directories[key] = sortFsEntries(directories[key]);
  }

  return directories;
}

export function sortFsEntries(entries: FsEntry[]) {
  return [...entries].sort((left, right) => {
    const leftPriority = left.kind === "directory" ? 0 : 1;
    const rightPriority = right.kind === "directory" ? 0 : 1;
    if (leftPriority !== rightPriority) return leftPriority - rightPriority;
    return left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: "base" });
  });
}
