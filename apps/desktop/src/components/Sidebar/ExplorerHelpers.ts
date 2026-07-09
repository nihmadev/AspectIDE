import { joinPath, normalizePath, parentPath, displayPath } from "../../lib/fileTree";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { FsEntry, WorkspaceInfo } from "../../lib/types";

// Git decoration logic now lives in lib/gitDecorations.ts (shared with editor tabs
// + status bar). Re-export so existing explorer imports keep working.
export { buildGitDecorations, gitDecoBadge, type GitDecoStatus } from "../../lib/gitDecorations";

export function buildDirectories(root: string, entries: FsEntry[]) {
  const directories: Record<string, FsEntry[]> = { [normalizePath(root)]: [] };

  for (const entry of entries) {
    const parentKey = normalizePath(parentPath(entry.path));
    directories[parentKey] ??= [];
    directories[parentKey].push(entry);
    if (entry.kind === "directory") directories[normalizePath(entry.path)] ??= [];
  }

  for (const key of Object.keys(directories)) {
    directories[key] = [...directories[key]].sort((left, right) => {
      const leftRank = left.kind === "directory" ? 0 : 1;
      const rightRank = right.kind === "directory" ? 0 : 1;
      if (leftRank !== rightRank) return leftRank - rightRank;
      return left.name.localeCompare(right.name, undefined, { numeric: true, sensitivity: "base" });
    });
  }

  return directories;
}

export function workspaceToEntry(workspace: WorkspaceInfo): FsEntry {
  return {
    name: workspace.name,
    path: workspace.root,
    kind: "directory",
    size: 0,
    modified_at: null,
    is_hidden: false,
  };
}

export function formatWorkspaceRootLabel(name: string) {
  return (name.startsWith("!") ? name : `!${name}`).toUpperCase();
}

export function uniqueDestinationPath(targetDirectory: string, name: string, directories: Record<string, FsEntry[]>, t: TranslateFn) {
  const existing = new Set((directories[normalizePath(targetDirectory)] ?? []).map((entry) => normalizePath(entry.name)));
  if (!existing.has(normalizePath(name))) return joinPath(targetDirectory, name);

  const dotIndex = name.lastIndexOf(".");
  const base = dotIndex > 0 ? name.slice(0, dotIndex) : name;
  const extension = dotIndex > 0 ? name.slice(dotIndex) : "";
  let index = 1;
  let candidate = t("sidebar.explorer.duplicateName.copy", { base, extension });

  while (existing.has(normalizePath(candidate))) {
    index += 1;
    candidate = t("sidebar.explorer.duplicateName.copyIndexed", { base, index, extension });
  }

  return joinPath(targetDirectory, candidate);
}

export function findEntryByNormalizedPath(directories: Record<string, FsEntry[]>, normalizedPath: string) {
  for (const entries of Object.values(directories)) {
    const found = entries.find((entry) => normalizePath(entry.path) === normalizedPath);
    if (found) return found;
  }
  return null;
}

export function countDescendants(path: string, directories: Record<string, FsEntry[]>) {
  const rootKey = normalizePath(path);
  let count = 0;
  const stack = [...(directories[rootKey] ?? [])];
  while (stack.length > 0) {
    const entry = stack.pop();
    if (!entry) continue;
    count += 1;
    if (entry.kind === "directory") stack.push(...(directories[normalizePath(entry.path)] ?? []));
  }
  return count;
}

export function deleteDialogTitle(entry: FsEntry, hasContents: boolean, t: TranslateFn) {
  if (entry.kind === "directory") return hasContents ? t("sidebar.explorer.delete.folderTitle", { name: entry.name }) : t("sidebar.explorer.delete.emptyFolderTitle", { name: entry.name });
  return t("sidebar.explorer.delete.fileTitle", { name: entry.name });
}

export function deleteDialogDescription(entry: FsEntry, childCount: number, t: TranslateFn) {
  if (entry.kind === "directory" && childCount > 0) {
    return t("sidebar.explorer.delete.nonEmptyFolderDescription", { name: entry.name });
  }
  if (entry.kind === "directory") return t("sidebar.explorer.delete.emptyFolderDescription", { name: entry.name });
  return t("sidebar.explorer.delete.fileDescription", { name: entry.name });
}

export function deleteOpenDocumentsMessage(entry: FsEntry, affectedDocumentCount: number, t: TranslateFn) {
  if (entry.kind !== "directory") {
    return t("sidebar.explorer.delete.unsavedOpenFile", { name: entry.name });
  }
  if (affectedDocumentCount <= 1) {
    return t("sidebar.explorer.delete.unsavedOpenFileInsideFolder", { name: entry.name });
  }
  return t("sidebar.explorer.delete.unsavedOpenFilesInsideFolder", { count: affectedDocumentCount, name: entry.name });
}

export function formatItemCount(count: number, t: TranslateFn) {
  return t("sidebar.explorer.itemCount", { count });
}

export function pathIsInsideEntry(entryPath: string, candidatePath: string) {
  const normalizedEntryPath = normalizePath(entryPath);
  const normalizedCandidatePath = normalizePath(candidatePath);
  return normalizedCandidatePath === normalizedEntryPath || normalizedCandidatePath.startsWith(`${normalizedEntryPath}/`);
}

export function validateMoveTarget(entry: FsEntry, targetDirectory: string, directories: Record<string, FsEntry[]>, t: TranslateFn) {
  const entryPath = normalizePath(entry.path);
  const entryParent = normalizePath(parentPath(entry.path));
  const targetPath = normalizePath(targetDirectory);

  if (entryParent === targetPath) return "same-directory";
  if (entry.kind === "directory" && (targetPath === entryPath || targetPath.startsWith(`${entryPath}/`))) {
    return t("sidebar.explorer.move.cannotMoveFolderIntoItself");
  }

  const siblingExists = (directories[targetPath] ?? []).some((candidate) => normalizePath(candidate.name) === normalizePath(entry.name));
  if (siblingExists) return t("sidebar.explorer.move.nameAlreadyExists", { name: entry.name });
  return null;
}

export function displayParentPath(path: string) {
  return displayPath(parentPath(path));
}

// A single flattened, ready-to-render row of the explorer tree. Only rows that
// are actually visible (inside an expanded ancestor chain) are emitted, so the
// virtual list mounts a bounded window regardless of total tree size.
export type ExplorerRow =
  | { kind: "folder-header"; path: string; folderKey: string; depth: number; guideMask: number }
  | { kind: "entry"; entry: FsEntry; entryKey: string; depth: number; guideMask: number }
  | { kind: "create"; parentPath: string; depth: number; guideMask: number };

// Walks the expanded directory map into a flat, depth-tagged row array suitable
// for windowed rendering. Mirrors the previous recursive JSX exactly: directory
// children appear only when the directory is expanded, an inline "create" row is
// injected directly under the parent whose `pendingCreate` is active, and
// multi-root workspaces prepend a folder header per root.
export function flattenVisibleRows(
  roots: { root: string; name: string }[],
  directories: Record<string, FsEntry[]>,
  expandedPaths: Set<string>,
  pendingCreateParentKey: string | null,
  fallbackRootEntries: FsEntry[],
  fallbackRootPath: string,
): ExplorerRow[] {
  const rows: ExplorerRow[] = [];
  const multiRoot = roots.length > 1;

  // `acc` accumulates the bitmask of open folder depths above the current level.
  // Bit P is set when the ancestor at depth P is an expanded directory — its
  // vertical guide line should be drawn through this subtree.
  const pushChildren = (parentKey: string, depth: number, acc: number) => {
    if (pendingCreateParentKey === parentKey) rows.push({ kind: "create", parentPath: parentKey, depth, guideMask: acc });
    for (const child of directories[parentKey] ?? []) {
      const childKey = normalizePath(child.path);
      rows.push({ kind: "entry", entry: child, entryKey: childKey, depth, guideMask: acc });
      if (child.kind === "directory" && expandedPaths.has(childKey)) {
        pushChildren(childKey, depth + 1, acc | (1 << depth));
      }
    }
  };

  for (const folder of roots) {
    const folderKey = normalizePath(folder.root);
    const baseDepth = multiRoot ? 1 : 0;
    if (multiRoot) rows.push({ kind: "folder-header", path: folder.root, folderKey, depth: 0, guideMask: 0 });
    if (!multiRoot || expandedPaths.has(folderKey)) {
      const hasDir = directories[folderKey] !== undefined;
      if (hasDir) {
        pushChildren(folderKey, baseDepth, 0);
      } else if (folder.root === fallbackRootPath) {
        if (pendingCreateParentKey === folderKey) rows.push({ kind: "create", parentPath: folderKey, depth: baseDepth, guideMask: 0 });
        for (const child of fallbackRootEntries) {
          const childKey = normalizePath(child.path);
          rows.push({ kind: "entry", entry: child, entryKey: childKey, depth: baseDepth, guideMask: 0 });
          if (child.kind === "directory" && expandedPaths.has(childKey)) {
            pushChildren(childKey, baseDepth + 1, 1 << baseDepth);
          }
        }
      }
    }
  }

  return rows;
}
