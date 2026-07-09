import { useLuxStore } from "./../store/index";
import {
  ensureRuntimeProvisionSubscription,
  getRuntimeProvisionProgressFor,
  onRuntimeProvisionFinished,
} from "./runtime-provision-store";
import { luxCommands } from "./../tauri/commands";

/**
 * Startup runtime bootstrap (the "smart baseline" policy).
 *
 * On a fresh machine the host toolchains language servers need may be absent. We
 * don't blindly download all of them — instead, at IDE startup we provision only
 * the single highest-leverage runtime in the background:
 *
 *   • Node — unblocks the npm server family (TS/JS, JSON, HTML, CSS, YAML, Bash),
 *     i.e. the languages almost every project touches. ~30 MB, worth it eagerly.
 *
 * Rust, Python and Go are NOT fetched here — they are provisioned on demand when a
 * project in that language is opened (see lspAutoInstall). This keeps the cold-start
 * download small while still making the common case "just works" out of the box.
 *
 * Runs once per session; gated by the `runtimeAutoProvision` preference. Idempotent
 * and safe on machines that already have a system toolchain (the catalog reports it
 * installed and we skip).
 */

/** Runtimes provisioned eagerly at startup. Everything else is on-demand. */
const STARTUP_BASELINE = ["node"] as const;

let bootstrapped = false;

export function bootstrapManagedRuntimes() {
  if (bootstrapped) return;
  if (!useLuxStore.getState().aiPreferences.runtimeAutoProvision) return;
  bootstrapped = true;

  ensureRuntimeProvisionSubscription();
  void luxCommands.runtimeCatalog()
    .then((catalog) => {
      for (const id of STARTUP_BASELINE) {
        const entry = catalog.find((candidate) => candidate.id === id);
        if (!entry || entry.installed || !entry.canAuto) continue;
        if (getRuntimeProvisionProgressFor(id)?.status === "installing") continue;
        void luxCommands.runtimeProvision(id).catch(() => undefined);
      }
    })
    .catch(() => undefined);

  // When the baseline Node finishes, re-pull the server list so any languages whose
  // servers were blocked purely on a missing runtime can come online without reopen.
  const stop = onRuntimeProvisionFinished((id, success) => {
    if (!success || !STARTUP_BASELINE.includes(id as (typeof STARTUP_BASELINE)[number])) return;
    luxCommands.lspServers()
      .then((servers) => useLuxStore.getState().setLanguageServers(servers))
      .catch(() => undefined);
  });
  window.setTimeout(stop, 20 * 60 * 1000);
}

/** Reset for tests / explicit re-run. */
export function resetRuntimeBootstrap() {
  bootstrapped = false;
}
