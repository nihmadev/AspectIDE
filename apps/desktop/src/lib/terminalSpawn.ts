import { useLuxStore } from "./store";
import { isTauriRuntime, luxCommands } from "./tauri";
import type { TerminalSessionInfo } from "./types";

// Single source of truth for creating a terminal session. Both the command palette
// ("terminal.new") and BottomPanel's auto-spawn route through here so they can never
// open two shells by racing each other: a module-level in-flight promise coalesces
// concurrent callers onto the same creation.
let inFlight: Promise<TerminalSessionInfo | null> | null = null;

/** True while a terminal create request is outstanding (callers can suppress auto-spawn). */
export function isTerminalSpawnInFlight(): boolean {
  return inFlight !== null;
}

/**
 * Create a terminal session and register it in the store (made active). Concurrent calls
 * share the same in-flight request and resolve to the same session, preventing duplicate
 * shells. Returns null outside the Tauri runtime.
 */
export function spawnTerminalSession(): Promise<TerminalSessionInfo | null> {
  if (inFlight) return inFlight;
  if (!isTauriRuntime()) return Promise.resolve(null);

  inFlight = luxCommands
    .terminalCreate()
    .then((session) => {
      useLuxStore.getState().upsertTerminalSession(session, true);
      return session;
    })
    .finally(() => {
      inFlight = null;
    });
  return inFlight;
}
