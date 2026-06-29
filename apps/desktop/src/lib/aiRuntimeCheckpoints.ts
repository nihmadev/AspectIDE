import type { PersistedFileCheckpoint } from "./aiChatCheckpointStore";
import { getFileCheckpoint, listFileCheckpoints, removeFileCheckpoint, upsertFileCheckpoint, workspaceCheckpointKey } from "./aiChatCheckpointStore";
import type { AiChatSendInput, AiToolApprovalRequest } from "./aiChatTypes";
import { createCheckpointRestoreApproval } from "./aiRuntimeApprovals";
import { mergeDiffAndStatusFiles } from "./aiRuntimeDiagnostics";
import { createRelatedFileDescriptor, isPathInsideWorkspace, resolveWorkspacePath } from "./aiRuntimeFileContext";
import { buildNumberedPreview, countLines, type RuntimePatchOperation } from "./aiRuntimePatch";
import { booleanArg, clamp, normalizePathSlashes, numberArg, readErrorMessage, stringArg, stringArrayArg, toolJson, truncateText, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { isTauriRuntime, luxCommands } from "./tauri";
import type { DocumentSnapshot } from "./types";

type CheckpointFileSnapshot = {
  path: string;
  relativePath: string;
  existed: boolean;
  text: string;
  size: number;
  truncated: boolean;
  source: "editor" | "disk" | "missing";
  error?: string;
};

type RuntimeCheckpoint = {
  id: string;
  label: string;
  workspaceRoot: string;
  createdAt: string;
  files: CheckpointFileSnapshot[];
  maxBytesPerFile: number;
};

type CheckpointAction = "create" | "list" | "diff" | "delete" | "restore";

type CheckpointCurrentFile = {
  existed: boolean;
  diskExists: boolean;
  text: string;
  size: number | null;
  truncated: boolean;
  source: "editor" | "disk" | "missing";
  error?: string;
};

type CheckpointFileDiff = {
  path: string;
  relativePath: string;
  status: "unchanged" | "modified" | "missing" | "created" | "truncated" | "error";
  existedAtCheckpoint: boolean;
  currentExists: boolean;
  diskExists: boolean;
  snapshotSource: CheckpointFileSnapshot["source"];
  currentSource: CheckpointCurrentFile["source"];
  snapshotSize: number;
  currentSize: number | null;
  snapshotTruncated: boolean;
  currentTruncated: boolean;
  lineDelta: number | null;
  beforePreview?: string;
  currentPreview?: string;
  error?: string;
};

type CheckpointToolUi = {
  requireApproval: (approval: AiToolApprovalRequest) => Promise<void>;
  applyPatch: (operations: RuntimePatchOperation[], saveToDisk: boolean, dryRun: boolean) => Promise<ToolResult>;
};

const maxCheckpointsPerWorkspace = 24;
const defaultCheckpointMaxFiles = 40;
const checkpointMaxFilesLimit = 80;
const defaultCheckpointMaxBytesPerFile = 500_000;
const checkpointMaxBytesPerFileLimit = 1_000_000;
const checkpointStoreByWorkspace = new Map<string, RuntimeCheckpoint[]>();

export type FileCheckpointCreateResult = {
  id: string;
  label: string;
  fileCount: number;
  restorableFileCount: number;
};

export async function checkpointTool(args: UnknownRecord, input: AiChatSendInput, ui: CheckpointToolUi): Promise<ToolResult> {
  const action = normalizeCheckpointAction(stringArg(args, "action", "list"));
  switch (action) {
    case "create":
      return createCheckpoint(args, input);
    case "list":
      return listCheckpoints(input);
    case "diff":
      return diffCheckpoint(args, input);
    case "delete":
      return deleteCheckpoint(args, input);
    case "restore":
      return restoreCheckpoint(args, input, ui);
    default:
      return toolJson("Checkpoint", { error: `Unsupported checkpoint action: ${stringArg(args, "action")}` });
  }
}

async function createCheckpoint(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const workspaceRoot = requireWorkspaceRoot(input);
  const maxFiles = clamp(numberArg(args, "maxFiles", defaultCheckpointMaxFiles), 1, checkpointMaxFilesLimit);
  const maxBytesPerFile = clamp(numberArg(args, "maxBytesPerFile", defaultCheckpointMaxBytesPerFile), 1_024, checkpointMaxBytesPerFileLimit);
  const paths = await checkpointTargetPaths(args, input, maxFiles);
  if (paths.length === 0) {
    return toolJson("Checkpoint", { status: "skipped", reason: "No file paths were available for checkpointing." });
  }

  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const files = await Promise.all(paths.map((path) => snapshotCheckpointFile(path, workspaceRoot, openByPath, maxBytesPerFile)));
  const checkpoint: RuntimeCheckpoint = {
    id: `cp-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 8)}`,
    label: truncateText(stringArg(args, "label", "").trim() || `Checkpoint ${new Date().toLocaleString()}`, 120),
    workspaceRoot,
    createdAt: new Date().toISOString(),
    files,
    maxBytesPerFile,
  };
  const store = checkpointStore(workspaceRoot);
  store.unshift(checkpoint);
  store.splice(maxCheckpointsPerWorkspace);
  persistRuntimeCheckpoint(checkpoint);

  return toolJson("Checkpoint", {
    status: "created",
    checkpoint: checkpointSummary(checkpoint),
    files: files.map(compactCheckpointFile),
    warnings: checkpointWarnings(files),
  });
}

function listCheckpoints(input: AiChatSendInput): ToolResult {
  const workspaceRoot = requireWorkspaceRoot(input);
  const store = checkpointStore(workspaceRoot);
  return toolJson("Checkpoint", {
    workspaceRoot,
    count: store.length,
    checkpoints: store.map(checkpointSummary),
  });
}

async function diffCheckpoint(args: UnknownRecord, input: AiChatSendInput): Promise<ToolResult> {
  const workspaceRoot = requireWorkspaceRoot(input);
  const checkpoint = selectCheckpoint(args, workspaceRoot);
  const pathFilter = checkpointPathFilter(args, workspaceRoot);
  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const files = checkpoint.files.filter((file) => checkpointFileSelected(file, pathFilter));
  const diffs = await Promise.all(files.map((file) => diffCheckpointFile(file, workspaceRoot, openByPath, checkpoint.maxBytesPerFile)));
  return toolJson("Checkpoint", {
    status: "diffed",
    checkpoint: checkpointSummary(checkpoint),
    summary: checkpointDiffSummary(diffs),
    files: diffs,
  });
}

function deleteCheckpoint(args: UnknownRecord, input: AiChatSendInput): ToolResult {
  const workspaceRoot = requireWorkspaceRoot(input);
  const checkpoint = selectCheckpoint(args, workspaceRoot);
  const store = checkpointStore(workspaceRoot);
  const removedAt = store.findIndex((candidate) => candidate.id === checkpoint.id);
  if (removedAt >= 0) store.splice(removedAt, 1);
  // Persist the removal too; without this the deleted checkpoint reappears after a
  // reload (the persisted store is the source of truth for checkpointStore()).
  removeFileCheckpoint(workspaceRoot, checkpoint.id);
  return toolJson("Checkpoint", {
    status: "deleted",
    checkpoint: checkpointSummary(checkpoint),
    remaining: store.map(checkpointSummary),
  });
}

async function restoreCheckpoint(args: UnknownRecord, input: AiChatSendInput, ui: CheckpointToolUi): Promise<ToolResult> {
  const workspaceRoot = requireWorkspaceRoot(input);
  const checkpoint = selectCheckpoint(args, workspaceRoot);
  const pathFilter = checkpointPathFilter(args, workspaceRoot);
  const saveToDisk = booleanArg(args, "saveToDisk", true);
  const dryRun = booleanArg(args, "dryRun", false);
  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const files = checkpoint.files.filter((file) => checkpointFileSelected(file, pathFilter));
  if (files.length === 0) {
    return toolJson("Checkpoint", { status: "skipped", checkpoint: checkpointSummary(checkpoint), reason: "No checkpoint files matched the requested paths." });
  }
  const blocked = files.filter((file) => file.truncated || file.error);
  if (blocked.length > 0) {
    return toolJson("Checkpoint", {
      status: "blocked",
      checkpoint: checkpointSummary(checkpoint),
      reason: "Restore refused because one or more snapshot files were truncated or failed to read.",
      blocked: blocked.map(compactCheckpointFile),
    });
  }

  const current = await Promise.all(files.map((file) => diffCheckpointFile(file, workspaceRoot, openByPath, checkpoint.maxBytesPerFile)));
  const operations = checkpointRestoreOperations(files, current);
  if (operations.length === 0) {
    return toolJson("Checkpoint", {
      status: "unchanged",
      checkpoint: checkpointSummary(checkpoint),
      summary: checkpointDiffSummary(current),
    });
  }

  const approval = createCheckpointRestoreApproval(input.locale, checkpoint, operations, saveToDisk, dryRun);
  await ui.requireApproval(approval);
  // The approval await is open-ended; files may have changed since the diff above. Recompute against
  // the post-approval disk/editor state so unchanged-at-diff-time files edited during the wait are
  // still reverted and the patch reflects what is actually on disk now.
  const currentAfterApproval = await Promise.all(files.map((file) => diffCheckpointFile(file, workspaceRoot, openByPath, checkpoint.maxBytesPerFile)));
  const operationsAfterApproval = checkpointRestoreOperations(files, currentAfterApproval);
  if (operationsAfterApproval.length === 0) {
    return toolJson("Checkpoint", {
      status: "unchanged",
      checkpoint: checkpointSummary(checkpoint),
      summary: checkpointDiffSummary(currentAfterApproval),
    });
  }
  return ui.applyPatch(operationsAfterApproval, saveToDisk, dryRun);
}

function normalizeCheckpointAction(value: string): CheckpointAction | "" {
  const normalized = value.trim().toLowerCase().replace(/[-_\s]+/g, "");
  if (normalized === "create" || normalized === "snapshot" || normalized === "save") return "create";
  if (normalized === "list" || normalized === "ls") return "list";
  if (normalized === "diff" || normalized === "compare") return "diff";
  if (normalized === "delete" || normalized === "remove" || normalized === "drop") return "delete";
  if (normalized === "restore" || normalized === "rollback" || normalized === "revert") return "restore";
  return "";
}

function requireWorkspaceRoot(input: AiChatSendInput) {
  const root = normalizePathSlashes(input.workspace?.root ?? "").replace(/\/+$/, "");
  if (!root) throw new Error("Checkpoint requires an open workspace.");
  return root;
}

async function checkpointTargetPaths(args: UnknownRecord, input: AiChatSendInput, maxFiles: number) {
  const workspaceRoot = requireWorkspaceRoot(input);
  const selected = new Map<string, string>();
  let hasExplicitPaths = false;
  const addPath = (path: string | null | undefined) => {
    if (!path || !path.trim()) return;
    const resolved = resolveWorkspacePath(path, workspaceRoot);
    if (!isPathInsideWorkspace(resolved, workspaceRoot)) return;
    const normalized = normalizePathSlashes(resolved);
    selected.set(normalized.toLowerCase(), normalized);
  };

  for (const path of stringArrayArg(args, "paths")) {
    hasExplicitPaths = true;
    addPath(path);
  }
  if (typeof args.path === "string" && args.path.trim()) {
    hasExplicitPaths = true;
    addPath(args.path);
  }

  if (hasExplicitPaths) return Array.from(selected.values()).slice(0, maxFiles);

  const includeOpenDocuments = booleanArg(args, "includeOpenDocuments", true);
  if (includeOpenDocuments) {
    addPath(input.activeDocument?.path);
    for (const document of input.openDocuments) addPath(document.path);
  }

  if (booleanArg(args, "includeGitChanges", true)) {
    try {
      const [status, diff] = await Promise.all([luxCommands.gitStatus(), luxCommands.gitDiff()]);
      for (const file of mergeDiffAndStatusFiles(diff.files, status.files)) {
        addPath(file.path);
        addPath(file.old_path);
      }
    } catch {
      // Git data is opportunistic. Open and explicit paths still make the checkpoint useful.
    }
  }

  return Array.from(selected.values()).slice(0, maxFiles);
}

function openDocumentByAbsolutePath(input: AiChatSendInput, workspaceRoot: string) {
  const byPath = new Map<string, DocumentSnapshot>();
  const add = (document: DocumentSnapshot | null | undefined) => {
    if (!document?.path) return;
    const path = resolveWorkspacePath(document.path, workspaceRoot);
    if (!isPathInsideWorkspace(path, workspaceRoot)) return;
    byPath.set(normalizePathSlashes(path).toLowerCase(), document);
  };
  for (const document of input.openDocuments) add(document);
  add(input.activeDocument);
  return byPath;
}

async function snapshotCheckpointFile(path: string, workspaceRoot: string, openByPath: Map<string, DocumentSnapshot>, maxBytesPerFile: number): Promise<CheckpointFileSnapshot> {
  const normalized = normalizePathSlashes(resolveWorkspacePath(path, workspaceRoot));
  const relativePath = createRelatedFileDescriptor({ path: normalized }, workspaceRoot).relativePath;
  const openDocument = openByPath.get(normalized.toLowerCase());
  if (openDocument) {
    const text = openDocument.text.slice(0, maxBytesPerFile);
    return {
      path: normalized,
      relativePath,
      existed: !openDocument.is_untitled,
      text,
      size: openDocument.text.length,
      truncated: openDocument.text.length > maxBytesPerFile,
      source: "editor",
    };
  }

  try {
    const response = await luxCommands.fsReadText(normalized, maxBytesPerFile);
    return {
      path: normalizePathSlashes(response.path),
      relativePath,
      existed: true,
      text: response.text,
      size: response.size,
      truncated: response.truncated,
      source: "disk",
    };
  } catch (error) {
    return {
      path: normalized,
      relativePath,
      existed: false,
      text: "",
      size: 0,
      truncated: false,
      source: "missing",
      error: readErrorMessage(error),
    };
  }
}

export async function createFileCheckpointForTurn(
  input: AiChatSendInput,
  label: string,
): Promise<FileCheckpointCreateResult> {
  const workspaceRoot = requireWorkspaceRoot(input);
  const maxFiles = defaultCheckpointMaxFiles;
  const maxBytesPerFile = defaultCheckpointMaxBytesPerFile;

  // Native Rust path: file snapshot store lives in ai_checkpoint.
  if (isTauriRuntime()) {
    try {
      const result = await luxCommands.aiCheckpoint("create", {
        label,
        maxFiles,
        maxBytesPerFile,
        nowMs: Date.now(),
      }) as { checkpoint?: { id: string; fileCount: number; restorableFileCount: number } };
      const cp = result.checkpoint;
      if (cp) {
        return { id: cp.id, label, fileCount: cp.fileCount, restorableFileCount: cp.restorableFileCount };
      }
    } catch {
      // Fall through to the TS snapshot path.
    }
  }

  const hasOpenEditorTabs = input.openDocuments.some((document) => Boolean(document.path?.trim()));
  const paths = await checkpointTargetPaths({
    includeOpenDocuments: hasOpenEditorTabs,
    includeGitChanges: true,
  }, input, maxFiles);
  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const files = await Promise.all(paths.map((path) => snapshotCheckpointFile(path, workspaceRoot, openByPath, maxBytesPerFile)));
  const checkpoint: RuntimeCheckpoint = {
    id: `cp-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 8)}`,
    label: truncateText(label.trim() || `Turn ${new Date().toLocaleString()}`, 120),
    workspaceRoot,
    createdAt: new Date().toISOString(),
    files,
    maxBytesPerFile,
  };
  const store = checkpointStore(workspaceRoot);
  store.unshift(checkpoint);
  store.splice(maxCheckpointsPerWorkspace);
  persistRuntimeCheckpoint(checkpoint);
  return {
    id: checkpoint.id,
    label: checkpoint.label,
    fileCount: files.length,
    restorableFileCount: files.filter((file) => !file.truncated && !file.error).length,
  };
}

export async function ensurePathsInFileCheckpoint(
  workspaceRoot: string,
  checkpointId: string,
  paths: string[],
  input: AiChatSendInput,
) {
  if (paths.length === 0) return;
  // Native turn checkpoints are owned by the Rust store and never inserted into the TS store; their
  // snapshots also never cross IPC (text is #[serde(skip)]). Route augmentation through the native
  // `augment` action so pre-edit snapshots for files the model is about to create/edit — files that
  // were neither open at turn start nor already in the native snapshot — are captured in the Rust
  // store and stay restorable. Without this, a later restore has no pre-edit state for those paths,
  // so the edit cannot be reverted and newly created files cannot be deleted (silent data loss).
  if (isTauriRuntime() && !hasTsCheckpoint(workspaceRoot, checkpointId)) {
    const nativePaths = paths
      .map((path) => resolveWorkspacePath(path, workspaceRoot))
      .filter((path) => isPathInsideWorkspace(path, workspaceRoot))
      .map((path) => normalizePathSlashes(path));
    if (nativePaths.length === 0) return;
    await luxCommands.aiCheckpoint("augment", {
      id: checkpointId,
      paths: nativePaths,
      nowMs: Date.now(),
    });
    return;
  }
  const checkpoint = selectCheckpointById(workspaceRoot, checkpointId);
  const existing = new Set(checkpoint.files.map((file) => normalizePathSlashes(file.path).toLowerCase()));
  const missing = paths
    .map((path) => resolveWorkspacePath(path, workspaceRoot))
    .filter((path) => isPathInsideWorkspace(path, workspaceRoot))
    .map((path) => normalizePathSlashes(path))
    .filter((path) => !existing.has(path.toLowerCase()));
  if (missing.length === 0) return;
  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const snapshots = await Promise.all(
    missing.map((path) => snapshotCheckpointFile(path, workspaceRoot, openByPath, checkpoint.maxBytesPerFile)),
  );
  checkpoint.files.push(...snapshots);
  const store = checkpointStore(workspaceRoot);
  const index = store.findIndex((candidate) => candidate.id === checkpoint.id);
  if (index >= 0) store[index] = checkpoint;
  else store.unshift(checkpoint);
  persistRuntimeCheckpoint(checkpoint);
}

export async function restoreFileCheckpointById(
  workspaceRoot: string,
  checkpointId: string,
  input: AiChatSendInput,
  applyPatch: CheckpointToolUi["applyPatch"],
) {
  // Native turn checkpoints live in the Rust store and are never inserted into the TS store, so an
  // id absent from the TS store is a native id. Route the restore through the native command, which
  // already diffs each snapshot against the current state and applies the guarded file patch.
  if (isTauriRuntime() && !hasTsCheckpoint(workspaceRoot, checkpointId)) {
    const response = (await luxCommands.aiCheckpoint("restore", {
      id: checkpointId,
      saveToDisk: true,
      dryRun: false,
      nowMs: Date.now(),
    })) as { status?: string; operations?: number; result?: { changedPaths?: string[] } };
    const restoredPaths = (response.result?.changedPaths ?? []).map((path) => normalizePathSlashes(path));
    return { restoredPaths, operations: response.operations ?? restoredPaths.length, result: response.result };
  }

  const checkpoint = selectCheckpointById(workspaceRoot, checkpointId);
  const openByPath = openDocumentByAbsolutePath(input, workspaceRoot);
  const files = checkpoint.files;
  const blocked = files.filter((file) => file.truncated || file.error);
  if (blocked.length > 0) {
    throw new Error("Restore blocked: one or more snapshot files were truncated or unreadable.");
  }
  const current = await Promise.all(files.map((file) => diffCheckpointFile(file, workspaceRoot, openByPath, checkpoint.maxBytesPerFile)));
  const operations = checkpointRestoreOperations(files, current);
  if (operations.length === 0) return { restoredPaths: [] as string[], operations: 0 };
  const result = await applyPatch(operations, true, false);
  const restoredPaths = operations.map((operation) => operation.path);
  return { restoredPaths, operations: operations.length, result };
}

function checkpointStore(workspaceRoot: string) {
  const key = workspaceCheckpointKey(workspaceRoot);
  const existing = checkpointStoreByWorkspace.get(key);
  if (existing) return existing;
  const persisted = listFileCheckpoints(workspaceRoot).map(runtimeCheckpointFromPersisted);
  const next = persisted.length > 0 ? persisted : [];
  checkpointStoreByWorkspace.set(key, next);
  return next;
}

function selectCheckpointById(workspaceRoot: string, id: string) {
  const store = checkpointStore(workspaceRoot);
  const checkpoint = store.find((candidate) => candidate.id === id);
  if (!checkpoint) throw new Error(`Checkpoint not found: ${id}`);
  return checkpoint;
}

function hasTsCheckpoint(workspaceRoot: string, id: string) {
  return checkpointStore(workspaceRoot).some((candidate) => candidate.id === id);
}

function persistRuntimeCheckpoint(checkpoint: RuntimeCheckpoint) {
  upsertFileCheckpoint(checkpoint.workspaceRoot, runtimeCheckpointToPersisted(checkpoint));
}

function runtimeCheckpointToPersisted(checkpoint: RuntimeCheckpoint): PersistedFileCheckpoint {
  return {
    id: checkpoint.id,
    label: checkpoint.label,
    workspaceRoot: checkpoint.workspaceRoot,
    createdAt: checkpoint.createdAt,
    maxBytesPerFile: checkpoint.maxBytesPerFile,
    files: checkpoint.files.map((file) => ({ ...file })),
  };
}

function runtimeCheckpointFromPersisted(checkpoint: PersistedFileCheckpoint): RuntimeCheckpoint {
  return {
    id: checkpoint.id,
    label: checkpoint.label,
    workspaceRoot: checkpoint.workspaceRoot,
    createdAt: checkpoint.createdAt,
    maxBytesPerFile: checkpoint.maxBytesPerFile,
    files: checkpoint.files.map((file) => ({ ...file })),
  };
}

function selectCheckpoint(args: UnknownRecord, workspaceRoot: string) {
  const store = checkpointStore(workspaceRoot);
  if (store.length === 0) {
    const persisted = getFileCheckpoint(workspaceRoot, stringArg(args, "id", "").trim());
    if (persisted) return runtimeCheckpointFromPersisted(persisted);
    throw new Error("No checkpoints exist for this workspace.");
  }
  const id = stringArg(args, "id", "").trim();
  if (!id) return store[0];
  const checkpoint = store.find((candidate) => candidate.id === id);
  if (!checkpoint) throw new Error(`Checkpoint not found: ${id}`);
  return checkpoint;
}

function checkpointSummary(checkpoint: RuntimeCheckpoint) {
  const files = checkpoint.files;
  return {
    id: checkpoint.id,
    label: checkpoint.label,
    workspaceRoot: checkpoint.workspaceRoot,
    createdAt: checkpoint.createdAt,
    fileCount: files.length,
    restorableFileCount: files.filter((file) => !file.truncated && !file.error).length,
    truncatedFileCount: files.filter((file) => file.truncated).length,
    errorFileCount: files.filter((file) => file.error).length,
    maxBytesPerFile: checkpoint.maxBytesPerFile,
  };
}

function compactCheckpointFile(file: CheckpointFileSnapshot) {
  return {
    path: file.path,
    relativePath: file.relativePath,
    existed: file.existed,
    source: file.source,
    size: file.size,
    lines: file.existed ? countLines(file.text) : 0,
    truncated: file.truncated,
    error: file.error,
  };
}

function checkpointWarnings(files: CheckpointFileSnapshot[]) {
  const warnings: string[] = [];
  const truncated = files.filter((file) => file.truncated);
  const errors = files.filter((file) => file.error);
  const missing = files.filter((file) => !file.existed && file.source === "missing");
  if (truncated.length > 0) warnings.push(`${truncated.length} file${truncated.length === 1 ? "" : "s"} exceeded the snapshot byte limit and cannot be restored.`);
  if (errors.length > 0) warnings.push(`${errors.length} file${errors.length === 1 ? "" : "s"} could not be read.`);
  if (missing.length > 0) warnings.push(`${missing.length} missing path${missing.length === 1 ? "" : "s"} recorded so restore can delete newly created files if needed.`);
  return warnings;
}

function checkpointPathFilter(args: UnknownRecord, workspaceRoot: string) {
  const paths = stringArrayArg(args, "paths");
  if (typeof args.path === "string" && args.path.trim()) paths.push(args.path);
  return paths
    .map((path) => resolveWorkspacePath(path, workspaceRoot))
    .filter((path) => isPathInsideWorkspace(path, workspaceRoot))
    .map((path) => normalizePathSlashes(path).toLowerCase());
}

function checkpointFileSelected(file: CheckpointFileSnapshot, pathFilter: string[]) {
  if (pathFilter.length === 0) return true;
  const lower = normalizePathSlashes(file.path).toLowerCase();
  return pathFilter.some((path) => lower === path || lower.endsWith(`/${path}`));
}

async function diffCheckpointFile(file: CheckpointFileSnapshot, workspaceRoot: string, openByPath: Map<string, DocumentSnapshot>, maxBytesPerFile: number): Promise<CheckpointFileDiff> {
  const current = await readCheckpointCurrentFile(file.path, workspaceRoot, openByPath, maxBytesPerFile);
  const status = checkpointDiffStatus(file, current);
  const lineDelta = current.existed && file.existed ? countLines(current.text) - countLines(file.text) : null;
  return {
    path: file.path,
    relativePath: file.relativePath,
    status,
    existedAtCheckpoint: file.existed,
    currentExists: current.existed,
    diskExists: current.diskExists,
    snapshotSource: file.source,
    currentSource: current.source,
    snapshotSize: file.size,
    currentSize: current.size,
    snapshotTruncated: file.truncated,
    currentTruncated: current.truncated,
    lineDelta,
    beforePreview: file.existed ? truncateText(buildNumberedPreview(file.text, 24), 2_400) : undefined,
    currentPreview: current.existed ? truncateText(buildNumberedPreview(current.text, 24), 2_400) : undefined,
    error: file.error ?? current.error,
  };
}

async function readCheckpointCurrentFile(path: string, workspaceRoot: string, openByPath: Map<string, DocumentSnapshot>, maxBytesPerFile: number): Promise<CheckpointCurrentFile> {
  const normalized = normalizePathSlashes(resolveWorkspacePath(path, workspaceRoot));
  const openDocument = openByPath.get(normalized.toLowerCase());
  if (openDocument) {
    const diskExists = await checkpointDiskExists(normalized, maxBytesPerFile);
    return {
      existed: true,
      diskExists,
      text: openDocument.text.slice(0, maxBytesPerFile),
      size: openDocument.text.length,
      truncated: openDocument.text.length > maxBytesPerFile,
      source: "editor",
    };
  }

  try {
    const response = await luxCommands.fsReadText(normalized, maxBytesPerFile);
    return {
      existed: true,
      diskExists: true,
      text: response.text,
      size: response.size,
      truncated: response.truncated,
      source: "disk",
    };
  } catch (error) {
    return {
      existed: false,
      diskExists: false,
      text: "",
      size: null,
      truncated: false,
      source: "missing",
      error: readErrorMessage(error),
    };
  }
}

async function checkpointDiskExists(path: string, maxBytesPerFile: number) {
  try {
    await luxCommands.fsReadText(path, Math.min(maxBytesPerFile, 1_024));
    return true;
  } catch {
    return false;
  }
}

function checkpointDiffStatus(file: CheckpointFileSnapshot, current: CheckpointCurrentFile): CheckpointFileDiff["status"] {
  if (file.error || current.error && current.existed) return "error";
  if (file.truncated || current.truncated) return "truncated";
  if (file.existed && !current.existed) return "missing";
  if (!file.existed && current.existed) return "created";
  if (!file.existed && !current.existed) return "unchanged";
  return file.text === current.text ? "unchanged" : "modified";
}

function checkpointDiffSummary(diffs: CheckpointFileDiff[]) {
  return {
    total: diffs.length,
    unchanged: diffs.filter((diff) => diff.status === "unchanged").length,
    modified: diffs.filter((diff) => diff.status === "modified").length,
    missing: diffs.filter((diff) => diff.status === "missing").length,
    created: diffs.filter((diff) => diff.status === "created").length,
    truncated: diffs.filter((diff) => diff.status === "truncated").length,
    errored: diffs.filter((diff) => diff.status === "error").length,
  };
}

function checkpointRestoreOperations(files: CheckpointFileSnapshot[], current: CheckpointFileDiff[]): RuntimePatchOperation[] {
  const currentByPath = new Map(current.map((file) => [normalizePathSlashes(file.path).toLowerCase(), file]));
  const operations: RuntimePatchOperation[] = [];

  for (const file of files) {
    const diff = currentByPath.get(normalizePathSlashes(file.path).toLowerCase());
    if (!diff || diff.status === "unchanged") continue;
    if (file.truncated || file.error || diff.currentTruncated || diff.status === "error" || diff.status === "truncated") continue;
    if (file.existed) {
      operations.push({
        action: diff.diskExists ? "rewrite" : "create",
        path: file.path,
        text: file.text,
        overwrite: diff.diskExists ? undefined : false,
      });
    } else if (diff.diskExists) {
      operations.push({ action: "delete", path: file.path });
    }
  }

  return operations;
}
