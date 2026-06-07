import type { AiModelConfig, AiPreferences, AiProviderConfig, AiVisionImageFormatPreference } from "./aiPreferences";

export type { AiVisionImageFormatPreference } from "./aiPreferences";

/** Target encoding for a vision image sent to the model. */
export type VisionImageFormat = "webp" | "png";

/**
 * Provider families whose hosted vision models reliably decode WebP. Anthropic,
 * OpenAI, Google Gemini, and xAI all document WebP support; OpenRouter proxies
 * to those same upstreams. Anything not on this list — local servers (Ollama,
 * LM Studio, custom proxies), Azure deployments of unknown vintage, or an
 * unrecognized provider — defaults to PNG, which every vision stack accepts.
 *
 * PNG is the safe floor: a model that cannot read WebP would silently drop the
 * image (or error the turn), so we only opt into WebP when we're confident.
 */
const WEBP_SAFE_PROVIDER_TYPES = new Set<AiProviderConfig["providerType"]>([
  "anthropic",
  "openai",
  "google",
  "openrouter",
  "xai",
]);

/** Model id/alias fragments that are known to be vision-capable WebP decoders,
 *  used to rescue WebP on otherwise-unknown providers (e.g. a custom gateway in
 *  front of Claude/GPT/Gemini). Matched case-insensitively against id + alias. */
const WEBP_SAFE_MODEL_HINTS = [
  "claude",
  "gpt-4o",
  "gpt-5",
  "gpt-4.1",
  "gemini",
  "grok",
  "llama-3.2", // Llama 3.2 Vision
  "llama-4",
  "qwen2-vl",
  "qwen2.5-vl",
  "pixtral",
];

function modelLooksWebpSafe(model: AiModelConfig | null | undefined): boolean {
  if (!model) return false;
  const haystack = `${model.id} ${model.alias}`.toLowerCase();
  return WEBP_SAFE_MODEL_HINTS.some((hint) => haystack.includes(hint));
}

/**
 * Resolves the vision image format to encode for the active provider/model.
 *
 * - `pref === "webp" | "png"` forces that format (user override).
 * - `pref === "auto"` (default) picks WebP only when the provider family is
 *   known WebP-safe, or the model name strongly implies a WebP-capable vision
 *   model behind an unknown provider; otherwise PNG.
 *
 * The Rust encoder degrades further on its own (PNG fallback on encode failure,
 * passthrough for undecodable sources), so an over-eager WebP choice never
 * corrupts a payload — at worst it costs one re-encode. This resolver's job is
 * to avoid handing WebP to a model that can't read it at all.
 */
export function resolveVisionImageFormat(
  provider: AiProviderConfig | null | undefined,
  model: AiModelConfig | null | undefined,
  pref: AiVisionImageFormatPreference,
): VisionImageFormat {
  if (pref === "webp" || pref === "png") return pref;
  if (!provider) return modelLooksWebpSafe(model) ? "webp" : "png";
  if (WEBP_SAFE_PROVIDER_TYPES.has(provider.providerType)) return "webp";
  // Unknown/custom/local provider: trust a recognizable vision model name only.
  return modelLooksWebpSafe(model) ? "webp" : "png";
}

export const AI_VISION_IMAGE_FORMATS: readonly AiVisionImageFormatPreference[] = ["auto", "webp", "png"];

/**
 * Pulls the active provider/model from preferences and resolves the format.
 * Convenience wrapper for call sites that already hold an `AiPreferences`.
 */
export function resolveVisionImageFormatFromPreferences(preferences: AiPreferences): VisionImageFormat {
  const provider = preferences.providers.find((entry) => entry.id === preferences.selectedProviderId) ?? null;
  const model = provider?.models.find((entry) => entry.id === preferences.selectedModelId) ?? null;
  return resolveVisionImageFormat(provider, model, preferences.visionImageFormat);
}
