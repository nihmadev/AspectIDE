import type { AiChatSendInput } from "./aiChatTypes";
import { normalizePath } from "./fileTree";
import { captureFileTextSnapshot, registerPendingFileReview } from "./aiPendingFileReview";

// Bridges native-turn file edits into the Cursor-style pending-review system
// (green/red editor diff + Accept/Reject bar). The TS browser runtime registers
// reviews inline in aiRuntimeFileTools; the native (Rust) loop only emits turn
// events, so this module reconstructs the same review from the tool's
// start (before text) and completion (after text) events.

/** Tools whose completion produces a reviewable file change. */
export const NATIVE_FILE_EDIT_TOOLS = new Set(["Write", "StrReplace", "PatchEngine"]);

type BeforeSnapshot = { path: string; before: string; faithful: boolean };

function looksAbsolute(path: string): boolean {
  return /^[a-zA-Z]:[\\/]/.test(path) || path.startsWith("\\\\") || path.startsWith("/");
}

/** Resolve a model-supplied path (absolute or workspace-relative) to absolute. */
function toAbsolute(path: string, workspaceRoot: string): string {
  if (!path) return path;
  if (looksAbsolute(path)) return path;
  if (!workspaceRoot) return path;
  const trimmedRoot = workspaceRoot.replace(/[\\/]+$/, "");
  const sep = trimmedRoot.includes("\\") ? "\\" : "/";
  return `${trimmedRoot}${sep}${path.replace(/^[\\/]+/, "")}`;
}

/** A rename op is stored as `OLD -> NEW`; the new path is the live file. */
function editTargetPath(path: string): string {
  const arrow = path.lastIndexOf(" -> ");
  return arrow === -1 ? path : path.slice(arrow + 4);
}

/** Pull the edited file path(s) from a tool call's raw JSON arguments. */
function extractEditPaths(tool: string, rawArgs: string): string[] {
  let args: Record<string, unknown>;
  try {
    args = JSON.parse(rawArgs || "{}") as Record<string, unknown>;
  } catch {
    return [];
  }
  if (tool === "PatchEngine") {
    const ops = Array.isArray(args.operations) ? args.operations : [];
    return ops
      .map((op) => (op && typeof op === "object" ? (op as Record<string, unknown>).path : null))
      .filter((value): value is string => typeof value === "string" && value.length > 0);
  }
  return typeof args.path === "string" && args.path ? [args.path] : [];
}

function relativeTo(workspaceRoot: string, absPath: string): string {
  if (!workspaceRoot) return absPath;
  const root = normalizePath(workspaceRoot);
  const target = normalizePath(absPath);
  return target.startsWith(`${root}/`) ? target.slice(root.length + 1) : absPath;
}

/**
 * Snapshot the pre-edit content of every file a tool call is about to touch.
 * In Default approval mode the loop suspends on the approval prompt before the
 * write lands, so this read reliably captures the BEFORE state with no race.
 */
export async function captureNativeEditBefore(
  tool: string,
  rawArgs: string,
  input: AiChatSendInput,
): Promise<BeforeSnapshot[]> {
  const workspaceRoot = input.workspace?.root ?? "";
  const paths = extractEditPaths(tool, rawArgs).map((path) => toAbsolute(editTargetPath(path), workspaceRoot));
  const seen = new Set<string>();
  const snapshots: BeforeSnapshot[] = [];
  for (const path of paths) {
    const key = normalizePath(path);
    if (seen.has(key)) continue;
    seen.add(key);
    const open = input.openDocuments.find((doc) => doc.path && normalizePath(doc.path) === key);
    const snapshot = await captureFileTextSnapshot(path, open?.text);
    snapshots.push({ path, before: snapshot.text, faithful: snapshot.faithful });
  }
  return snapshots;
}

/**
 * Register a pending review for each changed file once the native edit completes,
 * pairing the captured BEFORE text with the AFTER text from the tool result.
 * No-op in Automatic mode (full autonomy: nothing to accept/reject).
 */
export async function registerNativeEditReview(
  tool: string,
  toolCallId: string,
  resultJson: string,
  before: BeforeSnapshot[],
  input: AiChatSendInput,
): Promise<void> {
  if (input.preferences.agentMode === "automatic" || before.length === 0) return;
  let parsed: Record<string, unknown> = {};
  try {
    parsed = JSON.parse(resultJson || "{}") as Record<string, unknown>;
  } catch {
    // Result wasn't JSON — fall back to re-reading disk for the after text below.
  }
  const editedDocuments = Array.isArray(parsed.editedDocuments)
    ? (parsed.editedDocuments as Array<{ path?: string; text?: string }>)
    : [];
  // savedToDisk defaults true (the native tools persist unless explicitly staged);
  // previewOnly is its inverse — a staged edit must be written on Accept.
  const savedToDisk = parsed.savedToDisk !== false;
  const workspaceRoot = input.workspace?.root ?? "";

  const reviewedPaths: string[] = [];
  for (const { path, before: beforeText, faithful } of before) {
    const key = normalizePath(path);
    const editedText = editedDocuments.find((doc) => doc.path && normalizePath(doc.path) === key)?.text;
    const after = typeof editedText === "string"
      ? { text: editedText, faithful: true }
      : await captureFileTextSnapshot(path);
    if (after.text === beforeText) continue;
    registerPendingFileReview({
      sessionId: input.chatSessionId,
      path,
      relativePath: relativeTo(workspaceRoot, path),
      toolName: tool,
      toolCallId,
      beforeText,
      afterText: after.text,
      previewOnly: !savedToDisk,
      // An unfaithful snapshot (truncated / lossy non-UTF-8) is display-only:
      // the existing textTruncated guards stop accept/reject from writing it
      // back and corrupting or truncating the real file.
      textTruncated: !faithful || !after.faithful || undefined,
    });
    reviewedPaths.push(path);
  }
  // Open an edited file so its green/red diff + Accept/Reject bar are visible
  // (the bar is keyed to the active document), mirroring the TS runtime.
  if (reviewedPaths.length > 0) input.onFilePathsEdited?.(reviewedPaths);
}
