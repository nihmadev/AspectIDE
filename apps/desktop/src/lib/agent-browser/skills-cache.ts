import { luxCommands } from "../tauri/commands";

let cachedSkills: string | null = null;
let cacheAt = 0;
const cacheTtlMs = 15 * 60 * 1000;
const maxSkillsChars = 14_000;

export async function loadAgentBrowserSkillsReference(commandPath?: string) {
  const now = Date.now();
  if (cachedSkills && now - cacheAt < cacheTtlMs) return cachedSkills;
  try {
    const [core, catalog] = await Promise.all([
      luxCommands.agentBrowserSkills({ name: "core", all: false, commandPath: commandPath ?? null }),
      luxCommands.agentBrowserSkills({ all: true, commandPath: commandPath ?? null }),
    ]);
    const parts = [core.success ? core.content.trim() : "", catalog.success ? catalog.content.trim() : ""].filter(Boolean);
    if (parts.length === 0) return null;
    const merged = parts.join("\n\n");
    cachedSkills = merged.length > maxSkillsChars ? `${merged.slice(0, maxSkillsChars)}\n\n[truncated]` : merged;
    cacheAt = now;
    return cachedSkills;
  } catch {
    return null;
  }
}

export function invalidateAgentBrowserSkillsCache() {
  cachedSkills = null;
  cacheAt = 0;
}