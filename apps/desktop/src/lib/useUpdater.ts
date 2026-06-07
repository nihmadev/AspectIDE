import { useCallback, useEffect, useRef, useState } from "react";
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
/** Delay before the first auto-check so startup stays snappy. */
const INITIAL_CHECK_DELAY_MS = 30 * 1000;

export function useUpdater() {
  const [state, setState] = useState<UpdaterState>(initialState);
  // Guards against overlapping checks/installs and post-unmount state writes.
  const busyRef = useRef(false);
  const mountedRef = useRef(true);

  const safeSet = useCallback((patch: Partial<UpdaterState>) => {
    if (mountedRef.current) setState((current) => ({ ...current, ...patch }));
  }, []);

  const check = useCallback(
    async (options: { silent?: boolean } = {}) => {
      if (busyRef.current || !isTauriRuntime()) return;
      busyRef.current = true;
      if (!options.silent) safeSet({ status: "checking", error: null });
      try {
        const result: UpdateCheckResult = await luxCommands.updateCheck();
        safeSet({
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
          safeSet({ status: "idle" });
        } else {
          safeSet({ status: "error", error: readMessage(error) });
        }
      } finally {
        busyRef.current = false;
      }
    },
    [safeSet],
  );

  const install = useCallback(async () => {
    if (busyRef.current || !isTauriRuntime()) return;
    busyRef.current = true;
    safeSet({ status: "downloading", error: null, progress: null, downloadedBytes: 0, totalBytes: null });
    try {
      // The process is replaced on success, so this resolves only mid-relaunch
      // or never returns. Mark relaunching optimistically before awaiting.
      await luxCommands.updateInstall();
      safeSet({ status: "relaunching" });
    } catch (error) {
      safeSet({ status: "error", error: readMessage(error) });
      busyRef.current = false;
    }
  }, [safeSet]);

  const dismiss = useCallback(() => {
    // Return to a neutral state without forgetting the detected version, so a
    // dismissed banner can be re-surfaced from Settings.
    safeSet({ status: "idle", error: null });
  }, [safeSet]);

  // Stream download progress from the backend into the state machine.
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void subscribeUpdateProgress((event) => {
      if (event.kind === "started") {
        safeSet({ status: "downloading", totalBytes: event.contentLength, downloadedBytes: 0, progress: event.contentLength ? 0 : null });
      } else if (event.kind === "progress") {
        const fraction = event.contentLength ? Math.min(1, event.downloaded / event.contentLength) : null;
        safeSet({ downloadedBytes: event.downloaded, totalBytes: event.contentLength, progress: fraction });
      } else {
        safeSet({ progress: 1, status: "relaunching" });
      }
    }).then((dispose) => {
      if (cancelled) dispose();
      else unlisten = dispose;
    });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [safeSet]);

  // First check shortly after startup, then on a long interval.
  useEffect(() => {
    mountedRef.current = true;
    if (!isTauriRuntime()) return;
    const initial = window.setTimeout(() => void check({ silent: true }), INITIAL_CHECK_DELAY_MS);
    const interval = window.setInterval(() => void check({ silent: true }), AUTO_CHECK_INTERVAL_MS);
    return () => {
      mountedRef.current = false;
      window.clearTimeout(initial);
      window.clearInterval(interval);
    };
  }, [check]);

  return { state, check, install, dismiss };
}

function readMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
