// LuxIDE managed-gateway device linking (client side).
//
// The bundled "LuxIDE" provider ships with no API key. To use it a user links their
// Telegram account: we ask the gateway for a code + deep link, show a modal, the user
// solves a captcha in the bot, and we poll until the gateway hands back a per-device
// token (1 Telegram = 1 user = 1 set of limits). The token is cached + persisted and
// used as the provider's bearer credential.

import { luxCommands } from "./tauri";
import { useLuxStore } from "./store";
import { useLuxideLinkStore } from "./luxideLinkStore";
import type { AiProviderConfig } from "./aiPreferences";

const TOKEN_SETTING_KEY = "luxide.deviceToken";
const LINK_POLL_INTERVAL_MS = 2_000;
const LINK_TIMEOUT_MS = 12 * 60_000;
const LINK_MAX_POLL_ERRORS = 5;

let cachedToken: string | null = null;
let inflight: Promise<string> | null = null;

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));
const errText = (e: unknown) => (e instanceof Error ? e.message : String(e));

/** True for the bundled managed LuxIDE provider. */
export function isLuxideProvider(provider: Pick<AiProviderConfig, "providerType">): boolean {
  return provider.providerType === "luxide";
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

/**
 * Return the persisted LuxIDE token WITHOUT prompting to link. Used by background
 * paths (model sync, usage indicator) that must stay silent until the user links.
 */
export async function getLinkedTokenSilent(): Promise<string> {
  return loadPersistedToken();
}

/** Wipe the cached + persisted token and clear it from the in-memory providers. */
export async function clearLuxideToken(): Promise<void> {
  cachedToken = "";
  try {
    await luxCommands.settingsSet("user", TOKEN_SETTING_KEY, "");
  } catch {
    // ignore — the in-memory clear below still forces a re-link this session
  }
  try {
    const store = useLuxStore.getState();
    const prefs = store.aiPreferences;
    const providers = prefs.providers.map((provider) =>
      isLuxideProvider(provider) ? { ...provider, apiKey: "" } : provider,
    );
    store.setAiPreferences({ ...prefs, providers });
  } catch {
    // store not ready — non-fatal
  }
}

/**
 * Handle a LuxIDE auth failure (e.g. a stale/rejected token after the gateway moved
 * to Telegram-linked identity): drop the bad token and re-run the interactive link
 * flow so the modal appears. No-op while a link flow is already in flight.
 */
export async function relinkLuxide(baseUrl: string): Promise<string> {
  if (inflight) return inflight;
  await clearLuxideToken();
  return ensureLuxideToken(baseUrl);
}

/**
 * Return a valid LuxIDE device token, running the interactive Telegram-link flow
 * (modal + captcha + poll) on first use. Concurrent callers share one in-flight
 * flow. Resolves to "" if the user cancels or linking fails (the caller then sends
 * no key and surfaces the gateway 401).
 */
export async function ensureLuxideToken(baseUrl: string): Promise<string> {
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
    // Persist failure is non-fatal — the in-memory token still works this session.
  }
}

/** Drive the modal + gateway link/poll loop; returns the token or "" (cancelled/failed). */
async function runLinkFlow(baseUrl: string): Promise<string> {
  const link = useLuxideLinkStore.getState();
  link.show({ phase: "starting", code: "", deepLink: "", error: "" });

  let start: { code: string; deep_link: string };
  try {
    start = await luxCommands.luxideLinkStart(baseUrl);
  } catch (e) {
    useLuxideLinkStore.getState().show({ phase: "error", error: errText(e) });
    return "";
  }
  useLuxideLinkStore.getState().show({ phase: "waiting", code: start.code, deepLink: start.deep_link });

  const startedAt = Date.now();
  let consecutiveErrors = 0;
  while (useLuxideLinkStore.getState().open && Date.now() - startedAt < LINK_TIMEOUT_MS) {
    await sleep(LINK_POLL_INTERVAL_MS);
    if (!useLuxideLinkStore.getState().open) return ""; // user closed the modal
    let poll: { status: string; token: string };
    try {
      poll = await luxCommands.luxideLinkPoll(baseUrl, start.code);
      consecutiveErrors = 0;
    } catch (e) {
      // Tolerate transient network blips; only give up after several in a row.
      if (++consecutiveErrors >= LINK_MAX_POLL_ERRORS) {
        useLuxideLinkStore.getState().show({ phase: "error", error: errText(e) });
        return "";
      }
      continue;
    }
    if (poll.status === "ready") {
      if (poll.token) {
        await persistToken(poll.token);
        useLuxideLinkStore.getState().show({ phase: "linked" });
        setTimeout(() => useLuxideLinkStore.getState().hide(), 1_500);
        return poll.token;
      }
      // "ready" without a token shouldn't happen — surface it rather than loop.
      useLuxideLinkStore.getState().show({ phase: "error", error: "Linking returned no token — try again." });
      return "";
    }
  }
  // Timed out (not cancelled): leave the modal on an error so it isn't stuck spinning.
  if (useLuxideLinkStore.getState().open) {
    useLuxideLinkStore.getState().show({ phase: "error", error: "Link timed out — please try again." });
  }
  return "";
}

/**
 * Resolve the bearer credential for a provider: an explicit key wins; otherwise
 * the LuxIDE provider auto-enrolls. Returns "" when there is no credential.
 */
export async function resolveProviderApiKey(
  provider: Pick<AiProviderConfig, "providerType" | "apiKey" | "baseUrl">,
): Promise<string> {
  if (provider.apiKey) return provider.apiKey;
  if (isLuxideProvider(provider)) {
    try {
      return await ensureLuxideToken(provider.baseUrl);
    } catch {
      return "";
    }
  }
  return "";
}

/**
 * Populate the in-memory LuxIDE provider(s) with the token so every runtime path
 * (native turns, auto-compaction, goal evaluation, model listing) can read it as
 * provider.apiKey within this session. Persistence lives in the dedicated token
 * setting, so we don't rewrite ai.preferences here.
 */
function injectTokenIntoProviders(token: string): void {
  try {
    const store = useLuxStore.getState();
    const prefs = store.aiPreferences;
    let changed = false;
    const providers = prefs.providers.map((provider) => {
      if (isLuxideProvider(provider) && !provider.apiKey) {
        changed = true;
        return { ...provider, apiKey: token };
      }
      return provider;
    });
    if (changed) store.setAiPreferences({ ...prefs, providers });
  } catch {
    // Store not ready (e.g. during tests) — non-fatal.
  }
}

/** Clear the cached token (used by tests). */
export function __resetLuxideTokenCache(): void {
  cachedToken = null;
  inflight = null;
}
