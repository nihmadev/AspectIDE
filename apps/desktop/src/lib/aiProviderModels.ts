import { standardReasoningEfforts, type AiModelConfig, type AiProviderConfig, type AiProviderPresetId } from "./aiPreferences";
import { inferContextTokensFromModelRef } from "./aiModelContext";
import { isTauriRuntime, luxCommands } from "./tauri";

/** OpenCode Zen marks free models with an id suffix. */
const FREE_MODEL_SUFFIX = "-free";

/** Free models on OpenCode Zen expose a 1M-token window; seed that when we can't infer. */
const FREE_MODEL_CONTEXT_TOKENS = 1_000_000;

/**
 * Per-provider rules for turning a raw model id into a display config. Only
 * OpenCode Zen tags free models with `-free`, sorts them first, and grants them a
 * 1M window; every other provider is vendor-neutral — alphabetical order, plain
 * title-cased names, and name-based context inference (or auto-detect downstream).
 * Keying these rules on `providerType` stops one vendor's quirks (e.g. the free →
 * 1M override) from corrupting another provider's model list and token budgeting.
 */
type ProviderDiscoveryProfile = {
  /** Whether this id is a "free" model (only OpenCode Zen distinguishes them). */
  isFreeModel: (id: string) => boolean;
  /** Sort comparator for raw model ids, applied to the fetched catalog. */
  compareIds: (left: string, right: string) => number;
  /** Context-window tokens for an id, or null to auto-detect from the alias downstream. */
  contextTokensFor: (id: string) => number | null;
  /** Human-friendly display name for an id. */
  humanize: (id: string) => string;
};

/** Title-case a raw model id: split on -_/ separators, capitalize lowercase segments. */
function titleCaseModelId(id: string): string {
  const label = id
    .split(/[-_/]/g)
    .filter(Boolean)
    .map((part) => (/^[a-z]/.test(part) ? part.charAt(0).toUpperCase() + part.slice(1) : part))
    .join(" ");
  return label || id;
}

/** Default rules for any standard provider (OpenAI, Anthropic, OpenRouter, local, …). */
const VENDOR_NEUTRAL_PROFILE: ProviderDiscoveryProfile = {
  isFreeModel: () => false,
  compareIds: (left, right) => left.localeCompare(right),
  contextTokensFor: (id) => inferContextTokensFromModelRef(id) ?? null,
  humanize: titleCaseModelId,
};

/** OpenCode Zen rules: free-first ordering, free → 1M window, "(Free)" labelling. */
const OPENCODE_ZEN_PROFILE: ProviderDiscoveryProfile = {
  isFreeModel: (id) => id.trim().toLowerCase().endsWith(FREE_MODEL_SUFFIX),
  compareIds: (left, right) => {
    const leftFree = OPENCODE_ZEN_PROFILE.isFreeModel(left);
    const rightFree = OPENCODE_ZEN_PROFILE.isFreeModel(right);
    if (leftFree !== rightFree) return leftFree ? -1 : 1;
    return left.localeCompare(right);
  },
  // Free OpenCode Zen models expose a 1M window regardless of the base family — so
  // the free window MUST win over name-based inference (a "deepseek-…-free" id would
  // otherwise be inferred as DeepSeek's 128k). Non-free ids fall back to inference.
  contextTokensFor: (id) =>
    OPENCODE_ZEN_PROFILE.isFreeModel(id)
      ? FREE_MODEL_CONTEXT_TOKENS
      : (inferContextTokensFromModelRef(id) ?? null),
  humanize: (id) => {
    const free = OPENCODE_ZEN_PROFILE.isFreeModel(id);
    const base = free ? id.slice(0, -FREE_MODEL_SUFFIX.length) : id;
    const label = titleCaseModelId(base);
    return free ? `${label} (Free)` : label;
  },
};

/** Resolve the discovery rules for a provider by its preset type. */
export function resolveProviderDiscoveryProfile(providerType: AiProviderPresetId): ProviderDiscoveryProfile {
  return providerType === "opencode-zen" ? OPENCODE_ZEN_PROFILE : VENDOR_NEUTRAL_PROFILE;
}

/**
 * OpenCode Zen free-model marker. Consumed by the Settings auto-refresh effect,
 * which is itself gated on `providerType === "opencode-zen"`, so the OpenCode
 * meaning is the intended one.
 */
export function isFreeModelId(id: string): boolean {
  return OPENCODE_ZEN_PROFILE.isFreeModel(id);
}

/** Build a model config from a raw id using the provider's discovery profile. */
function modelConfigFromId(id: string, profile: ProviderDiscoveryProfile): AiModelConfig {
  return {
    id,
    name: profile.humanize(id),
    alias: id,
    contextTokens: profile.contextTokensFor(id),
    effortLevels: standardReasoningEfforts(),
  };
}

/**
 * Fetch a provider's live model list and return it as AiModelConfig[], ordered and
 * labelled per the provider's discovery profile. Pure data fetch — no hardcoded
 * catalog. Dedupes ids and drops blanks. Throws on transport/HTTP failure so the
 * caller can surface it; returns [] only when the provider genuinely reports none.
 */
export async function fetchProviderModelConfigs(provider: AiProviderConfig): Promise<AiModelConfig[]> {
  if (!isTauriRuntime()) {
    throw new Error("Live model discovery requires the desktop runtime.");
  }
  const profile = resolveProviderDiscoveryProfile(provider.providerType);
  const ids = await luxCommands.aiListProviderModels(provider.baseUrl, provider.apiKey || null);
  const unique = [...new Set(ids.map((id) => id.trim()).filter(Boolean))];
  unique.sort(profile.compareIds);
  return unique.map((id) => modelConfigFromId(id, profile));
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
    // The freshly-fetched values are the source of truth (the profile re-derives
    // name/context); only carry a prior value forward when the fetch produced none.
    // NOTE: manual prices are always preserved (the fetch never sets a price), but a
    // manual context override only survives for ids whose profile yields no context
    // (non-free) — a free OpenCode Zen id always re-derives to 1M and wins here.
    return {
      ...model,
      inputPricePerMillion: prior.inputPricePerMillion ?? model.inputPricePerMillion,
      outputPricePerMillion: prior.outputPricePerMillion ?? model.outputPricePerMillion,
      contextTokens: model.contextTokens ?? prior.contextTokens,
    };
  });
  return { ...provider, models };
}
