import { useCallback, useSyncExternalStore } from "react";
import { isTauriRuntime, luxCommands, subscribeUpdateProgress, type UpdateCheckResult } from "./tauri";

/**
 * Update lifecycle state machine:
 *   idle → checking → (available | upToDate | error)
 *   available → downloading → (relaunching | error)
 *
 * `downloading` carries byte progress so the UI can render a bar. `relaunching`
 * is terminal from the app's perspective — the process is about to be replaced.
 */
export type UpdaterStatus =
  | "idle"
  | "checking"
  | "available"
  | "upToDate"
  | "downloading"
  | "relaunching"
  | "error";

export type UpdaterState = {
  status: UpdaterStatus;
  currentVersion: string;
  availableVersion: string | null;
  notes: string | null;
  /** 0–1 download fraction when known, else null (indeterminate). */
  progress: number | null;
  downloadedBytes: number;
  totalBytes: number | null;
  error: string | null;
  /** Timestamp (ms) of the last completed check, for "checked just now" UI. */
  lastCheckedAt: number | null;
};

const initialState: UpdaterState = {
  status: "idle",
  currentVersion: "",
  availableVersion: null,
  notes: null,
  progress: null,
  downloadedBytes: 0,
  totalBytes: null,
  error: null,
  lastCheckedAt: null,
};

/** How often to auto-check in the background (6h). Kept long to avoid endpoint spam. */
const AUTO_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
/** Delay before the first auto-check. Short and off the critical path: the check
 *  is a background HTTP request that can't block boot, and an available update
 *  should be visible right after launch — not 30 seconds in. */
const INITIAL_CHECK_DELAY_MS = 1_500;

// ── Shared singleton store ──
// One updater drives the whole app, so the corner UpdateNotice and the Settings
// → Updates panel render the SAME state: a check (or install) started in one is
// instantly reflected in the other, and only one background interval + progress
// subscription ever runs no matter how many components mount the hook.

let state: UpdaterState = initialState;
const listeners = new Set<() => void>();
// Guards against overlapping checks/installs.
let busy = false;

function setState(patch: Partial<UpdaterState>) {
  state = { ...state, ...patch };
  for (const listener of listeners) listener();
}

async function runCheck(options: { silent?: boolean } = {}) {
  if (busy || !isTauriRuntime()) return;
  busy = true;
  if (!options.silent) setState({ status: "checking", error: null });
  try {
    const result: UpdateCheckResult = await luxCommands.updateCheck();
    setState({
      currentVersion: result.currentVersion,
      lastCheckedAt: Date.now(),
      ...(result.available
        ? { status: "available", availableVersion: result.version, notes: result.notes }
        : { status: "upToDate", availableVersion: null, notes: null }),
    });
  } catch (error) {
    // A silent background check that fails should not surface an error UI;
    // just stay idle until the next manual/auto attempt.
    if (options.silent) {
      setState({ status: "idle" });
    } else {
      setState({ status: "error", error: readMessage(error) });
    }
  } finally {
    busy = false;
  }
}

async function runInstall() {
  if (busy || !isTauriRuntime()) return;
  busy = true;
  setState({ status: "downloading", error: null, progress: null, downloadedBytes: 0, totalBytes: null });
  try {
    // The process is replaced on success, so this resolves only mid-relaunch
    // or never returns. Mark relaunching optimistically before awaiting.
    await luxCommands.updateInstall();
    setState({ status: "relaunching" });
  } catch (error) {
    setState({ status: "error", error: readMessage(error) });
    busy = false;
  }
}

function dismissNotice() {
  // Return to a neutral state without forgetting the detected version, so a
  // dismissed banner can be re-surfaced from Settings.
  setState({ status: "idle", error: null });
}

// Lazily wire the global progress stream + background check loop exactly once,
// the first time any component subscribes. Both run for the app's lifetime
// (UpdateNoticeHost is mounted at the root), so there is nothing to tear down.
let started = false;
function ensureStarted() {
  if (started || !isTauriRuntime()) return;
  started = true;

  void subscribeUpdateProgress((event) => {
    if (event.kind === "started") {
      setState({ status: "downloading", totalBytes: event.contentLength, downloadedBytes: 0, progress: event.contentLength ? 0 : null });
    } else if (event.kind === "progress") {
      const fraction = event.contentLength ? Math.min(1, event.downloaded / event.contentLength) : null;
      setState({ downloadedBytes: event.downloaded, totalBytes: event.contentLength, progress: fraction });
    } else {
      setState({ progress: 1, status: "relaunching" });
    }
  });

  window.setTimeout(() => void runCheck({ silent: true }), INITIAL_CHECK_DELAY_MS);
  window.setInterval(() => void runCheck({ silent: true }), AUTO_CHECK_INTERVAL_MS);
}

function subscribe(listener: () => void) {
  ensureStarted();
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot() {
  return state;
}

/**
 * Subscribe to the shared updater. Every caller sees the same state and shares
 * one background check loop, install action, and download-progress stream.
 */
export function useUpdater() {
  const current = useSyncExternalStore(subscribe, getSnapshot, getSnapshot);
  const check = useCallback((options: { silent?: boolean } = {}) => runCheck(options), []);
  const install = useCallback(() => runInstall(), []);
  const dismiss = useCallback(() => dismissNotice(), []);
  return { state: current, check, install, dismiss };
}

function readMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
