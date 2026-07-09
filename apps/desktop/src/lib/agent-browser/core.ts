import type { AiPreferences } from "../aspector/utils/preferences";
import type { AgentBrowserInvokeRequest } from "../tauri/commands";

export function browserSessionName(chatSessionId: string | undefined) {
  const cleaned = (chatSessionId ?? "default")
    .trim()
    .replace(/[^a-zA-Z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48);
  return cleaned ? `lux-${cleaned}` : "lux-default";
}

export function tokenizeBrowserCommand(command: string) {
  const trimmed = command.trim();
  if (!trimmed) return [];
  const tokens: string[] = [];
  let current = "";
  let quote: "'" | '"' | null = null;
  for (const ch of trimmed) {
    if (quote) {
      if (ch === quote) {
        quote = null;
        tokens.push(current);
        current = "";
      } else {
        current += ch;
      }
      continue;
    }
    if (ch === "'" || ch === '"') {
      if (current) tokens.push(current);
      current = "";
      quote = ch;
      continue;
    }
    if (/\s/.test(ch)) {
      if (current) tokens.push(current);
      current = "";
      continue;
    }
    current += ch;
  }
  if (current) tokens.push(current);
  return tokens;
}

export function buildBrowserInvokeRequest(
  preferences: AiPreferences,
  session: string,
  args: string[],
  overrides?: Partial<Pick<AgentBrowserInvokeRequest, "headed" | "timeoutSecs">>,
): AgentBrowserInvokeRequest {
  const persistenceName = preferences.agentBrowserPersistSession
    ? `${session}-persist`
    : undefined;
  return {
    session,
    args,
    headed: overrides?.headed ?? (preferences.agentBrowserHeaded ? true : undefined),
    allowedDomains: preferences.agentBrowserAllowedDomains.trim() || undefined,
    maxOutput: preferences.agentBrowserMaxOutput,
    timeoutSecs: overrides?.timeoutSecs,
    commandPath: preferences.agentBrowserCommand.trim() || undefined,
    sessionName: persistenceName ?? null,
    profile: preferences.agentBrowserProfile.trim() || null,
    statePath: preferences.agentBrowserStatePath.trim() || null,
    contentBoundaries: preferences.agentBrowserContentBoundaries ? true : null,
    ignoreHttpsErrors: preferences.agentBrowserIgnoreHttpsErrors ? true : null,
    allowFileAccess: preferences.agentBrowserAllowFileAccess ? true : null,
    provider: preferences.agentBrowserProvider.trim() || null,
    proxy: preferences.agentBrowserProxy.trim() || null,
  };
}

export function browserActArgs(command: string, batch?: string[]) {
  if (batch && batch.length > 0) {
    // agent-browser `batch` argument mode: each positional is a FULL command
    // string that the CLI tokenizes itself (e.g. `batch "open x" "snapshot -i"`).
    // We pass the entries verbatim. (The JSON-array shape is stdin-only, and the
    // native runner spawns with stdin=null, so it cannot be used here.)
    return ["batch", "--json", ...batch.map((entry) => entry.trim()).filter(Boolean)];
  }
  const tokens = tokenizeBrowserCommand(command);
  if (tokens.length === 0) throw new Error("BrowserAct requires a command.");
  return tokens;
}

const screenshotPathPattern = /(?:saved to|screenshot(?:\s+saved)?(?:\s+to)?)\s+([^\n\r]+)/i;

export function screenshotPathFromBrowserData(data: unknown, text?: string) {
  if (data && typeof data === "object" && !Array.isArray(data)) {
    const record = data as Record<string, unknown>;
    for (const key of ["path", "file", "screenshot", "screenshotPath", "outputPath"]) {
      const value = record[key];
      if (typeof value === "string" && value.trim()) return value.trim();
    }
  }
  const source = text?.trim() ?? "";
  if (!source) return null;
  const match = source.match(screenshotPathPattern);
  return match?.[1]?.trim().replace(/^['"]|['"]$/g, "") ?? null;
}

/** Read stream status without starting Chromium or enabling the WebSocket feed. */
export async function queryBrowserStream(session: string, commandPath?: string) {
  const { luxCommands } = await import("../tauri/commands");
  return luxCommands.agentBrowserStreamStatus({
    session,
    commandPath: commandPath ?? null,
    enable: null,
    port: null,
  });
}

/** Enable viewport streaming after BrowserOpen (or explicit user refresh). */
export async function ensureBrowserStream(session: string, commandPath?: string, port?: number) {
  const { luxCommands } = await import("../tauri/commands");
  return luxCommands.agentBrowserStreamStatus({
    session,
    commandPath: commandPath ?? null,
    enable: true,
    port: port ?? null,
  });
}