import type { FileTreeDirectories } from "./../explorer/file-tree";
import type { FsEntry, GitStatus, LanguageServerInfo, SearchResponse, WorkspaceDiagnostic } from "./../types/index";

/**
 * Equality predicates for the snapshot payloads that arrive over IPC (file tree,
 * git, language servers, search). Polling and event snapshots frequently re-send a
 * semantically identical payload; committing a fresh reference anyway re-renders
 * large UI regions (explorer, source control, search, LSP status) for no reason.
 * These helpers let the store skip the write when nothing actually changed.
 */

function arraysEqual<T>(left: readonly T[], right: readonly T[], itemsEqual: (a: T, b: T) => boolean): boolean {
  if (left === right) return true;
  if (left.length !== right.length) return false;
  for (let index = 0; index < left.length; index += 1) {
    if (!itemsEqual(left[index], right[index])) return false;
  }
  return true;
}

function fsEntryEqual(a: FsEntry, b: FsEntry): boolean {
  return a.path === b.path
    && a.name === b.name
    && a.kind === b.kind
    && a.size === b.size
    && a.modified_at === b.modified_at
    && a.is_hidden === b.is_hidden;
}

export function fsEntriesEqual(left: readonly FsEntry[], right: readonly FsEntry[]): boolean {
  return arraysEqual(left, right, fsEntryEqual);
}

export function fileTreeDirectoriesEqual(left: FileTreeDirectories, right: FileTreeDirectories): boolean {
  if (left === right) return true;
  const leftKeys = Object.keys(left);
  const rightKeys = Object.keys(right);
  if (leftKeys.length !== rightKeys.length) return false;
  for (const key of leftKeys) {
    const rightEntries = right[key];
    if (!rightEntries || !fsEntriesEqual(left[key], rightEntries)) return false;
  }
  return true;
}

export function gitStatusEqual(left: GitStatus | null, right: GitStatus | null): boolean {
  if (left === right) return true;
  if (!left || !right) return false;
  if (left.branch !== right.branch || left.ahead !== right.ahead || left.behind !== right.behind) return false;
  return arraysEqual(left.files, right.files, (a, b) =>
    a.path === b.path && a.index_status === b.index_status && a.worktree_status === b.worktree_status);
}

export function languageServersEqual(left: readonly LanguageServerInfo[], right: readonly LanguageServerInfo[]): boolean {
  return arraysEqual(left, right, (a, b) =>
    a.language_id === b.language_id
    && a.name === b.name
    && a.command === b.command
    && a.workspace_root === b.workspace_root
    && a.status === b.status
    && a.error === b.error
    && arraysEqual(a.args, b.args, (x, y) => x === y));
}

export function searchResponsesEqual(left: SearchResponse | null, right: SearchResponse | null): boolean {
  if (left === right) return true;
  if (!left || !right) return false;
  // `elapsed_ms` is timing noise — two runs of the same query with the same hits
  // are semantically the same snapshot, so it is intentionally excluded.
  if (left.query !== right.query || left.truncated !== right.truncated) return false;
  return arraysEqual(left.hits, right.hits, (a, b) =>
    a.path === b.path
    && a.line === b.line
    && a.column === b.column
    && a.match_length === b.match_length
    && a.match_text === b.match_text
    && a.preview === b.preview
    && a.preview_match_start === b.preview_match_start
    && a.preview_match_length === b.preview_match_length);
}

/**
 * Memoized flattened-diagnostics selector backing. `Object.values(...).flat()`
 * allocates a brand-new array on every call, so a raw selector hands React a fresh
 * reference on every unrelated store write (AI token deltas, terminal bytes) and
 * forces diagnostics-bound UI to re-render. Keying the result by the
 * `diagnosticsByPath` reference returns a stable array until diagnostics change.
 */
const flattenedDiagnosticsCache = new WeakMap<Record<string, WorkspaceDiagnostic[]>, WorkspaceDiagnostic[]>();

export function flattenDiagnostics(diagnosticsByPath: Record<string, WorkspaceDiagnostic[]>): WorkspaceDiagnostic[] {
  const cached = flattenedDiagnosticsCache.get(diagnosticsByPath);
  if (cached) return cached;
  const flattened = Object.values(diagnosticsByPath).flat();
  flattenedDiagnosticsCache.set(diagnosticsByPath, flattened);
  return flattened;
}
