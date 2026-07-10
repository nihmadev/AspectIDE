import { luxCommands } from "../tauri/commands";
import { useLuxStore } from "../store/index";
import { useAspectLinkStore } from "./link-store";
import type { AiProviderConfig } from "../aspector/utils/preferences";

const TOKEN_SETTING_KEY = "aspect.deviceToken";
const LINK_POLL_INTERVAL_MS = 2_000;
const LINK_TIMEOUT_MS = 12 * 60_000;
const LINK_MAX_POLL_ERRORS = 5;

let cachedToken: string | null = null;
let inflight: Promise<string> | null = null;

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));

export function isAspectProvider(provider: Pick<AiProviderConfig, "providerType">): boolean {
  return provider.providerType === ("aspect" as string);
}

async function loadPersistedToken(): Promise<string> {
  if (cachedToken !== null) return cachedToken;
  try {
    const setting = await luxCommands.settingsGet("user", TOKEN_SETTING_KEY);
    const value = setting?.value;
    if (typeof value === "string") cachedToken = value;
    else if (value && typeof value === "object" && typeof (value as { token?: unknown }).token === "string") {
      cachedToken = (value as { token: string }).token;
    } else cachedToken = "";
  } catch {
    cachedToken = "";
  }
  return cachedToken ?? "";
}

export async function getLinkedTokenSilent(): Promise<string> {
  return loadPersistedToken();
}

export async function clearAspectToken(): Promise<void> {
  cachedToken = "";
  try {
    await luxCommands.settingsSet("user", TOKEN_SETTING_KEY, "");
  } catch {
  }
  try {
    const store = useLuxStore.getState();
    const prefs = store.aiPreferences;
    const providers = prefs.providers.map((provider) =>
      isAspectProvider(provider) ? { ...provider, apiKey: "" } : provider,
    );
    store.setAiPreferences({ ...prefs, providers });
  } catch {
  }
}

export async function relinkAspect(baseUrl: string): Promise<string> {
  if (inflight) return inflight;
  await clearAspectToken();
  return ensureAspectToken(baseUrl);
}

export async function ensureAspectToken(baseUrl: string): Promise<string> {
  const existing = await loadPersistedToken();
  if (existing) return existing;
  if (inflight) return inflight;
  inflight = runLinkFlow(baseUrl).finally(() => {
    inflight = null;
  });
  return inflight;
}

async function persistToken(token: string): Promise<void> {
  cachedToken = token;
  injectTokenIntoProviders(token);
  try {
    await luxCommands.settingsSet("user", TOKEN_SETTING_KEY, token);
  } catch {
  }
}

async function runLinkFlow(baseUrl: string): Promise<string> {
  const link = useAspectLinkStore.getState();
  link.show({ phase: "starting", code: "", deepLink: "", error: "" });

  let start: { code: string; deep_link: string };
  try {
    start = await luxCommands.luxideLinkStart(baseUrl);
  } catch (e) {
    useAspectLinkStore.getState().show({ phase: "error", error: errText(e) });
    return "";
  }
  useAspectLinkStore.getState().show({ phase: "waiting", code: start.code, deepLink: start.deep_link });

  const startedAt = Date.now();
  let consecutiveErrors = 0;
  while (useAspectLinkStore.getState().open && Date.now() - startedAt < LINK_TIMEOUT_MS) {
    await sleep(LINK_POLL_INTERVAL_MS);
    if (!useAspectLinkStore.getState().open) return "";
    let poll: { status: string; token: string };
    try {
      poll = await luxCommands.luxideLinkPoll(baseUrl, start.code);
      consecutiveErrors = 0;
    } catch (e) {
      if (++consecutiveErrors >= LINK_MAX_POLL_ERRORS) {
        useAspectLinkStore.getState().show({ phase: "error", error: errText(e) });
        return "";
      }
      continue;
    }
    if (poll.status === "ready") {
      if (poll.token) {
        await persistToken(poll.token);
        useAspectLinkStore.getState().show({ phase: "linked" });
        setTimeout(() => useAspectLinkStore.getState().hide(), 1_500);
        return poll.token;
      }
      useAspectLinkStore.getState().show({ phase: "error", error: "Linking returned no token — try again." });
      return "";
    }
  }
  if (useAspectLinkStore.getState().open) {
    useAspectLinkStore.getState().show({ phase: "error", error: "Link timed out — please try again." });
  }
  return "";
}

export async function resolveProviderApiKey(
  provider: Pick<AiProviderConfig, "providerType" | "apiKey" | "baseUrl">,
): Promise<string> {
  if (provider.apiKey) return provider.apiKey;
  if (isAspectProvider(provider)) {
    try {
      return await ensureAspectToken(provider.baseUrl);
    } catch {
      return "";
    }
  }
  return "";
}

function injectTokenIntoProviders(token: string): void {
  try {
    const store = useLuxStore.getState();
    const prefs = store.aiPreferences;
    let changed = false;
    const providers = prefs.providers.map((provider) => {
      if (isAspectProvider(provider) && !provider.apiKey) {
        changed = true;
        return { ...provider, apiKey: token };
      }
      return provider;
    });
    if (changed) store.setAiPreferences({ ...prefs, providers });
  } catch {
  }
}

export function __resetAspectTokenCache(): void {
  cachedToken = null;
  inflight = null;
}
