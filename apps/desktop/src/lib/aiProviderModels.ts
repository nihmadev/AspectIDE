import { standardReasoningEfforts, type AiModelConfig, type AiProviderConfig } from "./aiPreferences";
import { inferContextTokensFromModelRef } from "./aiModelContext";
import { isTauriRuntime, luxCommands } from "./tauri";

/** OpenCode Zen marks free models with an id suffix. Free models sort to the top. */
const FREE_MODEL_SUFFIX = "-free";

/** Free models on OpenCode Zen expose a 1M-token window; seed that when we can't infer. */
const FREE_MODEL_CONTEXT_TOKENS = 1_000_000;

export function isFreeModelId(id: string): boolean {
  return id.trim().toLowerCase().endsWith(FREE_MODEL_SUFFIX);
}

/** Turn a raw provider model id into a display name: drop "-free", title-case segments. */
function humanizeModelId(id: string): string {
  const free = isFreeModelId(id);
  const base = free ? id.slice(0, -FREE_MODEL_SUFFIX.length) : id;
  const label = base
    .split(/[-_/]/g)
    .filter(Boolean)
    .map((part) => (/^[a-z]/.test(part) ? part.charAt(0).toUpperCase() + part.slice(1) : part))
    .join(" ");
  return free ? `${label} (Free)` : label || id;
}

/** Reasoning is enabled across OpenCode Zen models, so every entry gets effort levels. */
function modelConfigFromId(id: string): AiModelConfig {
  const free = isFreeModelId(id);
  // Free OpenCode Zen models expose a 1M-token window regardless of the base model
  // family — so the free window MUST win over name-based inference (e.g. a
  // "deepseek-…-free" id would otherwise be inferred as DeepSeek's 128k). Only
  // non-free models fall back to inference, then null (auto-detect downstream).
  const contextTokens = free ? FREE_MODEL_CONTEXT_TOKENS : (inferContextTokensFromModelRef(id) ?? null);
  return {
    id,
    name: humanizeModelId(id),
    alias: id,
    contextTokens,
    effortLevels: standardReasoningEfforts(),
  };
}

/**
 * Sort model ids free-first, then alphabetically within each group. This is the
 * ordering rule the picker shows — free OpenCode Zen models appear at the top.
 */
export function sortModelIdsFreeFirst(ids: string[]): string[] {
  return [...ids].sort((left, right) => {
    const leftFree = isFreeModelId(left);
    const rightFree = isFreeModelId(right);
    if (leftFree !== rightFree) return leftFree ? -1 : 1;
    return left.localeCompare(right);
  });
}

/**
 * Fetch a provider's live model list and return it as AiModelConfig[], free-first.
 * Pure data fetch — no hardcoded catalog. Dedupes ids and drops blanks. Throws on
 * transport/HTTP failure so the caller can surface it; returns [] only when the
 * provider genuinely reports no models.
 */
export async function fetchProviderModelConfigs(provider: AiProviderConfig): Promise<AiModelConfig[]> {
  if (!isTauriRuntime()) {
    throw new Error("Live model discovery requires the desktop runtime.");
  }
  const ids = await luxCommands.aiListProviderModels(provider.baseUrl, provider.apiKey || null);
  const unique = [...new Set(ids.map((id) => id.trim()).filter(Boolean))];
  return sortModelIdsFreeFirst(unique).map(modelConfigFromId);
}

/**
 * Refresh a provider in place with its live catalog, preserving any per-model
 * manual prices the user set (matched by id). Returns a new provider object; the
 * caller persists it. The selected model id is preserved when still present.
 */
export function mergeRefreshedModels(
  provider: AiProviderConfig,
  fetched: AiModelConfig[],
): AiProviderConfig {
  if (fetched.length === 0) return provider;
  const priorById = new Map(provider.models.map((model) => [model.id, model]));
  const models = fetched.map((model) => {
    const prior = priorById.get(model.id);
    if (!prior) return model;
    // Keep user-entered manual prices. For context: the freshly-fetched value is the
    // source of truth (free models = 1M), so it wins; only carry a prior context
    // forward when the fetch produced none, preserving a genuine user override.
    return {
      ...model,
      inputPricePerMillion: prior.inputPricePerMillion ?? model.inputPricePerMillion,
      outputPricePerMillion: prior.outputPricePerMillion ?? model.outputPricePerMillion,
      contextTokens: model.contextTokens ?? prior.contextTokens,
    };
  });
  return { ...provider, models };
}
