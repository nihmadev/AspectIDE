import { subscribeLspInstall, type LspInstallEvent } from "./tauri";

/**
 * Live install-progress state per language server, driven by the lux://lsp-install
 * event stream. In-memory only (progress is transient UI). The Settings panel reads
 * it through useSyncExternalStore; the catalog (installed/version) is fetched
 * separately and refreshed when an install finishes.
 */
export type LspInstallProgress = {
  status: "installing" | "error";
  /** 0–100 coarse progress (package managers don't report fine-grained steps). */
  percent: number;
  /** Human step label, e.g. "Downloading via npm". */
  step: string;
  /** Populated when status === "error". */
  error?: string;
};

type Listener = () => void;

let progressByLanguage: Record<string, LspInstallProgress> = {};
const listeners = new Set<Listener>();
let unlisten: (() => void) | null = null;
let starting = false;
/** Fired once when any install finishes, so the catalog can be re-fetched. */
const finishListeners = new Set<(languageId: string, success: boolean) => void>();

function emit() {
  for (const listener of listeners) listener();
}

export function subscribeLspInstallProgress(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getLspInstallProgressSnapshot() {
  return progressByLanguage;
}

export function getLspInstallProgressFor(languageId: string): LspInstallProgress | null {
  return progressByLanguage[languageId] ?? null;
}

/** Register a callback fired when an install finishes (to refresh the catalog). */
export function onLspInstallFinished(handler: (languageId: string, success: boolean) => void) {
  finishListeners.add(handler);
  return () => {
    finishListeners.delete(handler);
  };
}

function setProgress(languageId: string, value: LspInstallProgress | null) {
  if (value === null) {
    if (!(languageId in progressByLanguage)) return;
    const { [languageId]: _removed, ...rest } = progressByLanguage;
    progressByLanguage = rest;
  } else {
    progressByLanguage = { ...progressByLanguage, [languageId]: value };
  }
  emit();
}

function handleEvent(event: LspInstallEvent) {
  switch (event.kind) {
    case "started":
      setProgress(event.languageId, { status: "installing", percent: 0, step: "Starting" });
      break;
    case "progress":
      setProgress(event.languageId, {
        status: "installing",
        percent: Math.max(0, Math.min(100, Math.round(event.percent))),
        step: event.step,
      });
      break;
    case "finished":
      if (event.success) {
        // Clear progress on success; the catalog refresh will flip it to "installed".
        setProgress(event.languageId, null);
      } else {
        setProgress(event.languageId, {
          status: "error",
          percent: 0,
          step: "Failed",
          error: event.error ?? "Install failed",
        });
      }
      for (const handler of finishListeners) handler(event.languageId, event.success);
      break;
  }
}

/**
 * Begin listening for install events. Idempotent — safe to call from multiple
 * mounts; the single underlying subscription is shared and never torn down (the
 * stream is process-lifetime, cheap, and install events are rare).
 */
export function ensureLspInstallSubscription() {
  if (unlisten || starting) return;
  starting = true;
  void subscribeLspInstall(handleEvent)
    .then((stop) => {
      unlisten = stop;
    })
    .catch(() => undefined)
    .finally(() => {
      starting = false;
    });
}

/** Clear a stuck error entry (e.g. when the user dismisses or retries). */
export function clearLspInstallError(languageId: string) {
  const current = progressByLanguage[languageId];
  if (current?.status === "error") setProgress(languageId, null);
}
