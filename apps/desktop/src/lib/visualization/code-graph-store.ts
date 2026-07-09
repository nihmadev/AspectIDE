import { subscribeCodeGraph, type CodeGraphEvent, type CodeGraphStatus } from "./../tauri/commands";

/**
 * Live code-graph build state, driven by the lux://code-graph event stream.
 * In-memory only (build progress is transient UI). The Settings panel reads it
 * through useSyncExternalStore; the persisted status (node/edge/file counts) is
 * fetched separately via codeGraphStatus() and refreshed when a build finishes.
 *
 * Mirrors runtimeProvisionStore so the panels behave identically, but the code
 * graph is a single workspace-wide artifact (no per-id keying).
 */
export type CodeGraphState = {
  status: "idle" | "building" | "ready" | "error";
  /** 0–100 coarse build progress. */
  percent: number;
  /** Human step label, e.g. "Collecting and parsing source files". */
  step: string;
  /** Populated when status === "ready". */
  nodeCount: number;
  edgeCount: number;
  /** Populated when status === "error". */
  error?: string;
};

type Listener = () => void;

let state: CodeGraphState = {
  status: "idle",
  percent: 0,
  step: "",
  nodeCount: 0,
  edgeCount: 0,
};
const listeners = new Set<Listener>();
let unlisten: (() => void) | null = null;
let starting = false;
/** Fired once when a build finishes, so the status counts can be re-fetched. */
const finishListeners = new Set<(success: boolean) => void>();

function emit() {
  for (const listener of listeners) listener();
}

export function subscribeCodeGraphState(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getCodeGraphStateSnapshot() {
  return state;
}

/** Register a callback fired when a build finishes (to refresh status counts). */
export function onCodeGraphBuildFinished(handler: (success: boolean) => void) {
  finishListeners.add(handler);
  return () => {
    finishListeners.delete(handler);
  };
}

function setState(next: CodeGraphState) {
  state = next;
  emit();
}

/** Seed the store from a fetched status snapshot (e.g. on Settings mount). */
export function applyCodeGraphStatus(status: CodeGraphStatus) {
  // Don't clobber an in-flight build with a stale "ready/idle" snapshot.
  if (state.status === "building") return;
  setState({
    status: status.ready ? "ready" : "idle",
    percent: status.ready ? 100 : 0,
    step: status.ready ? "Ready" : "",
    nodeCount: status.nodeCount,
    edgeCount: status.edgeCount,
  });
}

function handleEvent(event: CodeGraphEvent) {
  switch (event.kind) {
    case "started":
      setState({ status: "building", percent: 0, step: "Starting", nodeCount: 0, edgeCount: 0 });
      break;
    case "progress":
      setState({
        ...state,
        status: "building",
        percent: Math.max(0, Math.min(100, Math.round(event.percent))),
        step: event.step,
      });
      break;
    case "finished":
      if (event.success) {
        setState({
          status: "ready",
          percent: 100,
          step: "Ready",
          nodeCount: event.nodeCount,
          edgeCount: event.edgeCount,
        });
      } else {
        setState({
          status: "error",
          percent: 0,
          step: "Failed",
          nodeCount: 0,
          edgeCount: 0,
          error: event.error ?? "Code graph build failed",
        });
      }
      for (const handler of finishListeners) handler(event.success);
      break;
    case "updated":
      // Incremental file-watcher refresh: update counts in place without flipping
      // through "building". The backend only emits this when no full build for the
      // current workspace is in flight, so applying it unconditionally is safe — and
      // it also recovers a stale "building"/"error" status (e.g. a failed rebuild
      // that never sent a terminal event).
      setState({
        status: "ready",
        percent: 100,
        step: "Ready",
        nodeCount: event.nodeCount,
        edgeCount: event.edgeCount,
      });
      for (const handler of finishListeners) handler(true);
      break;
  }
}

/**
 * Begin listening for code-graph events. Idempotent — safe to call from multiple
 * mounts; the single underlying subscription is shared and never torn down (the
 * stream is process-lifetime and build events are rare).
 */
export function ensureCodeGraphSubscription() {
  if (unlisten || starting) return;
  starting = true;
  void subscribeCodeGraph(handleEvent)
    .then((stop) => {
      unlisten = stop;
    })
    .catch(() => undefined)
    .finally(() => {
      starting = false;
    });
}

/** Clear a stuck error entry (e.g. when the user dismisses or retries). */
export function clearCodeGraphError() {
  if (state.status === "error") {
    setState({ status: "idle", percent: 0, step: "", nodeCount: 0, edgeCount: 0 });
  }
}
