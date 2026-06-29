import { describe, expect, it } from "vitest";
import {
  fileTreeDirectoriesEqual,
  flattenDiagnostics,
  fsEntriesEqual,
  gitStatusEqual,
  languageServersEqual,
  searchResponsesEqual,
} from "./storeEquality";
import type { FsEntry, GitStatus, LanguageServerInfo, SearchResponse, WorkspaceDiagnostic } from "./types";

const fsEntry = (path: string, overrides: Partial<FsEntry> = {}): FsEntry => ({
  name: path.split("/").pop() ?? path,
  path,
  kind: "file",
  size: 1,
  modified_at: "2026-01-01T00:00:00Z",
  is_hidden: false,
  ...overrides,
});

describe("fsEntriesEqual", () => {
  it("treats structurally identical snapshots as equal (skips redundant write)", () => {
    expect(fsEntriesEqual([fsEntry("a")], [fsEntry("a")])).toBe(true);
  });

  it("detects a changed field", () => {
    expect(fsEntriesEqual([fsEntry("a", { size: 1 })], [fsEntry("a", { size: 2 })])).toBe(false);
  });

  it("detects length changes", () => {
    expect(fsEntriesEqual([fsEntry("a")], [fsEntry("a"), fsEntry("b")])).toBe(false);
  });
});

describe("fileTreeDirectoriesEqual", () => {
  it("compares per-directory entries", () => {
    expect(fileTreeDirectoriesEqual({ root: [fsEntry("a")] }, { root: [fsEntry("a")] })).toBe(true);
    expect(fileTreeDirectoriesEqual({ root: [fsEntry("a")] }, { root: [fsEntry("b")] })).toBe(false);
  });

  it("detects added/removed directories", () => {
    expect(fileTreeDirectoriesEqual({ root: [] }, { root: [], extra: [] })).toBe(false);
  });
});

describe("gitStatusEqual", () => {
  const status = (overrides: Partial<GitStatus> = {}): GitStatus => ({ branch: "main", ahead: 0, behind: 0, files: [], ...overrides });

  it("is reference-stable for identical status", () => {
    expect(gitStatusEqual(status(), status())).toBe(true);
  });

  it("detects branch and file changes", () => {
    expect(gitStatusEqual(status({ branch: "main" }), status({ branch: "dev" }))).toBe(false);
    expect(gitStatusEqual(
      status({ files: [{ path: "a", index_status: "M", worktree_status: " " }] }),
      status({ files: [{ path: "a", index_status: " ", worktree_status: "M" }] }),
    )).toBe(false);
  });

  it("handles null transitions", () => {
    expect(gitStatusEqual(null, null)).toBe(true);
    expect(gitStatusEqual(null, status())).toBe(false);
  });
});

describe("languageServersEqual", () => {
  const server = (overrides: Partial<LanguageServerInfo> = {}): LanguageServerInfo => ({
    language_id: "rust",
    name: "rust-analyzer",
    command: "rust-analyzer",
    args: [],
    workspace_root: "/ws",
    status: "available",
    error: null,
    ...overrides,
  });

  it("ignores identical snapshots but catches status drift", () => {
    expect(languageServersEqual([server()], [server()])).toBe(true);
    expect(languageServersEqual([server({ status: "available" })], [server({ status: "missing" })])).toBe(false);
  });
});

describe("searchResponsesEqual", () => {
  const response = (overrides: Partial<SearchResponse> = {}): SearchResponse => ({
    query: "foo",
    hits: [],
    truncated: false,
    elapsed_ms: 5,
    ...overrides,
  });

  it("ignores elapsed_ms timing noise", () => {
    expect(searchResponsesEqual(response({ elapsed_ms: 5 }), response({ elapsed_ms: 999 }))).toBe(true);
  });

  it("detects query and hit changes", () => {
    expect(searchResponsesEqual(response({ query: "a" }), response({ query: "b" }))).toBe(false);
  });
});

describe("flattenDiagnostics", () => {
  const diag = (path: string): WorkspaceDiagnostic => ({
    path,
    line: 1,
    column: 1,
    severity: "error",
    message: "boom",
    source: "test",
  });

  it("returns a stable reference for the same diagnosticsByPath object", () => {
    const byPath = { a: [diag("a")], b: [diag("b")] };
    const first = flattenDiagnostics(byPath);
    const second = flattenDiagnostics(byPath);
    expect(first).toBe(second); // memoized — no fresh array on unrelated store writes
    expect(first).toHaveLength(2);
  });

  it("recomputes when diagnosticsByPath identity changes", () => {
    const a = flattenDiagnostics({ a: [diag("a")] });
    const b = flattenDiagnostics({ a: [diag("a")] });
    expect(a).not.toBe(b);
  });
});
