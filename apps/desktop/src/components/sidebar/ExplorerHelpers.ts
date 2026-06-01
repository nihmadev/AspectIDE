import { joinPath, normalizePath, parentPath, displayPath } from "../../lib/fileTree";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { FsEntry, WorkspaceInfo } from "../../lib/types";

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
