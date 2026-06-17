import { subscribeRuntimeProvision, type RuntimeProvisionEvent } from "./tauri";

/**
 * Live provision-progress state per managed runtime (Node/Rust/Python), driven by
 * the lux://runtime-provision event stream. In-memory only (progress is transient
 * UI). The Settings panel reads it through useSyncExternalStore; the catalog
 * (installed/path) is fetched separately and refreshed when a provision finishes.
 *
 * Mirrors lspInstallStore so both panels behave identically.
 */
export type RuntimeProvisionProgress = {
  status: "installing" | "error";
  /** 0–100 coarse progress (downloads report by Content-Length, steps otherwise). */
  percent: number;
  /** Human step label, e.g. "Downloading" / "Installing toolchain". */
  step: string;
  /** Populated when status === "error". */
  error?: string;
};

type Listener = () => void;

let progressById: Record<string, RuntimeProvisionProgress> = {};
const listeners = new Set<Listener>();
let unlisten: (() => void) | null = null;
let starting = false;
/** Fired once when any provision finishes, so the catalog can be re-fetched. */
const finishListeners = new Set<(id: string, success: boolean) => void>();

function emit() {
  for (const listener of listeners) listener();
}

export function subscribeRuntimeProvisionProgress(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getRuntimeProvisionProgressSnapshot() {
  return progressById;
}

export function getRuntimeProvisionProgressFor(id: string): RuntimeProvisionProgress | null {
  return progressById[id] ?? null;
}

/** Register a callback fired when a provision finishes (to refresh the catalog). */
export function onRuntimeProvisionFinished(handler: (id: string, success: boolean) => void) {
  finishListeners.add(handler);
  return () => {
    finishListeners.delete(handler);
  };
}

function setProgress(id: string, value: RuntimeProvisionProgress | null) {
  if (value === null) {
    if (!(id in progressById)) return;
    const { [id]: _removed, ...rest } = progressById;
    progressById = rest;
  } else {
    progressById = { ...progressById, [id]: value };
  }
  emit();
}

function handleEvent(event: RuntimeProvisionEvent) {
  switch (event.kind) {
    case "started":
      setProgress(event.id, { status: "installing", percent: 0, step: "Starting" });
      break;
    case "progress":
      setProgress(event.id, {
        status: "installing",
        percent: Math.max(0, Math.min(100, Math.round(event.percent))),
        step: event.step,
      });
      break;
    case "finished":
      if (event.success) {
        // Clear progress on success; the catalog refresh will flip it to "installed".
        setProgress(event.id, null);
      } else {
        setProgress(event.id, {
          status: "error",
          percent: 0,
          step: "Failed",
          error: event.error ?? "Setup failed",
        });
      }
      for (const handler of finishListeners) handler(event.id, event.success);
      break;
  }
}

/**
 * Begin listening for provision events. Idempotent — safe to call from multiple
 * mounts; the single underlying subscription is shared and never torn down (the
 * stream is process-lifetime, cheap, and provision events are rare).
 */
export function ensureRuntimeProvisionSubscription() {
  if (unlisten || starting) return;
  starting = true;
  void subscribeRuntimeProvision(handleEvent)
    .then((stop) => {
      unlisten = stop;
    })
    .catch(() => undefined)
    .finally(() => {
      starting = false;
    });
}

/** Clear a stuck error entry (e.g. when the user dismisses or retries). */
export function clearRuntimeProvisionError(id: string) {
  const current = progressById[id];
  if (current?.status === "error") setProgress(id, null);
}
