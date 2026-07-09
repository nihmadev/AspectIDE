import type { AiPreferences } from "../aspector/utils/preferences";
import { invalidateAgentBrowserSkillsCache } from "./skills-cache";
import { isTauriRuntime, luxCommands } from "../tauri/commands";

/** Check npm latest and upgrade bundled agent-browser when behind (no-op for custom CLI path). */
export async function ensureBundledAgentBrowserLatest(preferences: AiPreferences) {
  if (!isTauriRuntime() || !preferences.agentBrowserEnabled) return;
  if (preferences.agentBrowserCommand.trim()) return;
  try {
    const status = await luxCommands.agentBrowserStatus({ skipAutoUpdate: false, lightweight: true });
    if (status.updatePerformed) invalidateAgentBrowserSkillsCache();
  } catch {
    // Offline or npm unreachable — keep current bundled version.
  }
}