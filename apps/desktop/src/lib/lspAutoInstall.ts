import type { LanguageServerInfo } from "./types";
import { useLuxStore } from "./store";
import { ensureLspInstallSubscription, getLspInstallProgressFor, onLspInstallFinished } from "./lspInstallStore";
import {
  ensureRuntimeProvisionSubscription,
  getRuntimeProvisionProgressFor,
  onRuntimeProvisionFinished,
} from "./runtimeProvisionStore";
import { luxCommands } from "./tauri";

/** Languages we've already kicked off an auto-install for this session, so a
 *  workspace re-scan or repeated `lsp_servers` call never double-installs. */
const attempted = new Set<string>();
/** Runtimes (node/rust/python) we've already kicked off a provision for. */
const attemptedRuntimes = new Set<string>();

/** Cleared between workspaces so reopening a project can retry a failed install. */
export function resetLspAutoInstallAttempts() {
  attempted.clear();
  attemptedRuntimes.clear();
}

/** LSP install method → the host runtime it depends on. */
const METHOD_TO_RUNTIME: Record<string, string> = {
  npm: "node",
  rustup: "rust",
  pip: "python",
  go: "go",
};

/**
 * Ensure the host runtimes (Node/Rust/Python) the about-to-install servers need
 * are present, provisioning the missing auto-installable ones in the background.
 * The Rust side is idempotent + locked, so a server install that also triggers a
 * runtime bring-up is safe; doing it here just makes the bring-up visible in
 * Settings and parallel to the servers rather than serialized behind the first one.
 */
function ensureRuntimesForMethods(methods: Set<string>, onProvisioned: () => void) {
  const neededRuntimes = new Set(
    [...methods].map((method) => METHOD_TO_RUNTIME[method]).filter((id): id is string => Boolean(id)),
  );
  if (neededRuntimes.size === 0) return;

  ensureRuntimeProvisionSubscription();
  void luxCommands.runtimeCatalog()
    .then((catalog) => {
      for (const entry of catalog) {
        if (!neededRuntimes.has(entry.id)) continue;
        if (entry.installed || !entry.canAuto) continue;
        if (attemptedRuntimes.has(entry.id)) continue;
        if (getRuntimeProvisionProgressFor(entry.id)?.status === "installing") continue;
        attemptedRuntimes.add(entry.id);
        void luxCommands.runtimeProvision(entry.id).catch(() => undefined);
      }
    })
    .catch(() => undefined);

  const stop = onRuntimeProvisionFinished((_id, success) => {
    if (success) onProvisioned();
  });
  window.setTimeout(stop, 20 * 60 * 1000);
}

/**
 * For each discovered-but-missing language server, install it in the background
 * when auto-install is enabled and the server has an automated recipe (not a
 * "manual" one). Discovery only returns languages present in the workspace, so
 * this is scoped to what the project actually uses — never a blanket install.
 *
 * `onInstalled` is invoked after each successful install so the caller can
 * re-pull the server list and bring the language online without a restart.
 */
export function maybeAutoInstallLanguageServers(
  servers: LanguageServerInfo[],
  onInstalled: () => void,
) {
  if (!useLuxStore.getState().aiPreferences.lspAutoInstall) return;
  const missing = servers.filter((server) => server.status === "missing");
  if (missing.length === 0) return;

  ensureLspInstallSubscription();

  // Fetch the catalog once to learn which missing servers are auto-installable.
  void luxCommands.lspServerCatalog()
    .then((catalog) => {
      const missingIds = new Set(missing.map((server) => server.language_id));
      // Entries we will actually install: auto-installable + present in this workspace.
      const targets = catalog.filter(
        (entry) => entry.installMethod !== "manual" && !entry.installed && missingIds.has(entry.languageId),
      );

      // First, bring up the host runtimes those installs depend on (Node/Rust/…),
      // in the background and in parallel — so a clean machine self-provisions.
      const methods = new Set(targets.map((entry) => entry.installMethod));
      ensureRuntimesForMethods(methods, onInstalled);

      for (const entry of targets) {
        const languageId = entry.languageId;
        if (attempted.has(languageId)) continue;
        // Skip if an install for this language is already in flight.
        if (getLspInstallProgressFor(languageId)?.status === "installing") continue;
        attempted.add(languageId);
        void luxCommands.lspInstallServer(languageId).catch(() => undefined);
      }
    })
    .catch(() => undefined);

  // Refresh the server list whenever any install finishes successfully.
  const stop = onLspInstallFinished((_languageId, success) => {
    if (success) onInstalled();
  });
  // Detach after a generous window — installs are bounded by the Rust timeout.
  window.setTimeout(stop, 15 * 60 * 1000);
}
