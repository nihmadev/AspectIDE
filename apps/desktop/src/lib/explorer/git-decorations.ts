import { useMemo } from "react";
import { joinPath, normalizePath, parentPath } from "./file-tree";
import { useLuxStore } from "./../store/index";
import type { GitFileStatus } from "./../types/index";

// Shared git-status → UI decoration logic, used by the file explorer, editor
// tabs, and the status bar so a changed file looks the same everywhere.

export type GitDecoStatus = "added" | "modified" | "deleted" | "untracked" | "renamed" | "conflict";

// Higher wins when a directory rolls up several differing child statuses.
const GIT_DECO_PRIORITY: Record<GitDecoStatus, number> = {
  conflict: 6,
  deleted: 5,
  modified: 4,
  renamed: 3,
  added: 2,
  untracked: 1,
};

/** Single-letter badge shown on the right of a changed row (matches VS Code). */
export function gitDecoBadge(status: GitDecoStatus): string {
  switch (status) {
    case "untracked": return "U";
    case "added": return "A";
    case "deleted": return "D";
    case "renamed": return "R";
    case "conflict": return "!";
    default: return "M";
  }
}

/** A `path -> "old -> new"` rename entry resolves to the new path on disk. */
function gitStatusTargetPath(path: string): string {
  const separator = " -> ";
  const index = path.lastIndexOf(separator);
  return index === -1 ? path : path.slice(index + separator.length);
}

/** Map a porcelain (index, worktree) status pair to a decoration category. */
export function categorizeGitFile(indexStatus: string, worktreeStatus: string): GitDecoStatus {
  const index = indexStatus.trim();
  const worktree = worktreeStatus.trim();
  // Merge conflicts: either side is unmerged, or matching add/delete on both.
  if (index === "U" || worktree === "U" || (index && index === worktree && (index === "A" || index === "D"))) {
    return "conflict";
  }
  // The worktree (unstaged) change is what the user is actively editing; fall back
  // to the staged index change when the worktree side is clean.
  const code = worktree || index;
  switch (code) {
    case "?": return "untracked";
    case "A": return "added";
    case "D": return "deleted";
    case "R": return "renamed";
    default: return "modified"; // M, C, T, …
  }
}

function mergeDeco(map: Map<string, GitDecoStatus>, key: string, status: GitDecoStatus) {
  const current = map.get(key);
  if (!current || GIT_DECO_PRIORITY[status] > GIT_DECO_PRIORITY[current]) {
    map.set(key, status);
  }
}

/**
 * Build `normalizedAbsolutePath -> status` for every changed file plus its ancestor
 * directories (so a folder containing changes is tinted too), keyed to match
 * `normalizePath(absolutePath)`. Paths are git-repo-relative; we join them onto the
 * workspace root (the common repo-root case).
 */
export function buildGitDecorations(
  files: GitFileStatus[],
  workspaceRoot: string | null | undefined,
): Map<string, GitDecoStatus> {
  const map = new Map<string, GitDecoStatus>();
  if (!workspaceRoot) return map;
  const rootKey = normalizePath(workspaceRoot);
  for (const file of files) {
    const status = categorizeGitFile(file.index_status, file.worktree_status);
    const absKey = normalizePath(joinPath(workspaceRoot, gitStatusTargetPath(file.path)));
    mergeDeco(map, absKey, status);
    // Tint every ancestor directory up to (and including) the workspace root.
    let dir = normalizePath(parentPath(absKey));
    while (dir.length >= rootKey.length) {
      mergeDeco(map, dir, status);
      if (dir === rootKey) break;
      const next = normalizePath(parentPath(dir));
      if (next === dir) break;
      dir = next;
    }
  }
  return map;
}

/** Look up the decoration for an absolute path in a prebuilt map. */
export function gitStatusForPath(map: Map<string, GitDecoStatus>, absolutePath: string | null | undefined): GitDecoStatus | null {
  if (!absolutePath) return null;
  return map.get(normalizePath(absolutePath)) ?? null;
}

/**
 * Live decoration map derived from the store's git status + active workspace,
 * memoized so editor tabs / status bar / explorer all share one computation.
 */
export function useGitDecorations(): Map<string, GitDecoStatus> {
  const gitStatus = useLuxStore((state) => state.gitStatus);
  const workspaceRoot = useLuxStore((state) => state.workspace?.root ?? null);
  return useMemo(() => buildGitDecorations(gitStatus?.files ?? [], workspaceRoot), [gitStatus, workspaceRoot]);
}
